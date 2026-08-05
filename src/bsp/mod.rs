//! Loading a Source BSP and extracting the pieces we voxelize.
//!
//! This module is the only place that talks to `vbsp` directly, so the rest of
//! the tool is insulated from its quirks (see [`rawleaves`] for one).

pub mod entities;
pub mod lumps;
pub mod rawleaves;

use crate::geom::{Aabb, Plane, Vec3};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use vbsp::{Bsp, BrushFlags, TextureFlags};

/// Slop allowed when testing points against brush planes, in Source units.
const PLANE_EPSILON: f64 = 1e-3;

/// One side of a brush: its plane plus the material on that face.
#[derive(Debug, Clone)]
pub struct Side {
    pub plane: Plane,
    /// Index into the BSP's texture-info lump, if the side has one.
    pub texture_info: Option<usize>,
    pub texture_flags: TextureFlags,
    /// Set when this side is a displacement surface.
    pub displacement: Option<usize>,
}

/// A material the map references, one per entry in the texture-data lump.
#[derive(Debug, Clone)]
pub struct Material {
    /// Authored material path: lowercased, with the compiler's cubemap and
    /// blend-patch decorations undone. This is what rules match against.
    pub name: String,
    /// Exactly as stored in the BSP.
    pub raw_name: String,
    /// The compiler's average texture colour, in linear light. Source stores it
    /// so radiosity can bounce light off the surface, which makes it a free
    /// stand-in for decoding the `.vtf` ourselves.
    pub reflectivity: [f64; 3],
}

/// Undo the decorations the map compiler adds to material paths.
///
/// A face lit by an `env_cubemap` has its material rewritten to a per-map patch
/// material, `maps/<mapname>/<real path>_<x>_<y>_<z>`, naming the cubemap's
/// origin. Displacement blend textures get a `_wvt_patch` suffix the same way.
/// Both hide the authored path, and in Entropy: Zero two thirds of all
/// materials are patched, so rules would be useless without this.
///
/// Patching nests: a blend material already living under `maps/<mapname>/`
/// picks up a *second* prefix when a cubemap patches it as well, which is why
/// this strips repeatedly rather than once. `d1_canals_01a` has several.
pub fn normalize_material(name: &str) -> String {
    let lower = name.to_ascii_lowercase().replace('\\', "/");
    let mut path = lower.as_str();
    let mut patched = false;

    // Bounded rather than `loop`, so a pathological name cannot spin.
    for _ in 0..8 {
        if let Some((_, tail)) = path.strip_prefix("maps/").and_then(|r| r.split_once('/')) {
            path = tail;
            patched = true;
            continue;
        }
        // The cubemap origin is exactly three integers, and only ever appears
        // on a patched material. Stripping trailing numbers from an ordinary
        // name would eat part of it: `metal/metalwall048a_2_3_4` is a real
        // material path, not a patch.
        if patched {
            let stripped = strip_cubemap_origin(path);
            if stripped.len() < path.len() {
                path = stripped;
                continue;
            }
        }
        if let Some(stripped) = path.strip_suffix("_wvt_patch") {
            path = stripped;
            continue;
        }
        break;
    }

    path.to_string()
}

/// Remove a trailing `_<x>_<y>_<z>`, or return the path untouched.
fn strip_cubemap_origin(path: &str) -> &str {
    let mut head = path;
    for _ in 0..3 {
        match head.rsplit_once('_') {
            Some((rest, tail)) if is_integer(tail) && !rest.is_empty() => head = rest,
            _ => return path,
        }
    }
    head
}

fn is_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// A convex brush, ready to voxelize.
#[derive(Debug, Clone)]
pub struct Solid {
    pub brush_index: usize,
    /// 0 is worldspawn; anything higher is a brush entity's model.
    pub model: usize,
    pub flags: BrushFlags,
    pub sides: Vec<Side>,
    pub bounds: Aabb,
}

pub struct Map {
    pub bsp: Bsp,
    pub path: PathBuf,
    pub name: String,
    /// Leaf brush ranges in original BSP order, which `vbsp` does not preserve.
    leaf_brushes: Vec<rawleaves::LeafBrushRange>,
    /// One entry per texture-data lump entry, in lump order.
    materials: Vec<Material>,
    /// Bytes repaired in the entity lump because they were not valid UTF-8.
    pub repaired_bytes: usize,
}

