//! `src2mc` — convert Source Engine BSP maps into Minecraft schematics.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use src2mc::bsp::{Map, entities};
use src2mc::config::Config;
use src2mc::inspect;
use src2mc::output::{dimension, layout, tiling};
use src2mc::voxel::transform::Transform;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "src2mc", version, about = "Convert Source Engine maps to Minecraft schematics")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Summarize a map: bounds, counts, and what converting it would cost.
    Inspect {
        map: PathBuf,
        #[command(flatten)]
        common: Common,
        /// Emit the report as JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show what every material in the map resolves to, and why.
    Materials {
        map: PathBuf,
        #[command(flatten)]
        common: Common,
        /// Emit the colour-matched materials as rules, ready to edit.
        #[arg(long)]
        stubs: bool,
        /// Only materials decided by colour matching or the fallback.
        #[arg(long, conflicts_with = "stubs")]
        guessed: bool,
        /// Emit the report as JSON.
        #[arg(long, conflicts_with_all = ["stubs", "guessed"])]
        json: bool,
    },
    /// Show which materials have a real Source texture behind them.
    Textures {
        map: PathBuf,
        #[command(flatten)]
        common: Common,
        /// Only materials whose texture could not be found.
        #[arg(long)]
        missing: bool,
        /// Emit the report as JSON.
        #[arg(long, conflicts_with = "missing")]
        json: bool,
    },
    /// Voxelize a map and write Sponge v3 schematic tiles.
    Convert {
        map: PathBuf,
        #[command(flatten)]
        common: Common,
        /// Output directory.
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        #[command(flatten)]
        options: ConvertOptions,
    },
    /// Convert several maps into one output directory, laid out side by side.
    Batch {
        /// Maps to convert.
        #[arg(required = true)]
        maps: Vec<PathBuf>,
        #[command(flatten)]
        common: Common,
        /// Output directory; each map gets a subdirectory.
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        /// How to place the maps relative to each other.
        #[arg(long, value_enum, default_value_t = Layout::Grid)]
        layout: Layout,
        /// Blocks of clear space between maps under `grid`.
        #[arg(long, default_value_t = 256)]
        spacing: u32,
        #[command(flatten)]
        options: ConvertOptions,
    },
    /// Dump every entity, with positions mapped to Minecraft coordinates.
    Entities {
        map: PathBuf,
        #[command(flatten)]
        common: Common,
        /// Write to this file instead of stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Only entities with this classname; repeatable.
        #[arg(long = "classname")]
        classnames: Vec<String>,
    },
}

/// Flags shared by `convert` and `batch`.
#[derive(Args, Clone)]
struct ConvertOptions {
    /// Edge length in blocks of each schematic tile (max 32767).
    #[arg(long)]
    tile_size: Option<u32>,
    /// Write each map as one schematic instead of tiles.
    #[arg(long, conflicts_with = "tile_size")]
    single: bool,
    /// Fill brush interiors instead of hollowing them out.
    #[arg(long)]
    solid: bool,
    /// Voxels of surface kept when hollowing.
    #[arg(long)]
    shell_thickness: Option<u32>,
    /// Leave out displacement terrain.
    #[arg(long)]
    no_displacements: bool,
    /// Leave out `prop_static` models: fences, railings, catwalks, crates.
    #[arg(long)]
    no_props: bool,
    /// Also write a datapack defining a dimension tall enough for the map.
    #[arg(long)]
    emit_dimension: bool,
    /// Where surface blocks come from. `kubejs` generates a block per material
    /// carrying its real Source texture, plus a KubeJS pack registering them.
    #[arg(long, value_enum, default_value_t = Textures::Vanilla)]
    textures: Textures,
}

/// Where a surface's block comes from.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Textures {
    /// Vanilla blocks, chosen by rules and average colour. No mods needed.
    Vanilla,
    /// Blocks generated from the map's own textures, registered by KubeJS.
    Kubejs,
}

impl ConvertOptions {
    fn apply(&self, config: &mut Config) {
        if self.single {
            config.output.tile_size = None;
        } else if let Some(size) = self.tile_size {
            config.output.tile_size = Some(size);
        }
        if self.solid {
            config.fill.mode = src2mc::config::FillMode::Solid;
        }
        if let Some(thickness) = self.shell_thickness {
            config.fill.shell_thickness = thickness;
        }
        if self.no_displacements {
            config.displacement.enabled = false;
        }
        if self.no_props {
            config.props.enabled = false;
        }
        if self.emit_dimension {
            config.output.emit_dimension = true;
        }
        config.materials.mode = match self.textures {
            Textures::Vanilla => src2mc::config::MaterialMode::Vanilla,
            Textures::Kubejs => src2mc::config::MaterialMode::Kubejs,
        };
    }
}

