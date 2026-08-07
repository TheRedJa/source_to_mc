//! Turning a Source studio model into something Minecraft can actually draw.
//!
//! A prop is the one thing in a Source map that was never designed for a grid.
//! A forklift, a car, a rock, a length of chain-link fence: voxelizing those
//! gives a lump of cubes wearing whatever tile of the texture happened to be in
//! front of each one. Nothing about the shape survives.
//!
//! NeoForge ships a Wavefront OBJ model loader, so it does not have to. A model
//! JSON with `"loader": "neoforge:obj"` renders arbitrary triangles, and
//! KubeJS's `assets/` folder is loaded as a resource pack, so the mesh can be
//! shipped exactly where the textures already are:
//!
//! ```text
//! kubejs/assets/kubejs/models/props/<id>.obj
//! kubejs/assets/kubejs/models/props/<id>.mtl
//! kubejs/assets/kubejs/models/block/<id>.json
//! kubejs/assets/kubejs/blockstates/<id>.json
//! ```
//!
//! Two conventions differ from vanilla JSON models and both are easy to get
//! wrong:
//!
//! * **One OBJ unit is one block**, not the 1/16 of a block a JSON model's
//!   coordinates use. A mistake here is a prop 16 times too large or too small.
//! * **Textures come off the block atlas**, so UVs outside `0..1` do not wrap
//!   round — they read whatever sprite got stitched next door. Source models
//!   tile their sheets freely, so the texture is repeated into a larger image
//!   instead. See [`Repeat`].

use crate::config::Config;
use crate::geom::Vec3;
use crate::output::kubejs::{NAMESPACE, RenderType, block_id};
use crate::source::mdl;
use crate::source::vmt::Materials;
use crate::source::vtf::Textures;
use image::RgbaImage;
use std::collections::BTreeMap;

/// Where a prop's meshes live inside the pack.
const MODEL_DIR: &str = "props";

/// Where a prop's textures live inside the pack.
///
/// Under `block/`, and not in a folder of their own, because that is what
/// decides whether they are stitched into an atlas at all. Minecraft builds the
/// block atlas from the source directories listed in `atlases/blocks.json`,
/// which is `block` and `item` — a texture anywhere else is simply not on the
/// atlas, and a model referring to it draws the black-and-purple missing
/// texture. The directory source recurses, so a subfolder is fine.
const TEXTURE_DIR: &str = "block/props";

/// One prop model, as the files that render it.
#[derive(Debug, Clone)]
pub struct PropAsset {
    /// Id within the KubeJS namespace, e.g. `prop_props_c17_fence01a`.
    pub id: String,
    /// The `.mdl` path this came from.
    pub model: String,
    pub obj: String,
    /// Stem of the `.mtl` file this refers to. Every variant of one model
    /// wears the same materials, so they share one file rather than writing
    /// tens of thousands of identical copies.
    pub mtl_id: String,
    pub mtl: String,
    /// Model JSON texture slot (`texture0`) to the texture it refers to.
    pub textures: BTreeMap<String, String>,
    pub render_type: RenderType,
    pub surface_prop: Option<String>,
    /// Extent from the model's own origin, in blocks: half-width horizontally
    /// and height vertically, for the entity's culling box.
    pub width: f32,
    pub height: f32,
    pub triangles: usize,
}

/// A texture generated for a prop, at prop resolution and possibly repeated.
#[derive(Debug, Clone)]
pub struct PropTexture {
    /// Path under `textures/`, without the extension: `props/metal_wall_2x1`.
    pub name: String,
    pub image: RgbaImage,
}

impl PropAsset {
    /// The namespaced block id a display entity names.
    pub fn block_id(&self) -> String {
        format!("{NAMESPACE}:{}", self.id)
    }

    /// The block model JSON that points Minecraft at the mesh.
    pub fn model_json(&self, flip_v: bool) -> String {
        let mut textures: Vec<String> = self
            .textures
            .iter()
            .map(|(slot, path)| format!("    \"{slot}\": \"{path}\""))
            .collect();
        // Without a particle texture Minecraft logs a missing-texture warning
        // for every one of these and falls back to the checkerboard.
        if let Some(first) = self.textures.values().next() {
            textures.push(format!("    \"particle\": \"{first}\""));
        }
        let mut json = String::from("{\n  \"loader\": \"neoforge:obj\",\n");
        // Relative to the namespace root, not to `models/` — unlike every
        // other model reference in the game, which is why this is spelled out.
        // A path missing the folder resolves to nothing and the prop renders
        // as the missing-model checkerboard.
        json.push_str(&format!(
            "  \"model\": \"{NAMESPACE}:models/{MODEL_DIR}/{}.obj\",\n",
            self.id
        ));
        json.push_str(&format!("  \"flip_v\": {flip_v},\n"));
        // Props are open shells, and culling faces of an open shell eats them.
        json.push_str("  \"automatic_culling\": false,\n  \"shade_quads\": true,\n");
        // Ambient occlusion is computed from the neighbours of the block a
        // model belongs to, on the assumption that its faces line up with that
        // block. A prop's do not — a baked one reaches metres past its own
        // cell — so the shading it derives is banding that follows the grid
        // rather than the mesh.
        json.push_str("  \"ambientocclusion\": false,\n");
        json.push_str(&format!(
            "  \"textures\": {{\n{}\n  }}\n}}\n",
            textures.join(",\n")
        ));
        json
    }

