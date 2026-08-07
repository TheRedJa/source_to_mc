//! The Minecraft blocks the auto-palette is allowed to choose from.
//!
//! This is a curated list, not every block in the game. Auto-matching on colour
//! alone will happily suggest a block that falls, melts, burns, drops as an
//! item or shows a different texture on every face, none of which you want
//! several thousand times across a converted map. Everything here is a full
//! opaque cube with the same texture on all sides, stays where it is put, and
//! does nothing on its own.
//!
//! Deliberately excluded, with the reason:
//!
//! - Gravity blocks (sand, gravel, concrete powder, anvils) — they fall.
//! - Directional textures (logs, basalt, bone blocks, glazed terracotta,
//!   grass blocks, nylium, froglights) — a wall of them looks striped.
//! - Transparent or non-cube blocks (glass, ice, leaves, slabs, slime).
//! - Blocks that act: magma and campfires hurt, sponge soaks, TNT, note blocks,
//!   anything redstone reacts to, and unwaxed copper, which changes colour.
//!
//! The colours are hand-authored approximations of each texture's average, in
//! sRGB. They are good enough to order candidates by eye, not a substitute for
//! sampling the real texture atlas.

use crate::palette::color::{Oklab, oklab_from_srgb};
use std::sync::OnceLock;

pub const SET_STONE: u16 = 1 << 0;
pub const SET_CONCRETE: u16 = 1 << 1;
pub const SET_WOOL: u16 = 1 << 2;
pub const SET_TERRACOTTA: u16 = 1 << 3;
pub const SET_WOOD: u16 = 1 << 4;
pub const SET_NATURAL: u16 = 1 << 5;
pub const SET_METAL: u16 = 1 << 6;
pub const SET_NETHER: u16 = 1 << 7;
/// Strongly tinted blocks with no real-world counterpart. Kept out of the
/// other sets because they win colour matches they should lose: prismarine is
/// only a little more teal than a cold grey, but a wall of it is unmistakably
/// sea-green, and Half-Life 2's Citadel metal is exactly that cold grey.
pub const SET_EXOTIC: u16 = 1 << 8;
pub const SET_ALL: u16 = 0xffff;

/// The named sets `palette_set` accepts, besides `full`.
pub const SET_NAMES: &[(&str, u16)] = &[
    ("stone", SET_STONE),
    ("concrete", SET_CONCRETE),
    ("wool", SET_WOOL),
    ("terracotta", SET_TERRACOTTA),
    ("wood", SET_WOOD),
    ("natural", SET_NATURAL),
    ("metal", SET_METAL),
    ("nether", SET_NETHER),
    ("exotic", SET_EXOTIC),
];

/// Parse a `palette_set` value: `full`, or a comma-separated list of set names.
pub fn parse_set(spec: &str) -> Result<u16, String> {
    let spec = spec.trim();
    if spec.eq_ignore_ascii_case("full") || spec.is_empty() {
        return Ok(SET_ALL);
    }
    let mut mask = 0;
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match SET_NAMES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(part))
        {
            Some((_, bit)) => mask |= bit,
            None => {
                let known: Vec<&str> = SET_NAMES.iter().map(|(n, _)| *n).collect();
                return Err(format!(
                    "unknown palette set `{part}`; expected `full` or one of {}",
                    known.join(", ")
                ));
            }
        }
    }
    if mask == 0 {
        return Err(format!("palette set `{spec}` selects no blocks"));
    }
    Ok(mask)
}

#[derive(Debug, Clone, Copy)]
pub struct Block {
    /// Full block id, including the namespace.
    pub name: &'static str,
    /// Approximate average texture colour, sRGB.
    pub rgb: [u8; 3],
    pub sets: u16,
    /// Slab and stair variants, where vanilla has them. Sub-block shape
    /// fitting can only use a block that has them; the rest stay full cubes.
    pub shaped: Option<(&'static str, &'static str)>,
}

