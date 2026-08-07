//! Emitting the map's own textures as Minecraft blocks, via KubeJS.
//!
//! Colour matching gets a wall to the right shade of grey. This gets it the
//! actual concrete. Minecraft cannot add blocks from a resource pack alone —
//! a pack can only retexture blocks that already exist — so something has to
//! register them, and KubeJS is the least intrusive way to do that: its
//! `kubejs/assets/` folder is loaded exactly like a resource pack, and a
//! startup script registers the blocks under its own namespace, so nothing
//! vanilla is overwritten.
//!
//! The output drops straight into a NeoForge instance:
//!
//! ```text
//! kubejs/assets/kubejs/textures/block/<id>.png
//! kubejs/startup_scripts/src2mc_blocks.js
//! ```
//!
//! The catch is that a schematic using these blocks only pastes correctly
//! where they are registered. Both halves are written into the same output
//! directory and the required ids are listed in the manifest, so the pairing
//! is at least visible.

use crate::source::vmt::MaterialAssets;
use anyhow::{Context, Result};
use image::RgbaImage;
use std::collections::BTreeMap;
use std::path::Path;

/// The namespace KubeJS registers scripted content under.
pub const NAMESPACE: &str = "kubejs";

/// How Minecraft should draw a block's faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderType {
    /// Fully opaque.
    Solid,
    /// Alpha-tested: each texel is drawn or not, like a grate or a fence.
    Cutout,
    /// Blended, like glass.
    Translucent,
}

impl RenderType {
    fn name(self) -> &'static str {
        match self {
            RenderType::Solid => "solid",
            RenderType::Cutout => "cutout",
            RenderType::Translucent => "translucent",
        }
    }

    pub fn of(assets: &MaterialAssets) -> RenderType {
        // Alpha testing is checked first: a material can set both, and a
        // cutout grate drawn translucent turns into a foggy pane.
        if assets.alpha_test {
            RenderType::Cutout
        } else if assets.translucent {
            RenderType::Translucent
        } else {
            RenderType::Solid
        }
    }
}

/// One generated block.
#[derive(Debug, Clone)]
pub struct Block {
    /// Id within the namespace, e.g. `concrete_concretewall001a`.
    pub id: String,
    /// The Source material it came from.
    pub material: String,
    pub texture: RgbaImage,
    pub render_type: RenderType,
    /// `$surfaceprop`, used to pick a sound.
    pub surface_prop: Option<String>,
    /// Which piece of the material's texture this is, when the texture is
    /// split across several blocks. `None` means the whole thing.
    pub tile: Option<[u32; 2]>,
}

impl Block {
    /// The namespaced id a schematic palette refers to.
    pub fn block_id(&self) -> String {
        format!("{NAMESPACE}:{}", self.id)
    }

    /// Something readable in the creative menu.
    fn display_name(&self) -> String {
        let leaf = self.material.rsplit('/').next().unwrap_or(&self.material);
        let mut name = String::new();
        for (i, c) in leaf.chars().enumerate() {
            if i == 0 {
                name.extend(c.to_uppercase());
            } else if c == '_' {
                name.push(' ');
            } else {
                name.push(c);
            }
        }
        // Tiles of one texture are otherwise indistinguishable in the menu.
        if let Some([u, v]) = self.tile {
            name.push_str(&format!(" {}-{}", u + 1, v + 1));
        }
        name
    }

    /// Minecraft sound group closest to Source's `$surfaceprop`.
    fn sound_type(&self) -> &'static str {
        sound_for(self.surface_prop.as_deref())
    }
}

/// Minecraft sound group closest to Source's `$surfaceprop`.
fn sound_for(surface_prop: Option<&str>) -> &'static str {
    let prop = surface_prop.unwrap_or("").to_ascii_lowercase();
    {
        for (needle, sound) in [
            ("metalgrate", "metal"),
            ("metal", "metal"),
            ("wood", "wood"),
            ("glass", "glass"),
            ("dirt", "gravel"),
            ("sand", "sand"),
            ("gravel", "gravel"),
            ("grass", "grass"),
            ("snow", "snow"),
            ("water", "wet_grass"),
            ("tile", "stone"),
            ("plaster", "stone"),
            ("concrete", "stone"),
        ] {
            if prop.contains(needle) {
                return sound;
            }
        }
        "stone"
    }
}

