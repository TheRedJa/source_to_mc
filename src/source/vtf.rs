//! Decoding `.vtf` textures down to Minecraft block resolution.
//!
//! Source textures are 512x512 or larger and DXT-compressed; a Minecraft block
//! face is 16x16. Decoding the full image and downsampling with a good filter
//! gives a far better 16x16 than the thumbnail Source embeds for its own use,
//! which is a DXT1-compressed 16x16 and visibly mushy. The full decode costs
//! more, which is why every texture is decoded once and shared: in a real map
//! a single texture backs dozens of materials.

use crate::source::vfs::Vfs;
use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::{DynamicImage, RgbaImage};
use std::collections::HashMap;

/// Textures wider than this are downsampled in two steps.
///
/// Lanczos3 over a 2048x2048 source straight to 16x16 samples a tiny window
/// and aliases badly; halving repeatedly first is both faster and cleaner.
const PREFILTER_ABOVE: u32 = 128;

/// Alpha at or above this counts as solid for an alpha-tested surface.
///
/// Minecraft's `cutout` render type discards below 0.1, but a mid grey would
/// then render solid; half is the honest midpoint for deciding coverage.
const ALPHA_CUTOFF: u8 = 128;

/// Decode a VTF and resize it to `size` x `size` RGBA.
///
/// `alpha_test` matters more than it sounds. Averaging the alpha of a grate
/// down to 16x16 leaves every texel part-transparent, and Minecraft's cutout
/// rendering then draws the whole face solid — the grate pattern disappears
/// entirely. So for those textures the alpha is re-thresholded afterwards,
/// choosing the cutoff that best preserves how much of the original was
/// see-through.
pub fn decode(data: &[u8], size: u32, alpha_test: bool) -> Result<RgbaImage> {
    Ok(decode_tiles(data, size, alpha_test, [1, 1])?.remove(0))
}

/// Decode a VTF and cut it into a `grid` of `size` x `size` tiles, row by row.
///
/// This is what stops a wall looking like a smear. A Source wall texture is
/// laid out to cover several metres of surface, and squeezing all of it onto
/// one block face throws away everything that made it read as brick or
/// panelling. Cut into the pieces that really are in front of each block, the
/// detail comes back and the pattern lines up across the wall.
///
/// One tile is the whole texture, so [`decode`] is this with a 1x1 grid.
pub fn decode_tiles(
    data: &[u8],
    size: u32,
    alpha_test: bool,
    grid: [u32; 2],
) -> Result<Vec<RgbaImage>> {
    let vtf = vtf::from_bytes(data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let image: DynamicImage = vtf
        .highres_image
        .decode(0)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if image.width() == 0 || image.height() == 0 {
        bail!("texture has no pixels");
    }

    // Coverage is measured over the whole texture, not per tile. A grate's
    // solid bars and its holes are unevenly spread, and thresholding each
    // tile to its own coverage would make the sparse ones vanish.
    let coverage = alpha_test.then(|| {
        let pixels = image.to_rgba8();
        let solid = pixels.pixels().filter(|p| p.0[3] >= ALPHA_CUTOFF).count();
        solid as f64 / pixels.pixels().len().max(1) as f64
    });

    let (columns, rows) = (grid[0].max(1), grid[1].max(1));
    let mut tiles = Vec::with_capacity((columns * rows) as usize);
    for row in 0..rows {
        for column in 0..columns {
            // Boundaries are computed from the edges rather than by
            // multiplying a tile width, so a texture whose size does not
            // divide evenly still tiles it completely and without gaps.
            let x0 = image.width() * column / columns;
            let x1 = (image.width() * (column + 1) / columns).max(x0 + 1);
            let y0 = image.height() * row / rows;
            let y1 = (image.height() * (row + 1) / rows).max(y0 + 1);

            let cropped = if columns == 1 && rows == 1 {
                image.clone()
            } else {
                image.crop_imm(x0, y0, x1 - x0, y1 - y0)
            };
            let mut tile = resize(cropped, size);
            if let Some(coverage) = coverage {
                binarize_alpha(&mut tile, coverage);
            }
            tiles.push(tile);
        }
    }
    Ok(tiles)
}

/// Snap alpha to fully on or off, keeping roughly `coverage` of the image
/// solid. Searching for the cutoff rather than fixing one keeps a fine grate
/// from closing up and a sparse one from vanishing.
fn binarize_alpha(image: &mut RgbaImage, coverage: f64) {
    let total = image.pixels().len();
    if total == 0 {
        return;
    }
    let want = (coverage * total as f64).round() as usize;

    let mut alphas: Vec<u8> = image.pixels().map(|p| p.0[3]).collect();
    alphas.sort_unstable();
    // Keep the `want` highest-alpha texels: the cutoff is the value just
    // below them.
    let cutoff = if want == 0 {
        u8::MAX
    } else if want >= total {
        0
    } else {
        alphas[total - want]
    };

    for pixel in image.pixels_mut() {
        pixel.0[3] = if pixel.0[3] >= cutoff && cutoff < u8::MAX { 255 } else { 0 };
    }
}

/// Downsample to a square, halving first so the final filter has a sane
/// sampling window.
fn resize(mut image: DynamicImage, size: u32) -> RgbaImage {
    while image.width() > PREFILTER_ABOVE.max(size) && image.height() > PREFILTER_ABOVE.max(size) {
        image = image.resize_exact(
            (image.width() / 2).max(size),
            (image.height() / 2).max(size),
            FilterType::Triangle,
        );
    }
    image.resize_exact(size, size, FilterType::Lanczos3).to_rgba8()
}

/// What a texture's header says, without decoding its pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Header {
    /// Average colour in linear light — the same quantity the map compiler
    /// copies into the BSP for world materials.
    pub reflectivity: [f64; 3],
    pub size: [u32; 2],
}

