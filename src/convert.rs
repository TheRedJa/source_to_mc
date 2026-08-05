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

/// Texture flags that mean "this face is never drawn", so it should never
/// become blocks either. Checked per side, because a brush commonly mixes one
/// visible face with five nodraw ones.
fn is_invisible(flags: vbsp::TextureFlags) -> bool {
    use vbsp::TextureFlags as F;
    flags.intersects(F::NODRAW | F::SKIP | F::HINT | F::TRIGGER)
}

/// The block one brush side contributes, or `None` if it contributes nothing.
///
/// The material can veto the brush's contents, which matters more than it
/// sounds. A fog volume is a brush flagged `WINDOW`, because fog is
/// translucent, wearing `tools/toolsfog`. Taking the contents at face value
/// turns a city block of atmosphere into a solid cube of glass: in `az_c1_2`
/// that was 1.9 million glass blocks, six times the rest of the map put
/// together. The material knows it is not a window.
fn side_block<'a>(
    decision: &'a Decision,
    resolver: &'a Resolver,
    material: Option<usize>,
    flags: vbsp::TextureFlags,
    skip_sky: bool,
) -> Option<&'a str> {
    use vbsp::TextureFlags as F;
    if is_invisible(flags) || (skip_sky && flags.intersects(F::SKY | F::SKY2D)) {
        return None;
    }

    match material {
        // A material that resolves to no block drops the side outright.
        Some(index) => {
            let block = resolver.block_for_material(index)?;
            match decision {
                // A rule that names a block outranks the contents flag, which
                // is only ever a guess about what the brush is. Entropy: Zero
                // 2's arctic maps flag their snow sheets `WINDOW` because they
                // are translucent; the `*snow*` rule knows better than to make
                // them glass.
                Decision::Force(_) if resolver.named_by_rule(index) => Some(block),
                Decision::Force(forced) => Some(forced),
                Decision::ByMaterial => Some(block),
                Decision::Skip => None,
            }
        }
        // A side with no texture info at all still bounds the brush, so it
        // gets the fallback rather than punching a hole in it.
        None => match decision {
            Decision::Force(forced) => Some(forced),
            Decision::ByMaterial => Some(resolver.fallback_block()),
            Decision::Skip => None,
        },
    }
}

