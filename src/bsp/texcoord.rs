//! Where a texture sits on a surface.
//!
//! A Source face does not store UVs. It stores two 4-vectors that project a
//! world position straight into texel coordinates: `s = p·u + u.w`, and the
//! same for `t`. The length of the projection vector is therefore texels per
//! world unit, which is exactly the number needed to answer "how many
//! Minecraft blocks wide is this texture?".
//!
//! That question is the whole point of the module. A 512-pixel concrete
//! texture at Hammer's default scale of 0.25 covers 2048 units of wall, which
//! at 16 units per block is eight blocks. Squeezing all 512 pixels onto one
//! block face is what makes a converted wall look like a smear; cut into
//! eight-by-eight, each block gets the piece of the texture that really is in
//! front of it, and the bricks line up across the wall again.

use crate::geom::Vec3;

/// The affine map from a world position to texel coordinates on one face.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexCoord {
    /// `s = p·u.xyz + u.w`, in texels.
    pub u: [f64; 4],
    pub v: [f64; 4],
}

impl TexCoord {
    pub fn of(info: &vbsp::TextureInfo) -> TexCoord {
        TexCoord {
            u: info.texture_transforms_u.map(f64::from),
            v: info.texture_transforms_v.map(f64::from),
        }
    }

    pub fn s(&self, p: Vec3) -> f64 {
        self.u[0] * p.x + self.u[1] * p.y + self.u[2] * p.z + self.u[3]
    }

    pub fn t(&self, p: Vec3) -> f64 {
        self.v[0] * p.x + self.v[1] * p.y + self.v[2] * p.z + self.v[3]
    }

    /// Texels per Source unit along `s` and `t`.
    ///
    /// The inverse of Hammer's texture scale: 0.25 there is 4 here.
    pub fn texels_per_unit(&self) -> [f64; 2] {
        [
            Vec3::new(self.u[0], self.u[1], self.u[2]).length(),
            Vec3::new(self.v[0], self.v[1], self.v[2]).length(),
        ]
    }
}

/// How a material's texture is laid out on the surfaces that wear it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialScale {
    /// Texture size in texels, as the compiler recorded it.
    pub size: [u32; 2],
    /// Texels per Source unit, typical across the faces using this material.
    pub texels_per_unit: [f64; 2],
}

impl MaterialScale {
    /// How many Minecraft blocks one repeat of the texture covers.
    pub fn blocks_spanned(&self, units_per_block: f64) -> [f64; 2] {
        std::array::from_fn(|axis| {
            let texels_per_block = self.texels_per_unit[axis] * units_per_block;
            if texels_per_block <= 0.0 {
                1.0
            } else {
                self.size[axis] as f64 / texels_per_block
            }
        })
    }

    /// How to cut this material's texture up, given a cap on the tiles it may
    /// have along each axis.
    ///
    /// The rule that matters is that **one tile is one block**, always. It is
    /// tempting to meet the cap by letting each tile cover several blocks
    /// instead, and that is exactly wrong: a cliff blend texture spanning 77
    /// blocks then becomes eight tiles of nearly ten blocks each, and the
    /// wall comes out as flat 10x10 patches of identical stone with a hard
    /// seam between them. That reads far worse than the smear it replaced,
    /// because the eye finds the grid instantly.
    ///
    /// So the cap shrinks the *window* into the texture rather than the
    /// resolution: past it, only the first `max` blocks' worth of texels is
    /// used, and that window repeats. Detail per block stays exactly right and
    /// what is lost is the part of the texture that never repeats — which, for
    /// the ground and cliff blends that are the only things scaled this far, is
    /// more of the same rock.
    pub fn split(&self, units_per_block: f64, max: u32, texture_size: u32) -> Split {
        let max = max.max(1);
        let spanned = self.blocks_spanned(units_per_block);

        // Round to whole blocks so the tile grid lines up with the texture's
        // own repeat, rather than drifting a fraction of a block per tile.
        let blocks: [u32; 2] =
            std::array::from_fn(|axis| (spanned[axis].round() as i64).clamp(1, i64::from(u32::MAX)) as u32);
        // And never finer than the source can feed. Below one source texel per
        // output pixel a tile is upscaled mush: cutting a 512-pixel texture
        // 128 ways leaves four texels to fill a 16x16 block face. Past this
        // point splitting further invents detail rather than recovering it,
        // and it is the reason raising the cap is safe — quality stops
        // costing blocks exactly when it stops improving.
        let resolution: [u32; 2] =
            std::array::from_fn(|axis| (self.size[axis] / texture_size.max(1)).max(1));
        let grid: [u32; 2] =
            std::array::from_fn(|axis| blocks[axis].min(max).min(resolution[axis]));
        let texels_per_tile: [f64; 2] =
            std::array::from_fn(|axis| (self.size[axis] as f64 / blocks[axis] as f64).max(1.0));
        let window: [u32; 2] = std::array::from_fn(|axis| {
            ((texels_per_tile[axis] * grid[axis] as f64).round() as u32)
                .clamp(1, self.size[axis].max(1))
        });

        Split { grid, texels_per_tile, window }
    }
}

