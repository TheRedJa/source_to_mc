//! Versioned map-local prop visibility (PVS) table.

use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

use crate::bsp::pvs;

pub const MAGIC: [u8; 8] = *b"S2PVIS\0\0";
pub const VERSION: u32 = 2;

/// `VISIBILITY_CLUSTERS` in the Source BSP format.
pub const MAX_CLUSTERS: u64 = 65_536;
pub const MAX_LEAVES: u64 = 4_000_000;
pub const MAX_ROW_BYTES: u64 = MAX_CLUSTERS / 8;
pub const MAX_BITSET_BYTES: u64 = 256 * 1024 * 1024;

/// One BSP leaf's conservative box, in map-local block coordinates, with its
/// visibility cluster. Only leaves with a real cluster are exported; a camera
/// point that matches no leaf must fail open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Leaf {
    pub cluster: i16,
    /// Inclusive box bounds in map-local block coordinates.
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
}

/// Exact map-local point-leaf tree. A negative child is `-1 - cluster`; the
/// minimum i32 value means solid/outside and deliberately resolves to no PVS.
#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub normal: [f32; 3],
    pub dist: f32,
    pub children: [i32; 2],
}

pub fn tree_from_visibility(
    visibility: &pvs::ClusterVisibility,
    transform: &crate::voxel::transform::Transform,
) -> Option<(i32, Vec<Node>)> {
    let tree = visibility.tree.as_ref()?;
    let mut nodes = Vec::with_capacity(tree.nodes.len());
    for node in &tree.nodes {
        let plane = transform.transform_plane(crate::geom::Plane::new(
            crate::geom::Vec3::new(
                node.normal[0] as f64,
                node.normal[1] as f64,
                node.normal[2] as f64,
            ),
            node.dist as f64,
        ));
        let normal = [
            plane.normal.x as f32,
            plane.normal.y as f32,
            plane.normal.z as f32,
        ];
        let dist = plane.dist as f32;
        if !normal.iter().all(|v| v.is_finite()) || !dist.is_finite() {
            return None;
        }
        nodes.push(Node {
            normal,
            dist,
            children: node.children,
        });
    }
    Some((tree.root, nodes))
}

/// Builds the exportable view of a map's visibility data. `to_block` maps a
/// Source-unit corner to map-local block coordinates; leaf boxes are rounded
/// outward. Returns `None` when the map has no usable PVS.
pub fn from_visibility(
    visibility: &pvs::ClusterVisibility,
    to_block: impl Fn([f64; 3]) -> [f64; 3],
) -> Option<(Vec<Vec<u8>>, Vec<Leaf>)> {
    let mut leaves = Vec::new();
    for leaf in &visibility.leaves {
        if leaf.cluster < 0 {
            continue;
        }
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for i in 0..8 {
            let corner = [
                if i & 1 == 0 {
                    leaf.mins[0] as f64
                } else {
                    leaf.maxs[0] as f64
                },
                if i & 2 == 0 {
                    leaf.mins[1] as f64
                } else {
                    leaf.maxs[1] as f64
                },
                if i & 4 == 0 {
                    leaf.mins[2] as f64
                } else {
                    leaf.maxs[2] as f64
                },
            ];
            let block = to_block(corner);
            for axis in 0..3 {
                min[axis] = min[axis].min(block[axis]);
                max[axis] = max[axis].max(block[axis]);
            }
        }
        let mut mins = [0i16; 3];
        let mut maxs = [0i16; 3];
        for axis in 0..3 {
            mins[axis] = block_i16(min[axis].floor())?;
            maxs[axis] = block_i16(max[axis].ceil())?;
        }
        leaves.push(Leaf {
            cluster: leaf.cluster,
            mins,
            maxs,
        });
    }
    leaves.sort();
    leaves.dedup();
    Some((visibility.rows.clone(), leaves))
}

