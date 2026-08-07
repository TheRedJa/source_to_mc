//! What a prop is solid as.
//!
//! A prop drawn as a mesh has no collision of its own: the block carrying the
//! model is registered `noCollision`, because a solid metre cube in the middle
//! of a crate is worse than nothing. So the shape has to come from somewhere
//! else, and until now it came from a shell of `minecraft:barrier` voxelized
//! from the prop's triangles — one full cube per cell the surface passes
//! through. Fine to walk on, wrong in every detail: a catwalk floor three
//! pixels thick collides as a whole block, a railing as a wall, a crate as a
//! box a foot larger than it looks. Physics mods that resolve contacts against
//! block shapes then see a map of invisible boxes rather than the props.
//!
//! KubeJS 2101 can carry a real shape: `BlockBuilder.box()` appends to
//! `customShape` and `BasicKubeBlock.getShape` returns it, so a generated block
//! can be shaped like the geometry inside its own cell.
//!
//! Per cell, and not per prop, because Minecraft only tests blocks within one
//! block of an entity's bounding box. A shape describing geometry ten blocks
//! away is never consulted, so the shell stays a shell — what changes is that
//! each cell of it is shaped rather than cubic.

use crate::geom::{Aabb, Vec3};
use crate::voxel::grid::IVec3;
use crate::voxel::mesh::{Triangle, clip_triangle_to_voxel, voxelize_triangle};
use std::collections::HashMap;

/// Sixteenths of a block: the unit KubeJS's `box()` takes and Minecraft models
/// are authored in.
pub const STEPS: i64 = 16;

/// One cell's collision box, in sixteenths of that cell.
///
/// Ordered `[x1, y1, z1, x2, y2, z2]`, which is `box()`'s argument order, so
/// the script writes the array out as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Shape(pub [u8; 6]);

impl Shape {
    /// The box `bounds` occupies within `cell`, rounded outward to multiples of
    /// `step` sixteenths.
    ///
    /// Outward, never to nearest. A box rounded inward is a surface you fall
    /// through or a wall you clip into, and a sixteenth of a block of slack is
    /// invisible where a missing floor is not.
    pub fn new(cell: IVec3, bounds: Aabb, step: i64) -> Option<Shape> {
        let step = step.clamp(1, STEPS);
        let origin = Vec3::new(f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
        let mut out = [0u8; 6];
        for axis in 0..3 {
            let low = (bounds.min.axis(axis) - origin.axis(axis)) * STEPS as f64;
            let high = (bounds.max.axis(axis) - origin.axis(axis)) * STEPS as f64;
            if !low.is_finite() || !high.is_finite() {
                return None;
            }
            let low = down(low, step).clamp(0, STEPS);
            let high = up(high, step).clamp(0, STEPS);
            // A surface lying exactly in a cell boundary clips to nothing at
            // all. Give it the thinnest box there is rather than no collision:
            // a floor on a grid line is the most common thing in a map.
            let (low, high) = if low == high {
                if high < STEPS {
                    (low, high + step.min(STEPS - high))
                } else {
                    (low - step, high)
                }
            } else {
                (low, high)
            };
            out[axis] = low as u8;
            out[axis + 3] = high as u8;
        }
        Some(Shape(out))
    }

    /// Whether this is the whole cell, which is what a barrier was.
    pub fn is_full(&self) -> bool {
        self.0[0..3] == [0, 0, 0] && self.0[3..6] == [STEPS as u8; 3]
    }

    /// The block id for this shape, without a namespace.
    ///
    /// Spelled out rather than hashed, because the numbers are the whole of it:
    /// two maps converted separately name the same shape the same way, which is
    /// what lets `batch` merge their packs, and `collision_0_0_0_16_2_16` says
    /// what it is when it turns up in a palette.
    pub fn id(&self) -> String {
        let [x1, y1, z1, x2, y2, z2] = self.0;
        format!("collision_{x1}_{y1}_{z1}_{x2}_{y2}_{z2}")
    }

    /// The `box()` arguments, in sixteenths.
    pub fn box_args(&self) -> String {
        let [x1, y1, z1, x2, y2, z2] = self.0;
        format!("{x1}, {y1}, {z1}, {x2}, {y2}, {z2}")
    }
}

fn down(v: f64, step: i64) -> i64 {
    let step = step as f64;
    (v / step).floor() as i64 * step as i64
}

fn up(v: f64, step: i64) -> i64 {
    let step = step as f64;
    (v / step).ceil() as i64 * step as i64
}

/// The cells a prop's collision mesh passes through, and how much of each it
/// fills.
///
/// `triangles` are in block space. Cells come from the same exact overlap test
/// the voxelizer uses, so a shaped shell covers exactly the cells the barrier
/// shell used to; what is new is the box within each.
pub fn cells<'a>(triangles: impl Iterator<Item = &'a [Vec3; 3]>) -> HashMap<IVec3, Aabb> {
    let mut out: HashMap<IVec3, Aabb> = HashMap::new();
    for corners in triangles {
        let tri = Triangle::new(corners[0], corners[1], corners[2]);
        if tri.is_degenerate() {
            continue;
        }
        voxelize_triangle(&tri, |cell| {
            let Some(part) = clip_triangle_to_voxel(&tri, cell) else {
                return;
            };
            let entry = out.entry(cell).or_insert_with(Aabb::empty);
            entry.extend(part.min);
            entry.extend(part.max);
        });
    }
    out
}