/// How a material's texture is cut into blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    /// Tiles across and down, and so blocks before the pattern repeats.
    pub grid: [u32; 2],
    /// Texels one tile covers, which is one Minecraft block of surface.
    pub texels_per_tile: [f64; 2],
    /// Texels of the texture the whole grid uses. Equal to the texture size
    /// unless the cap cut the window short.
    pub window: [u32; 2],
}

impl Split {
    pub fn tiles(&self) -> u32 {
        self.grid[0] * self.grid[1]
    }

    /// Whether the texture is used whole, or only a window into it.
    pub fn is_whole(&self, size: [u32; 2]) -> bool {
        self.window == size
    }
}

/// The typical scale of every material in the map, indexed as
/// [`crate::bsp::Map::materials`] is.
///
/// A material is used at more than one scale — a wall texture reused on a
/// narrow trim gets stretched — so the median across the faces using it is
/// taken rather than the first. One block layout has to serve them all, and
/// the median is the one that is wrong by the least on the most surfaces.
pub fn material_scales(map: &crate::bsp::Map) -> Vec<Option<MaterialScale>> {
    let mut samples: Vec<Vec<[f64; 2]>> = vec![Vec::new(); map.materials().len()];

    for info in &map.bsp.textures_info {
        let Ok(index) = usize::try_from(info.texture_data_index) else { continue };
        let Some(bucket) = samples.get_mut(index) else { continue };
        let rate = TexCoord::of(info).texels_per_unit();
        if rate[0] > 0.0 && rate[1] > 0.0 && rate[0].is_finite() && rate[1].is_finite() {
            bucket.push(rate);
        }
    }

    samples
        .into_iter()
        .enumerate()
        .map(|(index, mut rates)| {
            let data = map.bsp.textures_data.get(index)?;
            let size = [u32::try_from(data.width).ok()?, u32::try_from(data.height).ok()?];
            if size[0] == 0 || size[1] == 0 || rates.is_empty() {
                return None;
            }
            Some(MaterialScale { size, texels_per_unit: median(&mut rates) })
        })
        .collect()
}

