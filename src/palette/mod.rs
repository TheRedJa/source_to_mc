//! Choosing a Minecraft block for a piece of Source geometry.
//!
//! Two things decide a block: the brush's contents flags (water, glass, grates,
//! ladders) and the material on the nearest face. Contents win, because a water
//! brush is water whatever texture it wears.

use crate::config::{ContentAction, Config};
use std::collections::BTreeMap;
use vbsp::BrushFlags;

/// Contents flags in the order they are tested. Earlier entries win, so the
/// specific non-solid volumes are checked before the generic ones.
const FLAG_ORDER: &[(&str, BrushFlags)] = &[
    ("origin", BrushFlags::ORIGIN),
    ("areaportal", BrushFlags::AREAPORTAL),
    ("playerclip", BrushFlags::PLAYERCLIP),
    ("monsterclip", BrushFlags::MONSTERCLIP),
    ("ladder", BrushFlags::LADDER),
    ("grate", BrushFlags::GRATE),
    ("window", BrushFlags::WINDOW),
    ("water", BrushFlags::WATER),
    ("slime", BrushFlags::SLIME),
    ("moveable", BrushFlags::MOVEABLE),
    ("solid", BrushFlags::SOLID),
];

/// What should happen to a brush.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Leave it out of the conversion.
    Skip,
    /// Force every voxel to this block.
    Force(String),
    /// Let the material on the nearest face decide.
    ByMaterial,
}

pub struct Resolver {
    actions: BTreeMap<String, ContentAction>,
    fallback: String,
    skip_tool_brushes: bool,
    skip_sky: bool,
}

impl Resolver {
    pub fn new(config: &Config) -> Resolver {
        Resolver {
            actions: config.contents.actions.clone(),
            fallback: config.materials.fallback_block.clone(),
            skip_tool_brushes: config.contents.skip_tool_brushes,
            skip_sky: config.contents.skip_sky,
        }
    }

    /// Decide what to do with a brush based on its contents flags.
    pub fn decide(&self, flags: BrushFlags) -> Decision {
        for (name, flag) in FLAG_ORDER {
            if !flags.intersects(*flag) {
                continue;
            }
            match self.actions.get(*name) {
                Some(ContentAction::Skip) => return Decision::Skip,
                Some(ContentAction::Block(block)) => return Decision::Force(block.clone()),
                Some(ContentAction::Solid) => return Decision::ByMaterial,
                None => continue,
            }
        }

        // Anything with no configured action is only kept if it is solid.
        if flags.intersects(BrushFlags::SOLID | BrushFlags::OPAQUE | BrushFlags::MOVEABLE) {
            Decision::ByMaterial
        } else {
            Decision::Skip
        }
    }

    /// Whether a material is a tool texture that should never become blocks.
    pub fn is_tool_material(&self, material: &str) -> bool {
        let lower = material.to_ascii_lowercase();
        let is_sky = lower.starts_with("tools/toolsskybox")
            || lower.starts_with("tools/toolsskybox2d")
            || lower.contains("skybox");

        if is_sky {
            return self.skip_sky;
        }
        if !self.skip_tool_brushes {
            return false;
        }
        lower.starts_with("tools/")
            || matches!(
                lower.as_str(),
                "tools/toolsnodraw" | "tools/toolsskip" | "tools/toolshint" | "tools/toolsclip"
            )
    }

    /// Block for a face material. Until the rules engine lands, everything
    /// that is not a tool texture becomes the fallback block.
    pub fn block_for_material(&self, material: Option<&str>) -> Option<&str> {
        match material {
            Some(name) if self.is_tool_material(name) => None,
            _ => Some(&self.fallback),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> Resolver {
        Resolver::new(&Config::default())
    }

    #[test]
    fn solid_brushes_use_their_material() {
        assert_eq!(resolver().decide(BrushFlags::SOLID), Decision::ByMaterial);
    }

    #[test]
    fn clip_brushes_are_skipped() {
        let r = resolver();
        assert_eq!(r.decide(BrushFlags::PLAYERCLIP), Decision::Skip);
        assert_eq!(r.decide(BrushFlags::MONSTERCLIP), Decision::Skip);
    }

    #[test]
    fn water_and_glass_are_forced_to_their_blocks() {
        let r = resolver();
        assert_eq!(
            r.decide(BrushFlags::WATER),
            Decision::Force("minecraft:water".into())
        );
        assert_eq!(
            r.decide(BrushFlags::WINDOW),
            Decision::Force("minecraft:glass".into())
        );
        assert_eq!(
            r.decide(BrushFlags::GRATE),
            Decision::Force("minecraft:iron_bars".into())
        );
    }

    /// A clip brush is also flagged solid; the more specific rule must win.
    #[test]
    fn specific_contents_beat_solid() {
        let r = resolver();
        assert_eq!(r.decide(BrushFlags::PLAYERCLIP | BrushFlags::SOLID), Decision::Skip);
        assert_eq!(
            r.decide(BrushFlags::WATER | BrushFlags::SOLID),
            Decision::Force("minecraft:water".into())
        );
    }

    #[test]
    fn empty_brushes_are_skipped() {
        assert_eq!(resolver().decide(BrushFlags::empty()), Decision::Skip);
    }

    #[test]
    fn tool_textures_are_recognised() {
        let r = resolver();
        assert!(r.is_tool_material("TOOLS/TOOLSNODRAW"));
        assert!(r.is_tool_material("tools/toolsskybox"));
        assert!(r.is_tool_material("tools/toolstrigger"));
        assert!(!r.is_tool_material("CONCRETE/CONCRETEWALL001A"));
    }

    #[test]
    fn tool_materials_produce_no_block() {
        let r = resolver();
        assert_eq!(r.block_for_material(Some("tools/toolsnodraw")), None);
        assert_eq!(
            r.block_for_material(Some("METAL/METALWALL001A")),
            Some("minecraft:stone")
        );
    }

    #[test]
    fn tool_skipping_can_be_turned_off() {
        let mut config = Config::default();
        config.contents.skip_tool_brushes = false;
        config.contents.skip_sky = false;
        let r = Resolver::new(&config);
        assert!(!r.is_tool_material("tools/toolsnodraw"));
        assert!(!r.is_tool_material("tools/toolsskybox"));
    }

    #[test]
    fn configured_actions_override_defaults() {
        let mut config = Config::default();
        config
            .contents
            .actions
            .insert("water".into(), ContentAction::Block("minecraft:lava".into()));
        let r = Resolver::new(&config);
        assert_eq!(
            r.decide(BrushFlags::WATER),
            Decision::Force("minecraft:lava".into())
        );
    }
}