/// A resource pack's worth of generated blocks.
///
/// Keyed by id so that converting a whole campaign registers a shared texture
/// once rather than once per map.
#[derive(Debug, Default)]
pub struct Pack {
    blocks: BTreeMap<String, Block>,
    /// How each split material's texture was actually cut.
    ///
    /// Stored rather than recomputed, because the cut and the projection that
    /// indexes it must agree exactly. Deriving one of them again from the
    /// config is how they drifted apart twice: once stretching a tile over ten
    /// blocks of cliff, once over two.
    tiling: BTreeMap<String, crate::bsp::texcoord::Split>,
    /// Prop models rendered as their own mesh, keyed by id.
    props: BTreeMap<String, crate::output::obj::PropAsset>,
    /// The textures those meshes wear, keyed by path under `textures/`.
    prop_textures: BTreeMap<String, RgbaImage>,
    /// Whether the OBJ files were written for a flipped V axis.
    flip_v: bool,
}

/// Turn a Source material path into a Minecraft resource id.
///
/// Resource ids allow only `[a-z0-9_.-]`, and `/` would make the id look like
/// a folder to Minecraft's resource loader, so it becomes `_`.
pub fn block_id(material: &str) -> String {
    let mut id: String = material
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' | '.' | '-' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            _ => '_',
        })
        .collect();
    // A leading digit or dot is legal but confusing; a leading `_` is not.
    while id.starts_with('_') {
        id.remove(0);
    }
    if id.is_empty() { "material".into() } else { id }
}