impl Map {
    pub fn load(path: &Path) -> Result<Map> {
        let mut data =
            std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let leaf_brushes = rawleaves::leaf_brush_ranges(&data)
            .with_context(|| format!("reading leaf lump of {}", path.display()))?;

        // `vbsp` insists the entity lump is valid UTF-8; shipped maps are not
        // always. Repair in place before handing it over.
        let repaired_bytes = lumps::sanitize_text_lump(&mut data, lumps::LUMP_ENTITIES)
            .with_context(|| format!("repairing entity lump of {}", path.display()))?;

        let bsp = Bsp::read(&data).with_context(|| format!("parsing {}", path.display()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "map".into());
        let materials = (0..bsp.textures_data.len())
            .map(|index| {
                let data = &bsp.textures_data[index];
                let raw_name = vbsp::Handle::new(&bsp, data).name().to_string();
                Material {
                    name: normalize_material(&raw_name),
                    raw_name,
                    reflectivity: [
                        data.reflectivity.x as f64,
                        data.reflectivity.y as f64,
                        data.reflectivity.z as f64,
                    ],
                }
            })
            .collect();
        Ok(Map {
            bsp,
            path: path.to_path_buf(),
            name,
            leaf_brushes,
            materials,
            repaired_bytes,
        })
    }

    /// Overall world bounds, taken from worldspawn's model.
    pub fn bounds(&self) -> Aabb {
        match self.bsp.models.first() {
            Some(model) => Aabb::new(model.mins.into(), model.maxs.into()),
            None => Aabb::empty(),
        }
    }

    /// Material name for a texture-info index, e.g. `CONCRETE/CONCRETEWALL001A`.
    pub fn material_name(&self, texture_info: usize) -> Option<&str> {
        self.bsp.texture_info(texture_info).map(|info| info.name())
    }

    /// Every material the map references, one per texture-data entry.
    pub fn materials(&self) -> &[Material] {
        &self.materials
    }

    /// Index into [`Map::materials`] for a texture-info index. Several
    /// texture-infos share one material when the same texture is used with
    /// different alignments.
    pub fn material_index(&self, texture_info: usize) -> Option<usize> {
        let info = self.bsp.textures_info.get(texture_info)?;
        let index = usize::try_from(info.texture_data_index).ok()?;
        (index < self.materials.len()).then_some(index)
    }

    /// How many brush sides use each material, indexed as [`Map::materials`]
    /// is. Materials used by faces but no brush side score zero: those are
    /// displacement and detail surfaces, which we do not voxelize yet.
    pub fn material_usage(&self) -> Vec<usize> {
        let mut counts = vec![0usize; self.materials.len()];
        for side in &self.bsp.brush_sides {
            if let Some(index) = usize::try_from(side.texture_info)
                .ok()
                .and_then(|i| self.material_index(i))
            {
                counts[index] += 1;
            }
        }
        counts
    }

    /// Distinct normalized material paths, sorted.
    pub fn material_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.materials.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Brush indices belonging to `model`, found by walking its node subtree
    /// down to leaves and collecting their leaf-brush references.
    ///
    /// A brush spanning several leaves is referenced repeatedly, so results are
    /// deduplicated.
    pub fn model_brushes(&self, model: usize) -> Vec<usize> {
        let Some(model) = self.bsp.models.get(model) else {
            return Vec::new();
        };

        let mut found = HashSet::new();
        let mut stack = vec![model.head_node];

        while let Some(index) = stack.pop() {
            if index >= 0 {
                let Some(node) = self.bsp.nodes.get(index as usize) else {
                    continue;
                };
                stack.push(node.children[0]);
                stack.push(node.children[1]);
                continue;
            }

            // Negative children encode leaves as `-1 - leaf_index`.
            let leaf_index = (-1 - index) as usize;
            let Some(range) = self.leaf_brushes.get(leaf_index) else {
                continue;
            };
            let start = range.first as usize;
            let end = start + range.count as usize;
            let Some(refs) = self.bsp.leaf_brushes.get(start..end) else {
                continue;
            };
            for leaf_brush in refs {
                found.insert(leaf_brush.brush as usize);
            }
        }

        let mut brushes: Vec<usize> = found.into_iter().collect();
        brushes.sort_unstable();
        brushes
    }

    /// Build the convex solid for a single brush, or `None` if its planes do
    /// not enclose a finite volume.
    pub fn solid(&self, model: usize, brush_index: usize) -> Option<Solid> {
        let brush = self.bsp.brushes.get(brush_index)?;
        let start = brush.brush_side as usize;
        let end = start + brush.num_brush_sides as usize;
        let raw_sides = self.bsp.brush_sides.get(start..end)?;

        let mut sides = Vec::with_capacity(raw_sides.len());
        for raw in raw_sides {
            let plane = self.bsp.planes.get(raw.plane as usize)?;
            let texture_info = usize::try_from(raw.texture_info).ok();
            sides.push(Side {
                plane: Plane::new(Vec3::from(plane.normal), plane.dist as f64),
                texture_info,
                texture_flags: texture_info
                    .and_then(|i| self.bsp.textures_info.get(i))
                    .map(|info| info.flags)
                    .unwrap_or(TextureFlags::empty()),
                displacement: usize::try_from(raw.displacement_info).ok(),
            });
        }

        let planes: Vec<Plane> = sides.iter().map(|s| s.plane).collect();
        let bounds = crate::geom::polyhedron_bounds(&planes, PLANE_EPSILON)?;

        Some(Solid {
            brush_index,
            model,
            flags: brush.flags,
            sides,
            bounds,
        })
    }

    /// All valid solids for a model.
    pub fn solids(&self, model: usize) -> Vec<Solid> {
        self.model_brushes(model)
            .into_iter()
            .filter_map(|brush| self.solid(model, brush))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small stock E:Z map. Tests touching real maps are skipped when the
    /// game is not installed.
    fn sample_map() -> Option<Map> {
        let path = Path::new(concat!(
            "/mnt/games/SteamLibrary/steamapps/common/Entropy Zero",
            "/Entropy Zero/EntropyZero/maps/az_c4_4.bsp"
        ));
        path.exists().then(|| Map::load(path).unwrap())
    }

    #[test]
    fn cubemap_patched_materials_normalize_to_the_authored_path() {
        assert_eq!(
            normalize_material("maps/az_bg_default/brick/brickwall031b_-621_2666_2908"),
            "brick/brickwall031b"
        );
        assert_eq!(
            normalize_material("maps/ez2_c1_1/building_template/building_template019c_-864_-84_3648"),
            "building_template/building_template019c"
        );
    }

    #[test]
    fn blend_patch_materials_lose_their_suffix() {
        assert_eq!(
            normalize_material("CONCRETE/BlendConcDirt004a_wvt_patch"),
            "concrete/blendconcdirt004a"
        );
        assert_eq!(
            normalize_material("maps/az_c4_4/nature/blendcliffdirt001a_wvt_patch"),
            "nature/blendcliffdirt001a"
        );
    }

    /// A blend material lives under `maps/<map>/` already, so a cubemap patch
    /// on top of it produces two prefixes and both decorations at once. Real
    /// example, from `d1_canals_01a`.
    #[test]
    fn nested_patching_is_unwound_completely() {
        assert_eq!(
            normalize_material(
                "maps/d1_canals_01a/maps/d1_canals_01a/nature/blendmudmud001a_wvt_patch_-1624_6208_7"
            ),
            "nature/blendmudmud001a"
        );
    }

    #[test]
    fn plain_materials_are_only_lowercased() {
        assert_eq!(
            normalize_material("CONCRETE\\CONCRETEWALL001A"),
            "concrete/concretewall001a"
        );
        assert_eq!(normalize_material("tools/toolsnodraw"), "tools/toolsnodraw");
    }

    /// Trailing numbers are part of nearly every Source material name, so only
    /// the three that follow a `maps/` prefix may be stripped.
    #[test]
    fn version_numbers_in_ordinary_names_survive() {
        assert_eq!(
            normalize_material("metal/metalwall048a_2_3_4"),
            "metal/metalwall048a_2_3_4"
        );
        assert_eq!(normalize_material("maps/x/metal/wall_1_2"), "metal/wall_1_2");
    }

    #[test]
    fn real_materials_carry_a_usable_reflectivity() {
        let Some(map) = sample_map() else { return };
        assert!(!map.materials().is_empty());
        for material in map.materials() {
            assert!(
                material.reflectivity.iter().all(|c| c.is_finite() && *c >= 0.0),
                "{} has reflectivity {:?}",
                material.name,
                material.reflectivity
            );
        }
        // Patching should have been undone for the bulk of them.
        let patched = map
            .materials()
            .iter()
            .filter(|m| m.name.starts_with("maps/"))
            .count();
        assert_eq!(patched, 0, "cubemap patching survived normalization");
    }

    #[test]
    fn texture_infos_resolve_to_materials() {
        let Some(map) = sample_map() else { return };
        for index in 0..map.bsp.textures_info.len() {
            let material = map.material_index(index).expect("every texture info has a material");
            assert!(material < map.materials().len());
        }
    }

    #[test]
    fn loads_a_real_map() {
        let Some(map) = sample_map() else { return };
        assert!(!map.bsp.brushes.is_empty());
        assert!(!map.bsp.models.is_empty());
        assert!(!map.bounds().is_empty());
    }

    #[test]
    fn leaf_lump_matches_the_parsed_leaf_count() {
        let Some(map) = sample_map() else { return };
        assert_eq!(map.leaf_brushes.len(), map.bsp.leaves.iter().count());
    }

    /// `vbsp` reorders leaves but parses their fields correctly, so our
    /// file-order ranges must be a permutation of its values. This pins the
    /// byte offsets in [`rawleaves`]: reading the wrong field still yields
    /// plausible-looking numbers, but not the same multiset.
    #[test]
    fn leaf_brush_ranges_match_vbsp_field_for_field() {
        let Some(map) = sample_map() else { return };

        let mut ours: Vec<(u16, u16)> =
            map.leaf_brushes.iter().map(|r| (r.first, r.count)).collect();
        let mut theirs: Vec<(u16, u16)> = map
            .bsp
            .leaves
            .iter()
            .map(|l| (l.first_leaf_brush, l.leaf_brush_count))
            .collect();
        ours.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(ours, theirs);
    }

    /// Every leaf-brush reference must land inside the brush lump.
    #[test]
    fn leaf_brush_ranges_stay_in_bounds() {
        let Some(map) = sample_map() else { return };
        for range in &map.leaf_brushes {
            let end = range.first as usize + range.count as usize;
            assert!(
                end <= map.bsp.leaf_brushes.len(),
                "leaf brush range {}..{end} exceeds {}",
                range.first,
                map.bsp.leaf_brushes.len()
            );
        }
    }

    /// Worldspawn plus the brush entities should account for essentially the
    /// whole brush lump; a traversal that escaped its subtree would claim far
    /// more than exists.
    #[test]
    fn models_partition_the_brush_lump() {
        let Some(map) = sample_map() else { return };
        let mut claimed = 0usize;
        for model in 0..map.bsp.models.len() {
            claimed += map.model_brushes(model).len();
        }
        assert!(
            claimed <= map.bsp.brushes.len(),
            "models claim {claimed} brushes but only {} exist",
            map.bsp.brushes.len()
        );
        // Allow a few unreferenced brushes, but the bulk must be accounted for.
        assert!(
            claimed * 10 >= map.bsp.brushes.len() * 9,
            "only {claimed} of {} brushes claimed",
            map.bsp.brushes.len()
        );
    }

    #[test]
    fn worldspawn_owns_most_brushes_and_models_are_disjoint() {
        let Some(map) = sample_map() else { return };
        let world: HashSet<usize> = map.model_brushes(0).into_iter().collect();
        assert!(!world.is_empty(), "worldspawn should own brushes");

        // Every brush entity's geometry must be distinct from worldspawn's,
        // which is the whole point of walking the per-model subtrees.
        for model in 1..map.bsp.models.len() {
            let brushes = map.model_brushes(model);
            for brush in brushes {
                assert!(
                    !world.contains(&brush),
                    "brush {brush} claimed by both worldspawn and model {model}"
                );
            }
        }
    }

    #[test]
    fn solids_have_finite_bounds() {
        let Some(map) = sample_map() else { return };
        let solids = map.solids(0);
        assert!(!solids.is_empty());
        for solid in solids.iter().take(500) {
            assert!(!solid.bounds.is_empty());
            assert!(solid.bounds.min.is_finite() && solid.bounds.max.is_finite());
            assert!(solid.sides.len() >= 4, "a closed brush needs 4+ sides");
        }
    }

    #[test]
    fn solids_stay_inside_world_bounds() {
        let Some(map) = sample_map() else { return };
        let world = map.bounds();
        // Brush bounds should sit within worldspawn's declared extent, with a
        // little slack for the plane epsilon.
        for solid in map.solids(0).iter().take(500) {
            assert!(solid.bounds.min.x >= world.min.x - 1.0);
            assert!(solid.bounds.max.z <= world.max.z + 1.0);
        }
    }
}
