//! Versioned runtime mesh payload used by mod-owned prop rendering.

use anyhow::{Result, ensure};
use std::collections::HashMap;

pub const MAGIC: [u8; 8] = *b"S2MESH\0\0";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submesh {
    pub first_index: u32,
    pub index_count: u32,
    /// Index into the model reference's map-local material-ID array.
    pub material_slot: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<Submesh>,
}

/// Convert the already resolved model-space prop mesh without discarding its
/// authored normals. The returned material names are indexed by each emitted
/// submesh's `material_slot` and are resolved to map-local IDs by mod export.
pub fn from_prop_mesh(source: &crate::output::obj::PropMesh) -> Result<(Mesh, Vec<String>)> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut submeshes = Vec::new();
    let mut materials = Vec::new();
    let mut interned: HashMap<[u32; 8], u32> = HashMap::new();

    for part in &source.parts {
        ensure!(
            part.triangles.len() == part.normals.len() && part.triangles.len() == part.uvs.len(),
            "prop mesh attribute arrays differ in length"
        );
        if part.triangles.is_empty() {
            continue;
        }
        let first_index = u32::try_from(indices.len())?;
        let material_slot = u32::try_from(materials.len())?;
        materials.push(part.material.clone());
        for ((triangle, normals), uvs) in part.triangles.iter().zip(&part.normals).zip(&part.uvs) {
            for corner in 0..3 {
                let values = [
                    triangle[corner].x as f32,
                    triangle[corner].y as f32,
                    triangle[corner].z as f32,
                    normals[corner].x as f32,
                    normals[corner].y as f32,
                    normals[corner].z as f32,
                    uvs[corner][0] as f32,
                    uvs[corner][1] as f32,
                ];
                ensure!(
                    values.iter().all(|value| value.is_finite()),
                    "prop mesh value cannot be represented as f32"
                );
                let key = values.map(|value| if value == 0.0 { 0 } else { value.to_bits() });
                let index = match interned.get(&key) {
                    Some(index) => *index,
                    None => {
                        let index = u32::try_from(vertices.len())?;
                        vertices.push(Vertex {
                            position: values[0..3].try_into().unwrap(),
                            normal: values[3..6].try_into().unwrap(),
                            uv: values[6..8].try_into().unwrap(),
                        });
                        interned.insert(key, index);
                        index
                    }
                };
                indices.push(index);
            }
        }
        submeshes.push(Submesh {
            first_index,
            index_count: u32::try_from(indices.len())? - first_index,
            material_slot,
        });
    }
    ensure!(
        !vertices.is_empty(),
        "prop mesh has no renderable triangles"
    );
    let mut bounds_min = [f32::INFINITY; 3];
    let mut bounds_max = [f32::NEG_INFINITY; 3];
    for vertex in &vertices {
        for axis in 0..3 {
            bounds_min[axis] = bounds_min[axis].min(vertex.position[axis]);
            bounds_max[axis] = bounds_max[axis].max(vertex.position[axis]);
        }
    }
    let mesh = Mesh {
        bounds_min,
        bounds_max,
        vertices,
        indices,
        submeshes,
    };
    encode(&mesh)?;
    Ok((mesh, materials))
}

/// Convert a brush that is drawn as geometry rather than voxelized.
///
/// Brush sides are flat, so the one face normal covers each of its triangle's
/// corners; nothing here is smooth-shaded. UVs are already normalized to the
/// texture's own `0..1` and routinely fall outside it, which the runtime
/// handles by repeating the sprite exactly as it does for a prop.
pub fn from_brush_mesh(source: &crate::convert::BrushMesh) -> Result<(Mesh, Vec<String>)> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut submeshes = Vec::new();
    let mut materials = Vec::new();
    let mut interned: HashMap<[u32; 8], u32> = HashMap::new();

    for part in &source.parts {
        ensure!(
            part.triangles.len() == part.normals.len() && part.triangles.len() == part.uvs.len(),
            "brush mesh attribute arrays differ in length"
        );
        if part.triangles.is_empty() {
            continue;
        }
        let first_index = u32::try_from(indices.len())?;
        let material_slot = u32::try_from(materials.len())?;
        materials.push(part.material.clone());
        for ((triangle, normal), uvs) in part.triangles.iter().zip(&part.normals).zip(&part.uvs) {
            for corner in 0..3 {
                let values = [
                    triangle[corner].x as f32,
                    triangle[corner].y as f32,
                    triangle[corner].z as f32,
                    normal.x as f32,
                    normal.y as f32,
                    normal.z as f32,
                    uvs[corner][0] as f32,
                    uvs[corner][1] as f32,
                ];
                ensure!(
                    values.iter().all(|value| value.is_finite()),
                    "brush mesh value cannot be represented as f32"
                );
                let key = values.map(|value| if value == 0.0 { 0 } else { value.to_bits() });
                let index = match interned.get(&key) {
                    Some(index) => *index,
                    None => {
                        let index = u32::try_from(vertices.len())?;
                        vertices.push(Vertex {
                            position: values[0..3].try_into().unwrap(),
                            normal: values[3..6].try_into().unwrap(),
                            uv: values[6..8].try_into().unwrap(),
                        });
                        interned.insert(key, index);
                        index
                    }
                };
                indices.push(index);
            }
        }
        submeshes.push(Submesh {
            first_index,
            index_count: u32::try_from(indices.len())? - first_index,
            material_slot,
        });
    }
    ensure!(
        !vertices.is_empty(),
        "brush mesh has no renderable triangles"
    );
    let mut bounds_min = [f32::INFINITY; 3];
    let mut bounds_max = [f32::NEG_INFINITY; 3];
    for vertex in &vertices {
        for axis in 0..3 {
            bounds_min[axis] = bounds_min[axis].min(vertex.position[axis]);
            bounds_max[axis] = bounds_max[axis].max(vertex.position[axis]);
        }
    }
    let mesh = Mesh {
        bounds_min,
        bounds_max,
        vertices,
        indices,
        submeshes,
    };
    encode(&mesh)?;
    Ok((mesh, materials))
}

