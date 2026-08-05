//! Recovering sub-block detail as slabs and stairs.
//!
//! A block is a 1 m cube, so at 16 units per block every 8-unit step, kerb and
//! window ledge in a Source map rounds to a whole block or vanishes. Minecraft
//! already has half-height and stepped blocks, and every schematic tool
//! carries them natively, so fitting geometry to slabs and stairs buys back a
//! factor of two vertically for nothing.
//!
//! The evidence used is a 2x2x2 occupancy mask recorded while voxelizing: one
//! bit per octant of the voxel, merged across every brush that touches it. It
//! is a byte per voxel and, unlike the brushes themselves, it survives into
//! the hollowing pass where the shape is finally chosen.
//!
//! Bit order is `x + 2*z + 4*y`, so the low nibble is the bottom half.

use crate::voxel::grid::IVec3;

/// A shape a voxel can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    /// The whole cube.
    Full,
    /// A half-height slab.
    Slab { top: bool },
    /// A step, facing the direction its riser looks toward.
    Stairs { facing: Facing, top: bool },
}

/// Which way a stair block faces.
///
/// Minecraft's `facing` points toward the *raised* side: the vanilla
/// `stairs.json` model puts its raised box at x 8..16, the east half, and the
/// blockstate maps `facing=east` to the unrotated model. Getting this backwards
/// builds every staircase inside out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Facing {
    North,
    South,
    East,
    West,
}

impl Facing {
    fn name(self) -> &'static str {
        match self {
            Facing::North => "north",
            Facing::South => "south",
            Facing::East => "east",
            Facing::West => "west",
        }
    }
}

impl Shape {
    /// The block state suffix for this shape, or none for a full cube.
    ///
    /// Sponge v3 palette entries are full block-state strings, so this is
    /// simply appended to a block id.
    pub fn state(self) -> Option<String> {
        match self {
            Shape::Full => None,
            Shape::Slab { top } => Some(format!(
                "[type={},waterlogged=false]",
                if top { "top" } else { "bottom" }
            )),
            Shape::Stairs { facing, top } => Some(format!(
                "[facing={},half={},shape=straight,waterlogged=false]",
                facing.name(),
                if top { "top" } else { "bottom" }
            )),
        }
    }

    /// Which variant of a block this shape needs.
    pub fn variant(self) -> Variant {
        match self {
            Shape::Full => Variant::Full,
            Shape::Slab { .. } => Variant::Slab,
            Shape::Stairs { .. } => Variant::Stairs,
        }
    }
}

/// The kind of block a shape needs to exist as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Full,
    Slab,
    Stairs,
}

/// Bit index of an octant. `x` and `z` select the horizontal quarter, `y` the
/// half.
pub const fn octant(x: bool, z: bool, y: bool) -> u8 {
    1 << ((x as u8) + 2 * (z as u8) + 4 * (y as u8))
}

/// Every bit of the bottom half.
const BOTTOM: u8 = 0b0000_1111;
/// Every bit of the top half.
const TOP: u8 = 0b1111_0000;

/// The four horizontal quarters, as (x, z) and the bit pair they occupy.
const QUARTERS: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

/// Choose the shape that best matches an occupancy mask.
///
/// Anything not recognised stays a full cube. That direction is deliberate:
/// an unrecognised mask can only ever cost detail, never open a hole in a
/// wall, and a hole is far more noticeable than a missing step.
pub fn shape_for(mask: u8) -> Shape {
    if mask == u8::MAX || mask == 0 {
        return Shape::Full;
    }

    // A clean half is a slab.
    if mask == BOTTOM {
        return Shape::Slab { top: false };
    }
    if mask == TOP {
        return Shape::Slab { top: true };
    }

    // A step is one full half plus two adjacent quarters of the other, so the
    // raised part spans a whole edge rather than a single corner.
    for (half, top) in [(BOTTOM, false), (TOP, true)] {
        let other = !half;
        if mask & half != half {
            continue;
        }
        let raised = mask & other;
        if raised == 0 || raised == other {
            continue;
        }
        // The raised quarters live in the *other* half from the solid one.
        if let Some(facing) = facing_of(raised, !top) {
            return Shape::Stairs { facing, top };
        }
    }

    Shape::Full
}

/// Which way a step faces, given the quarters of its raised half.
///
/// `raised_half` says which half those quarters sit in. The raised part must
/// cover exactly one edge of the block: two quarters sharing an x or a z. A
/// single corner or a diagonal pair is not a straight stair, and inner and
/// outer corner shapes are deliberately not fitted — they need neighbour
/// agreement to look right, and getting them wrong is more visible than
/// leaving a full block.
fn facing_of(raised: u8, raised_half: bool) -> Option<Facing> {
    let filled: Vec<(bool, bool)> = QUARTERS
        .iter()
        .copied()
        .filter(|(x, z)| raised & octant(*x, *z, raised_half) != 0)
        .collect();
    if filled.len() != 2 {
        return None;
    }

    // `facing` points at the raised side, and in Minecraft +X is east and
    // +Z is south.
    let (a, b) = (filled[0], filled[1]);
    if a.0 == b.0 {
        // Both quarters share an x, so the raised edge is on that side.
        Some(if a.0 { Facing::East } else { Facing::West })
    } else if a.1 == b.1 {
        Some(if a.1 { Facing::South } else { Facing::North })
    } else {
        // Diagonal quarters: not a straight stair.
        None
    }
}

