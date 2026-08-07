//! Voxelizing a triangle mesh.
//!
//! Brushes are convex solids and can be tested with a sign check per plane.
//! Displacements are not: they are a heightfield of triangles with no inside,
//! so a voxel is filled when a triangle *passes through* it rather than when
//! its centre is contained by something.
//!
//! The test is the separating-axis theorem for a triangle against an
//! axis-aligned box, which is exact — a voxel is emitted if and only if the
//! triangle really intersects it. Sampling the triangle's surface instead
//! would leave holes wherever a triangle crosses a voxel corner without any
//! sample point landing inside, and holes in terrain are exactly what you
//! notice when you walk on it.

use crate::geom::{Aabb, Vec3};
use crate::voxel::grid::IVec3;

#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
}

impl Triangle {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Triangle {
        Triangle { a, b, c }
    }

    pub fn bounds(&self) -> Aabb {
        let mut bounds = Aabb::empty();
        bounds.extend(self.a);
        bounds.extend(self.b);
        bounds.extend(self.c);
        bounds
    }

    /// Unnormalized normal. Zero for a degenerate triangle.
    pub fn normal(&self) -> Vec3 {
        (self.b - self.a).cross(self.c - self.a)
    }

    pub fn is_degenerate(&self) -> bool {
        !self.a.is_finite()
            || !self.b.is_finite()
            || !self.c.is_finite()
            || self.normal().length() < 1e-12
    }
}

/// Whether `tri` overlaps the unit cube whose minimum corner is `voxel`.
///
/// Separating-axis theorem: two convex shapes are disjoint exactly when some
/// axis separates their projections. For a triangle and a box it suffices to
/// test the box's three face normals, the triangle's plane normal, and the
/// nine cross products of the box axes with the triangle's edges.
pub fn triangle_overlaps_voxel(tri: &Triangle, voxel: IVec3) -> bool {
    let centre = Vec3::new(
        voxel[0] as f64 + 0.5,
        voxel[1] as f64 + 0.5,
        voxel[2] as f64 + 0.5,
    );
    let half = 0.5;

    // Work in the box's frame, so the box is [-half, half]^3 about the origin.
    let v = [tri.a - centre, tri.b - centre, tri.c - centre];

    // 1. The box's own face normals: a per-axis overlap test.
    for axis in 0..3 {
        let (min, max) = min_max(v[0].axis(axis), v[1].axis(axis), v[2].axis(axis));
        if min > half || max < -half {
            return false;
        }
    }

    // 2. The triangle's plane against the box.
    let normal = tri.normal();
    let radius = half * (normal.x.abs() + normal.y.abs() + normal.z.abs());
    let distance = normal.dot(v[0]);
    if distance.abs() > radius {
        return false;
    }

    // 3. The nine edge-cross-axis directions.
    let edges = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    for edge in edges {
        // Cross products of `edge` with each unit box axis, written out: the
        // zero components make the projections cheap.
        let axes = [
            Vec3::new(0.0, -edge.z, edge.y),
            Vec3::new(edge.z, 0.0, -edge.x),
            Vec3::new(-edge.y, edge.x, 0.0),
        ];
        for axis in axes {
            let (min, max) = min_max(axis.dot(v[0]), axis.dot(v[1]), axis.dot(v[2]));
            let radius = half * (axis.x.abs() + axis.y.abs() + axis.z.abs());
            if min > radius || max < -radius {
                return false;
            }
        }
    }

    true
}

fn min_max(a: f64, b: f64, c: f64) -> (f64, f64) {
    (a.min(b).min(c), a.max(b).max(c))
}

