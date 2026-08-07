//! Choosing a Minecraft block for a piece of Source geometry.
//!
//! Three things decide a block, in this order:
//!
//! 1. The brush's **contents flags**. A water brush is water whatever texture it
//!    wears, and a clip brush is nothing at all.
//! 2. A **material rule** — an ordered glob pattern, see [`rules`].
//! 3. The material's **average colour**, matched against a curated block list,
//!    see [`auto`].
//!
//! Steps 2 and 3 are resolved once per material when the resolver is built, not
//! per voxel: a map has a few hundred materials and tens of millions of voxels.

pub mod auto;
pub mod blocks;
pub mod color;
pub mod report;
pub mod rules;

use crate::bsp::Material;
use crate::config::{Config, ContentAction};
use anyhow::{Context, Result};
use rules::{Action, Rules};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use vbsp::BrushFlags;

/// Starter rules for Half-Life 2 and Entropy: Zero, compiled into the binary.
const BUILTIN_RULES: &str = include_str!("../../assets/rules/ez.toml");

/// The shipped rules, parsed once.
pub fn builtin_rules() -> &'static Rules {
    static CACHE: OnceLock<Rules> = OnceLock::new();
    CACHE.get_or_init(|| {
        Rules::parse(BUILTIN_RULES, "built-in rules").expect("the built-in rules must parse")
    })
}

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

/// Why a material ended up with the block it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A tool texture, dropped before rules were consulted.
    Tool,
    /// Matched rule number `n`, counting user rules before built-in ones.
    Rule(usize),
    /// A generated block carrying the material's own Source texture.
    Texture,
    /// Matched by average colour.
    Auto,
    /// Nothing matched, so the configured fallback was used.
    Fallback,
}

impl Source {
    pub fn label(&self) -> String {
        match self {
            Source::Tool => "tool".into(),
            Source::Rule(index) => format!("rule {}", index + 1),
            Source::Texture => "texture".into(),
            Source::Auto => "auto".into(),
            Source::Fallback => "fallback".into(),
        }
    }
}

/// The block one material resolves to, decided once at startup.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub material: String,
    /// `None` means faces with this material contribute no blocks.
    pub block: Option<String>,
    pub source: Source,
    /// The material's average colour as it would be displayed.
    pub color: [u8; 3],
    /// The rule that matched, if any. A rule can match and still leave the
    /// choice to colour matching, so this is not implied by `source`.
    pub rule: Option<usize>,
}

#[derive(Debug)]
pub struct Resolver {
    actions: BTreeMap<String, ContentAction>,
    fallback: String,
    skip_tool_brushes: bool,
    skip_sky: bool,
    /// One entry per material, indexed as [`crate::bsp::Map::materials`] is.
    assignments: Vec<Assignment>,
    rules: Rules,
}

impl Resolver {
    /// Build a resolver and resolve every material up front.
    ///
    /// Fails only on a bad configuration: an unknown `palette_set`, or rules
    /// that do not compile.
    pub fn new(config: &Config, materials: &[Material]) -> Result<Resolver> {
        Resolver::with_textures(config, materials, &BTreeMap::new())
    }

    /// As [`Resolver::new`], with generated texture blocks keyed by material.
    ///
    /// A texture beats colour matching but not a rule that names a block: no
    /// texture makes a grate see-through, so `*grate*` staying `iron_bars` is
    /// the difference between a grate and an opaque cube with a grate painted
    /// on it. Rules that only narrow colour matching yield to the texture.
    pub fn with_textures(
        config: &Config,
        materials: &[Material],
        textures: &BTreeMap<String, String>,
    ) -> Result<Resolver> {
        let sets = blocks::parse_set(&config.materials.palette_set)
            .map_err(|e| anyhow::anyhow!("{e}"))
            .context("in [materials] palette_set")?;

        let mut rules = config.materials.loaded_rules.clone();
        if config.materials.builtin_rules {
            rules = rules.followed_by(builtin_rules().clone())?;
        }

        let auto = config.materials.auto_palette.then_some(sets);

        let mut resolver = Resolver {
            actions: config.contents.actions.clone(),
            fallback: config.materials.fallback_block.clone(),
            skip_tool_brushes: config.contents.skip_tool_brushes,
            skip_sky: config.contents.skip_sky,
            assignments: Vec::with_capacity(materials.len()),
            rules,
        };

        resolver.assignments = materials
            .iter()
            .map(|material| resolver.assign(material, auto, textures))
            .collect();
        Ok(resolver)
    }

