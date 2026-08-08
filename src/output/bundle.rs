//! Writing the asset bundle the companion mod reads.
//!
//! The contract is `docs/format.md` §4, which is normative: where it and this
//! module disagree, this module has a bug.
//!
//! The difference from [`crate::output::kubejs`] is the whole reason the mod
//! exists. A KubeJS pack registers a block per material *per tile of its
//! texture*, so a wall texture spanning eight blocks becomes eight blocks, eight
//! models and eight blockstates. Here a material is one entry, one texture and
//! one index into a fixed pool of registered blocks, and the repetition is a
//! number the mod projects from world position (D10).
//!
//! Nothing in this module is content-dependent on the Minecraft side: whatever
//! a campaign contains, the mod registers `POOL_SIZE` blocks and no more.

use crate::output::kubejs::RenderType;
use crate::source::vmt::MaterialAssets;
use anyhow::{Context, Result, bail};
use image::RgbaImage;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The namespace the mod registers its blocks in.
pub const NAMESPACE: &str = "src2mc";

/// How many surface blocks the mod registers.
///
/// A compile-time constant on both sides, deliberately: the point of the format
/// is that the registry does not grow with the maps converted. Overrunning it is
/// a hard error naming the count, never a silent reuse of an index — two
/// materials sharing an index would paint one of them with the other's texture
/// everywhere it appears.
///
/// The number itself is provisional. How many a campaign really needs has not
/// been measured across Entropy: Zero 1 and 2 yet; that is an open question in
/// `docs/decisions.md`.
pub const POOL_SIZE: usize = 4096;

/// One material: a texture, and how it lies on the surfaces wearing it.
#[derive(Debug, Clone)]
pub struct Material {
    /// The Source material path, kept for diagnostics only. The mod matches on
    /// index, never on this.
    pub material: String,
    pub texture: RgbaImage,
    /// How many blocks one repeat of the texture covers along each axis,
    /// measured from the map's own texture vectors.
    pub blocks_per_repeat: [f64; 2],
    pub render_type: RenderType,
    pub surface_prop: Option<String>,
}

impl Material {
    /// File name for this material's texture, unique within a bundle because
    /// it is derived from the material path the bundle keys on.
    fn texture_file(&self) -> String {
        format!(
            "textures/{}.png",
            crate::output::kubejs::block_id(&self.material)
        )
    }
}

/// A campaign's worth of materials, indexed into the mod's block pool.
///
/// Indices are assigned in insertion order and never move: a schematic written
/// against this bundle refers to `src2mc:surface_<index>`, so renumbering would
/// silently repaint a map.
#[derive(Debug, Default)]
pub struct Bundle {
    name: String,
    units_per_block: f64,
    /// Material path to pool index.
    index: BTreeMap<String, usize>,
    /// Pool index to material, dense from 0.
    materials: Vec<Material>,
}

/// The namespaced id of a pool block.
pub fn surface_id(index: usize) -> String {
    format!("{NAMESPACE}:surface_{index}")
}