impl Pack {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty() && self.props.is_empty()
    }

    /// Add a prop model and the textures it wears.
    ///
    /// Keyed by id, so a campaign's shared crate is one mesh however many maps
    /// place it — the same thing that makes merging texture packs free.
    pub fn insert_prop(
        &mut self,
        asset: crate::output::obj::PropAsset,
        textures: Vec<crate::output::obj::PropTexture>,
    ) -> String {
        let id = asset.block_id();
        for texture in textures {
            self.prop_textures
                .entry(texture.name)
                .or_insert(texture.image);
        }
        self.props.entry(asset.id.clone()).or_insert(asset);
        id
    }

    /// Record how the prop meshes were written, so the model JSON agrees.
    pub fn set_flip_v(&mut self, flip_v: bool) {
        self.flip_v = flip_v;
    }

    /// The prop model registered under `id`, if any.
    pub fn prop(&self, id: &str) -> Option<&crate::output::obj::PropAsset> {
        self.props
            .get(id.strip_prefix(&format!("{NAMESPACE}:")).unwrap_or(id))
    }

    pub fn props(&self) -> impl Iterator<Item = &crate::output::obj::PropAsset> {
        self.props.values()
    }

    /// The id for a shape of a generated block.
    ///
    /// Only ever the full cube. KubeJS 2101 registers blocks through exactly
    /// two builders, `basic` and `detector` — there is no slab or stairs type
    /// — so a generated block cannot take a sub-block shape, and claiming one
    /// would name a block that never gets registered. Shape fitting therefore
    /// applies to the vanilla blocks in a conversion and not to these.
    pub fn shaped(&self, block_id: &str, variant: crate::voxel::shapes::Variant) -> Option<String> {
        use crate::voxel::shapes::Variant;
        let id = block_id.strip_prefix(&format!("{NAMESPACE}:"))?;
        if !self.blocks.contains_key(id) || variant != Variant::Full {
            return None;
        }
        Some(block_id.to_string())
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.blocks.values()
    }

    /// Register a material's texture, returning the namespaced block id.
    ///
    /// Registering the same material twice is a no-op, which is what makes
    /// merging a campaign into one pack free.
    pub fn insert(
        &mut self,
        material: &str,
        texture: RgbaImage,
        assets: &MaterialAssets,
    ) -> String {
        let whole = crate::bsp::texcoord::Split {
            grid: [1, 1],
            texels_per_tile: [1.0, 1.0],
            window: [u32::MAX; 2],
        };
        self.insert_tiled(material, std::slice::from_ref(&texture), whole, assets)
    }

    /// Register a material split into a `grid` of tiles, one block each, and
    /// return the id of the first.
    ///
    /// `tiles` is row-major, as [`crate::source::vtf::decode_tiles`] returns
    /// it. A 1x1 grid is an ordinary single block and is not recorded as
    /// tiled, so nothing downstream has to special-case it.
    pub fn insert_tiled(
        &mut self,
        material: &str,
        tiles: &[RgbaImage],
        split: crate::bsp::texcoord::Split,
        assets: &MaterialAssets,
    ) -> String {
        let base = block_id(material);
        let (columns, rows) = (split.grid[0].max(1), split.grid[1].max(1));
        let split_it = columns > 1 || rows > 1;

        let mut first = format!("{NAMESPACE}:{base}");
        for row in 0..rows {
            for column in 0..columns {
                let Some(texture) = tiles.get((row * columns + column) as usize) else {
                    continue;
                };
                let tile = split_it.then_some([column, row]);
                let id = match tile {
                    Some([u, v]) => format!("{base}_{u}_{v}"),
                    None => base.clone(),
                };
                let block = Block {
                    id: id.clone(),
                    material: material.to_string(),
                    texture: texture.clone(),
                    render_type: RenderType::of(assets),
                    surface_prop: assets.surface_prop.clone(),
                    tile,
                };
                if row == 0 && column == 0 {
                    first = block.block_id();
                }
                self.blocks.entry(id).or_insert(block);
            }
        }
        if split_it {
            self.tiling.insert(material.to_string(), split);
        }
        first
    }

    /// How many blocks across and down a material's texture is split, or
    /// `[1, 1]` if it is not.
    pub fn tiling(&self, material: &str) -> [u32; 2] {
        self.tiling.get(material).map(|s| s.grid).unwrap_or([1, 1])
    }

    /// Exactly how a material's texture was cut, for the projection that has
    /// to index it.
    pub fn split(&self, material: &str) -> Option<crate::bsp::texcoord::Split> {
        self.tiling.get(material).copied()
    }

    /// Every material's tiling, for building the palette.
    pub fn tilings(&self) -> &BTreeMap<String, crate::bsp::texcoord::Split> {
        &self.tiling
    }

    /// How many blocks the pack registers, which is what a KubeJS instance
    /// pays for at startup.
    pub fn registered(&self) -> usize {
        self.blocks.len() + self.props.len()
    }

    /// The block for one tile of a material, wrapping out-of-range indices so
    /// a texture that repeats across a wall keeps repeating.
    pub fn tile_id(&self, material: &str, column: u32, row: u32) -> Option<String> {
        let base = block_id(material);
        let [columns, rows] = self.tiling(material);
        let id = if columns > 1 || rows > 1 {
            format!("{base}_{}_{}", column % columns, row % rows)
        } else {
            base
        };
        self.blocks
            .contains_key(&id)
            .then(|| format!("{NAMESPACE}:{id}"))
    }

    /// Material path to namespaced block id, for the palette.
    ///
    /// A split material reports its first tile: that is what a surface gets
    /// when nothing knows which piece of the texture belongs in front of it.
    pub fn ids(&self) -> BTreeMap<String, String> {
        self.blocks
            .values()
            .filter(|b| b.tile.is_none_or(|t| t == [0, 0]))
            .map(|b| (b.material.clone(), b.block_id()))
            .collect()
    }

    /// Merge another pack in; existing entries win.
    pub fn merge(&mut self, other: Pack) {
        for (id, block) in other.blocks {
            self.blocks.entry(id).or_insert(block);
        }
        for (material, split) in other.tiling {
            self.tiling.entry(material).or_insert(split);
        }
        for (id, asset) in other.props {
            self.props.entry(id).or_insert(asset);
        }
        for (name, image) in other.prop_textures {
            self.prop_textures.entry(name).or_insert(image);
        }
        self.flip_v |= other.flip_v;
    }

    /// Write the textures and startup script under `dir/kubejs`.
    pub fn write(&self, dir: &Path) -> Result<Written> {
        let root = dir.join("kubejs");
        let textures = root
            .join("assets")
            .join(NAMESPACE)
            .join("textures")
            .join("block");
        let scripts = root.join("startup_scripts");
        std::fs::create_dir_all(&textures)
            .with_context(|| format!("creating {}", textures.display()))?;
        std::fs::create_dir_all(&scripts)
            .with_context(|| format!("creating {}", scripts.display()))?;

        let mut bytes = 0;
        for block in self.blocks.values() {
            let png = crate::source::vtf::to_png(&block.texture)?;
            bytes += png.len();
            let path = textures.join(format!("{}.png", block.id));
            std::fs::write(&path, png).with_context(|| format!("writing {}", path.display()))?;
        }

        // Prop meshes: the OBJ and its material library, the model JSON that
        // points the loader at them, and a blockstate so the block resolves to
        // that model rather than to KubeJS's generated cube.
        if !self.props.is_empty() {
            let assets = root.join("assets").join(NAMESPACE);
            let models = assets.join("models").join("props");
            let block_models = assets.join("models").join("block");
            let blockstates = assets.join("blockstates");
            let prop_textures = assets.join("textures");
            for dir in [&models, &block_models, &blockstates, &prop_textures] {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("creating {}", dir.display()))?;
            }

            let mut libraries: std::collections::HashSet<String> = std::collections::HashSet::new();
            for asset in self.props.values() {
                let write = |path: std::path::PathBuf, contents: &str| -> Result<()> {
                    std::fs::write(&path, contents)
                        .with_context(|| format!("writing {}", path.display()))
                };
                write(models.join(format!("{}.obj", asset.id)), &asset.obj)?;
                // Every baked variant of a model wears the same materials, so
                // the `.mtl` is written once per model rather than once per
                // variant — a campaign has tens of thousands of the latter.
                if libraries.insert(asset.mtl_id.clone()) {
                    write(models.join(format!("{}.mtl", asset.mtl_id)), &asset.mtl)?;
                }
                write(
                    block_models.join(format!("{}.json", asset.id)),
                    &asset.model_json(self.flip_v),
                )?;
                write(
                    blockstates.join(format!("{}.json", asset.id)),
                    &asset.blockstate_json(),
                )?;
            }

            for (name, image) in &self.prop_textures {
                let png = crate::source::vtf::to_png(image)?;
                bytes += png.len();
                // The name carries its folders, and they decide which atlas
                // the sprite lands on, so it is written where it says.
                let path = prop_textures.join(format!("{name}.png"));
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                std::fs::write(&path, png)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
        }

        let script = scripts.join("src2mc_blocks.js");
        std::fs::write(&script, self.script())
            .with_context(|| format!("writing {}", script.display()))?;

        Ok(Written {
            root,
            blocks: self.blocks.len(),
            props: self.props.len(),
            texture_bytes: bytes,
        })
    }

    /// The KubeJS startup script registering every block.
    pub fn script(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();
        s.push_str(
            "// Generated by src2mc. Do not edit: regenerating a map overwrites it.\n\
             //\n\
             // Blocks are registered from the Source materials the map uses, so the\n\
             // schematics next to this script only paste correctly with it installed.\n\
             // Registration cannot be hot-reloaded; restart the game after adding it.\n\
             //\n\
             // Install: copy the `kubejs` folder into your instance, next to `mods`.\n\n",
        );
        let _ = writeln!(
            s,
            "// {} blocks and {} prop models\n",
            self.blocks.len(),
            self.props.len()
        );
        s.push_str("StartupEvents.registry('block', event => {\n");

        for block in self.blocks.values() {
            let mut chain = vec![
                format!("event.create('{}')", block.block_id()),
                format!("  .displayName('{}')", escape(&block.display_name())),
                // `texture`, not `textureAll`: the latter is the pre-2101 name
                // and fails at startup with "Cannot find function textureAll".
                format!("  .texture('{NAMESPACE}:block/{}')", block.id),
                // A string here is resolved by KubeJS's SoundTypeWrapper,
                // which keys the lowercased static fields of `SoundType`.
                format!("  .soundType('{}')", block.sound_type()),
                // Source surfaces are not minable in any meaningful sense;
                // these just keep the blocks from behaving like bedrock.
                "  .hardness(1.5)".to_string(),
                "  .resistance(6.0)".to_string(),
            ];
            if block.render_type != RenderType::Solid {
                chain.push(format!("  .renderType('{}')", block.render_type.name()));
            }

            let _ = writeln!(s, "  // {}", block.material);
            for (i, line) in chain.iter().enumerate() {
                let last = i + 1 == chain.len();
                let _ = writeln!(s, "  {line}{}", if last { ";" } else { "" });
            }
            s.push('\n');
        }

        // Prop models. Registered the same way, and drawn by the model files
        // written alongside; nothing here says "mesh" because as far as the
        // registry is concerned these are ordinary blocks.
        for asset in self.props.values() {
            let mut chain = vec![
                format!("event.create('{}')", asset.block_id()),
                format!("  .displayName('{}')", escape(&prop_name(&asset.model))),
                // A texture is still declared: if the hand-written blockstate
                // is ever ignored, this at least gives a recognisable cube
                // instead of the missing-texture checkerboard.
                match asset.textures.values().next() {
                    Some(texture) => format!("  .texture('{texture}')"),
                    None => format!("  .texture('{NAMESPACE}:block/{}')", asset.id),
                },
                format!(
                    "  .soundType('{}')",
                    sound_for(asset.surface_prop.as_deref())
                ),
                "  .hardness(1.5)".to_string(),
                "  .resistance(6.0)".to_string(),
                // A baked prop is placed as a real block sitting inside its own
                // mesh, so what the block does in its cell matters. Left as an
                // ordinary cube it would be a solid metre of nothing you walk
                // into, it would cull the faces of the map blocks touching it
                // — holes in the wall behind every crate — and it would stop
                // light. The mesh is what you see and the barriers are what you
                // stand on; the block itself should be neither.
                "  .noCollision()".to_string(),
                "  .notSolid()".to_string(),
                "  .opaque(false)".to_string(),
                "  .fullBlock(false)".to_string(),
            ];
            if asset.render_type != RenderType::Solid {
                chain.push(format!("  .renderType('{}')", asset.render_type.name()));
            }

            let _ = writeln!(s, "  // {} ({} triangles)", asset.model, asset.triangles);
            for (i, line) in chain.iter().enumerate() {
                let last = i + 1 == chain.len();
                let _ = writeln!(s, "  {line}{}", if last { ";" } else { "" });
            }
            s.push('\n');
        }

        s.push_str("});\n");
        s
    }
}

