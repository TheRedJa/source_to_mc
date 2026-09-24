//! Writing Sponge Schematic v3 (`.schem`) files.
//!
//! Follows the SpongePowered specification: gzipped NBT whose unnamed root
//! compound holds a single `Schematic` compound. Block data is a varint array
//! of palette indices indexed by `x + z * Width + y * Width * Length`.

use crate::DATA_VERSION;
use crate::voxel::grid::{BlockId, IVec3, Palette, VoxelGrid};
use anyhow::{Context, Result, ensure};
use fastnbt::{ByteArray, IntArray};
use flate2::Compression;
use flate2::write::GzEncoder;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Width, Height and Length are NBT shorts, capping any axis at 32767.
const MAX_AXIS: i64 = i16::MAX as i64;

/// Schematics store one entry per cell, air included, so memory scales with the
/// bounding box rather than the block count. This caps a single file at roughly
/// a gigabyte of working memory and points at tiling instead.
const MAX_VOLUME: i64 = 400_000_000;

#[derive(Serialize)]
struct Root {
    #[serde(rename = "Schematic")]
    schematic: Schematic,
}

#[derive(Serialize)]
struct Schematic {
    #[serde(rename = "Version")]
    version: i32,
    #[serde(rename = "DataVersion")]
    data_version: i32,
    #[serde(rename = "Width")]
    width: i16,
    #[serde(rename = "Height")]
    height: i16,
    #[serde(rename = "Length")]
    length: i16,
    #[serde(rename = "Offset")]
    offset: IntArray,
    #[serde(rename = "Blocks")]
    blocks: Blocks,
    /// Props placed as display entities. Optional in the specification, so a
    /// schematic without any is byte-for-byte what it always was.
    #[serde(rename = "Entities", skip_serializing_if = "Option::is_none")]
    entities: Option<Vec<crate::output::display::Entity>>,
    #[serde(rename = "Metadata")]
    metadata: Metadata,
}

#[derive(Serialize)]
struct Blocks {
    /// Ordered, not hashed: the same input has to produce the same bytes, or
    /// the committed fixtures would differ on every run.
    #[serde(rename = "Palette")]
    palette: BTreeMap<String, i32>,
    #[serde(rename = "Data")]
    data: ByteArray,
    #[serde(rename = "BlockEntities", skip_serializing_if = "Vec::is_empty")]
    block_entities: Vec<ModBlockEntity>,
}

/// An authoritative src2mc block entity in a Sponge v3 schematic. Sponge's
/// `Pos` is relative to the schematic minimum; map-local coordinates remain in
/// `Data` so storage location and visible transform stay independent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModBlockEntity {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Pos")]
    pos: IntArray,
    #[serde(rename = "Data")]
    data: ModBlockEntityData,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
