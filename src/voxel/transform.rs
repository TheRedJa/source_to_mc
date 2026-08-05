//! Mapping Source world space onto Minecraft block space.
//!
//! Source is Z-up and right-handed: +X east, +Y north, +Z up. Minecraft is
//! Y-up with +X east and +Z *south*. Negating Y while moving Z to the vertical
//! axis preserves handedness, so the map is not mirrored:
//!
//! ```text
//! mc.x =  src.x / units_per_block
//! mc.y =  src.z / units_per_block
//! mc.z = -src.y / units_per_block
//! ```

use crate::config::{Config, OriginMode};
use crate::geom::{Aabb, Vec3};

#[derive(Debug, Clone)]
pub struct Transform {
    units_per_block: f64,
    yaw_sin: f64,
    yaw_cos: f64,
    /// Added after scaling, in block space.
    offset: Vec3,
}

impl Transform {
    /// Build the transform for a map with the given Source-space bounds.
    pub fn new(config: &Config, map_bounds: Aabb) -> Transform {
        let yaw = config.transform.rotate_yaw.to_radians();
        let mut transform = Transform {
            units_per_block: config.scale.units_per_block,
            yaw_sin: yaw.sin(),
            yaw_cos: yaw.cos(),
            offset: Vec3::ZERO,
        };

        if config.transform.origin_mode == OriginMode::BoundsMin && !map_bounds.is_empty() {
            // Rotation can move which corner is lowest, so measure the mapped
            // bounds rather than transforming the original minimum corner.
            let mapped = transform.transform_bounds(map_bounds);
            transform.offset = Vec3::new(
                -mapped.min.x,
                config.transform.y_base as f64 - mapped.min.y,
                -mapped.min.z,
            );
        }
        transform
    }

    /// Map a Source-space point to fractional Minecraft block coordinates.
    pub fn to_block_space(&self, src: Vec3) -> Vec3 {
        // Yaw rotates about Source's up axis, i.e. within the XY plane.
        let x = src.x * self.yaw_cos - src.y * self.yaw_sin;
        let y = src.x * self.yaw_sin + src.y * self.yaw_cos;
        Vec3::new(
            x / self.units_per_block,
            src.z / self.units_per_block,
            -y / self.units_per_block,
        ) + self.offset
    }

    /// Map a block-space point back to Source space.
    pub fn to_source_space(&self, block: Vec3) -> Vec3 {
        let p = block - self.offset;
        let x = p.x * self.units_per_block;
        let y = -p.z * self.units_per_block;
        Vec3::new(
            x * self.yaw_cos + y * self.yaw_sin,
            -x * self.yaw_sin + y * self.yaw_cos,
            p.y * self.units_per_block,
        )
    }

    /// Map a direction vector, ignoring translation.
    pub fn transform_direction(&self, dir: Vec3) -> Vec3 {
        (self.to_block_space(dir) - self.to_block_space(Vec3::ZERO)).normalized()
    }

    /// Map a plane into block space.
    ///
    /// The transform is a uniform scale, a rotation and a translation, so
    /// planes stay planes: rotate the normal, then re-measure the distance
    /// using any point known to lie on the plane.
    pub fn transform_plane(&self, plane: crate::geom::Plane) -> crate::geom::Plane {
        let normal = plane.normal.normalized();
        // `normal * dist` satisfies `n · p = dist` for a unit normal.
        let point_on_plane = normal * plane.dist;
        let mapped_normal = self.transform_direction(normal);
        crate::geom::Plane::new(
            mapped_normal,
            mapped_normal.dot(self.to_block_space(point_on_plane)),
        )
    }

    /// Map a Source-space box, covering all eight corners so rotation is
    /// handled correctly.
    pub fn transform_bounds(&self, bounds: Aabb) -> Aabb {
        if bounds.is_empty() {
            return bounds;
        }
        let mut out = Aabb::empty();
        for i in 0..8 {
            let corner = Vec3::new(
                if i & 1 == 0 { bounds.min.x } else { bounds.max.x },
                if i & 2 == 0 { bounds.min.y } else { bounds.max.y },
                if i & 4 == 0 { bounds.min.z } else { bounds.max.z },
            );
            out.extend(self.to_block_space(corner));
        }
        out
    }

