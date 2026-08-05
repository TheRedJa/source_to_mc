//! `src2mc` — convert Source Engine BSP maps into Minecraft schematics.

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use src2mc::bsp::{Map, entities};
use src2mc::config::Config;
use src2mc::inspect;
use src2mc::output::tiling;
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
    /// List every material the map references.
    Materials {
        map: PathBuf,
        #[command(flatten)]
        common: Common,
    },
    /// Voxelize a map and write Sponge v3 schematic tiles.
    Convert {
        map: PathBuf,
        #[command(flatten)]
        common: Common,
        /// Output directory.
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        /// Edge length in blocks of each schematic tile (max 32767).
        #[arg(long)]
        tile_size: Option<u32>,
        /// Write the whole map as one schematic instead of tiles.
        #[arg(long, conflicts_with = "tile_size")]
        single: bool,
        /// Fill brush interiors instead of hollowing them out.
        #[arg(long)]
        solid: bool,
        /// Voxels of surface kept when hollowing.
        #[arg(long)]
        shell_thickness: Option<u32>,
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

#[derive(Args, Clone)]
struct Common {
    /// Configuration TOML; defaults are used for anything it omits.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Source units per Minecraft block; overrides the config.
    #[arg(long)]
    units_per_block: Option<f64>,
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
        Ok(config)
    }
}

fn load(path: &Path) -> Result<Map> {
    Map::load(path).with_context(|| format!("loading map {}", path.display()))
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

        Command::Materials { map, common } => {
            common.resolve()?;
            let map = load(&map)?;
            for material in map.materials() {
                println!("{material}");
            }
        }

        Command::Convert { map, common, out, tile_size, single, solid, shell_thickness } => {
            let mut config = common.resolve()?;
            if single {
                config.output.tile_size = None;
            } else if let Some(size) = tile_size {
                config.output.tile_size = Some(size);
            }
            if solid {
                config.fill.mode = src2mc::config::FillMode::Solid;
            }
            if let Some(thickness) = shell_thickness {
                config.fill.shell_thickness = thickness;
            }

            let map = load(&map)?;
            eprintln!("converting {} at {} units/block...", map.name, config.scale.units_per_block);

            let result = src2mc::convert::convert(&map, &config);
            let stats = &result.stats;
            eprintln!(
                "  {} brushes voxelized ({} skipped), {} blocks after hollowing (from {})",
                stats.solids_voxelized,
                stats.solids_skipped,
                stats.blocks,
                stats.blocks_before_hollow,
            );

            let manifest = tiling::write_tiles(
                &out,
                &map.name,
                &result.grid,
                &result.palette,
                config.output.tile_size,
                config.scale.units_per_block,
                stats.block_counts.clone(),
            )?;

            std::fs::create_dir_all(&out)
                .with_context(|| format!("creating {}", out.display()))?;
            std::fs::write(
                out.join("manifest.json"),
                serde_json::to_string_pretty(&manifest)?,
            )?;

            if config.output.paste_script {
                std::fs::write(out.join("paste.txt"), tiling::paste_script(&manifest))?;
            }

            if config.entities.manifest {
                let records = entities::extract(&map, &result.transform);
                std::fs::write(
                    out.join("entities.json"),
                    serde_json::to_string_pretty(&records)?,
                )?;
                eprintln!("  {} entities recorded", records.len());
            }

            eprintln!("  {} tiles written to {}", manifest.tiles.len(), out.display());
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