/// Something readable in the creative menu for a prop model.
fn prop_name(model: &str) -> String {
    let leaf = model
        .trim_end_matches(".mdl")
        .rsplit('/')
        .next()
        .unwrap_or(model);
    let mut name = String::new();
    for (i, c) in leaf.chars().enumerate() {
        if i == 0 {
            name.extend(c.to_uppercase());
        } else if c == '_' {
            name.push(' ');
        } else {
            name.push(c);
        }
    }
    name
}

/// What was written, for reporting.
#[derive(Debug, Clone)]
pub struct Written {
    pub root: std::path::PathBuf,
    pub blocks: usize,
    /// Prop models written as meshes rather than as textured cubes.
    pub props: usize,
    pub texture_bytes: usize,
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(alpha_test: bool, translucent: bool, prop: Option<&str>) -> MaterialAssets {
        MaterialAssets {
            base_texture: "x/y".into(),
            alpha_test,
            translucent,
            surface_prop: prop.map(str::to_string),
        }
    }

    fn texture() -> RgbaImage {
        RgbaImage::from_pixel(16, 16, image::Rgba([1, 2, 3, 255]))
    }

    #[test]
    fn material_paths_become_legal_resource_ids() {
        assert_eq!(
            block_id("concrete/concretewall001a"),
            "concrete_concretewall001a"
        );
        assert_eq!(block_id("Metal/MetalWall048A"), "metal_metalwall048a");
        assert_eq!(block_id("halflife/-2lab3_flr1b"), "halflife_-2lab3_flr1b");
        // Minecraft would reject anything outside [a-z0-9_.-].
        assert!(
            block_id("props/some thing!")
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-'))
        );
        assert_eq!(block_id(""), "material");
        assert_eq!(block_id("///"), "material");
    }

