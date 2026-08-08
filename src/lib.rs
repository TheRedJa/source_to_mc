//! Convert Source Engine BSP maps into Minecraft schematics.

pub mod bsp;
pub mod config;
pub mod convert;
pub mod geom;
pub mod inspect;
pub mod output;
pub mod palette;
pub mod source;
pub mod voxel;

/// Minecraft 1.21.1. Written into every schematic so WorldEdit can convert
/// block states forward if the target version differs.
pub const DATA_VERSION: i32 = 3955;

/// Version of the interchange format described in `docs/format.md`: the
/// schematic fields and bundle layout the companion mod reads.
///
/// This is the only coordination point between this converter and the mod. It
/// is written into every schematic and must be bumped whenever the meaning or
/// layout of anything in that document changes, in the same commit as the
/// regenerated fixtures under `tests/fixtures`. The mod carries a constant of
/// the same name and refuses input it does not understand.
pub const FORMAT_VERSION: i32 = 1;

/// Build limits of a vanilla 1.21.1 overworld.
pub const VANILLA_MIN_Y: i32 = -64;
pub const VANILLA_MAX_Y: i32 = 319;

/// Limits a datapack `dimension_type` can raise the world to: `min_y` may go
/// down to -2032 and `height` up to 4064, with `min_y + height - 1 <= 2031`.
pub const DIMENSION_MIN_Y: i32 = -2032;
pub const DIMENSION_MAX_Y: i32 = 2031;
pub const DIMENSION_MAX_HEIGHT: i32 = 4064;
