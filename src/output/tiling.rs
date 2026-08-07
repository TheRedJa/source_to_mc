//! Splitting a converted map into pasteable schematic tiles.
//!
//! A whole E:Z2 map is far too large for one schematic, so the grid is cut into
//! a lattice of tiles. Only tiles holding geometry are written, which skips the
//! large empty volumes above and below the playable space.

use crate::output::schem;
use crate::voxel::grid::{BlockId, IVec3, Palette, VoxelGrid};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Tile {
    pub file: String,
    /// Inclusive block bounds in world space.
    pub min: IVec3,
    pub max: IVec3,
    pub size: IVec3,
    pub blocks: usize,
    /// Props placed in this tile as display entities. Pasted with `-e`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub props: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub map: String,
    pub units_per_block: f64,
    /// `None` when the whole map was written as a single schematic.
    pub tile_size: Option<u32>,
    pub total_blocks: usize,
    pub bounds_min: IVec3,
    pub bounds_max: IVec3,
    pub tiles: Vec<Tile>,
    /// Brush entities written to their own schematics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<EntityTile>,
    /// Props embedded in the schematics as display entities, which only paste
    /// with `//paste -e`.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub props: usize,
    /// The `summon` script that places the same props, if one was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prop_function: Option<String>,
    /// Blocks these schematics need registered before they will paste. Empty
    /// unless textures were generated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generated_blocks: Vec<String>,
    pub block_counts: std::collections::BTreeMap<String, usize>,
}

/// A brush entity written on its own, outside the world tiles.
#[derive(Debug, Clone, Serialize)]
pub struct EntityTile {
    pub file: String,
    pub classname: String,
    pub targetname: Option<String>,
    /// Index into the map's `entities.json`.
    pub entity: usize,
    /// Where the entity sits in the world, so it can be put back.
    pub min: IVec3,
    pub max: IVec3,
    pub blocks: usize,
}

/// Write one schematic per separated brush entity, into `dir/entities`.
pub fn write_entities(
    dir: &Path,
    entities: &[crate::convert::SeparateEntity],
    palette: &Palette,
) -> Result<Vec<EntityTile>> {
    if entities.is_empty() {
        return Ok(Vec::new());
    }
    let dir = dir.join("entities");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;

    let mut written = Vec::with_capacity(entities.len());
    for entity in entities {
        let Some((min, max)) = entity.grid.bounds() else { continue };
        let blocks: Vec<(IVec3, BlockId)> = entity.grid.iter().collect();
        let file = format!("{}.schem", entity.name());
        schem::write(&dir.join(&file), &blocks, palette, min, max, &file)?;

        written.push(EntityTile {
            file,
            classname: entity.classname.clone(),
            targetname: entity.targetname.clone(),
            entity: entity.entity,
            min,
            max,
            blocks: blocks.len(),
        });
    }
    Ok(written)
}

fn floor_div(value: i32, divisor: i32) -> i32 {
    value.div_euclid(divisor)
}

/// Write every non-empty tile, returning the manifest.
///
/// `tile_size` of `None` writes the entire map as one schematic.
pub fn write_tiles(
    dir: &Path,
    map_name: &str,
    grid: &VoxelGrid,
    palette: &Palette,
    tile_size: Option<u32>,
    units_per_block: f64,
    block_counts: std::collections::BTreeMap<String, usize>,
) -> Result<Manifest> {
    write_tiles_with_props(dir, map_name, grid, &[], palette, tile_size, units_per_block, block_counts)
}

