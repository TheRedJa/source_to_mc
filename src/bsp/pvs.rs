//! Source BSP visibility clusters (PVS) for static-prop render rejection.
//!
//! The visibility lump uses the Quake II run-length scheme, which Source kept
//! for its `VISIBILITY` lump: every nonzero byte copies eight clusters
//! (LSB-first), and a `0` byte followed by a count `NN` emits `NN` zero
//! *bytes*, not clusters (`NN == 0` is invalid). Rows are
//! `(cluster_count + 7) / 8` bytes with no serialized padding, and the final
//! zero run of a row may nominally overrun the row boundary; the reference
//! decompressor consumes both run bytes but clips the emitted zeros at the
//! row size, so the decoder must tolerate an overlong final run rather than
//! reject it. `vbsp` 0.9.1's own `visible_clusters` treats the skip count as
//! clusters, which misdecodes real compiler output, so the decode is
//! implemented here from the verified Valve semantics instead.

use std::collections::{BTreeMap, BTreeSet};

/// `VISIBILITY_CLUSTERS` in the Source BSP format.
pub const MAX_CLUSTERS: usize = 65_536;

/// One BSP leaf's conservative box, in Source units, with its visibility cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeafCluster {
    pub cluster: i16,
    /// Inclusive integer box bounds in Source units.
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
}

/// Decoded per-cluster PVS rows plus the leaf boxes that resolve points to clusters.
#[derive(Debug, Clone)]
pub struct ClusterVisibility {
    /// One row per cluster; bit `c` of row `k` set means cluster `c` is
    /// potentially visible from cluster `k` (LSB-first, as the engine tests).
    pub rows: Vec<Vec<u8>>,
    pub leaves: Vec<LeafCluster>,
    /// World-model BSP tree in original node/leaf order. This is required for
    /// an exact camera point-to-cluster lookup; leaf AABBs are not a valid
    /// substitute because convex BSP leaves overlap after being boxed.
    pub tree: Option<BspTree>,
}

#[derive(Debug, Clone)]
pub struct BspTree {
    pub root: i32,
    pub nodes: Vec<BspNode>,
}

#[derive(Debug, Clone, Copy)]
pub struct BspNode {
    pub normal: [f32; 3],
    pub dist: f32,
    /// Node indices are non-negative. Terminal values encode a cluster as
    /// `-1 - cluster`; `i32::MIN` is a solid/out-of-world leaf.
    pub children: [i32; 2],
}

impl ClusterVisibility {
    /// Decodes the visibility lump. `None` when the map has no usable PVS,
    /// including missing, truncated, or oversized data; callers must render
    /// unfiltered in that case.
    pub fn from_bsp(bsp: &vbsp::Bsp) -> Option<Self> {
        let cluster_count = bsp.vis_data.cluster_count as usize;
        if cluster_count == 0 || cluster_count > MAX_CLUSTERS {
            return None;
        }
        let offsets = &bsp.vis_data.pvs_offsets;
        if offsets.len() < cluster_count {
            return None;
        }
        let row_bytes = (cluster_count + 7) / 8;
        let mut rows = Vec::with_capacity(cluster_count);
        // `vbsp` consumes the dvis header and gives us only its trailing data,
        // while Source's offsets are relative to the beginning of the whole
        // visibility lump. Convert origins explicitly.
        let header_bytes = 4usize.checked_add(cluster_count.checked_mul(8)?)?;
        for cluster in 0..cluster_count {
            let offset = offsets[cluster];
            if offset < 0 {
                return None;
            }
            let relative = (offset as usize).checked_sub(header_bytes)?;
            let row = decode_row(&bsp.vis_data.data, relative, row_bytes)?;
            // A valid PVS always includes its originating cluster. This is a
            // cheap high-signal corruption check and prevents unsafe culling.
            if row[cluster >> 3] & (1 << (cluster & 7)) == 0 {
                return None;
            }
            rows.push(row);
        }
        let leaves = bsp
            .leaves
            .iter()
            .map(|leaf| LeafCluster {
                cluster: leaf.cluster,
                mins: leaf.mins,
                maxs: leaf.maxs,
            })
            .collect();
        Some(Self {
            rows,
            leaves,
            tree: None,
        })
    }

