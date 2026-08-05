//! A sparse voxel grid of Minecraft blocks.
//!
//! Maps are far too large to hold densely: a big E:Z2 map spans roughly
//! 1950 x 900 x 1750 blocks, a bounding box of some three billion cells, of
//! which only the surfaces are ever occupied. Storage is therefore chunked into
//! 16³ sections that are only allocated once something lands in them.

use std::collections::HashMap;

/// Index into a [`Palette`]. 0 is always air.
pub type BlockId = u16;
pub const AIR: BlockId = 0;

/// Integer block coordinates.
pub type IVec3 = [i32; 3];

const SECTION_BITS: i32 = 4;
const SECTION_SIZE: i32 = 1 << SECTION_BITS;
const SECTION_MASK: i32 = SECTION_SIZE - 1;
const SECTION_VOLUME: usize = (SECTION_SIZE * SECTION_SIZE * SECTION_SIZE) as usize;

/// Maps block state strings to compact ids.
#[derive(Debug, Clone)]
pub struct Palette {
    ids: HashMap<String, BlockId>,
    names: Vec<String>,
}

impl Default for Palette {
    fn default() -> Self {
        Palette::new()
    }
}

impl Palette {
    pub fn new() -> Palette {
        let mut palette = Palette {
            ids: HashMap::new(),
            names: Vec::new(),
        };
        // Air must be id 0 so an unset voxel reads as empty.
        palette.intern("minecraft:air");
        palette
    }

    /// Id for a block state, adding it if new.
    pub fn intern(&mut self, name: &str) -> BlockId {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = self.names.len() as BlockId;
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        id
    }

    pub fn name(&self, id: BlockId) -> &str {
        self.names.get(id as usize).map_or("minecraft:air", |s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }
}

fn section_of(pos: IVec3) -> IVec3 {
    [
        pos[0] >> SECTION_BITS,
        pos[1] >> SECTION_BITS,
        pos[2] >> SECTION_BITS,
    ]
}

fn index_in_section(pos: IVec3) -> usize {
    let x = (pos[0] & SECTION_MASK) as usize;
    let y = (pos[1] & SECTION_MASK) as usize;
    let z = (pos[2] & SECTION_MASK) as usize;
    (y << (SECTION_BITS * 2)) | (z << SECTION_BITS) | x
}

#[derive(Debug, Clone, Default)]
pub struct VoxelGrid {
    sections: HashMap<IVec3, Vec<BlockId>>,
}

impl VoxelGrid {
    pub fn new() -> VoxelGrid {
        VoxelGrid::default()
    }

    pub fn set(&mut self, pos: IVec3, block: BlockId) {
        if block == AIR {
            // Clearing is only meaningful in an already-allocated section.
            if let Some(section) = self.sections.get_mut(&section_of(pos)) {
                section[index_in_section(pos)] = AIR;
            }
            return;
        }
        self.sections
            .entry(section_of(pos))
            .or_insert_with(|| vec![AIR; SECTION_VOLUME])[index_in_section(pos)] = block;
    }

    pub fn get(&self, pos: IVec3) -> BlockId {
        self.sections
            .get(&section_of(pos))
            .map_or(AIR, |section| section[index_in_section(pos)])
    }

    pub fn is_solid(&self, pos: IVec3) -> bool {
        self.get(pos) != AIR
    }

    /// Number of non-air voxels.
    pub fn count(&self) -> usize {
        self.sections
            .values()
            .map(|s| s.iter().filter(|b| **b != AIR).count())
            .sum()
    }

    pub fn sections(&self) -> usize {
        self.sections.len()
    }

    /// Inclusive bounds of the occupied voxels, or `None` when empty.
    pub fn bounds(&self) -> Option<(IVec3, IVec3)> {
        let mut min = [i32::MAX; 3];
        let mut max = [i32::MIN; 3];
        let mut found = false;

        for (section, blocks) in &self.sections {
            for (index, block) in blocks.iter().enumerate() {
                if *block == AIR {
                    continue;
                }
                found = true;
                let pos = [
                    (section[0] << SECTION_BITS) | (index & SECTION_MASK as usize) as i32,
                    (section[1] << SECTION_BITS) | (index >> (SECTION_BITS * 2)) as i32,
                    (section[2] << SECTION_BITS)
                        | ((index >> SECTION_BITS) & SECTION_MASK as usize) as i32,
                ];
                for axis in 0..3 {
                    min[axis] = min[axis].min(pos[axis]);
                    max[axis] = max[axis].max(pos[axis]);
                }
            }
        }
        found.then_some((min, max))
    }

