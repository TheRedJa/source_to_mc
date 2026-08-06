//! Reading Source's own content: material definitions and textures.
//!
//! Everything here is optional. The converter works without a game install by
//! matching the average colour the compiler baked into the BSP; this module is
//! what turns that into the real texture.

pub mod extract;
pub mod mdl;
pub mod report;
pub mod vfs;
pub mod vmt;
pub mod vtf;
