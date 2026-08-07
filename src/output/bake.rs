//! Drawing a prop as a block instead of as an entity.
//!
//! A `block_display` entity places a mesh at any angle, which is exactly what
//! a Source prop needs, and it costs a frame's work to do it: display entities
//! go through the entity renderer every frame and are never baked into a chunk
//! section's vertex buffer the way an ordinary block's faces are. A few
//! hundred props in view is a few hundred thousand triangles resubmitted per
//! frame, and that is the difference between a map running and a map at 45
//! frames per second. Culling mods do not recover it; the geometry is being
//! rebuilt, not merely drawn.
//!
//! The way out is that nothing about a prop actually has to be an entity. The
//! entity exists to carry a rotation and a fractional position, and both of
//! those can be baked into the mesh instead — the OBJ is generated either way.
//! So the prop becomes a normal block, placed in a cell inside its own
//! geometry, wearing a model whose coordinates already have the map's rotation
//! and offset in them. Chunk-baked, it costs nothing per frame, and the chunk
//! mesher lights each of its faces instead of the whole prop taking the one
//! light value of the cell it stands in.
//!
//! The price is one registered block per distinct placement, which is what
//! [`Key`] exists to keep down.

use crate::geom::{Aabb, Vec3};
use crate::output::obj::Place;
use crate::voxel::grid::{AIR, IVec3, VoxelGrid};
use std::collections::HashSet;

/// What makes one baked variant of a model different from another.
///
/// Two placements of the same model at the same angle and the same position
/// within a block are the same mesh, so they should be one block: a row of
/// fence posts or a stack of grid-aligned crates collapses to a single
/// registration. Rotation and offset are rounded first, since placements that
/// differ by a hundredth of a degree are not two different props.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    /// The model-space asset's id, which is also the shared `.mtl` stem.
    pub model: String,
    /// The rotation, as [`crate::output::display::quantize`] rounds it.
    pub rotation: [i64; 4],
    /// The prop's origin measured from its block's corner, in grid steps.
    pub offset: [i64; 3],
    /// The prop's own scale, in 1/256ths.
    pub scale: i64,
}

/// Scale is rounded to this many steps. Props are almost always at 1.0 and the
/// few that are not are not at a hundredth of anything.
const SCALE_STEPS: f64 = 256.0;

impl Key {
    /// The key for one placement.
    ///
    /// `origin` is the prop's origin in block space, `anchor` the cell the
    /// block goes in.
    pub fn new(
        model: &str,
        rotation: [f64; 4],
        origin: Vec3,
        anchor: IVec3,
        scale: f64,
        grid: i64,
        angle_steps: i64,
    ) -> Key {
        let grid = grid.max(1);
        let local = [
            origin.x - f64::from(anchor[0]),
            origin.y - f64::from(anchor[1]),
            origin.z - f64::from(anchor[2]),
        ];
        Key {
            model: model.to_string(),
            rotation: crate::output::display::quantize(rotation, angle_steps),
            offset: local.map(|c| (c * grid as f64).round() as i64),
            scale: (scale * SCALE_STEPS).round() as i64,
        }
    }

    /// The transform this key stands for: what the mesh is written through.
    ///
    /// Derived from the rounded values rather than the exact ones, so the mesh
    /// a key names is the mesh every placement sharing that key gets. Rounding
    /// the key and then baking the unrounded transform would give each of them
    /// a slightly different mesh under one block id, and whichever was
    /// generated first would silently stand in for the rest.
    pub fn place(&self, grid: i64) -> Place {
        let grid = f64::from(grid.max(1) as i32);
        Place {
            basis: crate::output::display::basis_of(crate::output::display::dequantize(
                self.rotation,
            )),
            scale: self.scale as f64 / SCALE_STEPS,
            translation: Vec3::new(
                self.offset[0] as f64 / grid,
                self.offset[1] as f64 / grid,
                self.offset[2] as f64 / grid,
            ),
        }
    }

    /// The block id for this variant.
    ///
    /// A hash rather than a counter because packs are merged: `batch` converts
    /// each map on its own and folds the results into one pack, and a
    /// per-pack counter would give two different variants from two different
    /// maps the same id, which merging would then quietly collapse into one.
    /// A hash of the key itself has no such state to disagree about.
    pub fn id(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{}_b{:016x}", self.model, hasher.finish())
    }
}

