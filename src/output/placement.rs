//! Versioned map-local prop-root placement table.

use anyhow::{Result, ensure};

pub const MAGIC: [u8; 8] = *b"S2PROP\0\0";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// SHA-256-derived stable identity, independent of root-cell selection.
    pub stable_id: [u8; 32],
    /// Index into map metadata's model-reference array.
    pub model: u32,
    /// Authoritative storage cell; not the visible model origin.
    pub root_cell: [i32; 3],
    /// Exact model origin in map-local Minecraft block coordinates.
    pub translation: [f64; 3],
    /// Unit quaternion `[x, y, z, w]` in Minecraft axes.
    pub rotation: [f64; 4],
    pub scale: f64,
}

pub fn encode(mut placements: Vec<Placement>, model_count: u32) -> Result<Vec<u8>> {
    crate::output::limits::check_count(
        "prop placement",
        placements.len() as u64,
        crate::output::limits::MAX_PROPS_PER_MAP,
    )?;
    placements.sort_by_key(|p| p.stable_id);
    for pair in placements.windows(2) {
        ensure!(
            pair[0].stable_id != pair[1].stable_id,
            "duplicate stable prop ID"
        );
    }
    let count = u32::try_from(placements.len())?;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, count);
    for prop in placements {
        ensure!(
            prop.model < model_count,
            "prop model reference out of range"
        );
        for value in prop
            .translation
            .into_iter()
            .chain(prop.rotation)
            .chain([prop.scale])
        {
            ensure!(value.is_finite(), "non-finite prop transform");
        }
        ensure!(prop.scale > 0.0, "prop scale must be positive");
        let length_sq: f64 = prop.rotation.iter().map(|v| v * v).sum();
        ensure!(
            (length_sq - 1.0).abs() <= 1.0e-9,
            "prop quaternion is not unit length"
        );
        out.extend_from_slice(&prop.stable_id);
        put_u32(&mut out, prop.model);
        for value in prop.root_cell {
            put_i32(&mut out, value);
        }
        for value in prop
            .translation
            .into_iter()
            .chain(prop.rotation)
            .chain([prop.scale])
        {
            put_f64(&mut out, value);
        }
    }
    Ok(out)
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_f64(out: &mut Vec<u8>, value: f64) {
    let value = if value == 0.0 { 0.0 } else { value };
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(id: u8) -> Placement {
        Placement {
            stable_id: [id; 32],
            model: 0,
            root_cell: [1, 2, 3],
            translation: [-0.0, 2.5, 3.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
        }
    }

    #[test]
    fn order_does_not_change_bytes() {
        assert_eq!(
            encode(vec![prop(2), prop(1)], 1).unwrap(),
            encode(vec![prop(1), prop(2)], 1).unwrap()
        );
    }

    #[test]
    fn rejects_duplicate_ids_bad_models_and_transforms() {
        assert!(encode(vec![prop(1), prop(1)], 1).is_err());
        assert!(encode(vec![prop(1)], 0).is_err());
        let mut invalid = prop(1);
        invalid.rotation = [0.0; 4];
        assert!(encode(vec![invalid], 1).is_err());
    }
}
