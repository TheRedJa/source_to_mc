//! Sitting a prop on the floor the conversion actually built.
//!
//! A prop is placed at the exact position Source gave it, to a fraction of a
//! block. The floor under it is not: brushes are voxelized, so a floor at
//! 64.5 Source units becomes a layer of blocks whose surface is at a whole
//! number. The two disagree by up to a block, and since a mapper puts a crate
//! *on* the floor rather than a block above it, the disagreement almost always
//! goes one way — the crate sinks into the ground.
//!
//! Cubing the prop hid this, because the cubes were quantized by the same
//! grid. Drawing the real mesh does not.
//!
//! The fix is to measure where the floor ended up and move the prop to meet
//! it. Deliberately a small correction and not a physics drop: it only ever
//! looks a short distance, and a prop with nothing under it is left exactly
//! where the mapper put it, because a hanging lamp or a sign bolted to a wall
//! is not a thing that should fall.

use crate::geom::Aabb;
use crate::voxel::grid::VoxelGrid;

/// Columns sampled across a prop's footprint, per axis.
///
/// The median of these decides where the floor is, so it wants to be enough
/// to outvote the odd column that lands on a wall the prop is standing
/// against, and small enough that a map's worth of props costs nothing.
const SAMPLES: i32 = 5;

/// How far in from the edge the sampled columns start, as a fraction.
///
/// A crate pushed against a wall has the wall inside the very edge of its
/// bounding box, and a column there reads the wall's top as the floor. Coming
/// in from the edge measures what the prop is standing on instead.
const INSET: f64 = 0.2;

/// How far a prop is moved to meet the floor, in blocks.
///
/// The correction only exists to undo the grid's own rounding, so a block
/// either way covers it. Anything larger is not a rounding error — it is a
/// prop over a hole or on a ledge the conversion did not build — and moving it
/// that far would be inventing a position rather than recovering one.
pub fn offset(grid: &VoxelGrid, bounds: Aabb, max_shift: f64) -> f64 {
    if bounds.is_empty() || max_shift <= 0.0 {
        return 0.0;
    }

    let base = bounds.min.y;
    // Only blocks within reach of the base can be what the prop stands on.
    let lowest = (base - max_shift).floor() as i32;
    let highest = (base + max_shift).floor() as i32;

    let mut supports: Vec<f64> = Vec::new();
    for (x, z) in columns(bounds) {
        // The top surface of the highest block in reach: a block at y fills
        // y..y+1, so its surface is y+1.
        let support = (lowest..=highest)
            .rev()
            .find(|y| grid.is_solid([x, *y, z]))
            .map(|y| f64::from(y + 1));
        if let Some(support) = support {
            supports.push(support);
        }
    }

    // Nothing underneath within reach: the prop is not standing on anything
    // this conversion built, so leave it exactly where the map put it.
    if supports.len() * 2 <= (SAMPLES * SAMPLES) as usize {
        return 0.0;
    }

    // The median rather than the highest. A prop half over a step or brushing
    // a wall has a few columns reading much higher than the floor it is really
    // on, and following those would lift it into the air.
    supports.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = supports[supports.len() / 2];
    let shift = (floor - base).clamp(-max_shift, max_shift);

    // Lifting a prop out of the floor must not drive its head into the
    // ceiling. Under a low beam there may be less headroom than the floor is
    // asking for, and half in the ground beats half in the roof.
    if shift > 0.0 {
        shift.min(headroom(grid, bounds, shift))
    } else {
        shift
    }
}

/// How far the top of `bounds` can rise before it meets something solid.
fn headroom(grid: &VoxelGrid, bounds: Aabb, wanted: f64) -> f64 {
    let top = bounds.max.y;
    let lowest = top.floor() as i32;
    let highest = (top + wanted).floor() as i32;

    let mut room = wanted;
    for (x, z) in columns(bounds) {
        for y in lowest..=highest {
            // A block at y fills y..y+1. If any of that is above the prop's
            // top, the top may rise as far as the block's underside — and no
            // distance at all if the block is already around it.
            if f64::from(y + 1) > top && grid.is_solid([x, y, z]) {
                room = room.min((f64::from(y) - top).max(0.0));
                break;
            }
        }
    }
    room.max(0.0)
}