    pub fn units_per_block(&self) -> f64 {
        self.units_per_block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(units: f64, origin_mode: OriginMode, yaw: f64) -> Config {
        let mut config = Config::default();
        config.scale.units_per_block = units;
        config.transform.origin_mode = origin_mode;
        config.transform.rotate_yaw = yaw;
        config
    }

    #[test]
    fn maps_axes_without_mirroring() {
        let config = config_with(16.0, OriginMode::MapOrigin, 0.0);
        let t = Transform::new(&config, Aabb::empty());

        // Source up becomes Minecraft up.
        let up = t.to_block_space(Vec3::new(0.0, 0.0, 16.0));
        assert_eq!((up.x, up.y, up.z), (0.0, 1.0, 0.0));
        // Source north (+Y) becomes Minecraft north (-Z).
        let north = t.to_block_space(Vec3::new(0.0, 16.0, 0.0));
        assert_eq!((north.x, north.y, north.z), (0.0, 0.0, -1.0));
        // Source east (+X) stays Minecraft east.
        let east = t.to_block_space(Vec3::new(16.0, 0.0, 0.0));
        assert_eq!((east.x, east.y, east.z), (1.0, 0.0, 0.0));
    }

    #[test]
    fn scale_matches_the_documented_player_height() {
        let config = config_with(16.0, OriginMode::MapOrigin, 0.0);
        let t = Transform::new(&config, Aabb::empty());
        let player = t.to_block_space(Vec3::new(0.0, 0.0, 72.0));
        assert!((player.y - 4.5).abs() < 1e-9, "72 units should be 4.5 blocks");
    }

    #[test]
    fn round_trips_through_source_space() {
        for yaw in [0.0, 90.0, 37.5] {
            let config = config_with(16.0, OriginMode::BoundsMin, yaw);
            let bounds = Aabb::new(Vec3::new(-512.0, -256.0, 0.0), Vec3::new(512.0, 256.0, 384.0));
            let t = Transform::new(&config, bounds);

            let src = Vec3::new(123.5, -64.25, 200.0);
            let back = t.to_source_space(t.to_block_space(src));
            assert!((back - src).length() < 1e-6, "yaw {yaw}: {back:?} != {src:?}");
        }
    }

    #[test]
    fn bounds_min_anchors_the_map_at_the_origin() {
        let mut config = config_with(16.0, OriginMode::BoundsMin, 0.0);
        config.transform.y_base = -64;
        let bounds = Aabb::new(Vec3::new(-1024.0, -2048.0, -512.0), Vec3::new(1024.0, 512.0, 1024.0));
        let t = Transform::new(&config, bounds);

        let mapped = t.transform_bounds(bounds);
        assert!(mapped.min.x.abs() < 1e-9, "{:?}", mapped.min);
        assert!(mapped.min.z.abs() < 1e-9, "{:?}", mapped.min);
        assert!((mapped.min.y - -64.0).abs() < 1e-9, "{:?}", mapped.min);
    }

    #[test]
    fn bounds_stay_positive_under_rotation() {
        let config = config_with(16.0, OriginMode::BoundsMin, 45.0);
        let bounds = Aabb::new(Vec3::new(-1024.0, -1024.0, 0.0), Vec3::new(1024.0, 1024.0, 256.0));
        let t = Transform::new(&config, bounds);
        let mapped = t.transform_bounds(bounds);
        assert!(mapped.min.x >= -1e-9 && mapped.min.z >= -1e-9, "{mapped:?}");
    }

    /// A transformed plane must classify transformed points exactly as the
    /// original classified the originals.
    #[test]
    fn transformed_planes_classify_points_consistently() {
        use crate::geom::Plane;

        for yaw in [0.0, 90.0, 33.0] {
            let config = config_with(16.0, OriginMode::BoundsMin, yaw);
            let bounds = Aabb::new(Vec3::new(-512.0, -512.0, 0.0), Vec3::new(512.0, 512.0, 256.0));
            let t = Transform::new(&config, bounds);

            let plane = Plane::new(Vec3::new(1.0, 2.0, 3.0).normalized(), 64.0);
            let mapped = t.transform_plane(plane);

            for p in [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(100.0, -200.0, 50.0),
                Vec3::new(-480.0, 320.0, 240.0),
                plane.normal * plane.dist,
            ] {
                let before = plane.distance_to(p);
                let after = mapped.distance_to(t.to_block_space(p));
                assert_eq!(
                    before < 0.0,
                    after < 0.0,
                    "yaw {yaw}, point {p:?}: {before} vs {after}"
                );
                // Distances shrink by exactly the scale factor.
                assert!(
                    (after * 16.0 - before).abs() < 1e-6,
                    "yaw {yaw}: {after} * 16 != {before}"
                );
            }
        }
    }

    #[test]
    fn transformed_normals_stay_unit_length() {
        let config = config_with(16.0, OriginMode::BoundsMin, 17.0);
        let t = Transform::new(&config, Aabb::new(Vec3::ZERO, Vec3::splat(1024.0)));
        let mapped = t.transform_direction(Vec3::new(3.0, -4.0, 12.0));
        assert!((mapped.length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn scale_changes_the_mapped_size() {
        let bounds = Aabb::new(Vec3::ZERO, Vec3::new(1024.0, 1024.0, 1024.0));
        for units in [8.0, 16.0, 32.0] {
            let config = config_with(units, OriginMode::BoundsMin, 0.0);
            let t = Transform::new(&config, bounds);
            let size = t.transform_bounds(bounds).size();
            assert!((size.x - 1024.0 / units).abs() < 1e-9, "units {units}");
        }
    }
}