pub fn encode(
    rows: Vec<Vec<u8>>,
    nodes: Vec<Node>,
    root: i32,
    sections: BTreeMap<[i32; 3], BTreeSet<u16>>,
) -> Result<Vec<u8>> {
    crate::output::limits::check_count("pvs cluster", rows.len() as u64, MAX_CLUSTERS)?;
    crate::output::limits::check_count("pvs node", nodes.len() as u64, MAX_LEAVES)?;
    crate::output::limits::check_count(
        "pvs section",
        sections.len() as u64,
        crate::output::limits::MAX_SECTIONS_PER_MAP,
    )?;
    ensure!(!rows.is_empty(), "pvs table has no clusters");
    let row_bytes = (rows.len() as u64).div_ceil(8);
    crate::output::limits::check_count("pvs row", row_bytes, MAX_ROW_BYTES)?;
    crate::output::limits::check_count(
        "pvs bitset",
        row_bytes * rows.len() as u64,
        MAX_BITSET_BYTES,
    )?;
    for row in &rows {
        ensure!(
            row.len() as u64 == row_bytes,
            "pvs row length is inconsistent"
        );
    }
    ensure!(
        rows[0].len() == (rows.len() + 7) / 8,
        "pvs row length does not cover the cluster count"
    );
    ensure!(
        root >= 0 && (root as usize) < nodes.len(),
        "pvs root node is invalid"
    );
    for node in &nodes {
        ensure!(
            node.normal.iter().all(|v| v.is_finite()) && node.dist.is_finite(),
            "pvs node plane is non-finite"
        );
        for &child in &node.children {
            ensure!(
                child < 0 || (child as usize) < nodes.len(),
                "pvs node child is invalid"
            );
        }
    }
    let node_count = u32::try_from(nodes.len())?;
    let section_count = u32::try_from(sections.len())?;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, rows.len() as u32);
    put_u32(&mut out, row_bytes as u32);
    put_u32(&mut out, node_count);
    put_i32(&mut out, root);
    put_u32(&mut out, section_count);
    for node in nodes {
        for value in node.normal {
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        out.extend_from_slice(&node.dist.to_bits().to_le_bytes());
        put_i32(&mut out, node.children[0]);
        put_i32(&mut out, node.children[1]);
    }
    for (section, clusters) in sections {
        let count = u32::try_from(clusters.len())?;
        for value in section {
            put_i32(&mut out, value);
        }
        put_u32(&mut out, count);
        for cluster in clusters {
            put_u16(&mut out, cluster);
        }
    }
    for row in rows {
        out.extend_from_slice(&row);
    }
    Ok(out)
}

fn block_i16(value: f64) -> Option<i16> {
    i16::try_from(value as i64).ok()
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::iter;

    fn visible(rows: usize) -> Vec<Vec<u8>> {
        let row_bytes = rows.div_ceil(8);
        iter::repeat(vec![0xFFu8; row_bytes]).take(rows).collect()
    }

    fn node(children: [i32; 2]) -> Node {
        Node {
            normal: [1.0, 0.0, 0.0],
            dist: 0.0,
            children,
        }
    }

    #[test]
    fn round_trips_a_small_table() {
        let nodes = vec![node([-1, -2])];
        let mut sections = BTreeMap::new();
        sections.insert([0, 0, 0], [0u16].into());
        sections.insert([1, 0, 0], [0u16, 1u16].into());
        let bytes = encode(visible(2), nodes, 0, sections).unwrap();
        assert_eq!(&bytes[..8], &MAGIC);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 1);
        assert_eq!(i32::from_le_bytes(bytes[24..28].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 2);
    }

    #[test]
    fn rejects_inconsistent_rows_and_invalid_nodes() {
        let mut sections = BTreeMap::new();
        sections.insert([0, 0, 0], [0u16].into());
        // Nine clusters need two row bytes; a one-byte row is inconsistent.
        let mut rows = visible(9);
        rows[1] = vec![0xFF];
        assert!(encode(rows, vec![node([-1, -1])], 0, sections.clone()).is_err());
        assert!(encode(visible(2), vec![node([2, -1])], 0, sections.clone()).is_err());
        assert!(encode(visible(2), vec![node([-1, -1])], 1, sections).is_err());
    }

    #[test]
    fn empty_sections_are_not_required() {
        let bytes = encode(visible(1), vec![node([-1, -1])], 0, BTreeMap::new()).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 0);
        // One cluster, one node, zero sections, and one row byte.
        assert_eq!(bytes.len(), 32 + 24 + 1);
    }
}
