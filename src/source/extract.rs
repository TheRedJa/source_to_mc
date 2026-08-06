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

use crate::bsp::texcoord::{MaterialScale, material_scales};
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
    /// Blocks registered for materials split across several of them.
    pub tiles: usize,
}

/// One placed prop's triangles, ready to voxelize.
#[derive(Debug, Clone)]
pub struct PropSurface {
    /// Triangles in Source world space.
    pub triangles: Vec<[Vec3; 3]>,
    /// Each triangle's centroid in texture space, parallel to `triangles`.
    pub uvs: Vec<[f64; 2]>,
    /// Index into the material list [`Assets::materials`] returns.
    pub material: usize,
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
    let want_props = config.props.enabled && !map.bsp.static_props.props.props.is_empty();
    if !want_pack && !want_props {
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
        if !want_pack || material.name.starts_with("tools/") {
            continue;
        }
        let grid = scales
            .get(index)
            .copied()
            .flatten()
            .map(|scale| grid_for(scale, config))
            .unwrap_or([1, 1]);
        let resolved = insert_block(
            &mut assets.pack,
            &materials,
            &mut textures,
            &material.name,
            Some(&material.raw_name),
            grid,
        );
        if resolved {
            assets.stats.resolved += 1;
            assets.stats.tiles += (grid[0] * grid[1]).saturating_sub(1) as usize;
        }
    }

    if want_props {
        place_props(map, config, &vfs, &materials, &mut textures, want_pack, &mut assets);
    }

    assets
}

/// How many blocks across and down to split a material's texture.
///
/// The natural answer is however many blocks of surface one repeat of the
/// texture covers, which is what `blocks_spanned` computes. Past the cap a
/// tile simply covers more than one block: the texture still lines up with
/// itself, at coarser resolution, which is a far better failure than
/// registering a thousand blocks for one sign.
pub(crate) fn grid_for(scale: MaterialScale, config: &Config) -> [u32; 2] {
    let max = config.materials.tile_max.max(1);
    let spanned = scale.blocks_spanned(config.scale.units_per_block);
    std::array::from_fn(|axis| (spanned[axis].round() as i64).clamp(1, max as i64) as u32)
}

/// Texels of a model's texture taken to be worth one block.
///
/// A `.mdl` has no texture scale to read: its UVs are an unwrap of the whole
/// model onto the sheet, so nothing relates a texel to a world unit. 64 is
/// what the ordinary world material works out to — a 512 texture at Hammer's
/// default scale over 16 units per block — so props are split at the same
/// granularity as the walls behind them.
const PROP_TEXELS_PER_TILE: u32 = 64;

/// Resolve one material to a generated block and add it to the pack.
fn insert_block(
    pack: &mut Pack,
    materials: &Materials,
    textures: &mut Textures,
    name: &str,
    raw_name: Option<&str>,
    grid: [u32; 2],
) -> bool {
    let Some(assets) = materials.assets(name, raw_name) else { return false };
    let Some(tiles) = textures.tiles(&assets.base_texture, assets.alpha_test, grid) else {
        return false;
    };
    let tiles = tiles.to_vec();
    pack.insert_tiled(name, &tiles, grid, &assets);
    true
}

/// Load every static prop's model and place its triangles in world space.
fn place_props(
    map: &Map,
    config: &Config,
    vfs: &Vfs,
    materials: &Materials,
    textures: &mut Textures,
    want_pack: bool,
    assets: &mut Assets,
) {
    let props = crate::bsp::props::extract(&map.bsp);
    // A malformed pattern must not take the conversion down with it; the
    // config loader already reports one, so here it simply skips nothing.
    let skip = globset(&config.props.skip).unwrap_or_else(|_| globset(&[]).unwrap());
    // The 3D skybox is full of props, and they are the worst ones to keep:
    // the miniature horizon is modelled at a scale the map is not, so
    // `d1_trainstation_02`'s distant Citadel converts into a 7000-unit tower
    // standing in the middle of the station.
    let skybox = map.skybox().filter(|_| config.contents.skip_3d_skybox);

    let mut models = Models::new(vfs);
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

        for part in &model.parts {
            let material = match index_of.get(&part.material) {
                Some(index) => *index,
                None => {
                    let index = base + assets.prop_materials.len();
                    assets
                        .prop_materials
                        .push(prop_material(materials, &mut *textures, &part.material));
                    if want_pack {
                        let grid = prop_grid(materials, textures, &part.material, config);
                        if insert_block(
                            &mut assets.pack,
                            materials,
                            textures,
                            &part.material,
                            None,
                            grid,
                        ) {
                            assets.stats.resolved += 1;
                            assets.stats.tiles += (grid[0] * grid[1]).saturating_sub(1) as usize;
                        }
                    }
                    index_of.insert(part.material.clone(), index);
                    index
                }
            };

            assets.props.push(PropSurface {
                triangles: part.triangles.iter().map(|tri| tri.map(|v| prop.place(v))).collect(),
                uvs: part.uvs.clone(),
                material,
            });
        }
        assets.stats.props_placed += 1;
    }

    assets.stats.models_loaded = models.stats().1;
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

/// How finely to split a model's texture. See [`PROP_TEXELS_PER_TILE`].
pub(crate) fn prop_grid(
    materials: &Materials,
    textures: &mut Textures,
    name: &str,
    config: &Config,
) -> [u32; 2] {
    if !config.materials.tile_textures {
        return [1, 1];
    }
    let Some(header) = materials
        .assets(name, None)
        .and_then(|assets| textures.header(&assets.base_texture))
    else {
        return [1, 1];
    };
    let max = config.materials.tile_max.max(1);
    std::array::from_fn(|axis| (header.size[axis] / PROP_TEXELS_PER_TILE).clamp(1, max))
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
        assert_eq!(assets.pack.len(), assets.stats.resolved + assets.stats.tiles);
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

        let Some((material, grid)) = assets
            .pack
            .tilings()
            .iter()
            .find(|(_, grid)| grid[0] > 2 && grid[1] > 2)
        else {
            return;
        };

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

        for (material, grid) in assets.pack.tilings() {
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
            assets.pack.blocks().all(|b| !b.material.starts_with("tools/")),
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
        assert!(assets.stats.materials > 0, "materials should still be counted");
        assert_eq!(assets.stats.props_placed, 0);
    }

    /// The point of the prop half of the module: a map's props have to come
    /// back as geometry, in world space, wearing materials the palette can
    /// resolve.
    #[test]
    fn static_props_come_back_as_world_space_triangles() {
        let Some(map) = sample_map() else { return };
        let assets = extract(&map, &Config::default());

        assert!(assets.stats.props_placed > 100, "{} props placed", assets.stats.props_placed);
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
