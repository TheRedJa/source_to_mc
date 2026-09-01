//! Reading everything the conversion wants out of the game's own content.
//!
//! Two things need the search path, and both need it at the same moment: the
//! real texture behind each material, and the models the map places as static
//! props. They also overlap — a prop's material is a material like any other,
//! and wants the same `.vmt` lookup, the same texture and the same block — so
//! they are gathered together, over one `Vfs` and one set of caches.
//!
//! All of it is best-effort. Without a game install the pack is empty and no
//! props are placed, and the conversion falls back to what the BSP alone can
//! tell it.

use crate::bsp::texcoord::{MaterialScale, Split, material_scales};
use crate::bsp::{Map, Material};
use crate::config::Config;
use crate::geom::Vec3;
use crate::output::kubejs::Pack;
use crate::source::mdl::Models;
use crate::source::vfs::Vfs;
use crate::source::vmt::Materials;
use crate::source::vtf::Textures;

/// What extraction managed, for reporting.
#[derive(Debug, Clone, Default)]
pub struct Extracted {
    pub materials: usize,
    pub resolved: usize,
    /// Content sources searched, in order.
    pub search_path: Vec<String>,
    /// Static props placed into the world.
    pub props_placed: usize,
    /// Props whose model could not be read, or that a rule skipped.
    pub props_skipped: usize,
    /// Distinct models read successfully.
    pub models_loaded: usize,
    /// Props placed as their real mesh rather than as blocks.
    pub props_modelled: usize,
    /// Distinct meshes generated for them.
    pub prop_models: usize,
    /// Blocks registered for materials split across several of them.
    pub tiles: usize,
    /// Tiles per axis the budget allowed, which is what the textures were
    /// actually cut at.
    pub tile_cap: u32,
}

/// One placed prop's triangles, ready to voxelize.
#[derive(Debug, Clone)]
pub struct PropSurface {
    /// Triangles in Source world space.
    pub triangles: Vec<[Vec3; 3]>,
    /// Each triangle corner's position in texture space, parallel to
    /// `triangles`.
    pub uvs: Vec<[[f64; 2]; 3]>,
    /// Index into the material list [`Assets::materials`] returns.
    pub material: usize,
}

/// One prop drawn as its real mesh, and where the map puts it.
#[derive(Debug, Clone)]
pub struct PropPlacement {
    pub prop: crate::bsp::props::Prop,
    /// Namespaced id of the generated block whose model is this mesh, in model
    /// space — what a display entity names.
    pub block: String,
    /// Index into [`Assets::prop_meshes`], for baking the mesh at this
    /// placement's own rotation instead.
    pub mesh: usize,
    /// Culling box of the model, in blocks.
    pub width: f32,
    pub height: f32,
    /// Where the placed mesh sits, in Source world space. What decides how far
    /// the prop has to move to meet the floor the conversion built.
    pub bounds: crate::geom::Aabb,
    /// Triangles to make solid behind the mesh, in Source world space. Empty
    /// for a prop small enough to walk through.
    ///
    /// Kept with the placement rather than alongside the voxelized props
    /// because the two move together: settling a crate onto the floor has to
    /// take its collision with it, or you stand on air above it.
    pub collision: Vec<[Vec3; 3]>,
}

/// Everything read off the search path for one map.
#[derive(Default)]
pub struct Assets {
    /// Generated blocks carrying the map's own textures. Empty outside
    /// `kubejs` mode.
    pub pack: Pack,
    /// Materials that only props use, to be appended after the map's own.
    pub prop_materials: Vec<Material>,
    pub props: Vec<PropSurface>,
    /// Props rendered as their own mesh rather than voxelized.
    pub placements: Vec<PropPlacement>,
    /// The geometry behind those placements, one entry per distinct `.mdl`.
    ///
    /// Kept because the same mesh is written out more than once: once in model
    /// space for the entities to orient, and again with each placement's own
    /// rotation baked in for the props drawn as blocks. Which of those happens
    /// depends on the world the conversion built, which is not known here.
    pub prop_meshes: Vec<crate::output::obj::PropMesh>,
    pub stats: Extracted,
}

impl Assets {
    /// The map's materials followed by the prop-only ones, which is the list
    /// every material index in this struct refers to.
    ///
    /// Appending rather than merging is what keeps `Map::material_index`
    /// valid: a brush side's material index still means what the BSP said.
    pub fn materials(&self, map: &Map) -> Vec<Material> {
        let mut all = map.materials().to_vec();
        all.extend(self.prop_materials.iter().cloned());
        all
    }
}