    /// The blockstate JSON, which for these is one unconditional variant.
    pub fn blockstate_json(&self) -> String {
        format!(
            "{{\n  \"variants\": {{\n    \"\": {{ \"model\": \"{NAMESPACE}:block/{}\" }}\n  }}\n}}\n",
            self.id
        )
    }
}

/// A prop's geometry, resolved once per `.mdl` and ready to be written out at
/// whatever orientation a placement asks for.
///
/// Kept apart from [`PropAsset`] because the same mesh is written more than
/// once: in model space for the display entities that carry their own
/// rotation, and again with a placement's rotation and offset baked in for the
/// prop that is drawn as an ordinary block. Resolving the materials, decoding
/// the textures and working out the repeats is the expensive half and it is
/// the same every time, so it happens here and once.
#[derive(Debug, Clone)]
pub struct PropMesh {
    /// The `.mdl` path this came from.
    pub model: String,
    /// Id of the model-space asset, and the stem of the shared `.mtl`.
    pub id: String,
    pub parts: Vec<MeshPart>,
    pub mtl: String,
    pub textures: BTreeMap<String, String>,
    pub render_type: RenderType,
    pub surface_prop: Option<String>,
    pub width: f32,
    pub height: f32,
    pub triangles: usize,
}

/// The triangles of one mesh wearing one material.
#[derive(Debug, Clone)]
pub struct MeshPart {
    /// Name of the OBJ material, as the `.mtl` declares it.
    pub material: String,
    /// Model space, in blocks: one unit is one Minecraft block.
    pub triangles: Vec<[Vec3; 3]>,
    /// Already mapped into the sprite's `0..1` by [`Repeat`].
    pub uvs: Vec<[[f64; 2]; 3]>,
}

/// Where a baked copy of a mesh sits, relative to the block it is placed in.
///
/// The same transform a display entity would carry, resolved once instead of
/// every frame: `basis` is the rotation's columns in Minecraft axes, and
/// `translation` is the prop's origin measured from the corner of its block.
#[derive(Debug, Clone, Copy)]
pub struct Place {
    pub basis: [Vec3; 3],
    pub scale: f64,
    pub translation: Vec3,
}

impl Place {
    /// A model-space vertex, in the coordinates its block's model wants.
    pub fn apply(&self, v: Vec3) -> Vec3 {
        self.basis[0] * (v.x * self.scale)
            + self.basis[1] * (v.y * self.scale)
            + self.basis[2] * (v.z * self.scale)
            + self.translation
    }
}

/// One block's worth of a prop: the triangles near enough to it to be drawn
/// from it, and where it sits relative to the prop's own origin.
///
/// A prop small enough to be one block is one group covering everything, which
/// is nearly all of them. See [`PropMesh::split`] for why the rest are cut up.
#[derive(Debug, Clone)]
pub struct Group {
    /// Which triangles belong to it, as `(part, triangle)`.
    pub members: Vec<(usize, usize)>,
    /// Where the block wants to be, in blocks from the prop's origin.
    pub centre: Vec3,
    /// What this piece actually covers, in blocks from the prop's origin.
    /// Where the block goes is chosen from inside it.
    pub bounds: crate::geom::Aabb,
}

