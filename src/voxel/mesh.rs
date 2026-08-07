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
