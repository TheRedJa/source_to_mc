//! Canonical provenance for visible Minecraft shape surfaces.
//!
//! Occupancy and appearance are deliberately separate. Several Source solids
//! can contribute to one voxel, while exposed patches of a fitted slab or
//! stair can come from different Source sides. Candidates therefore retain
//! their Source planes until the final shape is known; provenance is resolved
//! at each actual patch centre, not once at the outer cube-face centre.

use crate::bsp::texcoord::BlockTexCoord;
use crate::geom::{Plane, Vec3};
use crate::voxel::grid::{IVec3, VoxelGrid};
use crate::voxel::shapes::{MaskGrid, Shape, octant, shape_for};
use std::cmp::Ordering;
use std::collections::HashMap;

const DIRECTIONS: [FaceDirection; 6] = [
    FaceDirection::Down,
    FaceDirection::Up,
    FaceDirection::North,
    FaceDirection::South,
    FaceDirection::West,
    FaceDirection::East,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaceDirection {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl FaceDirection {
    pub const ALL: [Self; 6] = DIRECTIONS;
    pub const fn offset(self) -> IVec3 {
        match self {
            Self::Down => [0, -1, 0],
            Self::Up => [0, 1, 0],
            Self::North => [0, 0, -1],
            Self::South => [0, 0, 1],
            Self::West => [-1, 0, 0],
            Self::East => [1, 0, 0],
        }
    }

    pub fn normal(self) -> Vec3 {
        match self {
            Self::Down => Vec3::new(0.0, -1.0, 0.0),
            Self::Up => Vec3::new(0.0, 1.0, 0.0),
            Self::North => Vec3::new(0.0, 0.0, -1.0),
            Self::South => Vec3::new(0.0, 0.0, 1.0),
            Self::West => Vec3::new(-1.0, 0.0, 0.0),
            Self::East => Vec3::new(1.0, 0.0, 0.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceProvenance {
    Brush {
        brush: usize,
        side: usize,
    },
    Displacement {
        displacement: usize,
        triangle: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceSource {
    pub provenance: SourceProvenance,
    pub material: usize,
    pub uv: BlockTexCoord,
}

#[derive(Debug, Clone, Copy)]
struct Candidate {
    source: FaceSource,
    plane: Plane,
}

fn closest_candidate<'a>(
    candidates: impl Iterator<Item = &'a Candidate>,
    centre: Vec3,
) -> Option<&'a Candidate> {
    candidates.min_by(|a, b| {
        let distance = |candidate: &Candidate| candidate.plane.distance_to(centre).abs();
        match distance(a).total_cmp(&distance(b)) {
            Ordering::Equal => a.source.provenance.cmp(&b.source.provenance),
            ordering => ordering,
        }
    })
}

#[derive(Debug, Clone, Default)]
pub struct FaceCandidates {
    cells: HashMap<IVec3, Vec<Candidate>>,
}

impl FaceCandidates {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, cell: IVec3, source: FaceSource, plane: Plane) {
        if !plane.dist.is_finite()
            || !plane.normal.x.is_finite()
            || !plane.normal.y.is_finite()
            || !plane.normal.z.is_finite()
        {
            return;
        }
        let candidates = self.cells.entry(cell).or_default();
        if candidates
            .iter()
            .any(|candidate| candidate.source.provenance == source.provenance)
        {
            return;
        }
        candidates.push(Candidate { source, plane });
    }

    pub fn merge(&mut self, other: Self) {
        for (cell, candidates) in other.cells {
            for candidate in candidates {
                self.add(cell, candidate.source, candidate.plane);
            }
        }
    }

    fn closest(&self, cell: IVec3, patch: FacePatch) -> Option<FaceSource> {
        let candidates = self.cells.get(&cell)?;
        let centre = patch.centre(cell);
        let normal = patch.direction.normal();
        let aligned = candidates
            .iter()
            .filter(|candidate| candidate.plane.normal.dot(normal) > 1.0e-9);
        if let Some(candidate) = closest_candidate(aligned, centre) {
            return Some(candidate.source);
        }

        // Voxelizing a slope or fitting a stair can introduce an axis-aligned
        // face for which no Source plane points forward. Preserve the old
        // nearest-plane fallback only in that case. When an aligned plane does
        // exist, as on the top of an ordinary floor brush, a closer wall plane
        // must never outrank it and project wall UVs across the floor.
        closest_candidate(candidates.iter(), centre).map(|candidate| candidate.source)
    }

    pub fn visible(
        &self,
        final_grid: &VoxelGrid,
        masks: &MaskGrid,
        shapes_enabled: bool,
    ) -> Vec<VisibleFaceRecord> {
        let mut records = Vec::new();
        for (cell, _) in final_grid.iter() {
            let shape = if shapes_enabled {
                shape_for(masks.get(cell))
            } else {
                Shape::Full
            };
            let mask = shape.occupancy_mask();
            for direction in DIRECTIONS {
                let offset = direction.offset();
                let neighbour = [
                    cell[0] + offset[0],
                    cell[1] + offset[1],
                    cell[2] + offset[2],
                ];
                let neighbour_mask = if !final_grid.is_solid(neighbour) {
                    0
                } else if shapes_enabled {
                    shape_for(masks.get(neighbour)).occupancy_mask()
                } else {
                    u8::MAX
                };
                for patch in exposed_patches(mask, neighbour_mask, direction) {
                    if let Some(source) = self.closest(cell, patch) {
                        records.push(VisibleFaceRecord {
                            cell,
                            shape,
                            patch,
                            source,
                        });
                    }
                }
            }
        }
        records.sort_by_key(|record| (record.cell, record.patch, record.source.provenance));
        records
    }
}

/// Axis-aligned rectangle in half-block coordinates (0 through 2). The normal
/// axis has equal minima and maxima and identifies the rectangle's plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FacePatch {
    pub direction: FaceDirection,
    pub min: [u8; 3],
    pub max: [u8; 3],
}

impl FacePatch {
    pub fn centre(self, cell: IVec3) -> Vec3 {
        Vec3::new(
            cell[0] as f64 + (self.min[0] + self.max[0]) as f64 / 4.0,
            cell[1] as f64 + (self.min[1] + self.max[1]) as f64 / 4.0,
            cell[2] as f64 + (self.min[2] + self.max[2]) as f64 / 4.0,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleFaceRecord {
    pub cell: IVec3,
    pub shape: Shape,
    pub patch: FacePatch,
    pub source: FaceSource,
}

fn occupied(mask: u8, x: u8, y: u8, z: u8) -> bool {
    mask & octant(x != 0, z != 0, y != 0) != 0
}

fn exposed_patches(mask: u8, neighbour: u8, direction: FaceDirection) -> Vec<FacePatch> {
    let mut out = Vec::new();
    for a in 0..2 {
        for b in 0..2 {
            let planes: [(bool, u8); 2] = match direction {
                FaceDirection::Down => [
                    (occupied(mask, a, 1, b) && !occupied(mask, a, 0, b), 1),
                    (occupied(mask, a, 0, b) && !occupied(neighbour, a, 1, b), 0),
                ],
                FaceDirection::Up => [
                    (occupied(mask, a, 0, b) && !occupied(mask, a, 1, b), 1),
                    (occupied(mask, a, 1, b) && !occupied(neighbour, a, 0, b), 2),
                ],
                FaceDirection::North => [
                    (occupied(mask, a, b, 1) && !occupied(mask, a, b, 0), 1),
                    (occupied(mask, a, b, 0) && !occupied(neighbour, a, b, 1), 0),
                ],
                FaceDirection::South => [
                    (occupied(mask, a, b, 0) && !occupied(mask, a, b, 1), 1),
                    (occupied(mask, a, b, 1) && !occupied(neighbour, a, b, 0), 2),
                ],
                FaceDirection::West => [
                    (occupied(mask, 1, b, a) && !occupied(mask, 0, b, a), 1),
                    (occupied(mask, 0, b, a) && !occupied(neighbour, 1, b, a), 0),
                ],
                FaceDirection::East => [
                    (occupied(mask, 0, b, a) && !occupied(mask, 1, b, a), 1),
                    (occupied(mask, 1, b, a) && !occupied(neighbour, 0, b, a), 2),
                ],
            };
            for (visible, plane) in planes {
                if visible {
                    out.push(micro_patch(direction, plane, a, b));
                }
            }
        }
    }
    out
}

fn micro_patch(direction: FaceDirection, plane: u8, a: u8, b: u8) -> FacePatch {
    let (min, max) = match direction {
        FaceDirection::Down | FaceDirection::Up => ([a, plane, b], [a + 1, plane, b + 1]),
        FaceDirection::North | FaceDirection::South => ([a, b, plane], [a + 1, b + 1, plane]),
        FaceDirection::West | FaceDirection::East => ([plane, b, a], [plane, b + 1, a + 1]),
    };
    FacePatch {
        direction,
        min,
        max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(brush: usize, side: usize, material: usize) -> FaceSource {
        FaceSource {
            provenance: SourceProvenance::Brush { brush, side },
            material,
            uv: BlockTexCoord {
                u: [material as f64, 0.0, 0.0, side as f64],
                v: [0.0, material as f64, 0.0, brush as f64],
            },
        }
    }
    #[test]
    fn corner_keeps_different_sources_on_each_surface() {
        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], 1);
        let mut c = FaceCandidates::new();
        c.add(
            [0, 0, 0],
            source(3, 4, 10),
            Plane::new(Vec3::new(0.0, 1.0, 0.0), 1.0),
        );
        c.add(
            [0, 0, 0],
            source(3, 5, 20),
            Plane::new(Vec3::new(0.0, 0.0, -1.0), 0.0),
        );
        c.add(
            [0, 0, 0],
            source(3, 6, 30),
            Plane::new(Vec3::new(1.0, 0.0, 0.0), 1.0),
        );
        let records = c.visible(&grid, &MaskGrid::new(), false);
        for (direction, material) in [
            (FaceDirection::Up, 10),
            (FaceDirection::North, 20),
            (FaceDirection::East, 30),
        ] {
            assert!(
                records
                    .iter()
                    .filter(|r| r.patch.direction == direction)
                    .all(|r| r.source.material == material)
            );
        }
    }
    #[test]
    fn outer_floor_patch_rejects_closer_wall_plane() {
        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], 1);
        let mut c = FaceCandidates::new();
        c.add(
            [0, 0, 0],
            source(690, 5, 28),
            Plane::new(Vec3::new(0.0, 1.0, 0.0), 1.0),
        );
        c.add(
            [0, 0, 0],
            source(690, 14, 1),
            Plane::new(Vec3::new(1.0, 0.0, 0.0), 0.25),
        );
        let records = c.visible(&grid, &MaskGrid::new(), false);
        assert!(
            records
                .iter()
                .filter(|record| record.patch.direction == FaceDirection::Up)
                .all(|record| record.source.material == 28)
        );
    }
    #[test]
    fn synthetic_face_without_a_forward_source_uses_nearest_fallback() {
        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], 1);
        let mut c = FaceCandidates::new();
        c.add(
            [0, 0, 0],
            source(690, 14, 1),
            Plane::new(Vec3::new(1.0, 0.0, 0.0), 0.25),
        );
        assert!(
            c.visible(&grid, &MaskGrid::new(), false)
                .iter()
                .filter(|record| record.patch.direction == FaceDirection::Up)
                .all(|record| record.source.material == 1)
        );
    }
    #[test]
    fn overlap_resolution_is_merge_order_independent() {
        let mut a = FaceCandidates::new();
        a.add(
            [1, 2, 3],
            source(8, 2, 80),
            Plane::new(Vec3::new(0.0, 0.0, 1.0), 4.25),
        );
        let mut b = FaceCandidates::new();
        b.add(
            [1, 2, 3],
            source(4, 7, 40),
            Plane::new(Vec3::new(0.0, 0.0, 1.0), 4.25),
        );
        let mut ab = a.clone();
        ab.merge(b.clone());
        let mut ba = b;
        ba.merge(a);
        let mut grid = VoxelGrid::new();
        grid.set([1, 2, 3], 1);
        assert_eq!(
            ab.visible(&grid, &MaskGrid::new(), false),
            ba.visible(&grid, &MaskGrid::new(), false)
        );
    }
    #[test]
    fn hollowed_cells_cannot_leak_records() {
        let mut original = VoxelGrid::new();
        let mut c = FaceCandidates::new();
        for x in 0..3 {
            for y in 0..3 {
                for z in 0..3 {
                    let cell = [x, y, z];
                    original.set(cell, 1);
                    c.add(
                        cell,
                        source(0, 0, 1),
                        Plane::new(Vec3::new(0.0, 1.0, 0.0), 3.0),
                    );
                }
            }
        }
        let shell = crate::voxel::shell::hollow(&original, 1, 6);
        assert!(!shell.is_solid([1, 1, 1]));
        assert!(
            c.visible(&shell, &MaskGrid::new(), false)
                .iter()
                .all(|r| r.cell != [1, 1, 1])
        );
    }
    #[test]
    fn stair_resolves_patches_at_their_own_centres() {
        let cell = [0, 0, 0];
        let mut grid = VoxelGrid::new();
        grid.set(cell, 1);
        let mut masks = MaskGrid::new();
        masks.add(
            cell,
            0b0000_1111 | octant(false, false, true) | octant(true, false, true),
        );
        let lower = source(1, 1, 10);
        let upper = source(2, 2, 20);
        let mut c = FaceCandidates::new();
        // Two Source planes cross this fitted stair. Sampling once at the
        // outer cube-face centre would tie; the two patch centres do not.
        c.add(cell, lower, Plane::new(Vec3::new(0.0, 1.0, 0.0), 0.25));
        c.add(cell, upper, Plane::new(Vec3::new(0.0, 1.0, 0.0), 0.75));
        let records = c.visible(&grid, &masks, true);
        let north: Vec<_> = records
            .iter()
            .filter(|r| r.patch.direction == FaceDirection::North)
            .collect();
        assert!(
            north
                .iter()
                .any(|r| r.patch.min[1] == 0 && r.source == lower)
        );
        assert!(
            north
                .iter()
                .any(|r| r.patch.min[1] == 1 && r.source == upper)
        );
    }
    #[test]
    fn internal_slab_face_survives_solid_neighbour() {
        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], 1);
        grid.set([0, 1, 0], 1);
        let mut masks = MaskGrid::new();
        masks.add([0, 0, 0], 0b0000_1111);
        masks.add([0, 1, 0], u8::MAX);
        let mut c = FaceCandidates::new();
        c.add(
            [0, 0, 0],
            source(1, 2, 3),
            Plane::new(Vec3::new(0.0, 1.0, 0.0), 0.5),
        );
        assert_eq!(
            c.visible(&grid, &masks, true)
                .iter()
                .filter(|r| r.patch.direction == FaceDirection::Up)
                .count(),
            4
        );
    }
    #[test]
    fn adjacent_full_blocks_hide_shared_face() {
        assert!(exposed_patches(u8::MAX, u8::MAX, FaceDirection::East).is_empty());
    }
}
