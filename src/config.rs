//! Conversion configuration.
//!
//! Everything is loaded from a single TOML file; every field has a default, so
//! a config may specify only what it changes. CLI flags override the file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub scale: Scale,
    pub transform: Transform,
    pub contents: Contents,
    pub materials: Materials,
    pub fill: Fill,
    pub displacement: Displacement,
    pub entities: Entities,
    pub output: Output,
    pub performance: Performance,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scale {
    /// Source units per Minecraft block. 16 is one Hammer grid square, which
    /// makes the 72-unit player 4.5 blocks tall.
    pub units_per_block: f64,
}

impl Default for Scale {
    fn default() -> Self {
        Scale { units_per_block: 16.0 }
    }
}

/// Where the converted map's origin lands in Minecraft space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginMode {
    /// Put the map's minimum corner at (0, `y_base`, 0).
    BoundsMin,
    /// Keep Source's own origin as the Minecraft origin.
    MapOrigin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Transform {
    pub origin_mode: OriginMode,
    /// Minecraft Y that the map's lowest point maps to under `bounds_min`.
    /// Vanilla bottoms out at -64; a custom `dimension_type` datapack allows
    /// down to -2032.
    pub y_base: i32,
    /// Extra rotation about the vertical axis, in degrees, applied before
    /// voxelization.
    pub rotate_yaw: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            origin_mode: OriginMode::BoundsMin,
            y_base: 0,
            rotate_yaw: 0.0,
        }
    }
}

