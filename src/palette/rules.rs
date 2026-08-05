//! Material rules: ordered glob patterns mapping Source materials to blocks.
//!
//! Rules are the escape hatch from colour matching. Average colour cannot tell
//! that `metal/metalgrate011a` is a grate or that `glass/glasswindow002a` is a
//! window, and it will confidently turn rusted metal into terracotta. A rule
//! says so outright.
//!
//! The file is TOML, and the first rule whose pattern matches wins:
//!
//! ```toml
//! [[rule]]
//! match = "tools/*"
//! skip = true
//!
//! [[rule]]
//! match = ["metal/metalwall*", "metal/metalhull*"]
//! block = "minecraft:iron_block"
//!
//! [[rule]]
//! match = "nature/blend*"
//! auto = true          # let colour matching decide, ignoring later rules
//! ```
//!
//! Patterns are matched case-insensitively against the normalized material
//! path, and `*` crosses `/` freely, so `*grate*` catches every grate wherever
//! it lives.

use anyhow::{Context, Result, bail};
use globset::{GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::path::Path;

/// What a matching rule does with the material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Use this block.
    Block(String),
    /// Drop faces using this material.
    Skip,
    /// Fall through to average-colour matching, optionally restricted to a
    /// block set. `None` uses the configured `palette_set`.
    Auto(Option<u16>),
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub patterns: Vec<String>,
    pub action: Action,
    /// Where the rule came from, for error messages and the material report.
    pub origin: String,
}

/// An ordered set of rules, compiled for matching.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    rules: Vec<Rule>,
    /// One glob per pattern across all rules, in order.
    set: Option<GlobSet>,
    /// `glob index -> rule index`.
    owner: Vec<usize>,
}

impl Rules {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn get(&self, index: usize) -> Option<&Rule> {
        self.rules.get(index)
    }

    /// Parse a rules file. `origin` is only used in messages.
    pub fn parse(text: &str, origin: &str) -> Result<Rules> {
        let file: RuleFile = toml::from_str(text)
            .with_context(|| format!("parsing material rules from {origin}"))?;

        let mut rules = Vec::with_capacity(file.rule.len());
        for (index, raw) in file.rule.into_iter().enumerate() {
            let action = raw.action().with_context(|| {
                format!("in rule {} of {origin}", index + 1)
            })?;
            let patterns = raw.patterns.into_vec();
            if patterns.is_empty() {
                bail!("rule {} of {origin} matches nothing", index + 1);
            }
            rules.push(Rule { patterns, action, origin: origin.to_string() });
        }
        Rules::compile(rules)
    }

    pub fn load(path: &Path) -> Result<Rules> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading material rules {}", path.display()))?;
        Rules::parse(&text, &path.display().to_string())
    }

    /// Concatenate two rule sets; `self`'s rules are tested first.
    pub fn followed_by(self, other: Rules) -> Result<Rules> {
        let mut rules = self.rules;
        rules.extend(other.rules);
        Rules::compile(rules)
    }

    fn compile(rules: Vec<Rule>) -> Result<Rules> {
        if rules.is_empty() {
            return Ok(Rules::default());
        }

        let mut builder = GlobSetBuilder::new();
        let mut owner = Vec::new();
        for (index, rule) in rules.iter().enumerate() {
            for pattern in &rule.patterns {
                let glob = globset::GlobBuilder::new(pattern)
                    .case_insensitive(true)
                    // `*` should cross directory separators: a rule like
                    // `*grate*` is meant to catch grates wherever they live.
                    .literal_separator(false)
                    .build()
                    .map_err(|e| {
                        anyhow::anyhow!("bad pattern `{pattern}` in {}: {e}", rule.origin)
                    })?;
                builder.add(glob);
                owner.push(index);
            }
        }

        Ok(Rules {
            set: Some(builder.build().context("compiling material rules")?),
            rules,
            owner,
        })
    }

    /// The first matching rule, by file order.
    pub fn matches(&self, material: &str) -> Option<(usize, &Rule)> {
        let set = self.set.as_ref()?;
        // `matches` returns every hit; the lowest owning rule index is the
        // earliest rule in the file, which is the one that wins.
        let index = set
            .matches(material)
            .into_iter()
            .map(|glob| self.owner[glob])
            .min()?;
        Some((index, &self.rules[index]))
    }

    /// Rules that no material in `materials` matched, so a config can be pruned.
    pub fn unused<'a>(&self, materials: impl Iterator<Item = &'a str>) -> Vec<usize> {
        let mut used = vec![false; self.rules.len()];
        for material in materials {
            if let Some((index, _)) = self.matches(material) {
                used[index] = true;
            }
        }
        used.iter()
            .enumerate()
            .filter(|(_, used)| !**used)
            .map(|(index, _)| index)
            .collect()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<RawRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    #[serde(rename = "match")]
    patterns: Patterns,
    block: Option<String>,
    #[serde(default)]
    skip: bool,
    #[serde(default)]
    auto: bool,
    /// Restricts `auto` to a block set, e.g. `set = "wood"`.
    set: Option<String>,
}

