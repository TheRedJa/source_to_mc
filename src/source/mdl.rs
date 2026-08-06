//! Reading Source's studio models, so `prop_static` becomes geometry.
//!
//! Everything a Half-Life 2 map puts *in* its rooms is a model: the fences and
//! railings along a platform, the catwalks over the canals, the crates,
//! radiators, lamps and signs. None of it is in the brush lumps, so a map
//! converted from brushes alone is an accurate but empty shell.
//! `d1_trainstation_02` alone places 299 of them.
//!
//! A model is three files that have to be read together — `.mdl` for the
//! structure and materials, `.vvd` for the vertices and `.dx90.vtx` for the
//! index buffer — which is what `vmdl` does. What is left is finding them on
//! the search path, flattening LOD 0 to triangles, and working out which
//! material each mesh wears.

use crate::geom::{Aabb, Vec3};
use crate::source::vfs::Vfs;
use std::collections::HashMap;
use std::sync::Arc;

/// One model's triangles that share a material.
#[derive(Debug, Clone)]
pub struct Part {
    /// Triangles in model space, in Source units.
    pub triangles: Vec<[Vec3; 3]>,
    /// Each triangle's centroid in texture space, so a model shows the piece
    /// of its sheet that really belongs there rather than one corner of it.
    ///
    /// The centroid, not the corners: a triangle covers a range of the sheet
    /// but a Minecraft block wears one texture, and at a metre per block the
    /// triangles are small enough that the middle is the honest answer.
    pub uvs: Vec<[f64; 2]>,
    /// Material path as it would be written in a `.vmt` lookup, lowercased and
    /// without the `materials/` prefix or extension.
    pub material: String,
}

/// A studio model flattened to what voxelization needs.
#[derive(Debug, Clone)]
pub struct Model {
    pub parts: Vec<Part>,
    /// Bounds in model space, from the model's own header.
    pub bounds: Aabb,
}

impl Model {
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.triangles.len()).sum()
    }
}

/// The index buffer has several possible names; Source picks by renderer, and
/// a few models ship only the software one.
const VTX_SUFFIXES: [&str; 4] = [".dx90.vtx", ".dx80.vtx", ".vtx", ".sw.vtx"];

/// Loads models on demand, keeping each one after the first time it is asked
/// for.
///
/// Maps place the same model over and over — a platform's railing is one
/// `.mdl` repeated a dozen times — so without the cache the same few megabytes
/// would be parsed again for every instance.
pub struct Models<'a> {
    vfs: &'a Vfs,
    cache: HashMap<String, Option<Arc<Model>>>,
}

