//! Turning a map's materials into generated Minecraft blocks.
//!
//! Ties the search path, material resolution and texture decoding together:
//! given a map, produce a [`Pack`] holding one block per material whose
//! texture could be found.

use crate::bsp::Map;
use crate::config::Config;
use crate::output::kubejs::Pack;
use crate::source::vfs::Vfs;
use crate::source::vmt::Materials;
use crate::source::vtf::Textures;

/// What extraction managed, for reporting.
#[derive(Debug, Clone, Default)]
pub struct Extracted {
    pub materials: usize,
    pub resolved: usize,
    /// Content sources searched, in order.
    pub search_path: Vec<String>,
}

/// Build a pack of generated blocks for every material of `map` that has a
/// texture behind it.
///
/// Materials without one are simply absent, and the palette falls back to
/// rules and colour matching for those — so a missing game install degrades
/// the output rather than failing the conversion.
pub fn extract(map: &Map, config: &Config) -> (Pack, Extracted) {
    let vfs = Vfs::for_map(&map.path, &config.materials.game_dir_paths());
    let materials = Materials::new(&vfs, Some(&map.bsp.pack));
    let mut textures = Textures::new(&vfs, config.materials.texture_size);

    let mut pack = Pack::default();
    let mut stats = Extracted {
        search_path: vfs.describe(),
        ..Extracted::default()
    };

    let mut seen: Vec<&str> = Vec::new();
    for material in map.materials() {
        if seen.contains(&material.name.as_str()) {
            continue;
        }
        seen.push(&material.name);
        stats.materials += 1;

        // Tool textures are dropped before any of this matters, and nodraw is
        // the single most-used material in every map.
        if material.name.starts_with("tools/") {
            continue;
        }

        let Some(assets) = materials.assets(&material.name, Some(&material.raw_name)) else {
            continue;
        };
        let Some(image) = textures.get(&assets.base_texture, assets.alpha_test) else {
            continue;
        };

        pack.insert(&material.name, image.clone(), &assets);
        stats.resolved += 1;
    }

    (pack, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MaterialMode;
    use std::path::Path;

    fn sample_map() -> Option<Map> {
        let path = Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_trainstation_02.bsp",
        );
        path.exists().then(|| Map::load(path).unwrap())
    }

    #[test]
    fn extracts_a_block_per_resolvable_material() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.materials.mode = MaterialMode::Kubejs;

        let (pack, stats) = extract(&map, &config);
        assert!(!stats.search_path.is_empty());
        assert!(stats.resolved > 100, "only {} materials resolved", stats.resolved);
        assert_eq!(pack.len(), stats.resolved);

        // Every generated block must carry a texture of the configured size.
        for block in pack.blocks() {
            assert_eq!(block.texture.dimensions(), (16, 16), "{}", block.id);
            assert!(!block.id.is_empty());
        }
    }

    /// Tool textures never become blocks, so generating one would be dead
    /// weight in every pack.
    #[test]
    fn tool_textures_are_not_registered() {
        let Some(map) = sample_map() else { return };
        let (pack, _) = extract(&map, &Config::default());
        assert!(
            pack.blocks().all(|b| !b.material.starts_with("tools/")),
            "a tool texture was registered"
        );
    }

    /// The ids the palette will use must all be registered by the script.
    #[test]
    fn every_id_the_palette_would_use_is_registered() {
        let Some(map) = sample_map() else { return };
        let (pack, _) = extract(&map, &Config::default());

        let script = pack.script();
        for (material, id) in pack.ids() {
            assert!(
                script.contains(&format!("event.create('{id}')")),
                "{material} resolves to {id}, which the script does not register"
            );
        }
    }

    #[test]
    fn a_map_with_no_content_yields_an_empty_pack() {
        let Some(map) = sample_map() else { return };
        let mut orphan = map;
        orphan.path = std::path::PathBuf::from("/nowhere/x.bsp");
        let (pack, stats) = extract(&orphan, &Config::default());
        assert!(pack.is_empty());
        assert_eq!(stats.resolved, 0);
        assert!(stats.materials > 0, "materials should still be counted");
    }
}