impl RawRule {
    fn action(&self) -> Result<Action> {
        if self.set.is_some() && !self.auto {
            bail!("`set` only means anything alongside `auto = true`");
        }
        let sets = match &self.set {
            Some(spec) => Some(crate::palette::blocks::parse_set(spec).map_err(|e| anyhow::anyhow!("{e}"))?),
            None => None,
        };
        match (&self.block, self.skip, self.auto) {
            (Some(block), false, false) => Ok(Action::Block(block.clone())),
            (None, true, false) => Ok(Action::Skip),
            (None, false, true) => Ok(Action::Auto(sets)),
            (None, false, false) => {
                bail!("a rule needs one of `block`, `skip = true` or `auto = true`")
            }
            _ => bail!("`block`, `skip` and `auto` are mutually exclusive"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Patterns {
    One(String),
    Many(Vec<String>),
}

impl Patterns {
    fn into_vec(self) -> Vec<String> {
        match self {
            Patterns::One(pattern) => vec![pattern],
            Patterns::Many(patterns) => patterns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str) -> Rules {
        Rules::parse(text, "test").unwrap()
    }

    #[test]
    fn empty_file_matches_nothing() {
        let rules = Rules::parse("", "test").unwrap();
        assert!(rules.is_empty());
        assert!(rules.matches("concrete/concretewall001a").is_none());
    }

    #[test]
    fn a_single_pattern_matches() {
        let rules = rules("[[rule]]\nmatch = \"concrete/*\"\nblock = \"minecraft:stone\"\n");
        let (_, rule) = rules.matches("concrete/concretewall001a").unwrap();
        assert_eq!(rule.action, Action::Block("minecraft:stone".into()));
        assert!(rules.matches("metal/metalwall001a").is_none());
    }

    #[test]
    fn a_list_of_patterns_matches_any_of_them() {
        let rules = rules(
            r#"
            [[rule]]
            match = ["metal/*", "brick/*"]
            block = "minecraft:iron_block"
            "#,
        );
        assert!(rules.matches("metal/metalwall001a").is_some());
        assert!(rules.matches("brick/brickwall001a").is_some());
        assert!(rules.matches("wood/woodwall001a").is_none());
    }

    /// The whole point of the ordering: a specific rule placed above a broad
    /// one must win, whichever order the globs happen to be tested in.
    #[test]
    fn the_first_matching_rule_wins() {
        let rules = rules(
            r#"
            [[rule]]
            match = "metal/metalgrate*"
            block = "minecraft:iron_bars"

            [[rule]]
            match = "metal/*"
            block = "minecraft:iron_block"
            "#,
        );
        let (_, grate) = rules.matches("metal/metalgrate011a").unwrap();
        assert_eq!(grate.action, Action::Block("minecraft:iron_bars".into()));
        let (_, wall) = rules.matches("metal/metalwall001a").unwrap();
        assert_eq!(wall.action, Action::Block("minecraft:iron_block".into()));
    }

    #[test]
    fn patterns_ignore_case() {
        let rules = rules("[[rule]]\nmatch = \"Concrete/*\"\nblock = \"minecraft:stone\"\n");
        assert!(rules.matches("concrete/concretewall001a").is_some());
    }

    /// `*` must cross `/`, so a bare `*grate*` catches grates in any directory.
    #[test]
    fn stars_cross_directory_separators() {
        let rules = rules("[[rule]]\nmatch = \"*grate*\"\nblock = \"minecraft:iron_bars\"\n");
        assert!(rules.matches("metal/metalgrate011a").is_some());
        assert!(rules.matches("props/grate_thing").is_some());
    }

    #[test]
    fn skip_and_auto_actions_parse() {
        let rules = rules(
            r#"
            [[rule]]
            match = "tools/*"
            skip = true

            [[rule]]
            match = "nature/blend*"
            auto = true
            "#,
        );
        assert_eq!(rules.matches("tools/toolsnodraw").unwrap().1.action, Action::Skip);
        assert_eq!(
            rules.matches("nature/blendrubble").unwrap().1.action,
            Action::Auto(None)
        );
    }

    #[test]
    fn auto_rules_may_name_a_block_set() {
        let rules = rules("[[rule]]\nmatch = \"wood/*\"\nauto = true\nset = \"wood\"\n");
        assert_eq!(
            rules.matches("wood/woodwall001a").unwrap().1.action,
            Action::Auto(Some(crate::palette::blocks::SET_WOOD))
        );
    }

    #[test]
    fn a_set_without_auto_is_rejected() {
        let err = Rules::parse(
            "[[rule]]\nmatch = \"a/*\"\nblock = \"minecraft:stone\"\nset = \"wood\"\n",
            "test",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("`set` only"), "{err:#}");
    }

    #[test]
    fn an_unknown_set_in_a_rule_is_rejected() {
        let err =
            Rules::parse("[[rule]]\nmatch = \"a/*\"\nauto = true\nset = \"cheese\"\n", "test")
                .unwrap_err();
        assert!(format!("{err:#}").contains("cheese"), "{err:#}");
    }

    #[test]
    fn a_rule_without_an_action_is_rejected() {
        let err = Rules::parse("[[rule]]\nmatch = \"a/*\"\n", "test").unwrap_err();
        assert!(err.to_string().contains("rule 1"), "{err:#}");
    }

    #[test]
    fn conflicting_actions_are_rejected() {
        let err = Rules::parse(
            "[[rule]]\nmatch = \"a/*\"\nblock = \"minecraft:stone\"\nskip = true\n",
            "test",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("mutually exclusive"), "{err:#}");
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let err =
            Rules::parse("[[rule]]\nmatch = \"a/*\"\nblok = \"x\"\n", "test").unwrap_err();
        assert!(format!("{err:#}").contains("unknown field"), "{err:#}");
    }

    /// User rules are concatenated ahead of the built-in ones, so they win.
    #[test]
    fn concatenation_keeps_the_first_set_in_front() {
        let user = rules("[[rule]]\nmatch = \"metal/*\"\nblock = \"minecraft:gold_block\"\n");
        let builtin = rules("[[rule]]\nmatch = \"metal/*\"\nblock = \"minecraft:iron_block\"\n");
        let combined = user.followed_by(builtin).unwrap();
        assert_eq!(combined.len(), 2);
        assert_eq!(
            combined.matches("metal/metalwall001a").unwrap().1.action,
            Action::Block("minecraft:gold_block".into())
        );
    }

    #[test]
    fn unused_rules_are_reported() {
        let rules = rules(
            r#"
            [[rule]]
            match = "metal/*"
            block = "minecraft:iron_block"

            [[rule]]
            match = "lava/*"
            block = "minecraft:magma_block"
            "#,
        );
        let unused = rules.unused(["metal/metalwall001a"].into_iter());
        assert_eq!(unused, vec![1]);
    }
}