/// What to do with a brush carrying a given contents flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentAction {
    /// Drop the brush entirely.
    Skip,
    /// Voxelize it and let material rules choose the blocks.
    Solid,
    /// Voxelize it and force every voxel to this block.
    Block(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Contents {
    /// Keyed by `BrushFlags` name (`solid`, `water`, `playerclip`, ...).
    /// The first matching entry in flag order wins; anything unlisted that is
    /// solid is voxelized normally.
    pub actions: BTreeMap<String, ContentAction>,
    /// Skip faces flagged `SKY`/`SKY2D` rather than building a ceiling.
    pub skip_sky: bool,
    /// Skip brushes whose sides are all nodraw/skip/hint tool textures.
    pub skip_tool_brushes: bool,
}

impl Default for Contents {
    fn default() -> Self {
        let mut actions = BTreeMap::new();
        // Trigger volumes carry no contents flag of their own; they are
        // dropped because they are brush entities with no solid contents.
        for flag in ["playerclip", "monsterclip", "areaportal", "origin"] {
            actions.insert(flag.to_string(), ContentAction::Skip);
        }
        actions.insert("water".into(), ContentAction::Block("minecraft:water".into()));
        actions.insert("slime".into(), ContentAction::Block("minecraft:water".into()));
        actions.insert("window".into(), ContentAction::Block("minecraft:glass".into()));
        actions.insert("grate".into(), ContentAction::Block("minecraft:iron_bars".into()));
        actions.insert("ladder".into(), ContentAction::Block("minecraft:ladder".into()));
        Contents {
            actions,
            skip_sky: true,
            skip_tool_brushes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Materials {
    /// Path to a rules TOML, relative to the config file.
    pub rules: Option<String>,
    /// Fall back to average-colour matching for unmatched materials.
    pub auto_palette: bool,
    /// Restrict auto-palette results to this named block set.
    pub palette_set: String,
    /// Block used when nothing matches and auto-palette is off.
    pub fallback_block: String,
    /// Extra directories searched for `.vmt`/`.vtf` content, alongside the
    /// BSP's embedded pakfile.
    pub game_dirs: Vec<String>,
}

impl Default for Materials {
    fn default() -> Self {
        Materials {
            rules: None,
            auto_palette: true,
            palette_set: "full".into(),
            fallback_block: "minecraft:stone".into(),
            game_dirs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillMode {
    /// Keep only voxels near a surface; hollow out the rest.
    Hollow,
    /// Fill brush interiors with `interior_block`.
    Solid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Fill {
    pub mode: FillMode,
    /// How many voxels deep the surface shell is.
    pub shell_thickness: u32,
    /// 6 = faces only, 26 = faces, edges and corners.
    pub shell_neighborhood: u8,
    pub interior_block: String,
}

impl Default for Fill {
    fn default() -> Self {
        Fill {
            mode: FillMode::Hollow,
            shell_thickness: 1,
            shell_neighborhood: 6,
            interior_block: "minecraft:stone".into(),
        }
    }
}

/// How voxel occupancy is decided for a brush.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleMode {
    /// Test the voxel centre only. Fastest.
    Center,
    /// Test an NxNxN grid of sample points and compare against `fill_threshold`.
    Samples,
    /// Clip the voxel cube against the brush and use the true volume fraction.
    Exact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Voxelize {
    pub mode: SampleMode,
    /// Grid resolution per axis for `samples` mode.
    pub samples: u32,
    /// Occupied fraction at or above which a voxel is filled.
    pub fill_threshold: f64,
    /// Guarantee at least one voxel layer for brushes thinner than a block, so
    /// E:Z's 4- and 8-unit trim does not disappear at 16 units/block.
    pub preserve_thin: bool,
}

impl Default for Voxelize {
    fn default() -> Self {
        Voxelize {
            mode: SampleMode::Samples,
            samples: 3,
            fill_threshold: 0.5,
            preserve_thin: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Displacement {
    pub enabled: bool,
    /// Voxels of solid backing added beneath a displacement surface, so
    /// terrain is not a one-block-thick shell.
    pub solidify: u32,
}

impl Default for Displacement {
    fn default() -> Self {
        Displacement { enabled: true, solidify: 2 }
    }
}

/// How brush entities (`func_door`, `func_brush`, ...) are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushEntityMode {
    /// Voxelize them into the world alongside worldspawn.
    Include,
    /// Voxelize them into their own schematic per entity.
    Separate,
    /// Record them in the manifest but do not voxelize.
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Entities {
    /// Write `entities.json`.
    pub manifest: bool,
    pub brush_entities: BrushEntityMode,
    /// Per-classname overrides of `brush_entities`.
    pub classname_modes: BTreeMap<String, BrushEntityMode>,
}

impl Default for Entities {
    fn default() -> Self {
        let mut classname_modes = BTreeMap::new();
        // Doors and platforms move, so their geometry is more useful on its own.
        for class in ["func_door", "func_door_rotating", "func_movelinear", "func_tracktrain"] {
            classname_modes.insert(class.to_string(), BrushEntityMode::Separate);
        }
        Entities {
            manifest: true,
            brush_entities: BrushEntityMode::Include,
            classname_modes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Output {
    /// Edge length in blocks of each emitted `.schem` tile. `None` writes the
    /// whole map as a single schematic.
    pub tile_size: Option<u32>,
    /// Emit a `dimension_type` JSON sized to the converted map.
    pub emit_dimension: bool,
    /// Write a WorldEdit paste macro next to the tiles.
    pub paste_script: bool,
    pub voxelize: Voxelize,
}

impl Default for Output {
    fn default() -> Self {
        Output {
            tile_size: Some(256),
            emit_dimension: false,
            paste_script: true,
            voxelize: Voxelize::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Performance {
    /// Worker threads; 0 uses one per core.
    pub threads: usize,
}

impl Default for Performance {
    fn default() -> Self {
        Performance { threads: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_all_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.scale.units_per_block, 16.0);
        assert_eq!(cfg.fill.mode, FillMode::Hollow);
        assert_eq!(cfg.output.tile_size, Some(256));
    }

    #[test]
    fn partial_config_keeps_other_defaults() {
        let cfg: Config = toml::from_str("[scale]\nunits_per_block = 8.0\n").unwrap();
        assert_eq!(cfg.scale.units_per_block, 8.0);
        assert_eq!(cfg.fill.shell_thickness, 1);
    }

    #[test]
    fn content_actions_parse_both_shapes() {
        let cfg: Config = toml::from_str(
            r#"
            [contents.actions]
            playerclip = "skip"
            water = { block = "minecraft:lava" }
            "#,
        )
        .unwrap();
        assert_eq!(cfg.contents.actions["playerclip"], ContentAction::Skip);
        assert_eq!(
            cfg.contents.actions["water"],
            ContentAction::Block("minecraft:lava".into())
        );
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err = toml::from_str::<Config>("[scale]\nunits_per_blok = 8.0\n").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn defaults_round_trip_through_toml() {
        let text = toml::to_string(&Config::default()).unwrap();
        let cfg: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg.scale.units_per_block, 16.0);
        assert_eq!(cfg.entities.brush_entities, BrushEntityMode::Include);
    }
}