/// Read the search path for `map`: textures, and static prop geometry.
///
/// Materials without a texture behind them are simply absent from the pack,
/// and the palette falls back to rules and colour matching for those.
pub fn extract(map: &Map, config: &Config) -> Assets {
    let want_pack = config.materials.mode == crate::config::MaterialMode::Kubejs;
    let want_textures = want_pack;
    let want_props = config.props.enabled;
    if !want_textures && !want_props {
        return Assets::default();
    }

    let vfs = Vfs::for_map(&map.path, &config.materials.game_dir_paths());
    let materials = Materials::new(&vfs, Some(&map.bsp.pack));
    let mut textures = Textures::new(&vfs, config.materials.texture_size);

    let mut assets = Assets {
        stats: Extracted {
            search_path: vfs.describe(),
            ..Extracted::default()
        },
        ..Assets::default()
    };

    // How many blocks each texture really covers in this map, so it can be
    // cut into that many pieces rather than shrunk onto one block face.
    let scales = if config.materials.tile_textures {
        material_scales(map)
    } else {
        vec![None; map.materials().len()]
    };

    // Nothing is cut until every material is known, because how finely each
    // one may be cut depends on how many there are in total: the budget is on
    // the pack, not on any one texture.
    let mut pending: Vec<Pending> = Vec::new();

    // The map's own materials first, so their indices stay exactly the ones
    // `Map::material_index` hands out.
    let mut seen: Vec<&str> = Vec::new();
    for (index, material) in map.materials().iter().enumerate() {
        if seen.contains(&material.name.as_str()) {
            continue;
        }
        seen.push(&material.name);
        assets.stats.materials += 1;

        // Tool textures are dropped before any of this matters, and nodraw is
        // the single most-used material in every map.
        if !want_textures || material.name.starts_with("tools/") {
            continue;
        }
        pending.push(Pending {
            name: material.name.clone(),
            raw_name: Some(material.raw_name.clone()),
            layout: match scales.get(index).copied().flatten() {
                Some(scale) => Layout::World(scale),
                None => Layout::Unknown,
            },
        });
    }

    if want_props {
        place_props(
            map,
            config,
            &vfs,
            &materials,
            &mut textures,
            want_textures.then_some(&mut pending),
            &mut assets,
        );
    }

    let layouts: Vec<Layout> = pending.iter().map(|p| p.layout).collect();
    // Props register blocks too, and a baked one registers a block per
    // distinct placement rather than per model. That is the same startup cost
    // a split texture's tiles are, so it comes out of the same budget: what is
    // left over is what the textures may be cut into.
    let reserved = assets.prop_meshes.len()
        + if config.props.bake {
            assets.placements.len()
        } else {
            0
        };
    // Collision shapes are registered blocks too, but they are not reserved
    // for here: how many a map needs is not known until it has been voxelized,
    // and reserving their cap instead would cut every texture coarser to make
    // room for shapes the map may never use. They sit on top of the budget,
    // as `batch`'s props already do, and `collision_max_shapes` is what bounds
    // them.
    let cap = choose_cap(&layouts, config, reserved);
    assets.stats.tile_cap = cap;

    for item in &pending {
        let split = item.layout.split(config, cap);
        let resolved = insert_block(
            &mut assets.pack,
            &materials,
            &mut textures,
            &item.name,
            item.raw_name.as_deref(),
            split,
        );
        if resolved {
            assets.stats.resolved += 1;
            assets.stats.tiles += split.tiles().saturating_sub(1) as usize;
        }
    }

    assets
}

/// A material waiting for the cap to be settled before its texture is cut.
struct Pending {
    name: String,
    raw_name: Option<String>,
    layout: Layout,
}

/// One block, one whole texture: what a material gets when nothing says how
/// its texture is laid out.
pub(crate) const WHOLE: Split = Split {
    grid: [1, 1],
    texels_per_tile: [1.0, 1.0],
    window: [u32::MAX; 2],
};

/// What is known about how a material's texture sits on the surfaces wearing
/// it, which is all that decides how finely it should be cut.
#[derive(Debug, Clone, Copy)]
pub enum Layout {
    /// A world material: the map's own texture vectors say how it is laid out,
    /// and the texture repeats, so the grid may cover a window of it.
    World(MaterialScale),
    /// A model's sheet. Its UVs are an unwrap rather than a repeat, so the
    /// whole sheet is always used and the rate has to be measured off the
    /// model's geometry.
    Sheet { uv_per_unit: f64, size: [u32; 2] },
    /// Nothing known: one block wearing the whole texture.
    Unknown,
}

