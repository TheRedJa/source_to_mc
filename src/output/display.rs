//! Placing prop meshes with `block_display` entities.
//!
//! A prop's mesh is registered once, as a block whose model is the model
//! (`output::obj`). Putting it where Source put it is a separate problem: a
//! block can only sit on the grid facing one of four ways, and a Source prop is
//! at an arbitrary yaw, pitch and roll, half a block off the lattice. Display
//! entities exist for exactly this — they render a blockstate under a free
//! transformation — so each placement becomes one entity carrying the map's
//! own rotation as a quaternion.
//!
//! The entities are written twice, deliberately: into the schematic's
//! `Entities` list, which WorldEdit pastes with `-e`, and as a `.mcfunction` of
//! `summon` commands. The Sponge specification says an implementation must keep
//! everything in an entity's `Data`, but display-entity NBT is unusual enough
//! that having a second route costs little and saves a conversion.

use crate::bsp::props::Prop;
use crate::geom::Vec3;
use crate::voxel::transform::Transform;
use serde::Serialize;

/// One prop, ready to place.
#[derive(Debug, Clone)]
pub struct Placement {
    /// Namespaced id of the block whose model is this prop's mesh.
    pub block: String,
    /// Where it sits, in Minecraft block space.
    pub pos: [f64; 3],
    /// The prop's orientation, as `[x, y, z, w]`.
    pub rotation: [f64; 4],
    pub scale: f64,
    /// Culling box, in blocks.
    pub width: f32,
    pub height: f32,
    /// `view_range`: beyond this times 64 blocks the entity stops rendering.
    pub view_range: f32,
    /// Light the mesh fully rather than by the cell it stands in.
    pub full_bright: bool,
    /// Which map this came from, so a bad paste can be undone in one command.
    pub tag: String,
}

/// The rotation that takes this prop's mesh from model space to where the map
/// wants it, in Minecraft's axes.
///
/// The mesh was written with the same Z-up-to-Y-up mapping the world uses but
/// without the map's yaw, because a model has no place in the world until an
/// entity gives it one. So the rotation wanted here is the whole world
/// transform's linear part, applied to the prop's own rotation, applied to the
/// inverse of that model-space mapping — which is what building it column by
/// column from the basis vectors does, without any of it having to be written
/// out as a matrix product.
pub fn rotation(prop: &Prop, transform: &Transform) -> [f64; 4] {
    let units = transform.units_per_block();
    let mut columns = [Vec3::ZERO; 3];
    for (axis, column) in columns.iter_mut().enumerate() {
        // The model-space basis vector, back in Source space.
        let model = Vec3::new(
            if axis == 0 { 1.0 } else { 0.0 },
            if axis == 1 { 1.0 } else { 0.0 },
            if axis == 2 { 1.0 } else { 0.0 },
        );
        let source = Vec3::new(model.x, -model.z, model.y) * units;
        *column = transform.transform_direction(prop.rotate(source));
    }
    quaternion(columns)
}

/// A rotation matrix, given as its columns, as a quaternion `[x, y, z, w]`.
///
/// Shepperd's method: pick whichever of the four components the diagonal makes
/// largest and derive the rest from it, so nothing is ever divided by a value
/// near zero.
fn quaternion(m: [Vec3; 3]) -> [f64; 4] {
    let (xx, yy, zz) = (m[0].x, m[1].y, m[2].z);
    let trace = xx + yy + zz;
    let out = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (m[1].z - m[2].y) / s,
            (m[2].x - m[0].z) / s,
            (m[0].y - m[1].x) / s,
            0.25 * s,
        ]
    } else if xx > yy && xx > zz {
        let s = (1.0 + xx - yy - zz).sqrt() * 2.0;
        [
            0.25 * s,
            (m[1].x + m[0].y) / s,
            (m[2].x + m[0].z) / s,
            (m[1].z - m[2].y) / s,
        ]
    } else if yy > zz {
        let s = (1.0 + yy - xx - zz).sqrt() * 2.0;
        [
            (m[1].x + m[0].y) / s,
            0.25 * s,
            (m[2].y + m[1].z) / s,
            (m[2].x - m[0].z) / s,
        ]
    } else {
        let s = (1.0 + zz - xx - yy).sqrt() * 2.0;
        [
            (m[2].x + m[0].z) / s,
            (m[2].y + m[1].z) / s,
            0.25 * s,
            (m[0].y - m[1].x) / s,
        ]
    };

    // Normalized, because the columns came from measured directions rather
    // than from an exact matrix and a display entity given a quaternion that
    // is not quite unit renders the model very slightly sheared.
    let length = out.iter().map(|c| c * c).sum::<f64>().sqrt();
    if length > 0.0 { out.map(|c| c / length) } else { [0.0, 0.0, 0.0, 1.0] }
}