enum ModBlockEntityData {
    Anchor(AnchorData),
    PropRoot(PropRootData),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct AnchorData {
    schema_version: i32,
    campaign_id: String,
    map_id: String,
    anchor_cell: IntArray,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct PropRootData {
    schema_version: i32,
    campaign_id: String,
    map_id: String,
    stable_id: String,
    model_content_id: String,
    root_cell: IntArray,
    translation: Vec<f64>,
    rotation: Vec<f64>,
    scale: f64,
    material_ids: IntArray,
    source_model: String,
}

impl ModBlockEntity {
    pub fn anchor(
        position: IVec3,
        campaign_id: &str,
        map_id: &str,
        anchor_cell: IVec3,
    ) -> Result<Self> {
        crate::output::bundle::validate_id(campaign_id, "campaign")?;
        crate::output::bundle::validate_id(map_id, "map")?;
        Ok(Self {
            id: "src2mc:map_anchor".into(),
            pos: IntArray::new(position.to_vec()),
            data: ModBlockEntityData::Anchor(AnchorData {
                schema_version: 1,
                campaign_id: campaign_id.into(),
                map_id: map_id.into(),
                anchor_cell: IntArray::new(anchor_cell.to_vec()),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prop_root(
        position: IVec3,
        campaign_id: &str,
        map_id: &str,
        stable_id: [u8; 32],
        model_content_id: &str,
        root_cell: IVec3,
        translation: [f64; 3],
        rotation: [f64; 4],
        scale: f64,
        material_ids: Vec<u32>,
        source_model: &str,
    ) -> Result<Self> {
        crate::output::bundle::validate_id(campaign_id, "campaign")?;
        crate::output::bundle::validate_id(map_id, "map")?;
        crate::output::metadata::validate_content_id(model_content_id)?;
        ensure!(
            !source_model.is_empty(),
            "source model path must not be empty"
        );
        ensure!(!material_ids.is_empty(), "prop root has no material slots");
        for value in translation.into_iter().chain(rotation).chain([scale]) {
            ensure!(value.is_finite(), "non-finite prop-root transform");
        }
        ensure!(scale > 0.0, "prop-root scale must be positive");
        let length_sq: f64 = rotation.iter().map(|value| value * value).sum();
        ensure!(
            (length_sq - 1.0).abs() <= 1.0e-9,
            "prop-root quaternion is not unit length"
        );
        let material_ids = material_ids
            .into_iter()
            .map(i32::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self {
            id: "src2mc:prop_root".into(),
            pos: IntArray::new(position.to_vec()),
            data: ModBlockEntityData::PropRoot(PropRootData {
                schema_version: 1,
                campaign_id: campaign_id.into(),
                map_id: map_id.into(),
                stable_id: stable_id.iter().map(|byte| format!("{byte:02x}")).collect(),
                model_content_id: model_content_id.into(),
                root_cell: IntArray::new(root_cell.to_vec()),
                translation: translation.into_iter().map(canonical_zero).collect(),
                rotation: rotation.into_iter().map(canonical_zero).collect(),
                scale: canonical_zero(scale),
                material_ids: IntArray::new(material_ids),
                source_model: source_model.into(),
            }),
        })
    }

    fn expected_block(&self) -> &'static str {
        match self.data {
            ModBlockEntityData::Anchor(_) => "src2mc:map_anchor",
            ModBlockEntityData::PropRoot(_) => "src2mc:prop_root",
        }
    }
}

fn canonical_zero(value: f64) -> f64 {
    if value == 0.0 { 0.0 } else { value }
}

#[derive(Serialize)]
struct Metadata {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Author")]
    author: String,
}

/// Append `value` as an unsigned LEB128 varint.
fn write_varint(out: &mut Vec<i8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte as i8);
        if value == 0 {
            return;
        }
    }
}

/// Build the schematic NBT for the inclusive block region `min..=max` from an
/// explicit list of occupied blocks.
///
/// Taking the blocks directly, rather than probing a [`VoxelGrid`] for every
/// cell, keeps this linear in the region volume instead of costing a hash
/// lookup per cell. Blocks outside the region are ignored.
pub fn encode_blocks(
    blocks: &[(IVec3, BlockId)],
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<Vec<u8>> {
    encode_all(blocks, &[], palette, min, max, name)
}

/// As [`encode_blocks`], and with the props that belong in this region placed
/// as display entities.
pub fn encode_all(
    blocks: &[(IVec3, BlockId)],
    props: &[crate::output::display::Placement],
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<Vec<u8>> {
    encode_contents(blocks, props, &[], palette, min, max, name)
}

/// Encode the mod transport variant with authoritative anchor/root data and no
/// legacy display entities.
pub fn encode_mod(
    blocks: &[(IVec3, BlockId)],
    block_entities: &[ModBlockEntity],
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<Vec<u8>> {
    encode_contents(blocks, &[], block_entities, palette, min, max, name)
}

fn encode_contents(
    blocks: &[(IVec3, BlockId)],
    props: &[crate::output::display::Placement],
    block_entities: &[ModBlockEntity],
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<Vec<u8>> {
    let width = max[0] as i64 - min[0] as i64 + 1;
    let height = max[1] as i64 - min[1] as i64 + 1;
    let length = max[2] as i64 - min[2] as i64 + 1;

    ensure!(
        width > 0 && height > 0 && length > 0,
        "empty schematic region {min:?}..={max:?}"
    );
    ensure!(
        width <= MAX_AXIS && height <= MAX_AXIS && length <= MAX_AXIS,
        "region {width}x{height}x{length} exceeds the {MAX_AXIS}-block limit \
         of the schematic format; reduce --tile-size"
    );
    ensure!(
        width * height * length <= MAX_VOLUME,
        "region {width}x{height}x{length} is {} cells, over the {MAX_VOLUME}-cell \
         limit for one schematic; use --tile-size to split it up",
        width * height * length
    );

    // Scatter the occupied blocks into a dense array, then encode it in the
    // order the format requires: x fastest, then z, then y.
    let volume = (width * height * length) as usize;
    let mut dense = vec![0u16; volume];
    let index = |x: i64, y: i64, z: i64| (x + z * width + y * width * length) as usize;

    for (pos, block) in blocks {
        let (x, y, z) = (
            pos[0] as i64 - min[0] as i64,
            pos[1] as i64 - min[1] as i64,
            pos[2] as i64 - min[2] as i64,
        );
        if x < 0 || y < 0 || z < 0 || x >= width || y >= height || z >= length {
            continue;
        }
        dense[index(x, y, z)] = *block;
    }

    let mut data = Vec::with_capacity(volume);
    for block in dense {
        write_varint(&mut data, block as u32);
    }

    let palette_map = palette
        .names()
        .iter()
        .enumerate()
        .map(|(id, name)| (name.clone(), id as i32))
        .collect();

    let mut block_entities = block_entities.to_vec();
    block_entities.sort_by_key(|entity| entity.pos.as_ref().to_vec());
    for pair in block_entities.windows(2) {
        ensure!(
            pair[0].pos != pair[1].pos,
            "duplicate block entity position"
        );
    }
    let occupied: BTreeMap<IVec3, BlockId> = blocks.iter().copied().collect();
    for entity in &block_entities {
        let relative = entity.pos.as_ref();
        ensure!(relative.len() == 3, "invalid block-entity position");
        ensure!(
            relative.iter().enumerate().all(|(axis, value)| {
                *value >= 0 && i64::from(*value) < [width, height, length][axis]
            }),
            "block entity outside schematic region"
        );
        let absolute = [
            min[0] + relative[0],
            min[1] + relative[1],
            min[2] + relative[2],
        ];
        let block = occupied
            .get(&absolute)
            .context("block entity has no block at its position")?;
        ensure!(
            palette.name(*block) == entity.expected_block(),
            "block entity does not match its block"
        );
    }

    let root = Root {
        schematic: Schematic {
            version: 3,
            data_version: DATA_VERSION,
            width: width as i16,
            height: height as i16,
            length: length as i16,
            offset: IntArray::new(vec![min[0], min[1], min[2]]),
            blocks: Blocks {
                palette: palette_map,
                data: ByteArray::new(data),
                block_entities,
            },
            entities: (!props.is_empty()).then(|| props.iter().map(|p| p.entity(min)).collect()),
            metadata: Metadata {
                name: name.to_string(),
                author: "src2mc".to_string(),
            },
        },
    };

    fastnbt::to_bytes(&root).context("serializing schematic NBT")
}

/// Convenience wrapper that pulls the region's blocks out of a grid.
///
/// This walks the whole grid, so it suits one-off regions and tests; the tiling
/// layer buckets blocks once and calls [`encode_blocks`] instead.
pub fn encode(
    grid: &VoxelGrid,
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<Vec<u8>> {
    let blocks: Vec<(IVec3, BlockId)> = grid
        .iter()
        .filter(|(pos, _)| (0..3).all(|a| pos[a] >= min[a] && pos[a] <= max[a]))
        .collect();
    encode_blocks(&blocks, palette, min, max, name)
}

/// Write a gzipped `.schem` to `path`.
pub fn write(
    path: &Path,
    blocks: &[(IVec3, BlockId)],
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<()> {
    write_all(path, blocks, &[], palette, min, max, name)
}

/// As [`write`], with the props belonging to this region.
pub fn write_all(
    path: &Path,
    blocks: &[(IVec3, BlockId)],
    props: &[crate::output::display::Placement],
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<()> {
    let nbt = encode_all(blocks, props, palette, min, max, name)?;
    write_gzip(path, &nbt)
}

/// Write the mod transport variant as the GZIP-compressed NBT required by
/// WorldEdit's Sponge schematic reader.
pub fn write_mod(
    path: &Path,
    blocks: &[(IVec3, BlockId)],
    block_entities: &[ModBlockEntity],
    palette: &Palette,
    min: IVec3,
    max: IVec3,
    name: &str,
) -> Result<()> {
    let nbt = encode_mod(blocks, block_entities, palette, min, max, name)?;
    write_gzip(path, &nbt)
}

fn write_gzip(path: &Path, nbt: &[u8]) -> Result<()> {
    let file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder
        .write_all(&nbt)
        .with_context(|| format!("writing {}", path.display()))?;
    encoder
        .finish()
        .with_context(|| format!("finishing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastnbt::Value;
    use std::collections::HashMap;

    fn read_varints(data: &[i8], count: usize) -> Vec<u32> {
        let mut out = Vec::new();
        let mut i = 0;
        while out.len() < count && i < data.len() {
            let mut value = 0u32;
            let mut shift = 0;
            loop {
                let byte = data[i] as u8;
                i += 1;
                value |= ((byte & 0x7F) as u32) << shift;
                if byte & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            out.push(value);
        }
        out
    }

    #[test]
    fn varints_round_trip() {
        for value in [0u32, 1, 127, 128, 255, 300, 16383, 16384, 1_000_000] {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            assert_eq!(read_varints(&buf, 1), vec![value], "value {value}");
        }
    }

    #[test]
    fn small_values_use_a_single_byte() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 5);
        assert_eq!(buf.len(), 1);
    }

    /// Decode a written schematic and check every block lands where it started.
    #[test]
    fn encodes_blocks_at_the_expected_indices() {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let glass = palette.intern("minecraft:glass");

        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], stone);
        grid.set([2, 1, 3], glass);

        let nbt = encode(&grid, &palette, [0, 0, 0], [3, 2, 4], "test").unwrap();
        let root: HashMap<String, Value> = fastnbt::from_bytes(&nbt).unwrap();
        let Value::Compound(schematic) = &root["Schematic"] else {
            panic!("missing Schematic compound")
        };

        assert_eq!(schematic["Version"], Value::Int(3));
        assert_eq!(schematic["DataVersion"], Value::Int(DATA_VERSION));
        assert_eq!(schematic["Width"], Value::Short(4));
        assert_eq!(schematic["Height"], Value::Short(3));
        assert_eq!(schematic["Length"], Value::Short(5));

        let Value::Compound(blocks) = &schematic["Blocks"] else {
            panic!("missing Blocks")
        };
        let Value::ByteArray(data) = &blocks["Data"] else {
            panic!("missing Data")
        };

        let (width, length) = (4usize, 5usize);
        let ids = read_varints(data, width * length * 3);
        let index = |x: usize, y: usize, z: usize| x + z * width + y * width * length;

        assert_eq!(ids[index(0, 0, 0)], stone as u32);
        assert_eq!(ids[index(2, 1, 3)], glass as u32);
        assert_eq!(ids[index(1, 1, 1)], 0, "everything else is air");
    }

    #[test]
    fn palette_and_offset_are_written() {
        let mut palette = Palette::new();
        palette.intern("minecraft:stone");
        let grid = VoxelGrid::new();

        let nbt = encode(&grid, &palette, [-5, 10, 7], [-5, 10, 7], "tile").unwrap();
        let root: HashMap<String, Value> = fastnbt::from_bytes(&nbt).unwrap();
        let Value::Compound(schematic) = &root["Schematic"] else {
            panic!()
        };

        assert_eq!(
            schematic["Offset"],
            Value::IntArray(IntArray::new(vec![-5, 10, 7]))
        );

        let Value::Compound(blocks) = &schematic["Blocks"] else {
            panic!()
        };
        let Value::Compound(written) = &blocks["Palette"] else {
            panic!()
        };
        assert_eq!(written["minecraft:air"], Value::Int(0));
        assert_eq!(written["minecraft:stone"], Value::Int(1));
    }

    #[test]
    fn rejects_regions_beyond_the_format_limit() {
        let palette = Palette::new();
        let grid = VoxelGrid::new();
        let err = encode(&grid, &palette, [0, 0, 0], [40_000, 0, 0], "big").unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{err}");
    }

    #[test]
    fn rejects_inverted_regions() {
        let palette = Palette::new();
        let grid = VoxelGrid::new();
        assert!(encode(&grid, &palette, [5, 0, 0], [0, 0, 0], "bad").is_err());
    }

    #[test]
    fn blocks_outside_the_region_are_ignored() {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let blocks = [
            ([0, 0, 0], stone),
            ([-1, 0, 0], stone),
            ([100, 0, 0], stone),
            ([0, 0, 50], stone),
        ];

        let nbt = encode_blocks(&blocks, &palette, [0, 0, 0], [1, 1, 1], "clip").unwrap();
        let root: HashMap<String, Value> = fastnbt::from_bytes(&nbt).unwrap();
        let Value::Compound(schematic) = &root["Schematic"] else {
            panic!()
        };
        let Value::Compound(b) = &schematic["Blocks"] else {
            panic!()
        };
        let Value::ByteArray(data) = &b["Data"] else {
            panic!()
        };

        let ids = read_varints(data, 8);
        assert_eq!(ids[0], stone as u32, "the in-region block survives");
        assert_eq!(ids.iter().filter(|id| **id != 0).count(), 1);
    }

    #[test]
    fn written_files_are_gzipped() {
        let dir = std::env::temp_dir().join("src2mc-schem-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.schem");

        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        write(
            &path,
            &[([0, 0, 0], stone)],
            &palette,
            [0, 0, 0],
            [1, 1, 1],
            "t",
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..2], &[0x1f, 0x8b], "gzip magic");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn mod_block_entities_use_sponge_v3_envelopes_and_canonical_data() {
        let mut palette = Palette::new();
        let anchor_block = palette.intern("src2mc:map_anchor");
        let root_block = palette.intern("src2mc:prop_root");
        let entities = vec![
            ModBlockEntity::prop_root(
                [2, 0, 0],
                "hl2",
                "d1_01",
                [0xab; 32],
                &"c".repeat(64),
                [12, 20, 30],
                [12.25, -0.0, 30.5],
                [0.0, 0.0, 0.0, 1.0],
                1.0,
                vec![4, 7],
                "models/props/barrel.mdl",
            )
            .unwrap(),
            ModBlockEntity::anchor([0, 0, 0], "hl2", "d1_01", [10, 20, 30]).unwrap(),
        ];
        let nbt = encode_mod(
            &[([10, 20, 30], anchor_block), ([12, 20, 30], root_block)],
            &entities,
            &palette,
            [10, 20, 30],
            [12, 20, 30],
            "d1_01",
        )
        .unwrap();
        let root: HashMap<String, Value> = fastnbt::from_bytes(&nbt).unwrap();
        let Value::Compound(schematic) = &root["Schematic"] else {
            panic!()
        };
        let Value::Compound(blocks) = &schematic["Blocks"] else {
            panic!()
        };
        let Value::List(written) = &blocks["BlockEntities"] else {
            panic!()
        };
        assert_eq!(written.len(), 2);

        let Value::Compound(anchor) = &written[0] else {
            panic!()
        };
        assert_eq!(anchor["Id"], Value::String("src2mc:map_anchor".into()));
        assert_eq!(anchor["Pos"], Value::IntArray(IntArray::new(vec![0, 0, 0])));
        let Value::Compound(anchor_data) = &anchor["Data"] else {
            panic!()
        };
        assert_eq!(anchor_data["campaign_id"], Value::String("hl2".into()));
        assert_eq!(
            anchor_data["anchor_cell"],
            Value::IntArray(IntArray::new(vec![10, 20, 30]))
        );

        let Value::Compound(prop) = &written[1] else {
            panic!()
        };
        assert_eq!(prop["Pos"], Value::IntArray(IntArray::new(vec![2, 0, 0])));
        let Value::Compound(prop_data) = &prop["Data"] else {
            panic!()
        };
        assert_eq!(prop_data["stable_id"], Value::String("ab".repeat(32)));
        let Value::List(translation) = &prop_data["translation"] else {
            panic!()
        };
        assert_eq!(translation[1], Value::Double(0.0));
    }

    #[test]
    fn mod_block_entities_are_sorted_and_must_be_unique_and_in_bounds() {
        let mut palette = Palette::new();
        palette.intern("src2mc:map_anchor");
        let anchor = ModBlockEntity::anchor([0, 0, 0], "hl2", "map", [0, 0, 0]).unwrap();
        assert!(
            encode_mod(
                &[],
                &[anchor.clone(), anchor],
                &palette,
                [0, 0, 0],
                [0, 0, 0],
                "x"
            )
            .is_err()
        );
        let outside = ModBlockEntity::anchor([1, 0, 0], "hl2", "map", [0, 0, 0]).unwrap();
        assert!(encode_mod(&[], &[outside], &palette, [0, 0, 0], [0, 0, 0], "x").is_err());
    }
}