    /// `default_sets` is the configured `palette_set`, or `None` when colour
    /// matching is switched off entirely.
    fn assign(
        &self,
        material: &Material,
        default_sets: Option<u16>,
        textures: &BTreeMap<String, String>,
    ) -> Assignment {
        let color = auto::display_color(material.reflectivity);
        let matched = self.rules.matches(&material.name).map(|(index, _)| index);
        let finish = |block: Option<String>, source: Source| Assignment {
            material: material.name.clone(),
            block,
            source,
            color,
            rule: matched,
        };

        if self.is_tool_material(&material.name) {
            return finish(None, Source::Tool);
        }

        let mut sets = default_sets;
        if let Some((index, rule)) = self.rules.matches(&material.name) {
            match &rule.action {
                Action::Block(block) => return finish(Some(block.clone()), Source::Rule(index)),
                Action::Skip => return finish(None, Source::Rule(index)),
                // `auto` hands over to the colour matcher, narrowed to the
                // rule's own block set if it named one. The reported source is
                // the colour match that follows, since that is what chose.
                // `auto_palette = false` means never guess, so a rule cannot
                // switch colour matching back on — only narrow it.
                Action::Auto(rule_sets) if default_sets.is_some() => {
                    sets = rule_sets.or(default_sets);
                }
                Action::Auto(_) => {}
            }
        }

        // The material's own texture, where one was generated for it.
        if let Some(block) = textures.get(&material.name) {
            return finish(Some(block.clone()), Source::Texture);
        }

        if let Some(block) =
            sets.and_then(|sets| auto::Auto::new(sets).block_for(material.reflectivity))
        {
            return finish(Some(block.name.to_string()), Source::Auto);
        }

        finish(Some(self.fallback.clone()), Source::Fallback)
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
        if lower.contains("skybox") {
            return self.skip_sky;
        }
        if !self.skip_tool_brushes {
            return false;
        }
        lower.starts_with("tools/")
    }

    /// The block for a material index, or `None` if its faces are dropped.
    pub fn block_for_material(&self, material: usize) -> Option<&str> {
        self.assignments.get(material)?.block.as_deref()
    }

    /// Whether a rule named this material's block outright, as opposed to
    /// colour matching having guessed it. A written rule is a statement of
    /// intent and outranks a brush's contents flags; a guess does not.
    pub fn named_by_rule(&self, material: usize) -> bool {
        self.assignments
            .get(material)
            .is_some_and(|a| matches!(a.source, Source::Rule(_)))
    }

    /// The block used for faces with no material at all.
    pub fn fallback_block(&self) -> &str {
        &self.fallback
    }

