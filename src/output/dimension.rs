//! Emitting a `dimension_type` datapack sized to a converted map.
//!
//! Source maps are tall. At 16 units per block `ez2_c2_1` needs 1213 blocks of
//! height, against vanilla's 384, and WorldEdit drops out-of-range blocks
//! silently — you paste, walk in, and find the top of the map missing with no
//! error anywhere. So the datapack has to exist *before* the paste, which is
//! what makes emitting it worth doing rather than describing.
//!
//! Minecraft 1.21.1 allows `min_y` in -2032..=2031 and `height` in 16..=4064,
//! both multiples of 16, with `min_y + height - 1 <= 2031`. Source caps maps at
//! +-16384 units, or 2048 blocks at 16 units per block, so the ceiling is only
//! reachable at finer scales.

use crate::inspect::YRangeReport;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::Path;

/// Datapack format number for Minecraft 1.21 and 1.21.1.
const PACK_FORMAT: i32 = 48;

/// Namespace the emitted dimension and its type live under.
const NAMESPACE: &str = "src2mc";

/// Write a datapack under `dir` defining a dimension shaped for `y_range`.
///
/// Returns the paths written, and the `/execute in` id for the new dimension.
pub fn write(dir: &Path, map_name: &str, y_range: &YRangeReport) -> Result<Emitted> {
    let id = sanitize(map_name);
    let data = dir.join("data").join(NAMESPACE);
    let type_dir = data.join("dimension_type");
    let dimension_dir = data.join("dimension");

    std::fs::create_dir_all(&type_dir)
        .with_context(|| format!("creating {}", type_dir.display()))?;
    std::fs::create_dir_all(&dimension_dir)
        .with_context(|| format!("creating {}", dimension_dir.display()))?;

    let mcmeta = json!({
        "pack": {
            "pack_format": PACK_FORMAT,
            "description": format!("src2mc: a dimension tall enough for {map_name}"),
        }
    });
    let mcmeta_path = dir.join("pack.mcmeta");
    write_json(&mcmeta_path, &mcmeta)?;

    // `logical_height` bounds where the game lets you teleport and where
    // portals may generate. Matching `height` keeps the whole map usable.
    let dimension_type = json!({
        "ultrawarm": false,
        "natural": true,
        "coordinate_scale": 1.0,
        "has_skylight": true,
        "has_ceiling": false,
        "ambient_light": 0.0,
        "monster_spawn_light_level": 0,
        "monster_spawn_block_light_limit": 0,
        "piglin_safe": false,
        "bed_works": true,
        "respawn_anchor_works": false,
        "has_raids": true,
        "min_y": y_range.dimension_min_y,
        "height": y_range.dimension_height,
        "logical_height": y_range.dimension_height,
        "infiniburn": "#minecraft:infiniburn_overworld",
        "effects": "minecraft:overworld",
    });
    let type_path = type_dir.join(format!("{id}.json"));
    write_json(&type_path, &dimension_type)?;

    // An empty flat generator: nothing to dig out before pasting, and nothing
    // to fight the schematic for space.
    let dimension = json!({
        "type": format!("{NAMESPACE}:{id}"),
        "generator": {
            "type": "minecraft:flat",
            "settings": {
                "biome": "minecraft:the_void",
                "lakes": false,
                "features": false,
                "layers": [],
            }
        }
    });
    let dimension_path = dimension_dir.join(format!("{id}.json"));
    write_json(&dimension_path, &dimension)?;

    Ok(Emitted {
        id: format!("{NAMESPACE}:{id}"),
        min_y: y_range.dimension_min_y,
        height: y_range.dimension_height,
        files: vec![mcmeta_path, type_path, dimension_path],
    })
}

#[derive(Debug, Clone)]
pub struct Emitted {
    /// Namespaced id, for `/execute in <id> run ...`.
    pub id: String,
    pub min_y: i32,
    pub height: i32,
    pub files: Vec<std::path::PathBuf>,
}

impl Emitted {
    /// What to tell the user to do with it.
    pub fn instructions(&self, dir: &Path) -> String {
        format!(
            "wrote a dimension datapack to {}\n  \
             y {}..{} ({} blocks)\n  \
             copy the folder into <world>/datapacks/, then `/reload` and \
             `/execute in {} run tp @s 0 {} 0`",
            dir.display(),
            self.min_y,
            self.min_y + self.height - 1,
            self.height,
            self.id,
            self.min_y,
        )
    }
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Reduce a map name to the characters a Minecraft resource id allows.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' | '-' | '.' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            _ => '_',
        })
        .collect();
    if cleaned.is_empty() { "map".into() } else { cleaned }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Aabb, Vec3};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("src2mc-dimension-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn y_range_for(height: f64) -> YRangeReport {
        crate::inspect::y_range(Aabb::new(Vec3::ZERO, Vec3::new(10.0, height, 10.0)))
    }

    #[test]
    fn writes_a_datapack_that_covers_the_map() {
        let dir = temp_dir("basic");
        let y = y_range_for(1000.0);
        let emitted = write(&dir, "ez2_c2_1", &y).unwrap();

        assert_eq!(emitted.id, "src2mc:ez2_c2_1");
        for path in &emitted.files {
            assert!(path.exists(), "{} was not written", path.display());
        }

        let type_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("data/src2mc/dimension_type/ez2_c2_1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(type_json["min_y"], y.dimension_min_y);
        assert_eq!(type_json["height"], y.dimension_height);

        // The dimension must actually contain the map.
        let min = type_json["min_y"].as_i64().unwrap();
        let height = type_json["height"].as_i64().unwrap();
        assert!(min <= y.min_y as i64);
        assert!(min + height - 1 >= y.max_y as i64);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The values Minecraft actually validates on load. Getting these wrong
    /// means the datapack is rejected and the world will not open.
    #[test]
    fn the_emitted_limits_are_ones_minecraft_accepts() {
        let dir = temp_dir("limits");
        for height in [100.0, 500.0, 1213.0, 2000.0] {
            let y = y_range_for(height);
            let emitted = write(&dir, "m", &y).unwrap();
            assert_eq!(emitted.min_y % 16, 0, "min_y {} is not a multiple of 16", emitted.min_y);
            assert_eq!(emitted.height % 16, 0, "height {} is not a multiple of 16", emitted.height);
            assert!((crate::DIMENSION_MIN_Y..=crate::DIMENSION_MAX_Y).contains(&emitted.min_y));
            assert!((16..=crate::DIMENSION_MAX_HEIGHT).contains(&emitted.height));
            assert!(emitted.min_y + emitted.height - 1 <= crate::DIMENSION_MAX_Y);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_dimension_references_its_own_type() {
        let dir = temp_dir("reference");
        write(&dir, "d1_town_01", &y_range_for(400.0)).unwrap();
        let dimension: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("data/src2mc/dimension/d1_town_01.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(dimension["type"], "src2mc:d1_town_01");
        assert_eq!(dimension["generator"]["settings"]["biome"], "minecraft:the_void");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn map_names_become_legal_resource_ids() {
        assert_eq!(sanitize("d1_town_01"), "d1_town_01");
        assert_eq!(sanitize("EZ2 Chapter/One"), "ez2_chapter_one");
        assert_eq!(sanitize(""), "map");
    }
}