/// Convert authored Source geometry directly, without legacy texture-repeat
/// subdivision. UVs remain authored sheet coordinates.
pub fn from_source_model(
    source: &crate::source::mdl::Model,
    units: f64,
) -> Result<(Mesh, Vec<String>)> {
    ensure!(units.is_finite() && units > 0.0, "invalid model scale");
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut submeshes = Vec::new();
    let mut materials = Vec::new();
    let mut interned: HashMap<[u32; 8], u32> = HashMap::new();
    for part in &source.parts {
        ensure!(
            part.triangles.len() == part.normals.len() && part.triangles.len() == part.uvs.len(),
            "source model attribute arrays differ in length"
        );
        if part.triangles.is_empty() {
            continue;
        }
        let first_index = indices.len() as u32;
        let slot = materials.len() as u32;
        materials.push(part.material.clone());
        for ((tri, normals), uvs) in part.triangles.iter().zip(&part.normals).zip(&part.uvs) {
            let geometric = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalized();
            if geometric.length() <= 1.0e-12 {
                continue;
            }
            for c in 0..3 {
                let p = tri[c];
                let n = if normals[c].length() <= 1.0e-12 {
                    geometric
                } else {
                    normals[c].normalized()
                };
                let values = [
                    (p.x / units) as f32,
                    (p.z / units) as f32,
                    (-p.y / units) as f32,
                    n.x as f32,
                    n.z as f32,
                    -n.y as f32,
                    uvs[c][0] as f32,
                    uvs[c][1] as f32,
                ];
                ensure!(
                    values.iter().all(|v| v.is_finite()),
                    "source model value cannot be represented as f32"
                );
                let key = values.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
                let index = match interned.get(&key) {
                    Some(i) => *i,
                    None => {
                        let i = vertices.len() as u32;
                        vertices.push(Vertex {
                            position: values[0..3].try_into().unwrap(),
                            normal: values[3..6].try_into().unwrap(),
                            uv: values[6..8].try_into().unwrap(),
                        });
                        interned.insert(key, i);
                        i
                    }
                };
                indices.push(index);
            }
        }
        let index_count = indices.len() as u32 - first_index;
        if index_count > 0 {
            submeshes.push(Submesh {
                first_index,
                index_count,
                material_slot: slot,
            });
        } else {
            materials.pop();
        }
    }
    ensure!(
        !vertices.is_empty(),
        "source model has no renderable triangles"
    );
    let mut bounds_min = [f32::INFINITY; 3];
    let mut bounds_max = [f32::NEG_INFINITY; 3];
    for v in &vertices {
        for axis in 0..3 {
            bounds_min[axis] = bounds_min[axis].min(v.position[axis]);
            bounds_max[axis] = bounds_max[axis].max(v.position[axis]);
        }
    }
    let mesh = Mesh {
        bounds_min,
        bounds_max,
        vertices,
        indices,
        submeshes,
    };
    encode(&mesh)?;
    Ok((mesh, materials))
}