/// How finely the shapes of a whole conversion may be cut before there are too
/// many distinct ones to register.
///
/// Every distinct box is a registered block, shared across every prop and every
/// map in a pack, so the count is bounded by how many *shapes* a campaign uses
/// rather than by how many props it has. When that is still too many, the
/// quantization coarsens — sixteenths, eighths, quarters, halves, whole cells —
/// and the cells are cut again. Coarsening is always outward, so it only ever
/// makes collision more generous, and the last step is the barrier shell this
/// replaces.
const COARSENING: [i64; 5] = [1, 2, 4, 8, 16];

/// The shape for every cell, at the finest step whose distinct count fits
/// `max_shapes`.
///
/// Returns the shapes and the step they were cut at, so the caller can say when
/// a map got a coarser answer than it asked for.
pub fn quantize(cells: &HashMap<IVec3, Aabb>, max_shapes: usize) -> (HashMap<IVec3, Shape>, i64) {
    let mut last = (HashMap::new(), *COARSENING.last().unwrap());
    for step in COARSENING {
        let shaped: HashMap<IVec3, Shape> = cells
            .iter()
            .filter_map(|(cell, bounds)| Shape::new(*cell, *bounds, step).map(|s| (*cell, s)))
            .collect();
        let distinct: std::collections::HashSet<Shape> = shaped.values().copied().collect();
        last = (shaped, step);
        if max_shapes == 0 || distinct.len() <= max_shapes {
            break;
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_box(shape: Shape) -> [u8; 6] {
        shape.0
    }

    /// The shape of a floor lying two pixels above a cell's bottom: thin, and
    /// thin at the right height.
    #[test]
    fn a_thin_surface_gets_a_thin_box() {
        let bounds = Aabb::new(Vec3::new(0.0, 0.125, 0.0), Vec3::new(1.0, 0.125, 1.0));
        let shape = Shape::new([0, 0, 0], bounds, 1).expect("in the cell");
        assert_eq!(cell_box(shape), [0, 2, 0, 16, 3, 16]);
        assert!(!shape.is_full());
    }

    /// Rounding is outward on both sides, so the box never sits inside the
    /// geometry it stands for.
    #[test]
    fn rounding_never_cuts_into_the_geometry() {
        let bounds = Aabb::new(Vec3::new(0.31, 0.02, 0.31), Vec3::new(0.69, 0.98, 0.69));
        let shape = Shape::new([0, 0, 0], bounds, 1).expect("in the cell");
        let [x1, y1, z1, x2, y2, z2] = cell_box(shape);
        assert!(f64::from(x1) / 16.0 <= 0.31 && f64::from(x2) / 16.0 >= 0.69);
        assert!(f64::from(y1) / 16.0 <= 0.02 && f64::from(y2) / 16.0 >= 0.98);
        assert!(f64::from(z1) / 16.0 <= 0.31 && f64::from(z2) / 16.0 >= 0.69);
    }

    /// A cell the mesh fills is the whole cube, which is exactly the barrier
    /// this replaces.
    #[test]
    fn a_filled_cell_is_the_full_cube() {
        let bounds = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
        let shape = Shape::new([0, 0, 0], bounds, 1).expect("in the cell");
        assert!(shape.is_full());
        assert_eq!(shape.id(), "collision_0_0_0_16_16_16");
    }

    /// A surface lying exactly in a cell boundary clips to zero thickness.
    /// Giving it no box would drop the collision of every floor built on a
    /// grid line, which at 16 units per block is most of them.
    #[test]
    fn a_surface_on_a_boundary_still_gets_a_box() {
        let flat = Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.0, 1.0));
        let shape = Shape::new([0, 0, 0], flat, 1).expect("in the cell");
        assert_eq!(cell_box(shape), [0, 0, 0, 16, 1, 16]);

        let top = Aabb::new(Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let shape = Shape::new([0, 0, 0], top, 1).expect("in the cell");
        assert_eq!(cell_box(shape), [0, 15, 0, 16, 16, 16]);
    }

    /// Cells are the ones the voxelizer would have put barriers in, and each
    /// box stays inside its own cell.
    #[test]
    fn cells_match_the_voxelizer_and_stay_inside_themselves() {
        let triangles = [[
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(3.5, 0.6, 0.5),
            Vec3::new(0.5, 0.7, 2.5),
        ]];
        let found = cells(triangles.iter());
        assert!(!found.is_empty());
        for (cell, bounds) in &found {
            for axis in 0..3 {
                let low = f64::from(cell[axis]);
                assert!(
                    bounds.min.axis(axis) >= low - 1e-9
                        && bounds.max.axis(axis) <= low + 1.0 + 1e-9,
                    "{bounds:?} escaped {cell:?}"
                );
            }
        }
    }

    /// The budget: too many distinct shapes and the cut coarsens until they
    /// fit, rather than registering thousands of blocks nobody asked for.
    #[test]
    fn a_tight_budget_coarsens_the_cut() {
        let mut cells: HashMap<IVec3, Aabb> = HashMap::new();
        for i in 0..40i32 {
            let y = f64::from(i) / 40.0;
            cells.insert(
                [i, 0, 0],
                Aabb::new(Vec3::new(0.0, y, 0.0), Vec3::new(1.0, y + 0.05, 1.0)),
            );
        }

        let (fine, step) = quantize(&cells, 0);
        assert_eq!(step, 1);
        let distinct = |shapes: &HashMap<IVec3, Shape>| {
            shapes
                .values()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
        };
        assert!(distinct(&fine) > 4, "{} shapes", distinct(&fine));

        let (coarse, step) = quantize(&cells, 4);
        assert!(step > 1, "did not coarsen");
        assert!(distinct(&coarse) <= 4, "{} shapes", distinct(&coarse));
    }

    /// Coarsening may only ever make a box bigger: a cheaper shape that cut
    /// into the geometry would be a hole to fall through.
    #[test]
    fn coarsening_only_grows_boxes() {
        let bounds = Aabb::new(Vec3::new(0.3, 0.1, 0.3), Vec3::new(0.7, 0.4, 0.7));
        let fine = Shape::new([0, 0, 0], bounds, 1).expect("in the cell");
        for step in [2, 4, 8, 16] {
            let coarse = Shape::new([0, 0, 0], bounds, step).expect("in the cell");
            for axis in 0..3 {
                assert!(
                    coarse.0[axis] <= fine.0[axis] && coarse.0[axis + 3] >= fine.0[axis + 3],
                    "step {step} cut into the geometry: {coarse:?} inside {fine:?}"
                );
            }
        }
    }

    /// Ids are the numbers, so two maps converted separately agree and their
    /// packs merge.
    #[test]
    fn the_same_box_has_the_same_id_everywhere() {
        let bounds = Aabb::new(Vec3::new(0.25, 0.0, 0.25), Vec3::new(0.75, 0.5, 0.75));
        let here = Shape::new([0, 0, 0], bounds, 1).expect("in the cell");
        let far = Shape::new(
            [100, -30, 7],
            Aabb::new(
                Vec3::new(100.25, -30.0, 7.25),
                Vec3::new(100.75, -29.5, 7.75),
            ),
            1,
        )
        .expect("in the cell");
        assert_eq!(here, far);
        assert_eq!(here.id(), far.id());
        assert_eq!(here.box_args(), "4, 0, 4, 12, 8, 12");
    }
}
