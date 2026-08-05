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
    let vtf = vtf::from_bytes(data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let image: DynamicImage = vtf
        .highres_image
        .decode(0)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if image.width() == 0 || image.height() == 0 {
        bail!("texture has no pixels");
    }

    let coverage = alpha_test.then(|| {
        let pixels = image.to_rgba8();
        let solid = pixels.pixels().filter(|p| p.0[3] >= ALPHA_CUTOFF).count();
        solid as f64 / pixels.pixels().len().max(1) as f64
    });

    let mut resized = resize(image, size);
    if let Some(coverage) = coverage {
        binarize_alpha(&mut resized, coverage);
    }
    Ok(resized)
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

/// Decodes textures on demand, keeping each one only once.
pub struct Textures<'a> {
    vfs: &'a Vfs,
    size: u32,
    /// `None` records a texture we already failed to find, so a missing file
    /// is not searched for once per material that references it.
    cache: HashMap<(String, bool), Option<RgbaImage>>,
}

impl<'a> Textures<'a> {
    pub fn new(vfs: &'a Vfs, size: u32) -> Textures<'a> {
        Textures { vfs, size, cache: HashMap::new() }
    }

    /// Load `$basetexture`, e.g. `Concrete/concretewall001a`.
    ///
    /// The alpha-test flag is part of the key because it changes how the
    /// texture is downsampled, so the same file can legitimately be wanted
    /// both ways.
    pub fn get(&mut self, base_texture: &str, alpha_test: bool) -> Option<&RgbaImage> {
        let key = (base_texture.to_ascii_lowercase().replace('\\', "/"), alpha_test);
        if !self.cache.contains_key(&key) {
            let decoded = self.load(&key.0, alpha_test);
            self.cache.insert(key.clone(), decoded);
        }
        self.cache.get(&key)?.as_ref()
    }

    fn load(&self, key: &str, alpha_test: bool) -> Option<RgbaImage> {
        let path = format!("materials/{}.vtf", key.trim_end_matches(".vtf"));
        let data = self.vfs.open(&path)?;
        decode(&data, self.size, alpha_test).ok()
    }

    /// How many distinct textures have been decoded successfully.
    pub fn decoded(&self) -> usize {
        self.cache.values().filter(|v| v.is_some()).count()
    }
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