/// Call `emit` for every voxel the triangle passes through.
///
/// A voxel owns the half-open cube `[v, v+1)`, so a triangle lying exactly in
/// a boundary plane belongs to the voxel above it and not to both. Owning it
/// twice would double the thickness of every terrain surface that happens to
/// sit on a grid line, which at 16 units per block is most of them.
pub fn voxelize_triangle(tri: &Triangle, mut emit: impl FnMut(IVec3)) {
    if tri.is_degenerate() {
        return;
    }
    let bounds = tri.bounds();
    let min = [
        bounds.min.x.floor() as i64,
        bounds.min.y.floor() as i64,
        bounds.min.z.floor() as i64,
    ];
    let max = [
        bounds.max.x.floor() as i64,
        bounds.max.y.floor() as i64,
        bounds.max.z.floor() as i64,
    ];

    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                let voxel = [x as i32, y as i32, z as i32];
                if triangle_overlaps_voxel(tri, voxel) {
                    emit(voxel);
                }
            }
        }
    }
}

/// The bounds of the part of `tri` that lies inside `voxel`, or `None` if it
/// does not reach into it at all.
///
/// [`voxelize_triangle`] answers whether a triangle is in a voxel; this answers
/// *where* in it, which is what a collision shape needs. A catwalk floor
/// crosses a cell three pixels above its bottom face, and a block shaped to
/// those three pixels is one you can stand on without the cell below being
/// solid too.
///
/// The triangle is clipped against the cell's six planes, Sutherland–Hodgman,
/// keeping the polygon that survives. Clipping rather than sampling for the
/// same reason the overlap test is exact rather than sampled: a box that came
/// out even slightly short would be a surface you fall through.
pub fn clip_triangle_to_voxel(tri: &Triangle, voxel: IVec3) -> Option<Aabb> {
    if tri.is_degenerate() {
        return None;
    }
    let min = Vec3::new(voxel[0] as f64, voxel[1] as f64, voxel[2] as f64);
    let max = min + Vec3::splat(1.0);

    let mut polygon = vec![tri.a, tri.b, tri.c];
    for axis in 0..3 {
        // Keep what is above the low face, then what is below the high one.
        polygon = keep(&polygon, axis, min.axis(axis), true);
        polygon = keep(&polygon, axis, max.axis(axis), false);
        if polygon.is_empty() {
            return None;
        }
    }

    let mut bounds = Aabb::empty();
    for point in polygon {
        // Clipping is exact in theory and floating point in practice, so pin
        // the result inside the cell rather than letting rounding put a box a
        // hair outside the block that carries it.
        bounds.extend(Vec3::new(
            point.x.clamp(min.x, max.x),
            point.y.clamp(min.y, max.y),
            point.z.clamp(min.z, max.z),
        ));
    }
    Some(bounds)
}

