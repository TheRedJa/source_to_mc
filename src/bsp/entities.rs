//! Extracting the entity lump into a structured, Minecraft-aware manifest.
//!
//! Nothing is discarded: every key/value pair is kept verbatim alongside the
//! derived fields, so logic that this tool does not translate can still be
//! rebuilt by hand from the manifest.

use crate::geom::Vec3;
use crate::voxel::transform::Transform;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct EntityRecord {
    /// Position in the entity lump.
    pub index: usize,
    pub classname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targetname: Option<String>,
    /// Value of the `model` key, e.g. `*12` or `models/props/foo.mdl`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Model index for brush entities, parsed from a `*N` model value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brush_model: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_source: Option<[f64; 3]>,
    /// `origin_source` mapped into Minecraft block space, fractional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_mc: Option<[f64; 3]>,
    /// Pitch/yaw/roll as authored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angles: Option<[f64; 3]>,
    pub properties: BTreeMap<String, String>,
}

impl EntityRecord {
    /// Entities that name a target, for rebuilding the I/O graph.
    pub fn targets(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for key in ["target", "filtername", "parentname"] {
            if let Some(value) = self.properties.get(key) {
                out.push(value.as_str());
            }
        }
        // Outputs look like `OnTrigger,targetname,input,param,delay,times`.
        for (key, value) in &self.properties {
            if key.starts_with("On") {
                if let Some((target, _)) = value.split_once(&[',', '\u{1b}'][..]) {
                    if !target.is_empty() {
                        out.push(target);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Parse a whitespace-separated triple such as an `origin` or `angles` value.
fn parse_triple(value: &str) -> Option<[f64; 3]> {
    let mut parts = value.split_whitespace().filter_map(|p| p.parse::<f64>().ok());
    let (x, y, z) = (parts.next()?, parts.next()?, parts.next()?);
    parts.next().is_none().then_some([x, y, z])
}

/// Parse a brush-entity model reference (`*12`) into its model index.
fn parse_brush_model(value: &str) -> Option<usize> {
    value.strip_prefix('*')?.parse().ok()
}

/// Read every entity from the map, mapping positions through `transform`.
pub fn extract(map: &crate::bsp::Map, transform: &Transform) -> Vec<EntityRecord> {
    map.bsp
        .entities
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            let properties: BTreeMap<String, String> = raw
                .properties()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            let origin_source = properties.get("origin").and_then(|v| parse_triple(v));
            let model = properties.get("model").cloned();

            EntityRecord {
                index,
                classname: properties
                    .get("classname")
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".into()),
                targetname: properties.get("targetname").cloned(),
                brush_model: model.as_deref().and_then(parse_brush_model),
                model,
                origin_source,
                origin_mc: origin_source.map(|o| {
                    let p = transform.to_block_space(Vec3::new(o[0], o[1], o[2]));
                    [p.x, p.y, p.z]
                }),
                angles: properties.get("angles").and_then(|v| parse_triple(v)),
                properties,
            }
        })
        .collect()
}

/// Count of each classname, most frequent first.
pub fn classname_histogram(entities: &[EntityRecord]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for entity in entities {
        *counts.entry(entity.classname.as_str()).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed_triples() {
        assert_eq!(parse_triple("1 2 3"), Some([1.0, 2.0, 3.0]));
        assert_eq!(parse_triple("-64.5  0   1024"), Some([-64.5, 0.0, 1024.0]));
    }

    #[test]
    fn rejects_malformed_triples() {
        assert_eq!(parse_triple("1 2"), None);
        assert_eq!(parse_triple("1 2 3 4"), None);
        assert_eq!(parse_triple(""), None);
        assert_eq!(parse_triple("a b c"), None);
    }

    #[test]
    fn parses_brush_model_references() {
        assert_eq!(parse_brush_model("*12"), Some(12));
        assert_eq!(parse_brush_model("*0"), Some(0));
        assert_eq!(parse_brush_model("models/props/crate.mdl"), None);
        assert_eq!(parse_brush_model("*"), None);
    }

    #[test]
    fn collects_targets_from_keys_and_outputs() {
        let mut properties = BTreeMap::new();
        properties.insert("target".to_string(), "door_a".to_string());
        properties.insert("OnTrigger".to_string(), "light_b,Toggle,,0,-1".to_string());
        let entity = EntityRecord {
            index: 0,
            classname: "trigger_once".into(),
            targetname: None,
            model: None,
            brush_model: None,
            origin_source: None,
            origin_mc: None,
            angles: None,
            properties,
        };
        assert_eq!(entity.targets(), vec!["door_a", "light_b"]);
    }
}
