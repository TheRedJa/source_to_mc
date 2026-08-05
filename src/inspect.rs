//! The `inspect` report: what a map contains and what converting it would cost.

use crate::bsp::{Map, entities};
use crate::config::Config;
use crate::geom::Aabb;
use crate::voxel::transform::Transform;
use crate::{DIMENSION_MAX_HEIGHT, DIMENSION_MAX_Y, DIMENSION_MIN_Y, VANILLA_MAX_Y, VANILLA_MIN_Y};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub map: String,
    pub bsp_version: i32,
    pub units_per_block: f64,

    pub bounds_source: BoundsReport,
    pub bounds_blocks: BoundsReport,

    pub brushes_total: usize,
    pub brushes_worldspawn: usize,
    pub brush_entity_models: usize,
    pub displacements: usize,
    pub static_props: usize,
    pub materials: usize,
    pub entities: usize,

    /// Volume of the map's bounding box in blocks; the real block count is far
    /// lower, but this bounds it.
    pub bounding_volume_blocks: u128,
    pub y_range: YRangeReport,
    pub top_classnames: Vec<(String, usize)>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoundsReport {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub size: [f64; 3],
}

impl From<Aabb> for BoundsReport {
    fn from(b: Aabb) -> Self {
        let (min, max, size) = (b.min, b.max, b.size());
        BoundsReport {
            min: [min.x, min.y, min.z],
            max: [max.x, max.y, max.z],
            size: [size.x, size.y, size.z],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct YRangeReport {
    pub min_y: i32,
    pub max_y: i32,
    pub height: i32,
    pub fits_vanilla: bool,
    /// A `min_y` for a custom dimension type: a multiple of 16 at or below the
    /// map's floor.
    pub dimension_min_y: i32,
    /// Matching `height`, rounded up to a multiple of 16.
    pub dimension_height: i32,
    pub fits_custom_dimension: bool,
}

fn round_down_to_16(v: i32) -> i32 {
    v.div_euclid(16) * 16
}

fn round_up_to_16(v: i32) -> i32 {
    v.div_euclid(16) * 16 + if v.rem_euclid(16) == 0 { 0 } else { 16 }
}

fn y_range(bounds_blocks: Aabb) -> YRangeReport {
    let min_y = bounds_blocks.min.y.floor() as i32;
    let max_y = bounds_blocks.max.y.ceil() as i32;
    let height = max_y - min_y + 1;

    let dimension_min_y = round_down_to_16(min_y).clamp(DIMENSION_MIN_Y, DIMENSION_MAX_Y);
    let dimension_height = round_up_to_16(max_y - dimension_min_y + 1).max(16);

    YRangeReport {
        min_y,
        max_y,
        height,
        fits_vanilla: min_y >= VANILLA_MIN_Y && max_y <= VANILLA_MAX_Y,
        dimension_min_y,
        dimension_height,
        fits_custom_dimension: dimension_height <= DIMENSION_MAX_HEIGHT
            && dimension_min_y + dimension_height - 1 <= DIMENSION_MAX_Y,
    }
}

pub fn report(map: &Map, config: &Config) -> Report {
    let bounds_source = map.bounds();
    let transform = Transform::new(config, bounds_source);
    let bounds_blocks = transform.transform_bounds(bounds_source);

    let entity_records = entities::extract(map, &transform);
    let worldspawn = map.model_brushes(0).len();
    let y_range = y_range(bounds_blocks);

    let size = bounds_blocks.size();
    let bounding_volume_blocks = (size.x.ceil().max(0.0) as u128)
        * (size.y.ceil().max(0.0) as u128)
        * (size.z.ceil().max(0.0) as u128);

    let mut warnings = Vec::new();
    if !y_range.fits_vanilla {
        warnings.push(format!(
            "Map needs Y {}..{} ({} blocks), outside vanilla's {VANILLA_MIN_Y}..{VANILLA_MAX_Y}. \
             Use a custom dimension_type with min_y = {} and height = {} (--emit-dimension), \
             or raise --units-per-block.",
            y_range.min_y, y_range.max_y, y_range.height, y_range.dimension_min_y, y_range.dimension_height,
        ));
    }
    if !y_range.fits_custom_dimension {
        warnings.push(format!(
            "Map needs {} blocks of height, beyond the {DIMENSION_MAX_HEIGHT}-block maximum a \
             dimension_type allows. Increase --units-per-block.",
            y_range.height,
        ));
    }
    if map.repaired_bytes > 0 {
        warnings.push(format!(
            "Entity lump contained {} byte(s) of invalid UTF-8, repaired in memory. \
             Affected key values may read slightly differently than authored.",
            map.repaired_bytes,
        ));
    }
    if worldspawn == 0 {
        warnings.push("No worldspawn brushes found; the map may use an unexpected lump layout.".into());
    }

    Report {
        map: map.name.clone(),
        bsp_version: map.bsp.header.version.clone() as i32,
        units_per_block: config.scale.units_per_block,
        bounds_source: bounds_source.into(),
        bounds_blocks: bounds_blocks.into(),
        brushes_total: map.bsp.brushes.len(),
        brushes_worldspawn: worldspawn,
        brush_entity_models: map.bsp.models.len().saturating_sub(1),
        displacements: map.bsp.displacements.len(),
        static_props: map.bsp.static_props().count(),
        materials: map.materials().len(),
        entities: entity_records.len(),
        bounding_volume_blocks,
        y_range,
        top_classnames: entities::classname_histogram(&entity_records)
            .into_iter()
            .take(15)
            .collect(),
        warnings,
    }
}

impl Report {
    /// Human-readable form for the terminal.
    pub fn render(&self) -> String {
        use std::fmt::Write;
        let mut s = String::new();

        let _ = writeln!(s, "{} (BSP v{})", self.map, self.bsp_version);
        let _ = writeln!(s, "  scale            {} units/block", self.units_per_block);

        let src = &self.bounds_source.size;
        let blk = &self.bounds_blocks.size;
        let _ = writeln!(s, "  size (source)    {:.0} x {:.0} x {:.0} units", src[0], src[1], src[2]);
        let _ = writeln!(s, "  size (blocks)    {:.0} x {:.0} x {:.0}", blk[0], blk[1], blk[2]);
        let _ = writeln!(
            s,
            "  bounding volume  {} blocks",
            thousands(self.bounding_volume_blocks)
        );

        let y = &self.y_range;
        let fit = if y.fits_vanilla {
            "fits vanilla".to_string()
        } else {
            format!(
                "needs dimension_type min_y={} height={}",
                y.dimension_min_y, y.dimension_height
            )
        };
        let _ = writeln!(s, "  Y range          {}..{} ({} blocks, {fit})", y.min_y, y.max_y, y.height);

        let _ = writeln!(s);
        let _ = writeln!(s, "  brushes          {} ({} worldspawn)", self.brushes_total, self.brushes_worldspawn);
        let _ = writeln!(s, "  brush entities   {}", self.brush_entity_models);
        let _ = writeln!(s, "  displacements    {}", self.displacements);
        let _ = writeln!(s, "  static props     {}", self.static_props);
        let _ = writeln!(s, "  materials        {}", self.materials);
        let _ = writeln!(s, "  entities         {}", self.entities);

        if !self.top_classnames.is_empty() {
            let _ = writeln!(s, "\n  most common entities:");
            for (classname, count) in &self.top_classnames {
                let _ = writeln!(s, "    {count:>6}  {classname}");
            }
        }

        for warning in &self.warnings {
            let _ = writeln!(s, "\nwarning: {warning}");
        }
        s
    }
}

fn thousands(mut n: u128) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut parts = Vec::new();
    while n > 0 {
        parts.push(format!("{:03}", n % 1000));
        n /= 1000;
    }
    parts.reverse();
    let joined = parts.join(",");
    joined.trim_start_matches('0').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Vec3;

    #[test]
    fn rounds_to_dimension_multiples_of_16() {
        assert_eq!(round_down_to_16(0), 0);
        assert_eq!(round_down_to_16(15), 0);
        assert_eq!(round_down_to_16(-1), -16);
        assert_eq!(round_down_to_16(-64), -64);
        assert_eq!(round_up_to_16(1), 16);
        assert_eq!(round_up_to_16(16), 16);
        assert_eq!(round_up_to_16(17), 32);
    }

    #[test]
    fn shallow_map_fits_vanilla() {
        let bounds = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 200.0, 100.0));
        let y = y_range(bounds);
        assert!(y.fits_vanilla);
        assert!(y.fits_custom_dimension);
    }

    #[test]
    fn tall_map_needs_a_custom_dimension_but_still_fits() {
        // 1000 blocks tall: far past vanilla, well inside a dimension_type.
        let bounds = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 1000.0, 100.0));
        let y = y_range(bounds);
        assert!(!y.fits_vanilla);
        assert!(y.fits_custom_dimension);
        assert_eq!(y.dimension_min_y % 16, 0);
        assert_eq!(y.dimension_height % 16, 0);
        assert!(y.dimension_min_y <= y.min_y);
        assert!(y.dimension_min_y + y.dimension_height - 1 >= y.max_y);
    }

    #[test]
    fn absurdly_tall_map_exceeds_even_a_custom_dimension() {
        let bounds = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 5000.0, 100.0));
        assert!(!y_range(bounds).fits_custom_dimension);
    }

    #[test]
    fn formats_large_numbers() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