/// How `batch` places maps relative to one another.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Layout {
    /// Side by side in a grid, so no two maps overlap.
    Grid,
    /// All at the same origin, for comparing versions of one map.
    Stacked,
}

#[derive(Args, Clone)]
struct Common {
    /// Configuration TOML; defaults are used for anything it omits.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Source units per Minecraft block; overrides the config.
    #[arg(long)]
    units_per_block: Option<f64>,
    /// Material rules TOML, tested before the built-in rules.
    #[arg(long)]
    rules: Option<PathBuf>,
    /// Blocks colour matching may pick: `full`, or a comma-separated list of
    /// stone, concrete, wool, terracotta, wood, natural, metal, nether.
    #[arg(long)]
    palette_set: Option<String>,
    /// Ignore the built-in Half-Life 2 / Entropy: Zero material rules.
    #[arg(long)]
    no_builtin_rules: bool,
    /// Extra game directory to search for `.vmt`/`.vtf` content; repeatable.
    /// The map's own game directory and whatever its `gameinfo.txt` mounts are
    /// searched automatically.
    #[arg(long = "game-dir")]
    game_dirs: Vec<PathBuf>,
}

impl Common {
    fn resolve(&self) -> Result<Config> {
        let mut config = match &self.config {
            Some(path) => Config::load(path)?,
            None => Config::default(),
        };
        if let Some(units) = self.units_per_block {
            anyhow::ensure!(units > 0.0, "--units-per-block must be positive");
            config.scale.units_per_block = units;
        }
        if let Some(rules) = &self.rules {
            // A path given on the command line is relative to the shell's
            // directory, not to whatever config file was also passed.
            config.materials.rules = Some(rules.display().to_string());
            config.load_rules(Path::new("."))?;
        }
        if let Some(set) = &self.palette_set {
            config.materials.palette_set = set.clone();
        }
        if self.no_builtin_rules {
            config.materials.builtin_rules = false;
        }
        config
            .materials
            .game_dirs
            .extend(self.game_dirs.iter().map(|p| p.display().to_string()));
        Ok(config)
    }
}

fn load(path: &Path) -> Result<Map> {
    Map::load(path).with_context(|| format!("loading map {}", path.display()))
}

/// What one converted map contributed, for the batch index.
struct Converted {
    name: String,
    blocks: usize,
    /// Generated blocks, so `batch` can merge them into one pack.
    pack: src2mc::output::kubejs::Pack,
    manifest: tiling::Manifest,
    y_range: src2mc::inspect::YRangeReport,
    origin: [i32; 3],
}

/// Voxelize one map and write its schematics, manifest, paste script and
/// entity dump into `out`.
fn convert_one(map: &Map, config: &Config, out: &Path) -> Result<Converted> {
    convert_into(map, config, out, true)
}