    #[test]
    fn render_type_follows_the_material_flags() {
        assert_eq!(
            RenderType::of(&assets(false, false, None)),
            RenderType::Solid
        );
        assert_eq!(
            RenderType::of(&assets(true, false, None)),
            RenderType::Cutout
        );
        assert_eq!(
            RenderType::of(&assets(false, true, None)),
            RenderType::Translucent
        );
        // A grate that also claims translucency must stay a cutout, or its
        // holes render as haze.
        assert_eq!(
            RenderType::of(&assets(true, true, None)),
            RenderType::Cutout
        );
    }

    #[test]
    fn sound_types_follow_surfaceprop() {
        let sound = |prop: Option<&str>| {
            Block {
                id: "x".into(),
                material: "x".into(),
                texture: texture(),
                render_type: RenderType::Solid,
                surface_prop: prop.map(str::to_string),
                tile: None,
            }
            .sound_type()
        };
        assert_eq!(sound(Some("metalgrate")), "metal");
        assert_eq!(sound(Some("wood_crate")), "wood");
        assert_eq!(sound(Some("concrete")), "stone");
        assert_eq!(sound(None), "stone");
    }

    #[test]
    fn inserting_the_same_material_twice_registers_one_block() {
        let mut pack = Pack::default();
        let a = pack.insert("concrete/wall001a", texture(), &assets(false, false, None));
        let b = pack.insert("concrete/wall001a", texture(), &assets(false, false, None));
        assert_eq!(a, b);
        assert_eq!(a, "kubejs:concrete_wall001a");
        assert_eq!(pack.len(), 1);
    }

