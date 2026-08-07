//! Hollowing out solid volumes.
//!
//! Voxelized brushes are solid, which is unusable at map scale: a filled E:Z2
//! map is billions of blocks. Only the voxels near a surface are ever visible,
//! so everything deeper than the configured shell is dropped.

use crate::voxel::grid::{IVec3, VoxelGrid};
use std::collections::HashSet;

/// The six face-adjacent offsets.
const FACE_NEIGHBOURS: [IVec3; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

fn neighbours(neighborhood: u8) -> Vec<IVec3> {
    if neighborhood <= 6 {
        return FACE_NEIGHBOURS.to_vec();
    }
    let mut out = Vec::with_capacity(26);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if (x, y, z) != (0, 0, 0) {
                    out.push([x, y, z]);
                }
            }
        }
    }
    out
}

/// Keep only voxels within `thickness` steps of an air voxel.
///
/// A `thickness` of 0 returns the grid unchanged.
pub fn hollow(grid: &VoxelGrid, thickness: u32, neighborhood: u8) -> VoxelGrid {
    if thickness == 0 {
        return grid.clone();
    }
    let offsets = neighbours(neighborhood);

    // The outermost layer: solid voxels touching air.
    let mut shell: HashSet<IVec3> = grid
        .iter()
        .map(|(pos, _)| pos)
        .filter(|pos| {
            offsets.iter().any(|offset| {
                !grid.is_solid([pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]])
            })
        })
        .collect();

    // Then grow inwards one layer at a time.
    let mut frontier: Vec<IVec3> = shell.iter().copied().collect();
    for _ in 1..thickness {
        let mut next = Vec::new();
        for pos in &frontier {
            for offset in &offsets {
                let neighbour = [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]];
                if grid.is_solid(neighbour) && shell.insert(neighbour) {
                    next.push(neighbour);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    let mut out = VoxelGrid::new();
    for pos in shell {
        out.set(pos, grid.get(pos));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid cube of side `n` at the origin.
    fn solid_cube(n: i32) -> VoxelGrid {
        let mut grid = VoxelGrid::new();
        for x in 0..n {
            for y in 0..n {
                for z in 0..n {
                    grid.set([x, y, z], 1);
                }
            }
        }
        grid
    }

    #[test]
    fn hollowing_a_cube_leaves_its_surface() {
        let grid = solid_cube(6);
        let shell = hollow(&grid, 1, 6);
        // 6³ solid minus a 4³ interior.
        assert_eq!(shell.count(), 6 * 6 * 6 - 4 * 4 * 4);
        assert!(shell.is_solid([0, 0, 0]));
        assert!(
            !shell.is_solid([3, 3, 3]),
            "the centre should be hollowed out"
        );
    }

    #[test]
    fn thicker_shells_keep_more() {
        let grid = solid_cube(8);
        let thin = hollow(&grid, 1, 6);
        let thick = hollow(&grid, 2, 6);
        assert_eq!(thin.count(), 8 * 8 * 8 - 6 * 6 * 6);
        assert_eq!(thick.count(), 8 * 8 * 8 - 4 * 4 * 4);
        assert!(thick.count() > thin.count());
    }

    #[test]
    fn a_shell_thicker_than_the_solid_keeps_everything() {
        let grid = solid_cube(3);
        assert_eq!(hollow(&grid, 10, 6).count(), grid.count());
    }

    #[test]
    fn thickness_zero_is_a_no_op() {
        let grid = solid_cube(4);
        assert_eq!(hollow(&grid, 0, 6).count(), grid.count());
    }

    #[test]
    fn block_ids_are_carried_through() {
        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], 7);
        grid.set([1, 0, 0], 9);
        let shell = hollow(&grid, 1, 6);
        assert_eq!(shell.get([0, 0, 0]), 7);
        assert_eq!(shell.get([1, 0, 0]), 9);
    }

    #[test]
    fn a_one_voxel_thick_wall_survives_hollowing() {
        // Every voxel of a flat wall touches air, so nothing may be removed.
        let mut grid = VoxelGrid::new();
        for x in 0..10 {
            for y in 0..10 {
                grid.set([x, y, 0], 1);
            }
        }
        assert_eq!(hollow(&grid, 1, 6).count(), 100);
    }

    #[test]
    fn the_26_neighbourhood_hollows_at_least_as_much() {
        let grid = solid_cube(7);
        assert!(hollow(&grid, 1, 26).count() >= hollow(&grid, 1, 6).count());
    }
}