    /// Every occupied voxel, in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (IVec3, BlockId)> + '_ {
        self.sections.iter().flat_map(|(section, blocks)| {
            blocks.iter().enumerate().filter_map(move |(index, block)| {
                (*block != AIR).then(|| {
                    (
                        [
                            (section[0] << SECTION_BITS) | (index & SECTION_MASK as usize) as i32,
                            (section[1] << SECTION_BITS) | (index >> (SECTION_BITS * 2)) as i32,
                            (section[2] << SECTION_BITS)
                                | ((index >> SECTION_BITS) & SECTION_MASK as usize) as i32,
                        ],
                        *block,
                    )
                })
            })
        })
    }

    /// Merge `other` into `self`; `other`'s blocks win where both are set.
    pub fn merge(&mut self, other: VoxelGrid) {
        for (key, blocks) in other.sections {
            match self.sections.get_mut(&key) {
                Some(existing) => {
                    for (slot, block) in existing.iter_mut().zip(blocks) {
                        if block != AIR {
                            *slot = block;
                        }
                    }
                }
                None => {
                    self.sections.insert(key, blocks);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_is_always_id_zero() {
        let mut palette = Palette::new();
        assert_eq!(palette.intern("minecraft:air"), AIR);
        assert_eq!(palette.name(AIR), "minecraft:air");
    }

    #[test]
    fn palette_interning_is_stable() {
        let mut palette = Palette::new();
        let stone = palette.intern("minecraft:stone");
        let glass = palette.intern("minecraft:glass");
        assert_ne!(stone, glass);
        assert_eq!(palette.intern("minecraft:stone"), stone);
        assert_eq!(palette.name(stone), "minecraft:stone");
        assert_eq!(palette.len(), 3);
    }

    #[test]
    fn round_trips_positions_including_negatives() {
        let mut grid = VoxelGrid::new();
        let positions = [[0, 0, 0], [1, 2, 3], [-1, -1, -1], [-17, 300, -4096], [15, 15, 15]];
        for (i, pos) in positions.iter().enumerate() {
            grid.set(*pos, i as BlockId + 1);
        }
        for (i, pos) in positions.iter().enumerate() {
            assert_eq!(grid.get(*pos), i as BlockId + 1, "at {pos:?}");
        }
        assert_eq!(grid.count(), positions.len());
    }

    /// Section indexing must not alias distinct positions onto one slot.
    #[test]
    fn every_position_in_a_section_is_distinct() {
        let mut seen = std::collections::HashSet::new();
        for x in -16..16 {
            for y in -16..16 {
                for z in -16..16 {
                    seen.insert((section_of([x, y, z]), index_in_section([x, y, z])));
                }
            }
        }
        assert_eq!(seen.len(), 32 * 32 * 32);
    }

    #[test]
    fn iter_reports_the_positions_that_were_set() {
        let mut grid = VoxelGrid::new();
        let positions = [[0, 0, 0], [-5, 20, 7], [100, -30, 62]];
        for pos in &positions {
            grid.set(*pos, 1);
        }
        let mut found: Vec<IVec3> = grid.iter().map(|(pos, _)| pos).collect();
        found.sort();
        let mut expected = positions.to_vec();
        expected.sort();
        assert_eq!(found, expected);
    }

    #[test]
    fn bounds_cover_the_occupied_voxels() {
        let mut grid = VoxelGrid::new();
        grid.set([-4, 7, 30], 1);
        grid.set([90, -12, 3], 1);
        let (min, max) = grid.bounds().unwrap();
        assert_eq!(min, [-4, -12, 3]);
        assert_eq!(max, [90, 7, 30]);
    }

    #[test]
    fn empty_grid_has_no_bounds() {
        assert!(VoxelGrid::new().bounds().is_none());
        assert_eq!(VoxelGrid::new().count(), 0);
    }

    #[test]
    fn setting_air_clears_a_voxel() {
        let mut grid = VoxelGrid::new();
        grid.set([3, 3, 3], 5);
        grid.set([3, 3, 3], AIR);
        assert_eq!(grid.count(), 0);
        assert!(!grid.is_solid([3, 3, 3]));
    }

    #[test]
    fn merge_combines_and_prefers_the_incoming_grid() {
        let mut a = VoxelGrid::new();
        a.set([0, 0, 0], 1);
        a.set([1, 0, 0], 1);

        let mut b = VoxelGrid::new();
        b.set([1, 0, 0], 2);
        b.set([500, 500, 500], 3);

        a.merge(b);
        assert_eq!(a.get([0, 0, 0]), 1);
        assert_eq!(a.get([1, 0, 0]), 2);
        assert_eq!(a.get([500, 500, 500]), 3);
        assert_eq!(a.count(), 3);
    }
}