/// A grid of per-voxel occupancy masks, parallel to the block grid.
///
/// Stored in the same sparse 16-cubed sections the block grid uses, for the
/// same reason: one byte per cell in an allocated section, rather than a hash
/// entry per voxel. Keyed individually this cost about 48 bytes a voxel, and
/// on Entropy: Zero 2's largest map that was 6 GB against 657 MB for the whole
/// rest of the conversion.
#[derive(Debug, Default, Clone)]
pub struct MaskGrid {
    sections: std::collections::HashMap<IVec3, Vec<u8>>,
}

const SECTION_BITS: i32 = 4;
const SECTION_SIZE: i32 = 1 << SECTION_BITS;
const SECTION_MASK: i32 = SECTION_SIZE - 1;
const SECTION_VOLUME: usize = (SECTION_SIZE * SECTION_SIZE * SECTION_SIZE) as usize;

fn section_of(pos: IVec3) -> IVec3 {
    [
        pos[0] >> SECTION_BITS,
        pos[1] >> SECTION_BITS,
        pos[2] >> SECTION_BITS,
    ]
}

fn index_in_section(pos: IVec3) -> usize {
    let x = (pos[0] & SECTION_MASK) as usize;
    let y = (pos[1] & SECTION_MASK) as usize;
    let z = (pos[2] & SECTION_MASK) as usize;
    (y << (SECTION_BITS * 2)) | (z << SECTION_BITS) | x
}

impl MaskGrid {
    pub fn new() -> MaskGrid {
        MaskGrid::default()
    }

    /// Record that part of a voxel is occupied.
    pub fn add(&mut self, pos: IVec3, mask: u8) {
        self.sections
            .entry(section_of(pos))
            .or_insert_with(|| vec![0; SECTION_VOLUME])[index_in_section(pos)] |= mask;
    }

    /// A voxel nothing was recorded for reads as fully solid, so geometry that
    /// never went through the mask pass keeps its full cube.
    pub fn get(&self, pos: IVec3) -> u8 {
        match self.sections.get(&section_of(pos)) {
            Some(section) => match section[index_in_section(pos)] {
                0 => u8::MAX,
                mask => mask,
            },
            None => u8::MAX,
        }
    }