/// As [`write_tiles`], placing each prop into the tile it stands in.
///
/// A prop that lands in a tile holding no blocks still gets one: dropping it
/// because nothing solid happens to share its cell would lose exactly the
/// free-standing scenery this is for.
#[allow(clippy::too_many_arguments)]
pub fn write_tiles_with_props(
    dir: &Path,
    map_name: &str,
    grid: &VoxelGrid,
    props: &[crate::output::display::Placement],
    palette: &Palette,
    tile_size: Option<u32>,
    units_per_block: f64,
    block_counts: std::collections::BTreeMap<String, usize>,
) -> Result<Manifest> {
    if let Some(size) = tile_size {
        ensure!(size > 0, "--tile-size must be at least 1");
        ensure!(
            size as i64 <= i16::MAX as i64,
            "--tile-size {size} exceeds the schematic format's 32767-block axis limit"
        );
    }

    let key_of = |pos: [i32; 3]| -> IVec3 {
        match tile_size {
            None => [0, 0, 0],
            Some(size) => {
                let size = size as i32;
                [
                    floor_div(pos[0], size),
                    floor_div(pos[1], size),
                    floor_div(pos[2], size),
                ]
            }
        }
    };
    let block_of = |p: &crate::output::display::Placement| -> IVec3 {
        [
            p.pos[0].floor() as i32,
            p.pos[1].floor() as i32,
            p.pos[2].floor() as i32,
        ]
    };

    let Some((min, max)) = grid.bounds() else {
        // Props alone are not a map; without any blocks there is nothing to
        // paste them into, and writing empty schematics would be noise.
        return Ok(Manifest {
            map: map_name.to_string(),
            units_per_block,
            tile_size,
            total_blocks: 0,
            bounds_min: [0, 0, 0],
            bounds_max: [0, 0, 0],
            tiles: Vec::new(),
            entities: Vec::new(),
            props: 0,
            prop_function: None,
            generated_blocks: Vec::new(),
            block_counts,
        });
    };

    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating output directory {}", dir.display()))?;

    // Bucket every block by the tile it belongs to in a single pass. Probing
    // the grid per tile instead would cost O(tiles x blocks), which is
    // hopeless once a map spans hundreds of tiles.
    //
    // Tiles are aligned to a global lattice, so maps converted with the same
    // settings line up with each other.
    let mut buckets: HashMap<IVec3, Vec<(IVec3, BlockId)>> = HashMap::new();
    for (pos, block) in grid.iter() {
        buckets.entry(key_of(pos)).or_default().push((pos, block));
    }

    let mut prop_buckets: HashMap<IVec3, Vec<crate::output::display::Placement>> = HashMap::new();
    for prop in props {
        prop_buckets
            .entry(key_of(block_of(prop)))
            .or_default()
            .push(prop.clone());
    }

    // Sort so output order is deterministic: y, then z, then x.
    let mut keys: Vec<IVec3> = buckets.keys().chain(prop_buckets.keys()).copied().collect();
    keys.sort_by_key(|k| (k[1], k[2], k[0]));
    keys.dedup();

    let mut tiles = Vec::with_capacity(keys.len());
    for key in keys {
        static NO_BLOCKS: Vec<(IVec3, BlockId)> = Vec::new();
        static NO_PROPS: Vec<crate::output::display::Placement> = Vec::new();
        let blocks = buckets.get(&key).unwrap_or(&NO_BLOCKS);
        let tile_props = prop_buckets.get(&key).unwrap_or(&NO_PROPS);
        if blocks.is_empty() && tile_props.is_empty() {
            continue;
        }

        // Shrink each tile to the geometry it actually holds, so a tile with a
        // single block does not carry a full cube of air. Props count as
        // geometry here: an entity outside the region it is written into is at
        // the mercy of whatever the pasting tool does with it.
        let mut tile_min = [i32::MAX; 3];
        let mut tile_max = [i32::MIN; 3];
        for pos in blocks
            .iter()
            .map(|(pos, _)| *pos)
            .chain(tile_props.iter().map(block_of))
        {
            for axis in 0..3 {
                tile_min[axis] = tile_min[axis].min(pos[axis]);
                tile_max[axis] = tile_max[axis].max(pos[axis]);
            }
        }

        let file = match tile_size {
            None => format!("{map_name}.schem"),
            Some(_) => format!("{map_name}_x{}_y{}_z{}.schem", key[0], key[1], key[2]),
        };
        schem::write_all(
            &dir.join(&file),
            blocks,
            tile_props,
            palette,
            tile_min,
            tile_max,
            &file,
        )?;

        tiles.push(Tile {
            file,
            min: tile_min,
            max: tile_max,
            size: [
                tile_max[0] - tile_min[0] + 1,
                tile_max[1] - tile_min[1] + 1,
                tile_max[2] - tile_min[2] + 1,
            ],
            blocks: blocks.len(),
            props: tile_props.len(),
        });
    }

    Ok(Manifest {
        map: map_name.to_string(),
        units_per_block,
        tile_size,
        total_blocks: grid.count(),
        bounds_min: min,
        bounds_max: max,
        tiles,
        entities: Vec::new(),
        props: 0,
        prop_function: None,
        generated_blocks: Vec::new(),
        block_counts,
    })
}