/// The cell a prop's block goes in, or `None` if there is nowhere to put it.
///
/// The cell has to be air. A prop block that replaced one of the map's own
/// blocks would punch a hole in whatever it stood against — the same failure
/// invisible barriers were causing — and one that replaced another prop's
/// block would delete that prop entirely, so cells already spoken for are
/// refused too.
///
/// Candidates are the cells of the prop's own footprint, nearest the centre
/// first. Inside its own geometry is where the block wants to be: the section
/// it is filed under is then the section the prop is really in, so it appears
/// and disappears with its surroundings rather than a chunk early or late.
pub fn anchor(grid: &VoxelGrid, bounds: Aabb, taken: &HashSet<IVec3>) -> Option<IVec3> {
    if bounds.is_empty() {
        return None;
    }
    // Inside the prop first. A sign bolted to a wall or a railing set into a
    // floor is thin enough that every cell it covers is the thing it is
    // attached to, and for those the search widens by a block — still next to
    // the prop, and a cell next to it beats no prop at all.
    let grown = Aabb::new(
        bounds.min - Vec3::new(1.0, 1.0, 1.0),
        bounds.max + Vec3::new(1.0, 1.0, 1.0),
    );
    free(grid, bounds, bounds, taken).or_else(|| free(grid, grown, bounds, taken))
}

/// The free cell of `search` nearest the centre of `bounds`.
fn free(
    grid: &VoxelGrid,
    search: Aabb,
    bounds: Aabb,
    taken: &HashSet<IVec3>,
) -> Option<IVec3> {
    let centre = (bounds.min + bounds.max) * 0.5;
    let bounds = search;

    let mut cells: Vec<IVec3> = Vec::new();
    for x in axis(bounds.min.x, bounds.max.x) {
        for y in axis(bounds.min.y, bounds.max.y) {
            for z in axis(bounds.min.z, bounds.max.z) {
                cells.push([x, y, z]);
            }
        }
    }

    let distance = |cell: &IVec3| -> f64 {
        let d = Vec3::new(
            f64::from(cell[0]) + 0.5 - centre.x,
            f64::from(cell[1]) + 0.5 - centre.y,
            f64::from(cell[2]) + 0.5 - centre.z,
        );
        d.dot(d)
    };
    cells.sort_by(|a, b| {
        distance(a).partial_cmp(&distance(b)).unwrap_or(std::cmp::Ordering::Equal)
    });

    cells.into_iter().find(|cell| grid.get(*cell) == AIR && !taken.contains(cell))
}

/// How many candidate cells one axis of a footprint offers.
///
/// A prop as large as a building would otherwise ask for its whole volume in
/// cells, and the answer is always one of the first few tried, so the range is
/// sampled rather than walked.
const SPAN: i32 = 8;

