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
    pub shapes: Shapes,
    pub displacement: Displacement,
    pub props: Props,
    pub entities: Entities,
    pub output: Output,
    pub performance: Performance,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let mut config: Config =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
        // A `rules` path is relative to the config that names it, so that a
        // config and its rules can be moved together.
        config.load_rules(path.parent().unwrap_or(Path::new(".")))?;
        Ok(config)
    }

    /// Read the rules file named by `[materials] rules`, resolving a relative
    /// path against `base`.
    pub fn load_rules(&mut self, base: &Path) -> Result<()> {
        let Some(rules) = &self.materials.rules else {
            return Ok(());
        };
        let path = base.join(rules);
        self.materials.loaded_rules = crate::palette::rules::Rules::load(&path)?;
        Ok(())
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
    /// Block-space offset applied last, after origin and rotation. `batch`
    /// uses it to lay several maps out side by side.
    pub offset: [i32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            origin_mode: OriginMode::BoundsMin,
            y_base: 0,
            rotate_yaw: 0.0,
            offset: [0, 0, 0],
        }
    }
}

/// Where a surface's block comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialMode {
    /// Vanilla blocks only, chosen by rules and average colour.
    Vanilla,
    /// Generate a block per material carrying its real Source texture, and
    /// emit a KubeJS pack registering them. Needs KubeJS installed to paste.
    Kubejs,
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
    /// Leave out the 3D skybox room: the sealed miniature of the horizon that
    /// the engine renders scaled up and far away. Converted literally it is a
    /// second, wrongly-sized map sitting in a corner of the first, and most of
    /// the empty volume between them.
    pub skip_3d_skybox: bool,
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
            skip_3d_skybox: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Materials {
    /// Vanilla blocks, or generated blocks wearing the map's own textures.
    pub mode: MaterialMode,
    /// Path to a rules TOML, relative to the config file. Its rules are tested
    /// before the built-in ones, so they win.
    pub rules: Option<String>,
    /// Also use the Half-Life 2 / Entropy: Zero rules shipped in the binary.
    pub builtin_rules: bool,
    /// Fall back to average-colour matching for unmatched materials.
    pub auto_palette: bool,
    /// Restrict auto-palette results to this named block set: `full`, or a
    /// comma-separated list of `stone`, `concrete`, `wool`, `terracotta`,
    /// `wood`, `natural`, `metal`, `nether`.
    pub palette_set: String,
    /// Block used when nothing matches and auto-palette is off.
    pub fallback_block: String,
    /// Extra game directories searched for `.vmt`/`.vtf` content, alongside
    /// the map's own game directory and whatever its `gameinfo.txt` mounts.
    pub game_dirs: Vec<String>,
    /// Edge length of generated block textures, in pixels. 16 matches vanilla.
    pub texture_size: u32,
    /// Split each texture across as many blocks as it really covers in the
    /// map, instead of squeezing all of it onto every block face.
    ///
    /// A 512-pixel wall texture at Hammer's default scale covers eight blocks
    /// of wall. Shrunk onto one block face it is a smear; cut into the pieces
    /// that are really in front of each block, the detail comes back and the
    /// pattern lines up across the wall. Costs one registered block per tile.
    pub tile_textures: bool,
    /// Largest number of tiles a texture may be split into along one axis.
    ///
    /// The cap on how many blocks the pack registers. 8 covers the common
    /// case exactly — a 512 texture at scale 0.25 — and past that a tile
    /// spans several blocks rather than being dropped, so raising it buys
    /// resolution on the largest textures and nothing else.
    pub tile_max: u32,
    /// The rules named by `rules`, filled in by [`Config::load`].
    #[serde(skip)]
    pub loaded_rules: crate::palette::rules::Rules,
}

impl Materials {
    /// `game_dirs` as paths.
    pub fn game_dir_paths(&self) -> Vec<std::path::PathBuf> {
        self.game_dirs.iter().map(std::path::PathBuf::from).collect()
    }
}

impl Default for Materials {
    fn default() -> Self {
        Materials {
            mode: MaterialMode::Vanilla,
            rules: None,
            builtin_rules: true,
            auto_palette: true,
            palette_set: "full".into(),
            fallback_block: "minecraft:stone".into(),
            game_dirs: Vec::new(),
            texture_size: 16,
            tile_textures: true,
            tile_max: 8,
            loaded_rules: Default::default(),
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

/// Fitting geometry to Minecraft's half-height and stepped blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Shapes {
    /// Emit slabs and stairs where the geometry is half-height or stepped.
    ///
    /// Vanilla blocks only. KubeJS 2101 exposes exactly two block builders,
    /// `basic` and `detector`, so a generated textured block cannot be a slab
    /// or a stair and keeps its full cube.
    pub enabled: bool,
}

impl Default for Shapes {
    fn default() -> Self {
        Shapes { enabled: true }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Props {
    /// Voxelize `prop_static` models into the world.
    ///
    /// Needs the game's content on the search path, since a map stores only
    /// the path to each model. Without it this quietly does nothing, exactly
    /// as texture extraction does.
    pub enabled: bool,
    /// Ignore props whose longest dimension is under this many Source units.
    /// At 16 units per block anything smaller cannot be more than a stray
    /// cube, and maps are full of pebbles, cans and tufts of grass.
    pub min_size: f64,
    /// Ignore props whose longest dimension is over this many Source units;
    /// 0 keeps every size.
    ///
    /// The lever for backdrop scenery. Distant architecture — Half-Life 2's
    /// Citadel, Entropy: Zero's Combine walls — is placed as ordinary props
    /// thousands of units across, and converting one is tens of thousands of
    /// blocks of a single dark material. Off by default, because that scenery
    /// is really there and dropping it silently is the worse surprise.
    pub max_size: f64,
    /// Glob patterns matched against the model path; a prop matching any of
    /// them is skipped.
    pub skip: Vec<String>,
    /// Give thin props this many extra voxels of backing along the surface
    /// normal, as displacements get. Props are usually closed shells already,
    /// so the default is none.
    pub solidify: u32,
}

impl Default for Props {
    fn default() -> Self {
        Props {
            enabled: true,
            min_size: 12.0,
            max_size: 0.0,
            // Foliage is alpha-tested cards that voxelize into solid slabs,
            // and there are thousands of them in an outdoor map.
            skip: vec!["*props_foliage*".into(), "*/foliage/*".into()],
            solidify: 0,
        }
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