impl PropMesh {
    /// Cut the placed mesh into pieces, none reaching further than `reach`
    /// from its own piece's centre.
    ///
    /// A block model may be drawn far outside its own block, but not
    /// arbitrarily far: Sodium packs each chunk vertex coordinate into 20 bits
    /// spanning -8 to +24 blocks from the section origin and masks away what
    /// does not fit, so a mesh reaching past that is drawn correctly up to the
    /// limit and then folds back on itself. Nearly every prop is well inside
    /// it and comes back as a single group; a gantry, a pipe run or a
    /// rooftop's worth of scenery is not, and is carried by several blocks
    /// instead, each drawing the part of the mesh nearest it.
    ///
    /// Grouping is by triangle centre, so the pieces tile the prop without
    /// gaps or overlap: every triangle is drawn exactly once.
    pub fn split(&self, basis: [Vec3; 3], scale: f64, reach: f64) -> Vec<Group> {
        let place = Place {
            basis,
            scale,
            translation: Vec3::ZERO,
        };
        let placed = |v: Vec3| place.apply(v);

        // The common case first, and without touching the grid: if the whole
        // mesh fits around its own origin there is nothing to cut.
        let mut extent: f64 = 0.0;
        let mut whole = crate::geom::Aabb::empty();
        for part in &self.parts {
            for triangle in &part.triangles {
                for corner in triangle {
                    let p = placed(*corner);
                    extent = extent.max(p.x.abs()).max(p.y.abs()).max(p.z.abs());
                    whole.extend(p);
                }
            }
        }
        if extent <= reach {
            let members = self
                .parts
                .iter()
                .enumerate()
                .flat_map(|(part, p)| (0..p.triangles.len()).map(move |t| (part, t)))
                .collect();
            return vec![Group {
                members,
                centre: Vec3::ZERO,
                bounds: whole,
            }];
        }

        // A triangle is filed by its centre, so its corners hang over its
        // cell's edge by however large the triangle is, and the block lands
        // somewhere inside the piece rather than exactly at its middle. Both
        // eat into the reach, so the cells are cut until what comes out
        // actually fits rather than until the arithmetic says it should. Most
        // props need one round; the limit is there because a single triangle
        // wider than the reach cannot be cut by grouping at all, and refining
        // forever would not help it.
        // Only the pieces that do not fit are cut again, and only they. Cutting
        // the whole prop finer because one corner of it is awkward multiplies
        // the blocks it needs — and every one of those needs a free cell of its
        // own, so over-splitting is what sends a prop back to being an entity.
        let target = reach * 0.6;
        let everything: Vec<(usize, usize)> = self
            .parts
            .iter()
            .enumerate()
            .flat_map(|(part, p)| (0..p.triangles.len()).map(move |t| (part, t)))
            .collect();

        let mut done = Vec::new();
        let mut queue = vec![(everything, reach)];
        while let Some((members, side)) = queue.pop() {
            for group in self.grid(&placed, &members, side) {
                // A group of one triangle is as cut up as it can get; nothing
                // splits a single triangle.
                if group.members.len() < 2
                    || side <= reach / 64.0
                    || self.reach_about(&placed, &group) <= target
                {
                    done.push(group);
                } else {
                    queue.push((group.members, side / 2.0));
                }
            }
        }
        done
    }

    /// Bucket `members` by which cell of `side` each triangle's centre falls in.
    fn grid(
        &self,
        placed: &impl Fn(Vec3) -> Vec3,
        members: &[(usize, usize)],
        side: f64,
    ) -> Vec<Group> {
        let side = side.max(f64::MIN_POSITIVE);
        let mut cells: BTreeMap<[i64; 3], Vec<(usize, usize)>> = BTreeMap::new();
        for (part, index) in members {
            let triangle = &self.parts[*part].triangles[*index];
            let centre = (placed(triangle[0]) + placed(triangle[1]) + placed(triangle[2])) / 3.0;
            let key = [centre.x, centre.y, centre.z].map(|c| (c / side).floor() as i64);
            cells.entry(key).or_default().push((*part, *index));
        }

        cells
            .into_iter()
            .map(|(key, members)| {
                let mut bounds = crate::geom::Aabb::empty();
                for (part, index) in &members {
                    for corner in &self.parts[*part].triangles[*index] {
                        bounds.extend(placed(*corner));
                    }
                }
                Group {
                    members,
                    centre: Vec3::new(
                        (key[0] as f64 + 0.5) * side,
                        (key[1] as f64 + 0.5) * side,
                        (key[2] as f64 + 0.5) * side,
                    ),
                    bounds,
                }
            })
            .collect()
    }

    /// How far `group` reaches from its own centre.
    fn reach_about(&self, placed: &impl Fn(Vec3) -> Vec3, group: &Group) -> f64 {
        let mut reach: f64 = 0.0;
        for (part, index) in &group.members {
            for corner in &self.parts[*part].triangles[*index] {
                let p = placed(*corner) - group.centre;
                reach = reach.max(p.x.abs()).max(p.y.abs()).max(p.z.abs());
            }
        }
        reach
    }

    /// How far the furthest corner of `group` sits from `place`'s origin.
    ///
    /// The check that the split actually worked. A single triangle larger than
    /// the reach cannot be cut up by grouping — nothing splits one triangle —
    /// so the caller needs to know when a piece is still too big and the prop
    /// has to be drawn some other way.
    pub fn reach_of(&self, place: &Place, group: &Group) -> f64 {
        let mut reach: f64 = 0.0;
        for (part, index) in &group.members {
            let Some(triangle) = self.parts.get(*part).and_then(|p| p.triangles.get(*index)) else {
                continue;
            };
            for corner in triangle {
                let p = place.apply(*corner);
                reach = reach.max(p.x.abs()).max(p.y.abs()).max(p.z.abs());
            }
        }
        reach
    }