pub fn encode(mesh: &Mesh) -> Result<Vec<u8>> {
    crate::output::limits::check_count(
        "mesh vertex",
        mesh.vertices.len() as u64,
        crate::output::limits::MAX_VERTICES_PER_MESH,
    )?;
    crate::output::limits::check_count(
        "mesh index",
        mesh.indices.len() as u64,
        crate::output::limits::MAX_INDICES_PER_MESH,
    )?;
    crate::output::limits::check_count(
        "mesh submesh",
        mesh.submeshes.len() as u64,
        crate::output::limits::MAX_SUBMESHES_PER_MESH,
    )?;
    ensure!(!mesh.vertices.is_empty(), "mesh has no vertices");
    ensure!(!mesh.indices.is_empty(), "mesh has no indices");
    ensure!(
        mesh.indices.len() % 3 == 0,
        "mesh indices are not triangles"
    );
    let vertex_count = u32::try_from(mesh.vertices.len())?;
    let index_count = u32::try_from(mesh.indices.len())?;
    let submesh_count = u32::try_from(mesh.submeshes.len())?;

    for axis in 0..3 {
        finite(mesh.bounds_min[axis])?;
        finite(mesh.bounds_max[axis])?;
        ensure!(
            mesh.bounds_min[axis] <= mesh.bounds_max[axis],
            "invalid mesh bounds"
        );
    }
    for vertex in &mesh.vertices {
        for value in vertex
            .position
            .into_iter()
            .chain(vertex.normal)
            .chain(vertex.uv)
        {
            finite(value)?;
        }
        let length_sq: f32 = vertex.normal.iter().map(|v| v * v).sum();
        ensure!(length_sq > 1.0e-12, "mesh vertex has a zero normal");
    }
    ensure!(
        mesh.indices.iter().all(|i| *i < vertex_count),
        "mesh index out of range"
    );

    let mut next = 0u32;
    for part in &mesh.submeshes {
        ensure!(
            part.first_index == next,
            "submeshes must be contiguous and ordered"
        );
        ensure!(
            part.index_count > 0 && part.index_count % 3 == 0,
            "invalid submesh range"
        );
        next = next
            .checked_add(part.index_count)
            .ok_or_else(|| anyhow::anyhow!("submesh range overflow"))?;
        ensure!(next <= index_count, "submesh range exceeds index buffer");
    }
    ensure!(
        next == index_count,
        "submeshes do not cover the index buffer"
    );

    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, vertex_count);
    put_u32(&mut out, index_count);
    put_u32(&mut out, submesh_count);
    for value in mesh.bounds_min.into_iter().chain(mesh.bounds_max) {
        put_f32(&mut out, value);
    }
    for vertex in &mesh.vertices {
        for value in vertex
            .position
            .into_iter()
            .chain(vertex.normal)
            .chain(vertex.uv)
        {
            put_f32(&mut out, value);
        }
    }
    for index in &mesh.indices {
        put_u32(&mut out, *index);
    }
    for part in &mesh.submeshes {
        put_u32(&mut out, part.first_index);
        put_u32(&mut out, part.index_count);
        put_u32(&mut out, part.material_slot);
    }
    Ok(out)
}

fn finite(value: f32) -> Result<()> {
    ensure!(value.is_finite(), "non-finite mesh value");
    Ok(())
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_f32(out: &mut Vec<u8>, value: f32) {
    let value = if value == 0.0 { 0.0 } else { value };
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> Mesh {
        Mesh {
            bounds_min: [0.0; 3],
            bounds_max: [1.0, 1.0, 0.0],
            vertices: vec![
                Vertex {
                    position: [0.0, 0.0, -0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2],
            submeshes: vec![Submesh {
                first_index: 0,
                index_count: 3,
                material_slot: 0,
            }],
        }
    }

    #[test]
    fn encodes_fixed_layout_and_normalizes_negative_zero() {
        let bytes = encode(&triangle()).unwrap();
        assert_eq!(&bytes[..8], &MAGIC);
        assert_eq!(bytes.len(), 48 + 3 * 32 + 3 * 4 + 12);
        assert_eq!(&bytes[48 + 8..48 + 12], &0.0f32.to_le_bytes());
    }

    #[test]
    fn rejects_bad_indices_ranges_and_values() {
        let mut mesh = triangle();
        mesh.indices[2] = 3;
        assert!(encode(&mesh).is_err());
        let mut mesh = triangle();
        mesh.submeshes[0].index_count = 6;
        assert!(encode(&mesh).is_err());
        let mut mesh = triangle();
        mesh.vertices[0].uv[0] = f32::NAN;
        assert!(encode(&mesh).is_err());
    }

    #[test]
    fn resolved_prop_mesh_preserves_authored_corner_normals() {
        use crate::geom::Vec3;
        use crate::output::kubejs::RenderType;
        use crate::output::obj::{MeshPart, PropMesh};
        use std::collections::BTreeMap;

        let source = PropMesh {
            model: "models/test.mdl".into(),
            id: "test".into(),
            parts: vec![MeshPart {
                material: "mat0".into(),
                triangles: vec![[
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                ]],
                normals: vec![[
                    Vec3::new(1.0, 0.0, 0.0),
                    Vec3::new(0.0, 1.0, 0.0),
                    Vec3::new(0.0, 0.0, 1.0),
                ]],
                uvs: vec![[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
            }],
            mtl: String::new(),
            textures: BTreeMap::new(),
            render_type: RenderType::Solid,
            surface_prop: None,
            width: 1.0,
            height: 1.0,
            triangles: 1,
        };
        let (mesh, materials) = from_prop_mesh(&source).unwrap();
        assert_eq!(materials, ["mat0"]);
        assert_eq!(mesh.vertices[0].normal, [1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[1].normal, [0.0, 1.0, 0.0]);
        assert_eq!(mesh.vertices[2].normal, [0.0, 0.0, 1.0]);
    }
}
