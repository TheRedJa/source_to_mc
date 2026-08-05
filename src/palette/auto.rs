//! Choosing a block for a material by colour alone.
//!
//! The obvious way to get a texture's average colour is to find its `.vmt`,
//! read `$basetexture`, locate the `.vtf` in the map's pakfile or the game's
//! VPKs, and decode its smallest mipmap. Source has already done all of that:
//! the compiler stores each texture's average colour in the texture-data lump
//! as `reflectivity`, because radiosity needs to know what colour light bounces
//! off a surface. It is in the BSP, it needs no game install, and it costs
//! nothing to read.
//!
//! One caveat worth knowing. `reflectivity` is an average taken in linear
//! light, whereas the block colours in [`blocks`](crate::palette::blocks) are
//! averages of what you see, taken in sRGB. Averaging in linear light gives a
//! slightly brighter result for a texture with strong light and dark areas, so
//! busy textures match a little lighter than they look. It is a small bias, and
//! a rule overrides it wherever it matters.

use crate::palette::blocks::{self, Block};
use crate::palette::color::{oklab_from_linear, srgb_from_linear};

/// Colour matching restricted to a set of blocks.
#[derive(Debug, Clone, Copy)]
pub struct Auto {
    sets: u16,
}

impl Auto {
    pub fn new(sets: u16) -> Auto {
        Auto { sets }
    }

    /// The closest block to a material's average colour, or `None` when the
    /// map carries no usable colour for it.
    pub fn block_for(&self, reflectivity: [f64; 3]) -> Option<&'static Block> {
        let color = usable(reflectivity)?;
        blocks::nearest(oklab_from_linear(color), self.sets)
    }
}

/// A material's colour as it would be displayed, for reports.
pub fn display_color(reflectivity: [f64; 3]) -> [u8; 3] {
    srgb_from_linear(usable(reflectivity).unwrap_or([0.0, 0.0, 0.0]))
}

/// Reject reflectivity values that cannot describe a colour.
///
/// Tool and nodraw textures are never rendered, so the compiler leaves their
/// reflectivity at zero; matching that would silently turn every one of them
/// into the darkest block in the palette. Values above 1 do occur and are
/// clamped rather than discarded.
fn usable(reflectivity: [f64; 3]) -> Option<[f64; 3]> {
    if !reflectivity.iter().all(|c| c.is_finite() && *c >= 0.0) {
        return None;
    }
    if reflectivity.iter().all(|c| *c < 1e-4) {
        return None;
    }
    Some(reflectivity.map(|c| c.min(1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto() -> Auto {
        Auto::new(blocks::SET_ALL)
    }

    /// A mid grey in linear light is a fairly light grey on screen. Treating
    /// reflectivity as if it were already sRGB would land on deepslate or
    /// darker, so this pins the gamma handling.
    #[test]
    fn a_linear_mid_grey_matches_a_mid_grey_block() {
        let block = auto().block_for([0.25, 0.25, 0.25]).unwrap();
        assert!(
            ["minecraft:stone", "minecraft:andesite", "minecraft:cobblestone"]
                .contains(&block.name),
            "got {}",
            block.name
        );
    }

    /// A dark texture must still come back dark: the two ends of the range
    /// have to stay apart, not collapse onto one grey.
    #[test]
    fn dark_and_light_greys_stay_apart() {
        let dark = auto().block_for([0.03, 0.03, 0.03]).unwrap();
        let light = auto().block_for([0.75, 0.75, 0.75]).unwrap();
        assert_ne!(dark.name, light.name);
        assert!(dark.rgb[0] < 100, "{} is not dark", dark.name);
        assert!(light.rgb[0] > 200, "{} is not light", light.name);
    }

    #[test]
    fn strong_colours_keep_their_hue() {
        let red = auto().block_for([0.35, 0.02, 0.02]).unwrap();
        assert!(
            red.name.contains("red") || red.name.contains("nether"),
            "red matched {}",
            red.name
        );
        let green = auto().block_for([0.03, 0.25, 0.03]).unwrap();
        assert!(
            green.name.contains("green") || green.name.contains("lime"),
            "green matched {}",
            green.name
        );
    }

    #[test]
    fn a_black_reflectivity_is_rejected_rather_than_matched() {
        assert!(auto().block_for([0.0, 0.0, 0.0]).is_none());
        assert!(auto().block_for([f64::NAN, 0.2, 0.2]).is_none());
        assert!(auto().block_for([-1.0, 0.2, 0.2]).is_none());
    }

    #[test]
    fn out_of_range_values_are_clamped_not_dropped() {
        let block = auto().block_for([4.0, 4.0, 4.0]).unwrap();
        assert_eq!(block.name, "minecraft:snow_block");
    }

    #[test]
    fn restricting_the_set_restricts_the_answer() {
        let auto = Auto::new(blocks::SET_TERRACOTTA);
        let block = auto.block_for([0.25, 0.25, 0.25]).unwrap();
        assert!(block.name.contains("terracotta"), "{}", block.name);
    }

    #[test]
    fn display_colour_brightens_linear_values() {
        // 0.25 linear is roughly 0.54 sRGB; showing 0.25 directly would be far
        // too dark to recognise the texture from.
        let [r, g, b] = display_color([0.25, 0.25, 0.25]);
        assert_eq!([r, g, b], [137, 137, 137]);
        assert_eq!(display_color([0.0, 0.0, 0.0]), [0, 0, 0]);
    }
}