    /// As [`Self::from_bsp`], plus the world BSP tree with leaf identities in
    /// file order. `vbsp` sorts leaves, so this must use `Map`'s raw leaf view.
    pub fn from_map(map: &crate::bsp::Map) -> Option<Self> {
        let mut result = Self::from_bsp(&map.bsp)?;
        let root = map.bsp.models.first()?.head_node;
        if root < 0 || root as usize >= map.bsp.nodes.len() {
            return None;
        }
        let terminal = |child: i32| -> Option<i32> {
            if child >= 0 {
                return Some(child);
            }
            let leaf_index = child.checked_neg()?.checked_sub(1)? as usize;
            let leaf = map.leaf_brushes.get(leaf_index)?;
            Some(if leaf.cluster >= 0 {
                -1 - leaf.cluster as i32
            } else {
                i32::MIN
            })
        };
        let mut nodes = Vec::with_capacity(map.bsp.nodes.len());
        for node in &map.bsp.nodes {
            let plane = map.bsp.planes.get(node.plane_index as usize)?;
            if !plane.normal.x.is_finite()
                || !plane.normal.y.is_finite()
                || !plane.normal.z.is_finite()
                || !plane.dist.is_finite()
            {
                return None;
            }
            let children = [terminal(node.children[0])?, terminal(node.children[1])?];
            if children
                .iter()
                .any(|&child| child >= 0 && child as usize >= map.bsp.nodes.len())
            {
                return None;
            }
            nodes.push(BspNode {
                normal: [plane.normal.x, plane.normal.y, plane.normal.z],
                dist: plane.dist,
                children,
            });
        }
        result.tree = Some(BspTree { root, nodes });
        Some(result)
    }

    /// The PVS row for one cluster, for direct callers that test bits.
    pub fn row(&self, cluster: u16) -> Option<&[u8]> {
        self.rows.get(cluster as usize).map(|row| row.as_slice())
    }
}

/// Decodes one cluster's PVS row with the verified Valve semantics. The
/// final zero run of a row may nominally overrun `row_bytes`; both run bytes
/// are consumed and the emitted zeros are clipped at the row size.
fn decode_row(data: &[u8], offset: usize, row_bytes: usize) -> Option<Vec<u8>> {
    let mut row = Vec::with_capacity(row_bytes);
    let mut index = offset;
    while row.len() < row_bytes {
        let byte = *data.get(index)?;
        if byte == 0 {
            let count = *data.get(index + 1)?;
            if count == 0 {
                return None;
            }
            let writable = (count as usize).min(row_bytes - row.len());
            row.extend(std::iter::repeat(0u8).take(writable));
            index += 2;
        } else {
            row.push(byte);
            index += 1;
        }
    }
    Some(row)
}

/// Whether the runtime-visible set for `aggregate` intersects the PVS row of
/// `camera`. Empty aggregate sets fail open.
pub fn aggregate_visible(row: &[u8], clusters: &[u16]) -> bool {
    clusters.is_empty()
        || clusters
            .iter()
            .any(|&cluster| row[(cluster as usize) >> 3] & (1 << (cluster & 7)) != 0)
}

/// Leaf boxes grouped by the 16-block section spans they cover, so a whole
/// map's prop sections can be resolved without scanning every leaf.
pub struct LeafSectionIndex<'a> {
    /// Conservative block-space box and cluster per indexed leaf; only leaves
    /// with a real cluster (`cluster >= 0`) are indexed, so indexes into this
    /// vector are not indexes into `visibility.leaves`.
    block_boxes: Vec<([f64; 3], [f64; 3], u16)>,
    /// Block-space leaf indexes per covered 16-block section.
    by_section: BTreeMap<[i32; 3], Vec<usize>>,
    _visibility: std::marker::PhantomData<&'a ClusterVisibility>,
}