const S: u16 = SET_STONE;
const C: u16 = SET_CONCRETE;
const W: u16 = SET_WOOL;
const T: u16 = SET_TERRACOTTA;
const D: u16 = SET_WOOD;
const N: u16 = SET_NATURAL;
const M: u16 = SET_METAL;
const H: u16 = SET_NETHER;
const X: u16 = SET_EXOTIC;

/// Every block the auto-palette may pick.
pub const BLOCKS: &[Block] = &[
    // Stone and its worked forms. The backbone of any Source-map conversion,
    // because most of what Source builds out of is grey.
    Block {
        name: "minecraft:stone",
        rgb: [125, 125, 125],
        sets: S | N,
        shaped: Some(("minecraft:stone_slab", "minecraft:stone_stairs")),
    },
    Block {
        name: "minecraft:cobblestone",
        rgb: [127, 127, 127],
        sets: S | N,
        shaped: Some(("minecraft:cobblestone_slab", "minecraft:cobblestone_stairs")),
    },
    Block {
        name: "minecraft:mossy_cobblestone",
        rgb: [118, 126, 109],
        sets: S | N,
        shaped: Some((
            "minecraft:mossy_cobblestone_slab",
            "minecraft:mossy_cobblestone_stairs",
        )),
    },
    Block {
        name: "minecraft:smooth_stone",
        rgb: [158, 158, 158],
        sets: S,
        shaped: None,
    },
    Block {
        name: "minecraft:stone_bricks",
        rgb: [122, 122, 122],
        sets: S,
        shaped: Some(("minecraft:stone_brick_slab", "minecraft:stone_brick_stairs")),
    },
    Block {
        name: "minecraft:cracked_stone_bricks",
        rgb: [118, 117, 117],
        sets: S,
        shaped: None,
    },
    Block {
        name: "minecraft:mossy_stone_bricks",
        rgb: [116, 121, 105],
        sets: S,
        shaped: Some((
            "minecraft:mossy_stone_brick_slab",
            "minecraft:mossy_stone_brick_stairs",
        )),
    },
    Block {
        name: "minecraft:chiseled_stone_bricks",
        rgb: [119, 119, 119],
        sets: S,
        shaped: None,
    },
    Block {
        name: "minecraft:andesite",
        rgb: [136, 136, 137],
        sets: S | N,
        shaped: Some(("minecraft:andesite_slab", "minecraft:andesite_stairs")),
    },
    Block {
        name: "minecraft:polished_andesite",
        rgb: [132, 134, 133],
        sets: S,
        shaped: Some((
            "minecraft:polished_andesite_slab",
            "minecraft:polished_andesite_stairs",
        )),
    },
    Block {
        name: "minecraft:diorite",
        rgb: [188, 188, 190],
        sets: S | N,
        shaped: Some(("minecraft:diorite_slab", "minecraft:diorite_stairs")),
    },
    Block {
        name: "minecraft:polished_diorite",
        rgb: [192, 193, 194],
        sets: S,
        shaped: Some((
            "minecraft:polished_diorite_slab",
            "minecraft:polished_diorite_stairs",
        )),
    },
    Block {
        name: "minecraft:granite",
        rgb: [149, 103, 86],
        sets: S | N,
        shaped: Some(("minecraft:granite_slab", "minecraft:granite_stairs")),
    },
    Block {
        name: "minecraft:polished_granite",
        rgb: [154, 106, 88],
        sets: S,
        shaped: Some((
            "minecraft:polished_granite_slab",
            "minecraft:polished_granite_stairs",
        )),
    },
    Block {
        name: "minecraft:tuff",
        rgb: [108, 109, 102],
        sets: S | N,
        shaped: Some(("minecraft:tuff_slab", "minecraft:tuff_stairs")),
    },
    Block {
        name: "minecraft:polished_tuff",
        rgb: [99, 100, 94],
        sets: S,
        shaped: Some((
            "minecraft:polished_tuff_slab",
            "minecraft:polished_tuff_stairs",
        )),
    },
    Block {
        name: "minecraft:tuff_bricks",
        rgb: [93, 94, 88],
        sets: S,
        shaped: Some(("minecraft:tuff_brick_slab", "minecraft:tuff_brick_stairs")),
    },
    Block {
        name: "minecraft:calcite",
        rgb: [223, 224, 221],
        sets: S | N,
        shaped: None,
    },
    Block {
        name: "minecraft:dripstone_block",
        rgb: [134, 107, 90],
        sets: S | N,
        shaped: None,
    },
    // Deepslate: the darker greys Source's shadowed concrete tends to average to.
    Block {
        name: "minecraft:deepslate",
        rgb: [80, 80, 84],
        sets: S | N,
        shaped: None,
    },
    Block {
        name: "minecraft:cobbled_deepslate",
        rgb: [77, 77, 80],
        sets: S | N,
        shaped: Some((
            "minecraft:cobbled_deepslate_slab",
            "minecraft:cobbled_deepslate_stairs",
        )),
    },
    Block {
        name: "minecraft:polished_deepslate",
        rgb: [72, 72, 74],
        sets: S,
        shaped: Some((
            "minecraft:polished_deepslate_slab",
            "minecraft:polished_deepslate_stairs",
        )),
    },
    Block {
        name: "minecraft:deepslate_bricks",
        rgb: [71, 71, 73],
        sets: S,
        shaped: Some((
            "minecraft:deepslate_brick_slab",
            "minecraft:deepslate_brick_stairs",
        )),
    },
    Block {
        name: "minecraft:deepslate_tiles",
        rgb: [55, 55, 57],
        sets: S,
        shaped: Some((
            "minecraft:deepslate_tile_slab",
            "minecraft:deepslate_tile_stairs",
        )),
    },
    Block {
        name: "minecraft:blackstone",
        rgb: [42, 36, 42],
        sets: S | H,
        shaped: Some(("minecraft:blackstone_slab", "minecraft:blackstone_stairs")),
    },
    Block {
        name: "minecraft:polished_blackstone",
        rgb: [53, 50, 55],
        sets: S | H,
        shaped: Some((
            "minecraft:polished_blackstone_slab",
            "minecraft:polished_blackstone_stairs",
        )),
    },
    Block {
        name: "minecraft:polished_blackstone_bricks",
        rgb: [48, 45, 50],
        sets: S | H,
        shaped: Some((
            "minecraft:polished_blackstone_brick_slab",
            "minecraft:polished_blackstone_brick_stairs",
        )),
    },
    Block {
        name: "minecraft:gilded_blackstone",
        rgb: [55, 44, 42],
        sets: S | H,
        shaped: None,
    },
    Block {
        name: "minecraft:obsidian",
        rgb: [21, 18, 30],
        sets: S,
        shaped: None,
    },
    Block {
        name: "minecraft:crying_obsidian",
        rgb: [32, 10, 60],
        sets: S,
        shaped: None,
    },
    // Brick, sandstone, quartz and the other worked stones.
    Block {
        name: "minecraft:bricks",
        rgb: [150, 97, 83],
        sets: S,
        shaped: Some(("minecraft:brick_slab", "minecraft:brick_stairs")),
    },
    Block {
        name: "minecraft:mud_bricks",
        rgb: [137, 109, 88],
        sets: S | N,
        shaped: Some(("minecraft:mud_brick_slab", "minecraft:mud_brick_stairs")),
    },
    Block {
        name: "minecraft:packed_mud",
        rgb: [143, 108, 82],
        sets: S | N,
        shaped: None,
    },
    Block {
        name: "minecraft:sandstone",
        rgb: [219, 207, 163],
        sets: S | N,
        shaped: Some(("minecraft:sandstone_slab", "minecraft:sandstone_stairs")),
    },
    Block {
        name: "minecraft:smooth_sandstone",
        rgb: [224, 213, 171],
        sets: S,
        shaped: Some((
            "minecraft:smooth_sandstone_slab",
            "minecraft:smooth_sandstone_stairs",
        )),
    },
    Block {
        name: "minecraft:chiseled_sandstone",
        rgb: [216, 205, 161],
        sets: S,
        shaped: None,
    },
    Block {
        name: "minecraft:red_sandstone",
        rgb: [186, 99, 29],
        sets: S | N,
        shaped: Some((
            "minecraft:red_sandstone_slab",
            "minecraft:red_sandstone_stairs",
        )),
    },
    Block {
        name: "minecraft:smooth_red_sandstone",
        rgb: [191, 103, 32],
        sets: S,
        shaped: Some((
            "minecraft:smooth_red_sandstone_slab",
            "minecraft:smooth_red_sandstone_stairs",
        )),
    },
    Block {
        name: "minecraft:quartz_block",
        rgb: [236, 233, 226],
        sets: S,
        shaped: Some(("minecraft:quartz_slab", "minecraft:quartz_stairs")),
    },
    Block {
        name: "minecraft:quartz_bricks",
        rgb: [234, 229, 220],
        sets: S,
        shaped: None,
    },
    Block {
        name: "minecraft:prismarine",
        rgb: [99, 156, 151],
        sets: X,
        shaped: Some(("minecraft:prismarine_slab", "minecraft:prismarine_stairs")),
    },
    Block {
        name: "minecraft:prismarine_bricks",
        rgb: [99, 171, 158],
        sets: X,
        shaped: Some((
            "minecraft:prismarine_brick_slab",
            "minecraft:prismarine_brick_stairs",
        )),
    },
    Block {
        name: "minecraft:dark_prismarine",
        rgb: [51, 91, 75],
        sets: X,
        shaped: Some((
            "minecraft:dark_prismarine_slab",
            "minecraft:dark_prismarine_stairs",
        )),
    },
    Block {
        name: "minecraft:end_stone",
        rgb: [221, 223, 159],
        sets: X,
        shaped: None,
    },
    Block {
        name: "minecraft:end_stone_bricks",
        rgb: [218, 224, 162],
        sets: X,
        shaped: Some((
            "minecraft:end_stone_brick_slab",
            "minecraft:end_stone_brick_stairs",
        )),
    },
    Block {
        name: "minecraft:purpur_block",
        rgb: [170, 126, 170],
        sets: X,
        shaped: Some(("minecraft:purpur_slab", "minecraft:purpur_stairs")),
    },
    // Nether stones, useful for the reds and near-blacks nothing else covers.
    Block {
        name: "minecraft:netherrack",
        rgb: [97, 38, 38],
        sets: H,
        shaped: None,
    },
    Block {
        name: "minecraft:nether_bricks",
        rgb: [44, 22, 26],
        sets: H,
        shaped: Some((
            "minecraft:nether_brick_slab",
            "minecraft:nether_brick_stairs",
        )),
    },
    Block {
        name: "minecraft:red_nether_bricks",
        rgb: [70, 0, 3],
        sets: H,
        shaped: Some((
            "minecraft:red_nether_brick_slab",
            "minecraft:red_nether_brick_stairs",
        )),
    },
    Block {
        name: "minecraft:nether_wart_block",
        rgb: [114, 1, 1],
        sets: H,
        shaped: None,
    },
    Block {
        name: "minecraft:warped_wart_block",
        rgb: [20, 110, 110],
        sets: H,
        shaped: None,
    },
    Block {
        name: "minecraft:soul_soil",
        rgb: [75, 58, 47],
        sets: H | N,
        shaped: None,
    },
    Block {
        name: "minecraft:shroomlight",
        rgb: [240, 146, 70],
        sets: H,
        shaped: None,
    },
    // The concrete set: the cleanest, most saturated colours in the game, and
    // what most people reach for when recreating painted or panelled surfaces.
    Block {
        name: "minecraft:white_concrete",
        rgb: [207, 213, 214],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:light_gray_concrete",
        rgb: [125, 125, 115],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:gray_concrete",
        rgb: [55, 58, 62],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:black_concrete",
        rgb: [8, 10, 15],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:brown_concrete",
        rgb: [96, 60, 32],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:red_concrete",
        rgb: [142, 33, 33],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:orange_concrete",
        rgb: [224, 97, 0],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:yellow_concrete",
        rgb: [241, 175, 21],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:lime_concrete",
        rgb: [94, 169, 24],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:green_concrete",
        rgb: [73, 91, 36],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:cyan_concrete",
        rgb: [21, 119, 136],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:light_blue_concrete",
        rgb: [36, 137, 199],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:blue_concrete",
        rgb: [45, 47, 143],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:purple_concrete",
        rgb: [100, 32, 156],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:magenta_concrete",
        rgb: [169, 48, 159],
        sets: C,
        shaped: None,
    },
    Block {
        name: "minecraft:pink_concrete",
        rgb: [214, 101, 143],
        sets: C,
        shaped: None,
    },
    // Terracotta: muted and slightly noisy, which reads closer to weathered
    // Source textures than concrete's flat fields do.
    Block {
        name: "minecraft:terracotta",
        rgb: [152, 94, 67],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:white_terracotta",
        rgb: [209, 178, 161],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:light_gray_terracotta",
        rgb: [135, 107, 98],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:gray_terracotta",
        rgb: [57, 42, 35],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:black_terracotta",
        rgb: [37, 22, 16],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:brown_terracotta",
        rgb: [77, 51, 35],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:red_terracotta",
        rgb: [143, 61, 46],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:orange_terracotta",
        rgb: [161, 83, 37],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:yellow_terracotta",
        rgb: [186, 133, 35],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:lime_terracotta",
        rgb: [103, 117, 52],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:green_terracotta",
        rgb: [76, 83, 42],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:cyan_terracotta",
        rgb: [86, 91, 91],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:light_blue_terracotta",
        rgb: [113, 108, 137],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:blue_terracotta",
        rgb: [74, 59, 91],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:purple_terracotta",
        rgb: [118, 70, 86],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:magenta_terracotta",
        rgb: [149, 88, 108],
        sets: T,
        shaped: None,
    },
    Block {
        name: "minecraft:pink_terracotta",
        rgb: [161, 78, 78],
        sets: T,
        shaped: None,
    },
    // Wool: flat, matte and flammable, so off by default in the mixed sets but
    // available when you want a poster-colour look.
    Block {
        name: "minecraft:white_wool",
        rgb: [233, 236, 236],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:light_gray_wool",
        rgb: [142, 142, 134],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:gray_wool",
        rgb: [62, 68, 71],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:black_wool",
        rgb: [20, 21, 25],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:brown_wool",
        rgb: [114, 71, 40],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:red_wool",
        rgb: [160, 39, 34],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:orange_wool",
        rgb: [240, 118, 19],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:yellow_wool",
        rgb: [248, 198, 39],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:lime_wool",
        rgb: [112, 185, 25],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:green_wool",
        rgb: [84, 109, 27],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:cyan_wool",
        rgb: [21, 137, 145],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:light_blue_wool",
        rgb: [58, 175, 217],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:blue_wool",
        rgb: [53, 57, 157],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:purple_wool",
        rgb: [121, 42, 172],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:magenta_wool",
        rgb: [189, 68, 179],
        sets: W,
        shaped: None,
    },
    Block {
        name: "minecraft:pink_wool",
        rgb: [237, 141, 172],
        sets: W,
        shaped: None,
    },
    // Planks and all-sided wood. Logs are excluded because their end grain
    // shows on two faces.
    Block {
        name: "minecraft:oak_planks",
        rgb: [162, 130, 78],
        sets: D,
        shaped: Some(("minecraft:oak_slab", "minecraft:oak_stairs")),
    },
    Block {
        name: "minecraft:spruce_planks",
        rgb: [114, 84, 48],
        sets: D,
        shaped: Some(("minecraft:spruce_slab", "minecraft:spruce_stairs")),
    },
    Block {
        name: "minecraft:birch_planks",
        rgb: [192, 175, 121],
        sets: D,
        shaped: Some(("minecraft:birch_slab", "minecraft:birch_stairs")),
    },
    Block {
        name: "minecraft:jungle_planks",
        rgb: [160, 115, 80],
        sets: D,
        shaped: Some(("minecraft:jungle_slab", "minecraft:jungle_stairs")),
    },
    Block {
        name: "minecraft:acacia_planks",
        rgb: [168, 90, 50],
        sets: D,
        shaped: Some(("minecraft:acacia_slab", "minecraft:acacia_stairs")),
    },
    Block {
        name: "minecraft:dark_oak_planks",
        rgb: [66, 43, 20],
        sets: D,
        shaped: Some(("minecraft:dark_oak_slab", "minecraft:dark_oak_stairs")),
    },
    Block {
        name: "minecraft:mangrove_planks",
        rgb: [117, 54, 48],
        sets: D,
        shaped: Some(("minecraft:mangrove_slab", "minecraft:mangrove_stairs")),
    },
    Block {
        name: "minecraft:cherry_planks",
        rgb: [226, 177, 168],
        sets: D,
        shaped: Some(("minecraft:cherry_slab", "minecraft:cherry_stairs")),
    },
    Block {
        name: "minecraft:bamboo_planks",
        rgb: [193, 154, 79],
        sets: D,
        shaped: Some(("minecraft:bamboo_slab", "minecraft:bamboo_stairs")),
    },
    Block {
        name: "minecraft:crimson_planks",
        rgb: [101, 48, 70],
        sets: D | H,
        shaped: Some(("minecraft:crimson_slab", "minecraft:crimson_stairs")),
    },
    Block {
        name: "minecraft:warped_planks",
        rgb: [43, 104, 99],
        sets: D | H,
        shaped: Some(("minecraft:warped_slab", "minecraft:warped_stairs")),
    },
    Block {
        name: "minecraft:oak_wood",
        rgb: [109, 85, 50],
        sets: D,
        shaped: None,
    },
    Block {
        name: "minecraft:spruce_wood",
        rgb: [58, 38, 20],
        sets: D,
        shaped: None,
    },
    Block {
        name: "minecraft:birch_wood",
        rgb: [216, 214, 208],
        sets: D,
        shaped: None,
    },
    Block {
        name: "minecraft:dark_oak_wood",
        rgb: [60, 46, 26],
        sets: D,
        shaped: None,
    },
    Block {
        name: "minecraft:acacia_wood",
        rgb: [103, 96, 86],
        sets: D,
        shaped: None,
    },
    // Mineral blocks: the only convincing bright metals and the strongest
    // saturated accents.
    Block {
        name: "minecraft:iron_block",
        rgb: [220, 220, 220],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:gold_block",
        rgb: [246, 208, 61],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:diamond_block",
        rgb: [98, 237, 228],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:emerald_block",
        rgb: [42, 203, 87],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:lapis_block",
        rgb: [30, 67, 140],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:redstone_block",
        rgb: [175, 24, 5],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:netherite_block",
        rgb: [66, 60, 63],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:coal_block",
        rgb: [16, 15, 15],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:amethyst_block",
        rgb: [133, 97, 191],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:raw_iron_block",
        rgb: [166, 135, 107],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:raw_copper_block",
        rgb: [154, 105, 79],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:raw_gold_block",
        rgb: [221, 169, 46],
        sets: M,
        shaped: None,
    },
    // Only the waxed copper variants: unwaxed copper oxidises, so a wall built
    // from it changes colour over the following in-game weeks.
    Block {
        name: "minecraft:waxed_copper_block",
        rgb: [192, 107, 79],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:waxed_exposed_copper",
        rgb: [161, 125, 103],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:waxed_weathered_copper",
        rgb: [108, 153, 122],
        sets: M,
        shaped: None,
    },
    Block {
        name: "minecraft:waxed_oxidized_copper",
        rgb: [82, 162, 132],
        sets: M,
        shaped: None,
    },
    // Ground, growth and the rest of the natural palette.
    Block {
        name: "minecraft:dirt",
        rgb: [134, 96, 67],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:coarse_dirt",
        rgb: [119, 85, 59],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:rooted_dirt",
        rgb: [144, 103, 76],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:clay",
        rgb: [160, 166, 179],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:moss_block",
        rgb: [89, 109, 45],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:sculk",
        rgb: [8, 22, 29],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:snow_block",
        rgb: [249, 254, 254],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:packed_ice",
        rgb: [141, 180, 250],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:blue_ice",
        rgb: [116, 167, 253],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:glowstone",
        rgb: [171, 131, 84],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:sea_lantern",
        rgb: [172, 199, 190],
        sets: N,
        shaped: None,
    },
    Block {
        name: "minecraft:honeycomb_block",
        rgb: [229, 148, 29],
        sets: N,
        shaped: None,
    },
];