    /// Every material's resolved block, for the `materials` report.
    pub fn assignments(&self) -> &[Assignment] {
        &self.assignments
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(name: &str, reflectivity: [f64; 3]) -> Material {
        Material {
            name: name.into(),
            raw_name: name.into(),
            reflectivity,
        }
    }

    fn resolver_with(config: &Config, materials: &[Material]) -> Resolver {
        Resolver::new(config, materials).unwrap()
    }

    fn resolver() -> Resolver {
        resolver_with(&Config::default(), &[])
    }

    #[test]
    fn the_builtin_rules_parse() {
        assert!(builtin_rules().len() > 10);
    }

    /// Every block a built-in rule names must exist, or a conversion silently
    /// produces a schematic WorldEdit refuses to paste.
    #[test]
    fn builtin_rules_only_name_real_blocks() {
        // Blocks the rules use that are deliberately outside the auto-palette
        // list, because they are not opaque full cubes.
        let extra = [
            "minecraft:iron_bars",
            "minecraft:ladder",
            "minecraft:glass",
            "minecraft:light_blue_stained_glass",
            "minecraft:water",
        ];
        for index in 0..builtin_rules().len() {
            let rule = builtin_rules().get(index).unwrap();
            if let Action::Block(block) = &rule.action {
                assert!(
                    blocks::find(block).is_some() || extra.contains(&block.as_str()),
                    "rule {} names unknown block {block}",
                    index + 1
                );
            }
        }
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
        assert_eq!(
            r.decide(BrushFlags::PLAYERCLIP | BrushFlags::SOLID),
            Decision::Skip
        );
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
        assert!(r.is_tool_material("tools/toolsnodraw"));
        assert!(r.is_tool_material("tools/toolsskybox"));
        assert!(r.is_tool_material("tools/toolstrigger"));
        assert!(!r.is_tool_material("concrete/concretewall001a"));
    }

    #[test]
    fn tool_skipping_can_be_turned_off() {
        let mut config = Config::default();
        config.contents.skip_tool_brushes = false;
        config.contents.skip_sky = false;
        let r = resolver_with(&config, &[]);
        assert!(!r.is_tool_material("tools/toolsnodraw"));
        assert!(!r.is_tool_material("tools/toolsskybox"));
    }

    #[test]
    fn configured_actions_override_defaults() {
        let mut config = Config::default();
        config.contents.actions.insert(
            "water".into(),
            ContentAction::Block("minecraft:lava".into()),
        );
        let r = resolver_with(&config, &[]);
        assert_eq!(
            r.decide(BrushFlags::WATER),
            Decision::Force("minecraft:lava".into())
        );
    }

    // --- material assignment ------------------------------------------------

    #[test]
    fn tool_materials_produce_no_block() {
        let materials = [
            material("tools/toolsnodraw", [0.0, 0.0, 0.0]),
            material("metal/metalwall001a", [0.2, 0.2, 0.2]),
        ];
        let r = resolver_with(&Config::default(), &materials);
        assert_eq!(r.block_for_material(0), None);
        assert_eq!(r.assignments()[0].source, Source::Tool);
        assert!(r.block_for_material(1).is_some());
    }

    /// Where the kind of block is the point, a built-in rule must name it:
    /// no colour implies "you can see through this" or "you can climb it".
    #[test]
    fn builtin_rules_name_the_blocks_colour_cannot_imply() {
        let materials = [
            material("metal/metalgrate011a", [0.2, 0.2, 0.2]),
            material("glass/glasswindow002a", [0.4, 0.4, 0.4]),
            material("metal/metalladder001a", [0.2, 0.2, 0.2]),
            material("nature/water_riverbed01", [0.1, 0.2, 0.2]),
            material("decals/posterthing", [0.3, 0.3, 0.3]),
        ];
        let r = resolver_with(&Config::default(), &materials);
        assert_eq!(r.block_for_material(0), Some("minecraft:iron_bars"));
        assert_eq!(r.block_for_material(1), Some("minecraft:glass"));
        assert_eq!(r.block_for_material(2), Some("minecraft:ladder"));
        assert_eq!(r.block_for_material(3), Some("minecraft:water"));
        assert_eq!(r.block_for_material(4), None);
    }

    /// Broad surface families route to colour matching, but narrowed to their
    /// own kind of block: wood stays wooden however dark the texture is.
    #[test]
    fn builtin_rules_narrow_broad_families_to_their_own_blocks() {
        let materials = [
            material("wood/woodwall009a", [0.30, 0.18, 0.08]),
            material("wood/woodfloor003a", [0.04, 0.02, 0.01]),
            material("concrete/concretewall013d", [0.50, 0.51, 0.50]),
        ];
        let r = resolver_with(&Config::default(), &materials);
        for index in 0..2 {
            let block = r.block_for_material(index).unwrap();
            assert!(
                block.contains("planks") || block.contains("wood"),
                "wood matched {block}"
            );
            assert_eq!(r.assignments()[index].source, Source::Auto);
        }
        // Two woods of very different brightness must not collapse onto one
        // block; that flattening is what the `set` mechanism exists to avoid.
        assert_ne!(r.block_for_material(0), r.block_for_material(1));

        let concrete = r.block_for_material(2).unwrap();
        assert!(
            blocks::find(concrete).unwrap().sets & blocks::SET_STONE != 0,
            "concrete matched {concrete}"
        );
    }

    /// A material no rule covers falls through to colour matching, and the
    /// colour has to actually be used.
    #[test]
    fn unmatched_materials_are_matched_by_colour() {
        let materials = [
            material("custom/rustything", [0.30, 0.06, 0.02]),
            material("custom/paleboard", [0.70, 0.70, 0.68]),
        ];
        let r = resolver_with(&Config::default(), &materials);
        assert_eq!(r.assignments()[0].source, Source::Auto);
        assert_eq!(r.assignments()[1].source, Source::Auto);
        assert_ne!(r.block_for_material(0), r.block_for_material(1));
    }

    #[test]
    fn colourless_materials_fall_back() {
        let materials = [material("custom/nothing", [0.0, 0.0, 0.0])];
        let r = resolver_with(&Config::default(), &materials);
        assert_eq!(r.assignments()[0].source, Source::Fallback);
        assert_eq!(r.block_for_material(0), Some("minecraft:stone"));
    }

    #[test]
    fn turning_off_the_auto_palette_uses_the_fallback() {
        let mut config = Config::default();
        config.materials.auto_palette = false;
        config.materials.fallback_block = "minecraft:cobblestone".into();
        let materials = [material("custom/rustything", [0.30, 0.06, 0.02])];
        let r = resolver_with(&config, &materials);
        assert_eq!(r.assignments()[0].source, Source::Fallback);
        assert_eq!(r.block_for_material(0), Some("minecraft:cobblestone"));
    }

    #[test]
    fn a_palette_set_restricts_auto_matches() {
        let mut config = Config::default();
        config.materials.palette_set = "concrete".into();
        let materials = [material("custom/rustything", [0.30, 0.06, 0.02])];
        let r = resolver_with(&config, &materials);
        assert!(
            r.block_for_material(0).unwrap().ends_with("_concrete"),
            "{:?}",
            r.block_for_material(0)
        );
    }

    #[test]
    fn an_unknown_palette_set_is_an_error() {
        let mut config = Config::default();
        config.materials.palette_set = "cheese".into();
        let err = Resolver::new(&config, &[]).unwrap_err();
        assert!(format!("{err:#}").contains("cheese"), "{err:#}");
    }

    /// The user's own rules are consulted before the shipped ones.
    #[test]
    fn user_rules_override_the_builtins() {
        let mut config = Config::default();
        config.materials.loaded_rules = Rules::parse(
            "[[rule]]\nmatch = \"brick/*\"\nblock = \"minecraft:gold_block\"\n",
            "test",
        )
        .unwrap();
        let materials = [material("brick/brickwall001a", [0.2, 0.1, 0.1])];
        let r = resolver_with(&config, &materials);
        assert_eq!(r.block_for_material(0), Some("minecraft:gold_block"));
    }

    #[test]
    fn the_builtins_can_be_turned_off_entirely() {
        let mut config = Config::default();
        config.materials.builtin_rules = false;
        let materials = [material("metal/metalgrate011a", [0.2, 0.2, 0.2])];
        let r = resolver_with(&config, &materials);
        assert_eq!(r.assignments()[0].source, Source::Auto);
    }
}
