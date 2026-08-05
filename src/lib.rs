//! Convert Source Engine BSP maps into Minecraft schematics.

pub mod bsp;
pub mod config;
pub mod convert;
pub mod geom;
pub mod inspect;
pub mod output;
pub mod palette;
pub mod voxel;

/// Minecraft 1.21.1. Written into every schematic so WorldEdit can convert
/// block states forward if the target version differs.
pub const DATA_VERSION: i32 = 3955;

/// Build limits of a vanilla 1.21.1 overworld.
pub const VANILLA_MIN_Y: i32 = -64;
pub const VANILLA_MAX_Y: i32 = 319;

/// Limits a datapack `dimension_type` can raise the world to: `min_y` may go
/// down to -2032 and `height` up to 4064, with `min_y + height - 1 <= 2031`.
pub const DIMENSION_MIN_Y: i32 = -2032;
pub const DIMENSION_MAX_Y: i32 = 2031;
pub const DIMENSION_MAX_HEIGHT: i32 = 4064;
