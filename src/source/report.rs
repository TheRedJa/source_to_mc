//! The `textures` report: which materials have a real texture behind them.
//!
//! Texture extraction depends on finding a game install, and the ways that
//! fails are quiet: a mod whose base game is elsewhere, a Steam library on
//! another drive, a map converted from a copied `maps/` folder. All of those
//! produce a conversion that silently falls back to colour matching. This
//! report is the one command that says why.

use crate::bsp::Map;
use crate::source::vfs::Vfs;
use crate::source::vmt::Materials;
use crate::source::vtf::Textures;
use serde::Serialize;
use std::path::PathBuf;

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
    /// Brush sides using it.
    pub uses: usize,
    /// `$basetexture`, when the `.vmt` was found.
    pub texture: Option<String>,
    /// Whether the `.vtf` decoded to an image.
    pub resolved: bool,
    pub alpha_test: bool,
    pub translucent: bool,
}

/// Resolve every material in the map to a texture, without writing anything.
pub fn report(map: &Map, size: u32, game_dirs: &[PathBuf]) -> TextureReport {
    let vfs = Vfs::for_map(&map.path, game_dirs);
    let materials = Materials::new(&vfs, Some(&map.bsp.pack));
    let mut textures = Textures::new(&vfs, size);
    let usage = map.material_usage();

    let mut entries: Vec<Entry> = Vec::new();
    for (index, material) in map.materials().iter().enumerate() {
        let uses = usage.get(index).copied().unwrap_or(0);
        if let Some(existing) = entries.iter_mut().find(|e| e.material == material.name) {
            existing.uses += uses;
            continue;
        }

        let assets = materials.assets(&material.name, Some(&material.raw_name));
        let resolved = assets
            .as_ref()
            .is_some_and(|a| textures.get(&a.base_texture, a.alpha_test).is_some());

        entries.push(Entry {
            material: material.name.clone(),
            uses,
            texture: assets.as_ref().map(|a| a.base_texture.clone()),
            resolved,
            alpha_test: assets.as_ref().is_some_and(|a| a.alpha_test),
            translucent: assets.as_ref().is_some_and(|a| a.translucent),
        });
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

        let _ = writeln!(s, "{:<width$}  {:>6}  {:<38}  {}", "material", "uses", "texture", "");
        for entry in &self.entries {
            let flags = match (entry.alpha_test, entry.translucent) {
                (true, _) => "cutout",
                (_, true) => "translucent",
                _ => "",
            };
            let _ = writeln!(
                s,
                "{:<width$}  {:>6}  {:<38}  {}{}",
                entry.material,
                entry.uses,
                entry.texture.as_deref().unwrap_or("-"),
                if entry.resolved { "ok " } else { "MISSING " },
                flags,
            );
        }

        let used = self.entries.iter().filter(|e| e.uses > 0).count();
        let used_resolved = self.entries.iter().filter(|e| e.uses > 0 && e.resolved).count();
        let _ = writeln!(
            s,
            "\n{} of {} materials resolved to a texture ({} of {} that are actually used)",
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
        let report = report(&map, 16, &[]);

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
        let report = report(&map, 16, &[]);
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
        let report = report(&map, 16, &[]);
        let mut names: Vec<&str> = report.entries.iter().map(|e| e.material.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate materials in the report");
        assert!(report.render().contains("material"));
    }

    /// Without a search path the report must still work and say so, rather
    /// than failing or pretending everything is fine.
    #[test]
    fn a_map_with_no_game_install_reports_nothing_resolved() {
        let Some(map) = hl2() else { return };
        // A map path outside any `maps/` folder finds no game directory.
        let mut orphan = map;
        orphan.path = PathBuf::from("/nowhere/d1_trainstation_02.bsp");
        let report = report(&orphan, 16, &[]);
        assert!(report.search_path.is_empty());
        assert_eq!(report.resolved(), 0);
        assert!(report.render().contains("--game-dir"));
    }
}