/// `write_pack` is false for `batch`, which merges every map's generated
/// blocks into one pack at the top level instead: a texture shared across a
/// campaign should be registered once, not once per map.
fn convert_into(
    map: &Map,
    config: &Config,
    out: &Path,
    write_pack: bool,
) -> Result<Converted> {
    eprintln!("converting {} at {} units/block...", map.name, config.scale.units_per_block);

    let result = src2mc::convert::convert(map, config)?;
    let stats = &result.stats;
    eprintln!(
        "  {} brushes voxelized ({} skipped), {} displacements ({} skipped)",
        stats.solids_voxelized,
        stats.solids_skipped,
        stats.displacements_voxelized,
        stats.displacements_skipped,
    );
    if stats.props_placed > 0 || stats.props_skipped > 0 {
        eprintln!(
            "  {} static props placed ({} skipped)",
            stats.props_placed, stats.props_skipped,
        );
    }
    eprintln!(
        "  {} blocks after hollowing (from {})",
        stats.blocks, stats.blocks_before_hollow,
    );
    if stats.shapes_fitted > 0 {
        eprintln!(
            "  {} blocks fitted to slabs or stairs ({:.1}%)",
            stats.shapes_fitted,
            100.0 * stats.shapes_fitted as f64 / stats.blocks.max(1) as f64,
        );
    }
    if config.materials.mode == src2mc::config::MaterialMode::Kubejs {
        let split = result.pack.tilings().len();
        eprintln!(
            "  {} materials carry their own texture ({split} split across several blocks)",
            stats.textures_resolved
        );
    }

    if stats.blocks == 0 {
        eprintln!(
            "  warning: no blocks produced. Every surface was skipped, which is \
             correct for a credits or skybox-only map but otherwise suggests the \
             material rules are dropping too much — check `src2mc materials`."
        );
    }

    let mut manifest = tiling::write_tiles(
        out,
        &map.name,
        &result.grid,
        &result.palette,
        config.output.tile_size,
        config.scale.units_per_block,
        stats.block_counts.clone(),
    )?;

    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;

    if !result.pack.is_empty() {
        if write_pack {
            let written = result.pack.write(out)?;
            eprintln!(
                "  {} generated blocks ({} KB of textures) in {}",
                written.blocks,
                written.texture_bytes / 1024,
                written.root.display(),
            );
        }
        manifest.generated_blocks = result
            .pack
            .blocks()
            .map(|b| b.block_id())
            .collect();
    }

    manifest.entities = tiling::write_entities(out, &result.separate, &result.palette)?;
    if !manifest.entities.is_empty() {
        eprintln!(
            "  {} moving brush entities written separately",
            manifest.entities.len()
        );
    }
    std::fs::write(out.join("manifest.json"), serde_json::to_string_pretty(&manifest)?)?;

    if config.output.paste_script {
        std::fs::write(out.join("paste.txt"), tiling::paste_script(&manifest))?;
    }

    if config.entities.manifest {
        let records = entities::extract(map, &result.transform);
        std::fs::write(out.join("entities.json"), serde_json::to_string_pretty(&records)?)?;
        eprintln!("  {} entities recorded", records.len());
    }

    eprintln!("  {} tiles written to {}", manifest.tiles.len(), out.display());

    Ok(Converted {
        name: map.name.clone(),
        blocks: stats.blocks,
        pack: result.pack,
        manifest,
        y_range: inspect::y_range(src2mc::convert::block_bounds(map, &result.transform)),
        origin: config.transform.offset,
    })
}