    /// Voxels with a recorded mask.
    pub fn len(&self) -> usize {
        self.sections
            .values()
            .map(|s| s.iter().filter(|m| **m != 0).count())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    pub fn merge(&mut self, other: MaskGrid) {
        for (key, section) in other.sections {
            match self.sections.get_mut(&key) {
                Some(existing) => {
                    for (a, b) in existing.iter_mut().zip(section) {
                        *a |= b;
                    }
                }
                None => {
                    self.sections.insert(key, section);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_or_empty_mask_is_a_full_block() {
        assert_eq!(shape_for(u8::MAX), Shape::Full);
        assert_eq!(shape_for(0), Shape::Full);
    }

    #[test]
    fn clean_halves_are_slabs() {
        assert_eq!(shape_for(BOTTOM), Shape::Slab { top: false });
        assert_eq!(shape_for(TOP), Shape::Slab { top: true });
    }

    /// A step is a full half plus one edge of the other. `facing` points at
    /// the raised side, with +X east and +Z south.
    #[test]
    fn an_edge_on_a_half_is_a_stair() {
        // Bottom half full, plus the two upper quarters at z = 0, the north
        // side. The raised side is north, so the stair faces north.
        let mask = BOTTOM | octant(false, false, true) | octant(true, false, true);
        assert_eq!(shape_for(mask), Shape::Stairs { facing: Facing::North, top: false });

        // The mirror: raised on the south side.
        let mask = BOTTOM | octant(false, true, true) | octant(true, true, true);
        assert_eq!(shape_for(mask), Shape::Stairs { facing: Facing::South, top: false });

        // Raised along x = 0, the west side.
        let mask = BOTTOM | octant(false, false, true) | octant(false, true, true);
        assert_eq!(shape_for(mask), Shape::Stairs { facing: Facing::West, top: false });

        let mask = BOTTOM | octant(true, false, true) | octant(true, true, true);
        assert_eq!(shape_for(mask), Shape::Stairs { facing: Facing::East, top: false });

        // Upside-down: top half full plus a lower edge on the north side.
        let mask = TOP | octant(false, false, false) | octant(true, false, false);
        assert_eq!(shape_for(mask), Shape::Stairs { facing: Facing::North, top: true });
    }

    /// A single raised corner, or two diagonal ones, is not a straight stair.
    #[test]
    fn corners_and_diagonals_stay_full_blocks() {
        assert_eq!(shape_for(BOTTOM | octant(false, false, true)), Shape::Full);
        let diagonal = BOTTOM | octant(false, false, true) | octant(true, true, true);
        assert_eq!(shape_for(diagonal), Shape::Full);
    }

    /// The property that matters: whatever the mask, a shape comes out, it
    /// never panics, and its block state is well formed.
    #[test]
    fn every_possible_mask_yields_a_usable_shape() {
        for mask in 0..=u8::MAX {
            let shape = shape_for(mask);
            match shape.state() {
                None => assert_eq!(shape, Shape::Full),
                Some(state) => {
                    assert!(state.starts_with('[') && state.ends_with(']'), "{state}");
                    assert!(state.contains("waterlogged=false"), "{state}");
                }
            }
        }
    }

    /// Fitting must never remove material a full block would have had in a
    /// place the mask says is empty — the conservative direction is to keep
    /// more, not less.
    #[test]
    fn a_fitted_shape_never_claims_an_empty_octant() {
        for mask in 0..=u8::MAX {
            let covered = match shape_for(mask) {
                Shape::Full => u8::MAX,
                Shape::Slab { top: false } => BOTTOM,
                Shape::Slab { top: true } => TOP,
                Shape::Stairs { .. } => mask,
            };
            // Every bit the mask sets must still be covered by the shape.
            assert_eq!(mask & !covered, 0, "shape for {mask:08b} loses occupied space");
        }
    }

    #[test]
    fn slab_and_stair_states_are_minecraft_syntax() {
        assert_eq!(
            Shape::Slab { top: false }.state().unwrap(),
            "[type=bottom,waterlogged=false]"
        );
        assert_eq!(
            Shape::Stairs { facing: Facing::North, top: true }.state().unwrap(),
            "[facing=north,half=top,shape=straight,waterlogged=false]"
        );
    }

    #[test]
    fn shapes_map_to_the_block_variant_they_need() {
        assert_eq!(Shape::Full.variant(), Variant::Full);
        assert_eq!(Shape::Slab { top: false }.variant(), Variant::Slab);
        assert_eq!(
            Shape::Stairs { facing: Facing::North, top: false }.variant(),
            Variant::Stairs
        );
    }

    #[test]
    fn masks_merge_by_union() {
        let mut a = MaskGrid::new();
        a.add([0, 0, 0], 0b0000_0011);
        let mut b = MaskGrid::new();
        b.add([0, 0, 0], 0b0000_1100);
        b.add([1, 0, 0], TOP);

        a.merge(b);
        assert_eq!(a.get([0, 0, 0]), 0b0000_1111);
        assert_eq!(a.get([1, 0, 0]), TOP);
        assert_eq!(a.len(), 2);
    }

    /// Merging must union across sections that only one side has, and across
    /// negative coordinates, which is where a sectioned layout goes wrong.
    #[test]
    fn merging_spans_sections_and_negative_coordinates() {
        let spread = [[-1, -1, -1], [-17, 3, -33], [0, 0, 0], [100, 200, 300]];
        let mut a = MaskGrid::new();
        let mut b = MaskGrid::new();
        for (i, pos) in spread.iter().enumerate() {
            if i % 2 == 0 {
                a.add(*pos, BOTTOM);
            } else {
                b.add(*pos, TOP);
            }
        }
        // One position recorded in both halves must end up unioned.
        a.add([-17, 3, -33], BOTTOM);

        a.merge(b);
        assert_eq!(a.get([-1, -1, -1]), BOTTOM);
        assert_eq!(a.get([-17, 3, -33]), BOTTOM | TOP);
        assert_eq!(a.get([0, 0, 0]), BOTTOM);
        assert_eq!(a.get([100, 200, 300]), TOP);
        // Anything untouched, including inside an allocated section.
        assert_eq!(a.get([1, 1, 1]), u8::MAX);
        assert_eq!(a.get([9999, 0, 0]), u8::MAX);
    }

    /// Positions within one section must not alias onto each other.
    #[test]
    fn every_cell_of_a_section_is_distinct() {
        let mut grid = MaskGrid::new();
        let mut expected = Vec::new();
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    // A distinct non-zero mask per cell, cycling through 1..=255.
                    let mask = (1 + ((x * 256 + y * 16 + z) % 255)) as u8;
                    grid.add([x, y, z], mask);
                    expected.push(([x, y, z], mask));
                }
            }
        }
        for (pos, mask) in expected {
            assert_eq!(grid.get(pos), mask, "at {pos:?}");
        }
        assert_eq!(grid.len(), 16 * 16 * 16);
    }

    /// A voxel nothing recorded a mask for must read as fully solid, or every
    /// block that predates the mask grid would turn into a slab.
    #[test]
    fn an_unrecorded_voxel_is_full() {
        assert_eq!(MaskGrid::new().get([5, 5, 5]), u8::MAX);
        assert_eq!(shape_for(MaskGrid::new().get([5, 5, 5])), Shape::Full);
    }
}
