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
        Scale {
            units_per_block: 16.0,
        }
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
        actions.insert(
            "water".into(),
            ContentAction::Block("minecraft:water".into()),
        );
        actions.insert(
            "slime".into(),
            ContentAction::Block("minecraft:water".into()),
        );
        actions.insert(
            "window".into(),
            ContentAction::Block("minecraft:glass".into()),
        );
        actions.insert(
            "grate".into(),
            ContentAction::Block("minecraft:iron_bars".into()),
        );
        actions.insert(
            "ladder".into(),
            ContentAction::Block("minecraft:ladder".into()),
        );
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
    /// Blocks one generated pack may register, or 0 for no limit.
    ///
    /// The real cost of splitting textures is not disk — 100k blocks is about
    /// 60 MB of 16x16 PNGs — but what a KubeJS instance pays to register them
    /// at startup. So the control is a ceiling on the pack, which means the
    /// same setting behaves sensibly for a single room and for a campaign:
    /// textures are cut as finely as the budget allows and no finer.
    ///
    /// Entropy: Zero's 17 maps as one pack come to about 92k blocks with every
    /// texture at full resolution, so the default lets that through untouched.
    pub max_blocks: usize,
    /// Hard ceiling on tiles per axis, whatever the budget allows.
    ///
    /// Rarely the binding constraint: a texture is never cut finer than its
    /// own resolution can feed, so most materials stop well short of this and
    /// the budget decides the rest.
    pub tile_max: u32,
    /// The rules named by `rules`, filled in by [`Config::load`].
    #[serde(skip)]
    pub loaded_rules: crate::palette::rules::Rules,
}

impl Materials {
    /// `game_dirs` as paths.
    pub fn game_dir_paths(&self) -> Vec<std::path::PathBuf> {
        self.game_dirs
            .iter()
            .map(std::path::PathBuf::from)
            .collect()
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
            max_blocks: 100_000,
            tile_max: 64,
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
        Displacement {
            enabled: true,
            solidify: 2,
        }
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
    /// Render props as their real triangle mesh instead of voxelizing them.
    ///
    /// A forklift, a car or a rock was never designed for a grid, so cubing it
    /// is where conversion looks worst. NeoForge's OBJ model loader takes an
    /// arbitrary mesh, and a `block_display` entity places it at any angle, so
    /// the prop can simply be itself. Needs the generated pack, so this only
    /// applies in `kubejs` material mode.
    pub models: bool,
    /// Also read props out of the entity lump — `prop_physics`, `prop_dynamic`
    /// and anything else naming a `.mdl` — rather than only the `sprp` lump.
    pub entity_props: bool,
    /// Resolution of a prop's own textures. Far above the 16x16 a block face
    /// gets, because a mesh shows its texture at real scale rather than
    /// squeezed onto one cube.
    pub texture_size: u32,
    /// How many times a texture may be repeated into one image so that a
    /// model's tiling UVs stay inside the atlas sprite. See `output::obj`.
    pub texture_repeat_max: u32,
    /// Move a prop vertically so it stands on the floor the conversion built
    /// rather than in it.
    ///
    /// A prop is placed to a fraction of a block; the floor under it is
    /// voxelized to whole ones, so the two disagree by up to a block and a
    /// crate sinks into the ground. Cubing the prop hid that, because the
    /// cubes were rounded by the same grid.
    pub settle: bool,
    /// How far settling may move a prop, in blocks.
    ///
    /// The correction exists to undo the grid's own rounding, so a block
    /// covers it. Further than that is not rounding — it is a prop over a hole
    /// the conversion did not build — and moving it there would invent a
    /// position rather than recover one.
    pub settle_max: f64,
    /// Props whose longest dimension is at least this many Source units get
    /// invisible barrier blocks so they are solid. Smaller clutter is left
    /// walk-through; 0 makes everything solid, and a huge value nothing.
    pub collision_min_size: f64,
    /// Draw props as blocks with their rotation baked in, rather than as
    /// display entities.
    ///
    /// A display entity is redrawn every frame instead of being baked into the
    /// chunk mesh, so a few hundred props in view costs real frame rate. A
    /// block costs nothing once its chunk is built, and its faces are lit
    /// individually rather than the whole prop taking the light of one cell.
    /// The price is a registered block per distinct placement.
    pub bake: bool,
    /// Steps per block that a baked prop's position within its block is
    /// rounded to. Higher is more faithful and shares fewer blocks between
    /// placements. At the default 16 units per block this represents every
    /// whole Hammer unit exactly, so props on Source's own grid do not move.
    pub bake_grid: i64,
    /// Steps a baked prop's rotation is rounded to, per quaternion component.
    ///
    /// 256 is about a fifth of a degree, which moves the far end of even a
    /// large prop by a couple of centimetres. Coarser rounding shares blocks
    /// between placements at odd angles, but hardly any: a prop turned by a
    /// whole number of degrees rounds to itself at any setting, and that is
    /// nearly all of them.
    pub bake_angle_steps: i64,
    /// Furthest a baked prop's geometry may sit from the block carrying it,
    /// in blocks. A prop reaching further is split across several blocks.
    ///
    /// Sodium packs each chunk vertex coordinate into 20 bits spanning -8 to
    /// +24 blocks from the section origin, and masks off what does not fit, so
    /// a model reaching past that folds back on itself — correct up to a point
    /// and then inside out. A block can sit anywhere in its 16-block section,
    /// which leaves 8 blocks either way as the reach that is safe wherever the
    /// block lands. Vanilla is more forgiving, but not by enough to matter.
    pub bake_reach: f64,
    /// Props longer than this many Source units stay display entities however
    /// `bake` is set; 0 bakes every size.
    ///
    /// A block's model is filed under the chunk section holding that block, so
    /// a mesh reaching far beyond it appears and disappears with a section it
    /// is barely in. For anything of a normal prop's size that is invisible.
    pub bake_max_size: f64,
    /// `view_range` on the generated entities: distances beyond this times 64
    /// blocks stop rendering. Below 1.0 trades draw distance for frame rate.
    /// Only reaches the props left as entities; a baked one is part of its
    /// chunk and is drawn whenever the chunk is.
    pub view_range: f32,
    /// Models with more triangles than this are voxelized instead of rendered.
    /// 0 keeps every model however heavy.
    pub max_triangles: usize,
    /// Light the prop fully rather than by the block it stands in. A display
    /// entity is lit at one point, so a large mesh in a dark cell goes black.
    /// A baked prop is lit face by face by the chunk mesher and ignores this.
    pub full_bright: bool,
    /// Mirror the V texture axis.
    ///
    /// Off, because Source and Minecraft agree: both run V downwards from the
    /// top of the image. The loader offers the flip for models authored in an
    /// OpenGL tool, and turning it on here mirrors every sheet — which on a
    /// model texture with unused areas shows up as half a prop wearing blank
    /// texture and the rest wearing pieces of something else.
    pub flip_v: bool,
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
            models: true,
            entity_props: true,
            texture_size: 128,
            texture_repeat_max: 4,
            settle: true,
            settle_max: 1.0,
            collision_min_size: 48.0,
            bake: true,
            bake_grid: 16,
            bake_angle_steps: 256,
            bake_reach: 8.0,
            bake_max_size: 0.0,
            view_range: 1.0,
            max_triangles: 4_000,
            full_bright: false,
            flip_v: false,
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
        for class in [
            "func_door",
            "func_door_rotating",
            "func_movelinear",
            "func_tracktrain",
        ] {
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
#[derive(Default)]
pub struct Performance {
    /// Worker threads; 0 uses one per core.
    pub threads: usize,
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