pub fn convert(map: &Map, config: &Config) -> anyhow::Result<Conversion> {
    let transform = Transform::new(config, map.bounds());
    let resolver = Resolver::new(config, map.materials())?;
    let skip_sky = config.contents.skip_sky;

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
                    let material = side.texture_info.and_then(|i| map.material_index(i));
                    side_block(&decision, &resolver, material, side.texture_flags, skip_sky)
                        .map(|name| palette.lock().unwrap().intern(name))
                })
                .collect();

            // A brush whose every side was vetoed contributes nothing.
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
    Ok(Conversion {
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
    })
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
    use crate::bsp::Material;
    use std::path::Path;
    use vbsp::TextureFlags;

    fn resolver(materials: &[Material]) -> Resolver {
        Resolver::new(&Config::default(), materials).unwrap()
    }

    fn material(name: &str) -> Material {
        Material {
            name: name.into(),
            raw_name: name.into(),
            reflectivity: [0.2, 0.2, 0.2],
        }
    }

    /// Fog volumes are brushes flagged `WINDOW`, because fog is translucent.
    /// Trusting the contents flag alone paves a city block in glass.
    #[test]
    fn a_tool_material_vetoes_the_contents_flag() {
        let materials = [material("tools/toolsfog"), material("tools/toolsinvisible")];
        let r = resolver(&materials);
        let glass = Decision::Force("minecraft:glass".into());
        for index in 0..materials.len() {
            assert_eq!(
                side_block(&glass, &r, Some(index), TextureFlags::empty(), true),
                None,
                "{} became glass",
                materials[index].name
            );
        }
    }

    /// The veto must not disarm the flag where it is right: a real window
    /// brush wears a glass material and has to stay glass.
    #[test]
    fn a_real_window_still_honours_the_contents_flag() {
        let materials = [material("glass/glasswindow002a")];
        let r = resolver(&materials);
        assert_eq!(
            side_block(
                &Decision::Force("minecraft:glass".into()),
                &r,
                Some(0),
                TextureFlags::empty(),
                true,
            ),
            Some("minecraft:glass")
        );
    }

    /// Translucent snow is flagged `WINDOW`, but a rule naming snow is a
    /// statement of intent and beats the flag's guess.
    #[test]
    fn a_named_rule_outranks_the_contents_flag() {
        let materials = [material("ground/snow01")];
        let r = resolver(&materials);
        assert!(r.named_by_rule(0));
        assert_eq!(
            side_block(
                &Decision::Force("minecraft:glass".into()),
                &r,
                Some(0),
                TextureFlags::empty(),
                true,
            ),
            Some("minecraft:snow_block")
        );
    }

    /// A colour match is only a guess, so it must not override the flag: a
    /// water brush wearing an unrecognised material is still water.
    #[test]
    fn a_colour_guess_does_not_outrank_the_contents_flag() {
        let materials = [material("custom/unknownsurface")];
        let r = resolver(&materials);
        assert!(!r.named_by_rule(0));
        assert_eq!(
            side_block(
                &Decision::Force("minecraft:water".into()),
                &r,
                Some(0),
                TextureFlags::empty(),
                true,
            ),
            Some("minecraft:water")
        );
    }

    #[test]
    fn faces_the_engine_never_draws_contribute_nothing() {
        let materials = [material("concrete/concretewall001a")];
        let r = resolver(&materials);
        for flag in [
            TextureFlags::NODRAW,
            TextureFlags::SKIP,
            TextureFlags::HINT,
            TextureFlags::TRIGGER,
            TextureFlags::SKY,
            TextureFlags::SKY2D,
        ] {
            assert_eq!(
                side_block(&Decision::ByMaterial, &r, Some(0), flag, true),
                None,
                "{flag:?} face was kept"
            );
        }
        assert!(
            side_block(&Decision::ByMaterial, &r, Some(0), TextureFlags::empty(), true).is_some()
        );
    }

    /// A side carrying no texture info at all still bounds its brush, so it
    /// must not punch a hole in it.
    #[test]
    fn a_side_without_a_material_uses_the_fallback() {
        let r = resolver(&[]);
        assert_eq!(
            side_block(&Decision::ByMaterial, &r, None, TextureFlags::empty(), true),
            Some("minecraft:stone")
        );
    }

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
        let result = convert(&map, &Config::default()).unwrap();

        assert!(result.stats.blocks > 10_000, "got {}", result.stats.blocks);
        assert!(result.stats.solids_voxelized > 0);
        assert!(result.palette.len() > 1, "palette should hold more than air");

        // Hollowing must remove a substantial share of a solid map.
        assert!(
            result.stats.blocks < result.stats.blocks_before_hollow,
            "hollowing removed nothing"
        );
    }

    /// No single block should dominate a converted map. Before materials
    /// landed, fog volumes flagged `WINDOW` made glass 86% of some Entropy:
    /// Zero maps; a regression there would show up here first.
    #[test]
    fn no_single_block_swamps_a_converted_map() {
        let Some(map) = sample_map() else { return };
        let result = convert(&map, &Config::default()).unwrap();
        let total = result.stats.blocks;
        let (block, count) = result
            .stats
            .block_counts
            .iter()
            .max_by_key(|(_, count)| **count)
            .unwrap();
        assert!(
            count * 2 < total,
            "{block} is {count} of {total} blocks"
        );
        assert!(
            result.stats.block_counts.len() > 8,
            "only {} block types",
            result.stats.block_counts.len()
        );
    }

    #[test]
    fn converted_geometry_lands_inside_the_map_bounds() {
        let Some(map) = sample_map() else { return };
        let result = convert(&map, &Config::default()).unwrap();
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
        let solid = convert(&map, &solid_config).unwrap();
        let hollow = convert(&map, &Config::default()).unwrap();

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

        let fine = convert(&map, &Config::default()).unwrap();
        let coarse = convert(&map, &coarse).unwrap();
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

        let without = convert(&map, &config).unwrap();
        let with = convert(&map, &Config::default()).unwrap();
        assert!(without.stats.blocks <= with.stats.blocks);
    }
}