impl<'a> Models<'a> {
    pub fn new(vfs: &'a Vfs) -> Models<'a> {
        Models { vfs, cache: HashMap::new() }
    }

    /// The model at `path` (`models/props_c17/fence01a.mdl`), or `None` if it
    /// is not on the search path or cannot be read.
    pub fn get(&mut self, path: &str) -> Option<Arc<Model>> {
        let key = path.to_ascii_lowercase().replace('\\', "/");
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let loaded = self.load(&key).map(Arc::new);
        self.cache.insert(key, loaded.clone());
        loaded
    }

    /// How many distinct models have been asked for, and how many resolved.
    pub fn stats(&self) -> (usize, usize) {
        (self.cache.len(), self.cache.values().filter(|m| m.is_some()).count())
    }

    fn load(&self, key: &str) -> Option<Model> {
        let stem = key.strip_suffix(".mdl").unwrap_or(key);

        let mdl = vmdl::Mdl::read(&self.vfs.open(&format!("{stem}.mdl"))?).ok()?;
        let vvd = vmdl::Vvd::read(&self.vfs.open(&format!("{stem}.vvd"))?).ok()?;
        let vtx = VTX_SUFFIXES
            .iter()
            .find_map(|suffix| self.vfs.open(&format!("{stem}{suffix}")))
            .and_then(|data| vmdl::Vtx::read(&data).ok())?;

        Some(self.flatten(vmdl::Model::from_parts(mdl, vtx, vvd)))
    }

    /// Turn a parsed model into triangles grouped by material.
    fn flatten(&self, model: vmdl::Model) -> Model {
        use cgmath::{Matrix4, Transform, Vector3};

        // Static props are authored in their final orientation and both of
        // these are the identity for them, but the same `.mdl` files are also
        // used for physics and NPC props, which are not. Composing once here
        // costs nothing and keeps those upright.
        let root: Matrix4<f32> = model.idle_transform() * model.root_transform();
        let place = |v: vmdl::Vector| -> Vec3 {
            let t = root.transform_vector(Vector3::new(v.x, v.y, v.z));
            Vec3::new(t.x as f64, t.y as f64, t.z as f64)
        };

        let mut parts: HashMap<String, (Vec<[Vec3; 3]>, Vec<[f64; 2]>)> = HashMap::new();
        let vertices = model.vertices();

        for mesh in model.meshes() {
            let material = self.material_of(&model, mesh.material_index());
            let (triangles, uvs) = parts.entry(material).or_default();
            for strip in mesh.vertex_strip_indices() {
                let indices: Vec<usize> = strip.collect();
                for tri in indices.chunks_exact(3) {
                    let corners = [tri[0], tri[1], tri[2]].map(|i| vertices.get(i));
                    let [Some(a), Some(b), Some(c)] = corners else { continue };
                    triangles.push([place(a.position), place(b.position), place(c.position)]);
                    uvs.push(std::array::from_fn(|axis| {
                        let sum = a.texture_coordinates[axis]
                            + b.texture_coordinates[axis]
                            + c.texture_coordinates[axis];
                        f64::from(sum) / 3.0
                    }));
                }
            }
        }

        let (min, max) = model.bounding_box();
        let mut bounds = Aabb::empty();
        for corner in [min, max] {
            bounds.extend(Vec3::new(corner.x as f64, corner.y as f64, corner.z as f64));
        }

        Model {
            parts: parts
                .into_iter()
                .map(|(material, (triangles, uvs))| Part { triangles, uvs, material })
                .collect(),
            bounds,
        }
    }

    /// Which material a mesh wears.
    ///
    /// A `.mdl` stores the material's *name* and, separately, a list of
    /// directories to look in — the same texture name can live under several,
    /// and only trying them tells you which. Whichever has a `.vmt` on the
    /// search path wins; if none does, the first is still returned so the
    /// material shows up as unresolved rather than vanishing.
    fn material_of(&self, model: &vmdl::Model, index: i32) -> String {
        let Some(info) = model.textures().get(index.max(0) as usize) else {
            return String::new();
        };
        let name = info.name.trim_start_matches('/').to_ascii_lowercase();

        // A name with a directory in it is already a complete path.
        let candidates: Vec<String> = if name.contains('/') {
            vec![name.clone()]
        } else {
            info.search_paths
                .iter()
                .map(|dir| {
                    let dir = dir.replace('\\', "/").to_ascii_lowercase();
                    format!("{}{name}", if dir.ends_with('/') { dir } else { format!("{dir}/") })
                })
                .collect()
        };

        candidates
            .iter()
            .find(|path| self.vfs.open(&format!("materials/{path}.vmt")).is_some())
            .cloned()
            .or_else(|| candidates.first().cloned())
            .unwrap_or(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn vfs() -> Option<Vfs> {
        let map = Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_trainstation_02.bsp",
        );
        map.exists().then(|| Vfs::for_map(map, &[] as &[PathBuf]))
    }

    #[test]
    fn loads_a_stock_model_as_triangles() {
        let Some(vfs) = vfs() else { return };
        let mut models = Models::new(&vfs);
        let Some(model) = models.get("models/props_c17/fence01a.mdl") else {
            panic!("a stock HL2 prop should be on the search path")
        };

        assert!(model.triangle_count() > 4, "only {} triangles", model.triangle_count());
        assert!(!model.bounds.is_empty());
        for part in &model.parts {
            assert!(!part.material.is_empty());
            assert_eq!(part.triangles.len(), part.uvs.len());
            for tri in &part.triangles {
                assert!(tri.iter().all(|v| v.is_finite()));
            }
            // An unwrap covers the sheet, so the centroids have to spread over
            // it rather than all landing in one corner.
            let spread = |axis: usize| {
                let lo = part.uvs.iter().map(|uv| uv[axis]).fold(f64::MAX, f64::min);
                let hi = part.uvs.iter().map(|uv| uv[axis]).fold(f64::MIN, f64::max);
                hi - lo
            };
            assert!(
                spread(0) > 0.05 && spread(1) > 0.05,
                "{} unwraps to a point: u spread {}, v spread {}",
                part.material,
                spread(0),
                spread(1)
            );
        }
    }

    /// The triangles have to sit inside the bounds the model declares, or they
    /// are being read wrong and every prop will be the wrong size.
    #[test]
    fn triangles_agree_with_the_declared_bounds() {
        let Some(vfs) = vfs() else { return };
        let mut models = Models::new(&vfs);
        let Some(model) = models.get("models/props_c17/fence01a.mdl") else { return };

        let mut actual = Aabb::empty();
        for tri in model.parts.iter().flat_map(|p| &p.triangles) {
            for v in tri {
                actual.extend(*v);
            }
        }
        for axis in 0..3 {
            assert!(
                actual.min.axis(axis) >= model.bounds.min.axis(axis) - 1.0
                    && actual.max.axis(axis) <= model.bounds.max.axis(axis) + 1.0,
                "axis {axis}: triangles span {:?}..{:?}, header says {:?}..{:?}",
                actual.min,
                actual.max,
                model.bounds.min,
                model.bounds.max
            );
        }
    }

    /// A prop's materials have to resolve to something the VMT reader can find,
    /// or every prop in the map ends up wearing the fallback block.
    #[test]
    fn model_materials_resolve_on_the_search_path() {
        let Some(vfs) = vfs() else { return };
        let mut models = Models::new(&vfs);
        let Some(model) = models.get("models/props_c17/fence01a.mdl") else { return };

        for part in &model.parts {
            assert!(
                vfs.open(&format!("materials/{}.vmt", part.material)).is_some(),
                "{} does not resolve",
                part.material
            );
        }
    }

    #[test]
    fn a_missing_model_is_none_and_is_only_looked_for_once() {
        let Some(vfs) = vfs() else { return };
        let mut models = Models::new(&vfs);
        assert!(models.get("models/nothing/at_all.mdl").is_none());
        assert!(models.get("models/nothing/at_all.mdl").is_none());
        assert_eq!(models.stats(), (1, 0));
    }

    #[test]
    fn the_same_model_is_only_parsed_once() {
        let Some(vfs) = vfs() else { return };
        let mut models = Models::new(&vfs);
        let a = models.get("models/props_c17/fence01a.mdl");
        let b = models.get("models/props_c17/fence01a.mdl");
        if let (Some(a), Some(b)) = (a, b) {
            assert!(Arc::ptr_eq(&a, &b));
        }
        assert_eq!(models.stats().0, 1);
    }
}