/// The cell coordinates to try along one axis.
fn axis(lo: f64, hi: f64) -> Vec<i32> {
    let (lo, hi) = (lo.floor() as i32, hi.floor() as i32);
    let count = hi - lo + 1;
    if count <= SPAN {
        return (lo..=hi).collect();
    }
    // Evenly spaced across the span, ends included.
    (0..SPAN).map(|i| lo + (i * (count - 1)) / (SPAN - 1)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::grid::Palette;

    fn bounds(min: [f64; 3], max: [f64; 3]) -> Aabb {
        Aabb::new(Vec3::new(min[0], min[1], min[2]), Vec3::new(max[0], max[1], max[2]))
    }

    fn key(rotation: [f64; 4], origin: Vec3, anchor: IVec3) -> Key {
        Key::new("prop_x", rotation, origin, anchor, 1.0, 16, 64)
    }

    /// The whole reason keys exist: two props placed identically must not
    /// register two blocks.
    #[test]
    fn identical_placements_share_one_variant() {
        let a = key([0.0, 0.0, 0.0, 1.0], Vec3::new(10.5, 4.25, -3.0), [10, 4, -3]);
        let b = key([0.0, 0.0, 0.0, 1.0], Vec3::new(20.5, 8.25, 5.0), [20, 8, 5]);
        assert_eq!(a, b, "same model, same angle, same offset in the block");
        assert_eq!(a.id(), b.id());
    }

    /// And two that really differ must not be collapsed into one, or every
    /// prop in a map ends up wearing the first one's mesh.
    #[test]
    fn a_different_angle_or_offset_is_a_different_variant() {
        let base = key([0.0, 0.0, 0.0, 1.0], Vec3::new(10.5, 4.25, -3.0), [10, 4, -3]);
        let turned = key([0.0, std::f64::consts::FRAC_1_SQRT_2, 0.0, std::f64::consts::FRAC_1_SQRT_2], Vec3::new(10.5, 4.25, -3.0), [10, 4, -3]);
        let moved = key([0.0, 0.0, 0.0, 1.0], Vec3::new(10.75, 4.25, -3.0), [10, 4, -3]);
        assert_ne!(base, turned);
        assert_ne!(base, moved);
        assert_ne!(base.id(), turned.id());
        assert_ne!(base.id(), moved.id());

        let other_model = Key::new(
            "prop_y",
            [0.0, 0.0, 0.0, 1.0],
            Vec3::new(10.5, 4.25, -3.0),
            [10, 4, -3],
            1.0,
            16,
            64,
        );
        assert_ne!(base.id(), other_model.id());
    }

    /// A quaternion and its negation are the same rotation, and must not
    /// become two blocks drawing the same thing.
    #[test]
    fn a_rotation_and_its_negation_are_one_variant() {
        let origin = Vec3::new(1.0, 2.0, 3.0);
        let q = [0.5, 0.5, 0.5, 0.5];
        let negated = q.map(|c: f64| -c);
        assert_eq!(key(q, origin, [1, 2, 3]), key(negated, origin, [1, 2, 3]));
    }

    /// The transform a key hands out has to be the one the key describes, or
    /// the mesh drawn is not the mesh the id promises.
    #[test]
    fn the_place_a_key_gives_matches_the_key() {
        let key = key([0.0, 0.0, 0.0, 1.0], Vec3::new(10.25, 4.5, -3.75), [10, 4, -4]);
        let place = key.place(16);
        assert!((place.translation.x - 0.25).abs() < 1e-9);
        assert!((place.translation.y - 0.5).abs() < 1e-9);
        assert!((place.translation.z - 0.25).abs() < 1e-9);
        assert!((place.scale - 1.0).abs() < 1e-9);
        // The identity rotation leaves a vertex where it was, plus the offset.
        let v = place.apply(Vec3::new(1.0, 2.0, 3.0));
        assert!((v - Vec3::new(1.25, 2.5, 3.25)).length() < 1e-9);
    }

    /// Rounding may not move a prop further than the grid it was rounded to.
    #[test]
    fn rounding_moves_a_prop_less_than_half_a_step() {
        for offset in [0.0, 0.03, 0.49, 0.5, 0.9, 0.999] {
            let origin = Vec3::new(10.0 + offset, 0.0, 0.0);
            let place = key([0.0, 0.0, 0.0, 1.0], origin, [10, 0, 0]).place(16);
            let error = (place.translation.x - offset).abs();
            assert!(error <= 0.5 / 16.0 + 1e-9, "offset {offset} moved by {error}");
        }
    }

    /// The test the whole thing rests on: a vertex baked into a block's model
    /// has to land where the same vertex would land placed by the map. Both
    /// routes exist — a prop with nowhere to put a block is still an entity —
    /// so if they disagree, some props in a map are simply in the wrong place.
    #[test]
    fn a_baked_vertex_lands_where_the_map_puts_it() {
        use crate::bsp::props::Prop;
        use crate::config::Config;
        use crate::voxel::transform::Transform;

        let transform = Transform::new(&Config::default(), Aabb::empty());
        let units = transform.units_per_block();
        let (grid, steps) = (16i64, 256i64);

        for angles in [
            [0.0, 0.0, 0.0],
            [0.0, 90.0, 0.0],
            [0.0, -135.0, 0.0],
            [12.0, 47.0, -80.0],
            [-33.0, 12.0, 175.0],
        ] {
            let prop = Prop {
                model: "models/x.mdl".into(),
                origin: Vec3::new(163.5, -327.25, 51.0),
                angles,
                scale: 1.0,
                classname: "prop_static".into(),
            };
            let origin = transform.to_block_space(prop.origin);
            let cell = [origin.x.floor() as i32, origin.y.floor() as i32, origin.z.floor() as i32];
            let key = Key::new(
                "prop_x",
                crate::output::display::rotation(&prop, &transform),
                origin,
                cell,
                prop.scale,
                grid,
                steps,
            );
            let place = key.place(grid);

            for v in [
                Vec3::new(32.0, 0.0, 0.0),
                Vec3::new(0.0, 16.0, 0.0),
                Vec3::new(0.0, 0.0, 64.0),
                Vec3::new(11.0, -23.0, 7.0),
            ] {
                // The mesh as `output::obj` writes it: model space, in blocks.
                let mesh = Vec3::new(v.x / units, v.z / units, -v.y / units);
                // Where the block's model puts it, in the world.
                let baked = place.apply(mesh)
                    + Vec3::new(
                        f64::from(cell[0]),
                        f64::from(cell[1]),
                        f64::from(cell[2]),
                    );
                let want = transform.to_block_space(prop.place(v));
                // What the rounding is allowed to cost: half a step of the
                // offset grid, plus what rounding the rotation swings a vertex
                // this far from the pivot.
                let tolerance =
                    0.5 / grid as f64 + mesh.length() * 2.0 / steps as f64 + 1e-9;
                assert!(
                    (baked - want).length() < tolerance,
                    "{angles:?}: {v:?} baked at {baked:?}, the map wants {want:?}"
                );
            }
        }
    }

    /// A rotation rounded onto the coarser set still has to be a rotation:
    /// unit columns at right angles, or every baked prop is sheared.
    #[test]
    fn a_rounded_rotation_is_still_a_rotation() {
        for q in [
            [0.0, 0.0, 0.0, 1.0],
            [0.13, -0.42, 0.77, 0.45],
            [0.5, 0.5, 0.5, 0.5],
            [-0.21, 0.09, -0.66, 0.71],
        ] {
            let basis = crate::output::display::basis_of(crate::output::display::dequantize(
                crate::output::display::quantize(q, 64),
            ));
            for column in basis {
                assert!((column.length() - 1.0).abs() < 1e-9, "{column:?} is not unit");
            }
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                assert!(basis[a].dot(basis[b]).abs() < 1e-9, "columns {a} and {b} are not square");
            }
        }
    }

    fn world() -> VoxelGrid {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let mut grid = VoxelGrid::new();
        for x in -4..4 {
            for z in -4..4 {
                grid.set([x, 0, z], stone);
            }
        }
        grid
    }

    /// A prop standing on the floor takes a cell inside itself, above it.
    #[test]
    fn the_anchor_is_a_free_cell_inside_the_prop() {
        let grid = world();
        let taken = HashSet::new();
        let cell = anchor(&grid, bounds([0.0, 1.0, 0.0], [2.0, 3.0, 2.0]), &taken)
            .expect("a crate on the floor has room in it");
        assert_eq!(grid.get(cell), AIR);
        assert!((1..=3).contains(&cell[1]), "{cell:?} is not inside the crate");
    }

    /// Taking one of the map's own blocks is the failure that shows up as a
    /// hole in a wall, so a solid footprint gets no block at all.
    #[test]
    fn a_prop_with_no_free_cell_gets_none() {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let mut grid = VoxelGrid::new();
        // Solid a block past the prop's own footprint, since the search widens
        // by that much before giving up.
        for x in -1..5 {
            for y in -1..5 {
                for z in -1..5 {
                    grid.set([x, y, z], stone);
                }
            }
        }
        assert_eq!(anchor(&grid, bounds([0.5, 0.5, 0.5], [3.5, 3.5, 3.5]), &HashSet::new()), None);
    }

    /// A sign on a wall or a railing in a floor is thin enough that every cell
    /// it covers is the thing it is bolted to. Those must not all fall back to
    /// entities, so the search widens to the cells beside it.
    #[test]
    fn a_prop_flat_against_a_wall_anchors_beside_it() {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let mut grid = VoxelGrid::new();
        for y in -4..4 {
            for z in -4..4 {
                grid.set([0, y, z], stone);
            }
        }
        // A sign filling the wall's own cell and nothing else.
        let cell = anchor(&grid, bounds([0.1, 0.1, 0.1], [0.9, 1.9, 1.9]), &HashSet::new())
            .expect("a sign on a wall still gets a block");
        assert_eq!(grid.get(cell), AIR);
        assert_eq!(cell[0], -1, "{cell:?} is not next to the wall");
    }

    /// Two props must not be given the same cell: the second block would
    /// replace the first and that prop would simply not be drawn.
    #[test]
    fn a_cell_is_only_given_out_once() {
        let grid = VoxelGrid::new();
        let mut taken = HashSet::new();
        let box_ = bounds([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
        let first = anchor(&grid, box_, &taken).expect("empty space has room");
        taken.insert(first);
        let second = anchor(&grid, box_, &taken).expect("and more of it");
        assert_ne!(first, second);
    }

    /// A prop the size of a building must not ask for its whole volume in
    /// candidate cells.
    #[test]
    fn a_huge_footprint_is_sampled_rather_than_walked() {
        assert_eq!(axis(0.0, 2.0).len(), 3);
        let wide = axis(0.0, 1000.0);
        assert_eq!(wide.len(), SPAN as usize);
        assert_eq!(wide[0], 0);
        assert_eq!(*wide.last().unwrap(), 1000);
    }

    #[test]
    fn an_empty_footprint_has_no_anchor() {
        assert_eq!(anchor(&VoxelGrid::new(), Aabb::empty(), &HashSet::new()), None);
    }
}
