//! Deterministic, lossless partitioning for mod-owned texture pages.

use anyhow::{Result, ensure};
use image::{Rgba, RgbaImage, imageops::FilterType};

pub const PAGE_SIZE: u32 = 4096;
pub const MAX_MIP_LEVEL: u8 = 4;
pub const GUTTER: u32 = 1 << MAX_MIP_LEVEL;
pub const USABLE_AXIS: u32 = PAGE_SIZE - 2 * GUTTER;
pub const TEXELS_PER_BLOCK: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionDecision {
    pub original: [u32; 2],
    pub output: [u32; 2],
    pub resampled: bool,
}

/// Retain up to 16 output texels per projected block, without inventing detail
/// by enlarging an image beyond its Source dimensions.
pub fn analyze_resolution(
    original: [u32; 2],
    blocks_spanned: [f64; 2],
) -> Result<ResolutionDecision> {
    ensure!(
        original[0] > 0 && original[1] > 0,
        "texture dimensions must be positive"
    );
    ensure!(
        blocks_spanned.iter().all(|v| v.is_finite() && *v > 0.0),
        "texture projection span must be finite and positive"
    );
    let output = std::array::from_fn(|axis| {
        let required = (blocks_spanned[axis] * TEXELS_PER_BLOCK)
            .round()
            .clamp(1.0, f64::from(u32::MAX)) as u32;
        required.min(original[axis])
    });
    Ok(ResolutionDecision {
        original,
        output,
        resampled: output != original,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalTexture {
    pub content_id: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub content_id: String,
    /// Pixel rectangle in the unsplit logical texture.
    pub source: Rect,
    pub page: u32,
    /// Page rectangle excluding its extruded gutter.
    pub allocation: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub page_count: u32,
    pub regions: Vec<Region>,
}

#[derive(Debug)]
pub struct ImageAsset {
    pub content_id: String,
    pub image: RgbaImage,
}

#[derive(Debug)]
pub struct PageImages {
    /// One complete page image per mip level, largest first.
    pub mips: Vec<RgbaImage>,
}

#[derive(Debug, Clone)]
struct Piece {
    content_id: String,
    source: Rect,
}

/// Deterministic shelf packing. Discovery order cannot change the result.
/// Oversized images become source rectangles without changing pixel count.
pub fn pack(textures: &[LogicalTexture]) -> Result<Layout> {
    let mut ordered = textures.to_vec();
    ordered.sort_by(|a, b| a.content_id.cmp(&b.content_id));
    ensure!(
        ordered
            .windows(2)
            .all(|p| p[0].content_id != p[1].content_id),
        "duplicate logical texture content ID"
    );
    let mut pieces = Vec::new();
    for texture in ordered {
        ensure!(
            texture.width > 0 && texture.height > 0,
            "texture dimensions must be positive"
        );
        for y in (0..texture.height).step_by(USABLE_AXIS as usize) {
            for x in (0..texture.width).step_by(USABLE_AXIS as usize) {
                pieces.push(Piece {
                    content_id: texture.content_id.clone(),
                    source: Rect {
                        x,
                        y,
                        width: (texture.width - x).min(USABLE_AXIS),
                        height: (texture.height - y).min(USABLE_AXIS),
                    },
                });
            }
        }
    }

    let mut regions = Vec::with_capacity(pieces.len());
    let (mut page, mut cursor_x, mut cursor_y, mut row_height) = (0u32, 0u32, 0u32, 0u32);
    for piece in pieces {
        let packed_width = align(piece.source.width + 2 * GUTTER, 1 << MAX_MIP_LEVEL);
        let packed_height = align(piece.source.height + 2 * GUTTER, 1 << MAX_MIP_LEVEL);
        ensure!(
            packed_width <= PAGE_SIZE && packed_height <= PAGE_SIZE,
            "atlas piece exceeds page"
        );
        if cursor_x + packed_width > PAGE_SIZE {
            cursor_x = 0;
            cursor_y += row_height;
            row_height = 0;
        }
        if cursor_y + packed_height > PAGE_SIZE {
            page = page
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("atlas page count overflow"))?;
            cursor_x = 0;
            cursor_y = 0;
            row_height = 0;
        }
        regions.push(Region {
            content_id: piece.content_id,
            source: piece.source,
            page,
            allocation: Rect {
                x: cursor_x + GUTTER,
                y: cursor_y + GUTTER,
                width: piece.source.width,
                height: piece.source.height,
            },
        });
        cursor_x += packed_width;
        row_height = row_height.max(packed_height);
    }
    Ok(Layout {
        page_count: if regions.is_empty() { 0 } else { page + 1 },
        regions,
    })
}

/// Materialize packed pages. Gutter samples come from the logical image, not
/// merely the partition edge, so a partition boundary stays continuous.
pub fn build_pages(layout: &Layout, assets: &[ImageAsset]) -> Result<Vec<PageImages>> {
    use std::collections::BTreeMap;
    let by_id: BTreeMap<_, _> = assets
        .iter()
        .map(|asset| (asset.content_id.as_str(), &asset.image))
        .collect();
    ensure!(
        by_id.len() == assets.len(),
        "duplicate texture image content ID"
    );
    let mut bases: Vec<RgbaImage> = (0..layout.page_count)
        .map(|_| RgbaImage::from_pixel(PAGE_SIZE, PAGE_SIZE, Rgba([0, 0, 0, 0])))
        .collect();
    for region in &layout.regions {
        let source = by_id
            .get(region.content_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing logical texture {}", region.content_id))?;
        ensure!(
            region.source.x + region.source.width <= source.width()
                && region.source.y + region.source.height <= source.height(),
            "atlas source region exceeds image"
        );
        let page = bases
            .get_mut(region.page as usize)
            .ok_or_else(|| anyhow::anyhow!("atlas page index out of range"))?;
        for dy in -(GUTTER as i64)..i64::from(region.source.height + GUTTER) {
            for dx in -(GUTTER as i64)..i64::from(region.source.width + GUTTER) {
                let source_x =
                    (i64::from(region.source.x) + dx).rem_euclid(i64::from(source.width())) as u32;
                let source_y =
                    (i64::from(region.source.y) + dy).rem_euclid(i64::from(source.height())) as u32;
                let page_x = (i64::from(region.allocation.x) + dx) as u32;
                let page_y = (i64::from(region.allocation.y) + dy) as u32;
                page.put_pixel(page_x, page_y, *source.get_pixel(source_x, source_y));
            }
        }
    }
    Ok(bases
        .into_iter()
        .map(|base| {
            let mut mips = vec![base];
            for level in 1..=MAX_MIP_LEVEL {
                let size = PAGE_SIZE >> level;
                mips.push(image::imageops::resize(
                    &mips[0],
                    size,
                    size,
                    FilterType::Triangle,
                ));
            }
            PageImages { mips }
        })
        .collect())
}

fn align(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(id: &str, width: u32, height: u32) -> LogicalTexture {
        LogicalTexture {
            content_id: id.into(),
            width,
            height,
        }
    }

    #[test]
    fn oversized_texture_is_partitioned_without_resampling() {
        let layout = pack(&[texture("a", 8192, 5000)]).unwrap();
        assert!(layout.regions.len() > 1);
        let area: u64 = layout
            .regions
            .iter()
            .map(|r| u64::from(r.source.width) * u64::from(r.source.height))
            .sum();
        assert_eq!(area, 8192 * 5000);
        for region in &layout.regions {
            assert!(region.allocation.x + region.allocation.width + GUTTER <= PAGE_SIZE);
            assert!(region.allocation.y + region.allocation.height + GUTTER <= PAGE_SIZE);
        }
    }

    #[test]
    fn packing_is_independent_of_discovery_order() {
        let a = texture("a", 64, 64);
        let b = texture("b", 128, 32);
        assert_eq!(
            pack(&[a.clone(), b.clone()]).unwrap(),
            pack(&[b, a]).unwrap()
        );
    }

    #[test]
    fn every_region_keeps_a_gutter_through_mip_four() {
        let layout = pack(&[texture("a", 31, 47), texture("b", 4000, 64)]).unwrap();
        for level in 0..=MAX_MIP_LEVEL {
            assert!(GUTTER >> level >= 1);
            for region in &layout.regions {
                assert_eq!(region.allocation.x % (1 << level), 0);
                assert_eq!(region.allocation.y % (1 << level), 0);
            }
        }
    }

    #[test]
    fn invalid_and_duplicate_inputs_fail() {
        assert!(pack(&[texture("a", 0, 1)]).is_err());
        assert!(pack(&[texture("a", 1, 1), texture("a", 2, 2)]).is_err());
    }

    #[test]
    fn resolution_targets_sixteen_texels_per_projected_block() {
        assert_eq!(
            analyze_resolution([512, 256], [4.0, 2.0]).unwrap(),
            ResolutionDecision {
                original: [512, 256],
                output: [64, 32],
                resampled: true
            }
        );
        assert_eq!(
            analyze_resolution([8, 8], [4.0, 4.0]).unwrap().output,
            [8, 8]
        );
        assert!(analyze_resolution([1, 1], [f64::NAN, 1.0]).is_err());
    }

    #[test]
    fn four_block_1024_texture_keeps_sixteen_texels_per_block() {
        // Regression for the INFRA floor: once its texture-info record is
        // resolved to its owning material, a 1024 texture repeats across four
        // Minecraft blocks and therefore needs a 64 pixel export.
        assert_eq!(
            analyze_resolution([1024, 1024], [4.0, 4.0]).unwrap().output,
            [64, 64]
        );
    }
}