/// A WorldEdit macro that pastes every tile at its recorded position.
///
/// Each tile's absolute minimum corner is stored in the schematic's `Offset`,
/// and `//paste -o` places it there. Plain `//paste` is relative to wherever
/// the player happens to stand, so moving between pastes pulls the tiles out
/// of alignment with each other.
pub fn paste_script(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("# Generated by src2mc. Paste with WorldEdit or FAWE.\n");
    out.push_str("#\n");
    out.push_str("#   -o  pastes at the coordinates baked into the schematic\n");
    out.push_str("#   -a  skips air, so tiles do not erase each other\n");
    if manifest.props > 0 {
        out.push_str("#   -e  brings the props, which are display entities\n");
    }
    out.push_str("#\n");
    out.push_str(
        "# Without -o, WorldEdit pastes relative to where you are standing, so\n\
         # you would have to stand perfectly still for every tile to line up.\n",
    );
    out.push_str(&format!(
        "# Map: {} at {} units/block, {} tile(s), {} blocks total.\n",
        manifest.map,
        manifest.units_per_block,
        manifest.tiles.len(),
        manifest.total_blocks
    ));
    if !manifest.generated_blocks.is_empty() {
        out.push_str(&format!(
            "#\n# These schematics use {} generated blocks and will NOT paste\n\
             # correctly without the `kubejs` folder written next to them.\n",
            manifest.generated_blocks.len()
        ));
    }
    out.push_str(&format!(
        "# Occupies X {}..{}, Y {}..{}, Z {}..{}.\n\n",
        manifest.bounds_min[0],
        manifest.bounds_max[0],
        manifest.bounds_min[1],
        manifest.bounds_max[1],
        manifest.bounds_min[2],
        manifest.bounds_max[2],
    ));

    if manifest.props > 0 {
        out.push_str(&format!(
            "#\n# {} props are drawn as their real models, by display entities inside\n\
             # these schematics. They need `-e` to come across. If they do not\n\
             # arrive, run {} as a datapack function instead; it places\n\
             # exactly the same props at exactly the same coordinates.\n\n",
            manifest.props,
            manifest.prop_function.as_deref().unwrap_or("the props function"),
        ));
    }

    let paste = if manifest.props > 0 { "//paste -a -o -e" } else { "//paste -a -o" };
    for tile in &manifest.tiles {
        let stem = tile.file.trim_end_matches(".schem");
        out.push_str(&format!(
            "# tile at {},{},{}  ({} x {} x {}, {} blocks)\n//schem load {}\n{paste}\n\n",
            tile.min[0],
            tile.min[1],
            tile.min[2],
            tile.size[0],
            tile.size[1],
            tile.size[2],
            tile.blocks,
            stem
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_with(positions: &[IVec3]) -> (VoxelGrid, Palette) {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let mut grid = VoxelGrid::new();
        for pos in positions {
            grid.set(*pos, stone);
        }
        (grid, palette)
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("src2mc-tiling-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn empty_grid_writes_no_tiles() {
        let dir = temp_dir("empty");
        let (grid, palette) = grid_with(&[]);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, Some(64), 16.0, Default::default()).unwrap();
        assert!(manifest.tiles.is_empty());
        assert_eq!(manifest.total_blocks, 0);
    }

    #[test]
    fn geometry_in_one_tile_writes_one_file() {
        let dir = temp_dir("single");
        let (grid, palette) = grid_with(&[[1, 2, 3], [4, 5, 6]]);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, Some(64), 16.0, Default::default()).unwrap();

        assert_eq!(manifest.tiles.len(), 1);
        assert_eq!(manifest.tiles[0].blocks, 2);
        assert!(dir.join(&manifest.tiles[0].file).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spread_geometry_splits_across_tiles_and_skips_empty_ones() {
        let dir = temp_dir("split");
        // Two clusters far apart: the tiles between them must not be written.
        let (grid, palette) = grid_with(&[[0, 0, 0], [500, 0, 500]]);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, Some(64), 16.0, Default::default()).unwrap();

        assert_eq!(manifest.tiles.len(), 2, "only occupied tiles get written");
        assert_eq!(manifest.total_blocks, 2);
        assert_eq!(manifest.tiles.iter().map(|t| t.blocks).sum::<usize>(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every block must land in exactly one tile.
    #[test]
    fn tiles_partition_all_geometry() {
        let dir = temp_dir("partition");
        let mut positions = Vec::new();
        for x in -20i32..40 {
            for z in -5i32..25 {
                positions.push([x, x.rem_euclid(7), z]);
            }
        }
        let (grid, palette) = grid_with(&positions);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, Some(16), 16.0, Default::default()).unwrap();

        let tiled: usize = manifest.tiles.iter().map(|t| t.blocks).sum();
        assert_eq!(tiled, grid.count());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tiles_never_exceed_the_requested_size() {
        let dir = temp_dir("size");
        let mut positions = Vec::new();
        for x in 0..100 {
            positions.push([x, 0, 0]);
        }
        let (grid, palette) = grid_with(&positions);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, Some(32), 16.0, Default::default()).unwrap();

        for tile in &manifest.tiles {
            assert!(tile.size.iter().all(|s| *s <= 32), "tile too big: {tile:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_a_tile_size_beyond_the_format_limit() {
        let dir = temp_dir("toobig");
        let (grid, palette) = grid_with(&[[0, 0, 0]]);
        assert!(write_tiles(&dir, "m", &grid, &palette, Some(40_000), 16.0, Default::default()).is_err());
    }

    #[test]
    fn single_mode_writes_one_file_however_far_apart_the_geometry_is() {
        let dir = temp_dir("singlemode");
        let (grid, palette) = grid_with(&[[0, 0, 0], [500, 300, 500], [-40, 20, 90]]);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, None, 16.0, Default::default()).unwrap();

        assert_eq!(manifest.tiles.len(), 1);
        assert_eq!(manifest.tiles[0].file, "m.schem", "no tile suffix when unsplit");
        assert_eq!(manifest.tiles[0].blocks, 3);
        assert_eq!(manifest.tiles[0].min, [-40, 0, 0]);
        assert_eq!(manifest.tiles[0].max, [500, 300, 500]);
        assert!(dir.join("m.schem").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_single_schematic_too_large_to_encode_is_rejected() {
        let dir = temp_dir("hugesingle");
        // Two blocks far enough apart that the bounding box blows the cell cap.
        let (grid, palette) = grid_with(&[[0, 0, 0], [5000, 5000, 5000]]);
        let err = write_tiles(&dir, "m", &grid, &palette, None, 16.0, Default::default())
            .unwrap_err();
        assert!(err.to_string().contains("--tile-size"), "{err}");
    }

    /// `//pos1` does not affect `//paste`, so the script must not imply it does.
    #[test]
    fn paste_script_pastes_at_the_recorded_origin() {
        let dir = temp_dir("origin");
        let (grid, palette) = grid_with(&[[0, 0, 0], [500, 0, 500]]);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, Some(64), 16.0, Default::default()).unwrap();

        let script = paste_script(&manifest);
        assert!(!script.contains("//pos1"), "pos1 has no effect on paste");
        assert_eq!(
            script.matches("//paste -a -o").count(),
            manifest.tiles.len(),
            "every tile must paste at its own origin"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn paste_script_covers_every_tile() {
        let dir = temp_dir("script");
        let (grid, palette) = grid_with(&[[0, 0, 0], [500, 0, 500]]);
        let manifest =
            write_tiles(&dir, "m", &grid, &palette, Some(64), 16.0, Default::default()).unwrap();

        let script = paste_script(&manifest);
        for tile in &manifest.tiles {
            let stem = tile.file.trim_end_matches(".schem");
            assert!(script.contains(&format!("//schem load {stem}")), "script missing {stem}");
            assert!(script.contains(&format!(
                "# tile at {},{},{}",
                tile.min[0], tile.min[1], tile.min[2]
            )));
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