impl Layout {
    /// How to cut this texture at a given cap on tiles per axis.
    pub fn split(&self, config: &Config, cap: u32) -> Split {
        if !config.materials.tile_textures {
            return WHOLE;
        }
        let units = config.scale.units_per_block;
        let out = config.materials.texture_size;
        match *self {
            Layout::World(scale) => scale.split(units, cap, out),
            Layout::Sheet { uv_per_unit, size } => {
                // How many blocks of surface one pass over the sheet covers.
                let blocks = if uv_per_unit > 0.0 && uv_per_unit.is_finite() {
                    1.0 / (uv_per_unit * units)
                } else {
                    1.0
                };
                let limit = |axis: usize| (size[axis] / out.max(1)).max(1);
                let grid: [u32; 2] = std::array::from_fn(|axis| {
                    ((blocks.round() as i64).clamp(1, i64::from(cap.max(1))) as u32)
                        .min(limit(axis))
                });
                Split {
                    grid,
                    texels_per_tile: std::array::from_fn(|axis| {
                        (size[axis] as f64 / grid[axis] as f64).max(1.0)
                    }),
                    window: size,
                }
            }
            Layout::Unknown => WHOLE,
        }
    }
}

/// Every material one map would register, and how its texture is laid out.
///
/// For planning a budget across several maps before any of them is converted:
/// `batch` merges the packs, so the ceiling belongs to the merged pack rather
/// than to each map separately, and that total is only knowable up front.
/// Geometry is deliberately not built here — only what decides the cut.
pub fn layouts(map: &Map, config: &Config) -> std::collections::BTreeMap<String, Layout> {
    let mut out = std::collections::BTreeMap::new();
    if config.materials.mode != crate::config::MaterialMode::Kubejs {
        return out;
    }

    let vfs = Vfs::for_map(&map.path, &config.materials.game_dir_paths());
    let materials = Materials::new(&vfs, Some(&map.bsp.pack));
    let mut textures = Textures::new(&vfs, config.materials.texture_size);

    for (index, scale) in material_scales(map).into_iter().enumerate() {
        let Some(material) = map.materials().get(index) else {
            continue;
        };
        if material.name.starts_with("tools/") {
            continue;
        }
        out.insert(
            material.name.clone(),
            scale.map(Layout::World).unwrap_or(Layout::Unknown),
        );
    }

    if config.props.enabled {
        let skip = globset(&config.props.skip).unwrap_or_else(|_| globset(&[]).unwrap());
        let skybox = map.skybox().filter(|_| config.contents.skip_3d_skybox);
        let mut models = Models::new(&vfs);
        for prop in crate::bsp::props::extract(map) {
            if skybox.is_some_and(|room| room.contains_point(prop.origin))
                || skip.is_match(&prop.model)
            {
                continue;
            }
            let Some(model) = models.get(&prop.model) else {
                continue;
            };
            for part in &model.parts {
                if out.contains_key(&part.material) {
                    continue;
                }
                let layout =
                    sheet_layout(&materials, &mut textures, &part.material, part.uv_per_unit);
                out.insert(part.material.clone(), layout);
            }
        }
    }
    out
}

/// The cap a set of materials should be cut at, for callers that gathered
/// them with [`layouts`].
///
/// `reserved` is what the pack owes before any texture is cut. `batch` plans a
/// cap across every map at once, before any of them has been read for props,
/// so it has nothing to reserve and passes 0; the props it then registers are
/// on top of the budget rather than inside it.
pub fn cap_for(layouts: &[Layout], config: &Config, reserved: usize) -> u32 {
    choose_cap(layouts, config, reserved)
}

/// How many blocks a set of materials would register at a given cap.
pub fn blocks_at(layouts: &[Layout], config: &Config, cap: u32) -> usize {
    layouts
        .iter()
        .map(|l| l.split(config, cap).tiles() as usize)
        .sum()
}

/// Pick the finest cut that stays inside the block budget.
///
/// Every registered block costs a KubeJS instance startup time and memory, so
/// the honest control is a ceiling on the pack rather than on tiles per axis:
/// the same cap means very different totals for a one-room map and a whole
/// campaign. The count only rises with the cap, so the largest cap that fits
/// is found by walking down from the ceiling.
///
/// The resolution limit in [`Layout::split`] means this usually saturates well
/// before the ceiling — past a certain point a finer cut costs nothing because
/// the source texture has no more detail to give.
/// `reserved` is what the pack owes before a single texture is cut — the
/// blocks the map's props will register.
fn choose_cap(layouts: &[Layout], config: &Config, reserved: usize) -> u32 {
    let ceiling = config.materials.tile_max.max(1);
    let budget = config.materials.max_blocks.saturating_sub(reserved);
    if config.materials.max_blocks == 0 || !config.materials.tile_textures {
        return ceiling;
    }
    let blocks = |cap: u32| -> usize {
        layouts
            .iter()
            .map(|l| l.split(config, cap).tiles() as usize)
            .sum()
    };
    (1..=ceiling)
        .rev()
        .find(|cap| blocks(*cap) <= budget)
        .unwrap_or(1)
}

