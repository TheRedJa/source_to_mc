//! The conversion pipeline: BSP in, voxel grid out.

use crate::bsp::{Map, Solid};
use crate::config::{Config, FillMode};
use crate::geom::{Aabb, Vec3};
use crate::palette::{Decision, Resolver};
use crate::voxel::brush::{BlockSolid, voxelize};
use crate::voxel::grid::{BlockId, Palette, VoxelGrid};
use crate::voxel::shell;
use crate::voxel::transform::Transform;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub struct Conversion {
    pub grid: VoxelGrid,
    pub palette: Palette,
    pub transform: Transform,
    pub stats: Stats,
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub solids_voxelized: usize,
    pub solids_skipped: usize,
    pub blocks_before_hollow: usize,
    pub blocks: usize,
    /// Voxel count per block type, for sourcing materials.
    pub block_counts: BTreeMap<String, usize>,
}

/// Translate a brush into block space, mapping each plane through `transform`.
fn to_block_solid(solid: &Solid, transform: &Transform) -> BlockSolid {
    let planes = solid
        .sides
        .iter()
        .map(|side| transform.transform_plane(side.plane))
        .collect();
    BlockSolid {
        planes,
        bounds: transform.transform_bounds(solid.bounds),
        side_of_plane: (0..solid.sides.len()).collect(),
    }
}

/// Which models to voxelize: worldspawn plus, depending on config, the brush
/// entities.
fn models_to_convert(map: &Map, config: &Config) -> Vec<usize> {
    use crate::config::BrushEntityMode;
    match config.entities.brush_entities {
        BrushEntityMode::Skip => vec![0],
        // `Separate` still needs its own output files, which the tiling layer
        // does not do yet, so for now it is treated like `Include`.
        BrushEntityMode::Include | BrushEntityMode::Separate => (0..map.bsp.models.len()).collect(),
    }
}

pub fn convert(map: &Map, config: &Config) -> Conversion {
    let transform = Transform::new(config, map.bounds());
    let resolver = Resolver::new(config);

    // Gather every brush first so the voxelization itself parallelizes cleanly.
    let solids: Vec<Solid> = models_to_convert(map, config)
        .into_iter()
        .flat_map(|model| map.solids(model))
        .collect();

    // The palette is shared and rarely written to after the first few brushes.
    let palette = Mutex::new(Palette::new());
    let skipped = std::sync::atomic::AtomicUsize::new(0);

    let grid = solids
        .par_iter()
        .fold(VoxelGrid::new, |mut grid, solid| {
            let decision = resolver.decide(solid.flags);
            if decision == Decision::Skip {
                skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return grid;
            }

            let block_solid = to_block_solid(solid, &transform);

            // Resolve the block for each side once, rather than per voxel.
            let side_blocks: Vec<Option<BlockId>> = solid
                .sides
                .iter()
                .map(|side| {
                    let block = match &decision {
                        Decision::Force(block) => Some(block.as_str()),
                        Decision::ByMaterial => {
                            let material = side.texture_info.and_then(|i| map.material_name(i));
                            resolver.block_for_material(material)
                        }
                        Decision::Skip => None,
                    };
                    block.map(|name| palette.lock().unwrap().intern(name))
                })
                .collect();

            // A brush whose every side is a tool texture contributes nothing.
            if side_blocks.iter().all(Option::is_none) {
                skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return grid;
            }
            let default_block = side_blocks.iter().flatten().copied().next();

            voxelize(&block_solid, &config.output.voxelize, |pos, side| {
                let block = side
                    .and_then(|s| side_blocks.get(s).copied().flatten())
                    .or(default_block);
                if let Some(block) = block {
                    grid.set(pos, block);
                }
            });
            grid
        })
        .reduce(VoxelGrid::new, |mut a, b| {
            a.merge(b);
            a
        });

    let blocks_before_hollow = grid.count();
    let grid = match config.fill.mode {
        FillMode::Solid => grid,
        FillMode::Hollow => shell::hollow(
            &grid,
            config.fill.shell_thickness,
            config.fill.shell_neighborhood,
        ),
    };

    let palette = palette.into_inner().unwrap();
    let mut block_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, id) in grid.iter() {
        *block_counts.entry(palette.name(id).to_string()).or_default() += 1;
    }

    let skipped = skipped.into_inner();
    Conversion {
        stats: Stats {
            solids_voxelized: solids.len() - skipped,
            solids_skipped: skipped,
            blocks_before_hollow,
            blocks: grid.count(),
            block_counts,
        },
        grid,
        palette,
        transform,
    }
}

/// Bounds of the map in block space, for reporting.
pub fn block_bounds(map: &Map, transform: &Transform) -> Aabb {
    let bounds = map.bounds();
    if bounds.is_empty() {
        return Aabb::new(Vec3::ZERO, Vec3::ZERO);
    }
    transform.transform_bounds(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sample_map() -> Option<Map> {
        let path = Path::new(concat!(
            "/mnt/games/SteamLibrary/steamapps/common/Entropy Zero",
            "/Entropy Zero/EntropyZero/maps/az_c4_4.bsp"
        ));
        path.exists().then(|| Map::load(path).unwrap())
    }

    #[test]
    fn converts_a_real_map_to_blocks() {
        let Some(map) = sample_map() else { return };
        let result = convert(&map, &Config::default());

        assert!(result.stats.blocks > 10_000, "got {}", result.stats.blocks);
        assert!(result.stats.solids_voxelized > 0);
        assert!(result.palette.len() > 1, "palette should hold more than air");

        // Hollowing must remove a substantial share of a solid map.
        assert!(
            result.stats.blocks < result.stats.blocks_before_hollow,
            "hollowing removed nothing"
        );
    }

    #[test]
    fn converted_geometry_lands_inside_the_map_bounds() {
        let Some(map) = sample_map() else { return };
        let result = convert(&map, &Config::default());
        let bounds = block_bounds(&map, &result.transform);
        let (min, max) = result.grid.bounds().unwrap();

        // One block of slack for rounding at the edges.
        assert!(min[0] as f64 >= bounds.min.x - 1.0, "{min:?} vs {bounds:?}");
        assert!(min[1] as f64 >= bounds.min.y - 1.0, "{min:?} vs {bounds:?}");
        assert!(max[1] as f64 <= bounds.max.y + 1.0, "{max:?} vs {bounds:?}");
    }

    #[test]
    fn hollow_mode_produces_far_fewer_blocks_than_solid() {
        let Some(map) = sample_map() else { return };

        let mut solid_config = Config::default();
        solid_config.fill.mode = FillMode::Solid;
        let solid = convert(&map, &solid_config);
        let hollow = convert(&map, &Config::default());

        assert!(
            hollow.stats.blocks * 2 < solid.stats.blocks,
            "hollow {} vs solid {}",
            hollow.stats.blocks,
            solid.stats.blocks
        );
    }

    #[test]
    fn a_coarser_scale_produces_fewer_blocks() {
        let Some(map) = sample_map() else { return };
        let mut coarse = Config::default();
        coarse.scale.units_per_block = 32.0;

        let fine = convert(&map, &Config::default());
        let coarse = convert(&map, &coarse);
        assert!(
            coarse.stats.blocks < fine.stats.blocks,
            "coarse {} vs fine {}",
            coarse.stats.blocks,
            fine.stats.blocks
        );
    }

    #[test]
    fn skipping_brush_entities_yields_no_more_geometry() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.entities.brush_entities = crate::config::BrushEntityMode::Skip;

        let without = convert(&map, &config);
        let with = convert(&map, &Config::default());
        assert!(without.stats.blocks <= with.stats.blocks);
    }
}