/// Every block in `sets`, with its colour precomputed in Oklab.
fn candidates() -> &'static [(Oklab, &'static Block)] {
    static CACHE: OnceLock<Vec<(Oklab, &'static Block)>> = OnceLock::new();
    CACHE.get_or_init(|| {
        BLOCKS
            .iter()
            .map(|block| (oklab_from_srgb(block.rgb), block))
            .collect()
    })
}

/// The block in `sets` that looks most like `target`.
///
/// Returns `None` only if `sets` selects nothing, which `parse_set` already
/// rules out for anything it accepts.
pub fn nearest(target: Oklab, sets: u16) -> Option<&'static Block> {
    candidates()
        .iter()
        .filter(|(_, block)| block.sets & sets != 0)
        .min_by(|(a, _), (b, _)| {
            let (a, b) = (
                crate::palette::color::distance_squared(target, *a),
                crate::palette::color::distance_squared(target, *b),
            );
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, block)| *block)
}

/// Look a block up by its id, for validating configured names.
pub fn find(name: &str) -> Option<&'static Block> {
    BLOCKS.iter().find(|block| block.name == name)
}

/// The block id to use for a shape, or `None` if this block has no such
/// variant and should stay a full cube.
pub fn shaped(name: &str, variant: crate::voxel::shapes::Variant) -> Option<&str> {
    use crate::voxel::shapes::Variant;
    match variant {
        Variant::Full => Some(name),
        Variant::Slab => find(name).and_then(|b| b.shaped).map(|(slab, _)| slab),
        Variant::Stairs => find(name).and_then(|b| b.shaped).map(|(_, stairs)| stairs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::color::oklab_from_srgb;

    #[test]
    fn every_block_is_namespaced_and_unique() {
        let mut names: Vec<&str> = BLOCKS.iter().map(|b| b.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate block in the table");
        assert!(BLOCKS.iter().all(|b| b.name.starts_with("minecraft:")));
        assert!(
            BLOCKS.iter().all(|b| b.sets != 0),
            "a block in no set is unreachable"
        );
    }

    /// Every named set must actually contain blocks, or `palette_set` would
    /// accept a value that then matches nothing.
    #[test]
    fn every_named_set_is_populated() {
        for (name, bit) in SET_NAMES {
            let count = BLOCKS.iter().filter(|b| b.sets & bit != 0).count();
            assert!(count > 0, "set `{name}` is empty");
        }
    }

    #[test]
    fn sets_parse_singly_and_in_lists() {
        assert_eq!(parse_set("full").unwrap(), SET_ALL);
        assert_eq!(parse_set("").unwrap(), SET_ALL);
        assert_eq!(parse_set("concrete").unwrap(), SET_CONCRETE);
        assert_eq!(
            parse_set("stone, concrete").unwrap(),
            SET_STONE | SET_CONCRETE
        );
        assert!(parse_set("plastic").unwrap_err().contains("plastic"));
    }

    #[test]
    fn nearest_finds_the_obvious_answers() {
        let all = SET_ALL;
        assert_eq!(
            nearest(oklab_from_srgb([250, 250, 250]), all).unwrap().name,
            "minecraft:snow_block"
        );
        assert_eq!(
            nearest(oklab_from_srgb([5, 5, 8]), all).unwrap().name,
            "minecraft:black_concrete"
        );
        // Mid grey, which is most of a Source map, must land on plain stone.
        assert_eq!(
            nearest(oklab_from_srgb([125, 125, 125]), all).unwrap().name,
            "minecraft:stone"
        );
    }

    #[test]
    fn a_restricted_set_only_returns_its_own_blocks() {
        let block = nearest(oklab_from_srgb([125, 125, 125]), SET_CONCRETE).unwrap();
        assert!(block.name.ends_with("_concrete"), "{}", block.name);
    }

    /// The cold grey of Half-Life 2's Citadel metal must stay grey. Prismarine
    /// sits close enough in Oklab to win this on lightness, which is why it is
    /// filed under `exotic` and out of the sets used for grey surfaces.
    #[test]
    fn a_cold_grey_does_not_come_back_teal() {
        for set in [SET_STONE, SET_METAL | SET_STONE, SET_CONCRETE] {
            let block = nearest(oklab_from_srgb([132, 151, 151]), set).unwrap();
            assert!(
                !block.name.contains("prismarine"),
                "cold grey matched {} in set {set:#b}",
                block.name
            );
        }
    }

    /// Every block must be the nearest match to its own colour, which fails if
    /// two entries were given colours close enough to be interchangeable.
    #[test]
    fn each_block_is_its_own_best_match() {
        for block in BLOCKS {
            let found = nearest(oklab_from_srgb(block.rgb), SET_ALL).unwrap();
            assert_eq!(
                found.name, block.name,
                "{} is out-matched by {} at its own colour",
                block.name, found.name
            );
        }
    }

    /// A typo in a variant name is invisible here but fatal in game: the
    /// schematic names a block that does not exist and the paste fails. Every
    /// variant must at least be a plausible vanilla id derived from a family
    /// vanilla actually has slabs and stairs for.
    #[test]
    fn slab_and_stair_variants_are_well_formed() {
        let mut shaped = 0;
        for block in BLOCKS {
            let Some((slab, stairs)) = block.shaped else {
                continue;
            };
            shaped += 1;
            assert!(slab.starts_with("minecraft:"), "{slab}");
            assert!(stairs.starts_with("minecraft:"), "{stairs}");
            assert!(slab.ends_with("_slab"), "{slab} is not a slab id");
            assert!(stairs.ends_with("_stairs"), "{stairs} is not a stairs id");
            // The two must belong to the same family.
            assert_eq!(
                slab.trim_end_matches("_slab"),
                stairs.trim_end_matches("_stairs"),
                "{slab} and {stairs} are different families"
            );
        }
        assert!(shaped > 30, "only {shaped} blocks have variants");
    }

    /// Vanilla's naming is irregular, so the ones that trip people up are
    /// pinned rather than derived.
    #[test]
    fn irregular_variant_names_are_spelled_out() {
        use crate::voxel::shapes::Variant;
        assert_eq!(
            shaped("minecraft:stone_bricks", Variant::Slab),
            Some("minecraft:stone_brick_slab")
        );
        assert_eq!(
            shaped("minecraft:deepslate_tiles", Variant::Stairs),
            Some("minecraft:deepslate_tile_stairs")
        );
        assert_eq!(
            shaped("minecraft:bricks", Variant::Slab),
            Some("minecraft:brick_slab")
        );
        assert_eq!(
            shaped("minecraft:quartz_block", Variant::Stairs),
            Some("minecraft:quartz_stairs")
        );
    }

    /// A block with no variants must stay a full cube rather than invent one.
    #[test]
    fn blocks_without_variants_have_no_shapes() {
        use crate::voxel::shapes::Variant;
        assert_eq!(shaped("minecraft:gilded_blackstone", Variant::Slab), None);
        assert_eq!(shaped("minecraft:white_concrete", Variant::Stairs), None);
        // Full always works, including for blocks outside the table.
        assert_eq!(
            shaped("minecraft:glass", Variant::Full),
            Some("minecraft:glass")
        );
        assert_eq!(shaped("kubejs:whatever", Variant::Slab), None);
    }

    #[test]
    fn find_locates_blocks_by_id() {
        assert!(find("minecraft:stone").is_some());
        assert!(find("minecraft:cheese").is_none());
    }
}
