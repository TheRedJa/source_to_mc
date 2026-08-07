//! Raw parsing of the BSP leaf lump.
//!
//! `vbsp` sorts its `leaves` vector by cluster (see `Leaves::new`), which makes
//! `Bsp::leaf(n)` unusable for tree traversal: BSP node children reference
//! leaves by their *original* index. Associating brushes with the model they
//! belong to requires walking each model's node subtree down to leaves, so we
//! read that one lump ourselves, in file order.
//!
//! Only the leaf-brush range is extracted; everything else still comes from
//! `vbsp`.

use super::lumps::{self, LUMP_LEAFS};
use anyhow::{Result, bail, ensure};

/// Leaf layout is identical across versions up to the light cube; version 0
/// carries a `CompressedLightCube` that version 1 omits.
const LEAF_STRIDE_V0: usize = 56;
const LEAF_STRIDE_V1: usize = 32;
/// Offset of `firstleafbrush` within a leaf; `numleafbrushes` follows it.
///
/// `dleaf_t` runs: contents (4), cluster (2), packed area/flags (2),
/// mins (6), maxs (6), firstleafface (2), numleaffaces (2) — putting
/// firstleafbrush at byte 24.
const LEAF_FIRST_BRUSH_OFFSET: usize = 24;

/// The leaf-brush range of a single leaf, in original BSP order.
#[derive(Debug, Clone, Copy)]
pub struct LeafBrushRange {
    pub first: u16,
    pub count: u16,
}

/// Read every leaf's brush range from `data`, preserving BSP leaf indices.
pub fn leaf_brush_ranges(data: &[u8]) -> Result<Vec<LeafBrushRange>> {
    let entry = lumps::lump_entry(data, LUMP_LEAFS)?;

    if entry.four_cc != 0 {
        bail!("leaf lump is LZMA-compressed (console map?); not supported");
    }

    let stride = match entry.version {
        0 => LEAF_STRIDE_V0,
        1 => LEAF_STRIDE_V1,
        v => bail!("unsupported leaf lump version {v}"),
    };

    let length = entry.length;
    ensure!(
        length % stride == 0,
        "leaf lump length {length} is not a multiple of the version-{} stride {stride}",
        entry.version
    );

    let lump = &data[entry.range()];
    let mut ranges = Vec::with_capacity(length / stride);
    for leaf in lump.chunks_exact(stride) {
        let at = LEAF_FIRST_BRUSH_OFFSET;
        ranges.push(LeafBrushRange {
            first: u16::from_le_bytes([leaf[at], leaf[at + 1]]),
            count: u16::from_le_bytes([leaf[at + 2], leaf[at + 3]]),
        });
    }
    Ok(ranges)
}

#[cfg(test)]
mod tests {
    use super::lumps::LUMP_COUNT;
    use super::*;

    /// Byte offset of the lump directory within the BSP header.
    const LUMP_DIRECTORY_OFFSET: usize = 8;
    const LUMP_ENTRY_SIZE: usize = 16;

    /// Build a minimal BSP-shaped buffer with a leaf lump of `leaves` entries.
    fn synthetic_bsp(version: i32, leaves: &[(u16, u16)]) -> Vec<u8> {
        let stride = if version == 0 {
            LEAF_STRIDE_V0
        } else {
            LEAF_STRIDE_V1
        };
        let header_len = LUMP_DIRECTORY_OFFSET + LUMP_COUNT * LUMP_ENTRY_SIZE + 4;
        let mut data = vec![0u8; header_len];
        data[0..4].copy_from_slice(b"VBSP");
        data[4..8].copy_from_slice(&20i32.to_le_bytes());

        let lump_offset = data.len();
        let lump_len = stride * leaves.len();
        let entry = LUMP_DIRECTORY_OFFSET + LUMP_LEAFS * LUMP_ENTRY_SIZE;
        data[entry..entry + 4].copy_from_slice(&(lump_offset as i32).to_le_bytes());
        data[entry + 4..entry + 8].copy_from_slice(&(lump_len as i32).to_le_bytes());
        data[entry + 8..entry + 12].copy_from_slice(&version.to_le_bytes());

        data.resize(lump_offset + lump_len, 0);
        for (i, (first, count)) in leaves.iter().enumerate() {
            let at = lump_offset + i * stride + LEAF_FIRST_BRUSH_OFFSET;
            data[at..at + 2].copy_from_slice(&first.to_le_bytes());
            data[at + 2..at + 4].copy_from_slice(&count.to_le_bytes());
        }
        data
    }

    #[test]
    fn reads_ranges_in_file_order_v1() {
        let data = synthetic_bsp(1, &[(0, 3), (3, 0), (7, 12)]);
        let ranges = leaf_brush_ranges(&data).unwrap();
        assert_eq!(ranges.len(), 3);
        assert_eq!((ranges[0].first, ranges[0].count), (0, 3));
        assert_eq!((ranges[2].first, ranges[2].count), (7, 12));
    }

    #[test]
    fn reads_ranges_with_the_version_0_light_cube() {
        let data = synthetic_bsp(0, &[(5, 1), (6, 2)]);
        let ranges = leaf_brush_ranges(&data).unwrap();
        assert_eq!(ranges.len(), 2);
        assert_eq!((ranges[1].first, ranges[1].count), (6, 2));
    }

    #[test]
    fn rejects_non_bsp_data() {
        let mut data = synthetic_bsp(1, &[(0, 1)]);
        data[0..4].copy_from_slice(b"XXXX");
        assert!(leaf_brush_ranges(&data).is_err());
    }

    #[test]
    fn rejects_lump_running_past_end_of_file() {
        let mut data = synthetic_bsp(1, &[(0, 1)]);
        let entry = LUMP_DIRECTORY_OFFSET + LUMP_LEAFS * LUMP_ENTRY_SIZE;
        data[entry + 4..entry + 8].copy_from_slice(&1_000_000i32.to_le_bytes());
        assert!(leaf_brush_ranges(&data).is_err());
    }
}
