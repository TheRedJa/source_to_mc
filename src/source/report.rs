//! The `textures` report: which materials have a real texture behind them.
//!
//! Texture extraction depends on finding a game install, and the ways that
//! fails are quiet: a mod whose base game is elsewhere, a Steam library on
//! another drive, a map converted from a copied `maps/` folder. All of those
//! produce a conversion that silently falls back to colour matching. This
//! report is the one command that says why.

use crate::bsp::Map;
use crate::bsp::texcoord::material_scales;
use crate::source::vfs::Vfs;
use crate::source::vmt::Materials;
use crate::source::vtf::Textures;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TextureReport {
    pub map: String,
    /// Content sources, in the order they are searched.
    pub search_path: Vec<String>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub material: String,
    /// Brush sides using it. Zero for a material only static props wear,
    /// which is most of them in a prop-heavy map.
    pub uses: usize,
    /// How many blocks across and down the texture is split, when it covers
    /// more than one.
    pub tiles: [u32; 2],
    /// The material comes from a model rather than a brush face.
    pub from_prop: bool,
    /// `$basetexture`, when the `.vmt` was found.
    pub texture: Option<String>,
    /// Whether the `.vtf` decoded to an image.
    pub resolved: bool,
    pub alpha_test: bool,
    pub translucent: bool,
}

/// Resolve every material in the map to a texture, without writing anything.
pub fn report(map: &Map, config: &crate::config::Config) -> TextureReport {
    let vfs = Vfs::for_map(&map.path, &config.materials.game_dir_paths());
    let materials = Materials::new(&vfs, Some(&map.bsp.pack));
    let mut textures = Textures::new(&vfs, config.materials.texture_size);
    let usage = map.material_usage();
    let scales = material_scales(map);

    let mut entries: Vec<Entry> = Vec::new();
    let add = |entries: &mut Vec<Entry>,
                   textures: &mut Textures,
                   name: &str,
                   raw: Option<&str>,
                   uses: usize,
                   split: crate::bsp::texcoord::Split,
                   from_prop: bool| {
        if let Some(existing) = entries.iter_mut().find(|e| e.material == name) {
            existing.uses += uses;
            return;
        }
        let assets = materials.assets(name, raw);
        let resolved = assets
            .as_ref()
            .is_some_and(|a| textures.get(&a.base_texture, a.alpha_test).is_some());
        entries.push(Entry {
            material: name.to_string(),
            uses,
            tiles: split.grid,
            from_prop,
            texture: assets.as_ref().map(|a| a.base_texture.clone()),
            resolved,
            alpha_test: assets.as_ref().is_some_and(|a| a.alpha_test),
            translucent: assets.as_ref().is_some_and(|a| a.translucent),
        });
    };

    for (index, material) in map.materials().iter().enumerate() {
        let split = scales
            .get(index)
            .copied()
            .flatten()
            .map(crate::source::extract::Layout::World)
            .unwrap_or(crate::source::extract::Layout::Unknown)
            .split(config, config.materials.tile_max);
        add(
            &mut entries,
            &mut textures,
            &material.name,
            Some(&material.raw_name),
            usage.get(index).copied().unwrap_or(0),
            split,
            false,
        );
    }

    // Static props bring their own materials, and in a prop-heavy map they are
    // a third of everything registered. A report that left them out would say
    // the search path is fine while every prop in the world came out grey.
    if config.props.enabled {
        let mut models = crate::source::mdl::Models::new(&vfs);
        let mut seen: Vec<String> = Vec::new();
        for prop in crate::bsp::props::extract(&map.bsp) {
            let Some(model) = models.get(&prop.model) else { continue };
            for part in &model.parts {
                if seen.contains(&part.material) {
                    continue;
                }
                seen.push(part.material.clone());
                let split = crate::source::extract::sheet_layout(
                    &materials,
                    &mut textures,
                    &part.material,
                    part.uv_per_unit,
                )
                .split(config, config.materials.tile_max);
                add(&mut entries, &mut textures, &part.material, None, 0, split, true);
            }
        }
    }

    entries.sort_by(|a, b| b.uses.cmp(&a.uses).then_with(|| a.material.cmp(&b.material)));

    TextureReport {
        map: map.name.clone(),
        search_path: vfs.describe(),
        entries,
    }
}

impl TextureReport {
    pub fn resolved(&self) -> usize {
        self.entries.iter().filter(|e| e.resolved).count()
    }

