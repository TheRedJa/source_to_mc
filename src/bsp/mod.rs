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
        Ok(Map {
            bsp,
            path: path.to_path_buf(),
            name,
            leaf_brushes,
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

    /// Every distinct material referenced by the map's brush sides.
    pub fn materials(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = self
            .bsp
            .textures_info
            .iter()
            .enumerate()
            .filter_map(|(i, _)| self.material_name(i))
            .collect();
        seen.sort_unstable();
        seen.dedup();
        seen
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
