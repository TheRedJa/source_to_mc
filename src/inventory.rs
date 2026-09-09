//! Phase 0.5 inventory used before the mod bundle format is locked.

use crate::bsp::Map;
use crate::config::{Config, MaterialMode};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub map: String,
    pub units_per_block: f64,
    pub schematic_dimensions: [u64; 3],
    pub projected_schematic_volume: u128,
    pub source_materials: usize,
    pub texture_memory: crate::source::report::TextureMemoryEstimate,
    pub blocks_before_hollowing: usize,
    pub output_cells: usize,
    pub visible_faces: usize,
    pub distinct_prop_models: usize,
    pub prop_triangles: usize,
    pub prop_placements: usize,
    pub collision_cells: usize,
}

pub fn report(map: &Map, config: &Config) -> anyhow::Result<Report> {
    validate_scale(config.scale.units_per_block)?;

    let mut conversion_config = config.clone();
    conversion_config.materials.mode = MaterialMode::Kubejs;
    conversion_config.props.models = true;

    let map_report = crate::inspect::report(map, &conversion_config);
    let dimensions = map_report
        .bounds_blocks
        .size
        .map(|axis| axis.ceil().max(0.0) as u64);
    let texture_report = crate::source::report::report(map, &conversion_config);
    let conversion = crate::convert::convert(map, &conversion_config)?;
    let stats = conversion.stats;

    Ok(Report {
        map: map.name.clone(),
        units_per_block: conversion_config.scale.units_per_block,
        schematic_dimensions: dimensions,
        projected_schematic_volume: dimensions.into_iter().map(u128::from).product(),
        source_materials: map.materials().len(),
        texture_memory: texture_report.memory_estimate(),
        blocks_before_hollowing: stats.blocks_before_hollow,
        output_cells: stats.blocks,
        visible_faces: stats.visible_faces,
        distinct_prop_models: stats.prop_models,
        prop_triangles: stats.prop_triangles,
        prop_placements: stats.props_modelled,
        collision_cells: stats.prop_collision_blocks + stats.prop_barriers,
    })
}

fn validate_scale(units_per_block: f64) -> anyhow::Result<()> {
    anyhow::ensure!(
        (units_per_block - 32.0).abs() < f64::EPSILON,
        "the Phase 0.5 mod inventory is fixed at 32 Source units per block"
    );
    Ok(())
}

impl Report {
    pub fn render(&self) -> String {
        format!(
            "{} at {} units/block\n\
             \x20 schematic bounds  {} x {} x {} ({} cells projected)\n\
             \x20 output geometry   {} cells, {} before hollowing, {} visible faces\n\
             \x20 Source content    {} materials, {} textures, {} models / {} triangles / {} placements\n\
             \x20 texture memory    {} encoded, {} decoded RGBA8, {} mipmapped RGBA8\n\
             \x20 collision         {} occupied cells\n",
            self.map,
            self.units_per_block,
            self.schematic_dimensions[0],
            self.schematic_dimensions[1],
            self.schematic_dimensions[2],
            self.projected_schematic_volume,
            self.output_cells,
            self.blocks_before_hollowing,
            self.visible_faces,
            self.source_materials,
            self.texture_memory.distinct_textures,
            self.distinct_prop_models,
            self.prop_triangles,
            self.prop_placements,
            bytes(self.texture_memory.encoded_bytes),
            bytes(self.texture_memory.decoded_rgba_bytes),
            bytes(self.texture_memory.mipmapped_rgba_bytes),
            self.collision_cells,
        )
    }
}

fn bytes(value: u64) -> String {
    format!("{:.1} MiB", value as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_refuses_a_scale_that_cannot_describe_the_target() {
        assert!(validate_scale(16.0).is_err());
        assert!(validate_scale(32.0).is_ok());
    }

    #[test]
    fn byte_format_is_stable() {
        assert_eq!(bytes(1024 * 1024), "1.0 MiB");
    }
}
