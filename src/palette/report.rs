//! The `materials` report: what every material in a map resolves to, and why.
//!
//! This is the tuning loop. Convert a map, look at what the colour matcher
//! guessed for the materials that cover the most surface, and promote the ones
//! it got wrong into rules — which the report emits ready to paste.

use crate::bsp::Map;
use crate::config::Config;
use crate::palette::Resolver;
use crate::palette::color::hex;
use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MaterialReport {
    pub map: String,
    pub entries: Vec<Entry>,
    /// Rules that matched nothing in this map.
    pub unused_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub material: String,
    /// Brush sides using it, which is how much of the map it decides.
    pub uses: usize,
    /// Average texture colour, `#rrggbb`.
    pub color: String,
    /// `None` when faces with this material are dropped.
    pub block: Option<String>,
    /// `rule 4`, `auto`, `fallback` or `tool`.
    pub decided_by: String,
    /// Whether any rule matched at all. A rule can match and still hand the
    /// choice to colour matching, so this is not implied by `decided_by`.
    pub matched_rule: bool,
}

/// Resolve every material in `map` and describe the result.
pub fn report(map: &Map, config: &Config) -> Result<MaterialReport> {
    let resolver = Resolver::new(config, map.materials())?;
    let usage = map.material_usage();

    // Several texture-data entries can share a normalized name once cubemap
    // patching is undone, so merge them and sum their usage.
    let mut entries: Vec<Entry> = Vec::new();
    for (index, assignment) in resolver.assignments().iter().enumerate() {
        let uses = usage.get(index).copied().unwrap_or(0);
        match entries
            .iter_mut()
            .find(|e| e.material == assignment.material)
        {
            Some(existing) => existing.uses += uses,
            None => entries.push(Entry {
                material: assignment.material.clone(),
                uses,
                color: hex(assignment.color),
                block: assignment.block.clone(),
                decided_by: assignment.source.label(),
                matched_rule: assignment.rule.is_some(),
            }),
        }
    }

    // Most-used first: those are the ones worth writing a rule for.
    entries.sort_by(|a, b| {
        b.uses
            .cmp(&a.uses)
            .then_with(|| a.material.cmp(&b.material))
    });

    let rules = resolver.rules();
    let unused_rules = rules
        .unused(entries.iter().map(|e| e.material.as_str()))
        .into_iter()
        .filter_map(|index| rules.get(index))
        .map(|rule| rule.patterns.join(", "))
        .collect();

    Ok(MaterialReport {
        map: map.name.clone(),
        entries,
        unused_rules,
    })
}

impl MaterialReport {
    /// Only the materials nothing decided for them but their colour.
    pub fn guessed(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .iter()
            .filter(|e| e.decided_by == "auto" || e.decided_by == "fallback")
    }

    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();

        let width = self
            .entries
            .iter()
            .map(|e| e.material.len())
            .max()
            .unwrap_or(8)
            .clamp(8, 52);

        let _ = writeln!(
            s,
            "{:<width$}  {:>6}  {:<7}  {:<28}  by",
            "material", "uses", "colour", "block"
        );
        for entry in &self.entries {
            let _ = writeln!(
                s,
                "{:<width$}  {:>6}  {:<7}  {:<28}  {}",
                truncate(&entry.material, width),
                entry.uses,
                entry.color,
                entry.block.as_deref().unwrap_or("-"),
                entry.decided_by,
            );
        }

        let guessed = self.guessed().count();
        let _ = writeln!(
            s,
            "\n{} materials, {guessed} decided by colour alone",
            self.entries.len()
        );

