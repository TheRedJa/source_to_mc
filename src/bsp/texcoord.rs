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
