//! Voxelizing a convex brush.
//!
//! A Source brush is an intersection of half-spaces, so testing whether a point
//! is inside it is just a sign check against every side. Working in block space
//! means a voxel is the unit cube `[x, x+1)³`.

use crate::config::{SampleMode, Voxelize};
use crate::geom::{Aabb, Plane, Vec3};
use crate::voxel::grid::IVec3;

/// A brush prepared for voxelization: planes and bounds already in block space.
#[derive(Debug, Clone)]
pub struct BlockSolid {
    pub planes: Vec<Plane>,
    pub bounds: Aabb,
    /// Index of the originating side for each plane, so a filled voxel can be
    /// traced back to the material on that face.
    pub side_of_plane: Vec<usize>,
}

impl BlockSolid {
    pub fn contains(&self, point: Vec3) -> bool {
        self.planes.iter().all(|p| p.distance_to(point) <= 0.0)
    }

    /// Index of the side whose plane the point sits closest to.
    ///
    /// Inside the brush every distance is negative, so the *largest* one is the
    /// nearest surface. That side's material is what the voxel should wear.
    pub fn nearest_side(&self, point: Vec3) -> Option<usize> {
        self.planes
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.distance_to(point)
                    .partial_cmp(&b.distance_to(point))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| self.side_of_plane[i])
    }
}

/// Half the diagonal of a unit cube: the furthest any point of a voxel can be
/// from its centre.
const VOXEL_HALF_DIAGONAL: f64 = 0.866_025_403_784_438_6;

/// Occupied fraction of the voxel at `pos`, in `[0, 1]`.
fn occupancy(solid: &BlockSolid, pos: IVec3, config: &Voxelize) -> f64 {
    let corner = Vec3::new(pos[0] as f64, pos[1] as f64, pos[2] as f64);
    let centre = corner + Vec3::splat(0.5);

    // Voxels comfortably clear of every boundary are wholly in or wholly out,
    // so the sample grid cannot tell us anything new. Levels are sealed with
    // very large brushes whose interiors are almost entirely this case, which
    // makes the shortcut worth far more than it costs.
    let depth = depth(solid, centre);
    if depth <= -VOXEL_HALF_DIAGONAL {
        return 1.0;
    }
    if depth >= VOXEL_HALF_DIAGONAL {
        return 0.0;
    }

    match config.mode {
        SampleMode::Center => {
            if solid.contains(centre) {
                1.0
            } else {
                0.0
            }
        }
        // `exact` is not implemented yet and falls back to dense sampling,
        // which converges to the same answer as `samples` rises.
        SampleMode::Samples | SampleMode::Exact => {
            let n = config.samples.max(1);
            let step = 1.0 / n as f64;
            let mut inside = 0u32;
            for i in 0..n {
                for j in 0..n {
                    for k in 0..n {
                        let p = corner
                            + Vec3::new(
                                (i as f64 + 0.5) * step,
                                (j as f64 + 0.5) * step,
                                (k as f64 + 0.5) * step,
                            );
                        if solid.contains(p) {
                            inside += 1;
                        }
                    }
                }
            }
            inside as f64 / (n * n * n) as f64
        }
    }
}

/// Signed distance from `point` to the nearest bounding plane. Negative inside,
/// and the closer to zero the nearer the surface.
fn depth(solid: &BlockSolid, point: Vec3) -> f64 {
    solid
        .planes
        .iter()
        .map(|p| p.distance_to(point))
        .fold(f64::NEG_INFINITY, f64::max)
}