        if !self.unused_rules.is_empty() {
            let _ = writeln!(
                s,
                "\n{} rules matched nothing in this map:",
                self.unused_rules.len()
            );
            for patterns in &self.unused_rules {
                let _ = writeln!(s, "    {patterns}");
            }
        }
        s
    }

    /// The colour-matched materials as rules, ready to paste into a rules file
    /// and edit. Emitting what the matcher already chose means a stub is a
    /// no-op until you change it.
    pub fn stubs(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        let _ = writeln!(
            s,
            "# Materials in {} that were decided by colour alone, as rules.\n\
             # The blocks are what the matcher picked; change the ones it got wrong.",
            self.map
        );
        for entry in self.guessed() {
            let Some(block) = &entry.block else { continue };
            let _ = writeln!(
                s,
                "\n# {} sides, {}\n[[rule]]\nmatch = \"{}\"\nblock = \"{block}\"",
                entry.uses, entry.color, entry.material,
            );
        }
        s
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.len() <= width {
        return text.to_string();
    }
    // Keep the tail: material names differ at the end far more than the start.
    format!("~{}", &text[text.len() - (width - 1)..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn sample_map() -> Option<Map> {
        let path = Path::new(concat!(
            "/mnt/games/SteamLibrary/steamapps/common/Entropy Zero",
            "/Entropy Zero/EntropyZero/maps/az_c4_4.bsp"
        ));
        path.exists().then(|| Map::load(path).unwrap())
    }

    #[test]
    fn truncation_keeps_the_distinguishing_tail() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("concrete/concretewall001a", 12), "~etewall001a");
    }

    #[test]
    fn reports_a_real_map() {
        let Some(map) = sample_map() else { return };
        let report = report(&map, &Config::default()).unwrap();

        assert!(!report.entries.is_empty());
        // The busiest materials must come first, since that is what makes the
        // report worth reading.
        for pair in report.entries.windows(2) {
            assert!(pair[0].uses >= pair[1].uses);
        }
        // Normalization should have merged the cubemap-patched duplicates.
        let mut names: Vec<&str> = report.entries.iter().map(|e| e.material.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate materials in the report");

        assert!(report.render().contains("material"));
    }

    /// Colour matching is meant to decide most surfaces, but the *fallback* is
    /// the failure case: it means neither a rule nor a colour had an answer, so
    /// the surface becomes an undifferentiated block. Almost nothing in a real
    /// map should reach it.
    #[test]
    fn almost_nothing_in_a_real_map_reaches_the_fallback() {
        let Some(map) = sample_map() else { return };
        let report = report(&map, &Config::default()).unwrap();

        let used: usize = report.entries.iter().map(|e| e.uses).sum();
        let fallback: usize = report
            .entries
            .iter()
            .filter(|e| e.decided_by == "fallback")
            .map(|e| e.uses)
            .sum();
        assert!(used > 0);
        assert!(
            fallback * 20 < used,
            "{fallback} of {used} brush sides fell back to a default block"
        );
    }

    /// Every material family a real map uses should be routed by some rule, so
    /// that colour matching runs against the right family of blocks rather
    /// than the whole palette.
    #[test]
    fn the_builtin_rules_route_the_material_families_of_a_real_map() {
        let Some(map) = sample_map() else { return };
        let report = report(&map, &Config::default()).unwrap();

        let unrouted: Vec<&str> = report
            .entries
            .iter()
            .filter(|e| e.uses > 0 && !e.matched_rule && e.decided_by != "tool")
            .map(|e| e.material.as_str())
            .collect();
        assert!(unrouted.is_empty(), "no built-in rule covers {unrouted:?}");
    }

    /// The few materials that fall back are the ones Source stores no colour
    /// for at all — self-illuminated screens and logos — not gaps in the rules.
    #[test]
    fn fallbacks_are_only_materials_with_no_colour() {
        let Some(map) = sample_map() else { return };
        let report = report(&map, &Config::default()).unwrap();
        for entry in report.entries.iter().filter(|e| e.decided_by == "fallback") {
            assert_eq!(
                entry.color, "#000000",
                "{} fell back despite having a colour",
                entry.material
            );
        }
    }

    #[test]
    fn stubs_are_parseable_rules() {
        let Some(map) = sample_map() else { return };
        let report = report(&map, &Config::default()).unwrap();
        let stubs = report.stubs();
        let rules = crate::palette::rules::Rules::parse(&stubs, "stubs").unwrap();
        assert_eq!(
            rules.len(),
            report.guessed().filter(|e| e.block.is_some()).count()
        );
    }
}