/// A display entity as the schematic format wants it.
#[derive(Debug, Clone, Serialize)]
pub struct Entity {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Pos")]
    pos: Vec<f64>,
    #[serde(rename = "Data")]
    data: Data,
}

#[derive(Debug, Clone, Serialize)]
struct Data {
    id: String,
    #[serde(rename = "Pos")]
    pos: Vec<f64>,
    /// Yaw and pitch. Every entity has them and a display entity leaves them
    /// at zero, but WorldEdit's Sponge v3 reader does not treat them as
    /// optional: it reads `Rotation` straight out of `Data` and throws
    /// `NoSuchElementException` if it is absent, which fails the whole load
    /// rather than the one entity. Writing it costs two floats.
    #[serde(rename = "Rotation")]
    rotation: Vec<f32>,
    block_state: BlockState,
    transformation: Transformation,
    width: f32,
    height: f32,
    view_range: f32,
    #[serde(rename = "Tags")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness: Option<Brightness>,
}

#[derive(Debug, Clone, Serialize)]
struct BlockState {
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Debug, Clone, Serialize)]
struct Transformation {
    translation: Vec<f32>,
    left_rotation: Vec<f32>,
    right_rotation: Vec<f32>,
    scale: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
struct Brightness {
    block: i32,
    sky: i32,
}

/// The entity type every prop is placed as.
const KIND: &str = "minecraft:block_display";

impl Placement {
    /// This placement as a schematic entity, with `Pos` relative to `origin` as
    /// the specification requires.
    pub fn entity(&self, origin: [i32; 3]) -> Entity {
        let pos: Vec<f64> = (0..3).map(|a| self.pos[a] - f64::from(origin[a])).collect();
        Entity {
            id: KIND.to_string(),
            // Absolute, because `Data` is the entity's own NBT and an entity's
            // own `Pos` is where it is in the world.
            data: Data {
                id: KIND.to_string(),
                pos: self.pos.to_vec(),
                rotation: vec![0.0, 0.0],
                block_state: BlockState { name: self.block.clone() },
                transformation: self.transformation(),
                width: self.width,
                height: self.height,
                view_range: self.view_range,
                tags: vec!["src2mc".to_string(), self.tag.clone()],
                brightness: self
                    .full_bright
                    .then_some(Brightness { block: 15, sky: 15 }),
            },
            pos,
        }
    }

    fn transformation(&self) -> Transformation {
        Transformation {
            translation: vec![0.0, 0.0, 0.0],
            left_rotation: self.rotation.iter().map(|c| *c as f32).collect(),
            right_rotation: vec![0.0, 0.0, 0.0, 1.0],
            scale: vec![self.scale as f32; 3],
        }
    }

