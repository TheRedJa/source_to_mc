//! Versioned bitmask of cells that block light without holding a block.
//!
//! A brush drawn as geometry rather than voxelized leaves its cells empty, and
//! Minecraft only ever blocks light with a block, so a ceiling built from thin
//! plates stops casting any shade at all. Rather than fill those cells with
//! invisible blocks — which would also make them solid to walk into — the cells
//! are recorded here and the mod teaches its light engine to treat them as
//! opaque. One bit per cell, in the same 16³ sections the surface table uses.

use crate::voxel::grid::IVec3;
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub const MAGIC: [u8; 8] = *b"S2OCCL\0\0";
pub const VERSION: u32 = 1;
/// 16³ bits, one per cell of a section.
pub const SECTION_BYTES: usize = 512;

/// Encode occluding cells into sparse 16³ section buckets. Input order is
/// deliberately irrelevant to the output bytes.
pub fn encode(cells: &BTreeSet<IVec3>) -> Result<Vec<u8>> {
    let mut buckets: BTreeMap<IVec3, [u8; SECTION_BYTES]> = BTreeMap::new();
    for cell in cells {
        let bits = buckets
            .entry(crate::output::surface::section_of(*cell))
            .or_insert([0; SECTION_BYTES]);
        let index = local_index(*cell) as usize;
        bits[index >> 3] |= 1 << (index & 7);
    }
    ensure!(
        buckets.len() as u64 <= crate::output::limits::MAX_SECTIONS_PER_MAP,
        "occlusion section limit exceeded"
    );

    let mut out = Vec::with_capacity(16 + buckets.len() * (12 + SECTION_BYTES));
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(buckets.len() as u32).to_le_bytes());
    for (section, bits) in buckets {
        for coordinate in section {
            out.extend_from_slice(&coordinate.to_le_bytes());
        }
        out.extend_from_slice(&bits);
    }
    Ok(out)
}

/// Bit position of a cell within its section: X low, then Z, then Y, matching
/// the surface table's local-cell packing.
fn local_index(cell: IVec3) -> u16 {
    ((cell[1] & 15) << 8 | (cell[2] & 15) << 4 | (cell[0] & 15)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_sections(bytes: &[u8]) -> Vec<(IVec3, Vec<u8>)> {
        assert_eq!(&bytes[0..8], &MAGIC);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), VERSION);
        let count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut at = 16;
        let mut out = Vec::new();
        for _ in 0..count {
            let mut section = [0i32; 3];
            for value in &mut section {
                *value = i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                at += 4;
            }
            out.push((section, bytes[at..at + SECTION_BYTES].to_vec()));
            at += SECTION_BYTES;
        }
        assert_eq!(at, bytes.len(), "trailing bytes");
        out
    }

    fn set(bits: &[u8], cell: IVec3) -> bool {
        let index = local_index(cell) as usize;
        bits[index >> 3] & (1 << (index & 7)) != 0
    }

    #[test]
    fn an_empty_mask_is_just_a_header() {
        let bytes = encode(&BTreeSet::new()).unwrap();
        assert_eq!(bytes.len(), 16);
        assert!(decode_sections(&bytes).is_empty());
    }

    #[test]
    fn a_cell_comes_back_set_and_its_neighbours_do_not() {
        let cell = [3, -20, 17];
        let bytes = encode(&BTreeSet::from([cell])).unwrap();
        let sections = decode_sections(&bytes);
        assert_eq!(sections.len(), 1);
        let (section, bits) = &sections[0];
        assert_eq!(*section, [0, -2, 1]);
        assert!(set(bits, cell));
        assert!(!set(bits, [4, -20, 17]));
        assert!(!set(bits, [3, -19, 17]));
        assert!(!set(bits, [3, -20, 18]));
    }

    /// Cells landing in different sections must not be folded together by the
    /// wrap that turns a world cell into a bit index.
    #[test]
    fn cells_a_section_apart_stay_apart() {
        let bytes = encode(&BTreeSet::from([[0, 0, 0], [16, 0, 0]])).unwrap();
        let sections = decode_sections(&bytes);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, [0, 0, 0]);
        assert_eq!(sections[1].0, [1, 0, 0]);
        assert!(set(&sections[0].1, [0, 0, 0]));
        assert!(set(&sections[1].1, [16, 0, 0]));
    }

    #[test]
    fn the_bytes_do_not_depend_on_insertion_order() {
        let cells = [[1, 2, 3], [40, -5, 9], [-17, 2, 3]];
        let forward: BTreeSet<IVec3> = cells.iter().copied().collect();
        let backward: BTreeSet<IVec3> = cells.iter().rev().copied().collect();
        assert_eq!(encode(&forward).unwrap(), encode(&backward).unwrap());
    }
}