    #[test]
    fn merging_packs_keeps_one_of_each() {
        let mut a = Pack::default();
        a.insert("concrete/wall", texture(), &assets(false, false, None));
        let mut b = Pack::default();
        b.insert("concrete/wall", texture(), &assets(false, false, None));
        b.insert("metal/hull", texture(), &assets(false, false, None));

        a.merge(b);
        assert_eq!(a.len(), 2);
    }

    /// The script and the schematic palette must agree, so every registered
    /// block has to appear in the script exactly once.
    #[test]
    fn the_script_registers_every_block() {
        let mut pack = Pack::default();
        let mut ids = Vec::new();
        for material in ["concrete/wall001a", "metal/grate011a", "glass/window002a"] {
            ids.push(pack.insert(material, texture(), &assets(false, false, None)));
        }

        let script = pack.script();
        for id in &ids {
            assert_eq!(
                script.matches(&format!("event.create('{id}')")).count(),
                1,
                "{id} not registered exactly once"
            );
        }
        assert!(script.contains("StartupEvents.registry('block'"));
    }

    #[test]
    fn non_solid_blocks_declare_a_render_type() {
        let mut pack = Pack::default();
        pack.insert("metal/grate011a", texture(), &assets(true, false, None));
        pack.insert("glass/window002a", texture(), &assets(false, true, None));
        pack.insert("concrete/wall001a", texture(), &assets(false, false, None));

        let script = pack.script();
        assert!(script.contains(".renderType('cutout')"));
        assert!(script.contains(".renderType('translucent')"));
        // Solid is the default, so saying so would be noise.
        assert!(!script.contains(".renderType('solid')"));
    }