    /// Materials in actual use that have no texture behind them.
    pub fn unresolved(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| !e.resolved && e.uses > 0)
    }

    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();

        if self.search_path.is_empty() {
            let _ = writeln!(
                s,
                "no content found. The map needs to sit in its game's `maps/` folder, \
                 or pass --game-dir.\n"
            );
        } else {
            let _ = writeln!(s, "searching, in order:");
            for source in &self.search_path {
                let _ = writeln!(s, "    {source}");
            }
            let _ = writeln!(s);
        }

        let width = self
            .entries
            .iter()
            .map(|e| e.material.len())
            .max()
            .unwrap_or(8)
            .clamp(8, 46);

        let _ = writeln!(
            s,
            "{:<width$}  {:>6}  {:<38}  {:>7}  {}",
            "material", "uses", "texture", "blocks", ""
        );
        for entry in &self.entries {
            let flags = match (entry.alpha_test, entry.translucent) {
                (true, _) => "cutout",
                (_, true) => "translucent",
                _ => "",
            };
            let _ = writeln!(
                s,
                "{:<width$}  {:>6}  {:<38}  {:>7}  {}{}{}",
                entry.material,
                entry.uses,
                entry.texture.as_deref().unwrap_or("-"),
                format!("{}x{}", entry.tiles[0], entry.tiles[1]),
                if entry.resolved { "ok " } else { "MISSING " },
                flags,
                if entry.from_prop { " prop" } else { "" },
            );
        }

        let used = self.entries.iter().filter(|e| e.uses > 0).count();
        let used_resolved = self.entries.iter().filter(|e| e.uses > 0 && e.resolved).count();
        // Tool materials never become blocks, so counting them here would
        // overstate what the pack registers.
        let blocks: usize = self
            .entries
            .iter()
            .filter(|e| e.resolved && !e.material.starts_with("tools/"))
            .map(|e| (e.tiles[0] * e.tiles[1]) as usize)
            .sum();
        let _ = writeln!(
            s,
            "\n{} of {} materials resolved to a texture ({} of {} used by brush faces), \n\
             up to {blocks} generated blocks once split across the surfaces they cover",
            self.resolved(),
            self.entries.len(),
            used_resolved,
            used,
        );
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;
    use std::path::Path;

    fn sample_map(path: &str) -> Option<Map> {
        let path = Path::new(path);
        path.exists().then(|| Map::load(path).unwrap())
    }

    fn hl2() -> Option<Map> {
        sample_map("/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_trainstation_02.bsp")
    }

    #[test]
    fn resolves_nearly_every_material_of_a_real_map() {
        let Some(map) = hl2() else { return };
        let report = report(&map, &Config::default());

        assert!(!report.search_path.is_empty(), "no content sources");
        let used = report.entries.iter().filter(|e| e.uses > 0).count();
        let resolved = report.entries.iter().filter(|e| e.uses > 0 && e.resolved).count();
        assert!(
            resolved * 10 >= used * 9,
            "only {resolved} of {used} used materials resolved: {:?}",
            report.unresolved().map(|e| &e.material).take(10).collect::<Vec<_>>()
        );
    }

    /// The flags that decide a block's render type have to survive.
    #[test]
    fn alpha_tested_materials_are_flagged() {
        let Some(map) = hl2() else { return };
        let report = report(&map, &Config::default());
        let grates: Vec<&Entry> = report
            .entries
            .iter()
            .filter(|e| e.material.contains("grate") && e.resolved)
            .collect();
        if grates.is_empty() {
            return;
        }
        assert!(
            grates.iter().any(|e| e.alpha_test),
            "no grate material was flagged alpha-tested"
        );
    }

    /// Tool textures have no `.vmt` to find, and that is not a failure.
    #[test]
    fn the_report_lists_every_material_once() {
        let Some(map) = hl2() else { return };
        let report = report(&map, &Config::default());
        let mut names: Vec<&str> = report.entries.iter().map(|e| e.material.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate materials in the report");
        assert!(report.render().contains("material"));
    }

    /// Props bring their own materials, and in a prop-heavy map they are a
    /// large share of everything registered. Leaving them out would let the
    /// report say the search path is fine while every prop came out grey.
    #[test]
    fn the_report_covers_prop_materials() {
        let Some(map) = hl2() else { return };
        let report = report(&map, &Config::default());

        let from_props = report.entries.iter().filter(|e| e.from_prop).count();
        assert!(from_props > 10, "only {from_props} prop materials listed");
        assert!(
            report.entries.iter().filter(|e| e.from_prop && e.resolved).count() * 2
                > from_props,
            "most prop materials should resolve"
        );
    }

    /// The split is what decides how many blocks get registered, so the
    /// report has to show it.
    #[test]
    fn the_report_shows_how_far_each_texture_is_split() {
        let Some(map) = hl2() else { return };
        let report = report(&map, &Config::default());

        let split = report.entries.iter().filter(|e| e.tiles != [1, 1]).count();
        assert!(split > 10, "only {split} materials are split at all");
        assert!(
            report.entries.iter().all(|e| e.tiles[0] >= 1 && e.tiles[1] >= 1),
            "a material claims fewer than one block"
        );

        let mut off = Config::default();
        off.materials.tile_textures = false;
        let plain = self::report(&map, &off);
        assert!(
            plain.entries.iter().all(|e| e.tiles == [1, 1]),
            "tiling is off but the report still splits"
        );
    }

    /// Without a search path the report must still work and say so, rather
    /// than failing or pretending everything is fine.
    #[test]
    fn a_map_with_no_game_install_reports_nothing_resolved() {
        let Some(map) = hl2() else { return };
        // A map path outside any `maps/` folder finds no game directory.
        let mut orphan = map;
        orphan.path = PathBuf::from("/nowhere/d1_trainstation_02.bsp");
        let report = report(&orphan, &Config::default());
        assert!(report.search_path.is_empty());
        assert_eq!(report.resolved(), 0);
        assert!(report.render().contains("--game-dir"));
    }
}
