//! The conversion pipeline: BSP in, voxel grid out.

use crate::bsp::{Map, Solid};
use crate::config::{Config, FillMode};
use crate::geom::{Aabb, Vec3};
use crate::palette::{Decision, Resolver};
use crate::voxel::brush::BlockSolid;
use crate::voxel::grid::{AIR, BlockId, IVec3, Palette, VoxelGrid};
use crate::voxel::shell;
use crate::voxel::transform::Transform;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// What a prop drawn as its own mesh leaves behind to stand on: solid, and
/// invisible, so the mesh in front of it is all you see.
const BARRIER: &str = "minecraft:barrier";

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
    /// The same textures as a bundle for the companion mod, when
    /// `[materials] mode = "bundle"`. One entry per material.
    pub bundle: crate::output::bundle::Bundle,
    /// Props drawn as their real mesh, as the display entities that place
    /// them. Empty unless `[props] models` is on.
    pub props: Vec<crate::output::display::Placement>,
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
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
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
    /// Tiles per axis the block budget allowed.
    pub tile_cap: u32,
    /// Static props voxelized into the world.
    pub props_placed: usize,
    /// Props skipped: model missing, too small, or matched by a skip rule.
    pub props_skipped: usize,
    /// Props drawn as their real mesh instead of being voxelized.
    pub props_modelled: usize,
    /// Distinct meshes generated for them.
    pub prop_models: usize,
    /// Props moved vertically to stand on the floor rather than in it.
    pub props_settled: usize,
    /// Props drawn as a block with their rotation baked in, which the chunk
    /// mesh absorbs, rather than as an entity redrawn every frame.
    pub props_baked: usize,
    /// Blocks those props needed. More than `props_baked` when a prop reached
    /// too far to be drawn from one block and had to be split across several.
    pub prop_blocks: usize,
    /// Invisible barrier cubes placed to make the big ones solid.
    pub prop_barriers: usize,
    /// Cells given a generated block shaped like the mesh passing through them,
    /// instead of a barrier cube.
    pub prop_collision_blocks: usize,
    /// Distinct shapes those cells needed, which is what the pack registers.
    pub prop_collision_shapes: usize,
    /// Sixteenths of a block the shapes were rounded to. 1 unless there were
    /// more distinct shapes than `collision_max_shapes` allowed, in which case
    /// they were rounded outward more coarsely until they fit.
    pub prop_collision_step: i64,
    /// Voxels emitted as a slab or stair instead of a full cube.
    pub shapes_fitted: usize,
    /// Brush and terrain blocks before hollowing removed the interiors.
    ///
    /// The world only: props are added after hollowing, deliberately, so they
    /// cannot seal a wall's air side and have that wall taken for interior.
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
fn entity_models(map: &Map, config: &Config, transform: &Transform) -> Vec<EntityModel> {
    crate::bsp::entities::extract(map, transform)
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
                || modes
                    .get(model)
                    .copied()
                    .unwrap_or(config.entities.brush_entities)
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
    tiles: Option<&TileSet>,
) -> VoxelGrid {
    use crate::voxel::mesh::{Triangle, voxelize_triangle};

    let mut grid = VoxelGrid::new();
    let inward = transform.transform_direction(-surface.normal);
    // Terrain is world geometry like any other, so a split texture is chosen
    // by the same world projection the brushes use.
    let uv = tiles.zip(surface.texcoord).map(|(set, tex)| {
        let uv = Uv::new(
            tex,
            transform,
            Vec3::ZERO,
            set.texels_per_tile,
            transform.units_per_block(),
        );
        (uv, set)
    });

    for tri in &surface.triangles {
        let mapped = Triangle::new(
            transform.to_block_space(tri.a),
            transform.to_block_space(tri.b),
            transform.to_block_space(tri.c),
        );
        voxelize_triangle(&mapped, |pos| {
            let block = match uv {
                Some((uv, set)) => {
                    let centre = Vec3::new(
                        pos[0] as f64 + 0.5,
                        pos[1] as f64 + 0.5,
                        pos[2] as f64 + 0.5,
                    );
                    let (s, t) = uv.at(centre);
                    set.at(s, t)
                }
                None => block,
            };
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

/// Voxelize one static prop's triangles.
///
/// A model is a surface, not a solid, so this is the displacement treatment:
/// rasterize the triangles and, if asked, drive `solidify` more voxels along
/// each one's inward normal. Props are usually closed shells already — a crate
/// really is a box — so the default is none, and a fence stays one block
/// thick instead of becoming a wall.
fn voxelize_prop(
    surface: &crate::source::extract::PropSurface,
    transform: &Transform,
    solidify: u32,
    block: BlockId,
    tiles: Option<&TileSet>,
) -> VoxelGrid {
    use crate::voxel::mesh::{Triangle, voxelize_triangle};

    let mut grid = VoxelGrid::new();
    for (index, tri) in surface.triangles.iter().enumerate() {
        let mapped = Triangle::new(
            transform.to_block_space(tri[0]),
            transform.to_block_space(tri[1]),
            transform.to_block_space(tri[2]),
        );
        if mapped.is_degenerate() {
            continue;
        }
        let inward = -mapped.normal();
        // A model's UVs are an unwrap of the whole sheet, so the tile comes
        // off the triangle rather than from a world projection — but
        // interpolated across it, not taken once for the whole triangle. A
        // model's triangles are not block-sized: `rockcliff02a` is a handful
        // of huge ones, and one tile each paints the cliff in patches.
        let corners = tiles.and(surface.uvs.get(index));
        voxelize_triangle(&mapped, |pos| {
            let block = match (tiles, corners) {
                (Some(set), Some(corners)) => {
                    let centre = Vec3::new(
                        pos[0] as f64 + 0.5,
                        pos[1] as f64 + 0.5,
                        pos[2] as f64 + 0.5,
                    );
                    set.at_uv(interpolate(&mapped, corners, centre))
                }
                _ => block,
            };
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

/// A material's texture split across several blocks, and the blocks it was
/// split into.
///
/// The point of the whole exercise: a Source wall texture covers metres of
/// surface, so squeezing it onto one block face throws away everything that
/// made it read as brick or panelling. With the pieces registered separately,
/// each voxel can take the one that is really in front of it.
struct TileSet {
    grid: [u32; 2],
    /// How many texels wide and tall one tile is.
    texels_per_tile: [f64; 2],
    /// Blocks in row-major order, as the pack registered them.
    ids: Vec<BlockId>,
}

impl TileSet {
    /// The tile at a position already measured in tiles. See [`Uv`].
    fn at(&self, column: f64, row: f64) -> BlockId {
        let index = |value: f64, axis: usize| {
            // A texture repeats across a wall, so a coordinate off the end of
            // it wraps rather than clamping.
            (value.floor() as i64).rem_euclid(self.grid[axis] as i64) as usize
        };
        let (column, row) = (index(column, 0), index(row, 1));
        self.ids[row * self.grid[0] as usize + column]
    }

    /// The tile at a normalized texture coordinate, as a model stores it.
    fn at_uv(&self, uv: [f64; 2]) -> BlockId {
        self.at(uv[0] * self.grid[0] as f64, uv[1] * self.grid[1] as f64)
    }
}

/// Tile coordinates as a function of block-space position: how many tiles
/// along the surface's own texture axes a point lies.
///
/// Both the texture projection and the block transform are affine, so their
/// composition is too and can be reduced to two dot products — which matters,
/// because this is evaluated once per voxel.
///
/// The division by tile size happens here rather than in [`TileSet`] because
/// **the tile size is a property of the face, not of the material**. One
/// material is used at several scales in the same map: `nature/cliffface001a`
/// appears in `d2_coast_07` at six different rates, from a third of a texel
/// per unit to two. A tile size taken from the material's typical scale is
/// then too wide for every face using a larger one, and the wall comes out in
/// 2x2 blocks of the same picture.
#[derive(Clone, Copy)]
struct Uv {
    s: (Vec3, f64),
    t: (Vec3, f64),
}

impl Uv {
    /// `origin` is the brush entity's own origin: its geometry is stored
    /// relative to it and so are its texture vectors, so it has to come back
    /// off before the projection is applied.
    ///
    /// `texels_per_tile` is what the material's tiles were actually cut at.
    /// A face wanting *fewer* texels per block than that would repeat a tile
    /// across several blocks, so on those faces the tile is resized to the
    /// block instead; the texture then covers `grid` blocks rather than its
    /// true span, which is the compromise a single shared set of tiles forces.
    /// Faces wanting more are left exact — they advance by more than one tile
    /// per block, which shows no repeat and so needs no correction.
    fn new(
        tex: crate::bsp::texcoord::TexCoord,
        transform: &Transform,
        origin: Vec3,
        texels_per_tile: [f64; 2],
        units_per_block: f64,
    ) -> Uv {
        let source = |p: Vec3| transform.to_source_space(p) - origin;
        let at = Vec3::ZERO;
        let (s0, t0) = (tex.s(source(at)), tex.t(source(at)));
        let axis = |i: usize| {
            let mut e = Vec3::ZERO;
            match i {
                0 => e.x = 1.0,
                1 => e.y = 1.0,
                _ => e.z = 1.0,
            }
            (tex.s(source(e)) - s0, tex.t(source(e)) - t0)
        };
        let (sx, tx) = axis(0);
        let (sy, ty) = axis(1);
        let (sz, tz) = axis(2);

        // How many texels of this face's own projection one block covers. A
        // degenerate texture vector leaves the material's own size in place
        // rather than dividing by zero.
        let per_block = tex.texels_per_unit();
        let divisor: [f64; 2] = std::array::from_fn(|axis| {
            let face = per_block[axis] * units_per_block;
            let tile = texels_per_tile[axis];
            if face > 0.0 && face.is_finite() {
                face.min(tile)
            } else {
                tile
            }
            .max(f64::MIN_POSITIVE)
        });

        Uv {
            s: (Vec3::new(sx, sy, sz) / divisor[0], s0 / divisor[0]),
            t: (Vec3::new(tx, ty, tz) / divisor[1], t0 / divisor[1]),
        }
    }

    fn at(&self, p: Vec3) -> (f64, f64) {
        (self.s.0.dot(p) + self.s.1, self.t.0.dot(p) + self.t.1)
    }
}

/// Work out, once, which materials have a split texture and what blocks it was
/// split into.
///
/// Only materials the palette actually resolves to their generated block are
/// included: a rule naming `iron_bars` for a grate outranks the texture, and
/// tiling a block the map will never place would be nonsense.
fn tile_sets(
    map: &Map,
    materials: &[crate::bsp::Material],
    resolver: &Resolver,
    pack: &crate::output::kubejs::Pack,
    palette: &Mutex<Palette>,
) -> Vec<Option<TileSet>> {
    if pack.tilings().is_empty() {
        return Vec::new();
    }
    let _ = map;

    materials
        .iter()
        .enumerate()
        .map(|(index, material)| {
            // Read back exactly how the texture was cut rather than working
            // it out again. Recomputing it is how the cut and the projection
            // drifted apart twice, and with the cap now chosen from a budget
            // the config alone no longer determines it.
            let split = pack.split(&material.name)?;
            let (grid, texels_per_tile) = (split.grid, split.texels_per_tile);

            // The resolver has the last word: a named rule beats the texture.
            let assigned = resolver.block_for_material(index)?;
            if assigned != pack.tile_id(&material.name, 0, 0)? {
                return None;
            }

            let mut ids = Vec::with_capacity((grid[0] * grid[1]) as usize);
            for row in 0..grid[1] {
                for column in 0..grid[0] {
                    let id = pack.tile_id(&material.name, column, row)?;
                    ids.push(palette.lock().unwrap().intern(&id));
                }
            }
            Some(TileSet {
                grid,
                texels_per_tile,
                ids,
            })
        })
        .collect()
}

/// Interpolate a per-corner value to where `p` falls on the triangle.
///
/// Barycentric coordinates, which also project `p` onto the triangle's plane,
/// so a voxel centre slightly off the surface still lands somewhere sensible.
/// A degenerate triangle falls back to the first corner rather than dividing
/// by zero.
fn interpolate(tri: &crate::voxel::mesh::Triangle, corners: &[[f64; 2]; 3], p: Vec3) -> [f64; 2] {
    let (v0, v1, v2) = (tri.b - tri.a, tri.c - tri.a, p - tri.a);
    let (d00, d01, d11) = (v0.dot(v0), v0.dot(v1), v1.dot(v1));
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() < 1e-12 {
        return corners[0];
    }
    let (d20, d21) = (v2.dot(v0), v2.dot(v1));
    let b = (d11 * d20 - d01 * d21) / denom;
    let c = (d00 * d21 - d01 * d20) / denom;
    let a = 1.0 - b - c;
    std::array::from_fn(|axis| a * corners[0][axis] + b * corners[1][axis] + c * corners[2][axis])
}

/// Whether props should be solid as shaped blocks rather than barrier cubes.
///
/// A shape has to be registered somewhere, so this needs the generated pack:
/// vanilla output has no pack and falls back to barriers, which is also what
/// `collision = "barrier"` asks for outright.
fn shaped_collision_wanted(config: &Config) -> bool {
    config.props.collision == crate::config::CollisionMode::Shaped
        && config.materials.mode == crate::config::MaterialMode::Kubejs
}

/// Whether props should be solid as barrier cubes.
fn barriers_wanted(config: &Config) -> bool {
    match config.props.collision {
        crate::config::CollisionMode::None => false,
        crate::config::CollisionMode::Barrier => true,
        // Shaped collision falls back to barriers without a pack to register
        // shapes in. With one, the only barriers left are for the props that
        // stayed display entities, and those are placed separately.
        crate::config::CollisionMode::Shaped => !shaped_collision_wanted(config),
    }
}

/// Voxelize a set of brushes into one grid, interning blocks into `palette`.
// Every argument here is one of the conversion's inputs; bundling them into a
// struct would only move the same list somewhere else.
#[allow(clippy::too_many_arguments)]
fn voxelize_solids(
    solids: &[Solid],
    map: &Map,
    config: &Config,
    resolver: &Resolver,
    transform: &Transform,
    origins: &std::collections::HashMap<usize, Vec3>,
    tiles: &[Option<TileSet>],
    palette: &Mutex<Palette>,
    skipped: &std::sync::atomic::AtomicUsize,
) -> (VoxelGrid, crate::voxel::shapes::MaskGrid) {
    use crate::voxel::shapes::MaskGrid;
    let skip_sky = config.contents.skip_sky;
    let want_masks = config.shapes.enabled;

    solids
        .par_iter()
        .fold(
            || (VoxelGrid::new(), MaskGrid::new()),
            |(mut grid, mut masks), solid| {
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
                // The side a voxel falls back to when its own nearest face is one
                // the engine never draws. Half a brush's sides are nodraw, so this
                // is not a rare case: it decides the block for a large share of
                // the voxels in the map, and taking the tiled path here too is
                // what keeps a wall from being half one repeated tile.
                let default_side = side_blocks.iter().position(Option::is_some);

                // Where a face's material was split across several blocks, the
                // projection that says which piece belongs at each voxel.
                let side_tiles: Vec<Option<(Uv, &TileSet)>> = solid
                    .sides
                    .iter()
                    .map(|side| {
                        let info_index = side.texture_info?;
                        let set = tiles.get(map.material_index(info_index)?)?.as_ref()?;
                        let info = map.bsp.textures_info.get(info_index)?;
                        let tex = crate::bsp::texcoord::TexCoord::of(info);
                        let uv = Uv::new(
                            tex,
                            transform,
                            origin,
                            set.texels_per_tile,
                            transform.units_per_block(),
                        );
                        Some((uv, set))
                    })
                    .collect();

                let block_of = |side: usize, pos: IVec3| -> Option<BlockId> {
                    let base = side_blocks.get(side).copied().flatten()?;
                    let Some((uv, set)) = side_tiles.get(side).and_then(Option::as_ref) else {
                        return Some(base);
                    };
                    let centre = Vec3::new(
                        pos[0] as f64 + 0.5,
                        pos[1] as f64 + 0.5,
                        pos[2] as f64 + 0.5,
                    );
                    let (s, t) = uv.at(centre);
                    Some(set.at(s, t))
                };

                crate::voxel::brush::voxelize_with_shape(
                    &block_solid,
                    &config.output.voxelize,
                    |pos, side, mask| {
                        let block = side
                            .and_then(|s| block_of(s, pos))
                            .or_else(|| default_side.and_then(|s| block_of(s, pos)));
                        if let Some(block) = block {
                            grid.set(pos, block);
                            if want_masks {
                                masks.add(pos, mask);
                            }
                        }
                    },
                );
                (grid, masks)
            },
        )
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
    // Bounds of what will be kept, which is not the same as worldspawn's own
    // box once the 3D skybox room is left out of it.
    let skybox = map.skybox().filter(|_| config.contents.skip_3d_skybox);
    let transform = Transform::new(config, map.converted_bounds(config.contents.skip_3d_skybox));

    // Reading the game's own content is best-effort: without it there are no
    // generated textures and no props, and the palette works exactly as it
    // did before, from rules and the compiler's average colour.
    let assets = crate::source::extract::extract(map, config);
    let materials = assets.materials(map);
    // A material's block comes from whichever of the two outputs is in use;
    // only one of them is ever populated, so concatenating them is safe and
    // keeps the resolver from having to know which mode this is.
    let mut texture_ids = assets.pack.ids();
    texture_ids.extend(assets.bundle.ids());
    let resolver = Resolver::with_textures(config, &materials, &texture_ids)?;

    // A material that could not be given a pool index would be painted with
    // some other material's texture, so this stops the run rather than
    // producing a map that looks subtly wrong everywhere.
    if let Some(first) = assets.bundle_errors.first() {
        anyhow::bail!(
            "{first}{}",
            match assets.bundle_errors.len() {
                1 => String::new(),
                n => format!(" ({} more materials did not fit)", n - 1),
            }
        );
    }

    let entity_models = entity_models(map, config, &transform);

    // Gather every brush first so the voxelization itself parallelizes cleanly.
    let solids: Vec<Solid> = models_to_convert(map, config, &entity_models)
        .into_iter()
        .flat_map(|model| map.solids(model))
        .filter(|solid| !skybox.is_some_and(|room| room.contains(&solid.bounds)))
        .collect();

    // The palette is shared and rarely written to after the first few brushes.
    let palette = Mutex::new(Palette::new());
    let skipped = std::sync::atomic::AtomicUsize::new(0);

    let origins = model_origins(&entity_models);
    let tiles = tile_sets(map, &materials, &resolver, &assets.pack, &palette);
    let (grid, masks) = voxelize_solids(
        &solids, map, config, &resolver, &transform, &origins, &tiles, &palette, &skipped,
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
                &solids, map, config, &resolver, &transform, &origins, &tiles, &palette, &skipped,
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
                        surface
                            .material
                            .and_then(|m| tiles.get(m))
                            .and_then(Option::as_ref),
                    )),
                    None => {
                        displacements_skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

    // How far each modelled prop has to move to meet the floor. Measured here,
    // against the world as brushes and terrain left it and before any prop has
    // been added to it, so props cannot end up standing on each other.
    let settle: Vec<f64> = assets
        .placements
        .iter()
        .map(|placement| {
            if !config.props.settle {
                return 0.0;
            }
            crate::voxel::settle::offset(
                &grid,
                transform.transform_bounds(placement.bounds),
                config.props.settle_max,
            )
        })
        .collect();
    let settled = settle.iter().filter(|shift| **shift != 0.0).count();

    // What the props are solid as, kept apart from the world's own blocks
    // until the very end. Merging it in now would let hollowing see a wall
    // whose air side is sealed by a collision block as interior and carve it
    // away.
    //
    // Shaped collision measures the mesh per cell instead of filling the cell:
    // where the barrier shell put a metre cube, this puts a box the size of
    // what actually passes through. Which cells is the same question either
    // way, and the same exact test answers it.
    let shaped_collision: Vec<(usize, std::collections::HashMap<IVec3, Aabb>)> =
        if shaped_collision_wanted(config) {
            assets
                .placements
                .par_iter()
                .enumerate()
                .zip(&settle)
                .filter(|((_, placement), _)| !placement.collision.is_empty())
                .map(|((index, placement), shift)| {
                    let lift = Vec3::new(0.0, 0.0, shift * transform.units_per_block());
                    let triangles: Vec<[Vec3; 3]> = placement
                        .collision
                        .iter()
                        .map(|tri| tri.map(|v| transform.to_block_space(v + lift)))
                        .collect();
                    (index, crate::output::collision::cells(triangles.iter()))
                })
                .collect()
        } else {
            Vec::new()
        };

    let collision = assets
        .placements
        .par_iter()
        .zip(&settle)
        .filter(|_| barriers_wanted(config))
        .filter(|(placement, _)| !placement.collision.is_empty())
        .fold(VoxelGrid::new, |mut grid, (placement, shift)| {
            let barrier = palette.lock().unwrap().intern(BARRIER);
            // The mesh moved, so what you can stand on moves with it.
            let lift = Vec3::new(0.0, 0.0, shift * transform.units_per_block());
            let surface = crate::source::extract::PropSurface {
                triangles: placement
                    .collision
                    .iter()
                    .map(|tri| tri.map(|v| v + lift))
                    .collect(),
                uvs: Vec::new(),
                material: 0,
            };
            grid.merge(voxelize_prop(
                &surface,
                &transform,
                config.props.solidify,
                barrier,
                None,
            ));
            grid
        })
        .reduce(VoxelGrid::new, |mut a, b| {
            a.merge(b);
            a
        });

    // Static props: everything a map puts *in* its rooms. Fences, railings,
    // catwalks, crates, signs and lamps are all models, none of which is in
    // any brush lump, which is why a map converted from brushes alone is an
    // accurate but empty shell.
    let voxelized = if assets.props.is_empty() {
        VoxelGrid::new()
    } else {
        assets
            .props
            .par_iter()
            .fold(VoxelGrid::new, |mut grid, surface| {
                let block = resolver
                    .block_for_material(surface.material)
                    .map(|name| palette.lock().unwrap().intern(name));
                if let Some(block) = block {
                    grid.merge(voxelize_prop(
                        surface,
                        &transform,
                        config.props.solidify,
                        block,
                        tiles.get(surface.material).and_then(Option::as_ref),
                    ));
                }
                grid
            })
            .reduce(VoxelGrid::new, |mut a, b| {
                a.merge(b);
                a
            })
    };

    // Hollowing removes what nothing can see, and it decides that by asking
    // whether a block touches air. Props are put in *after* it, not before,
    // because a prop standing against a wall seals that wall's air side and
    // hollowing then takes the wall for interior and carves it out — which is
    // how a crate in a corridor turns into a hole through the corridor.
    // Props have no interior of their own to lose: they are already surfaces.
    let blocks_before_hollow = grid.count();
    let mut shapes_fitted = 0;
    let mut grid = match config.fill.mode {
        FillMode::Solid => grid,
        FillMode::Hollow => shell::hollow(
            &grid,
            config.fill.shell_thickness,
            config.fill.shell_neighborhood,
        ),
    };
    grid.merge(voxelized);
    let grid = grid;

    // Shapes are fitted last, after hollowing: the mask grid outlives the
    // brushes precisely so this can happen here, on the voxels that survived.
    let mut palette = palette.into_inner().unwrap();
    let grid = if config.shapes.enabled && !masks.is_empty() {
        let (fitted, count) = fit_shapes(&grid, &masks, &mut palette, &assets.pack);
        shapes_fitted = count;
        fitted
    } else {
        grid
    };

    // Props drawn as blocks rather than as entities. A prop's block may only
    // take a cell that is air, since taking one of the map's own would punch a
    // hole in whatever the prop stands against, and taking another prop's
    // would delete that prop.
    //
    // Before the barriers, not after, because a big prop's barriers fill the
    // shell its own geometry occupies — which is exactly where the blocks
    // drawing that geometry want to sit. Letting the barriers go first left
    // large props with nowhere to put their pieces and sent them back to being
    // entities. The barrier pass gives way instead: it skips whatever is
    // already there, so a prop block costs one voxel of collision out of a
    // shell that runs to thousands.
    let mut grid = grid;
    let mut baked = Vec::new();
    let mut taken: std::collections::HashSet<IVec3> = std::collections::HashSet::new();
    let mut assets = assets;
    // Held apart from `assets` for the loop, which reads the placements it is
    // registering meshes for.
    let mut pack = std::mem::take(&mut assets.pack);
    if config.props.bake {
        for (index, placement) in assets.placements.iter().enumerate() {
            let size = placement.bounds.size();
            let longest = size.x.max(size.y).max(size.z);
            if config.props.bake_max_size > 0.0 && longest > config.props.bake_max_size {
                continue;
            }
            let Some(mesh) = assets.prop_meshes.get(placement.mesh) else {
                continue;
            };

            let quaternion = crate::output::display::rotation(&placement.prop, &transform);
            let rounded = crate::output::display::dequantize(crate::output::display::quantize(
                quaternion,
                config.props.bake_angle_steps,
            ));
            let basis = crate::output::display::basis_of(rounded);
            let origin = transform.to_block_space(placement.prop.origin);
            let origin = Vec3::new(origin.x, origin.y + settle[index], origin.z);

            // A block model may be drawn outside its own block, but Sodium
            // packs chunk vertex coordinates into a range only 32 blocks wide
            // and masks off the rest, so a mesh reaching too far folds back on
            // itself. Anything that big is carried by several blocks instead.
            let reach = config.props.bake_reach.max(f64::MIN_POSITIVE);
            let pieces = mesh.split(basis, placement.prop.scale, reach);

            // All of a prop's pieces are placed or none of them is: half a
            // gantry is worse than a gantry drawn the slow way. Cells are
            // claimed as they are chosen and given back if the prop is
            // abandoned, so two pieces of the same prop cannot be handed the
            // same cell — the second block would replace the first and that
            // part of the mesh would simply not be drawn.
            let mut placing = Vec::with_capacity(pieces.len());
            let mut claimed: Vec<IVec3> = Vec::new();
            for piece in &pieces {
                // Inside what this piece actually covers, which for a prop
                // small enough not to be split is the whole prop's extent.
                let Some(cell) = crate::output::bake::anchor(
                    &grid,
                    Aabb::new(origin + piece.bounds.min, origin + piece.bounds.max),
                    &taken,
                ) else {
                    placing.clear();
                    break;
                };
                taken.insert(cell);
                claimed.push(cell);
                let key = crate::output::bake::Key::new(
                    &mesh.id,
                    quaternion,
                    origin,
                    cell,
                    placement.prop.scale,
                    config.props.bake_grid,
                    config.props.bake_angle_steps,
                    piece.centre,
                );
                // What the split actually achieved, rather than what it was
                // asked for: a single triangle wider than the reach cannot be
                // cut up by grouping, since nothing splits one triangle. A
                // prop still over the limit keeps the entity route, which is
                // drawn by the entity renderer and has no such limit.
                if mesh.reach_of(&key.place(config.props.bake_grid), piece) > reach {
                    placing.clear();
                    break;
                }
                placing.push((key, cell, piece));
            }
            if placing.is_empty() {
                for cell in claimed {
                    taken.remove(&cell);
                }
                continue;
            }

            for (key, cell, piece) in placing {
                let id = key.id();
                if pack.prop(&id).is_none() {
                    let place = key.place(config.props.bake_grid);
                    let asset = mesh.asset(id.clone(), Some(&place), Some(piece));
                    pack.insert_prop(asset, Vec::new());
                }
                baked.push((
                    index,
                    cell,
                    format!("{}:{id}", crate::output::kubejs::NAMESPACE),
                ));
            }
        }
    }
    assets.pack = pack;

    for (_, cell, block) in &baked {
        let id = palette.intern(block);
        grid.set(*cell, id);
    }

    let is_baked: std::collections::HashSet<usize> =
        baked.iter().map(|(index, _, _)| *index).collect();

    // Collision last, and only where there is nothing already. It is
    // invisible, so overwriting a wall with it opens a hole you can see
    // straight through; and coming after hollowing means it cannot make the
    // map's own blocks look like interior worth removing.
    let mut barriers = 0;
    for (pos, block) in collision.iter() {
        if grid.get(pos) == AIR {
            grid.set(pos, block);
            barriers += 1;
        }
    }

    // Shaped collision, for the props that were baked into blocks. A prop that
    // stayed a display entity keeps the barrier shell: it has no block of its
    // own, and cubes are what it always had.
    let mut collision_blocks = 0;
    let mut collision_shapes = 0;
    let mut collision_step = 1;
    if !shaped_collision.is_empty() {
        let mut wanted: std::collections::HashMap<IVec3, Aabb> = std::collections::HashMap::new();
        let mut fallback: std::collections::HashSet<IVec3> = std::collections::HashSet::new();
        for (index, cells) in &shaped_collision {
            for (cell, bounds) in cells {
                if is_baked.contains(index) {
                    // Two props sharing a cell share its block, so the box has
                    // to cover both. Whichever reaches further decides.
                    let entry = wanted.entry(*cell).or_insert_with(Aabb::empty);
                    entry.extend(bounds.min);
                    entry.extend(bounds.max);
                } else {
                    fallback.insert(*cell);
                }
            }
        }

        let (shapes, step) =
            crate::output::collision::quantize(&wanted, config.props.collision_max_shapes);
        collision_step = step;
        let mut distinct: std::collections::HashSet<crate::output::collision::Shape> =
            std::collections::HashSet::new();
        for (cell, shape) in shapes {
            if grid.get(cell) != AIR {
                continue;
            }
            let id = assets.pack.insert_collision(shape);
            let block = palette.intern(&id);
            grid.set(cell, block);
            distinct.insert(shape);
            collision_blocks += 1;
        }
        collision_shapes = distinct.len();

        let barrier = palette.intern(BARRIER);
        for cell in fallback {
            if grid.get(cell) == AIR {
                grid.set(cell, barrier);
                barriers += 1;
            }
        }
    }
    let grid = grid;
    // Props, not blocks: one that had to be split is still one prop.
    let props_baked = is_baked.len();
    let prop_blocks = baked.len();

    let palette = palette;
    let mut block_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (_, id) in grid.iter() {
        *block_counts
            .entry(palette.name(id).to_string())
            .or_default() += 1;
    }

    let skipped = skipped.into_inner();

    // The props that could not be baked into a block: too large for it, or
    // with no free cell to stand the block in. These keep the old route — one
    // display entity each, carrying the map's own rotation — which costs frame
    // rate but is never wrong, and there are few of them.
    let props: Vec<crate::output::display::Placement> = assets
        .placements
        .iter()
        .zip(&settle)
        .enumerate()
        .filter(|(index, _)| !is_baked.contains(index))
        .map(|(_, (placement, shift))| {
            let origin = transform.to_block_space(placement.prop.origin);
            crate::output::display::Placement {
                block: placement.block.clone(),
                pos: [origin.x, origin.y + shift, origin.z],
                rotation: crate::output::display::rotation(&placement.prop, &transform),
                scale: placement.prop.scale,
                width: placement.width,
                height: placement.height,
                view_range: config.props.view_range,
                full_bright: config.props.full_bright,
                tag: crate::output::display::tag_for(&map.name),
            }
        })
        .collect();

    Ok(Conversion {
        stats: Stats {
            solids_voxelized: solids.len() - skipped,
            solids_skipped: skipped,
            displacements_voxelized: surfaces.len() - displacements_skipped,
            displacements_skipped,
            textures_resolved: assets.stats.resolved,
            tile_cap: assets.stats.tile_cap,
            props_placed: assets.stats.props_placed,
            props_skipped: assets.stats.props_skipped,
            props_modelled: assets.stats.props_modelled,
            prop_models: assets.stats.prop_models,
            props_settled: settled,
            props_baked,
            prop_blocks,
            prop_barriers: barriers,
            prop_collision_blocks: collision_blocks,
            prop_collision_shapes: collision_shapes,
            prop_collision_step: collision_step,
            shapes_fitted,
            blocks_before_hollow,
            blocks: grid.count(),
            block_counts,
        },
        grid,
        palette,
        transform,
        separate,
        props,
        pack: assets.pack,
        bundle: assets.bundle,
    })
}

/// Bounds of the map in block space, for reporting.
pub fn block_bounds(map: &Map, transform: &Transform) -> Aabb {
    let bounds = map.converted_bounds(true);
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
        for (index, material) in materials.iter().enumerate() {
            assert_eq!(
                side_block(&glass, &r, Some(index), TextureFlags::empty(), true),
                None,
                "{} became glass",
                material.name
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
            side_block(
                &Decision::ByMaterial,
                &r,
                Some(0),
                TextureFlags::empty(),
                true
            )
            .is_some()
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
        // Props are added after hollowing and would make the two counts
        // measure different things, so this compares the world with itself.
        let mut config = Config::default();
        config.props.enabled = false;
        let result = convert(&map, &config).unwrap();

        assert!(result.stats.blocks > 5_000, "got {}", result.stats.blocks);
        assert!(result.stats.solids_voxelized > 0);
        assert!(
            result.palette.len() > 1,
            "palette should hold more than air"
        );

        // Hollowing must remove a substantial share of a solid map.
        assert!(
            result.stats.blocks < result.stats.blocks_before_hollow,
            "hollowing removed nothing"
        );
    }

    /// No single block should dominate the *brushwork* of a converted map.
    /// Before materials landed, fog volumes flagged `WINDOW` made glass 86% of
    /// some Entropy: Zero maps; a regression there would show up here first.
    ///
    /// Props are left out deliberately. Entropy: Zero's backdrop architecture
    /// is one enormous, genuinely near-black Combine wall, so with props on a
    /// single block legitimately owns three quarters of `az_c4_4` and this
    /// test would only ever be measuring that.
    #[test]
    fn no_single_block_swamps_a_converted_map() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.props.enabled = false;
        let result = convert(&map, &config).unwrap();
        let total = result.stats.blocks;
        let (block, count) = result
            .stats
            .block_counts
            .iter()
            .max_by_key(|(_, count)| **count)
            .unwrap();
        assert!(count * 2 < total, "{block} is {count} of {total} blocks");
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
        let origins = model_origins(&entity_models(&map, &config, &transform));

        let _ = &resolver;
        let mut checked = 0;
        for (model, origin) in &origins {
            if origin.x == 0.0 && origin.y == 0.0 && origin.z == 0.0 {
                continue;
            }
            let Some(solid) = map.solids(*model).into_iter().next() else {
                continue;
            };
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
        let origins = model_origins(&entity_models(&map, &config, &transform));

        let (mut with_total, mut without_total, mut count) = (0.0, 0.0, 0);
        for (model, origin) in &origins {
            if origin.x == 0.0 && origin.y == 0.0 && origin.z == 0.0 {
                continue;
            }
            let Some(solid) = map.solids(*model).into_iter().next() else {
                continue;
            };
            let entity = transform.to_block_space(*origin);
            with_total +=
                (to_block_solid(&solid, &transform, *origin).bounds.center() - entity).length();
            without_total += (to_block_solid(&solid, &transform, Vec3::ZERO)
                .bounds
                .center()
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
            let Some(_) = name.strip_prefix("kubejs:") else {
                continue;
            };
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
        let resolver =
            Resolver::with_textures(&config, map.materials(), &result.pack.ids()).unwrap();

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
            assert!(
                name.starts_with("minecraft:"),
                "vanilla mode emitted {name}"
            );
        }
    }

    /// Highway 17: cliffs and ground built from blend textures stretched over
    /// dozens of blocks, which is where the tiling cap actually bites.
    fn coast_map() -> Option<Map> {
        let path = Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d2_coast_03.bsp",
        );
        path.exists().then(|| Map::load(path).unwrap())
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

    /// Hollowing is about brush interiors, so props — which are surfaces
    /// already and have no interior to remove — are left out of the
    /// comparison rather than diluting it.
    #[test]
    fn hollow_mode_produces_far_fewer_blocks_than_solid() {
        let Some(map) = sample_map() else { return };

        let mut hollow_config = Config::default();
        hollow_config.props.enabled = false;
        let mut solid_config = hollow_config.clone();
        solid_config.fill.mode = FillMode::Solid;
        let solid = convert(&map, &solid_config).unwrap();
        let hollow = convert(&map, &hollow_config).unwrap();

        assert!(
            hollow.stats.blocks < solid.stats.blocks,
            "hollow {} vs solid {}",
            hollow.stats.blocks,
            solid.stats.blocks
        );
    }

    fn kubejs_config() -> Config {
        let mut config = Config::default();
        config.materials.mode = crate::config::MaterialMode::Kubejs;
        config
    }

    /// The failure the user sees as a hole in a wall: a barrier is invisible,
    /// so putting one where a block already was is the same as deleting it. A
    /// prop's own block is nearly as bad — it is a mesh floating where a wall
    /// used to be. Adding a map's props must not take a single one of its own
    /// blocks away, by either route.
    #[test]
    fn prop_collision_never_replaces_a_block_of_the_map() {
        let Some(map) = sample_map() else { return };

        let mut without = kubejs_config();
        without.props.enabled = false;
        let without = convert(&map, &without).unwrap();
        let with = convert(&map, &kubejs_config()).unwrap();
        if with.stats.prop_barriers == 0 && with.stats.props_baked == 0 {
            return;
        }

        for (pos, id) in without.grid.iter() {
            let before = without.palette.name(id);
            let after = with.palette.name(with.grid.get(pos));
            // A prop is allowed to put its own visible geometry where a block
            // was — that is only a swap you can see. What it may never do is
            // replace one with something invisible: a barrier, or the block a
            // baked mesh hangs off, which draws nothing in its own cell. Both
            // read as a hole through the wall.
            assert_ne!(
                after, BARRIER,
                "{pos:?} was {before} and a prop turned it into an invisible barrier"
            );
            assert!(
                !after.contains(":prop_"),
                "{pos:?} was {before} and a baked prop took the cell"
            );
            assert!(
                !after.contains(":collision_"),
                "{pos:?} was {before} and a prop's collision took the cell"
            );
        }
    }

    /// What the shaped route is for: a prop is solid as its own shape rather
    /// than as a stack of metre cubes. The shapes have to cover what the cubes
    /// covered — a cell that used to be solid must still be solid — and they
    /// have to be shapes, not full cubes wearing a new name.
    #[test]
    fn shaped_collision_covers_what_barriers_covered() {
        let Some(map) = sample_map() else { return };

        let mut cubes = kubejs_config();
        cubes.props.collision = crate::config::CollisionMode::Barrier;
        let cubes = convert(&map, &cubes).unwrap();
        if cubes.stats.prop_barriers == 0 {
            return; // no game install, or nothing big enough to be solid
        }
        let shaped = convert(&map, &kubejs_config()).unwrap();

        assert!(
            shaped.stats.prop_collision_blocks > 0,
            "no cell got a shape"
        );
        assert!(
            shaped.stats.prop_barriers * 20 < cubes.stats.prop_barriers,
            "{} barriers left of {}",
            shaped.stats.prop_barriers,
            cubes.stats.prop_barriers
        );

        // Every cell that was solid is still solid. Losing one is a floor you
        // fall through, which is the whole risk of measuring a box instead of
        // filling the cell.
        for (pos, id) in cubes.grid.iter() {
            if cubes.palette.name(id) != BARRIER {
                continue;
            }
            assert_ne!(
                shaped.grid.get(pos),
                crate::voxel::grid::AIR,
                "{pos:?} was solid with barriers and is empty with shapes"
            );
        }

        // And they really are shaped: a map of full cubes under another name
        // would pass everything above.
        let partial = shaped
            .pack
            .collisions()
            .filter(|shape| !shape.is_full())
            .count();
        assert!(
            partial * 2 > shaped.stats.prop_collision_shapes,
            "only {partial} of {} shapes are smaller than a whole cell",
            shaped.stats.prop_collision_shapes
        );
    }

    /// Vanilla output has no pack to register a shape in. Its props are
    /// voxelized into ordinary blocks and are solid by being there, so what
    /// must not happen is a conversion asking for shapes and getting neither
    /// those nor blocks.
    #[test]
    fn vanilla_output_needs_no_shapes_to_be_solid() {
        let Some(map) = sample_map() else { return };
        let result = convert(&map, &Config::default()).unwrap();
        assert_eq!(result.stats.prop_collision_blocks, 0, "no pack to use");
        assert_eq!(result.stats.props_modelled, 0, "vanilla drew a mesh");
        assert!(result.stats.props_placed > 0, "the props went missing");
    }

    /// Turning collision off leaves the props there and walk-through, rather
    /// than dropping them.
    #[test]
    fn collision_can_be_turned_off_without_losing_the_props() {
        let Some(map) = sample_map() else { return };
        let mut config = kubejs_config();
        config.props.collision = crate::config::CollisionMode::None;
        let result = convert(&map, &config).unwrap();
        assert_eq!(result.stats.prop_barriers, 0);
        assert_eq!(result.stats.prop_collision_blocks, 0);
        assert!(result.stats.props_baked > 0, "the props went too");
    }

    /// The other half of the same complaint: a barrier sealing the air side of
    /// a wall must not make hollowing mistake that wall for interior and carve
    /// it out. Every block the map had without props it still has with them.
    #[test]
    fn props_never_cause_the_map_behind_them_to_be_carved_away() {
        let Some(map) = sample_map() else { return };

        let mut without = kubejs_config();
        without.props.enabled = false;
        let without = convert(&map, &without).unwrap();
        let with = convert(&map, &kubejs_config()).unwrap();

        let lost = without
            .grid
            .iter()
            .filter(|(pos, _)| with.grid.get(*pos) == crate::voxel::grid::AIR)
            .count();
        assert_eq!(
            lost, 0,
            "{lost} of {} blocks vanished once props were added",
            without.stats.blocks
        );
    }

    /// What the whole change is for: props stop being entities. Almost all of
    /// them should end up as blocks the chunk mesh absorbs, and only the ones
    /// with nowhere to put a block stay as entities redrawn every frame.
    #[test]
    fn baking_turns_props_into_blocks_instead_of_entities() {
        let Some(map) = sample_map() else { return };

        let mut off = kubejs_config();
        off.props.bake = false;
        let off = convert(&map, &off).unwrap();
        if off.props.is_empty() {
            return;
        }
        let on = convert(&map, &kubejs_config()).unwrap();

        assert_eq!(off.stats.props_baked, 0, "baking was off");
        assert_eq!(
            on.stats.props_baked + on.props.len(),
            off.props.len(),
            "a prop was lost between the two routes"
        );
        assert!(
            on.props.len() * 4 < off.props.len(),
            "{} of {} props are still entities",
            on.props.len(),
            off.props.len()
        );

        // Every baked prop is a block in the grid naming a mesh the pack
        // registers, which is the drift that makes a paste silently empty.
        let mut blocks = 0;
        for (_, id) in on.grid.iter() {
            let name = on.palette.name(id);
            if name.contains(":prop_") {
                assert!(
                    on.pack.prop(name).is_some(),
                    "{name} is placed but not registered"
                );
                blocks += 1;
            }
        }
        assert_eq!(blocks, on.stats.prop_blocks, "a baked prop has no block");
        assert!(
            on.stats.prop_blocks >= on.stats.props_baked,
            "a prop cannot take fewer than one block"
        );
    }

    /// The failure that showed up as huge black sheets folded over the map.
    ///
    /// Sodium packs each chunk vertex coordinate into 20 bits spanning -8 to
    /// +24 blocks from the section origin and masks off the rest, so a block
    /// model reaching further than that is drawn correctly up to the limit and
    /// then folds back on itself. A block can sit anywhere in its 16-block
    /// section, so 8 blocks either way is the reach that is safe wherever it
    /// lands, and no generated mesh may exceed it.
    #[test]
    fn no_baked_mesh_reaches_further_than_a_chunk_vertex_can_be_encoded() {
        let Some(map) = sample_map() else { return };
        let config = kubejs_config();
        let converted = convert(&map, &config).unwrap();
        if converted.stats.props_baked == 0 {
            return;
        }

        let mut worst: f64 = 0.0;
        let mut worst_id = String::new();
        let mut checked = 0;
        for asset in converted.pack.props() {
            // Only the baked variants; the model-space assets an entity places
            // are drawn by the entity renderer, which has no such limit.
            if !asset.id.contains("_b") || asset.id == asset.mtl_id {
                continue;
            }
            checked += 1;
            for line in asset.obj.lines().filter(|l| l.starts_with("v ")) {
                for value in line.split_whitespace().skip(1) {
                    let reach: f64 = value.parse().unwrap_or(0.0);
                    if reach.abs() > worst {
                        worst = reach.abs();
                        worst_id = asset.id.clone();
                    }
                }
            }
        }

        assert!(checked > 0, "no baked variants to check");
        assert!(
            worst <= config.props.bake_reach,
            "{worst_id} reaches {worst:.1} blocks from its block; past \
             {} the coordinate wraps and the mesh folds back",
            config.props.bake_reach
        );
    }

    /// Two props must never be given the same cell, or the second block
    /// replaces the first and that prop is simply not drawn.
    #[test]
    fn no_two_baked_props_share_a_block() {
        let Some(map) = sample_map() else { return };
        let converted = convert(&map, &kubejs_config()).unwrap();
        if converted.stats.props_baked == 0 {
            return;
        }
        // One block per baked prop is exactly what the count above asserts;
        // this is the same statement from the other side, that the meshes
        // registered are distinct enough to be worth registering.
        assert!(
            converted.pack.props().count() > 1,
            "a whole map of props collapsed to one mesh"
        );
    }

    /// Props sink into the ground because they are placed to a fraction of a
    /// block and the floor under them is rounded to whole ones. Settling has
    /// to actually move some of them, and never by more than the limit.
    #[test]
    fn settling_lifts_props_out_of_the_floor() {
        let Some(map) = sample_map() else { return };

        // Baking is off so that every placement shows up in `props` and can be
        // compared position for position. What settling does is the same
        // either way; only where the answer is written down differs.
        let mut on = kubejs_config();
        on.props.bake = false;
        let mut off = on.clone();
        off.props.settle = false;
        let off = convert(&map, &off).unwrap();
        let on = convert(&map, &on).unwrap();
        if on.props.is_empty() {
            return;
        }

        assert_eq!(off.stats.props_settled, 0);
        assert!(
            on.stats.props_settled > on.props.len() / 10,
            "only {} of {} props were settled",
            on.stats.props_settled,
            on.props.len()
        );

        let limit = Config::default().props.settle_max;
        for (a, b) in off.props.iter().zip(&on.props) {
            assert_eq!(a.block, b.block, "settling reordered the props");
            assert_eq!([a.pos[0], a.pos[2]], [b.pos[0], b.pos[2]], "moved sideways");
            assert!(
                (b.pos[1] - a.pos[1]).abs() <= limit + 1e-9,
                "{} moved {} blocks",
                a.block,
                b.pos[1] - a.pos[1]
            );
        }
    }

    /// The claim the whole tiling feature rests on: at Hammer's default
    /// texture scale, one Minecraft block of wall is one tile of a 512-pixel
    /// texture split eight ways. Walk a block along the wall, advance one
    /// tile — and wrap round at the end, because the texture repeats.
    #[test]
    fn one_block_of_wall_advances_one_tile() {
        use crate::bsp::texcoord::TexCoord;

        let mut config = Config::default();
        config.scale.units_per_block = 16.0;
        config.transform.origin_mode = crate::config::OriginMode::MapOrigin;
        let transform = Transform::new(&config, Aabb::new(Vec3::ZERO, Vec3::splat(1024.0)));

        // A wall in the X/Z plane at four texels per unit: 16 units per block
        // is 64 texels, and a 512-pixel texture split into 8 gives 64-texel
        // tiles. So one block of wall is exactly one tile.
        let tex = TexCoord {
            u: [4.0, 0.0, 0.0, 0.0],
            v: [0.0, 0.0, -4.0, 0.0],
        };
        let uv = Uv::new(
            tex,
            &transform,
            Vec3::ZERO,
            [64.0, 64.0],
            transform.units_per_block(),
        );
        let set = TileSet {
            grid: [8, 8],
            texels_per_tile: [64.0, 64.0],
            ids: (0..64).collect(),
        };

        // Blocks 0..8 along the wall must give tiles 0..8 in order.
        let tile_at = |block: Vec3| {
            let (s, t) = uv.at(block);
            set.at(s, t)
        };
        let base = transform.to_block_space(Vec3::new(8.0, 0.0, -8.0));
        for step in 0..8 {
            let here = Vec3::new(base.x + step as f64, base.y, base.z);
            assert_eq!(
                tile_at(here),
                step as BlockId,
                "block {step} along the wall should be tile {step}"
            );
        }
        // The ninth block starts the texture again.
        assert_eq!(tile_at(Vec3::new(base.x + 8.0, base.y, base.z)), 0);
        // And so does the block eight before the first, going the other way.
        assert_eq!(tile_at(Vec3::new(base.x - 8.0, base.y, base.z)), 0);
        assert_eq!(tile_at(Vec3::new(base.x - 1.0, base.y, base.z)), 7);
    }

    /// One material is used at several scales in the same map — Highway 17's
    /// `nature/cliffface001a` at six of them — so a tile size taken from the
    /// material's typical scale is too wide for every face using a larger
    /// one, and the wall comes out in 2x2 blocks of the same picture.
    #[test]
    fn a_face_scaled_off_its_materials_median_still_gets_one_tile_per_block() {
        use crate::bsp::texcoord::TexCoord;

        let mut config = Config::default();
        config.scale.units_per_block = 16.0;
        config.transform.origin_mode = crate::config::OriginMode::MapOrigin;
        let transform = Transform::new(&config, Aabb::new(Vec3::ZERO, Vec3::splat(4096.0)));

        // Tiles cut for a material whose typical face is 4 texels per unit.
        let set = TileSet {
            grid: [8, 8],
            texels_per_tile: [64.0, 64.0],
            ids: (0..64).collect(),
        };

        // Every rate `cliffface001a` is really used at, plus the reference.
        for rate in [0.33, 0.5, 0.67, 1.0, 1.43, 2.0, 4.0, 8.0] {
            let tex = TexCoord {
                u: [rate, 0.0, 0.0, 0.0],
                v: [0.0, 0.0, -rate, 0.0],
            };
            let uv = Uv::new(
                tex,
                &transform,
                Vec3::ZERO,
                set.texels_per_tile,
                transform.units_per_block(),
            );

            // Walk a straight line of blocks along the wall. No two in a row
            // may wear the same tile.
            let base = transform.to_block_space(Vec3::new(8.0, 0.0, -8.0));
            let mut previous = None;
            let mut distinct = std::collections::HashSet::new();
            for step in 0..16 {
                let here = Vec3::new(base.x + step as f64, base.y, base.z);
                let (column, row) = uv.at(here);
                let tile = set.at(column, row);
                assert_ne!(
                    previous,
                    Some(tile),
                    "at {rate} texels/unit, blocks {} and {step} share tile {tile}",
                    step - 1,
                );
                previous = Some(tile);
                distinct.insert(tile);
            }
            // A face scaled *finer* than the material's reference advances by
            // more than one tile per block, so it cycles the grid faster and
            // legitimately shows fewer distinct tiles. That shows no repeat,
            // which is why it is left exact rather than corrected.
            let want = if rate * 16.0 <= set.texels_per_tile[0] {
                8
            } else {
                4
            };
            assert!(
                distinct.len() >= want,
                "at {rate} texels/unit only {} distinct tiles over 16 blocks",
                distinct.len()
            );
        }
    }

    /// A face with a degenerate texture vector must not divide by zero; it
    /// falls back to the material's own tile size.
    #[test]
    fn a_face_with_no_texture_axis_still_resolves() {
        use crate::bsp::texcoord::TexCoord;
        let config = Config::default();
        let transform = Transform::new(&config, Aabb::new(Vec3::ZERO, Vec3::splat(256.0)));
        let tex = TexCoord {
            u: [0.0; 4],
            v: [0.0; 4],
        };
        let uv = Uv::new(
            tex,
            &transform,
            Vec3::ZERO,
            [64.0, 64.0],
            transform.units_per_block(),
        );
        let (column, row) = uv.at(Vec3::new(3.0, 4.0, 5.0));
        assert!(column.is_finite() && row.is_finite(), "got {column}, {row}");
    }

    /// The other axis, and the one easiest to get upside down: Source's V
    /// points *down* a wall, so climbing must walk back up the tile rows.
    #[test]
    fn climbing_a_wall_walks_up_the_texture() {
        use crate::bsp::texcoord::TexCoord;

        let mut config = Config::default();
        config.scale.units_per_block = 16.0;
        config.transform.origin_mode = crate::config::OriginMode::MapOrigin;
        let transform = Transform::new(&config, Aabb::new(Vec3::ZERO, Vec3::splat(1024.0)));

        let tex = TexCoord {
            u: [4.0, 0.0, 0.0, 0.0],
            v: [0.0, 0.0, -4.0, 0.0],
        };
        let uv = Uv::new(
            tex,
            &transform,
            Vec3::ZERO,
            [64.0, 64.0],
            transform.units_per_block(),
        );
        let set = TileSet {
            grid: [8, 8],
            texels_per_tile: [64.0, 64.0],
            ids: (0..64).collect(),
        };

        // Source Z is Minecraft Y: one block up is one row earlier. Start a
        // few rows in, so the step being measured is not the wrap.
        let low = transform.to_block_space(Vec3::new(8.0, 0.0, -56.0));
        let (s, t) = uv.at(low);
        let below = set.at(s, t);
        let (s, t) = uv.at(Vec3::new(low.x, low.y + 1.0, low.z));
        let above = set.at(s, t);
        assert_eq!(
            above + set.grid[0] as BlockId,
            below,
            "going up a block should move one tile row towards the top of the texture"
        );
    }

    /// The regression that produced flat 10x10 patches of identical stone on
    /// Highway 17's cliffs: a cap on the tile count that stretched each tile
    /// over several blocks instead of shortening the window into the texture.
    /// Every tile must cover exactly one block, at any cap, on every material
    /// a real map uses.
    #[test]
    fn a_tile_never_covers_more_than_one_block_on_a_real_map() {
        let Some(map) = coast_map() else { return };
        let mut config = Config::default();

        for max in [4, 8, 16, 64] {
            config.materials.tile_max = max;
            let scales = crate::bsp::texcoord::material_scales(&map);
            let mut checked = 0;
            for scale in scales.into_iter().flatten() {
                let split = scale.split(config.scale.units_per_block, max, 16);
                for axis in 0..2 {
                    let texels_per_block =
                        scale.texels_per_unit[axis] * config.scale.units_per_block;
                    if texels_per_block <= 0.0 {
                        continue;
                    }
                    let blocks = split.texels_per_tile[axis] / texels_per_block;
                    assert!(
                        blocks < 1.5,
                        "tile_max {max}: a {:?} texture at {:.2} texels/unit gives \
                         tiles {blocks:.1} blocks wide",
                        scale.size,
                        scale.texels_per_unit[axis],
                    );
                    checked += 1;
                }
            }
            assert!(
                checked > 50,
                "only {checked} materials checked at tile_max {max}"
            );
        }
    }

    /// Raising the cap must buy a longer run before the pattern repeats, not
    /// change how much of the texture one block shows.
    #[test]
    fn raising_the_cap_widens_the_window_and_nothing_else() {
        let Some(map) = coast_map() else { return };
        let config = Config::default();

        let mut widened = 0;
        for scale in crate::bsp::texcoord::material_scales(&map)
            .into_iter()
            .flatten()
        {
            let small = scale.split(config.scale.units_per_block, 8, 16);
            let large = scale.split(config.scale.units_per_block, 32, 16);
            assert_eq!(
                small.texels_per_tile, large.texels_per_tile,
                "the cap changed how much texture one block shows"
            );
            assert!(large.grid[0] >= small.grid[0] && large.grid[1] >= small.grid[1]);
            if large.grid != small.grid {
                assert!(large.window[0] >= small.window[0]);
                widened += 1;
            }
        }
        assert!(widened > 0, "no material on this map is over the cap");
    }

    /// A whole map's worth: no tile may dominate, or the projection is not
    /// really varying and every wall is the same smear it was before.
    #[test]
    fn tiles_spread_across_a_real_map() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.materials.mode = crate::config::MaterialMode::Kubejs;

        let result = convert(&map, &config).unwrap();
        if result.pack.tilings().is_empty() {
            return; // no game install to read textures from
        }

        // Group the counts by material and check the busiest tile of the
        // busiest material is not most of it.
        let mut per_material: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (name, count) in &result.stats.block_counts {
            let Some(rest) = name.strip_prefix("kubejs:") else {
                continue;
            };
            // Collision shapes are named for their six numbers, which reads
            // exactly like a tile index and is not one.
            if rest.starts_with("collision_") {
                continue;
            }
            let Some((base, _)) = rest.rsplit_once('_').and_then(|(a, b)| {
                b.parse::<u32>().ok()?;
                a.rsplit_once('_')
            }) else {
                continue;
            };
            per_material
                .entry(base.to_string())
                .or_default()
                .push(*count);
        }

        let (material, counts) = per_material
            .iter()
            .max_by_key(|(_, counts)| counts.iter().sum::<usize>())
            .expect("kubejs mode produced no tiled blocks");
        let total: usize = counts.iter().sum();
        let busiest = *counts.iter().max().unwrap();
        assert!(
            counts.len() > 4,
            "{material} only used {} tiles",
            counts.len()
        );
        assert!(
            busiest * 4 < total,
            "{material}: one tile is {busiest} of {total} blocks, so the texture              is not really being split across the wall"
        );
    }

    /// The gap this closes: fences, railings, catwalks, crates and signs are
    /// all models, so a map converted from brushes alone is an accurate but
    /// empty shell.
    #[test]
    fn static_props_add_geometry_and_can_be_turned_off() {
        let Some(map) = sample_map() else { return };
        let mut without = Config::default();
        without.props.enabled = false;

        let without = convert(&map, &without).unwrap();
        let with = convert(&map, &Config::default()).unwrap();
        if with.stats.props_placed == 0 {
            return; // no game install to read models from
        }

        assert_eq!(without.stats.props_placed, 0);
        assert!(
            with.stats.blocks > without.stats.blocks,
            "props added nothing: {} vs {}",
            with.stats.blocks,
            without.stats.blocks
        );
    }

    /// The 3D skybox is a scale model of the horizon in a sealed room off in a
    /// corner. Converting it gives a second, wrongly-sized map, and the void
    /// between the two is most of the schematic's volume.
    #[test]
    fn leaving_out_the_3d_skybox_shrinks_the_map() {
        let Some(map) = terrain_map() else { return };
        if map.skybox().is_none() {
            return;
        }

        let mut with = Config::default();
        with.contents.skip_3d_skybox = false;
        let with = convert(&map, &with).unwrap();
        let without = convert(&map, &Config::default()).unwrap();

        let (a, b) = (with.grid.bounds().unwrap(), without.grid.bounds().unwrap());
        let volume = |(min, max): ([i32; 3], [i32; 3])| {
            (0..3)
                .map(|i| (max[i] - min[i] + 1) as i64)
                .product::<i64>()
        };
        assert!(
            volume(b) < volume(a),
            "excluding the skybox did not shrink the map: {} vs {}",
            volume(b),
            volume(a)
        );
        assert!(without.stats.blocks < with.stats.blocks);
    }

    /// Whatever the detector finds, the playable map has to survive it. This
    /// is the failure that would be worst and quietest: a converted map with
    /// its middle missing.
    #[test]
    fn the_skybox_never_eats_the_playable_map() {
        for map in [sample_map(), terrain_map()].into_iter().flatten() {
            let mut with = Config::default();
            with.contents.skip_3d_skybox = false;
            let with = convert(&map, &with).unwrap();
            let without = convert(&map, &Config::default()).unwrap();
            assert!(
                without.stats.blocks * 2 > with.stats.blocks,
                "{}: excluding the skybox removed more than half the map, {} of {}",
                map.name,
                with.stats.blocks - without.stats.blocks,
                with.stats.blocks
            );
        }
    }

    #[test]
    fn a_coarser_scale_produces_fewer_blocks() {
        let Some(map) = sample_map() else { return };
        let mut coarse = Config::default();
        coarse.scale.units_per_block = 64.0;

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