    /// Write this mesh out as the files one block needs.
    ///
    /// With no `place` the mesh is written in model space, for an entity to
    /// orient. With one, the orientation is baked into the coordinates and the
    /// result is a block that draws the prop where the map put it.
    /// With a `group`, only that piece of the mesh is written; without one,
    /// all of it.
    pub fn asset(&self, id: String, place: Option<&Place>, group: Option<&Group>) -> PropAsset {
        let mut positions = Index::default();
        let mut coords = Index::default();
        let mut faces = String::new();
        // Which triangles of each part this piece draws. `None` is all of them.
        let kept: Option<std::collections::HashSet<(usize, usize)>> =
            group.map(|g| g.members.iter().copied().collect());
        let mut written = 0usize;

        for (index, part) in self.parts.iter().enumerate() {
            if kept
                .as_ref()
                .is_some_and(|kept| !(0..part.triangles.len()).any(|t| kept.contains(&(index, t))))
            {
                continue;
            }
            faces.push_str(&format!("usemtl {}\n", part.material));
            for (triangle, uv) in part
                .triangles
                .iter()
                .zip(&part.uvs)
                .enumerate()
                .filter(|(t, _)| kept.as_ref().is_none_or(|kept| kept.contains(&(index, *t))))
                .map(|(_, pair)| pair)
            {
                written += 1;
                let mut corners = [(0usize, 0usize); 3];
                for (slot, (vertex, coord)) in corners.iter_mut().zip(triangle.iter().zip(uv)) {
                    let p = match place {
                        Some(place) => place.apply(*vertex),
                        None => *vertex,
                    };
                    *slot = (
                        positions.intern("v", &[p.x, p.y, p.z]),
                        coords.intern("vt", &[coord[0], coord[1]]),
                    );
                }
                faces.push_str(&format!(
                    "f {}/{} {}/{} {}/{}\n",
                    corners[0].0,
                    corners[0].1,
                    corners[1].0,
                    corners[1].1,
                    corners[2].0,
                    corners[2].1
                ));
            }
        }

        let obj = format!(
            "# Generated by src2mc from {}\n# One unit is one Minecraft block.\nmtllib {}.mtl\n\n{}\n{}\n{faces}",
            self.model, self.id, positions.lines, coords.lines
        );

        PropAsset {
            id,
            model: self.model.clone(),
            obj,
            mtl_id: self.id.clone(),
            mtl: self.mtl.clone(),
            textures: self.textures.clone(),
            render_type: self.render_type,
            surface_prop: self.surface_prop.clone(),
            width: self.width,
            height: self.height,
            triangles: written,
        }
    }
}

/// Turn a model path into a resource id: `models/props_c17/fence01a.mdl`
/// becomes `prop_props_c17_fence01a`.
///
/// Prefixed so a prop can never collide with the block generated for a
/// material of the same name, which share one namespace and one registry.
pub fn prop_id(model_path: &str) -> String {
    let stem = model_path
        .trim_start_matches('/')
        .strip_prefix("models/")
        .unwrap_or(model_path)
        .trim_end_matches(".mdl");
    format!("prop_{}", block_id(stem))
}

/// How many times a texture is repeated, and from which tile, so that a part's
/// UVs land inside `0..1`.
///
/// Source models tile their sheets: a fence runs the same 128 texels along its
/// length a dozen times, with UVs from 0 to 12. A resource pack texture is a
/// sprite on a shared atlas, so those coordinates would sample whatever was
/// stitched alongside it rather than wrapping. Repeating the image `count`
/// times and dividing the UVs to match reproduces the tiling exactly, as long
/// as the repeat is not capped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repeat {
    /// Tile the UVs are measured from, so a part using 5.0..6.0 is not
    /// repeated six times to reach it.
    pub origin: [i32; 2],
    pub count: [u32; 2],
    /// Whether the span was larger than the cap, so the UVs wrap and a seam
    /// appears wherever they do.
    pub capped: bool,
}