/// Call `emit` for every voxel the brush fills, passing the voxel position and
/// the index of the side whose material it should take.
///
/// Brushes are voxelized solid; hollowing happens afterwards over the union of
/// all of them. Culling each brush's interior here instead is tempting but
/// wrong: it turns the removed volume into apparent air, so the hollowing pass
/// then preserves a spurious second shell around it, and it cannot see that a
/// voxel on one brush's surface may be buried inside another brush.
pub fn voxelize(solid: &BlockSolid, config: &Voxelize, mut emit: impl FnMut(IVec3, Option<usize>)) {
    if solid.bounds.is_empty() || solid.planes.is_empty() {
        return;
    }

    let min = [
        solid.bounds.min.x.floor() as i64,
        solid.bounds.min.y.floor() as i64,
        solid.bounds.min.z.floor() as i64,
    ];
    let max = [
        solid.bounds.max.x.ceil() as i64,
        solid.bounds.max.y.ceil() as i64,
        solid.bounds.max.z.ceil() as i64,
    ];

    // Brushes thinner than a block would vanish under a plain threshold test.
    // Detect that per axis so those voxels can be kept on a lower bar.
    let thin_axis = if config.preserve_thin {
        let size = solid.bounds.size();
        (0..3).find(|axis| size.axis(*axis) < 1.0)
    } else {
        None
    };

    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                let pos = [x as i32, y as i32, z as i32];
                let fraction = occupancy(solid, pos, config);

                let filled = if fraction >= config.fill_threshold {
                    true
                } else {
                    // A sliver of a thin brush still counts: without this,
                    // E:Z's 4- and 8-unit trim disappears at 16 units/block.
                    thin_axis.is_some() && fraction > 0.0
                };

                if filled {
                    let centre = Vec3::new(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5);
                    emit(pos, solid.nearest_side(centre));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Axis-aligned box as outward-facing half-spaces, already in block space.
    fn block_box(min: Vec3, max: Vec3) -> BlockSolid {
        let planes = vec![
            Plane::new(Vec3::new(-1.0, 0.0, 0.0), -min.x),
            Plane::new(Vec3::new(1.0, 0.0, 0.0), max.x),
            Plane::new(Vec3::new(0.0, -1.0, 0.0), -min.y),
            Plane::new(Vec3::new(0.0, 1.0, 0.0), max.y),
            Plane::new(Vec3::new(0.0, 0.0, -1.0), -min.z),
            Plane::new(Vec3::new(0.0, 0.0, 1.0), max.z),
        ];
        BlockSolid {
            side_of_plane: (0..planes.len()).collect(),
            planes,
            bounds: Aabb::new(min, max),
        }
    }

    fn voxels(solid: &BlockSolid, config: &Voxelize) -> Vec<IVec3> {
        let mut out = Vec::new();
        voxelize(solid, config, |pos, _| out.push(pos));
        out.sort();
        out
    }

    #[test]
    fn grid_aligned_cube_fills_exactly_its_volume() {
        let solid = block_box(Vec3::ZERO, Vec3::new(4.0, 4.0, 4.0));
        for mode in [SampleMode::Center, SampleMode::Samples] {
            let config = Voxelize { mode, ..Voxelize::default() };
            assert_eq!(voxels(&solid, &config).len(), 4 * 4 * 4, "mode {mode:?}");
        }
    }

    #[test]
    fn offset_cube_lands_at_the_right_coordinates() {
        let solid = block_box(Vec3::new(-2.0, 5.0, 10.0), Vec3::new(0.0, 6.0, 12.0));
        let config = Voxelize::default();
        let found = voxels(&solid, &config);
        assert_eq!(found, vec![[-2, 5, 10], [-2, 5, 11], [-1, 5, 10], [-1, 5, 11]]);
    }

    #[test]
    fn half_filled_voxels_follow_the_threshold() {
        // A slab occupying the lower half of one voxel layer.
        let solid = block_box(Vec3::ZERO, Vec3::new(1.0, 0.5, 1.0));

        let permissive = Voxelize {
            mode: SampleMode::Samples,
            samples: 4,
            fill_threshold: 0.5,
            preserve_thin: false,
        };
        assert_eq!(voxels(&solid, &permissive).len(), 1);

        let strict = Voxelize { fill_threshold: 0.9, ..permissive };
        assert_eq!(voxels(&solid, &strict).len(), 0);
    }

    #[test]
    fn thin_brushes_survive_when_preservation_is_on() {
        // An 8-unit wall at 16 units/block is half a block thick; a quarter of
        // a block stands in for E:Z's 4-unit trim.
        let solid = block_box(Vec3::ZERO, Vec3::new(0.25, 3.0, 3.0));

        let dropped = Voxelize {
            mode: SampleMode::Samples,
            samples: 4,
            fill_threshold: 0.5,
            preserve_thin: false,
        };
        assert!(voxels(&solid, &dropped).is_empty(), "thin wall should fail the threshold");

        let kept = Voxelize { preserve_thin: true, ..dropped };
        assert_eq!(voxels(&solid, &kept).len(), 9, "thin wall should be preserved");
    }

    /// The whole-cube shortcut must agree with brute-force sampling, including
    /// on brushes deliberately offset off the grid and cut at odd angles.
    #[test]
    fn the_inside_outside_shortcut_matches_brute_force_sampling() {
        let mut solid = block_box(Vec3::new(-3.3, 0.7, 1.15), Vec3::new(6.8, 9.25, 5.4));
        solid.planes.push(Plane::new(Vec3::new(1.0, 2.0, 3.0).normalized(), 6.0));
        solid.side_of_plane.push(6);

        let config = Voxelize {
            mode: SampleMode::Samples,
            samples: 4,
            fill_threshold: 0.5,
            preserve_thin: false,
        };

        // Recompute occupancy the slow way and compare, cube by cube.
        for x in -6..10 {
            for y in -2..12 {
                for z in -2..8 {
                    let pos = [x, y, z];
                    let corner = Vec3::new(x as f64, y as f64, z as f64);
                    let n = config.samples;
                    let step = 1.0 / n as f64;
                    let mut inside = 0;
                    for i in 0..n {
                        for j in 0..n {
                            for k in 0..n {
                                let p = corner
                                    + Vec3::new(
                                        (i as f64 + 0.5) * step,
                                        (j as f64 + 0.5) * step,
                                        (k as f64 + 0.5) * step,
                                    );
                                if solid.contains(p) {
                                    inside += 1;
                                }
                            }
                        }
                    }
                    let brute = inside as f64 / (n * n * n) as f64;
                    let fast = occupancy(&solid, pos, &config);
                    assert!(
                        (fast - brute).abs() < 1e-12,
                        "at {pos:?}: shortcut {fast} vs brute force {brute}"
                    );
                }
            }
        }
    }

    #[test]
    fn deep_interior_voxels_are_fully_occupied() {
        let solid = block_box(Vec3::ZERO, Vec3::new(20.0, 20.0, 20.0));
        let config = Voxelize::default();
        assert_eq!(occupancy(&solid, [10, 10, 10], &config), 1.0);
        assert_eq!(occupancy(&solid, [100, 100, 100], &config), 0.0);
    }

    #[test]
    fn nearest_side_picks_the_closest_face() {
        // Sides are ordered -x, +x, -y, +y, -z, +z.
        let solid = block_box(Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0));
        assert_eq!(solid.nearest_side(Vec3::new(0.1, 5.0, 5.0)), Some(0));
        assert_eq!(solid.nearest_side(Vec3::new(9.9, 5.0, 5.0)), Some(1));
        assert_eq!(solid.nearest_side(Vec3::new(5.0, 5.0, 9.9)), Some(5));
    }

    #[test]
    fn degenerate_solids_emit_nothing() {
        let empty = BlockSolid {
            planes: Vec::new(),
            bounds: Aabb::empty(),
            side_of_plane: Vec::new(),
        };
        assert!(voxels(&empty, &Voxelize::default()).is_empty());
    }

    #[test]
    fn a_wedge_fills_about_half_of_its_bounding_box() {
        let mut solid = block_box(Vec3::ZERO, Vec3::new(8.0, 8.0, 2.0));
        // Diagonal cut through the x/y square.
        solid.planes.push(Plane::new(Vec3::new(1.0, 1.0, 0.0).normalized(), 8.0 / 2f64.sqrt()));
        solid.side_of_plane.push(6);

        let config = Voxelize { preserve_thin: false, ..Voxelize::default() };
        let count = voxels(&solid, &config).len();
        let full = 8 * 8 * 2;
        assert!(
            (count as f64) > full as f64 * 0.4 && (count as f64) < full as f64 * 0.6,
            "wedge filled {count} of {full}"
        );
    }
}