/// The block columns sampled under a footprint.
fn columns(bounds: Aabb) -> Vec<(i32, i32)> {
    let span = |lo: f64, hi: f64, step: i32| -> f64 {
        let inset = (hi - lo) * INSET;
        let (lo, hi) = (lo + inset, hi - inset);
        if SAMPLES <= 1 {
            (lo + hi) / 2.0
        } else {
            lo + (hi - lo) * f64::from(step) / f64::from(SAMPLES - 1)
        }
    };

    let mut out = Vec::with_capacity((SAMPLES * SAMPLES) as usize);
    for i in 0..SAMPLES {
        for j in 0..SAMPLES {
            out.push((
                span(bounds.min.x, bounds.max.x, i).floor() as i32,
                span(bounds.min.z, bounds.max.z, j).floor() as i32,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec3;
    use crate::voxel::grid::Palette;

    /// A floor of blocks filling y = 0, so its surface is at y = 1.
    fn floor() -> (VoxelGrid, Palette) {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let mut grid = VoxelGrid::new();
        for x in -10..10 {
            for z in -10..10 {
                grid.set([x, 0, z], stone);
            }
        }
        (grid, palette)
    }

    fn crate_at(y: f64) -> Aabb {
        Aabb::new(Vec3::new(0.0, y, 0.0), Vec3::new(2.0, y + 2.0, 2.0))
    }

    /// The whole point: a prop whose base fell below the voxelized floor comes
    /// back up to stand on it.
    #[test]
    fn a_sunken_prop_is_lifted_onto_the_floor() {
        let (grid, _) = floor();
        assert!((offset(&grid, crate_at(0.4), 1.5) - 0.6).abs() < 1e-9);
        assert!((offset(&grid, crate_at(0.0), 1.5) - 1.0).abs() < 1e-9);
    }

    /// And one left hanging just above it comes down.
    #[test]
    fn a_floating_prop_is_dropped_onto_the_floor() {
        let (grid, _) = floor();
        assert!((offset(&grid, crate_at(1.7), 1.5) + 0.7).abs() < 1e-9);
    }

    #[test]
    fn a_prop_already_on_the_floor_is_left_alone() {
        let (grid, _) = floor();
        assert_eq!(offset(&grid, crate_at(1.0), 1.5), 0.0);
    }

    /// A hanging lamp or a sign on a wall has nothing under it, and dropping
    /// it to the ground would be far worse than leaving it where it was.
    #[test]
    fn a_prop_with_nothing_under_it_does_not_move() {
        let (grid, _) = floor();
        assert_eq!(offset(&grid, crate_at(20.0), 1.5), 0.0);
        assert_eq!(offset(&VoxelGrid::new(), crate_at(0.4), 1.5), 0.0);
    }

    /// The correction undoes the grid's rounding and nothing more, so a prop
    /// over a hole stays where the mapper put it rather than sinking to
    /// whatever is far below.
    #[test]
    fn the_shift_never_exceeds_the_limit() {
        let (grid, _) = floor();
        for base in [-8.0, -3.0, 0.2, 4.0, 9.0] {
            let shift = offset(&grid, crate_at(base), 1.0);
            assert!(shift.abs() <= 1.0, "base {base} moved by {shift}");
        }
    }

    /// A crate against a wall must not be lifted onto the wall. Most of its
    /// footprint is over floor, and the median follows the majority.
    #[test]
    fn standing_against_a_wall_does_not_climb_it() {
        let (mut grid, mut palette) = floor();
        let stone = palette.intern("minecraft:stone");
        // A wall filling the column at x = 2, up past the crate's top.
        for y in 1..6 {
            for z in -10..10 {
                grid.set([2, y, z], stone);
            }
        }
        let shift = offset(&grid, crate_at(0.4), 1.5);
        assert!(
            (shift - 0.6).abs() < 1e-9,
            "the wall pulled the crate up by {shift} instead of 0.6"
        );
    }

    /// A prop standing on a step reads two different heights; following the
    /// higher one would leave half of it floating, so the majority wins.
    #[test]
    fn a_prop_half_on_a_step_follows_the_larger_share() {
        let (mut grid, mut palette) = floor();
        let stone = palette.intern("minecraft:stone");
        // One column of the five raised by a block.
        for z in -10..10 {
            grid.set([0, 1, z], stone);
        }
        let shift = offset(&grid, crate_at(0.9), 1.5);
        assert!(
            (shift - 0.1).abs() < 1e-9,
            "shifted by {shift}, expected 0.1"
        );
    }

    /// Under a low ceiling there may be less headroom than the floor asks
    /// for, and a prop half in the roof is worse than one half in the ground.
    #[test]
    fn lifting_a_prop_stops_at_the_ceiling() {
        let (mut grid, mut palette) = floor();
        let stone = palette.intern("minecraft:stone");
        // A ceiling whose underside is at y = 2.6's block, i.e. blocks at y=3.
        for x in -10..10 {
            for z in -10..10 {
                grid.set([x, 3, z], stone);
            }
        }
        // Base 0.4, top 2.4: the floor wants +0.6, but only 0.6 of headroom
        // exists to y = 3.0, so it just fits.
        assert!((offset(&grid, crate_at(0.4), 1.5) - 0.6).abs() < 1e-9);

        // Now with the ceiling one block lower there is no room at all.
        let mut tight = grid.clone();
        for x in -10..10 {
            for z in -10..10 {
                tight.set([x, 2, z], stone);
            }
        }
        assert_eq!(
            offset(&tight, crate_at(0.4), 1.5),
            0.0,
            "pushed into the ceiling"
        );
    }

    /// Dropping a prop is never blocked by a ceiling it is moving away from.
    #[test]
    fn headroom_does_not_interfere_with_dropping() {
        let (mut grid, mut palette) = floor();
        let stone = palette.intern("minecraft:stone");
        for x in -10..10 {
            for z in -10..10 {
                grid.set([x, 4, z], stone);
            }
        }
        assert!((offset(&grid, crate_at(1.7), 1.5) + 0.7).abs() < 1e-9);
    }

    #[test]
    fn settling_can_be_turned_off_by_allowing_no_shift() {
        let (grid, _) = floor();
        assert_eq!(offset(&grid, crate_at(0.4), 0.0), 0.0);
    }
}
