//! Basic 3D geometry: vectors, planes, boxes, and convex polyhedra.
//!
//! Everything here works in Source units and Source's coordinate system
//! (X forward, Y left, Z up). Conversion to Minecraft space happens in
//! [`crate::voxel::transform`].

use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }

    pub fn splat(v: f64) -> Self {
        Vec3 { x: v, y: v, z: v }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn normalized(self) -> Vec3 {
        let len = self.length();
        if len == 0.0 { self } else { self / len }
    }

    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    /// Index the largest-magnitude component (0 = x, 1 = y, 2 = z).
    pub fn major_axis(self) -> usize {
        let (ax, ay, az) = (self.x.abs(), self.y.abs(), self.z.abs());
        if ax >= ay && ax >= az {
            0
        } else if ay >= az {
            1
        } else {
            2
        }
    }

    pub fn axis(self, i: usize) -> f64 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl From<vbsp::Vector> for Vec3 {
    fn from(v: vbsp::Vector) -> Self {
        Vec3::new(v.x as f64, v.y as f64, v.z as f64)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Div<f64> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f64) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// A plane in the form `normal · p = dist`.
///
/// Source brush sides store *outward*-facing normals, so a point lies inside
/// the brush exactly when `normal · p - dist <= 0` holds for every side.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f64,
}

impl Plane {
    pub fn new(normal: Vec3, dist: f64) -> Self {
        Plane { normal, dist }
    }

    /// Signed distance from `p` to the plane; negative means inside.
    pub fn distance_to(&self, p: Vec3) -> f64 {
        self.normal.dot(p) - self.dist
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// An inverted box that absorbs the first point passed to [`Aabb::extend`].
    pub fn empty() -> Self {
        Aabb {
            min: Vec3::splat(f64::INFINITY),
            max: Vec3::splat(f64::NEG_INFINITY),
        }
    }

    pub fn new(min: Vec3, max: Vec3) -> Self {
        Aabb { min, max }
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    pub fn extend(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub fn union(&self, o: &Aabb) -> Aabb {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Aabb::new(self.min.min(o.min), self.max.max(o.max))
    }

    pub fn size(&self) -> Vec3 {
        if self.is_empty() { Vec3::ZERO } else { self.max - self.min }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

/// Vertices of the convex polyhedron formed by intersecting the half-spaces
/// `normal · p <= dist`.
///
/// Works by intersecting every triple of planes and keeping the points that
/// satisfy all the other half-spaces. Brushes have on the order of 6-20 sides,
/// so the cubic triple loop stays cheap, and it is far more robust than relying
/// on VBSP's axis-aligned bevel planes being present.
///
/// `epsilon` absorbs the floating-point slop in the plane equations; points
/// within it of a boundary still count as inside.
pub fn polyhedron_vertices(planes: &[Plane], epsilon: f64) -> Vec<Vec3> {
    let n = planes.len();
    let mut out: Vec<Vec3> = Vec::new();

    for i in 0..n {
        for j in (i + 1)..n {
            for k in (j + 1)..n {
                let (a, b, c) = (&planes[i], &planes[j], &planes[k]);
                let bc = b.normal.cross(c.normal);
                let denom = a.normal.dot(bc);
                // Near-parallel triples have no single intersection point.
                if denom.abs() < 1e-6 {
                    continue;
                }
                let ca = c.normal.cross(a.normal);
                let ab = a.normal.cross(b.normal);
                let p = (bc * a.dist + ca * b.dist + ab * c.dist) / denom;
                if !p.is_finite() {
                    continue;
                }
                if planes.iter().all(|pl| pl.distance_to(p) <= epsilon) {
                    // Triples meeting at a shared corner produce duplicates.
                    if !out.iter().any(|q| (*q - p).length() < epsilon) {
                        out.push(p);
                    }
                }
            }
        }
    }

    out
}

/// Exact volume of the convex polyhedron defined by `planes`.
///
/// The vertices alone do not give a volume — they have to be organised into
/// faces first. Each plane's face is the set of vertices lying on it, which is
/// a convex polygon; fanning that polygon into triangles and taking the signed
/// tetrahedron volume from an interior point sums to the whole. Using the
/// polyhedron's own centroid as that point keeps every tetrahedron positive,
/// so no winding order needs to be established.
pub fn polyhedron_volume(planes: &[Plane], epsilon: f64) -> f64 {
    let verts = polyhedron_vertices(planes, epsilon);
    if verts.len() < 4 {
        return 0.0;
    }

    let centroid = verts.iter().fold(Vec3::ZERO, |a, b| a + *b) / verts.len() as f64;
    let mut volume = 0.0;

    for plane in planes {
        let face: Vec<Vec3> = verts
            .iter()
            .copied()
            .filter(|v| plane.distance_to(*v).abs() <= epsilon)
            .collect();
        if face.len() < 3 {
            continue;
        }

        // Order the face's vertices around its own centre, in the plane's own
        // two-dimensional basis. Unordered, a fan would produce overlapping
        // triangles and the wrong area.
        let hub = face.iter().fold(Vec3::ZERO, |a, b| a + *b) / face.len() as f64;
        let u = perpendicular(plane.normal);
        let v = plane.normal.cross(u);
        let mut ordered: Vec<(f64, Vec3)> = face
            .iter()
            .map(|p| {
                let d = *p - hub;
                (d.dot(v).atan2(d.dot(u)), *p)
            })
            .collect();
        ordered.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        for window in 1..ordered.len() - 1 {
            let (a, b, c) = (ordered[0].1, ordered[window].1, ordered[window + 1].1);
            volume += tetrahedron_volume(centroid, a, b, c);
        }
    }

    volume
}

/// Any unit vector perpendicular to `n`.
fn perpendicular(n: Vec3) -> Vec3 {
    // Cross with whichever axis `n` is least aligned to, so the result is
    // never near zero.
    let axis = match n.major_axis() {
        0 => Vec3::new(0.0, 1.0, 0.0),
        1 => Vec3::new(0.0, 0.0, 1.0),
        _ => Vec3::new(1.0, 0.0, 0.0),
    };
    n.cross(axis).normalized()
}

fn tetrahedron_volume(apex: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f64 {
    let (x, y, z) = (a - apex, b - apex, c - apex);
    x.dot(y.cross(z)).abs() / 6.0
}

/// The six outward-facing planes of an axis-aligned box.
pub fn box_planes(min: Vec3, max: Vec3) -> [Plane; 6] {
    [
        Plane::new(Vec3::new(-1.0, 0.0, 0.0), -min.x),
        Plane::new(Vec3::new(1.0, 0.0, 0.0), max.x),
        Plane::new(Vec3::new(0.0, -1.0, 0.0), -min.y),
        Plane::new(Vec3::new(0.0, 1.0, 0.0), max.y),
        Plane::new(Vec3::new(0.0, 0.0, -1.0), -min.z),
        Plane::new(Vec3::new(0.0, 0.0, 1.0), max.z),
    ]
}

/// Bounding box of the convex polyhedron defined by `planes`, or `None` when
/// the half-spaces do not enclose a finite volume.
pub fn polyhedron_bounds(planes: &[Plane], epsilon: f64) -> Option<Aabb> {
    let verts = polyhedron_vertices(planes, epsilon);
    if verts.is_empty() {
        return None;
    }
    let mut bounds = Aabb::empty();
    for v in verts {
        bounds.extend(v);
    }
    Some(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six outward-facing planes forming an axis-aligned box.
    fn box_planes(min: Vec3, max: Vec3) -> Vec<Plane> {
        super::box_planes(min, max).to_vec()
    }

    #[test]
    fn cube_has_eight_corners() {
        let planes = box_planes(Vec3::ZERO, Vec3::splat(16.0));
        let verts = polyhedron_vertices(&planes, 1e-4);
        assert_eq!(verts.len(), 8, "got {verts:?}");
    }

    #[test]
    fn cube_bounds_match_input() {
        let (min, max) = (Vec3::new(-32.0, 8.0, 0.0), Vec3::new(64.0, 24.0, 128.0));
        let bounds = polyhedron_bounds(&box_planes(min, max), 1e-4).unwrap();
        assert!((bounds.min - min).length() < 1e-6, "{:?}", bounds.min);
        assert!((bounds.max - max).length() < 1e-6, "{:?}", bounds.max);
    }

    #[test]
    fn wedge_drops_the_clipped_corners() {
        // A cube sliced by a diagonal plane through two opposite edges: the
        // two corners on the far side of the cut disappear, leaving 6.
        let mut planes = box_planes(Vec3::ZERO, Vec3::splat(16.0));
        planes.push(Plane::new(Vec3::new(1.0, 1.0, 0.0).normalized(), 16.0 / 2f64.sqrt()));
        let verts = polyhedron_vertices(&planes, 1e-4);
        assert_eq!(verts.len(), 6, "got {verts:?}");
    }

    #[test]
    fn unbounded_half_spaces_have_no_bounds() {
        // Two parallel planes alone never close off a volume.
        let planes = vec![
            Plane::new(Vec3::new(0.0, 0.0, 1.0), 16.0),
            Plane::new(Vec3::new(0.0, 0.0, -1.0), 0.0),
        ];
        assert!(polyhedron_bounds(&planes, 1e-4).is_none());
    }

    #[test]
    fn box_volume_is_the_product_of_its_sides() {
        let planes = box_planes(Vec3::new(-1.0, 2.0, 0.5), Vec3::new(3.0, 8.0, 2.5));
        let volume = polyhedron_volume(&planes, 1e-9);
        assert!((volume - 4.0 * 6.0 * 2.0).abs() < 1e-9, "got {volume}");
    }

    #[test]
    fn a_cube_cut_corner_to_corner_has_half_the_volume() {
        let mut planes = box_planes(Vec3::ZERO, Vec3::splat(2.0));
        // Diagonal through two opposite edges of the x/y square.
        planes.push(Plane::new(Vec3::new(1.0, 1.0, 0.0).normalized(), 2.0 / 2f64.sqrt()));
        let volume = polyhedron_volume(&planes, 1e-9);
        assert!((volume - 4.0).abs() < 1e-9, "got {volume}");
    }

    /// Slicing a unit cube at `x + y + z = c` removes a corner tetrahedron of
    /// side `3 - c`, so long as that side fits within one edge.
    #[test]
    fn a_sliced_corner_matches_the_hand_computed_volume() {
        let cut = |c: f64| {
            let mut planes = box_planes(Vec3::ZERO, Vec3::splat(1.0));
            planes.push(Plane::new(Vec3::splat(1.0).normalized(), c / 3f64.sqrt()));
            polyhedron_volume(&planes, 1e-9)
        };

        let corner = (3.0f64 - 2.5).powi(3) / 6.0;
        assert!((cut(2.5) - (1.0 - corner)).abs() < 1e-9, "got {}", cut(2.5));

        // Cutting through the centre is exactly half, by symmetry.
        assert!((cut(1.5) - 0.5).abs() < 1e-9, "got {}", cut(1.5));
    }

    /// Volume and the sampling approximation must agree as sampling gets finer,
    /// which is the property `exact` mode is built on.
    #[test]
    fn volume_agrees_with_dense_sampling() {
        let mut planes = box_planes(Vec3::ZERO, Vec3::splat(1.0));
        planes.push(Plane::new(Vec3::new(1.0, 2.0, 0.5).normalized(), 1.1));
        let volume = polyhedron_volume(&planes, 1e-9);

        let n = 120;
        let step = 1.0 / n as f64;
        let mut inside = 0u32;
        for i in 0..n {
            for j in 0..n {
                for k in 0..n {
                    let p = Vec3::new(
                        (i as f64 + 0.5) * step,
                        (j as f64 + 0.5) * step,
                        (k as f64 + 0.5) * step,
                    );
                    if planes.iter().all(|pl| pl.distance_to(p) <= 0.0) {
                        inside += 1;
                    }
                }
            }
        }
        let sampled = inside as f64 / (n * n * n) as f64;
        assert!((volume - sampled).abs() < 2e-3, "exact {volume} vs sampled {sampled}");
    }

    #[test]
    fn degenerate_and_empty_polyhedra_have_no_volume() {
        assert_eq!(polyhedron_volume(&[], 1e-9), 0.0);
        // A box with min above max encloses nothing.
        let planes = box_planes(Vec3::splat(1.0), Vec3::ZERO);
        assert_eq!(polyhedron_volume(&planes, 1e-9), 0.0);
    }

    #[test]
    fn plane_distance_sign_marks_inside() {
        let p = Plane::new(Vec3::new(0.0, 0.0, 1.0), 16.0);
        assert!(p.distance_to(Vec3::new(0.0, 0.0, 8.0)) < 0.0);
        assert!(p.distance_to(Vec3::new(0.0, 0.0, 24.0)) > 0.0);
    }
}