    /// A `summon` command placing this prop at absolute coordinates, matching
    /// where the schematic puts it when pasted with `-o`.
    pub fn summon(&self) -> String {
        let q = self.rotation;
        let brightness = if self.full_bright {
            ",brightness:{block:15,sky:15}".to_string()
        } else {
            String::new()
        };
        format!(
            "summon {KIND} {:.3} {:.3} {:.3} {{block_state:{{Name:\"{}\"}},\
             transformation:{{translation:[0f,0f,0f],left_rotation:[{:.6}f,{:.6}f,{:.6}f,{:.6}f],\
             right_rotation:[0f,0f,0f,1f],scale:[{s:.4}f,{s:.4}f,{s:.4}f]}},\
             width:{:.2}f,height:{:.2}f,view_range:{:.2}f,Tags:[\"src2mc\",\"{}\"]{brightness}}}",
            self.pos[0],
            self.pos[1],
            self.pos[2],
            self.block,
            q[0],
            q[1],
            q[2],
            q[3],
            self.width,
            self.height,
            self.view_range,
            self.tag,
            s = self.scale,
        )
    }
}

/// The whole map's props as a function file.
///
/// Starts by removing anything an earlier run of the same map left behind, so
/// running it twice does not leave two forklifts in the same place.
pub fn function(placements: &[Placement], map: &str) -> String {
    let tag = tag_for(map);
    let mut out = String::new();
    out.push_str(&format!(
        "# Generated by src2mc: the props of {map}, as display entities.\n\
         #\n\
         # These are already inside the schematics; this file is the way back if\n\
         # WorldEdit drops them. Coordinates are absolute and match a `//paste -o`.\n\
         #\n\
         # Install as a datapack function and run it once:\n\
         #   /function <namespace>:{map}_props\n\
         #\n\
         # Every entity is tagged `{tag}`, so to undo:\n\
         #   /kill @e[type={KIND},tag={tag}]\n\n"
    ));
    out.push_str(&format!("kill @e[type={KIND},tag={tag}]\n\n"));
    for placement in placements {
        out.push_str(&placement.summon());
        out.push('\n');
    }
    out
}

/// The tag every entity from one map carries.
pub fn tag_for(map: &str) -> String {
    let name: String = map
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' { c } else { '_' })
        .collect();
    format!("src2mc_{name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::geom::Aabb;

    fn transform() -> Transform {
        Transform::new(&Config::default(), Aabb::empty())
    }

    fn prop(angles: [f64; 3]) -> Prop {
        Prop {
            model: "models/x.mdl".into(),
            origin: Vec3::new(160.0, -320.0, 48.0),
            angles,
            scale: 1.0,
            classname: "prop_static".into(),
        }
    }

    /// Rotate a vector by a quaternion, the way Minecraft will.
    fn rotate(q: [f64; 4], v: Vec3) -> Vec3 {
        let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
        let u = Vec3::new(x, y, z);
        let cross = |a: Vec3, b: Vec3| a.cross(b);
        u * (2.0 * u.dot(v)) + v * (w * w - u.dot(u)) + cross(u, v) * (2.0 * w)
    }

    /// The test that matters: a mesh vertex, rotated by the quaternion the
    /// entity carries, has to land exactly where transforming the vertex
    /// through Source and into block space puts it. Everything about props
    /// looking right rests on these two paths agreeing.
    #[test]
    fn the_quaternion_agrees_with_placing_the_vertex_directly() {
        let transform = transform();
        let units = transform.units_per_block();
        for angles in [
            [0.0, 0.0, 0.0],
            [0.0, 90.0, 0.0],
            [0.0, -135.0, 0.0],
            [90.0, 0.0, 0.0],
            [0.0, 0.0, 90.0],
            [15.0, 200.0, -70.0],
            [-33.0, 12.0, 175.0],
        ] {
            let prop = prop(angles);
            let q = rotation(&prop, &transform);
            for v in [
                Vec3::new(32.0, 0.0, 0.0),
                Vec3::new(0.0, 16.0, 0.0),
                Vec3::new(0.0, 0.0, 64.0),
                Vec3::new(11.0, -23.0, 7.0),
            ] {
                // The mesh as written: Source model space mapped to blocks.
                let mesh = Vec3::new(v.x / units, v.z / units, -v.y / units);
                let placed = rotate(q, mesh);
                // Where the vertex really belongs, in block space.
                let want = transform.to_block_space(prop.place(v))
                    - transform.to_block_space(prop.origin);
                assert!(
                    (placed - want).length() < 1e-9,
                    "{angles:?}: {v:?} placed at {placed:?}, want {want:?}"
                );
            }
        }
    }

    /// An unrotated prop must get the identity, or every prop in every map is
    /// subtly turned.
    #[test]
    fn no_rotation_is_the_identity_quaternion() {
        let q = rotation(&prop([0.0, 0.0, 0.0]), &transform());
        assert!((q[3].abs() - 1.0).abs() < 1e-12, "{q:?} is not the identity");
    }

    #[test]
    fn quaternions_come_out_unit_length() {
        for angles in [[0.0, 0.0, 0.0], [180.0, 0.0, 0.0], [0.0, 180.0, 0.0], [45.0, 45.0, 45.0]] {
            let q = rotation(&prop(angles), &transform());
            let length = q.iter().map(|c| c * c).sum::<f64>().sqrt();
            assert!((length - 1.0).abs() < 1e-9, "{angles:?} gave length {length}");
        }
    }

    fn placement() -> Placement {
        Placement {
            block: "kubejs:prop_x".into(),
            pos: [10.5, 64.0, -20.25],
            rotation: [0.0, std::f64::consts::FRAC_1_SQRT_2, 0.0, std::f64::consts::FRAC_1_SQRT_2],
            scale: 1.0,
            width: 2.0,
            height: 3.0,
            view_range: 1.0,
            full_bright: false,
            tag: "src2mc_map".into(),
        }
    }

    /// The specification puts `Pos` relative to the schematic's own origin,
    /// which is not the same as the entity's own `Pos` inside `Data`.
    #[test]
    fn schematic_positions_are_relative_to_the_region() {
        let entity = placement().entity([10, 60, -30]);
        assert_eq!(entity.pos, vec![0.5, 4.0, 9.75]);
        assert_eq!(entity.data.pos, vec![10.5, 64.0, -20.25]);
        assert_eq!(entity.id, KIND);
        assert_eq!(entity.data.block_state.name, "kubejs:prop_x");
    }

    /// WorldEdit reads `Rotation` out of `Data` without checking whether it is
    /// there, and one missing tag fails the whole schematic load — not the one
    /// entity — with `NoSuchElementException`.
    #[test]
    fn every_entity_carries_the_tags_worldedit_demands() {
        let entity = placement().entity([0, 0, 0]);
        assert_eq!(entity.data.rotation, vec![0.0, 0.0], "Rotation must be present");
        assert_eq!(entity.pos.len(), 3, "Pos must be a triple");
        assert!(!entity.id.is_empty(), "Id names the entity type");
        assert!(!entity.data.id.is_empty(), "Data carries the id too");
    }

    /// A culling box of zero means "never cull", which would render every prop
    /// in the map every frame.
    #[test]
    fn placements_carry_a_culling_box() {
        let entity = placement().entity([0, 0, 0]);
        assert!(entity.data.width > 0.0 && entity.data.height > 0.0);
        assert!(entity.data.view_range > 0.0);
    }

    /// Both routes have to place the same prop the same way, or the fallback is
    /// not a fallback.
    #[test]
    fn the_command_and_the_entity_agree() {
        let placement = placement();
        let command = placement.summon();
        let entity = placement.entity([0, 0, 0]);

        assert!(command.starts_with("summon minecraft:block_display 10.500 64.000 -20.250"));
        assert!(command.contains("Name:\"kubejs:prop_x\""));
        assert!(command.contains("0.707107f"), "the rotation is in the command");
        assert_eq!(
            entity.data.transformation.left_rotation[1],
            std::f64::consts::FRAC_1_SQRT_2 as f32
        );
        assert!(command.contains("src2mc_map"));
        assert!(entity.data.tags.contains(&"src2mc_map".to_string()));
    }

    #[test]
    fn full_bright_is_opt_in() {
        let mut placement = placement();
        assert!(!placement.summon().contains("brightness"));
        assert!(placement.entity([0, 0, 0]).data.brightness.is_none());

        placement.full_bright = true;
        assert!(placement.summon().contains("brightness:{block:15,sky:15}"));
        assert!(placement.entity([0, 0, 0]).data.brightness.is_some());
    }

    /// Running the function twice must not double every prop.
    #[test]
    fn the_function_clears_its_own_props_first() {
        let text = function(&[placement()], "d1_trainstation_02");
        let kill = text.find("kill @e").expect("no cleanup line");
        let summon = text.find("summon ").expect("no props");
        assert!(kill < summon, "cleanup has to come first");
        assert!(text.contains("tag=src2mc_d1_trainstation_02"));
        assert_eq!(text.matches("summon ").count(), 1, "one placement, one summon");
    }

    #[test]
    fn tags_are_legal_scoreboard_tags() {
        assert_eq!(tag_for("d1_trainstation_02"), "src2mc_d1_trainstation_02");
        assert!(!tag_for("a map/name").contains('/'));
        assert!(!tag_for("a map/name").contains(' '));
    }
}
