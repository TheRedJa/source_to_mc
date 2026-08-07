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
    /// Each corner's position in texture space, parallel to `triangles`.
    ///
    /// Per corner, not one value per triangle: a model's triangles are not
    /// block-sized. A cliff prop is a handful of huge ones, and giving every
    /// voxel of a triangle the same tile paints the whole face in repeated
    /// patches. Interpolating across the triangle gives each block the piece
    /// of the sheet that is really in front of it.
    pub uvs: Vec<[[f64; 2]; 3]>,
    /// Material path as it would be written in a `.vmt` lookup, lowercased and
    /// without the `materials/` prefix or extension.
    pub material: String,
    /// Texture coordinates covered per Source unit of surface, typical across
    /// this part's triangles.
    ///
    /// The only thing that relates a model's sheet to world size, and there is
    /// nothing in the `.mdl` header that states it — it has to be measured off
    /// the geometry. Assuming a fixed rate instead is wrong by a factor of
    /// several on anything large: `props_wasteland/rockcliff02a` stretches one
    /// sheet over 39 blocks of cliff.
    pub uv_per_unit: f64,
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

/// How much of the texture sheet a unit of surface covers.
///
/// The only thing relating a model's sheet to world size, and nothing in the
/// `.mdl` header states it — it has to be measured off the geometry. Taken
/// from the ratio of areas rather than edge lengths, so it does not depend on
/// how each triangle happens to be shaped, and as the median across triangles
/// so a few degenerate slivers cannot skew it.
fn uv_rate(triangles: &[[Vec3; 3]], uvs: &[[[f64; 2]; 3]]) -> f64 {
    let mut rates: Vec<f64> = triangles
        .iter()
        .zip(uvs)
        .filter_map(|(tri, uv)| {
            let world = (tri[1] - tri[0]).cross(tri[2] - tri[0]).length();
            let du = [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]];
            let dv = [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]];
            let sheet = (du[0] * dv[1] - du[1] * dv[0]).abs();
            (world > 1e-6 && sheet > 1e-12).then(|| (sheet / world).sqrt())
        })
        .collect();
    if rates.is_empty() {
        return 0.0;
    }
    rates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    rates[rates.len() / 2]
}

/// Stop `vmdl`'s own panics from printing, once.
///
/// A map can reference hundreds of models it cannot read, and each one would
/// otherwise put a panic message and a backtrace on stderr for something that
/// is handled. Only panics raised inside `vmdl` are silenced; anything else
/// still goes through the hook that was installed before.
fn quiet_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let from_vmdl = info
                .location()
                .is_some_and(|location| location.file().contains("vmdl"));
            if !from_vmdl {
                previous(info);
            }
        }));
    });
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
        // `vmdl` does not merely fail on the parts of the format it has not
        // implemented — it panics. Reading animation blocks is one such gap,
        // and the entity lump's `prop_dynamic` and `prop_ragdoll` models are
        // full of them, where `prop_static` never was. A model that cannot be
        // read has to be one skipped prop, not a dead conversion.
        quiet_panics();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.read(key))).ok()?
    }

    fn read(&self, key: &str) -> Option<Model> {
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

        let mut parts: HashMap<String, (Vec<[Vec3; 3]>, Vec<[[f64; 2]; 3]>)> = HashMap::new();
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
                    uvs.push([a, b, c].map(|v| -> [f64; 2] {
                        std::array::from_fn(|axis| f64::from(v.texture_coordinates[axis]))
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
                .map(|(material, (triangles, uvs))| Part {
                    uv_per_unit: uv_rate(&triangles, &uvs),
                    triangles,
                    uvs,
                    material,
                })
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
            // An unwrap covers the sheet, so the coordinates have to spread
            // over it rather than all landing in one corner.
            let spread = |axis: usize| {
                let all = || part.uvs.iter().flatten().map(|uv| uv[axis]);
                let lo = all().fold(f64::MAX, f64::min);
                let hi = all().fold(f64::MIN, f64::max);
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

    /// The rate that relates a model's sheet to world size. Nothing in the
    /// file states it, so it is measured — and getting it wrong by a factor
    /// of several is what paints a big prop in repeated patches.
    #[test]
    fn models_report_a_plausible_uv_rate() {
        let Some(vfs) = vfs() else { return };
        let mut models = Models::new(&vfs);
        let Some(model) = models.get("models/props_c17/fence01a.mdl") else { return };

        for part in &model.parts {
            assert!(part.uv_per_unit > 0.0, "{} has no UV rate", part.material);
            // One pass over the sheet should cover somewhere between a
            // fraction of a block and a few hundred; outside that the
            // measurement is wrong, not the model.
            let blocks = 1.0 / (part.uv_per_unit * 16.0);
            assert!(
                (0.1..=512.0).contains(&blocks),
                "{} covers {blocks} blocks per sheet",
                part.material
            );
        }
    }

    /// The rate has to track the model's real size: a cliff that stretches one
    /// sheet across tens of blocks must report a far lower rate than a fence.
    #[test]
    fn a_stretched_model_reports_a_lower_uv_rate() {
        let Some(vfs) = vfs() else { return };
        let mut models = Models::new(&vfs);
        let (Some(fence), Some(cliff)) = (
            models.get("models/props_c17/fence01a.mdl"),
            models.get("models/props_wasteland/rockcliff02a.mdl"),
        ) else {
            return;
        };
        let rate = |m: &Arc<Model>| {
            m.parts
                .iter()
                .map(|p| p.uv_per_unit)
                .fold(0.0f64, f64::max)
        };
        assert!(
            rate(&cliff) < rate(&fence),
            "cliff {} should stretch further than fence {}",
            rate(&cliff),
            rate(&fence)
        );
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