fn median(rates: &mut [[f64; 2]]) -> [f64; 2] {
    std::array::from_fn(|axis| {
        rates.sort_by(|a, b| a[axis].partial_cmp(&b[axis]).unwrap_or(std::cmp::Ordering::Equal));
        rates[rates.len() / 2][axis]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bsp::Map;
    use std::path::Path;

    fn coord(u: [f64; 4], v: [f64; 4]) -> TexCoord {
        TexCoord { u, v }
    }

    /// Hammer's texture scale is the reciprocal of what the BSP stores, and
    /// getting that backwards would tile every wall 256 times over.
    #[test]
    fn texel_rate_is_the_reciprocal_of_hammer_scale() {
        // A wall at the default 0.25 scale: four texels per unit.
        let c = coord([4.0, 0.0, 0.0, 0.0], [0.0, 0.0, -4.0, 0.0]);
        assert_eq!(c.texels_per_unit(), [4.0, 4.0]);

        let half = coord([2.0, 0.0, 0.0, 0.0], [0.0, 0.0, -2.0, 0.0]);
        assert_eq!(half.texels_per_unit(), [2.0, 2.0]);
    }

    /// The projection is affine, so moving along the surface has to move the
    /// texel coordinate by exactly the rate.
    #[test]
    fn texels_advance_with_position() {
        let c = coord([4.0, 0.0, 0.0, 100.0], [0.0, 0.0, -4.0, 7.0]);
        assert_eq!(c.s(Vec3::ZERO), 100.0);
        assert_eq!(c.s(Vec3::new(1.0, 0.0, 0.0)), 104.0);
        assert_eq!(c.t(Vec3::ZERO), 7.0);
        // Source's V axis points down a wall, which is why it is negated.
        assert_eq!(c.t(Vec3::new(0.0, 0.0, 1.0)), 3.0);
    }

    /// The number this all exists to produce: a 512 texture at scale 0.25 is
    /// eight Minecraft blocks of wall.
    #[test]
    fn a_default_scaled_texture_spans_eight_blocks() {
        let scale = MaterialScale { size: [512, 512], texels_per_unit: [4.0, 4.0] };
        assert_eq!(scale.blocks_spanned(16.0), [8.0, 8.0]);
        // Half the units per block, twice the blocks.
        assert_eq!(scale.blocks_spanned(8.0), [16.0, 16.0]);
    }

    /// The bug this exists to prevent. A cliff blend texture spans 77 blocks;
    /// capped at 8 tiles, each tile used to cover nearly ten blocks, and the
    /// wall came out as flat 10x10 patches of identical stone. A tile is one
    /// block, always — the cap shortens the window into the texture instead.
    #[test]
    fn a_tile_is_never_more_than_one_block() {
        for (size, rate) in [
            ([1024, 1024], 0.83), // nature/blendrockdirt008a: 77 blocks
            ([256, 256], 0.33),   // nature/water_coast01: 48 blocks
            ([512, 512], 1.0),    // 32 blocks
            ([512, 512], 2.0),    // 16 blocks
            ([512, 512], 4.0),    // 8 blocks, inside the cap
            ([2048, 512], 4.0),   // wider than tall
            ([64, 64], 16.0),     // a quarter of a block
        ] {
            let scale = MaterialScale { size, texels_per_unit: [rate, rate] };
            let split = scale.split(16.0, 8, 16);
            let spanned = scale.blocks_spanned(16.0);

            for axis in 0..2 {
                let blocks_per_tile =
                    split.texels_per_tile[axis] / (rate * 16.0);
                assert!(
                    blocks_per_tile < 1.5,
                    "{size:?} at {rate} texels/unit: one tile covers                      {blocks_per_tile:.1} blocks",
                );
                assert!(split.grid[axis] >= 1 && split.grid[axis] <= 8);
                assert!(split.window[axis] <= size[axis]);
                // The window is exactly the tiles it holds.
                let expected =
                    (split.texels_per_tile[axis] * split.grid[axis] as f64).round() as u32;
                assert_eq!(split.window[axis], expected.min(size[axis]));
            }
            let _ = spanned;
        }
    }

    /// A tile may not be cut finer than the source can feed it. Splitting a
    /// 512-pixel texture 128 ways leaves four texels to fill a 16x16 face,
    /// which is upscaled mush rather than recovered detail — and it is what
    /// makes raising the cap safe, since cost stops rising exactly where
    /// quality stops improving.
    #[test]
    fn a_texture_is_never_cut_finer_than_its_own_resolution() {
        for (size, out, most) in [
            ([512u32, 512], 16u32, 32u32),
            ([1024, 1024], 16, 64),
            ([256, 256], 16, 16),
            ([128, 128], 16, 8),
            ([512, 512], 32, 16),
        ] {
            // A rate stretched far enough that the span alone would allow far
            // more tiles than the texture has detail for.
            let scale = MaterialScale { size, texels_per_unit: [0.05, 0.05] };
            let split = scale.split(16.0, 1024, out);
            assert_eq!(
                split.grid,
                [most, most],
                "{size:?} at {out}px output should stop at {most} tiles"
            );
            // A tile always carries at least one source texel. It may still
            // be magnified — a texture stretched over more blocks than it has
            // `out`-sized pieces cannot do better, and neither does Source —
            // but past this point extra tiles only re-cut the same texels.
            for axis in 0..2 {
                assert!(split.texels_per_tile[axis] >= 1.0);
            }
        }
    }

    /// The limit must not disturb the ordinary case, where the span runs out
    /// long before the resolution does.
    #[test]
    fn the_resolution_limit_leaves_normal_textures_alone() {
        let scale = MaterialScale { size: [512, 512], texels_per_unit: [4.0, 4.0] };
        assert_eq!(scale.split(16.0, 16, 16).grid, [8, 8]);
        assert_eq!(scale.split(16.0, 64, 16).grid, [8, 8]);
    }

    /// Under the cap nothing is windowed: the whole texture is used, which is
    /// the case that already looked right and must not regress.
    #[test]
    fn a_texture_that_fits_the_cap_is_used_whole() {
        let scale = MaterialScale { size: [512, 512], texels_per_unit: [4.0, 4.0] };
        let split = scale.split(16.0, 8, 16);
        assert_eq!(split.grid, [8, 8]);
        assert_eq!(split.texels_per_tile, [64.0, 64.0]);
        assert!(split.is_whole(scale.size), "window {:?}", split.window);
    }

    /// Over the cap the window shrinks in proportion, so the pattern repeats
    /// every `max` blocks instead of stretching.
    #[test]
    fn a_texture_over_the_cap_uses_a_window_of_itself() {
        // 32 blocks of wall from a 512 texture: one block is 16 texels.
        let scale = MaterialScale { size: [512, 512], texels_per_unit: [1.0, 1.0] };
        let split = scale.split(16.0, 8, 16);
        assert_eq!(split.grid, [8, 8]);
        assert_eq!(split.texels_per_tile, [16.0, 16.0]);
        assert_eq!(split.window, [128, 128], "only a quarter of the texture is used");
        assert!(!split.is_whole(scale.size));

        // Raising the cap widens the window without changing the tile size.
        let wider = scale.split(16.0, 32, 16);
        assert_eq!(wider.texels_per_tile, split.texels_per_tile);
        assert_eq!(wider.window, [512, 512]);
    }

    /// A texture smaller than a block repeats several times within one, and
    /// must not ask for a fraction of a tile.
    #[test]
    fn a_texture_smaller_than_a_block_is_a_single_tile() {
        // A quarter of a block: rounding the span would give zero tiles.
        let scale = MaterialScale { size: [64, 64], texels_per_unit: [16.0, 16.0] };
        assert_eq!(scale.blocks_spanned(16.0), [0.25, 0.25]);

        let split = scale.split(16.0, 8, 16);
        assert_eq!(split.grid, [1, 1]);
        assert!(split.texels_per_tile[0] >= 1.0);
        assert!(split.is_whole(scale.size));
    }

    #[test]
    fn a_degenerate_rate_spans_one_block_rather_than_dividing_by_zero() {
        let scale = MaterialScale { size: [512, 512], texels_per_unit: [0.0, 0.0] };
        assert_eq!(scale.blocks_spanned(16.0), [1.0, 1.0]);
    }

    #[test]
    fn the_median_ignores_an_outlier() {
        let mut rates = [[4.0, 4.0], [4.0, 4.0], [64.0, 64.0]];
        assert_eq!(median(&mut rates), [4.0, 4.0]);
    }

    fn sample_map() -> Option<Map> {
        let path = Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_trainstation_02.bsp",
        );
        path.exists().then(|| Map::load(path).unwrap())
    }

    /// Real maps must come out at plausible scales: Hammer's own grid means
    /// nearly everything is between an eighth of a texel and 16 texels per
    /// unit, and a stock map should sit around the 0.25 default.
    #[test]
    fn real_material_scales_are_plausible() {
        let Some(map) = sample_map() else { return };
        let scales = material_scales(&map);
        assert_eq!(scales.len(), map.materials().len());

        let found: Vec<MaterialScale> = scales.into_iter().flatten().collect();
        assert!(found.len() > 50, "only {} materials had a scale", found.len());

        let mut multi_block = 0;
        for scale in &found {
            for axis in 0..2 {
                assert!(
                    (0.01..=64.0).contains(&scale.texels_per_unit[axis]),
                    "implausible rate {:?}",
                    scale.texels_per_unit
                );
            }
            if scale.blocks_spanned(16.0).iter().any(|b| *b > 1.5) {
                multi_block += 1;
            }
        }
        assert!(
            multi_block * 2 > found.len(),
            "only {multi_block} of {} materials span more than one block, which \
             would mean there is nothing to gain from splitting them",
            found.len()
        );
    }
}