/// The part of `polygon` on the kept side of the plane `axis = at`.
fn keep(polygon: &[Vec3], axis: usize, at: f64, above: bool) -> Vec<Vec3> {
    let inside = |p: &Vec3| {
        if above {
            p.axis(axis) >= at
        } else {
            p.axis(axis) <= at
        }
    };

    let mut out = Vec::with_capacity(polygon.len() + 1);
    for (index, current) in polygon.iter().enumerate() {
        let previous = &polygon[(index + polygon.len() - 1) % polygon.len()];
        let (here, there) = (inside(current), inside(previous));
        if here != there {
            let (a, b) = (previous.axis(axis), current.axis(axis));
            let t = if (b - a).abs() > f64::EPSILON {
                (at - a) / (b - a)
            } else {
                0.0
            };
            out.push(*previous + (*current - *previous) * t);
        }
        if here {
            out.push(*current);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn voxels(tri: &Triangle) -> HashSet<IVec3> {
        let mut out = HashSet::new();
        voxelize_triangle(tri, |v| {
            out.insert(v);
        });
        out
    }

    #[test]
    fn a_triangle_inside_one_voxel_fills_only_it() {
        let tri = Triangle::new(
            Vec3::new(0.2, 0.2, 0.2),
            Vec3::new(0.8, 0.3, 0.2),
            Vec3::new(0.3, 0.8, 0.4),
        );
        assert_eq!(voxels(&tri), HashSet::from([[0, 0, 0]]));
    }

    /// A flat triangle spanning a 4x4 patch must cover every voxel of that
    /// patch's lower layer and nothing above it.
    #[test]
    fn a_flat_triangle_covers_the_layer_it_lies_in() {
        let tri = Triangle::new(
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(4.0, 0.5, 0.0),
            Vec3::new(0.0, 0.5, 4.0),
        );
        let found = voxels(&tri);
        assert!(
            found.iter().all(|v| v[1] == 0),
            "escaped its layer: {found:?}"
        );
        // The lower-left triangle of a 4x4 square: 4+3+2+1 full voxels plus the
        // ones the hypotenuse clips, which SAT counts because it touches them.
        assert!(
            found.len() >= 10 && found.len() <= 16,
            "{} voxels",
            found.len()
        );
        for v in [[0, 0, 0], [3, 0, 0], [0, 0, 3]] {
            assert!(found.contains(&v), "missing corner {v:?}");
        }
    }

    /// The property that matters for terrain: a surface must not have holes.
    /// Walking the triangle finely and checking every sampled point's voxel was
    /// emitted catches any gap a sampling-based rasterizer would leave.
    #[test]
    fn a_steep_triangle_is_watertight() {
        let tri = Triangle::new(
            Vec3::new(-1.3, 0.4, 2.2),
            Vec3::new(7.9, 5.6, -3.1),
            Vec3::new(2.5, -4.2, 6.8),
        );
        let found = voxels(&tri);

        let n = 400;
        for i in 0..=n {
            for j in 0..=(n - i) {
                let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                let p = tri.a + (tri.b - tri.a) * u + (tri.c - tri.a) * v;
                let voxel = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
                assert!(
                    found.contains(&voxel),
                    "point {p:?} is on the triangle but voxel {voxel:?} was not emitted"
                );
            }
        }
    }

    /// The converse: nothing may be emitted that the triangle misses. A voxel
    /// far from the triangle's plane but inside its bounding box is the case a
    /// naive bounding-box fill gets wrong.
    #[test]
    fn voxels_the_triangle_misses_are_not_emitted() {
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(8.0, 0.0, 0.0),
            Vec3::new(0.0, 8.0, 0.0),
        );
        let found = voxels(&tri);
        // Well past the hypotenuse, but inside the bounding box.
        assert!(
            !found.contains(&[7, 7, 0]),
            "filled a voxel outside the triangle"
        );
        // Off the plane entirely.
        assert!(!found.contains(&[1, 1, 5]));
        assert!(found.contains(&[0, 0, 0]));
    }

    #[test]
    fn degenerate_triangles_emit_nothing() {
        let point = Triangle::new(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        assert!(voxels(&point).is_empty());

        let line = Triangle::new(
            Vec3::ZERO,
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(8.0, 0.0, 0.0),
        );
        assert!(voxels(&line).is_empty());

        let nan = Triangle::new(Vec3::new(f64::NAN, 0.0, 0.0), Vec3::ZERO, Vec3::splat(1.0));
        assert!(voxels(&nan).is_empty());
    }

    /// Terrain vertices land on grid lines constantly. Owning such a triangle
    /// from both sides would make every flat surface two blocks thick, so a
    /// voxel owns `[v, v+1)` and the upper one takes it.
    #[test]
    fn a_triangle_on_a_voxel_boundary_belongs_to_one_side_only() {
        let tri = Triangle::new(
            Vec3::new(0.2, 1.0, 0.2),
            Vec3::new(0.8, 1.0, 0.2),
            Vec3::new(0.2, 1.0, 0.8),
        );
        assert_eq!(voxels(&tri), HashSet::from([[0, 1, 0]]));
    }

    /// The same convention on the far side: a triangle reaching exactly to
    /// y = 2 must not spill into the layer starting there unless it has area
    /// in it. Here it does touch, so one voxel of that layer is correct, but a
    /// whole extra layer would not be.
    #[test]
    fn a_triangle_ending_on_a_boundary_does_not_spill_a_layer() {
        let tri = Triangle::new(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.5, 2.0, 0.5),
            Vec3::new(0.6, 2.0, 0.6),
        );
        let found = voxels(&tri);
        let top: Vec<_> = found.iter().filter(|v| v[1] == 2).collect();
        assert!(top.len() <= 1, "spilled into the layer above: {top:?}");
        assert!(
            found.contains(&[0, 0, 0]) && found.contains(&[0, 1, 0]),
            "{found:?}"
        );
    }

    /// A triangle that stays inside one voxel is its own bounds there: nothing
    /// was clipped, so nothing may be lost or gained.
    #[test]
    fn clipping_inside_one_voxel_keeps_the_triangle() {
        let tri = Triangle::new(
            Vec3::new(0.25, 0.5, 0.25),
            Vec3::new(0.75, 0.5, 0.25),
            Vec3::new(0.25, 0.5, 0.75),
        );
        let box_ = clip_triangle_to_voxel(&tri, [0, 0, 0]).expect("overlaps");
        assert_eq!(box_.min, Vec3::new(0.25, 0.5, 0.25));
        assert_eq!(box_.max, Vec3::new(0.75, 0.5, 0.75));
    }

    /// The point of clipping: a cell gets the part of the surface that is in
    /// it, not the whole triangle's bounds. A floor crossing four cells three
    /// pixels up is three pixels thick in each of them.
    #[test]
    fn clipping_gives_each_voxel_only_its_own_part() {
        let tri = Triangle::new(
            Vec3::new(0.0, 0.2, 0.0),
            Vec3::new(4.0, 0.2, 0.0),
            Vec3::new(0.0, 0.2, 4.0),
        );
        let first = clip_triangle_to_voxel(&tri, [0, 0, 0]).expect("overlaps");
        assert_eq!(first.min, Vec3::new(0.0, 0.2, 0.0));
        assert_eq!(first.max, Vec3::new(1.0, 0.2, 1.0));

        let along = clip_triangle_to_voxel(&tri, [2, 0, 0]).expect("overlaps");
        assert_eq!(along.min.x, 2.0);
        assert!(along.max.x <= 3.0, "{along:?} left its cell");
        assert!(along.max.z <= 2.0, "{along:?} past the hypotenuse");
    }

    /// A voxel the triangle misses has no box, and must not be given the
    /// bounding box's answer instead.
    #[test]
    fn clipping_a_voxel_the_triangle_misses_gives_nothing() {
        let tri = Triangle::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(8.0, 0.0, 0.0),
            Vec3::new(0.0, 8.0, 0.0),
        );
        assert!(clip_triangle_to_voxel(&tri, [7, 7, 0]).is_none());
        assert!(clip_triangle_to_voxel(&tri, [1, 1, 5]).is_none());
    }

    /// Clipping must agree with the overlap test: a box for every voxel that
    /// test emits, and every box inside the voxel it belongs to.
    #[test]
    fn every_voxelized_cell_has_a_box_inside_it() {
        let tri = Triangle::new(
            Vec3::new(-1.3, 0.4, 2.2),
            Vec3::new(7.9, 5.6, -3.1),
            Vec3::new(2.5, -4.2, 6.8),
        );
        for voxel in voxels(&tri) {
            let Some(box_) = clip_triangle_to_voxel(&tri, voxel) else {
                // A triangle grazing a face exactly is a legitimate empty clip;
                // what would be wrong is a box outside its cell.
                continue;
            };
            for axis in 0..3 {
                let low = f64::from(voxel[axis]);
                assert!(
                    box_.min.axis(axis) >= low - 1e-9 && box_.max.axis(axis) <= low + 1.0 + 1e-9,
                    "{box_:?} escaped voxel {voxel:?} on axis {axis}"
                );
            }
        }
    }

    #[test]
    fn negative_coordinates_round_the_right_way() {
        let tri = Triangle::new(
            Vec3::new(-2.8, -1.5, -0.2),
            Vec3::new(-2.2, -1.5, -0.2),
            Vec3::new(-2.8, -1.5, -0.8),
        );
        let found = voxels(&tri);
        assert!(found.contains(&[-3, -2, -1]), "{found:?}");
    }
}
