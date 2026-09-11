//! Versioned binary spatial table for canonical visible map faces.

use crate::bsp::texcoord::BlockTexCoord;
use crate::voxel::grid::IVec3;
use crate::voxel::surface::{FaceDirection, FacePatch, SourceProvenance, VisibleFaceRecord};
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, HashMap};

pub const MAGIC: [u8; 8] = *b"S2FACE\0\0";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MaterialId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_uv_regions: u32,
    pub max_sections: u32,
    pub max_faces: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_uv_regions: crate::output::limits::MAX_UV_REGIONS_PER_MAP as u32,
            max_sections: crate::output::limits::MAX_SECTIONS_PER_MAP as u32,
            max_faces: crate::output::limits::MAX_FACES_PER_MAP as u32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncodedFace {
    pub cell: IVec3,
    pub patch: FacePatch,
    pub material: MaterialId,
    pub uv: BlockTexCoord,
    pub provenance: SourceProvenance,
}

impl EncodedFace {
    pub fn from_visible(face: VisibleFaceRecord, material: MaterialId) -> Self {
        Self {
            cell: face.cell,
            patch: face.patch,
            material,
            uv: face.source.uv,
            provenance: face.source.provenance,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct UvKey([u64; 8]);

/// Encode faces into sparse 16³ section buckets. Input order is deliberately
/// irrelevant to the output bytes.
pub fn encode(faces: impl IntoIterator<Item = EncodedFace>, limits: Limits) -> Result<Vec<u8>> {
    let mut buckets: BTreeMap<IVec3, Vec<(EncodedFace, UvKey)>> = BTreeMap::new();
    let mut uv_keys = Vec::new();
    let mut face_count = 0u32;

    for face in faces {
        ensure!(face_count < limits.max_faces, "surface face limit exceeded");
        let uv = uv_key(face.uv)?;
        validate_patch(face.patch)?;
        uv_keys.push(uv);
        buckets
            .entry(section_of(face.cell))
            .or_default()
            .push((face, uv));
        face_count += 1;
    }
    ensure!(
        buckets.len() <= limits.max_sections as usize,
        "surface section limit exceeded"
    );

    uv_keys.sort_unstable();
    uv_keys.dedup();
    ensure!(
        uv_keys.len() <= limits.max_uv_regions as usize,
        "UV-region limit exceeded"
    );
    let uv_ids: HashMap<_, _> = uv_keys
        .iter()
        .copied()
        .enumerate()
        .map(|(index, key)| (key, index as u32))
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, uv_keys.len() as u32);
    put_u32(&mut out, buckets.len() as u32);
    put_u32(&mut out, face_count);
    for uv in &uv_keys {
        for value in uv.0 {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    for (section, mut records) in buckets {
        records.sort_by_key(|(face, uv)| {
            (
                local_index(face.cell),
                patch_byte(face.patch).expect("validated patch"),
                face.material,
                *uv,
                face.provenance,
            )
        });
        for coordinate in section {
            put_i32(&mut out, coordinate);
        }
        put_u32(&mut out, records.len() as u32);
        for (face, uv) in records {
            put_u16(&mut out, local_index(face.cell));
            out.push(patch_byte(face.patch)?);
            out.push(provenance_tag(face.provenance));
            put_u32(&mut out, face.material.0);
            put_u32(&mut out, uv_ids[&uv]);
            let (primary, secondary) = provenance_values(face.provenance)?;
            put_u32(&mut out, primary);
            put_u32(&mut out, secondary);
        }
    }
    Ok(out)
}

fn uv_key(uv: BlockTexCoord) -> Result<UvKey> {
    let values = [uv.u, uv.v].concat();
    let mut bits = [0; 8];
    for (index, value) in values.into_iter().enumerate() {
        ensure!(value.is_finite(), "non-finite UV transform");
        bits[index] = if value == 0.0 {
            0.0f64.to_bits()
        } else {
            value.to_bits()
        };
    }
    Ok(UvKey(bits))
}

pub(crate) fn section_of(cell: IVec3) -> IVec3 {
    [cell[0] >> 4, cell[1] >> 4, cell[2] >> 4]
}

fn local_index(cell: IVec3) -> u16 {
    ((cell[1] & 15) << 8 | (cell[2] & 15) << 4 | (cell[0] & 15)) as u16
}

fn patch_byte(patch: FacePatch) -> Result<u8> {
    validate_patch(patch)?;
    let direction = direction_number(patch.direction);
    let (plane_axis, a_axis, b_axis) = patch_axes(patch.direction);
    let plane = patch.min[plane_axis];
    let a = patch.min[a_axis];
    let b = patch.min[b_axis];
    Ok(direction | (plane << 3) | (a << 5) | (b << 6))
}

fn validate_patch(patch: FacePatch) -> Result<()> {
    let (plane_axis, a_axis, b_axis) = patch_axes(patch.direction);
    ensure!(
        patch.min[plane_axis] == patch.max[plane_axis],
        "patch is not planar"
    );
    ensure!(
        patch.min[plane_axis] <= 2,
        "patch plane is outside its cell"
    );
    for axis in [a_axis, b_axis] {
        ensure!(patch.min[axis] <= 1, "patch minimum is outside its cell");
        ensure!(
            patch.max[axis] == patch.min[axis] + 1,
            "patch is not a half-block micro-patch"
        );
    }
    Ok(())
}

fn patch_axes(direction: FaceDirection) -> (usize, usize, usize) {
    match direction {
        FaceDirection::Down | FaceDirection::Up => (1, 0, 2),
        FaceDirection::North | FaceDirection::South => (2, 0, 1),
        FaceDirection::West | FaceDirection::East => (0, 2, 1),
    }
}

fn direction_number(direction: FaceDirection) -> u8 {
    match direction {
        FaceDirection::Down => 0,
        FaceDirection::Up => 1,
        FaceDirection::North => 2,
        FaceDirection::South => 3,
        FaceDirection::West => 4,
        FaceDirection::East => 5,
    }
}

fn provenance_tag(provenance: SourceProvenance) -> u8 {
    match provenance {
        SourceProvenance::Brush { .. } => 0,
        SourceProvenance::Displacement { .. } => 1,
    }
}

fn provenance_values(provenance: SourceProvenance) -> Result<(u32, u32)> {
    let (a, b) = match provenance {
        SourceProvenance::Brush { brush, side } => (brush, side),
        SourceProvenance::Displacement {
            displacement,
            triangle,
        } => (displacement, triangle),
    };
    Ok((u32::try_from(a)?, u32::try_from(b)?))
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(cell: IVec3, uv_offset: f64) -> EncodedFace {
        EncodedFace {
            cell,
            patch: FacePatch {
                direction: FaceDirection::Up,
                min: [0, 2, 1],
                max: [1, 2, 2],
            },
            material: MaterialId(7),
            uv: BlockTexCoord {
                u: [1.0, 0.0, 0.0, uv_offset],
                v: [0.0, 0.0, 1.0, 0.0],
            },
            provenance: SourceProvenance::Brush { brush: 12, side: 3 },
        }
    }

    #[test]
    fn bytes_are_independent_of_input_order() {
        let a = face([-1, 16, 31], 2.0);
        let b = face([16, -1, 0], 1.0);
        assert_eq!(
            encode([a, b], Limits::default()).unwrap(),
            encode([b, a], Limits::default()).unwrap()
        );
    }

    #[test]
    fn equal_uvs_share_one_region_and_negative_zero_is_canonical() {
        let a = face([0, 0, 0], -0.0);
        let b = face([1, 0, 0], 0.0);
        let bytes = encode([a, b], Limits::default()).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1);
    }

    #[test]
    fn rejects_nonfinite_uv_and_non_micro_patch() {
        let mut invalid_uv = face([0, 0, 0], f64::NAN);
        assert!(encode([invalid_uv], Limits::default()).is_err());
        invalid_uv.uv.u[3] = 0.0;
        invalid_uv.patch.max[0] = 2;
        assert!(encode([invalid_uv], Limits::default()).is_err());
    }

    #[test]
    fn limits_are_checked_before_output() {
        let limits = Limits {
            max_faces: 1,
            ..Limits::default()
        };
        assert!(encode([face([0, 0, 0], 0.0), face([1, 0, 0], 0.0)], limits).is_err());
    }
}