impl Repeat {
    /// The repeat a set of texture coordinates needs.
    pub fn of<'a>(uvs: impl Iterator<Item = &'a [[f64; 2]; 3]>, max: u32) -> Repeat {
        let max = max.max(1);
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for corner in uvs.flatten() {
            for axis in 0..2 {
                if corner[axis].is_finite() {
                    lo[axis] = lo[axis].min(corner[axis]);
                    hi[axis] = hi[axis].max(corner[axis]);
                }
            }
        }

        let mut repeat = Repeat {
            origin: [0, 0],
            count: [1, 1],
            capped: false,
        };
        let mut spans = [1.0f64; 2];
        for axis in 0..2 {
            if lo[axis] > hi[axis] {
                continue;
            }
            let origin = lo[axis].floor();
            // A wildly stretched unwrap would otherwise ask for a texture of
            // thousands of tiles, which is neither useful nor affordable.
            spans[axis] = (hi[axis] - origin).ceil().max(1.0);
            repeat.origin[axis] = origin as i32;
        }

        // Square, even when only one axis tiles. Minecraft reads a sprite
        // taller than it is wide as an animation strip and shows one frame of
        // it; wider than tall it rejects outright as a broken aspect ratio, and
        // the model then draws the missing texture. Repeating the other axis
        // too costs image area and nothing else — the coordinates still divide
        // by their own count, so one pass over the sheet is still one copy.
        let square = spans
            .iter()
            .map(|span| span.min(f64::from(max)) as u32)
            .max()
            .unwrap_or(1)
            .max(1);
        repeat.count = [square; 2];
        repeat.capped = spans.iter().any(|span| *span > f64::from(square));
        repeat
    }

    /// Map a texture coordinate into the repeated image's `0..1`.
    ///
    /// Within the repeat this is exact. Past it the coordinate wraps, which
    /// keeps the texture at its true scale — the alternative, stretching it to
    /// fit, would be wrong everywhere rather than at one seam.
    pub fn apply(&self, uv: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|axis| {
            let count = f64::from(self.count[axis].max(1));
            let local = uv[axis] - f64::from(self.origin[axis]);
            if self.capped {
                local.rem_euclid(count) / count
            } else {
                local / count
            }
        })
    }

    /// The suffix that distinguishes one repeat of a texture from another.
    fn suffix(&self) -> String {
        format!("{}x{}", self.count[0], self.count[1])
    }
}

/// Repeat an image into a `count[0]` by `count[1]` grid of itself.
fn tile_image(image: &RgbaImage, count: [u32; 2]) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (cx, cy) = (count[0].max(1), count[1].max(1));
    if cx == 1 && cy == 1 {
        return image.clone();
    }
    let mut out = RgbaImage::new(w * cx, h * cy);
    for ty in 0..cy {
        for tx in 0..cx {
            for (x, y, pixel) in image.enumerate_pixels() {
                out.put_pixel(tx * w + x, ty * h + y, *pixel);
            }
        }
    }
    out
}

/// Deduplicating writer for the `v` and `vt` lists.
///
/// Written out per corner an OBJ would be three times the size it needs to be,
/// and a campaign's worth of models is already tens of megabytes of text.
#[derive(Default)]
struct Index {
    keys: std::collections::HashMap<[i64; 3], usize>,
    lines: String,
}

impl Index {
    fn intern(&mut self, tag: &str, values: &[f64]) -> usize {
        use std::fmt::Write;
        // Quantized so that coordinates equal to the precision written are
        // equal as keys; otherwise the file grows without the mesh changing.
        let mut key = [0i64; 3];
        for (slot, value) in key.iter_mut().zip(values) {
            *slot = (value * 10_000.0).round() as i64;
        }
        if let Some(index) = self.keys.get(&key) {
            return *index;
        }
        let index = self.keys.len() + 1;
        self.keys.insert(key, index);
        let _ = write!(self.lines, "{tag}");
        for value in values {
            let _ = write!(self.lines, " {:.4}", value);
        }
        self.lines.push('\n');
        index
    }
}

