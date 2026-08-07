//! Where a map places its static props.
//!
//! `prop_static` entities do not survive compilation: VBSP strips them from
//! the entity lump and writes them into the `sprp` game lump as a dictionary
//! of model paths plus one record per placement. That record is a position and
//! a set of Euler angles, so putting a model back where the mapper put it
//! means reproducing Source's own `AngleMatrix`.

use crate::geom::Vec3;

/// One placed static prop.
#[derive(Debug, Clone)]
pub struct Prop {
    /// Model path as the dictionary spells it, e.g.
    /// `models/props_c17/fence01a.mdl`.
    pub model: String,
    pub origin: Vec3,
    /// Pitch, yaw and roll in degrees, as authored.
    pub angles: [f64; 3],
    /// Uniform scale. Only later Source branches store one; older lumps are
    /// always 1.
    pub scale: f64,
    /// The entity this came from, or `prop_static` for the `sprp` lump, which
    /// no longer has entities of its own by the time the map is compiled.
    pub classname: String,
}

impl Prop {
    /// Rotate and translate a point from model space into world space.
    pub fn place(&self, v: Vec3) -> Vec3 {
        self.rotate(v * self.scale) + self.origin
    }

    /// Rotate a direction from model space into world space, without moving or
    /// scaling it.
    ///
    /// What the display-entity path needs: a prop rendered as its own mesh is
    /// placed by position and rotation separately, rather than by transforming
    /// every vertex.
    pub fn rotate(&self, v: Vec3) -> Vec3 {
        let m = self.rotation();
        Vec3::new(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }

    /// Source's `AngleMatrix`, as rows.
    ///
    /// The composition is yaw, then pitch, then roll, about Z, Y and X — and
    /// the pitch sign is inverted relative to the usual right-handed
    /// convention, which is why this is spelled out rather than assembled from
    /// three generic rotations. Getting it wrong lays every fence on its side.
    fn rotation(&self) -> [[f64; 3]; 3] {
        let [pitch, yaw, roll] = self.angles.map(f64::to_radians);
        let (sp, cp) = pitch.sin_cos();
        let (sy, cy) = yaw.sin_cos();
        let (sr, cr) = roll.sin_cos();
        [
            [cp * cy, sr * sp * cy - cr * sy, cr * sp * cy + sr * sy],
            [cp * sy, sr * sp * sy + cr * cy, cr * sp * sy - sr * cy],
            [-sp, sr * cp, cr * cp],
        ]
    }
}

/// Every static prop in the map, in placement order.
pub fn extract(bsp: &vbsp::Bsp) -> Vec<Prop> {
    let names = &bsp.static_props.dict.name;
    bsp.static_props
        .props
        .props
        .iter()
        .filter(|prop| !prop.flags.contains(vbsp::data::StaticPropLumpFlags::NO_DRAW))
        .filter_map(|prop| {
            let model = names.get(prop.prop_type as usize)?.as_str();
            (!model.is_empty()).then(|| Prop {
                model: model.replace('\\', "/").to_ascii_lowercase(),
                origin: Vec3::from(prop.origin),
                angles: [
                    prop.angles.pitch as f64,
                    prop.angles.yaw as f64,
                    prop.angles.roll as f64,
                ],
                scale: 1.0,
                classname: "prop_static".to_string(),
            })
        })
        .collect()
}

/// Every prop the *entity lump* places, in lump order.
///
/// The `sprp` lump is only half the story. `prop_static` is what the compiler
/// bakes away, but a map's crates, barrels, doors, cars and set dressing that
/// can be shot, opened or thrown are `prop_physics`, `prop_dynamic` and their
/// relatives, and those stay in the entity lump as ordinary entities with a
/// `model` key. Leaving them out is why a converted warehouse is an empty
/// warehouse.
///
/// Anything naming a `.mdl` counts, whatever its classname: the point is the
/// model, and enumerating classnames would only miss the mod-specific ones
/// Entropy: Zero adds. Brush entities name their model as `*12` and are
/// handled elsewhere, so they fall out here for want of a `.mdl`.
pub fn extract_entities(bsp: &vbsp::Bsp) -> Vec<Prop> {
    bsp.entities
        .iter()
        .filter_map(|raw| {
            let mut model = None;
            let mut origin = None;
            let mut angles = [0.0; 3];
            let mut scale = 1.0;
            let mut classname = String::new();
            for (key, value) in raw.properties() {
                match key {
                    "model" => model = Some(value.to_string()),
                    "classname" => classname = value.to_string(),
                    "origin" => origin = triple(value),
                    "angles" => angles = triple(value).unwrap_or([0.0; 3]),
                    // Two spellings, one meaning; whichever is present wins.
                    "uniformscale" | "modelscale" => {
                        scale = value.parse().ok().filter(|s| *s > 0.0).unwrap_or(1.0)
                    }
                    _ => {}
                }
            }

            let model = model?.replace('\\', "/").to_ascii_lowercase();
            if !model.ends_with(".mdl") {
                return None;
            }
            Some(Prop {
                model,
                origin: origin.map(|[x, y, z]| Vec3::new(x, y, z))?,
                angles,
                scale,
                classname,
            })
        })
        .collect()
}

fn triple(value: &str) -> Option<[f64; 3]> {
    let mut parts = value.split_whitespace().filter_map(|p| p.parse::<f64>().ok());
    Some([parts.next()?, parts.next()?, parts.next()?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(angles: [f64; 3]) -> Prop {
        Prop {
            model: "models/x.mdl".into(),
            origin: Vec3::ZERO,
            angles,
            scale: 1.0,
            classname: "prop_static".into(),
        }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn an_unrotated_prop_only_moves() {
        let mut p = prop([0.0, 0.0, 0.0]);
        p.origin = Vec3::new(10.0, -20.0, 5.0);
        assert!(close(p.place(Vec3::new(1.0, 2.0, 3.0)), Vec3::new(11.0, -18.0, 8.0)));
    }

    /// Yaw turns a model about the vertical axis; Source's forward is +X.
    #[test]
    fn yaw_turns_the_model_about_z() {
        let p = prop([0.0, 90.0, 0.0]);
        assert!(close(p.place(Vec3::new(1.0, 0.0, 0.0)), Vec3::new(0.0, 1.0, 0.0)));
        assert!(close(p.place(Vec3::new(0.0, 0.0, 1.0)), Vec3::new(0.0, 0.0, 1.0)));
    }

    /// Positive pitch tips the nose *down* in Source, which is the sign
    /// inversion that makes this worth a test.
    #[test]
    fn positive_pitch_points_forward_downwards() {
        let p = prop([90.0, 0.0, 0.0]);
        assert!(close(p.place(Vec3::new(1.0, 0.0, 0.0)), Vec3::new(0.0, 0.0, -1.0)));
    }

    #[test]
    fn roll_turns_the_model_about_its_own_forward_axis() {
        let p = prop([0.0, 0.0, 90.0]);
        assert!(close(p.place(Vec3::new(0.0, 1.0, 0.0)), Vec3::new(0.0, 0.0, 1.0)));
        assert!(close(p.place(Vec3::new(1.0, 0.0, 0.0)), Vec3::new(1.0, 0.0, 0.0)));
    }

    /// Rotation must not stretch anything, whatever the angles.
    #[test]
    fn rotation_preserves_length() {
        let v = Vec3::new(3.0, -4.0, 12.0);
        for angles in [
            [0.0, 0.0, 0.0],
            [15.0, 200.0, -70.0],
            [-90.0, 45.0, 180.0],
            [33.0, -12.0, 7.5],
        ] {
            let placed = prop(angles).place(v);
            assert!(
                (placed.length() - v.length()).abs() < 1e-9,
                "{angles:?} scaled {} to {}",
                v.length(),
                placed.length()
            );
        }
    }

    #[test]
    fn scale_multiplies_model_space_only() {
        let mut p = prop([0.0, 0.0, 0.0]);
        p.origin = Vec3::new(100.0, 0.0, 0.0);
        p.scale = 2.0;
        assert!(close(p.place(Vec3::new(1.0, 0.0, 0.0)), Vec3::new(102.0, 0.0, 0.0)));
    }
}