impl<'a> LeafSectionIndex<'a> {
    /// Indexes leaves by block-space section span. `to_block` maps a Source
    /// unit corner to map-local block coordinates; leaf boxes are rounded
    /// outward so the index never under-covers.
    pub fn build(
        visibility: &'a ClusterVisibility,
        to_block: impl Fn([f64; 3]) -> [f64; 3],
    ) -> Self {
        let mut block_boxes: Vec<([f64; 3], [f64; 3], u16)> = Vec::new();
        let mut by_section: BTreeMap<[i32; 3], Vec<usize>> = BTreeMap::new();
        for leaf in &visibility.leaves {
            if leaf.cluster < 0 {
                continue;
            }
            let mut min = [f64::INFINITY; 3];
            let mut max = [f64::NEG_INFINITY; 3];
            for corner in corner_units(leaf) {
                let block = to_block(corner);
                for axis in 0..3 {
                    min[axis] = min[axis].min(block[axis]);
                    max[axis] = max[axis].max(block[axis]);
                }
            }
            for axis in 0..3 {
                min[axis] = min[axis].floor();
                max[axis] = max[axis].ceil();
            }
            let index = block_boxes.len();
            block_boxes.push((min, max, leaf.cluster as u16));
            for x in floor_div(min[0] as i32)..=floor_div(max[0] as i32) {
                for y in floor_div(min[1] as i32)..=floor_div(max[1] as i32) {
                    for z in floor_div(min[2] as i32)..=floor_div(max[2] as i32) {
                        by_section.entry([x, y, z]).or_default().push(index);
                    }
                }
            }
        }
        Self {
            block_boxes,
            by_section,
            _visibility: std::marker::PhantomData,
        }
    }

    /// Clusters of leaves whose conservative block-space boxes intersect the
    /// 16-block section at `section`. Sections without overlapping leaves
    /// yield an empty set, which callers must treat as "no information".
    pub fn clusters_in_section(&self, section: [i32; 3]) -> BTreeSet<u16> {
        let mut result = BTreeSet::new();
        for &index in self.by_section.get(&section).into_iter().flatten() {
            let (min, max, cluster) = self.block_boxes[index];
            let overlaps = (0..3).all(|axis| {
                min[axis] <= (section[axis] * 16 + 16) as f64
                    && max[axis] >= (section[axis] * 16) as f64
            });
            if overlaps {
                result.insert(cluster);
            }
        }
        result
    }
}

/// The eight Source-unit corners of a leaf's inclusive box.
fn corner_units(leaf: &LeafCluster) -> [[f64; 3]; 8] {
    let mut corners = [[0.0; 3]; 8];
    for (i, corner) in corners.iter_mut().enumerate() {
        for axis in 0..3 {
            corner[axis] = if i & (1 << axis) == 0 {
                leaf.mins[axis] as f64
            } else {
                leaf.maxs[axis] as f64
            };
        }
    }
    corners
}