/// Convert several maps into one output directory.
///
/// Under `grid` each map is offset so none overlaps another, which is what
/// makes a whole campaign walkable end to end. The offsets are computed from
/// each map's own converted size rather than a fixed stride, because Source
/// map sizes vary by an order of magnitude and a stride large enough for the
/// biggest would leave the rest adrift in empty space.
fn batch(
    maps: &[PathBuf],
    config: &Config,
    out: &Path,
    layout: Layout,
    spacing: u32,
) -> Result<()> {
    // Sizes first, so the layout is known before anything is written.
    let mut loaded = Vec::with_capacity(maps.len());
    let mut footprints = Vec::with_capacity(maps.len());
    for path in maps {
        let map = load(path)?;
        let size = Transform::new(config, map.bounds())
            .transform_bounds(map.bounds())
            .size();
        footprints.push(layout::Footprint::new(size.x.ceil() as i32, size.z.ceil() as i32));
        loaded.push(map);
    }

    let placements = match layout {
        Layout::Stacked => layout::stacked(loaded.len()),
        Layout::Grid => layout::grid(
            &footprints,
            layout::columns_for(loaded.len()),
            spacing as i32,
        ),
    };

    let mut converted = Vec::with_capacity(loaded.len());
    let mut failures = Vec::new();
    for (map, origin) in loaded.iter().zip(placements) {
        let mut config = config.clone();
        config.transform.offset = origin;
        let dir = out.join(&map.name);
        match convert_into(map, &config, &dir, false) {
            Ok(result) => converted.push(result),
            // One bad map should not lose the rest of a campaign's work.
            Err(error) => {
                eprintln!("  error: {} failed: {error:#}", map.name);
                failures.push(map.name.clone());
            }
        }
    }

    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;

    // One pack for the whole batch, so a texture shared between maps is
    // registered once and every map's schematics resolve against it.
    let mut pack = src2mc::output::kubejs::Pack::default();
    for result in &mut converted {
        pack.merge(std::mem::take(&mut result.pack));
    }
    if !pack.is_empty() {
        let written = pack.write(out)?;
        eprintln!(
            "{} generated blocks shared across the batch ({} KB of textures) in {}",
            written.blocks,
            written.texture_bytes / 1024,
            written.root.display(),
        );
    }

    let index: Vec<_> = converted
        .iter()
        .map(|c| {
            serde_json::json!({
                "map": c.name,
                "origin": c.origin,
                "blocks": c.blocks,
                "tiles": c.manifest.tiles.len(),
                "min_y": c.y_range.min_y,
                "max_y": c.y_range.max_y,
            })
        })
        .collect();
    std::fs::write(
        out.join("batch.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "maps": index,
            "failed": failures,
        }))?,
    )?;

    // One script that pastes every tile of every map, in order.
    let mut script = String::from(
        "# Generated by src2mc batch. Paste with WorldEdit or FAWE.\n\
         # Each map is offset so none overlaps another; -o places every tile\n\
         # at the coordinates baked into it, so standing still is not required.\n\n",
    );
    for c in &converted {
        script.push_str(&format!("# --- {} ({} blocks) ---\n", c.name, c.blocks));
        for line in tiling::paste_script(&c.manifest).lines() {
            if !line.starts_with('#') && !line.trim().is_empty() {
                script.push_str(line);
                script.push('\n');
            }
        }
        script.push('\n');
    }
    std::fs::write(out.join("paste_all.txt"), script)?;

    if config.output.emit_dimension && !converted.is_empty() {
        // One dimension has to hold every map, so take the union of their
        // ranges rather than the last one's.
        let min_y = converted.iter().map(|c| c.y_range.min_y).min().unwrap();
        let max_y = converted.iter().map(|c| c.y_range.max_y).max().unwrap();
        let union = inspect::y_range(src2mc::geom::Aabb::new(
            src2mc::geom::Vec3::new(0.0, min_y as f64, 0.0),
            src2mc::geom::Vec3::new(0.0, max_y as f64, 0.0),
        ));
        let dir = out.join("dimension");
        let emitted = dimension::write(&dir, "batch", &union)?;
        eprintln!("{}", emitted.instructions(&dir));
    }

    eprintln!(
        "\n{} of {} maps converted, {} blocks total, index at {}",
        converted.len(),
        maps.len(),
        converted.iter().map(|c| c.blocks).sum::<usize>(),
        out.join("batch.json").display(),
    );
    if !failures.is_empty() {
        eprintln!("failed: {}", failures.join(", "));
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Inspect { map, common, json } => {
            let config = common.resolve()?;
            let map = load(&map)?;
            let report = inspect::report(&map, &config);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render());
            }
        }

        Command::Materials { map, common, stubs, guessed, json } => {
            let config = common.resolve()?;
            let map = load(&map)?;
            let mut report = src2mc::palette::report::report(&map, &config)?;
            if guessed {
                report.entries.retain(|e| e.decided_by == "auto" || e.decided_by == "fallback");
            }
            if stubs {
                print!("{}", report.stubs());
            } else if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render());
            }
        }

        Command::Textures { map, common, missing, json } => {
            let config = common.resolve()?;
            let map = load(&map)?;
            let mut report = src2mc::source::report::report(&map, &config);
            if missing {
                report.entries.retain(|e| !e.resolved && e.uses > 0);
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render());
            }
        }

        Command::Convert { map, common, out, options } => {
            let mut config = common.resolve()?;
            options.apply(&mut config);
            let map = load(&map)?;
            let converted = convert_one(&map, &config, &out)?;

            if config.output.emit_dimension {
                let emitted = dimension::write(&out.join("dimension"), &map.name, &converted.y_range)?;
                eprintln!("  {}", emitted.instructions(&out.join("dimension")));
            }
        }

        Command::Batch { maps, common, out, layout, spacing, options } => {
            let mut config = common.resolve()?;
            options.apply(&mut config);
            batch(&maps, &config, &out, layout, spacing)?;
        }

        Command::Entities { map, common, out, classnames } => {
            let config = common.resolve()?;
            let map = load(&map)?;
            let transform = Transform::new(&config, map.bounds());
            let mut records = entities::extract(&map, &transform);
            if !classnames.is_empty() {
                records.retain(|e| classnames.iter().any(|c| c == &e.classname));
            }
            let json = serde_json::to_string_pretty(&records)?;
            match out {
                Some(path) => {
                    std::fs::write(&path, json)
                        .with_context(|| format!("writing {}", path.display()))?;
                    eprintln!("wrote {} entities to {}", records.len(), path.display());
                }
                None => println!("{json}"),
            }
        }
    }
    Ok(())
}
