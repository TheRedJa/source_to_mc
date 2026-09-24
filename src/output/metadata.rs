//! Canonical JSON metadata carried by a version-1 campaign bundle.

use crate::output::bundle::{Bundle, canonical_f64, canonical_json, validate_id};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const CAMPAIGN_PATH: &str = "campaign.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Campaign {
    pub format: &'static str,
    pub version: u32,
    pub campaign_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atlas: Option<String>,
    pub maps: Vec<CampaignMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AtlasMetadata {
    pub format: &'static str,
    pub version: u32,
    pub page_size: u32,
    pub max_mip_level: u8,
    pub gutter: u32,
    pub pages: Vec<AtlasPage>,
    pub textures: Vec<AtlasTexture>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AtlasPage {
    pub page: u32,
    pub mips: Vec<AtlasMip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AtlasMip {
    pub level: u8,
    pub content_id: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AtlasTexture {
    pub content_id: String,
    pub width: u32,
    pub height: u32,
    pub regions: Vec<AtlasRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct AtlasRegion {
    pub source: [u32; 4],
    pub page: u32,
    pub allocation: [u32; 4],
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CampaignMap {
    pub map_id: String,
    pub metadata: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MapMetadata {
    pub format: &'static str,
    pub version: u32,
    pub map_id: String,
    pub source_name: String,
    pub units_per_block: f64,
    /// Inclusive map-local cell bounds occupied by the schematic payload.
    pub cell_min: [i32; 3],
    pub cell_max: [i32; 3],
    pub anchor_cell: [i32; 3],
    pub surfaces: String,
    pub materials: Vec<MaterialReference>,
    pub models: Vec<ModelReference>,
    pub props: String,
    /// Optional prop visibility table. Absent when the map has no usable PVS.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pvs: Option<String>,
    /// Optional light-occlusion mask. Absent when the map draws no brush as
    /// geometry, or was exported with the mask turned off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occlusion: Option<String>,
    pub diagnostics: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MaterialReference {
    /// Authored, normalized Source material path. Diagnostic provenance only.
    pub source_material: String,
    /// Exact BSP material spelling when it differs from `source_material`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_material_raw: Option<String>,
    pub render_class: RenderClass,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_prop: Option<String>,
    pub reflectivity: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderClass {
    Solid,
    Cutout,
    Translucent,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextureReference {
    pub content_id: String,
    pub original_width: u32,
    pub original_height: u32,
    pub output_width: u32,
    pub output_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ModelReference {
    pub content_id: String,
    /// Normalized Source model path. Diagnostic provenance only.
    pub source_model: String,
    /// Map-local material IDs indexed by the mesh's material slots.
    pub materials: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    pub format: &'static str,
    pub version: u32,
    pub messages: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    /// Stable machine-readable values, sorted by key by construction.
    pub context: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Campaign {
    pub fn new(campaign_id: impl Into<String>, maps: Vec<CampaignMap>) -> Result<Self> {
        Self::with_atlas(campaign_id, None, maps)
    }

    pub fn with_atlas(
        campaign_id: impl Into<String>,
        atlas: Option<String>,
        mut maps: Vec<CampaignMap>,
    ) -> Result<Self> {
        let campaign_id = campaign_id.into();
        validate_id(&campaign_id, "campaign")?;
        ensure!(
            atlas.as_deref().is_none_or(|path| path == "atlas.json"),
            "non-canonical atlas path"
        );
        crate::output::limits::check_count(
            "campaign map",
            maps.len() as u64,
            crate::output::limits::MAX_MAPS,
        )?;
        maps.sort();
        let mut ids = BTreeSet::new();
        for map in &maps {
            validate_id(&map.map_id, "map")?;
            ensure!(ids.insert(&map.map_id), "duplicate map ID `{}`", map.map_id);
            ensure!(
                map.metadata == format!("maps/{}.json", map.map_id),
                "non-canonical metadata path for map `{}`",
                map.map_id
            );
        }
        Ok(Self {
            format: "src2mc-campaign-metadata",
            version: 1,
            campaign_id,
            atlas,
            maps,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        // Reconstructing also verifies callers did not mutate public fields
        // into a non-canonical order before encoding.
        ensure!(
            Self::with_atlas(
                self.campaign_id.clone(),
                self.atlas.clone(),
                self.maps.clone()
            )? == *self,
            "campaign metadata is not canonical"
        );
        canonical_json(self)
    }
}

impl AtlasMetadata {
    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.format == "src2mc-atlas" && self.version == 1,
            "invalid atlas schema"
        );
        ensure!(
            self.page_size == crate::output::atlas::PAGE_SIZE
                && self.max_mip_level == crate::output::atlas::MAX_MIP_LEVEL
                && self.gutter == crate::output::atlas::GUTTER,
            "non-canonical atlas parameters"
        );
        ensure!(
            self.pages
                .iter()
                .enumerate()
                .all(|(i, p)| p.page == i as u32),
            "atlas pages are not canonical"
        );
        ensure!(
            self.textures
                .windows(2)
                .all(|p| p[0].content_id < p[1].content_id),
            "atlas textures are not uniquely sorted"
        );
        for page in &self.pages {
            ensure!(
                page.mips.len() == usize::from(self.max_mip_level) + 1,
                "atlas mip chain is incomplete"
            );
            for (level, mip) in page.mips.iter().enumerate() {
                validate_content_id(&mip.content_id)?;
                ensure!(
                    mip.level as usize == level
                        && mip.width == self.page_size >> level
                        && mip.height == self.page_size >> level,
                    "invalid atlas mip"
                );
            }
        }
        for texture in &self.textures {
            validate_content_id(&texture.content_id)?;
            ensure!(
                texture.width > 0
                    && texture.height > 0
                    && texture.width <= 16_384
                    && texture.height <= 16_384,
                "invalid logical texture dimensions"
            );
            ensure!(
                !texture.regions.is_empty(),
                "logical texture has no atlas regions"
            );
        }
        canonical_json(self)
    }
}

impl MapMetadata {
    pub fn encode(mut self) -> Result<Vec<u8>> {
        ensure!(
            self.format == "src2mc-map" && self.version == 1,
            "invalid map schema"
        );
        validate_id(&self.map_id, "map")?;
        ensure!(
            !self.source_name.is_empty(),
            "source map name must not be empty"
        );
        self.units_per_block = canonical_f64(self.units_per_block)?;
        ensure!(
            self.units_per_block > 0.0,
            "units per block must be positive"
        );
        for axis in 0..3 {
            ensure!(
                self.cell_min[axis] <= self.cell_max[axis],
                "invalid cell bounds"
            );
        }
        ensure!(
            self.surfaces == format!("maps/{}/surfaces.s2faces", self.map_id),
            "non-canonical surface path"
        );
        ensure!(
            self.diagnostics == format!("maps/{}/diagnostics.json", self.map_id),
            "non-canonical diagnostics path"
        );
        ensure!(
            self.props == format!("maps/{}/props.s2props", self.map_id),
            "non-canonical prop path"
        );
        if let Some(pvs) = &self.pvs {
            ensure!(
                pvs == &format!("maps/{}/pvs.s2pvs", self.map_id),
                "non-canonical pvs path"
            );
        }
        if let Some(occlusion) = &self.occlusion {
            ensure!(
                occlusion == &format!("maps/{}/occlusion.s2occl", self.map_id),
                "non-canonical occlusion path"
            );
        }
        for material in &mut self.materials {
            material.validate()?;
        }
        crate::output::limits::check_count(
            "map material",
            self.materials.len() as u64,
            crate::output::limits::MAX_MATERIALS_PER_MAP,
        )?;
        crate::output::limits::check_count(
            "map model",
            self.models.len() as u64,
            crate::output::limits::MAX_MODELS_PER_CAMPAIGN,
        )?;
        ensure!(
            self.models.windows(2).all(|pair| pair[0] < pair[1]),
            "model references must be unique and canonically sorted"
        );
        for model in &self.models {
            validate_content_id(&model.content_id)?;
            ensure!(
                !model.source_model.is_empty(),
                "source model path must not be empty"
            );
            ensure!(
                model
                    .materials
                    .iter()
                    .all(|id| (*id as usize) < self.materials.len()),
                "model material reference out of range"
            );
            ensure!(!model.materials.is_empty(), "model has no material slots");
        }
        canonical_json(&self)
    }
}

impl MaterialReference {
    fn validate(&mut self) -> Result<()> {
        ensure!(
            !self.source_material.is_empty(),
            "source material path must not be empty"
        );
        if self.source_material_raw.as_deref() == Some(&self.source_material) {
            self.source_material_raw = None;
        }
        for value in &mut self.reflectivity {
            *value = canonical_f64(*value)?;
        }
        if let Some(texture) = &self.texture {
            validate_content_id(&texture.content_id)?;
            ensure!(
                texture.original_width > 0
                    && texture.original_height > 0
                    && texture.output_width > 0
                    && texture.output_height > 0,
                "texture dimensions must be positive"
            );
            ensure!(
                u64::from(texture.original_width) <= crate::output::limits::MAX_TEXTURE_DIMENSION
                    && u64::from(texture.original_height)
                        <= crate::output::limits::MAX_TEXTURE_DIMENSION
                    && u64::from(texture.output_width)
                        <= crate::output::limits::MAX_OUTPUT_TEXTURE_DIMENSION
                    && u64::from(texture.output_height)
                        <= crate::output::limits::MAX_OUTPUT_TEXTURE_DIMENSION,
                "texture dimensions exceed format limits"
            );
        }
        Ok(())
    }
}

impl Diagnostics {
    pub fn new(mut messages: Vec<Diagnostic>) -> Result<Self> {
        for message in &messages {
            validate_code(&message.code)?;
            ensure!(
                !message.message.is_empty(),
                "diagnostic message must not be empty"
            );
        }
        messages.sort();
        Ok(Self {
            format: "src2mc-diagnostics",
            version: 1,
            messages,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.format == "src2mc-diagnostics" && self.version == 1,
            "invalid diagnostics schema"
        );
        ensure!(
            Self::new(self.messages.clone())? == *self,
            "diagnostics are not canonical"
        );
        canonical_json(self)
    }
}

pub fn add_campaign(bundle: &mut Bundle, campaign: &Campaign) -> Result<()> {
    bundle.add(CAMPAIGN_PATH, campaign.encode()?)
}

pub fn add_map(bundle: &mut Bundle, map: MapMetadata, diagnostics: &Diagnostics) -> Result<()> {
    let map_id = map.map_id.clone();
    bundle.add(format!("maps/{map_id}.json"), map.encode()?)?;
    bundle.add(
        format!("maps/{map_id}/diagnostics.json"),
        diagnostics.encode()?,
    )
}

pub(crate) fn validate_content_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 64
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid content ID `{id}`"
    );
    Ok(())
}

fn validate_code(code: &str) -> Result<()> {
    ensure!(
        !code.is_empty()
            && code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
        "invalid diagnostic code `{code}`"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    #[test]
    fn campaign_and_diagnostics_order_is_deterministic() {
        let campaign = Campaign::new(
            "hl2",
            vec![
                CampaignMap {
                    map_id: "d1_02".into(),
                    metadata: "maps/d1_02.json".into(),
                },
                CampaignMap {
                    map_id: "d1_01".into(),
                    metadata: "maps/d1_01.json".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(campaign.maps[0].map_id, "d1_01");

        let diagnostics = Diagnostics::new(vec![
            Diagnostic {
                severity: Severity::Warning,
                code: "Z_LAST".into(),
                message: "z".into(),
                context: BTreeMap::new(),
            },
            Diagnostic {
                severity: Severity::Info,
                code: "A_FIRST".into(),
                message: "a".into(),
                context: BTreeMap::new(),
            },
        ])
        .unwrap();
        assert_eq!(diagnostics.messages[0].code, "A_FIRST");
    }

    #[test]
    fn map_normalizes_floats_and_requires_model_order() {
        let mut map = MapMetadata {
            format: "src2mc-map",
            version: 1,
            map_id: "d1_01".into(),
            source_name: "d1_01".into(),
            units_per_block: 32.0,
            cell_min: [-1, 0, 2],
            cell_max: [4, 5, 6],
            anchor_cell: [0, 0, 0],
            surfaces: "maps/d1_01/surfaces.s2faces".into(),
            materials: vec![MaterialReference {
                source_material: "brick/wall".into(),
                source_material_raw: Some("brick/wall".into()),
                render_class: RenderClass::Solid,
                texture: None,
                surface_prop: Some("brick".into()),
                reflectivity: [-0.0, 0.5, 1.0],
            }],
            models: vec![
                ModelReference {
                    content_id: id('a'),
                    source_model: "a.mdl".into(),
                    materials: vec![0],
                },
                ModelReference {
                    content_id: id('b'),
                    source_model: "b.mdl".into(),
                    materials: vec![0],
                },
            ],
            props: "maps/d1_01/props.s2props".into(),
            pvs: Some("maps/d1_01/pvs.s2pvs".into()),
            occlusion: Some("maps/d1_01/occlusion.s2occl".into()),
            diagnostics: "maps/d1_01/diagnostics.json".into(),
        };
        let value: serde_json::Value =
            serde_json::from_slice(&map.clone().encode().unwrap()).unwrap();
        assert_eq!(value["materials"][0]["reflectivity"][0], 0.0);
        assert!(value["materials"][0].get("source_material_raw").is_none());
        assert!(value["materials"][0].get("texture").is_none());
        assert_eq!(value["models"][0]["content_id"], id('a'));
        assert_eq!(value["pvs"], "maps/d1_01/pvs.s2pvs");
        assert_eq!(value["occlusion"], "maps/d1_01/occlusion.s2occl");

        map.models.swap(0, 1);
        assert!(map.encode().is_err());
    }

    #[test]
    fn rejects_invalid_references_and_values() {
        assert!(Campaign::new("Bad ID", vec![]).is_err());
        assert!(
            Diagnostics::new(vec![Diagnostic {
                severity: Severity::Error,
                code: "bad-code".into(),
                message: "bad".into(),
                context: BTreeMap::new(),
            }])
            .is_err()
        );
    }
}