/// Resolve one material to a generated block and add it to the pack.
fn insert_block(
    pack: &mut Pack,
    materials: &Materials,
    textures: &mut Textures,
    name: &str,
    raw_name: Option<&str>,
    split: Split,
) -> bool {
    let Some(assets) = materials.assets(name, raw_name) else {
        return false;
    };
    let Some(tiles) = textures.tiles(
        &assets.base_texture,
        assets.alpha_test,
        split.grid,
        split.window,
    ) else {
        return false;
    };
    let tiles = tiles.to_vec();
    pack.insert_tiled(name, &tiles, split, &assets);
    true
}

/// Load every static prop's model and place its triangles in world space.
fn place_props(
    map: &Map,
    config: &Config,
    vfs: &Vfs,
    materials: &Materials,
    textures: &mut Textures,
    mut pending: Option<&mut Vec<Pending>>,
    assets: &mut Assets,
) {
    let mut props = crate::bsp::props::extract(map);
    if config.props.entity_props {
        // Crates, barrels, doors and cars are entities, not `sprp` records.
        props.extend(crate::bsp::props::extract_entities(&map.bsp));
    }
    // Whether props may be drawn as their own mesh. It needs the generated
    // pack, since that is what registers the models, so this is a `kubejs`
    // mode feature and vanilla output is unchanged.
    //
    let modelled = config.props.models
        && pending.is_some()
        && config.materials.mode == crate::config::MaterialMode::Kubejs;
    let mut prop_textures = Textures::new(vfs, config.props.texture_size);
    // A malformed pattern must not take the conversion down with it; the
    // config loader already reports one, so here it simply skips nothing.
    let skip = globset(&config.props.skip).unwrap_or_else(|_| globset(&[]).unwrap());
    // The 3D skybox is full of props, and they are the worst ones to keep:
    // the miniature horizon is modelled at a scale the map is not, so
    // `d1_trainstation_02`'s distant Citadel converts into a 7000-unit tower
    // standing in the middle of the station.
    let skybox = map.skybox().filter(|_| config.contents.skip_3d_skybox);

    let mut models = Models::new(vfs);
    // Model path to the mesh built for it, so a fence repeated a dozen times
    // is one OBJ and one attempt at building it.
    let mut meshes: std::collections::HashMap<String, Option<(String, usize, f32, f32)>> =
        std::collections::HashMap::new();
    // Material name to its index in the combined list. The map's own
    // materials come first and keep the indices the BSP gave them.
    let mut index_of: std::collections::HashMap<String, usize> = map
        .materials()
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.clone(), i))
        .collect();
    let base = map.materials().len();

    for prop in &props {
        if skybox.is_some_and(|room| room.contains_point(prop.origin)) {
            assets.stats.props_skipped += 1;
            continue;
        }
        if skip.is_match(&prop.model) {
            assets.stats.props_skipped += 1;
            continue;
        }
        let Some(model) = models.get(&prop.model) else {
            assets.stats.props_skipped += 1;
            continue;
        };
        let size = model.bounds.size() * prop.scale;
        let longest = size.x.max(size.y).max(size.z);
        let too_big = config.props.max_size > 0.0 && longest > config.props.max_size;
        if longest < config.props.min_size || too_big {
            assets.stats.props_skipped += 1;
            continue;
        }

        // Drawn as itself, if a mesh can be built for it. Everything that can
        // go wrong here — a material with no texture, a model heavier than the
        // budget — falls back to voxelizing, so a prop is never lost for want
        // of a mesh.
        if modelled {
            let mesh = match meshes.get(&prop.model) {
                Some(cached) => cached.clone(),
                None => {
                    let built = crate::output::obj::build(
                        &prop.model,
                        &model,
                        config,
                        materials,
                        &mut prop_textures,
                    )
                    .map(|(mesh, textures)| {
                        let (width, height) = (mesh.width, mesh.height);
                        // The model-space asset, which is what a display
                        // entity places and what a baked variant falls back
                        // to. Registering it costs one block per model.
                        let block = assets
                            .pack
                            .insert_prop(mesh.asset(mesh.id.clone(), None, None), textures);
                        assets.prop_meshes.push(mesh);
                        (block, assets.prop_meshes.len() - 1, width, height)
                    });
                    meshes.insert(prop.model.clone(), built.clone());
                    built
                }
            };

            if let Some((block, mesh, width, height)) = mesh {
                // Big props are solid, small ones are scenery you walk
                // through. A display entity has no collision of its own, so
                // being solid means invisible barriers behind the mesh.
                let collision: Vec<[Vec3; 3]> = if longest >= config.props.collision_min_size {
                    model
                        .parts
                        .iter()
                        .flat_map(|part| &part.triangles)
                        .map(|tri| tri.map(|v| prop.place(v)))
                        .collect()
                } else {
                    Vec::new()
                };

                let mut bounds = crate::geom::Aabb::empty();
                for corner in 0..8 {
                    let pick = |axis: usize, lo: Vec3, hi: Vec3| {
                        if corner & (1 << axis) == 0 {
                            lo.axis(axis)
                        } else {
                            hi.axis(axis)
                        }
                    };
                    bounds.extend(prop.place(Vec3::new(
                        pick(0, model.bounds.min, model.bounds.max),
                        pick(1, model.bounds.min, model.bounds.max),
                        pick(2, model.bounds.min, model.bounds.max),
                    )));
                }

                assets.placements.push(PropPlacement {
                    prop: prop.clone(),
                    block,
                    mesh,
                    width: width * prop.scale as f32,
                    height: height * prop.scale as f32,
                    bounds,
                    collision,
                });
                assets.stats.props_placed += 1;
                assets.stats.props_modelled += 1;
                continue;
            }
        }

        for part in &model.parts {
            let material = match index_of.get(&part.material) {
                Some(index) => *index,
                None => {
                    let index = base + assets.prop_materials.len();
                    assets.prop_materials.push(prop_material(
                        materials,
                        &mut *textures,
                        &part.material,
                    ));
                    if let Some(pending) = pending.as_mut() {
                        pending.push(Pending {
                            name: part.material.clone(),
                            raw_name: None,
                            layout: sheet_layout(
                                materials,
                                textures,
                                &part.material,
                                part.uv_per_unit,
                            ),
                        });
                    }
                    index_of.insert(part.material.clone(), index);
                    index
                }
            };

            assets.props.push(PropSurface {
                triangles: part
                    .triangles
                    .iter()
                    .map(|tri| tri.map(|v| prop.place(v)))
                    .collect(),
                uvs: part.uvs.clone(),
                material,
            });
        }
        assets.stats.props_placed += 1;
    }

    assets.stats.models_loaded = models.stats().1;
    assets.stats.prop_models = meshes.values().filter(|m| m.is_some()).count();
    assets.pack.set_flip_v(config.props.flip_v);
}