    /// KubeJS 2101 has no slab or stairs block builder, so a generated block
    /// must never claim a sub-block shape — the id would never be registered
    /// and the paste would fail on it.
    #[test]
    fn generated_blocks_never_claim_a_sub_block_shape() {
        use crate::voxel::shapes::Variant;
        let mut pack = Pack::default();
        let id = pack.insert("concrete/wall001a", texture(), &assets(false, false, None));

        assert_eq!(pack.shaped(&id, Variant::Full), Some(id.clone()));
        assert_eq!(pack.shaped(&id, Variant::Slab), None);
        assert_eq!(pack.shaped(&id, Variant::Stairs), None);
        assert!(!pack.script().contains("_slab"));
        assert!(!pack.script().contains("_stairs"));

        // Blocks from elsewhere are not claimed at all.
        assert_eq!(pack.shaped("minecraft:stone", Variant::Full), None);
        assert_eq!(pack.shaped("kubejs:never_registered", Variant::Full), None);
    }

    /// Every method the generated script calls must be one KubeJS 2101
    /// actually has. `textureAll` is the pre-2101 name and fails at startup
    /// with "Cannot find function textureAll in object BasicKubeBlock$Builder".
    #[test]
    fn the_script_only_calls_methods_that_exist() {
        let mut pack = Pack::default();
        pack.insert(
            "metal/grate011a",
            texture(),
            &assets(true, false, Some("metalgrate")),
        );
        let script = pack.script();

        assert!(script.contains(".texture('kubejs:block/metal_grate011a')"));
        assert!(
            !script.contains(".textureAll("),
            "textureAll does not exist in KubeJS 2101"
        );
        // Only `basic` blocks exist, so `create` is never given a type.
        assert!(!script.contains("event.create('kubejs:metal_grate011a',"));

        for method in [
            ".displayName(",
            ".texture(",
            ".soundType(",
            ".hardness(",
            ".resistance(",
        ] {
            assert!(script.contains(method), "{method} missing");
        }
    }

    #[test]
    fn display_names_are_escaped() {
        let mut pack = Pack::default();
        pack.insert("props/it's_a_thing", texture(), &assets(false, false, None));
        let script = pack.script();
        assert!(
            script.contains("\\'"),
            "an apostrophe must not close the string"
        );
    }

    #[test]
    fn writes_a_texture_per_block_and_one_script() {
        let dir = std::env::temp_dir().join("src2mc-kubejs-write");
        let _ = std::fs::remove_dir_all(&dir);

        let mut pack = Pack::default();
        pack.insert("concrete/wall001a", texture(), &assets(false, false, None));
        pack.insert("metal/grate011a", texture(), &assets(true, false, None));
        let written = pack.write(&dir).unwrap();

        assert_eq!(written.blocks, 2);
        assert!(written.texture_bytes > 0);
        for id in ["concrete_wall001a", "metal_grate011a"] {
            let png = dir.join(format!("kubejs/assets/kubejs/textures/block/{id}.png"));
            assert!(png.exists(), "{} missing", png.display());
            assert_eq!(&std::fs::read(&png).unwrap()[1..4], b"PNG");
        }
        assert!(dir.join("kubejs/startup_scripts/src2mc_blocks.js").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_pack_writes_only_the_script() {
        let dir = std::env::temp_dir().join("src2mc-kubejs-empty");
        let _ = std::fs::remove_dir_all(&dir);
        let written = Pack::default().write(&dir).unwrap();
        assert_eq!(written.blocks, 0);
        assert!(dir.join("kubejs/startup_scripts/src2mc_blocks.js").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
