//! The conversion pipeline: BSP in, voxel grid out.

use crate::bsp::{Map, Solid};
use crate::config::{Config, FillMode};
use crate::geom::{Aabb, Vec3};
use crate::palette::{Decision, Resolver};
use crate::voxel::brush::BlockSolid;
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
    /// Brush entities pulled out into their own grids, for classnames
    /// configured as `separate`.
    pub separate: Vec<SeparateEntity>,
    /// Generated blocks carrying the map's own textures, when
    /// `[materials] mode = "kubejs"`.
    pub pack: crate::output::kubejs::Pack,
}

/// One brush entity converted on its own.
///
/// Doors, platforms and trains move, so their geometry belongs where the world
/// is not: pasted into the world it would seal the doorway it is supposed to
/// open. Kept apart, it is a schematic you can place wherever the mechanism you
/// build for it needs it.
#[derive(Debug, Clone)]
pub struct SeparateEntity {
    /// Index into the map's entity list.
    pub entity: usize,
    pub classname: String,
    pub targetname: Option<String>,
    /// The `*N` brush model the entity uses.
    pub model: usize,
    pub grid: VoxelGrid,
}

impl SeparateEntity {
    /// A filename stem unique within one map.
    pub fn name(&self) -> String {
        let label = self
            .targetname
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("unnamed");
        let sanitized: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        // The entity index keeps two doors with the same targetname apart.
        format!("{}_{}_{}", self.classname, sanitized, self.entity)
    }
}

#[derive(Debug, Default, Clone)]
pub struct Stats {
    pub solids_voxelized: usize,
    pub solids_skipped: usize,
    pub displacements_voxelized: usize,
    pub displacements_skipped: usize,
    /// Materials that resolved to a generated textured block.
    pub textures_resolved: usize,
    /// Voxels emitted as a slab or stair instead of a full cube.
    pub shapes_fitted: usize,
    pub blocks_before_hollow: usize,
    pub blocks: usize,
    /// Voxel count per block type, for sourcing materials.
    pub block_counts: BTreeMap<String, usize>,
}

/// Translate a brush into block space, mapping each plane through `transform`.
///
/// `origin` is the brush entity's own origin, which has to be added back
/// first. VBSP rewrites a brush entity's geometry to be relative to its
/// `origin` keyvalue, leaving the model's stored origin at zero, so the
/// coordinates in the plane lump are *not* world space. In
/// `d1_trainstation_02` that is 103 of 115 brush entity models: taken at face
/// value, every door, button, trigger and func_brush in the map piles up
/// around wherever Source's origin happens to land.
fn to_block_solid(solid: &Solid, transform: &Transform, origin: Vec3) -> BlockSolid {
    let planes = solid
        .sides
        .iter()
        .map(|side| {
            // Translating a plane moves its distance along its own normal.
            let plane = crate::geom::Plane::new(
                side.plane.normal,
                side.plane.dist + side.plane.normal.dot(origin),
            );
            transform.transform_plane(plane)
        })
        .collect();
    BlockSolid {
        planes,
        bounds: transform.transform_bounds(Aabb::new(
            solid.bounds.min + origin,
            solid.bounds.max + origin,
        )),
        side_of_plane: (0..solid.sides.len()).collect(),
    }
}

/// Which brush model each entity owns, and what to do with it.
struct EntityModel {
    entity: usize,
    classname: String,
    targetname: Option<String>,
    model: usize,
    /// The entity's `origin`, which its geometry is stored relative to.
    origin: Vec3,
    mode: crate::config::BrushEntityMode,
}

/// Match every brush entity to its model and the mode configured for its
/// classname.
fn entity_models(map: &Map, config: &Config) -> Vec<EntityModel> {
    let transform = Transform::new(config, map.bounds());
    crate::bsp::entities::extract(map, &transform)
        .into_iter()
        .filter_map(|record| {
            let model = record.brush_model?;
            // Worldspawn is model 0 and is never a brush entity.
            if model == 0 || model >= map.bsp.models.len() {
                return None;
            }
            let mode = config
                .entities
                .classname_modes
                .get(&record.classname)
                .copied()
                .unwrap_or(config.entities.brush_entities);
            let origin = record
                .origin_source
                .map(|[x, y, z]| Vec3::new(x, y, z))
                .unwrap_or(Vec3::ZERO);
            Some(EntityModel {
                entity: record.index,
                classname: record.classname,
                targetname: record.targetname,
                model,
                origin,
                mode,
            })
        })
        .collect()
}