/// Resolve one prop model into geometry and the textures it needs.
///
/// Returns `None` when the model has no usable geometry, or when it is heavier
/// than the configured triangle budget — in which case the caller falls back to
/// voxelizing it, which is worse-looking but bounded.
pub fn build(
    path: &str,
    model: &mdl::Model,
    config: &Config,
    materials: &Materials,
    textures: &mut Textures,
) -> Option<(PropMesh, Vec<PropTexture>)> {
    let triangles = model.triangle_count();
    if triangles == 0 {
        return None;
    }
    if config.props.max_triangles > 0 && triangles > config.props.max_triangles {
        return None;
    }

    let units = config.scale.units_per_block.max(f64::MIN_POSITIVE);
    // No triangle may be wider than a piece of a split prop is allowed to be,
    // or it cannot be put in one: see [`subdivide`].
    let limit = if config.props.bake {
        config.props.bake_reach * 0.5
    } else {
        0.0
    };
    let mut mesh_parts: Vec<MeshPart> = Vec::new();
    let mut mtl = String::new();
    let mut slots: BTreeMap<String, String> = BTreeMap::new();
    // Texture path to the OBJ material already declared for it.
    let mut used: BTreeMap<String, String> = BTreeMap::new();
    let mut emitted: Vec<PropTexture> = Vec::new();
    let mut render_type = RenderType::Solid;
    let mut surface_prop = None;
    let mut written = 0usize;

    for part in &model.parts {
        if part.triangles.is_empty() {
            continue;
        }
        let Some(assets) = materials.assets(&part.material, None) else {
            continue;
        };
        let repeat = Repeat::of(part.uvs.iter(), config.props.texture_repeat_max);
        let name = format!(
            "{TEXTURE_DIR}/{}_{}",
            block_id(&part.material),
            repeat.suffix()
        );

        // One model commonly wears the same sheet in several parts, and at the
        // same repeat; that is one slot and one image, not several.
        let material_name = match used.get(&name) {
            Some(existing) => existing.clone(),
            None => {
                let Some(base) = textures.get(&assets.base_texture, assets.alpha_test) else {
                    continue;
                };
                let image = tile_image(base, repeat.count);
                emitted.push(PropTexture {
                    name: name.clone(),
                    image,
                });

                let slot = format!("texture{}", slots.len());
                let material_name = format!("mat{}", slots.len());
                slots.insert(slot.clone(), format!("{NAMESPACE}:{name}"));
                mtl.push_str(&format!("newmtl {material_name}\nmap_Kd #{slot}\n\n"));
                used.insert(name.clone(), material_name.clone());
                material_name
            }
        };

        // A model wears one render type, so the least forgiving part wins: a
        // cutout drawn translucent turns a grate into haze.
        let part_type = RenderType::of(&assets);
        if part_type == RenderType::Cutout || render_type == RenderType::Solid {
            render_type = part_type;
        }
        surface_prop = surface_prop.or_else(|| assets.surface_prop.clone());

        let mut mesh_part = MeshPart {
            material: material_name,
            triangles: Vec::new(),
            uvs: Vec::new(),
        };
        for (triangle, uv) in part.triangles.iter().zip(&part.uvs) {
            let corners = triangle.map(|v| to_model_space(v, units));
            // Texture coordinates written as they are. Both conventions run V
            // downwards from the top of the image: Source's because it is a
            // Direct3D engine, Minecraft's because `TextureAtlasSprite.getV`
            // maps 0 to the sprite's top edge. Flipping to "correct" for
            // OpenGL — which is what the loader's `flip_v` is for — mirrors
            // the sheet, and on a model sheet with unused areas that shows up
            // as half a prop wearing blank texture and the rest wearing pieces
            // of something else.
            let coords = uv.map(|coord| repeat.apply(coord));
            // Cut anything too large to be drawn from one block. A prop is
            // carried by as many blocks as it needs, but the pieces are made by
            // grouping whole triangles, and nothing groups one triangle: a
            // 32-block light shaft or a citadel wall panel is often a single
            // pair of them. Splitting the edge is exact — the surface is flat
            // and the texture coordinates run linearly across it — so this
            // costs triangles and changes nothing you can see.
            subdivide(corners, coords, limit, &mut mesh_part, 0);
            written += 1;
        }
        mesh_parts.push(mesh_part);
    }

    if written == 0 || slots.is_empty() {
        return None;
    }

    // The culling box, in blocks from the model's own origin.
    let size = model.bounds.size() / units;
    let width = size.x.max(size.y) as f32;
    let height = (model.bounds.max.z / units).max(size.z / 2.0) as f32;

    Some((
        PropMesh {
            id: prop_id(path),
            model: path.to_string(),
            parts: mesh_parts,
            mtl,
            textures: slots,
            render_type,
            surface_prop,
            width,
            height,
            triangles: written,
        },
        emitted,
    ))
}