/// Describe a prop's material the way the BSP describes a wall's, so both go
/// through the same rules and the same colour matching.
///
/// The average colour comes from the `.vtf` header, which is where the map
/// compiler reads it from too when it bakes `reflectivity` into the BSP.
fn prop_material(materials: &Materials, textures: &mut Textures, name: &str) -> Material {
    let reflectivity = materials
        .assets(name, None)
        .and_then(|assets| textures.header(&assets.base_texture))
        .map(|header| header.reflectivity)
        .unwrap_or([0.0; 3]);
    Material {
        name: name.to_string(),
        raw_name: name.to_string(),
        reflectivity,
    }
}

/// What a model's material looks like, measured off the geometry that wears
/// it. See [`Layout::Sheet`].
pub(crate) fn sheet_layout(
    materials: &Materials,
    textures: &mut Textures,
    name: &str,
    uv_per_unit: f64,
) -> Layout {
    match materials
        .assets(name, None)
        .and_then(|assets| textures.header(&assets.base_texture))
    {
        Some(header) => Layout::Sheet {
            uv_per_unit,
            size: header.size,
        },
        None => Layout::Unknown,
    }
}

fn globset(patterns: &[String]) -> Result<globset::GlobSet, globset::Error> {
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(globset::Glob::new(pattern)?);
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MaterialMode;
    use std::path::Path;

    fn sample_map() -> Option<Map> {
        let path = Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_trainstation_02.bsp",
        );
        path.exists().then(|| Map::load(path).unwrap())
    }

    fn kubejs() -> Config {
        let mut config = Config::default();
        config.materials.mode = MaterialMode::Kubejs;
        config
    }

    #[test]
    fn extracts_a_block_per_resolvable_material() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());

        assert!(!assets.stats.search_path.is_empty());
        assert!(
            assets.stats.resolved > 100,
            "only {} materials resolved",
            assets.stats.resolved
        );
        // One block per material, plus the extra tiles each split one needs.
        assert_eq!(
            assets.pack.len(),
            assets.stats.resolved + assets.stats.tiles
        );
        assert!(
            assets.stats.tiles > assets.stats.resolved,
            "only {} extra tiles for {} materials: textures are barely being split",
            assets.stats.tiles,
            assets.stats.resolved
        );

        // Every generated block must carry a texture of the configured size.
        for block in assets.pack.blocks() {
            assert_eq!(block.texture.dimensions(), (16, 16), "{}", block.id);
            assert!(!block.id.is_empty());
        }
    }

    /// A texture split across N blocks must give N genuinely different tiles,
    /// not N copies of the same downsample.
    #[test]
    fn splitting_a_texture_gives_different_tiles() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());

        let Some((material, split)) = assets
            .pack
            .tilings()
            .iter()
            .find(|(_, split)| split.grid[0] > 2 && split.grid[1] > 2)
        else {
            return;
        };
        let grid = split.grid;
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for row in 0..grid[1] {
            for column in 0..grid[0] {
                let id = assets.pack.tile_id(material, column, row).unwrap();
                let block = assets
                    .pack
                    .blocks()
                    .find(|b| b.block_id() == id)
                    .unwrap_or_else(|| panic!("{id} is not registered"));
                seen.push(block.texture.as_raw().clone());
            }
        }
        let distinct = {
            let mut sorted = seen.clone();
            sorted.sort();
            sorted.dedup();
            sorted.len()
        };
        assert!(
            distinct * 2 > seen.len(),
            "{material} split into {} tiles but only {distinct} are distinct",
            seen.len()
        );
    }

    /// Every tile has to be reachable through the id the palette will use, or
    /// the schematic names blocks the pack never registered.
    #[test]
    fn every_tile_of_a_split_material_is_registered() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());
        let script = assets.pack.script();

        for (material, split) in assets.pack.tilings() {
            let grid = split.grid;
            for row in 0..grid[1] {
                for column in 0..grid[0] {
                    let id = assets
                        .pack
                        .tile_id(material, column, row)
                        .unwrap_or_else(|| panic!("{material} tile {column},{row} has no id"));
                    assert!(
                        script.contains(&format!("event.create('{id}')")),
                        "{id} is not registered"
                    );
                }
            }
            // Out-of-range indices wrap, because a texture repeats along a
            // wall and a face can run well past one repeat of it.
            assert_eq!(
                assets.pack.tile_id(material, grid[0], grid[1]),
                assets.pack.tile_id(material, 0, 0)
            );
        }
    }

    /// The budget is what a KubeJS instance actually pays for, so it has to
    /// bind: a pack asked to fit a small number of blocks must come in under
    /// it, by cutting textures more coarsely rather than by dropping any.
    #[test]
    fn a_tight_budget_shrinks_the_pack_without_losing_materials() {
        let Some(map) = sample_map() else { return };

        let generous = extract(&map, &kubejs());
        if generous.pack.is_empty() {
            return;
        }

        let mut tight = kubejs();
        tight.materials.max_blocks = 2_000;
        let tight = extract(&map, &tight);

        assert!(
            tight.pack.registered() <= 2_000,
            "budget of 2000 gave {} blocks",
            tight.pack.registered()
        );
        assert!(tight.pack.registered() < generous.pack.registered());
        // Coarser, not smaller: every material still gets a block.
        assert_eq!(tight.stats.resolved, generous.stats.resolved);
        assert!(tight.stats.tile_cap < generous.stats.tile_cap);
    }

    /// With no budget the cap is the ceiling, and the pack saturates: past the
    /// point where every texture is at its own resolution, raising the ceiling
    /// buys nothing and costs nothing.
    #[test]
    fn quality_saturates_once_every_texture_is_at_full_resolution() {
        let Some(map) = sample_map() else { return };
        let mut config = kubejs();
        config.materials.max_blocks = 0;

        config.materials.tile_max = 64;
        let a = extract(&map, &config);
        if a.pack.is_empty() {
            return;
        }
        config.materials.tile_max = 256;
        let b = extract(&map, &config);
        assert_eq!(
            a.pack.registered(),
            b.pack.registered(),
            "raising the ceiling past saturation changed the pack"
        );
    }

    /// The budget must never be met by registering fewer materials.
    #[test]
    fn a_budget_never_drops_a_material() {
        let Some(map) = sample_map() else { return };
        for budget in [0, 500, 5_000, 100_000] {
            let mut config = kubejs();
            config.materials.max_blocks = budget;
            let assets = extract(&map, &config);
            if assets.pack.is_empty() {
                return;
            }
            let ids = assets.pack.ids();
            assert!(
                ids.len() >= assets.stats.resolved,
                "budget {budget} lost materials: {} ids for {} resolved",
                ids.len(),
                assets.stats.resolved
            );
        }
    }

    /// Turning tiling off has to give exactly one block per material again.
    #[test]
    fn tiling_can_be_turned_off() {
        let Some(map) = sample_map() else { return };
        let mut config = kubejs();
        config.materials.tile_textures = false;

        let assets = extract(&map, &config);
        assert!(assets.pack.tilings().is_empty());
        assert_eq!(assets.stats.tiles, 0);
        assert_eq!(assets.pack.len(), assets.stats.resolved);
    }

    /// Tool textures never become blocks, so generating one would be dead
    /// weight in every pack.
    #[test]
    fn tool_textures_are_not_registered() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());
        assert!(
            assets
                .pack
                .blocks()
                .all(|b| !b.material.starts_with("tools/")),
            "a tool texture was registered"
        );
    }

    /// The ids the palette will use must all be registered by the script.
    #[test]
    fn every_id_the_palette_would_use_is_registered() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());

        let script = assets.pack.script();
        for (material, id) in assets.pack.ids() {
            assert!(
                script.contains(&format!("event.create('{id}')")),
                "{material} resolves to {id}, which the script does not register"
            );
        }
    }

    #[test]
    fn a_map_with_no_content_yields_an_empty_pack() {
        let Some(map) = sample_map() else { return };
        let mut orphan = map;
        orphan.path = std::path::PathBuf::from("/nowhere/x.bsp");
        let assets = extract(&orphan, &kubejs());
        assert!(assets.pack.is_empty());
        assert_eq!(assets.stats.resolved, 0);
        assert!(
            assets.stats.materials > 0,
            "materials should still be counted"
        );
        assert_eq!(assets.stats.props_placed, 0);
    }

    /// The point of the prop half of the module: a map's props have to come
    /// back as geometry, in world space, wearing materials the palette can
    /// resolve.
    #[test]
    fn static_props_come_back_as_world_space_triangles() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &Config::default());

        assert!(
            assets.stats.props_placed > 100,
            "{} props placed",
            assets.stats.props_placed
        );
        assert!(!assets.props.is_empty());

        let all = assets.materials(&map);
        let bounds = map.bounds();
        for surface in &assets.props {
            assert!(surface.material < all.len());
            for v in surface.triangles.iter().flatten() {
                assert!(v.is_finite());
                for axis in 0..3 {
                    assert!(
                        v.axis(axis) >= bounds.min.axis(axis) - 512.0
                            && v.axis(axis) <= bounds.max.axis(axis) + 512.0,
                        "prop vertex {v:?} is outside the map"
                    );
                }
            }
        }
    }

    /// Prop materials must carry a real average colour, or every prop in
    /// vanilla mode falls back to the same grey.
    #[test]
    fn prop_materials_get_a_colour_from_their_texture() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &Config::default());
        if assets.prop_materials.is_empty() {
            return;
        }
        let coloured = assets
            .prop_materials
            .iter()
            .filter(|m| m.reflectivity.iter().any(|c| *c > 0.0))
            .count();
        assert!(
            coloured * 2 > assets.prop_materials.len(),
            "only {coloured} of {} prop materials have a colour",
            assets.prop_materials.len()
        );
    }

    /// The point of drawing props as meshes: most of them stop being blocks.
    #[test]
    fn props_come_back_as_meshes_rather_than_voxels() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());
        if assets.stats.props_placed == 0 {
            return;
        }

        assert!(
            assets.stats.props_modelled * 2 > assets.stats.props_placed,
            "only {} of {} props are drawn as themselves",
            assets.stats.props_modelled,
            assets.stats.props_placed
        );
        assert_eq!(assets.placements.len(), assets.stats.props_modelled);
        // A model placed many times is one mesh, or a map of fences would
        // register hundreds of copies of one fence.
        assert!(
            assets.stats.prop_models < assets.stats.props_modelled,
            "{} meshes for {} placements: nothing is being shared",
            assets.stats.prop_models,
            assets.stats.props_modelled
        );
    }

    /// The failure that produces a silently broken paste: an entity naming a
    /// block the pack never registered. Nothing else in the output shows it.
    #[test]
    fn every_prop_an_entity_names_is_registered_and_has_its_files() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());
        if assets.placements.is_empty() {
            return;
        }

        let script = assets.pack.script();
        for placement in &assets.placements {
            assert!(
                script.contains(&format!("event.create('{}')", placement.block)),
                "{} is placed but never registered",
                placement.block
            );
            let asset = assets
                .pack
                .prop(&placement.block)
                .unwrap_or_else(|| panic!("{} has no mesh", placement.block));
            assert!(!asset.obj.is_empty() && !asset.mtl.is_empty());
            assert!(!asset.textures.is_empty(), "{} wears nothing", asset.id);
            // The MTL and the model JSON have to agree on every slot, or the
            // loader silently draws the model untextured.
            for slot in asset.textures.keys() {
                assert!(
                    asset.mtl.contains(&format!("#{slot}")),
                    "{slot} is not in the MTL"
                );
            }
        }
    }

    /// A prop drawn as a mesh must not also be voxelized into visible blocks;
    /// the whole point is that the cubes are gone.
    #[test]
    fn modelled_props_leave_only_invisible_collision_behind() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());
        if assets.placements.is_empty() {
            return;
        }
        // A modelled prop's solid backing lives on the placement, so it can
        // move with the mesh and stay out of the world's own blocks.
        let solid = assets
            .placements
            .iter()
            .filter(|p| !p.collision.is_empty())
            .count();
        assert!(solid > 0, "no props are solid at all");
        assert!(
            solid < assets.placements.len(),
            "even small clutter is solid"
        );
    }

    /// Turning the meshes off has to give back exactly the old behaviour.
    #[test]
    fn prop_meshes_can_be_turned_off() {
        let Some(map) = sample_map() else { return };
        let mut config = kubejs();
        config.props.models = false;
        let assets = extract(&map, &config);

        assert!(assets.placements.is_empty());
        assert_eq!(assets.stats.props_modelled, 0);
        assert!(
            !assets.props.is_empty(),
            "props should be voxelized instead"
        );
    }

    /// Vanilla output has no pack to register meshes in, so it must keep
    /// voxelizing however the config is set.
    #[test]
    fn vanilla_mode_never_produces_meshes() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.props.models = true;
        let assets = extract(&map, &config);
        assert!(assets.placements.is_empty());
        assert!(assets.pack.is_empty());
    }

    /// Crates, barrels, doors and cars live in the entity lump, and leaving
    /// them out is why a converted warehouse is an empty warehouse.
    #[test]
    fn entity_props_are_placed_too() {
        let Some(map) = sample_map() else { return };
        let mut without = Config::default();
        without.props.entity_props = false;
        let without = extract(&map, &without);
        let with = extract(&map, &Config::default());

        assert!(
            with.stats.props_placed > without.stats.props_placed,
            "{} props either way: the entity lump contributed nothing",
            with.stats.props_placed
        );
    }

    /// A model `vmdl` cannot read must cost one prop, not the conversion. The
    /// entity lump is full of animated models it panics on.
    #[test]
    fn an_unreadable_model_does_not_take_the_map_down() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &Config::default());
        assert!(assets.stats.props_placed > 0);
    }

    #[test]
    fn props_can_be_turned_off() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.props.enabled = false;
        let assets = extract(&map, &config);
        assert!(assets.props.is_empty());
        assert_eq!(assets.stats.props_placed, 0);
    }

    /// A skip pattern has to actually keep the model out.
    #[test]
    fn skip_patterns_drop_matching_models() {
        let Some(map) = sample_map() else { return };
        let mut config = Config::default();
        config.props.skip = vec!["*".into()];
        let assets = extract(&map, &config);
        assert_eq!(assets.stats.props_placed, 0);
        assert!(assets.stats.props_skipped > 0);
    }

    /// In `kubejs` mode a prop's material has to be registered too, or every
    /// prop pastes as a hole.
    #[test]
    fn prop_materials_are_registered_in_the_pack() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &kubejs());
        if assets.pack.is_empty() || assets.prop_materials.is_empty() {
            return;
        }
        let ids = assets.pack.ids();
        let registered = assets
            .prop_materials
            .iter()
            .filter(|m| ids.contains_key(&m.name))
            .count();
        assert!(
            registered * 2 > assets.prop_materials.len(),
            "only {registered} of {} prop materials are in the pack",
            assets.prop_materials.len()
        );
    }
}