/// Floor division matching the runtime's `floor(coordinate / 16.0)` section
/// mapping, which differs from truncating division on negative coordinates.
fn floor_div(coordinate: i32) -> i32 {
    coordinate.div_euclid(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[0, 3]` skips three zero bytes; a nonzero byte copies eight clusters.
    #[test]
    fn decodes_quake_style_run_length_rows() {
        // clusters 0-7 visible, bytes 1-3 (clusters 8-31) skipped, cluster 34 visible
        let data = [0xFFu8, 0x00, 0x03, 0x04, 0x00, 0x03];
        let row = decode_row(&data, 0, 5).unwrap();
        assert_eq!(row.len(), 5);
        assert_eq!(row[0], 0xFF);
        assert_eq!(row[1], 0x00);
        assert_eq!(row[2], 0x00);
        assert_eq!(row[3], 0x00);
        assert_eq!(row[4], 0x04); // cluster 32+2 = 34
        assert!(aggregate_visible(&row, &[0, 34]));
        assert!(!aggregate_visible(&row, &[8, 33]));
    }

    #[test]
    fn truncated_rows_are_rejected() {
        assert!(decode_row(&[0xFF], 0, 4).is_none());
        assert!(decode_row(&[0x00], 0, 4).is_none());
        assert!(decode_row(&[0x00, 0x00], 0, 4).is_none());
    }

    /// Valve's decompressor tolerates a final zero run that nominally
    /// overruns the row: both run bytes are consumed and the zeros are
    /// clipped at the row size.
    #[test]
    fn overlong_final_zero_run_is_clipped() {
        // 4-byte row: one copied byte, then a run that exactly fills the row.
        let data = [0xFFu8, 0x00, 0x03];
        let row = decode_row(&data, 0, 4).unwrap();
        assert_eq!(row, [0xFF, 0x00, 0x00, 0x00]);
        // Overlong run: 4-byte row has one byte left after 3 copied bytes,
        // but the token nominally emits 5 zeros.
        let data = [0x01u8, 0x02u8, 0x03u8, 0x00, 0x05];
        let row = decode_row(&data, 0, 4).unwrap();
        assert_eq!(row, [0x01, 0x02, 0x03, 0x00]);
    }

    #[test]
    fn aggregate_visible_fails_open_on_empty_sets() {
        assert!(aggregate_visible(&[0x00, 0x00], &[]));
    }

    #[test]
    fn aggregate_visible_tests_lsb_first_bits() {
        let row = [0b0000_0100u8];
        assert!(aggregate_visible(&row, &[2]));
        assert!(!aggregate_visible(&row, &[0, 1, 3, 7]));
    }

    fn leaf(cluster: i16, mins: [i16; 3], maxs: [i16; 3]) -> LeafCluster {
        LeafCluster {
            cluster,
            mins,
            maxs,
        }
    }

    #[test]
    fn section_index_maps_blocks_through_the_transform() {
        // One leaf box covering Source units 0..=320 on each axis (10 blocks):
        // only section 0 is covered, sections beyond it are empty.
        let visibility = ClusterVisibility {
            rows: vec![vec![0xFF]],
            leaves: vec![leaf(3, [0, 0, 0], [320, 320, 320])],
            tree: None,
        };
        // Identity-ish transform: 32 units per block, no axis flips.
        let index =
            LeafSectionIndex::build(&visibility, |p| [p[0] / 32.0, p[1] / 32.0, p[2] / 32.0]);
        assert_eq!(index.clusters_in_section([0, 0, 0]), [3u16].into());
        assert!(index.clusters_in_section([0, 0, 1]).is_empty());
        // An outward-rounded box reaching into block 16 covers section 1 too:
        // 513 units = 16.03 blocks, rounded up to block 17.
        let visibility = ClusterVisibility {
            rows: vec![vec![0xFF]],
            leaves: vec![leaf(3, [0, 0, 0], [513, 513, 513])],
            tree: None,
        };
        let index =
            LeafSectionIndex::build(&visibility, |p| [p[0] / 32.0, p[1] / 32.0, p[2] / 32.0]);
        assert_eq!(index.clusters_in_section([0, 0, 0]), [3u16].into());
        assert_eq!(index.clusters_in_section([1, 1, 1]), [3u16].into());
        assert!(index.clusters_in_section([2, 2, 2]).is_empty());
    }

    #[test]
    fn negative_cluster_leaves_are_never_indexed() {
        let visibility = ClusterVisibility {
            rows: vec![vec![0xFF]],
            leaves: vec![leaf(-1, [0, 0, 0], [320, 320, 320])],
            tree: None,
        };
        let index =
            LeafSectionIndex::build(&visibility, |p| [p[0] / 32.0, p[1] / 32.0, p[2] / 32.0]);
        assert!(index.clusters_in_section([0, 0, 0]).is_empty());
    }

    /// Regression: `clusters_in_section` must use the cluster stored with the
    /// indexed box, not `visibility.leaves[same index]` — skipped negative
    /// cluster leaves desync the two lists, which exported 0xFFFF (cluster -1)
    /// section sets for d1_trainstation_02 and broke runtime validation.
    #[test]
    fn section_clusters_survive_skipped_negative_leaves() {
        let visibility = ClusterVisibility {
            rows: vec![vec![0xFF]],
            leaves: vec![
                leaf(-1, [0, 0, 0], [320, 320, 320]),
                leaf(7, [0, 0, 0], [320, 320, 320]),
            ],
            tree: None,
        };
        let index =
            LeafSectionIndex::build(&visibility, |p| [p[0] / 32.0, p[1] / 32.0, p[2] / 32.0]);
        assert_eq!(index.clusters_in_section([0, 0, 0]), [7u16].into());
    }
}