/// Split a triangle until no edge is longer than `limit`, in blocks.
///
/// The longest edge is halved and the triangle becomes two, which is exact:
/// the surface is flat, so the midpoint lies on it, and texture coordinates
/// run linearly across a triangle, so the midpoint's are the average of the
/// edge's. Depth is capped because a limit of zero would otherwise never be
/// reached.
fn subdivide(corners: [Vec3; 3], uvs: [[f64; 2]; 3], limit: f64, out: &mut MeshPart, depth: u32) {
    const MAX_DEPTH: u32 = 8;
    let edges = [(0, 1), (1, 2), (2, 0)];
    let longest = edges
        .iter()
        .enumerate()
        .max_by(|a, b| {
            let length = |e: &(usize, usize)| (corners[e.0] - corners[e.1]).length();
            length(a.1)
                .partial_cmp(&length(b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, _)| index)
        .unwrap_or(0);
    let (a, b) = edges[longest];
    let c = 3 - a - b;

    if depth >= MAX_DEPTH || limit <= 0.0 || (corners[a] - corners[b]).length() <= limit {
        out.triangles.push(corners);
        out.uvs.push(uvs);
        return;
    }

    let middle = (corners[a] + corners[b]) * 0.5;
    let middle_uv = [(uvs[a][0] + uvs[b][0]) * 0.5, (uvs[a][1] + uvs[b][1]) * 0.5];
    // Both halves keep the winding of the original, so the faces still point
    // the way the model meant them to.
    subdivide(
        [corners[a], middle, corners[c]],
        [uvs[a], middle_uv, uvs[c]],
        limit,
        out,
        depth + 1,
    );
    subdivide(
        [middle, corners[b], corners[c]],
        [middle_uv, uvs[b], uvs[c]],
        limit,
        out,
        depth + 1,
    );
}

/// Source model space to Minecraft model space, in blocks.
///
/// The same axis mapping [`crate::voxel::transform`] applies to the world —
/// Z up, Y mirrored — but without translation or the map's yaw, because a
/// model's orientation is carried by the entity that places it.
fn to_model_space(v: Vec3, units_per_block: f64) -> Vec3 {
    Vec3::new(
        v.x / units_per_block,
        v.z / units_per_block,
        -v.y / units_per_block,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uvs(corners: &[[f64; 2]]) -> Vec<[[f64; 2]; 3]> {
        corners.chunks(3).map(|c| [c[0], c[1], c[2]]).collect()
    }

    #[test]
    fn model_ids_are_legal_and_distinct_from_material_ids() {
        assert_eq!(
            prop_id("models/props_c17/fence01a.mdl"),
            "prop_props_c17_fence01a"
        );
        assert_eq!(prop_id("models/Props/Barrel.mdl"), "prop_props_barrel");
        // A model and a material of the same name must not collide.
        assert_ne!(
            prop_id("models/concrete/wall.mdl"),
            block_id("concrete/wall")
        );
        assert!(
            prop_id("models/a b/c!.mdl")
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-'))
        );
    }

    /// The axis mapping has to match the world's, or props are placed correctly
    /// and drawn lying on their side.
    #[test]
    fn model_space_is_z_up_to_y_up() {
        let p = to_model_space(Vec3::new(16.0, 32.0, 48.0), 16.0);
        assert_eq!((p.x, p.y, p.z), (1.0, 3.0, -2.0));
    }

    /// One OBJ unit is one block. At 16 units per block a 16-unit crate is one
    /// block across — not 16, which is what a vanilla JSON model would mean.
    #[test]
    fn one_obj_unit_is_one_block() {
        let p = to_model_space(Vec3::new(16.0, 0.0, 0.0), 16.0);
        assert_eq!(p.x, 1.0);
    }

    #[test]
    fn uvs_inside_one_tile_are_not_repeated() {
        let repeat = Repeat::of(uvs(&[[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]]).iter(), 4);
        assert_eq!(repeat.count, [1, 1]);
        assert!(!repeat.capped);
        assert_eq!(repeat.apply([0.25, 0.5]), [0.25, 0.5]);
    }

    /// The whole point: a fence tiling its sheet six times gets a texture six
    /// tiles wide, and coordinates that stay inside the sprite.
    #[test]
    fn tiling_uvs_are_brought_inside_the_sprite() {
        let corners = uvs(&[[0.0, 0.0], [6.0, 0.0], [3.0, 1.0]]);
        let repeat = Repeat::of(corners.iter(), 8);
        assert_eq!(repeat.count, [6, 6], "the image has to stay square");
        for corner in corners.iter().flatten() {
            let [u, v] = repeat.apply(*corner);
            assert!((0.0..=1.0).contains(&u), "u {u} escaped the sprite");
            assert!((0.0..=1.0).contains(&v), "v {v} escaped the sprite");
        }
        // And at the true scale: one sheet is one sixth of the repeated image.
        assert_eq!(repeat.apply([1.0, 0.0])[0], 1.0 / 6.0);
    }

    /// UVs that start well into the sheet must not drag the repeat out to
    /// cover everything between the origin and them.
    #[test]
    fn a_repeat_is_measured_from_where_the_uvs_start() {
        let repeat = Repeat::of(uvs(&[[5.0, 0.0], [6.0, 0.0], [5.5, 1.0]]).iter(), 4);
        assert_eq!(repeat.count, [1, 1]);
        assert_eq!(repeat.apply([5.5, 0.0])[0], 0.5);
    }

    /// Past the cap the coordinates wrap rather than escaping the sprite, which
    /// costs a seam and keeps the texture at its real size.
    #[test]
    fn a_capped_repeat_wraps_instead_of_escaping() {
        let corners = uvs(&[[0.0, 0.0], [40.0, 0.0], [20.0, 1.0]]);
        let repeat = Repeat::of(corners.iter(), 4);
        assert_eq!(repeat.count, [4, 4]);
        assert!(repeat.capped);
        for u in [0.0, 3.9, 4.0, 17.5, 39.9] {
            let mapped = repeat.apply([u, 0.0])[0];
            assert!((0.0..=1.0).contains(&mapped), "{u} mapped to {mapped}");
        }
    }

    /// Whatever the UVs ask for, the image stays square. Minecraft reads a
    /// taller-than-wide sprite as an animation and shows one frame, and
    /// rejects a wider-than-tall one outright — either way the prop draws the
    /// missing texture.
    #[test]
    fn a_repeat_is_always_square() {
        for corners in [
            vec![[0.0, 0.0], [7.0, 0.0], [3.0, 1.0]],
            vec![[0.0, 0.0], [1.0, 0.0], [0.5, 9.0]],
            vec![[-3.0, -2.0], [0.0, 0.0], [-1.0, 5.0]],
            vec![[0.0, 0.0], [0.2, 0.0], [0.1, 0.3]],
        ] {
            let repeat = Repeat::of(uvs(&corners).iter(), 8);
            assert_eq!(
                repeat.count[0], repeat.count[1],
                "{corners:?} asked for a {:?} image",
                repeat.count
            );
            // Still inside the sprite, which is the point of repeating at all.
            for corner in &corners {
                let [u, v] = repeat.apply(*corner);
                assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v));
            }
        }
    }

    /// A sprite outside `block/` or `item/` is on no atlas at all, and every
    /// model naming it draws the missing texture.
    #[test]
    fn prop_textures_live_where_the_block_atlas_looks() {
        assert!(
            TEXTURE_DIR == "block" || TEXTURE_DIR.starts_with("block/"),
            "{TEXTURE_DIR} is not a source directory of the block atlas"
        );
    }

    #[test]
    fn repeating_an_image_lays_out_copies_of_it() {
        let mut image = RgbaImage::new(2, 2);
        image.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
        let tiled = tile_image(&image, [3, 2]);
        assert_eq!(tiled.dimensions(), (6, 4));
        for (x, y) in [(0, 0), (2, 0), (4, 0), (0, 2), (2, 2), (4, 2)] {
            assert_eq!(tiled.get_pixel(x, y).0, [1, 2, 3, 255], "copy at {x},{y}");
        }
        assert_eq!(tile_image(&image, [1, 1]).dimensions(), (2, 2));
    }

    /// The same vertex used by several triangles must be written once.
    #[test]
    fn the_vertex_list_is_deduplicated() {
        let mut index = Index::default();
        assert_eq!(index.intern("v", &[1.0, 2.0, 3.0]), 1);
        assert_eq!(index.intern("v", &[1.0, 2.0, 3.0]), 1);
        assert_eq!(index.intern("v", &[1.0, 2.0, 4.0]), 2);
        assert_eq!(index.lines.lines().count(), 2);
        assert!(index.lines.starts_with("v 1.0000 2.0000 3.0000"));
    }

    #[test]
    fn the_model_json_names_the_obj_and_its_textures() {
        let asset = PropAsset {
            id: "prop_x".into(),
            model: "models/x.mdl".into(),
            obj: String::new(),
            mtl_id: "prop_x".into(),
            mtl: String::new(),
            textures: [("texture0".to_string(), "kubejs:props/y_1x1".to_string())]
                .into_iter()
                .collect(),
            render_type: RenderType::Solid,
            surface_prop: None,
            width: 1.0,
            height: 1.0,
            triangles: 1,
        };
        let json = asset.model_json(false);
        assert!(json.contains("\"loader\": \"neoforge:obj\""));
        // From the namespace root: `assets/kubejs/models/props/prop_x.obj`.
        // Without the `models/` the loader finds nothing and the prop renders
        // as the missing-model checkerboard.
        assert!(
            json.contains("\"model\": \"kubejs:models/props/prop_x.obj\""),
            "the OBJ path is relative to the namespace root: {json}"
        );
        assert!(json.contains("\"texture0\": \"kubejs:props/y_1x1\""));
        assert!(
            json.contains("\"particle\""),
            "a missing particle logs warnings"
        );
        assert!(asset.blockstate_json().contains("kubejs:block/prop_x"));
        // Valid JSON, not just a string that looks like it.
        let _: serde_json::Value = serde_json::from_str(&json).expect("model json");
        let _: serde_json::Value =
            serde_json::from_str(&asset.blockstate_json()).expect("blockstate json");
    }
}