/// Decodes textures on demand, keeping each one only once.
pub struct Textures<'a> {
    vfs: &'a Vfs,
    size: u32,
    /// `None` records a texture we already failed to find, so a missing file
    /// is not searched for once per material that references it.
    cache: HashMap<(String, bool, u32, u32), Option<Vec<RgbaImage>>>,
    headers: HashMap<String, Option<Header>>,
}

impl<'a> Textures<'a> {
    pub fn new(vfs: &'a Vfs, size: u32) -> Textures<'a> {
        Textures { vfs, size, cache: HashMap::new(), headers: HashMap::new() }
    }

    /// Load `$basetexture`, e.g. `Concrete/concretewall001a`.
    ///
    /// The alpha-test flag is part of the key because it changes how the
    /// texture is downsampled, so the same file can legitimately be wanted
    /// both ways.
    pub fn get(&mut self, base_texture: &str, alpha_test: bool) -> Option<&RgbaImage> {
        self.tiles(base_texture, alpha_test, [1, 1])?.first()
    }

    /// Load a texture cut into a `grid` of tiles, row by row.
    ///
    /// The grid is part of the key, as the alpha-test flag is: the same file
    /// can legitimately be wanted at two layouts, and neither answer is a
    /// substitute for the other.
    pub fn tiles(
        &mut self,
        base_texture: &str,
        alpha_test: bool,
        grid: [u32; 2],
    ) -> Option<&[RgbaImage]> {
        let key = (
            base_texture.to_ascii_lowercase().replace('\\', "/"),
            alpha_test,
            grid[0].max(1),
            grid[1].max(1),
        );
        if !self.cache.contains_key(&key) {
            let decoded = self.load(&key.0, alpha_test, [key.2, key.3]);
            self.cache.insert(key.clone(), decoded);
        }
        self.cache.get(&key)?.as_deref()
    }

    fn load(&self, key: &str, alpha_test: bool, grid: [u32; 2]) -> Option<Vec<RgbaImage>> {
        let data = self.vfs.open(&path_of(key))?;
        decode_tiles(&data, self.size, alpha_test, grid).ok()
    }

    /// How many distinct textures have been decoded successfully.
    pub fn decoded(&self) -> usize {
        self.cache.values().filter(|v| v.is_some()).count()
    }

    /// What a texture's header says, without decoding its pixels.
    ///
    /// Reading a header is far cheaper than decompressing a 1024x1024 DXT
    /// image, and it answers both questions the palette asks before it knows
    /// whether it wants the pixels at all: what colour the surface averages
    /// to, and how many texels there are to divide between blocks.
    pub fn header(&mut self, base_texture: &str) -> Option<Header> {
        let key = base_texture.to_ascii_lowercase().replace('\\', "/");
        if let Some(cached) = self.headers.get(&key) {
            return *cached;
        }
        let header = self.read_header(&key);
        self.headers.insert(key, header);
        header
    }

    fn read_header(&self, key: &str) -> Option<Header> {
        let data = self.vfs.open(&path_of(key))?;
        let header = vtf::from_bytes(&data).ok()?.header;
        Some(Header {
            reflectivity: header.reflectivity.map(f64::from),
            size: [u32::from(header.width), u32::from(header.height)],
        })
    }
}

fn path_of(key: &str) -> String {
    format!("materials/{}.vtf", key.trim_end_matches(".vtf"))
}

/// Encode an image as PNG bytes.
pub fn to_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .context("encoding PNG")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn vfs() -> Option<Vfs> {
        let map = Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_trainstation_02.bsp",
        );
        map.exists().then(|| Vfs::for_map(map, &[]))
    }

    #[test]
    fn decodes_a_real_texture_to_the_requested_size() {
        let Some(vfs) = vfs() else { return };
        let data = vfs
            .open("materials/concrete/concretewall001a.vtf")
            .expect("texture should be in the VPKs");

        let image = decode(&data, 16, false).unwrap();
        assert_eq!(image.dimensions(), (16, 16));

        // Concrete is an opaque mid-grey; a decode that silently produced
        // black or transparent pixels would still be the right size.
        let opaque = image.pixels().filter(|p| p.0[3] > 200).count();
        assert_eq!(opaque, 256, "every pixel of concrete should be opaque");
        let mean: u32 = image.pixels().map(|p| p.0[0] as u32).sum::<u32>() / 256;
        assert!((60..200).contains(&mean), "mean red channel {mean} is not a grey");
    }

    /// Reassembling the tiles has to give back the picture. This is the test
    /// that catches the mistakes that would be invisible in a diff and
    /// obvious on a wall: tiles emitted column-major, rows counted from the
    /// bottom, or a crop off by one tile.
    #[test]
    fn tiles_reassemble_into_the_original_texture() {
        let Some(vfs) = vfs() else { return };
        let data = vfs
            .open("materials/concrete/concretewall001a.vtf")
            .expect("texture should be in the VPKs");

        let grid = [4u32, 4u32];
        let tiles = decode_tiles(&data, 16, false, grid).unwrap();
        assert_eq!(tiles.len(), 16);

        // Lay the tiles back out, and compare against downsampling the whole
        // texture to the same size in one step.
        let (w, h) = (16 * grid[0], 16 * grid[1]);
        let mut assembled = RgbaImage::new(w, h);
        for (index, tile) in tiles.iter().enumerate() {
            let (column, row) = (index as u32 % grid[0], index as u32 / grid[0]);
            for (x, y, pixel) in tile.enumerate_pixels() {
                assembled.put_pixel(column * 16 + x, row * 16 + y, *pixel);
            }
        }

        let whole = vtf::from_bytes(&data).unwrap().highres_image.decode(0).unwrap();
        let reference = whole.resize_exact(w, h, FilterType::Lanczos3).to_rgba8();

        let error: f64 = assembled
            .pixels()
            .zip(reference.pixels())
            .map(|(a, b)| {
                (0..3).map(|c| (a.0[c] as f64 - b.0[c] as f64).abs()).sum::<f64>() / 3.0
            })
            .sum::<f64>()
            / (w * h) as f64;

        // Filtering differs a little at the tile seams, so this is not exact;
        // any ordering or orientation mistake is worth tens of levels, not
        // ones.
        assert!(error < 8.0, "reassembled tiles differ from the whole by {error:.1}/255");
    }

    /// The tiles have to be different from each other, or splitting bought
    /// nothing but registrations.
    #[test]
    fn tiles_of_a_detailed_texture_differ() {
        let Some(vfs) = vfs() else { return };
        let Some(data) = vfs.open("materials/brick/brickwall017a.vtf") else { return };

        let tiles = decode_tiles(&data, 16, false, [4, 4]).unwrap();
        let mut raw: Vec<&Vec<u8>> = tiles.iter().map(|t| t.as_raw()).collect();
        raw.sort();
        raw.dedup();
        assert!(raw.len() > 8, "only {} of 16 tiles are distinct", raw.len());
    }

    /// A 1x1 grid is the whole texture, so it has to match [`decode`] exactly.
    #[test]
    fn a_single_tile_is_the_whole_texture() {
        let Some(vfs) = vfs() else { return };
        let Some(data) = vfs.open("materials/concrete/concretewall001a.vtf") else { return };
        let tiles = decode_tiles(&data, 16, false, [1, 1]).unwrap();
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].as_raw(), decode(&data, 16, false).unwrap().as_raw());
    }

    /// A grate is a third see-through at full resolution. Averaging alpha on
    /// the way down to 16x16 loses that entirely, so the cutout is restored
    /// and its coverage should still be in the right ballpark.
    #[test]
    fn an_alpha_tested_texture_keeps_its_holes() {
        let Some(vfs) = vfs() else { return };
        let Some(data) = vfs.open("materials/metal/metalgrate011a.vtf") else { return };

        let averaged = decode(&data, 16, false).unwrap();
        assert_eq!(
            averaged.pixels().filter(|p| p.0[3] < 128).count(),
            0,
            "averaging alpha is expected to close the grate up; that is the bug"
        );

        let cutout = decode(&data, 16, true).unwrap();
        let holes = cutout.pixels().filter(|p| p.0[3] == 0).count();
        assert!(holes > 20, "only {holes} of 256 texels see-through");
        assert!(holes < 236, "{holes} of 256 texels see-through, grate vanished");
        // Alpha must be strictly on or off, or cutout rendering is a lottery.
        assert!(cutout.pixels().all(|p| p.0[3] == 0 || p.0[3] == 255));
    }

    /// A fully opaque texture must not be punched full of holes by the
    /// coverage pass.
    #[test]
    fn thresholding_leaves_an_opaque_texture_alone() {
        let Some(vfs) = vfs() else { return };
        let data = vfs.open("materials/concrete/concretewall001a.vtf").unwrap();
        let image = decode(&data, 16, true).unwrap();
        assert!(image.pixels().all(|p| p.0[3] == 255), "opaque concrete lost pixels");
    }

    #[test]
    fn other_sizes_work_too() {
        let Some(vfs) = vfs() else { return };
        let data = vfs.open("materials/concrete/concretewall001a.vtf").unwrap();
        assert_eq!(decode(&data, 32, false).unwrap().dimensions(), (32, 32));
        assert_eq!(decode(&data, 64, false).unwrap().dimensions(), (64, 64));
    }

    #[test]
    fn garbage_input_is_an_error_rather_than_a_panic() {
        assert!(decode(&[], 16, false).is_err());
        assert!(decode(b"not a vtf at all, not even close", 16, false).is_err());
    }

    /// A texture shared by many materials must only be decoded once.
    #[test]
    fn the_cache_decodes_each_texture_once() {
        let Some(vfs) = vfs() else { return };
        let mut textures = Textures::new(&vfs, 16);

        assert!(textures.get("Concrete/concretewall001a", false).is_some());
        assert!(textures.get("concrete/CONCRETEWALL001A", false).is_some());
        assert_eq!(textures.decoded(), 1, "case differences should hit the cache");

        assert!(textures.get("nothing/at/all", false).is_none());
        // A miss is remembered too, so it is not re-searched per material.
        assert!(textures.get("nothing/at/all", false).is_none());
        assert_eq!(textures.decoded(), 1);
    }

    #[test]
    fn images_encode_to_png() {
        let image = RgbaImage::from_pixel(16, 16, image::Rgba([12, 34, 56, 255]));
        let png = to_png(&image).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(png.len() > 8);
    }
}