impl Bundle {
    pub fn new(name: &str, units_per_block: f64) -> Bundle {
        Bundle {
            name: name.to_string(),
            units_per_block,
            ..Bundle::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// What the mod registers to read this bundle, which is the number that is
    /// supposed to stay constant however much is converted.
    pub fn registered(&self) -> usize {
        POOL_SIZE
    }

    pub fn materials(&self) -> impl Iterator<Item = &Material> {
        self.materials.iter()
    }

    /// Add a material, returning the block id a schematic should use.
    ///
    /// Adding the same material twice returns the index it already has, which
    /// is what makes merging a campaign into one bundle free.
    pub fn insert(
        &mut self,
        material: &str,
        texture: RgbaImage,
        blocks_per_repeat: [f64; 2],
        assets: &MaterialAssets,
    ) -> Result<String> {
        if let Some(&index) = self.index.get(material) {
            return Ok(surface_id(index));
        }
        if self.materials.len() >= POOL_SIZE {
            bail!(
                "the material pool is full at {POOL_SIZE} entries and `{material}` does not \
                 fit. The mod registers a fixed pool, so this is a limit on distinct \
                 materials per bundle, not on maps: convert fewer maps into one bundle, or \
                 raise POOL_SIZE in the mod and in src/output/bundle.rs together."
            );
        }

        let index = self.materials.len();
        self.materials.push(Material {
            material: material.to_string(),
            texture,
            // A degenerate or missing measurement means one repeat per block,
            // which is wrong but visible, rather than a division by zero.
            blocks_per_repeat: std::array::from_fn(|axis| {
                let span = blocks_per_repeat[axis];
                if span.is_finite() && span > 1e-6 {
                    span
                } else {
                    1.0
                }
            }),
            render_type: RenderType::of(assets),
            surface_prop: assets.surface_prop.clone(),
        });
        self.index.insert(material.to_string(), index);
        Ok(surface_id(index))
    }

    /// The pool index a material was given, if it has one.
    pub fn index_of(&self, material: &str) -> Option<usize> {
        self.index.get(material).copied()
    }

    /// Material path to namespaced block id, in the shape the palette wants.
    pub fn ids(&self) -> BTreeMap<String, String> {
        self.index
            .iter()
            .map(|(material, &index)| (material.clone(), surface_id(index)))
            .collect()
    }

    /// Merge another bundle in. Materials this bundle already has keep their
    /// index; the rest are appended in the other bundle's order.
    pub fn merge(&mut self, other: Bundle) -> Result<()> {
        for material in other.materials {
            if self.index.contains_key(&material.material) {
                continue;
            }
            if self.materials.len() >= POOL_SIZE {
                bail!(
                    "the material pool is full at {POOL_SIZE} entries while merging \
                     `{}`. Convert fewer maps into one bundle.",
                    material.material
                );
            }
            self.index
                .insert(material.material.clone(), self.materials.len());
            self.materials.push(material);
        }
        if self.name.is_empty() {
            self.name = other.name;
        }
        if self.units_per_block == 0.0 {
            self.units_per_block = other.units_per_block;
        }
        Ok(())
    }

    /// Write the bundle to `dir`, creating it if needed.
    pub fn write(&self, dir: &Path) -> Result<Written> {
        std::fs::create_dir_all(dir.join("textures"))
            .with_context(|| format!("creating {}", dir.join("textures").display()))?;

        let mut bytes = 0u64;
        for material in &self.materials {
            let path = dir.join(material.texture_file());
            material
                .texture
                .save(&path)
                .with_context(|| format!("writing {}", path.display()))?;
            bytes += std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        }

        let materials: Vec<MaterialJson> = self
            .materials
            .iter()
            .map(|m| MaterialJson {
                material: m.material.clone(),
                texture: m.texture_file(),
                blocks_per_repeat: m.blocks_per_repeat,
                render_type: m.render_type.name(),
                surface_prop: m.surface_prop.clone(),
                sound: crate::output::kubejs::sound_for(m.surface_prop.as_deref()),
            })
            .collect();

        let index = IndexJson {
            format_version: crate::FORMAT_VERSION,
            name: self.name.clone(),
            scale: ScaleJson {
                units_per_block: self.units_per_block,
            },
            materials: "materials.json",
            models: "models.json",
        };

        write_json(&dir.join("bundle.json"), &index)?;
        write_json(&dir.join("materials.json"), &materials)?;
        // Props are not in the bundle yet. An empty table is written anyway so
        // the mod's loader has nothing to special-case, and so a bundle written
        // today stays readable when models arrive.
        write_json(&dir.join("models.json"), &Vec::<ModelJson>::new())?;

        Ok(Written {
            materials: self.materials.len(),
            texture_bytes: bytes,
            files: self.materials.len() + 3,
        })
    }
}

/// Pretty-printed and newline-terminated: these files are read by people as
/// often as by the mod, and a diff of one is how a format change is reviewed.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)
        .with_context(|| format!("encoding {}", path.display()))?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

#[derive(Serialize)]
struct IndexJson {
    format_version: i32,
    name: String,
    scale: ScaleJson,
    materials: &'static str,
    models: &'static str,
}

#[derive(Serialize)]
struct ScaleJson {
    units_per_block: f64,
}

#[derive(Serialize)]
struct MaterialJson {
    material: String,
    texture: String,
    blocks_per_repeat: [f64; 2],
    render_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    surface_prop: Option<String>,
    sound: &'static str,
}

/// Placeholder for `models.json` until props move into the bundle.
#[derive(Serialize)]
struct ModelJson {
    mesh: String,
    materials: Vec<usize>,
}

/// What writing a bundle produced, for the run report.
#[derive(Debug, Default, Clone, Copy)]
pub struct Written {
    pub materials: usize,
    pub texture_bytes: u64,
    pub files: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Temp directory per test, in the style the other writer tests use.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("src2mc-bundle-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn assets() -> MaterialAssets {
        MaterialAssets {
            base_texture: String::new(),
            alpha_test: false,
            translucent: false,
            surface_prop: Some("concrete".into()),
        }
    }

    fn texture() -> RgbaImage {
        RgbaImage::new(4, 4)
    }

    #[test]
    fn indices_are_dense_and_stable() {
        let mut bundle = Bundle::new("test", 16.0);
        let a = bundle
            .insert("a/one", texture(), [1.0, 1.0], &assets())
            .unwrap();
        let b = bundle
            .insert("b/two", texture(), [1.0, 1.0], &assets())
            .unwrap();

        assert_eq!(a, "src2mc:surface_0");
        assert_eq!(b, "src2mc:surface_1");

        // Re-inserting must not move anything: schematics already refer to it.
        let again = bundle
            .insert("a/one", texture(), [8.0, 8.0], &assets())
            .unwrap();
        assert_eq!(again, a);
        assert_eq!(bundle.len(), 2);
    }

    /// The one thing a bundle must never do: two materials, one index.
    #[test]
    fn a_full_pool_is_an_error_rather_than_a_collision() {
        let mut bundle = Bundle::new("test", 16.0);
        for i in 0..POOL_SIZE {
            bundle
                .insert(&format!("m/{i}"), texture(), [1.0, 1.0], &assets())
                .unwrap();
        }
        let over = bundle.insert("m/one_too_many", texture(), [1.0, 1.0], &assets());
        assert!(over.is_err(), "the pool must not wrap around");
        assert_eq!(bundle.len(), POOL_SIZE);
    }

    #[test]
    fn a_missing_measurement_never_divides_by_zero() {
        let mut bundle = Bundle::new("test", 16.0);
        bundle
            .insert("a/one", texture(), [0.0, f64::NAN], &assets())
            .unwrap();
        assert_eq!(bundle.materials[0].blocks_per_repeat, [1.0, 1.0]);
    }

    #[test]
    fn merging_keeps_the_indices_a_schematic_already_used() {
        let mut first = Bundle::new("first", 16.0);
        first
            .insert("a/one", texture(), [1.0, 1.0], &assets())
            .unwrap();
        first
            .insert("b/two", texture(), [1.0, 1.0], &assets())
            .unwrap();

        let mut second = Bundle::new("second", 16.0);
        second
            .insert("b/two", texture(), [1.0, 1.0], &assets())
            .unwrap();
        second
            .insert("c/three", texture(), [1.0, 1.0], &assets())
            .unwrap();

        first.merge(second).unwrap();

        assert_eq!(first.index_of("a/one"), Some(0));
        assert_eq!(first.index_of("b/two"), Some(1));
        assert_eq!(first.index_of("c/three"), Some(2));
    }

    #[test]
    fn a_written_bundle_has_the_files_the_format_names() {
        let dir = temp_dir("write");
        let mut bundle = Bundle::new("test", 16.0);
        bundle
            .insert("concrete/wall001a", texture(), [8.0, 4.0], &assets())
            .unwrap();
        let written = bundle.write(&dir).unwrap();

        assert_eq!(written.materials, 1);
        for name in ["bundle.json", "materials.json", "models.json"] {
            assert!(dir.join(name).exists(), "missing {name}");
        }
        assert!(
            dir.join("textures/concrete_wall001a.png").exists(),
            "material texture was not written where materials.json points"
        );

        let text = std::fs::read_to_string(dir.join("materials.json")).unwrap();
        assert!(
            text.contains("\"blocks_per_repeat\": [\n      8.0,\n      4.0\n    ]"),
            "{text}"
        );
        assert!(text.contains("\"sound\": \"stone\""), "{text}");
    }

    /// The mod reads this before anything else.
    #[test]
    fn the_index_declares_the_format_version() {
        let dir = temp_dir("index");
        Bundle::new("test", 16.0).write(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join("bundle.json")).unwrap();
        assert!(
            text.contains(&format!("\"format_version\": {}", crate::FORMAT_VERSION)),
            "{text}"
        );
    }
}