/// Where each brush entity model's geometry has to be moved back to.
fn model_origins(entities: &[EntityModel]) -> std::collections::HashMap<usize, Vec3> {
    entities.iter().map(|e| (e.model, e.origin)).collect()
}

/// Which models go into the world grid: worldspawn, plus every brush entity
/// not being written separately or skipped.
fn models_to_convert(map: &Map, config: &Config, entities: &[EntityModel]) -> Vec<usize> {
    use crate::config::BrushEntityMode;
    let modes: std::collections::HashMap<usize, BrushEntityMode> =
        entities.iter().map(|e| (e.model, e.mode)).collect();

    (0..map.bsp.models.len())
        .filter(|model| {
            // Worldspawn is the world and is always converted. A model no
            // entity claims falls back to the global setting rather than being
            // dropped, so malformed entity data cannot silently lose geometry.
            *model == 0
                || modes.get(model).copied().unwrap_or(config.entities.brush_entities)
                    == BrushEntityMode::Include
        })
        .collect()
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

/// Voxelize one displacement into its own grid.
///
/// The surface is a shell one voxel thick, which you would fall straight
/// through and which looks like paper from below, so it is backed by
/// `solidify` more voxels driven into the solid side. That direction is the
/// face's own inward normal rather than simply down: displacements make walls
/// and ceilings as often as they make ground, and thickening a cliff downwards
/// would leave its face just as thin.
fn voxelize_displacement(
    surface: &crate::bsp::displacement::Surface,
    transform: &Transform,
    solidify: u32,
    block: BlockId,
) -> VoxelGrid {
    use crate::voxel::mesh::{Triangle, voxelize_triangle};

    let mut grid = VoxelGrid::new();
    let inward = transform.transform_direction(-surface.normal);

    for tri in &surface.triangles {
        let mapped = Triangle::new(
            transform.to_block_space(tri.a),
            transform.to_block_space(tri.b),
            transform.to_block_space(tri.c),
        );
        voxelize_triangle(&mapped, |pos| {
            grid.set(pos, block);
            for step in 1..=solidify {
                let offset = inward * step as f64;
                grid.set(
                    [
                        pos[0] + offset.x.round() as i32,
                        pos[1] + offset.y.round() as i32,
                        pos[2] + offset.z.round() as i32,
                    ],
                    block,
                );
            }
        });
    }
    grid
}

/// Voxelize a set of brushes into one grid, interning blocks into `palette`.
fn voxelize_solids(
    solids: &[Solid],
    map: &Map,
    config: &Config,
    resolver: &Resolver,
    transform: &Transform,
    origins: &std::collections::HashMap<usize, Vec3>,
    palette: &Mutex<Palette>,
    skipped: &std::sync::atomic::AtomicUsize,
) -> (VoxelGrid, crate::voxel::shapes::MaskGrid) {
    use crate::voxel::shapes::MaskGrid;
    let skip_sky = config.contents.skip_sky;
    let want_masks = config.shapes.enabled;

    solids
        .par_iter()
        .fold(|| (VoxelGrid::new(), MaskGrid::new()), |(mut grid, mut masks), solid| {
            let decision = resolver.decide(solid.flags);
            if decision == Decision::Skip {
                skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return (grid, masks);
            }

            let origin = origins.get(&solid.model).copied().unwrap_or(Vec3::ZERO);
            let block_solid = to_block_solid(solid, transform, origin);

            // Resolve the block for each side once, rather than per voxel.
            let side_blocks: Vec<Option<BlockId>> = solid
                .sides
                .iter()
                .map(|side| {
                    let material = side.texture_info.and_then(|i| map.material_index(i));
                    side_block(&decision, resolver, material, side.texture_flags, skip_sky)
                        .map(|name| palette.lock().unwrap().intern(name))
                })
                .collect();

            // A brush whose every side was vetoed contributes nothing.
            if side_blocks.iter().all(Option::is_none) {
                skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return (grid, masks);
            }
            let default_block = side_blocks.iter().flatten().copied().next();

            crate::voxel::brush::voxelize_with_shape(
                &block_solid,
                &config.output.voxelize,
                |pos, side, mask| {
                    let block = side
                        .and_then(|s| side_blocks.get(s).copied().flatten())
                        .or(default_block);
                    if let Some(block) = block {
                        grid.set(pos, block);
                        if want_masks {
                            masks.add(pos, mask);
                        }
                    }
                },
            );
            (grid, masks)
        })
        .reduce(
            || (VoxelGrid::new(), MaskGrid::new()),
            |(mut ga, mut ma), (gb, mb)| {
                ga.merge(gb);
                ma.merge(mb);
                (ga, ma)
            },
        )
}

/// Replace full cubes with slabs and stairs wherever the octant mask says the
/// geometry was really half-height or stepped.
///
/// Only blocks that have vanilla slab and stair variants can change; anything
/// else keeps its full cube. A generated textured block has no variants unless
/// the pack was told to register them, so this quietly does nothing there
/// rather than naming a block that does not exist.
fn fit_shapes(
    grid: &VoxelGrid,
    masks: &crate::voxel::shapes::MaskGrid,
    palette: &mut Palette,
    pack: &crate::output::kubejs::Pack,
) -> (VoxelGrid, usize) {
    use crate::voxel::shapes::{Shape, shape_for};

    // Resolve every (block, shape) pair once. A palette holds a few hundred
    // entries against millions of voxels, so doing this per voxel would spend
    // the whole pass formatting strings.
    let shapes: Vec<Shape> = (0..=u8::MAX).map(shape_for).collect();
    let mut resolved: std::collections::HashMap<(BlockId, Shape), Option<BlockId>> =
        std::collections::HashMap::new();

    let mut out = VoxelGrid::new();
    let mut fitted = 0;

    for (pos, id) in grid.iter() {
        let shape = shapes[masks.get(pos) as usize];
        if shape == Shape::Full {
            out.set(pos, id);
            continue;
        }

        let block = match resolved.get(&(id, shape)) {
            Some(cached) => *cached,
            None => {
                let name = palette.name(id).to_string();
                let block = crate::palette::blocks::shaped(&name, shape.variant())
                    .map(str::to_string)
                    .or_else(|| pack.shaped(&name, shape.variant()))
                    .zip(shape.state())
                    .map(|(base, state)| palette.intern(&format!("{base}{state}")));
                resolved.insert((id, shape), block);
                block
            }
        };

        match block {
            Some(block) => {
                out.set(pos, block);
                fitted += 1;
            }
            None => out.set(pos, id),
        }
    }
    (out, fitted)
}

pub fn convert(map: &Map, config: &Config) -> anyhow::Result<Conversion> {
    let transform = Transform::new(config, map.bounds());

    // Extracting the map's real textures is opt-in; without it the palette
    // works exactly as before, from rules and average colour.
    let (pack, extracted) = match config.materials.mode {
        crate::config::MaterialMode::Kubejs => crate::source::extract::extract(map, config),
        crate::config::MaterialMode::Vanilla => Default::default(),
    };
    let resolver = Resolver::with_textures(config, map.materials(), &pack.ids())?;

    let entity_models = entity_models(map, config);

    // Gather every brush first so the voxelization itself parallelizes cleanly.
    let solids: Vec<Solid> = models_to_convert(map, config, &entity_models)
        .into_iter()
        .flat_map(|model| map.solids(model))
        .collect();

    // The palette is shared and rarely written to after the first few brushes.
    let palette = Mutex::new(Palette::new());
    let skipped = std::sync::atomic::AtomicUsize::new(0);

    let origins = model_origins(&entity_models);
    let (grid, masks) = voxelize_solids(
        &solids, map, config, &resolver, &transform, &origins, &palette, &skipped,
    );

    // Brush entities configured as `separate` get their own grid each, so a
    // door is a schematic you can place where your mechanism needs it rather
    // than a slab sealing the doorway it should open.
    let separate: Vec<SeparateEntity> = entity_models
        .iter()
        .filter(|e| e.mode == crate::config::BrushEntityMode::Separate)
        .filter_map(|entity| {
            let solids = map.solids(entity.model);
            if solids.is_empty() {
                return None;
            }
            let (grid, _) = voxelize_solids(
                &solids, map, config, &resolver, &transform, &origins, &palette, &skipped,
            );
            (grid.count() > 0).then(|| SeparateEntity {
                entity: entity.entity,
                classname: entity.classname.clone(),
                targetname: entity.targetname.clone(),
                model: entity.model,
                grid,
            })
        })
        .collect();

    // Displacements: Source's terrain, and the reason a converted outdoor map
    // used to be a floating shell of buildings over nothing.
    let displacements_skipped = std::sync::atomic::AtomicUsize::new(0);
    let surfaces = if config.displacement.enabled {
        map.displacement_surfaces()
    } else {
        Vec::new()
    };
    let mut grid = grid;
    if !surfaces.is_empty() {
        let terrain = surfaces
            .par_iter()
            .fold(VoxelGrid::new, |mut grid, surface| {
                let block = surface
                    .material
                    .and_then(|m| resolver.block_for_material(m))
                    .map(|name| palette.lock().unwrap().intern(name));
                match block {
                    Some(block) => grid.merge(voxelize_displacement(
                        surface,
                        &transform,
                        config.displacement.solidify,
                        block,
                    )),
                    None => {
                        displacements_skipped
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                grid
            })
            .reduce(VoxelGrid::new, |mut a, b| {
                a.merge(b);
                a
            });
        grid.merge(terrain);
    }
    let displacements_skipped = displacements_skipped.into_inner();

    let blocks_before_hollow = grid.count();
    let mut shapes_fitted = 0;
    let grid = match config.fill.mode {
        FillMode::Solid => grid,
        FillMode::Hollow => shell::hollow(
            &grid,
            config.fill.shell_thickness,
            config.fill.shell_neighborhood,
        ),
    };

    // Shapes are fitted last, after hollowing: the mask grid outlives the
    // brushes precisely so this can happen here, on the voxels that survived.
    let mut palette = palette.into_inner().unwrap();
    let grid = if config.shapes.enabled && !masks.is_empty() {
        let (fitted, count) = fit_shapes(&grid, &masks, &mut palette, &pack);
        shapes_fitted = count;
        fitted
    } else {
        grid
    };

    let palette = palette;
    let mut block_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, id) in grid.iter() {
        *block_counts.entry(palette.name(id).to_string()).or_default() += 1;
    }

    let skipped = skipped.into_inner();
    Ok(Conversion {
        stats: Stats {
            solids_voxelized: solids.len() - skipped,
            solids_skipped: skipped,
            displacements_voxelized: surfaces.len() - displacements_skipped,
            displacements_skipped,
            textures_resolved: extracted.resolved,
            shapes_fitted,
            blocks_before_hollow,
            blocks: grid.count(),
            block_counts,
        },
        grid,
        palette,
        transform,
        separate,
        pack,
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

    /// VBSP stores a brush entity's geometry relative to its `origin`, so the
    /// coordinates in the plane lump are not world space. Every converted
    /// brush entity has to end up near the entity that owns it, not piled up
    /// around wherever Source's origin lands.
    #[test]
    fn brush_entities_land_where_their_entity_is() {
        let Some(map) = sample_map() else { return };
        let config = Config::default();
        let transform = Transform::new(&config, map.bounds());
        let resolver = Resolver::new(&config, map.materials()).unwrap();
        let origins = model_origins(&entity_models(&map, &config));

        let _ = &resolver;
        let mut checked = 0;
        for (model, origin) in &origins {
            if origin.x == 0.0 && origin.y == 0.0 && origin.z == 0.0 {
                continue;
            }
            let Some(solid) = map.solids(*model).into_iter().next() else { continue };
            let placed = to_block_solid(&solid, &transform, *origin).bounds;
            let entity = transform.to_block_space(*origin);

            // An entity's origin need not be inside its own brush — a door's
            // is at its hinge — so the test is proximity, not containment.
            // Half the brush's own extent plus a few blocks is generous, and
            // still nothing like the hundreds of blocks the untranslated
            // geometry is out by.
            let slack = placed.size().length() + 8.0;
            let distance = (placed.center() - entity).length();
            assert!(
                distance <= slack,
                "model {model} is {distance:.0} blocks from its entity, allowing {slack:.0}"
            );
            checked += 1;
        }
        assert!(checked > 0, "no origin-relative brush entities to check");
    }

    /// The same in the other direction, so the test above cannot quietly pass
    /// on a no-op: dropping the origin must put the geometry far from its
    /// entity, which is the bug this fixes.
    #[test]
    fn ignoring_the_entity_origin_misplaces_the_geometry() {
        let Some(map) = sample_map() else { return };
        let config = Config::default();
        let transform = Transform::new(&config, map.bounds());
        let origins = model_origins(&entity_models(&map, &config));

        let (mut with_total, mut without_total, mut count) = (0.0, 0.0, 0);
        for (model, origin) in &origins {
            if origin.x == 0.0 && origin.y == 0.0 && origin.z == 0.0 {
                continue;
            }
            let Some(solid) = map.solids(*model).into_iter().next() else { continue };
            let entity = transform.to_block_space(*origin);
            with_total += (to_block_solid(&solid, &transform, *origin).bounds.center() - entity)
                .length();
            without_total += (to_block_solid(&solid, &transform, Vec3::ZERO).bounds.center()
                - entity)
                .length();
            count += 1;
        }

        assert!(count > 0);
        let (with, without) = (with_total / count as f64, without_total / count as f64);
        assert!(
            without > with * 10.0,
            "untranslated geometry averages {without:.0} blocks from its entity \
             and translated {with:.0}; the translation is doing nothing"
        );
    }

    /// The failure this guards against is silent: a schematic naming a block
    /// its pack does not register pastes as a hole in the world, with no
    /// error anywhere. Every generated id in the palette must be registered.
    #[test]
    fn every_generated_block_in_the_palette_is_registered_by_the_pack() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.materials.mode = crate::config::MaterialMode::Kubejs;

        let result = convert(&map, &config).unwrap();
        if result.pack.is_empty() {
            return; // no game install to read textures from
        }

        let script = result.pack.script();
        let mut generated = 0;
        for id in 0..result.palette.len() {
            let name = result.palette.name(id as crate::voxel::grid::BlockId);
            let Some(_) = name.strip_prefix("kubejs:") else { continue };
            generated += 1;
            assert!(
                script.contains(&format!("event.create('{name}')")),
                "{name} is in the palette but not registered"
            );
        }
        assert!(generated > 0, "kubejs mode produced no generated blocks");
    }

    /// Textures must not displace the rules that carry meaning: a grate has
    /// to stay see-through even though it has a perfectly good texture.
    #[test]
    fn named_rules_still_win_over_generated_textures() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.materials.mode = crate::config::MaterialMode::Kubejs;

        let result = convert(&map, &config).unwrap();
        if result.pack.is_empty() {
            return;
        }
        let resolver = Resolver::with_textures(&config, map.materials(), &result.pack.ids())
            .unwrap();

        for (index, material) in map.materials().iter().enumerate() {
            if material.name.contains("grate") || material.name.starts_with("glass/") {
                let block = resolver.block_for_material(index);
                assert!(
                    block.is_some_and(|b| b.starts_with("minecraft:")),
                    "{} became {block:?} instead of keeping its vanilla block",
                    material.name
                );
            }
        }
    }

    /// Vanilla mode must be untouched by any of this.
    #[test]
    fn vanilla_mode_generates_nothing() {
        let Some(map) = sample_map() else { return };
        let result = convert(&map, &Config::default()).unwrap();
        assert!(result.pack.is_empty());
        for id in 0..result.palette.len() {
            let name = result.palette.name(id as crate::voxel::grid::BlockId);
            assert!(name.starts_with("minecraft:"), "vanilla mode emitted {name}");
        }
    }

    /// A map with terrain in it, since `az_c4_4` is nearly all interiors.
    fn terrain_map() -> Option<Map> {
        let path = Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_canals_01a.bsp",
        );
        path.exists().then(|| Map::load(path).unwrap())
    }

    #[test]
    fn displacements_add_terrain_and_can_be_turned_off() {
        let Some(map) = terrain_map() else { return };
        assert!(!map.bsp.displacements.is_empty());

        let mut without = Config::default();
        without.displacement.enabled = false;
        let without = convert(&map, &without).unwrap();
        let with = convert(&map, &Config::default()).unwrap();

        assert_eq!(without.stats.displacements_voxelized, 0);
        assert!(with.stats.displacements_voxelized > 0);
        assert!(
            with.stats.blocks > without.stats.blocks,
            "terrain added nothing: {} vs {}",
            with.stats.blocks,
            without.stats.blocks
        );
    }

    /// Terrain must land inside the map, not somewhere off in space, and it
    /// must be thicker than the single-voxel shell the triangles alone give.
    #[test]
    fn terrain_is_solid_and_inside_the_map() {
        let Some(map) = terrain_map() else { return };
        let result = convert(&map, &Config::default()).unwrap();
        let bounds = block_bounds(&map, &result.transform);
        let (min, max) = result.grid.bounds().unwrap();

        for axis in 0..3 {
            assert!(
                min[axis] as f64 >= bounds.min.axis(axis) - 4.0,
                "geometry at {min:?} escapes {bounds:?}"
            );
            assert!(
                max[axis] as f64 <= bounds.max.axis(axis) + 4.0,
                "geometry at {max:?} escapes {bounds:?}"
            );
        }

        let mut thin = Config::default();
        thin.displacement.solidify = 0;
        let thin = convert(&map, &thin).unwrap();
        assert!(
            result.stats.blocks_before_hollow > thin.stats.blocks_before_hollow,
            "solidify added no backing"
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

