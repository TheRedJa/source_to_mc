//! Assembly of complete v1 campaign bundles and their transport schematics.

use crate::geom::{Aabb, Vec3};
use crate::output::{atlas, bundle, metadata, placement, schem, surface};
use crate::voxel::brush::BlockSolid;
use crate::voxel::grid::{IVec3, Palette, VoxelGrid};
use crate::voxel::settle;
use crate::voxel::surface::FaceDirection;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub const UNITS_PER_BLOCK: f64 = 32.0;

pub struct ModelAsset {
    pub source_model: String,
    pub bytes: Vec<u8>,
    pub materials: Vec<u32>,
}

pub struct TextureAsset {
    pub content_id: String,
    pub bytes: Vec<u8>,
    pub image: image::RgbaImage,
}

pub struct Prop {
    pub source_ordinal: u64,
    pub source_model: String,
    pub model_content_id: String,
    pub root_cell: IVec3,
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: f64,
    pub material_ids: Vec<u32>,
}

pub struct MapExport {
    pub map_id: String,
    pub source_name: String,
    pub cell_min: IVec3,
    pub cell_max: IVec3,
    pub anchor_cell: IVec3,
    pub blocks: Vec<(IVec3, crate::voxel::grid::BlockId)>,
    pub palette: Palette,
    pub faces: Vec<surface::EncodedFace>,
    pub materials: Vec<metadata::MaterialReference>,
    pub textures: Vec<TextureAsset>,
    pub models: Vec<ModelAsset>,
    pub props: Vec<Prop>,
    /// Encoded prop visibility table; absent when the map has no usable PVS.
    pub pvs: Option<Vec<u8>>,
    pub diagnostics: metadata::Diagnostics,
}

pub struct WrittenCampaign {
    pub bundle: PathBuf,
    pub schematics: Vec<PathBuf>,
    pub manifest: bundle::Manifest,
}

/// Build one export map from the canonical converter result. The anchor gets
/// its own guaranteed-free layer directly below the map, as agreed for v1.
pub fn from_conversion(
    map: &crate::bsp::Map,
    config: &crate::config::Config,
    conversion: &crate::convert::Conversion,
) -> Result<MapExport> {
    ensure!(
        conversion.transform.units_per_block() == UNITS_PER_BLOCK,
        "mod export requires exactly 32 Source units per block"
    );
    let (grid_min, grid_max) = conversion
        .grid
        .bounds()
        .context("mod export produced an empty map")?;
    let anchor_cell = [
        grid_min[0],
        grid_min[1]
            .checked_sub(1)
            .context("anchor coordinate underflow")?,
        grid_min[2],
    ];
    let mut palette = Palette::new();
    let surface_block = palette.intern("src2mc:surface");
    palette.intern("src2mc:map_anchor");
    palette.intern("src2mc:prop_root");
    let blocks = conversion
        .grid
        .iter()
        .map(|(cell, _)| (cell, surface_block))
        .collect();
    let extracted = crate::source::extract::extract_mod_props(map, config);
    // Prop and surface sections both drive the visibility table: every
    // 16-block section a transformed prop box spans, or a surface face's
    // cell falls in, collects the clusters of leaves whose boxes overlap it,
    // so the runtime can reject sections no visible leaf covers.
    let mut pvs_sections: BTreeSet<[i32; 3]> = BTreeSet::new();
    let visibility = crate::bsp::pvs::ClusterVisibility::from_map(map);
    let pvs_index = visibility.as_ref().map(|visibility| {
        crate::bsp::pvs::LeafSectionIndex::build(visibility, |corner| {
            let block = conversion
                .transform
                .to_block_space(crate::geom::Vec3::new(corner[0], corner[1], corner[2]));
            [block.x, block.y, block.z]
        })
    });
    if pvs_index.is_some() {
        insert_surface_sections(&conversion.surfaces, &mut pvs_sections);
    }
    // Model UVs are normalized sheet coordinates. Preserve enough pixels for
    // the largest world-space use of each sheet; otherwise model-only
    // materials never enter the atlas and every prop silently becomes a
    // fallback material.
    let mut prop_texture_spans = BTreeMap::<String, f64>::new();
    for item in &extracted {
        for part in &item.model.parts {
            if part.uv_per_unit.is_finite() && part.uv_per_unit > 0.0 {
                let blocks = 1.0 / (part.uv_per_unit * UNITS_PER_BLOCK);
                prop_texture_spans
                    .entry(part.material.clone())
                    .and_modify(|old| *old = old.max(blocks))
                    .or_insert(blocks);
            }
        }
    }
    let (mut materials, textures, face_material_ids, prop_bucket_ids) =
        extract_materials(map, config, &prop_texture_spans, &conversion.surfaces);
    let faces = conversion
        .surfaces
        .iter()
        .copied()
        .zip(face_material_ids.iter().copied())
        .map(|(face, material_id)| {
            surface::EncodedFace::from_visible(face, surface::MaterialId(material_id))
        })
        .collect();
    let mut material_ids: BTreeMap<String, u32> =
        materials
            .iter()
            .enumerate()
            .fold(BTreeMap::new(), |mut ids, (i, m)| {
                ids.entry(m.source_material.clone()).or_insert(i as u32);
                ids
            });
    let mut model_by_path: BTreeMap<String, (String, Vec<u32>, Vec<u8>)> = BTreeMap::new();
    let mut model_occupancy: BTreeMap<String, PropOccupancy> = BTreeMap::new();
    for item in &extracted {
        if model_by_path.contains_key(&item.prop.model) {
            continue;
        }
        let (mesh, slots) = crate::output::mesh::from_source_model(&item.model, UNITS_PER_BLOCK)?;
        let slot_ids = slots
            .into_iter()
            .map(|name| {
                if let Some(id) = prop_bucket_ids.get(&name) {
                    return *id;
                }
                if let Some(id) = material_ids.get(&name) {
                    return *id;
                }
                let id = materials.len() as u32;
                material_ids.insert(name.clone(), id);
                materials.push(metadata::MaterialReference {
                    source_material: name,
                    source_material_raw: None,
                    render_class: metadata::RenderClass::Fallback,
                    texture: None,
                    surface_prop: None,
                    reflectivity: [0.0; 3],
                });
                id
            })
            .collect::<Vec<_>>();
        model_occupancy.insert(item.prop.model.clone(), PropOccupancy::build(&mesh));
        let bytes = crate::output::mesh::encode(&mesh)?;
        let id = bundle::content_id(&bytes);
        model_by_path.insert(item.prop.model.clone(), (id, slot_ids, bytes));
    }
    let mut visible_normals = BTreeMap::<IVec3, Vec<FaceDirection>>::new();
    for face in &conversion.surfaces {
        visible_normals
            .entry(face.cell)
            .or_default()
            .push(face.patch.direction);
    }
    for normals in visible_normals.values_mut() {
        normals.sort();
        normals.dedup();
    }

    // Stage 1: each prop's own settle shift and placement transform,
    // independent of every other prop. Settling still only ever sees world
    // geometry, exactly as before; the prop-to-prop relationships below are
    // layered on top of it rather than folded in, since props are
    // deliberately excluded from the voxel grid settling queries.
    let snap_cap = config.props.snap_max.min(MAX_PROP_SNAP_BLOCKS);
    struct PropStage {
        unsettled_bounds: Aabb,
        settle_shift: f64,
        settled_bounds: Aabb,
        transform: PropTransform,
    }
    let stages: Vec<PropStage> = extracted
        .iter()
        .map(|item| {
            let unsettled_bounds = conversion.transform.transform_bounds(item.bounds);
            let settle_shift = if config.props.settle {
                settle::offset(&conversion.grid, unsettled_bounds, config.props.settle_max)
            } else {
                0.0
            };
            let settled_bounds =
                translated_bounds(unsettled_bounds, Vec3::new(0.0, settle_shift, 0.0));
            let columns = crate::output::display::basis(&item.prop, &conversion.transform);
            let origin = conversion.transform.to_block_space(item.prop.origin)
                + Vec3::new(0.0, settle_shift, 0.0);
            PropStage {
                unsettled_bounds,
                settle_shift,
                settled_bounds,
                transform: PropTransform {
                    origin,
                    columns,
                    scale: item.prop.scale,
                },
            }
        })
        .collect();

    // Stage 2: each prop's own sub-block correction, judged against its own
    // mesh rather than its bounding box, as if it alone existed.
    struct PropSnap {
        offset: Vec3,
        intersects: bool,
        resolved: bool,
    }
    let individual: Vec<PropSnap> = extracted
        .iter()
        .zip(&stages)
        .map(|(item, stage)| {
            if !config.props.snap {
                return PropSnap {
                    offset: Vec3::ZERO,
                    intersects: false,
                    resolved: true,
                };
            }
            let occupancy = &model_occupancy[&item.prop.model];
            let snap = snap_prop_bounds(
                &conversion.grid,
                &conversion.solids,
                &conversion.solid_index,
                &visible_normals,
                stage.settled_bounds,
                occupancy,
                &stage.transform,
                snap_cap,
            );
            PropSnap {
                offset: snap.offset,
                intersects: snap.intersects,
                resolved: snap.resolved,
            }
        })
        .collect();

    // Stage 3: group touching or overlapping props into assemblies and let
    // whichever member needs the largest correction speak for the whole
    // group, so a pipe run or a railing moves as one piece or not at all
    // rather than splitting at whichever segment happened to clip.
    let settled_bounds: Vec<Aabb> = stages.iter().map(|stage| stage.settled_bounds).collect();
    let ordinals: Vec<u64> = extracted.iter().map(|item| item.source_ordinal).collect();
    let assembly_of = build_assemblies(&settled_bounds);
    let individual_offsets: Vec<Vec3> = individual.iter().map(|snap| snap.offset).collect();
    let mut offset_final = assign_assembly_offsets(&assembly_of, &individual_offsets, &ordinals);
    let mut settle_shift_final: Vec<f64> = stages.iter().map(|stage| stage.settle_shift).collect();

    // Stage 4: a prop whose base rests on another prop's top inherits that
    // prop's total shift exactly, so a crate does not sink into or float
    // above a pallet that settled or snapped by a slightly different amount.
    let unsettled_bounds: Vec<Aabb> = stages.iter().map(|stage| stage.unsettled_bounds).collect();
    let mut order: Vec<usize> = (0..extracted.len()).collect();
    order.sort_by(|&a, &b| {
        settled_bounds[a]
            .min
            .y
            .total_cmp(&settled_bounds[b].min.y)
            .then_with(|| ordinals[a].cmp(&ordinals[b]))
    });
    apply_stacking_inheritance(
        &order,
        &unsettled_bounds,
        &mut settle_shift_final,
        &mut offset_final,
        &ordinals,
    );

    let mut taken = std::collections::HashSet::new();
    let mut props = Vec::new();
    let mut diagnostics = Vec::new();
    let mut snapped_props = 0usize;
    let mut unresolved_props = 0usize;
    let mut maximum_snap_distance = 0.0f64;
    let mut settled_props = 0usize;
    let mut snap_diagnostics_emitted = 0usize;
    let mut cell_max = grid_max;
    let mut cell_min = anchor_cell;
    for (index, item) in extracted.into_iter().enumerate() {
        let settle_shift = settle_shift_final[index];
        if settle_shift != 0.0 {
            settled_props += 1;
        }
        let offset = offset_final[index];
        let original_bounds = translated_bounds(
            stages[index].unsettled_bounds,
            Vec3::new(0.0, settle_shift, 0.0),
        );
        let transformed_bounds = translated_bounds(original_bounds, offset);
        if offset != Vec3::ZERO {
            snapped_props += 1;
            maximum_snap_distance = maximum_snap_distance.max(offset.length());
            if snap_diagnostics_emitted < config.props.snap_diagnostics_limit {
                snap_diagnostics_emitted += 1;
                let mut context = BTreeMap::new();
                context.insert("source_ordinal".into(), item.source_ordinal.to_string());
                context.insert("source_model".into(), item.prop.model.clone());
                context.insert("offset".into(), format_vec3(offset));
                context.insert("settle_shift".into(), format!("{settle_shift:.6}"));
                diagnostics.push(metadata::Diagnostic {
                    severity: metadata::Severity::Info,
                    code: "PROP_GRID_SNAP_OFFSET".into(),
                    message: "recorded the exact correction applied to this prop".into(),
                    context,
                });
            }
        } else if individual[index].intersects && !individual[index].resolved {
            unresolved_props += 1;
            if unresolved_props <= MAX_UNRESOLVED_PROP_DIAGNOSTICS {
                let mut context = BTreeMap::new();
                context.insert("source_ordinal".into(), item.source_ordinal.to_string());
                context.insert("source_model".into(), item.prop.model.clone());
                context.insert(
                    "original_translation".into(),
                    format_vec3(conversion.transform.to_block_space(item.prop.origin)),
                );
                diagnostics.push(metadata::Diagnostic { severity: metadata::Severity::Warning, code: "PROP_GRID_SNAP_UNRESOLVED".into(), message: "prop intersects converted map geometry but no sub-block correction within one block cleared it".into(), context });
            }
        }
        let root_cell = crate::output::bake::anchor(&conversion.grid, transformed_bounds, &taken)
            .with_context(|| {
            format!(
                "{}: no free root cell for prop {}",
                crate::output::limits::ErrorCode::NoFreePropRoot.as_str(),
                item.source_ordinal
            )
        })?;
        taken.insert(root_cell);
        if pvs_index.is_some() {
            for x in section_coord(transformed_bounds.min.x)
                ..=section_coord(transformed_bounds.max.x - 1.0e-8)
            {
                for y in section_coord(transformed_bounds.min.y)
                    ..=section_coord(transformed_bounds.max.y - 1.0e-8)
                {
                    for z in section_coord(transformed_bounds.min.z)
                        ..=section_coord(transformed_bounds.max.z - 1.0e-8)
                    {
                        pvs_sections.insert([x, y, z]);
                    }
                }
            }
        }
        for axis in 0..3 {
            cell_max[axis] = cell_max[axis].max(root_cell[axis]);
            cell_min[axis] = cell_min[axis].min(root_cell[axis]);
        }
        let (model_content_id, slots, _) = &model_by_path[&item.prop.model];
        let origin = conversion.transform.to_block_space(item.prop.origin)
            + Vec3::new(0.0, settle_shift, 0.0)
            + offset;
        props.push(Prop {
            source_ordinal: item.source_ordinal,
            source_model: item.prop.model.clone(),
            model_content_id: model_content_id.clone(),
            root_cell,
            translation: [origin.x, origin.y, origin.z],
            rotation: crate::output::display::rotation(&item.prop, &conversion.transform),
            scale: item.prop.scale,
            material_ids: slots.clone(),
        });
    }
    if snapped_props > 0 {
        let mut context = BTreeMap::new();
        context.insert("props_snapped".into(), snapped_props.to_string());
        context.insert(
            "maximum_displacement_blocks".into(),
            format!("{maximum_snap_distance:.6}"),
        );
        diagnostics.push(metadata::Diagnostic {
            severity: metadata::Severity::Info,
            code: "PROP_GRID_SNAP_SUMMARY".into(),
            message: "props were nudged to clear grid overlap the source map did not have".into(),
            context,
        });
    }
    if settled_props > 0 {
        let mut context = BTreeMap::new();
        context.insert("props_settled".into(), settled_props.to_string());
        diagnostics.push(metadata::Diagnostic {
            severity: metadata::Severity::Info,
            code: "PROP_GRID_SETTLE_SUMMARY".into(),
            message: "props were moved vertically to stand on the converted floor".into(),
            context,
        });
    }
    if unresolved_props > MAX_UNRESOLVED_PROP_DIAGNOSTICS {
        let mut context = BTreeMap::new();
        context.insert("unresolved_props".into(), unresolved_props.to_string());
        context.insert(
            "reported_examples".into(),
            MAX_UNRESOLVED_PROP_DIAGNOSTICS.to_string(),
        );
        diagnostics.push(metadata::Diagnostic {
            severity: metadata::Severity::Warning,
            code: "PROP_GRID_SNAP_UNRESOLVED_SUMMARY".into(),
            message: "additional unresolved prop/grid intersections were omitted from diagnostics"
                .into(),
            context,
        });
    }
    let models = model_by_path
        .into_iter()
        .map(|(source_model, (_, materials, bytes))| ModelAsset {
            source_model,
            bytes,
            materials,
        })
        .collect();
    let pvs = if let Some((visibility, index)) = visibility.as_ref().zip(pvs_index.as_ref()) {
        let rows = visibility.rows.clone();
        let Some((root, nodes)) =
            crate::output::pvs::tree_from_visibility(visibility, &conversion.transform)
        else {
            return Err(anyhow::anyhow!("validated PVS has no exportable BSP tree"));
        };
        let sections: BTreeMap<[i32; 3], BTreeSet<u16>> = pvs_sections
            .iter()
            .map(|&section| (section, index.clusters_in_section(section)))
            .filter(|(_, clusters)| !clusters.is_empty())
            .collect();
        Some(
            crate::output::pvs::encode(rows, nodes, root, sections)
                .context("encoding validated PVS table")?,
        )
    } else {
        None
    };
    Ok(MapExport {
        map_id: portable_id(&map.name),
        source_name: map.name.clone(),
        cell_min,
        cell_max,
        anchor_cell,
        blocks,
        palette,
        faces,
        materials,
        textures,
        models,
        props,
        pvs,
        diagnostics: metadata::Diagnostics::new(diagnostics)?,
    })
}

/// A single face or prop use contributing texel demand to a material.
#[derive(Debug, Clone, Copy)]
enum Contrib {
    Face(usize),
    Prop,
}

/// Group contributions by the exact output resolution `analyze_resolution`
/// would assign them, so faces/props needing the same resolution share one
/// texture and nobody is forced onto another use's worst-case size.
fn bucket_by_output(
    original: [u32; 2],
    contributions: impl IntoIterator<Item = (Contrib, [f64; 2])>,
) -> BTreeMap<[u32; 2], Vec<Contrib>> {
    let mut buckets: BTreeMap<[u32; 2], Vec<Contrib>> = BTreeMap::new();
    for (contrib, blocks_spanned) in contributions {
        if let Ok(decision) = atlas::analyze_resolution(original, blocks_spanned) {
            buckets.entry(decision.output).or_default().push(contrib);
        }
    }
    buckets
}

fn assign_material(face_material_ids: &mut [u32], face_indices: &[usize], material_index: u32) {
    for &face_index in face_indices {
        face_material_ids[face_index] = material_index;
    }
}

fn extract_materials(
    map: &crate::bsp::Map,
    config: &crate::config::Config,
    prop_texture_spans: &BTreeMap<String, f64>,
    surfaces: &[crate::voxel::surface::VisibleFaceRecord],
) -> (
    Vec<metadata::MaterialReference>,
    Vec<TextureAsset>,
    Vec<u32>,
    BTreeMap<String, u32>,
) {
    let vfs = crate::source::vfs::Vfs::for_map(&map.path, &config.materials.game_dir_paths());
    let resolver = crate::source::vmt::Materials::new(&vfs, Some(&map.bsp.pack));
    let mut decoder = crate::source::vtf::Textures::new(&vfs, config.materials.texture_size);
    let mut assets_by_id = BTreeMap::new();
    let face_rates = per_face_rates(surfaces);
    let mut faces_by_material: Vec<Vec<usize>> = vec![Vec::new(); map.materials().len()];
    for (face_index, face) in surfaces.iter().enumerate() {
        if let Some(bucket) = faces_by_material.get_mut(face.source.material) {
            bucket.push(face_index);
        }
    }
    let mut face_material_ids: Vec<u32> = vec![0; surfaces.len()];
    let mut prop_bucket_ids: BTreeMap<String, u32> = BTreeMap::new();
    let mut materials: Vec<metadata::MaterialReference> = Vec::new();

    for (index, material) in map.materials().iter().enumerate() {
        let base_reference = metadata::MaterialReference {
            source_material: material.name.clone(),
            source_material_raw: (material.raw_name != material.name)
                .then(|| material.raw_name.clone()),
            render_class: metadata::RenderClass::Fallback,
            texture: None,
            surface_prop: None,
            reflectivity: material.reflectivity,
        };
        let face_indices = &faces_by_material[index];
        let Some(material_assets) = resolver.assets(&material.name, Some(&material.raw_name))
        else {
            materials.push(base_reference);
            assign_material(
                &mut face_material_ids,
                face_indices,
                (materials.len() - 1) as u32,
            );
            continue;
        };
        let mut reference_template = base_reference;
        reference_template.render_class = if material_assets.alpha_test {
            metadata::RenderClass::Cutout
        } else if material_assets.translucent {
            metadata::RenderClass::Translucent
        } else {
            metadata::RenderClass::Solid
        };
        reference_template.surface_prop = material_assets.surface_prop;
        let Some(header) = decoder.header(&material_assets.base_texture) else {
            materials.push(reference_template);
            assign_material(
                &mut face_material_ids,
                face_indices,
                (materials.len() - 1) as u32,
            );
            continue;
        };
        let mut contributions: Vec<(Contrib, [f64; 2])> = Vec::new();
        for &face_index in face_indices {
            let Some(rate) = face_rates[face_index] else {
                continue;
            };
            let blocks_spanned = std::array::from_fn(|axis| header.size[axis] as f64 / rate[axis]);
            contributions.push((Contrib::Face(face_index), blocks_spanned));
        }
        let prop_span = prop_texture_spans.get(&material.name).copied();
        if let Some(prop) = prop_span {
            contributions.push((Contrib::Prop, [prop; 2]));
        }
        let buckets = bucket_by_output(header.size, contributions);
        if buckets.is_empty() {
            materials.push(reference_template);
            assign_material(
                &mut face_material_ids,
                face_indices,
                (materials.len() - 1) as u32,
            );
            continue;
        }
        let mut assigned: BTreeSet<usize> = BTreeSet::new();
        let mut first_bucket_material_index: Option<u32> = None;
        for (output, contribs) in buckets {
            let Some(mut image) = decoder.resized(
                &material_assets.base_texture,
                output,
                material_assets.alpha_test,
            ) else {
                continue;
            };
            if !material_assets.alpha_test && !material_assets.translucent {
                crate::source::vtf::force_opaque(&mut image);
            }
            let Ok(bytes) = crate::source::vtf::to_png(&image) else {
                continue;
            };
            let content_id = bundle::content_id(&bytes);
            assets_by_id
                .entry(content_id.clone())
                .or_insert((bytes, image));
            let mut reference = reference_template.clone();
            reference.texture = Some(metadata::TextureReference {
                content_id,
                original_width: header.size[0],
                original_height: header.size[1],
                output_width: output[0],
                output_height: output[1],
            });
            materials.push(reference);
            let material_index = (materials.len() - 1) as u32;
            first_bucket_material_index.get_or_insert(material_index);
            for contrib in contribs {
                match contrib {
                    Contrib::Face(face_index) => {
                        face_material_ids[face_index] = material_index;
                        assigned.insert(face_index);
                    }
                    Contrib::Prop => {
                        prop_bucket_ids.insert(material.name.clone(), material_index);
                    }
                }
            }
        }
        let unassigned: Vec<usize> = face_indices
            .iter()
            .copied()
            .filter(|face_index| !assigned.contains(face_index))
            .collect();
        if !unassigned.is_empty() {
            let fallback_index = first_bucket_material_index.unwrap_or_else(|| {
                materials.push(reference_template.clone());
                (materials.len() - 1) as u32
            });
            assign_material(&mut face_material_ids, &unassigned, fallback_index);
        }
    }
    let existing: BTreeSet<String> = materials
        .iter()
        .map(|material| material.source_material.clone())
        .collect();
    for (name, blocks_spanned) in prop_texture_spans {
        if existing.contains(name.as_str()) {
            continue;
        }
        let mut reference = metadata::MaterialReference {
            source_material: name.clone(),
            source_material_raw: None,
            render_class: metadata::RenderClass::Fallback,
            texture: None,
            surface_prop: None,
            reflectivity: [0.0; 3],
        };
        let Some(material_assets) = resolver.assets(name, None) else {
            materials.push(reference);
            continue;
        };
        reference.render_class = if material_assets.alpha_test {
            metadata::RenderClass::Cutout
        } else if material_assets.translucent {
            metadata::RenderClass::Translucent
        } else {
            metadata::RenderClass::Solid
        };
        reference.surface_prop = material_assets.surface_prop;
        let Some(header) = decoder.header(&material_assets.base_texture) else {
            materials.push(reference);
            continue;
        };
        reference.reflectivity = header.reflectivity;
        let Ok(decision) = atlas::analyze_resolution(header.size, [*blocks_spanned; 2]) else {
            materials.push(reference);
            continue;
        };
        let Some(mut image) = decoder.resized(
            &material_assets.base_texture,
            decision.output,
            material_assets.alpha_test,
        ) else {
            materials.push(reference);
            continue;
        };
        if !material_assets.alpha_test && !material_assets.translucent {
            crate::source::vtf::force_opaque(&mut image);
        }
        let Ok(bytes) = crate::source::vtf::to_png(&image) else {
            materials.push(reference);
            continue;
        };
        let content_id = bundle::content_id(&bytes);
        assets_by_id
            .entry(content_id.clone())
            .or_insert((bytes, image));
        reference.texture = Some(metadata::TextureReference {
            content_id,
            original_width: decision.original[0],
            original_height: decision.original[1],
            output_width: decision.output[0],
            output_height: decision.output[1],
        });
        materials.push(reference);
        prop_bucket_ids.insert(name.clone(), (materials.len() - 1) as u32);
    }
    let textures = assets_by_id
        .into_iter()
        .map(|(content_id, (bytes, image))| TextureAsset {
            content_id,
            bytes,
            image,
        })
        .collect();
    (materials, textures, face_material_ids, prop_bucket_ids)
}

const GRID_COLLISION_EPSILON: f64 = 1.0e-6;
const MAX_UNRESOLVED_PROP_DIAGNOSTICS: usize = 20;

/// Hard ceiling on the sub-block snap correction, in blocks, whatever
/// `config.props.snap_max` asks for.
///
/// The error this corrects is voxelization rounding a brush face by up to a
/// block, so a correction can never legitimately need to be larger than that.
/// A prop still colliding past this distance is not a rounding artifact — it
/// is genuinely somewhere the conversion did not build room for it — and is
/// left alone rather than moved to an invented position.
const MAX_PROP_SNAP_BLOCKS: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct PropGridSnap {
    offset: Vec3,
    intersects: bool,
    resolved: bool,
}

/// Local-space occupancy of a prop's own mesh, used to tell a real collision
/// with the model's geometry apart from a collision with empty space inside
/// its bounding box.
///
/// A diagonal pipe, a railing or a ladder has a box that is mostly air, so
/// testing collision against the box rather than the mesh is what made those
/// register as clipping through walls they never actually touched. The mesh
/// is shared by every placement of the same model, so this is built once per
/// model and reused for every instance, however many thousand there are.
struct PropOccupancy {
    cells: HashSet<(i32, i32, i32)>,
}

/// Side length of one occupancy cell, in the mesh's own local units — blocks,
/// before a prop's rotation, translation or scale is applied.
///
/// Finer than a Minecraft block, since the point of sampling the mesh at all
/// is to resolve detail an axis-aligned block-sized test would smear away;
/// coarse enough that even a heavy model builds its occupancy set in a blink.
const PROP_OCCUPANCY_CELL: f64 = 0.25;

/// Occupancy cells checked around a query point, per axis, before declaring
/// it clear of the mesh. The one cell of slack absorbs the rasterizer's own
/// sampling resolution, so a thin sheet that a sample point lands just to one
/// side of is not read as a hole through the model.
const PROP_OCCUPANCY_MARGIN: i32 = 1;

/// Samples per triangle edge used to rasterize it into occupancy cells,
/// capped so one enormous flat face — a shipping-container wall, say — cannot
/// blow up build time. Above the cap the face is covered coarsely rather than
/// exactly, which costs nothing this test cares about: it is already an
/// approximation of where the mesh is, not a measurement of it.
const PROP_OCCUPANCY_SAMPLES_MAX: usize = 24;

impl PropOccupancy {
    fn build(mesh: &crate::output::mesh::Mesh) -> Self {
        let mut cells = HashSet::new();
        for triangle in mesh.indices.chunks_exact(3) {
            let corners = [
                mesh_vertex(mesh, triangle[0]),
                mesh_vertex(mesh, triangle[1]),
                mesh_vertex(mesh, triangle[2]),
            ];
            let edge_a = corners[1] - corners[0];
            let edge_b = corners[2] - corners[0];
            let samples = |edge: Vec3| -> usize {
                ((edge.length() / (PROP_OCCUPANCY_CELL * 0.5)).ceil() as usize + 1)
                    .clamp(1, PROP_OCCUPANCY_SAMPLES_MAX)
            };
            let (samples_u, samples_v) = (samples(edge_a), samples(edge_b));
            for ui in 0..samples_u {
                let u = fraction(ui, samples_u);
                for vi in 0..samples_v {
                    let v = fraction(vi, samples_v);
                    if u + v > 1.0 {
                        continue;
                    }
                    let point = corners[0] + edge_a * u + edge_b * v;
                    cells.insert(occupancy_cell(point));
                }
            }
        }
        PropOccupancy { cells }
    }

    /// Whether a point in the mesh's own local space lands on or near the
    /// mesh, within the rasterizer's own resolution.
    fn hit(&self, local: Vec3) -> bool {
        if self.cells.is_empty() {
            // Nothing rasterized at all — degenerate or unreadable geometry —
            // is safer treated as solid everywhere than as a hole nothing can
            // ever be found to collide with.
            return true;
        }
        let (cx, cy, cz) = occupancy_cell(local);
        for dx in -PROP_OCCUPANCY_MARGIN..=PROP_OCCUPANCY_MARGIN {
            for dy in -PROP_OCCUPANCY_MARGIN..=PROP_OCCUPANCY_MARGIN {
                for dz in -PROP_OCCUPANCY_MARGIN..=PROP_OCCUPANCY_MARGIN {
                    if self.cells.contains(&(cx + dx, cy + dy, cz + dz)) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// An occupancy that reports every point as part of the mesh. Used where
    /// a prop's real geometry either is not available or does not matter —
    /// tests of the grid/brush logic in isolation — so those callers get the
    /// old whole-box behaviour without duplicating it.
    #[cfg(test)]
    fn always() -> Self {
        // An empty cell set is exactly what `hit` treats as "solid
        // everywhere" — the same fallback a model that failed to rasterize
        // gets — so this reuses that rather than needing its own flag.
        PropOccupancy {
            cells: HashSet::new(),
        }
    }
}

fn fraction(index: usize, count: usize) -> f64 {
    if count <= 1 {
        0.0
    } else {
        index as f64 / (count - 1) as f64
    }
}

fn occupancy_cell(point: Vec3) -> (i32, i32, i32) {
    (
        (point.x / PROP_OCCUPANCY_CELL).floor() as i32,
        (point.y / PROP_OCCUPANCY_CELL).floor() as i32,
        (point.z / PROP_OCCUPANCY_CELL).floor() as i32,
    )
}

fn mesh_vertex(mesh: &crate::output::mesh::Mesh, index: u32) -> Vec3 {
    let position = mesh.vertices[index as usize].position;
    Vec3::new(position[0] as f64, position[1] as f64, position[2] as f64)
}

/// The exact placement of one prop instance, sufficient to map any world
/// block-space point back into the mesh's own local space.
///
/// `columns` are the world-block-space directions of the mesh's local axes —
/// an orthonormal set, since they come from a rotation — so their transpose is
/// their inverse and no matrix needs to be built or inverted to undo it.
#[derive(Clone, Copy)]
struct PropTransform {
    origin: Vec3,
    columns: [Vec3; 3],
    scale: f64,
}

impl PropTransform {
    /// Map a world block-space point into the mesh's own local space.
    #[allow(clippy::wrong_self_convention)]
    fn to_local(&self, world: Vec3) -> Vec3 {
        let relative = world - self.origin;
        Vec3::new(
            self.columns[0].dot(relative),
            self.columns[1].dot(relative),
            self.columns[2].dot(relative),
        ) / self.scale
    }
}

fn translated_bounds(bounds: Aabb, offset: Vec3) -> Aabb {
    Aabb::new(bounds.min + offset, bounds.max + offset)
}

fn with_axis(mut v: Vec3, axis: usize, value: f64) -> Vec3 {
    match axis {
        0 => v.x = value,
        1 => v.y = value,
        _ => v.z = value,
    }
    v
}

/// Samples taken across a solid grid cell's overlap with `bounds`, per axis,
/// when deciding whether the prop's own mesh actually reaches into it. Shared
/// with the "was this already a real brush" test below, since both are asking
/// the same kind of question of the same region.
const CELL_SAMPLES_PER_AXIS: usize = 3;

fn overlap_region(bounds: Aabb, cell: IVec3) -> (Vec3, Vec3) {
    let cell_min = Vec3::new(cell[0] as f64, cell[1] as f64, cell[2] as f64);
    let cell_max = cell_min + Vec3::splat(1.0);
    (bounds.min.max(cell_min), bounds.max.min(cell_max))
}

fn sample_region<F: FnMut(Vec3)>(min: Vec3, max: Vec3, n: usize, mut visit: F) {
    let extent = max - min;
    for xi in 0..n {
        for yi in 0..n {
            for zi in 0..n {
                let frac = |i: usize| (i as f64 + 0.5) / n as f64;
                let point = min
                    + Vec3::new(
                        extent.x * frac(xi),
                        extent.y * frac(yi),
                        extent.z * frac(zi),
                    );
                visit(point);
            }
        }
    }
}

/// Whether the prop's own geometry — not just its bounding box — actually
/// reaches into `cell`, judged by sampling the overlap region and testing
/// each point against the mesh's local occupancy. A single hit is enough:
/// the occupancy set is already a coarse approximation of a thin surface, so
/// requiring a volume fraction the way the brush-intent test does would miss
/// exactly the thin features this test exists to catch.
fn cell_touches_prop_geometry(
    bounds: Aabb,
    cell: IVec3,
    occupancy: &PropOccupancy,
    transform: &PropTransform,
) -> bool {
    let (min, max) = overlap_region(bounds, cell);
    let mut touches = false;
    sample_region(min, max, CELL_SAMPLES_PER_AXIS, |point| {
        touches = touches || occupancy.hit(transform.to_local(point));
    });
    touches
}

fn intersecting_cells(
    grid: &VoxelGrid,
    bounds: Aabb,
    occupancy: &PropOccupancy,
    transform: &PropTransform,
) -> Vec<IVec3> {
    if bounds.is_empty() {
        return Vec::new();
    }
    let min = [
        bounds.min.x.floor() as i32,
        bounds.min.y.floor() as i32,
        bounds.min.z.floor() as i32,
    ];
    let max = [
        (bounds.max.x - GRID_COLLISION_EPSILON).floor() as i32,
        (bounds.max.y - GRID_COLLISION_EPSILON).floor() as i32,
        (bounds.max.z - GRID_COLLISION_EPSILON).floor() as i32,
    ];
    let mut cells = Vec::new();
    for x in min[0]..=max[0] {
        for y in min[1]..=max[1] {
            for z in min[2]..=max[2] {
                let cell = [x, y, z];
                if grid.is_solid(cell)
                    && overlaps_cell(bounds, cell)
                    && cell_touches_prop_geometry(bounds, cell, occupancy, transform)
                {
                    cells.push(cell);
                }
            }
        }
    }
    cells
}

fn overlaps_cell(bounds: Aabb, cell: IVec3) -> bool {
    let min = Vec3::new(cell[0] as f64, cell[1] as f64, cell[2] as f64);
    let max = min + Vec3::splat(1.0);
    bounds.max.x > min.x + GRID_COLLISION_EPSILON
        && bounds.min.x < max.x - GRID_COLLISION_EPSILON
        && bounds.max.y > min.y + GRID_COLLISION_EPSILON
        && bounds.min.y < max.y - GRID_COLLISION_EPSILON
        && bounds.max.z > min.z + GRID_COLLISION_EPSILON
        && bounds.min.z < max.z - GRID_COLLISION_EPSILON
}

/// A cell's overlap counts as the mapper's intentional clip only once at
/// least this fraction of its sampled volume is real brush geometry. Below
/// it, the overlap is treated as a voxelization rounding artifact even if a
/// brush grazes a corner of it.
const INTENT_VOLUME_THRESHOLD: f64 = 0.2;

/// Whether the map's original brushes already occupy the space where `bounds`
/// overlaps `cell` — a mapper's intentional clip, not a rounding artifact of
/// voxelization. Estimated by sampling a dense grid across the overlap
/// region: a brush must actually cover a meaningful share of it, not just
/// graze a single point.
fn cell_overlap_is_intentional(
    solids: &[BlockSolid],
    solid_index: &BTreeMap<IVec3, Vec<u32>>,
    bounds: Aabb,
    cell: IVec3,
) -> bool {
    let Some(candidates) = solid_index.get(&cell) else {
        return false;
    };
    let (overlap_min, overlap_max) = overlap_region(bounds, cell);
    let mut covered = 0usize;
    let mut total = 0usize;
    sample_region(overlap_min, overlap_max, CELL_SAMPLES_PER_AXIS, |point| {
        total += 1;
        if candidates
            .iter()
            .any(|&index| solids[index as usize].contains(point))
        {
            covered += 1;
        }
    });
    total > 0 && covered as f64 / total as f64 >= INTENT_VOLUME_THRESHOLD
}

/// Cells `bounds` collides with in the grid, minus any whose overlap the
/// original map's own brushes already account for and any the prop's own
/// mesh does not actually reach into. Only what remains is a voxelization
/// artifact worth correcting.
fn unintentional_cells(
    grid: &VoxelGrid,
    solids: &[BlockSolid],
    solid_index: &BTreeMap<IVec3, Vec<u32>>,
    bounds: Aabb,
    occupancy: &PropOccupancy,
    transform: &PropTransform,
) -> Vec<IVec3> {
    intersecting_cells(grid, bounds, occupancy, transform)
        .into_iter()
        .filter(|&cell| !cell_overlap_is_intentional(solids, solid_index, bounds, cell))
        .collect()
}

/// The sub-block correction that undoes voxelization rounding: a single move
/// along the dominant collision normal, sized from the real brush surface
/// rather than searched for across a diagonal candidate grid.
///
/// A wall's true plane is looked up from the same `solids`/`solid_index` used
/// to tell intentional overlap apart from an artifact, in the colliding cells
/// and their immediate neighbour along the normal (the true wall can be a
/// cell further over than the one that read solid, since that offset is
/// exactly the rounding being corrected). Among the planes actually facing
/// the prop, the one demanding the largest correction wins, so the move
/// clears the deepest penetration and not just the shallowest. When no plane
/// is found at all — the cell reads solid with nothing behind it in
/// `solids`, so there is no better answer available — a full block is used,
/// matching what the old whole-block snap did in the same situation.
/// `(offset, resolved)`: the move that clears the collision along the
/// dominant normal, and whether it is trustworthy enough to apply.
///
/// A plane found nearby gives an exact answer, applied whenever it fits under
/// the cap; past the cap it is left alone rather than clipped to an
/// admittedly-wrong distance. With no plane at all — the cell reads solid
/// with nothing in `solids` behind it — a full block is used and treated as
/// resolved, matching what the old whole-block snap did in the same
/// situation, where nothing better than "one grid cell" was ever knowable.
fn sub_block_correction(
    solids: &[BlockSolid],
    solid_index: &BTreeMap<IVec3, Vec<u32>>,
    collisions: &[IVec3],
    normal: Vec3,
    bounds: Aabb,
    max_shift: f64,
) -> (Vec3, bool) {
    if normal == Vec3::ZERO || max_shift <= 0.0 || collisions.is_empty() {
        return (Vec3::ZERO, false);
    }
    let cap = max_shift.min(MAX_PROP_SNAP_BLOCKS);
    let axis = normal.major_axis();
    let direction = normal.axis(axis).signum();
    // The trailing face in the push direction: the part of the prop that is
    // last to clear the solid as the whole box moves, so its distance to the
    // true surface is what decides how far the move must be.
    let face = if direction >= 0.0 {
        bounds.min.axis(axis)
    } else {
        bounds.max.axis(axis)
    };
    let point = with_axis(bounds.center(), axis, face);

    // `found_plane` tracks whether any real geometry was seen at all, so a
    // point already outside every plane found nearby — genuinely resting on
    // the true surface, with the grid cell only reading solid because of its
    // own conservative rounding — is told apart from a point with no real
    // geometry to measure against in the first place. The former needs no
    // move; only the latter falls back to guessing a full block.
    let mut found_plane = false;
    let mut needed = 0.0f64;
    for &cell in collisions {
        // Looked up at the colliding cell itself: a solid's bounding box is
        // indexed at every cell it spans, so whatever voxelized this cell
        // solid is already listed here. A tight alignment threshold keeps
        // an unrelated plane that happens to graze the same broad-phase cell
        // from being mistaken for the wall actually responsible.
        let Some(candidates) = solid_index.get(&cell) else {
            continue;
        };
        for &index in candidates {
            for plane in &solids[index as usize].planes {
                if plane.normal.dot(normal) <= 0.9 {
                    continue;
                }
                found_plane = true;
                let distance = plane.distance_to(point);
                if distance < 0.0 {
                    needed = needed.max(-distance);
                }
            }
        }
    }
    if !found_plane {
        return (with_axis(Vec3::ZERO, axis, cap * direction), true);
    }
    if needed <= cap {
        (with_axis(Vec3::ZERO, axis, needed * direction), true)
    } else {
        (Vec3::ZERO, false)
    }
}

/// The combined push direction of a set of colliding cells, from the visible
/// surface normals of the faces the conversion drew there. Used only to
/// choose which axis and side the sub-block correction moves along.
fn combined_collision_normal(
    collisions: &[IVec3],
    visible_normals: &BTreeMap<IVec3, Vec<FaceDirection>>,
) -> Vec3 {
    collisions.iter().fold(Vec3::ZERO, |sum, cell| {
        sum + visible_normals
            .get(cell)
            .into_iter()
            .flatten()
            .fold(Vec3::ZERO, |acc, direction| acc + direction.normal())
    })
}

#[allow(clippy::too_many_arguments)]
fn snap_prop_bounds(
    grid: &VoxelGrid,
    solids: &[BlockSolid],
    solid_index: &BTreeMap<IVec3, Vec<u32>>,
    visible_normals: &BTreeMap<IVec3, Vec<FaceDirection>>,
    bounds: Aabb,
    occupancy: &PropOccupancy,
    transform: &PropTransform,
    max_shift: f64,
) -> PropGridSnap {
    let collisions = unintentional_cells(grid, solids, solid_index, bounds, occupancy, transform);
    if collisions.is_empty() {
        return PropGridSnap {
            offset: Vec3::ZERO,
            intersects: false,
            resolved: true,
        };
    }
    let normal = combined_collision_normal(&collisions, visible_normals);
    let (offset, resolved) =
        sub_block_correction(solids, solid_index, &collisions, normal, bounds, max_shift);
    if resolved {
        PropGridSnap {
            offset,
            intersects: true,
            resolved: true,
        }
    } else {
        PropGridSnap {
            offset: Vec3::ZERO,
            intersects: true,
            resolved: false,
        }
    }
}

/// Union-find over prop indices, used to group props whose bounds touch or
/// overlap into one assembly that a correction moves as a unit.
///
/// Always attaching the larger index under the smaller keeps the resulting
/// partition — which prop ends up representing which assembly — a function of
/// the touch graph alone, not of the order pairs happened to be unioned in,
/// which is what keeps assembly grouping (and therefore the campaign's
/// exported bytes) reproducible.
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra.max(rb)] = ra.min(rb);
        }
    }
}

/// Side of one bucket in the spatial hash used to find candidate touching
/// pairs of props without testing every pair on the map. Wider than most
/// individual props so a pipe run's neighbouring segments always share at
/// least one bucket, narrow enough that a bucket in a dense room does not
/// collect the whole room.
const ASSEMBLY_BUCKET_BLOCKS: f64 = 4.0;

/// How far apart two props' bounds may be and still count as touching for
/// assembly grouping. Zero would miss two pipe segments modelled with a hair
/// of a gap between them, which is common enough in Source content that
/// treating them as unrelated would defeat the point of grouping at all.
const ASSEMBLY_TOUCH_EPSILON: f64 = 0.05;

fn bucket_of(value: f64) -> i32 {
    (value / ASSEMBLY_BUCKET_BLOCKS).floor() as i32
}

fn bounds_touch(a: Aabb, b: Aabb) -> bool {
    for axis in 0..3 {
        if a.max.axis(axis) + ASSEMBLY_TOUCH_EPSILON < b.min.axis(axis) {
            return false;
        }
        if b.max.axis(axis) + ASSEMBLY_TOUCH_EPSILON < a.min.axis(axis) {
            return false;
        }
    }
    true
}

/// Groups prop indices into assemblies by touching or overlapping bounds,
/// returning each prop's assembly representative (the smallest index in its
/// group). A pipe run or a railing is many independent static props of the
/// same model; grouping them here is what lets one shared correction move
/// the whole run rather than splitting it at whichever piece happened to clip.
fn build_assemblies(bounds: &[Aabb]) -> Vec<usize> {
    let mut buckets: BTreeMap<(i32, i32, i32), Vec<usize>> = BTreeMap::new();
    for (index, bound) in bounds.iter().enumerate() {
        if bound.is_empty() {
            continue;
        }
        let min = bound.min - Vec3::splat(ASSEMBLY_TOUCH_EPSILON);
        let max = bound.max + Vec3::splat(ASSEMBLY_TOUCH_EPSILON);
        for x in bucket_of(min.x)..=bucket_of(max.x) {
            for y in bucket_of(min.y)..=bucket_of(max.y) {
                for z in bucket_of(min.z)..=bucket_of(max.z) {
                    buckets.entry((x, y, z)).or_default().push(index);
                }
            }
        }
    }
    let mut union_find = UnionFind::new(bounds.len());
    for members in buckets.values() {
        for a in 0..members.len() {
            for b in (a + 1)..members.len() {
                let (i, j) = (members[a], members[b]);
                if bounds_touch(bounds[i], bounds[j]) {
                    union_find.union(i, j);
                }
            }
        }
    }
    (0..bounds.len()).map(|i| union_find.find(i)).collect()
}

/// For each prop, the assembly-wide correction: whichever member of its
/// touching/overlapping group needed the largest individual correction, or
/// zero if none of them needed one at all. Applying the same vector to every
/// member — rather than each keeping its own, independently-found offset —
/// is what keeps a pipe run or a railing moving as one piece instead of
/// splitting at whichever segment happened to clip. Ties break on the lowest
/// source ordinal so the choice does not depend on iteration order.
fn assign_assembly_offsets(
    assembly_of: &[usize],
    individual: &[Vec3],
    ordinals: &[u64],
) -> Vec<Vec3> {
    let mut winners: BTreeMap<usize, (f64, u64, Vec3)> = BTreeMap::new();
    for (index, &offset) in individual.iter().enumerate() {
        if offset == Vec3::ZERO {
            continue;
        }
        let root = assembly_of[index];
        let magnitude = offset.length();
        let ordinal = ordinals[index];
        let entry = winners.entry(root).or_insert((0.0, u64::MAX, Vec3::ZERO));
        if magnitude > entry.0 || (magnitude == entry.0 && ordinal < entry.1) {
            *entry = (magnitude, ordinal, offset);
        }
    }
    (0..individual.len())
        .map(|index| {
            winners
                .get(&assembly_of[index])
                .map(|&(_, _, offset)| offset)
                .unwrap_or(Vec3::ZERO)
        })
        .collect()
}

/// Walks props bottom-up and lets one resting on another's finalized top
/// inherit that prop's total shift — settle plus snap — exactly, in place,
/// rather than each having independently settled and snapped by whatever the
/// two happen to disagree by.
///
/// `order` must visit supports before their dependents; callers sort by
/// original (pre-shift) base height with a stable tiebreak so the result does
/// not depend on the props' extraction order. A spatial bucket over each
/// prop's finalized footprint keeps this near-linear instead of testing every
/// pair, which matters on an 8000-prop map.
fn apply_stacking_inheritance(
    order: &[usize],
    unsettled_bounds: &[Aabb],
    settle_shift: &mut [f64],
    offset: &mut [Vec3],
    ordinals: &[u64],
) {
    let place = |unsettled: Aabb, shift: f64, offset: Vec3| -> Aabb {
        translated_bounds(unsettled, Vec3::new(offset.x, shift + offset.y, offset.z))
    };
    let mut finalized_bounds: Vec<Option<Aabb>> = vec![None; unsettled_bounds.len()];
    let mut support_buckets: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
    for &index in order {
        let mut current_bounds = place(unsettled_bounds[index], settle_shift[index], offset[index]);
        let mut best: Option<(f64, u64, usize)> = None;
        for (bx, bz) in bucket_range_2d(current_bounds) {
            let Some(candidates) = support_buckets.get(&(bx, bz)) else {
                continue;
            };
            for &candidate in candidates {
                let Some(support_bounds) = finalized_bounds[candidate] else {
                    continue;
                };
                let Some(area) = settle::resting_on(current_bounds, support_bounds) else {
                    continue;
                };
                let ordinal = ordinals[candidate];
                let better = match best {
                    None => true,
                    Some((best_area, best_ordinal, _)) => {
                        area > best_area || (area == best_area && ordinal < best_ordinal)
                    }
                };
                if better {
                    best = Some((area, ordinal, candidate));
                }
            }
        }
        if let Some((_, _, support)) = best {
            offset[index] = offset[support];
            settle_shift[index] = settle_shift[support];
            current_bounds = place(unsettled_bounds[index], settle_shift[index], offset[index]);
        }
        finalized_bounds[index] = Some(current_bounds);
        for (bx, bz) in bucket_range_2d(current_bounds) {
            support_buckets.entry((bx, bz)).or_default().push(index);
        }
    }
}

/// Side of one bucket in the spatial hash used to find candidate stacking
/// supports without testing every already-placed prop. Only the horizontal
/// axes matter here — the pass that uses this always walks props bottom-up,
/// so "already placed" already means "below or level with" in Y.
const STACK_BUCKET_BLOCKS: f64 = 2.0;

/// The horizontal buckets a prop's footprint spans, for indexing or querying
/// the stacking-support spatial hash.
fn bucket_range_2d(bounds: Aabb) -> Vec<(i32, i32)> {
    let bucket = |v: f64| (v / STACK_BUCKET_BLOCKS).floor() as i32;
    let mut out = Vec::new();
    for x in bucket(bounds.min.x)..=bucket(bounds.max.x) {
        for z in bucket(bounds.min.z)..=bucket(bounds.max.z) {
            out.push((x, z));
        }
    }
    out
}

fn format_vec3(value: Vec3) -> String {
    format!("{:.6},{:.6},{:.6}", value.x, value.y, value.z)
}

/// Per-face texel rates in Minecraft block space, measured from the exact UV
/// transform emitted for each visible face. Each face keeps its own rate so
/// it can be bucketed onto the smallest texture that still meets the
/// texels-per-block target, instead of every face sharing one material's
/// worst-case (most-stretched) rate.
fn per_face_rates(surfaces: &[crate::voxel::surface::VisibleFaceRecord]) -> Vec<Option<[f64; 2]>> {
    surfaces
        .iter()
        .map(|face| {
            let axes: [usize; 2] = match face.patch.direction {
                FaceDirection::Down | FaceDirection::Up => [0, 2],
                FaceDirection::North | FaceDirection::South => [0, 1],
                FaceDirection::West | FaceDirection::East => [1, 2],
            };
            let rate = std::array::from_fn(|texture_axis| {
                let projection = if texture_axis == 0 {
                    face.source.uv.u
                } else {
                    face.source.uv.v
                };
                axes.into_iter()
                    .map(|axis| projection[axis] * projection[axis])
                    .sum::<f64>()
                    .sqrt()
            });
            rate.iter()
                .all(|value: &f64| value.is_finite() && *value > 0.0)
                .then_some(rate)
        })
        .collect()
}

pub fn portable_id(name: &str) -> String {
    let mut id: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if id.is_empty() {
        id.push_str("map");
    }
    id
}

/// Placement identity deliberately excludes the selected root cell.
pub fn stable_prop_id(map_id: &str, source_ordinal: u64, source_model: &str) -> Result<[u8; 32]> {
    bundle::validate_id(map_id, "map")?;
    ensure!(
        !source_model.is_empty(),
        "source model path must not be empty"
    );
    let mut hash = Sha256::new();
    hash.update(b"src2mc-prop-placement-v1\0");
    hash.update((map_id.len() as u32).to_le_bytes());
    hash.update(map_id.as_bytes());
    hash.update(source_ordinal.to_le_bytes());
    hash.update((source_model.len() as u32).to_le_bytes());
    hash.update(source_model.as_bytes());
    Ok(hash.finalize().into())
}

pub fn write_campaign(
    out: &Path,
    campaign_id: &str,
    mut maps: Vec<MapExport>,
) -> Result<WrittenCampaign> {
    bundle::validate_id(campaign_id, "campaign")?;
    maps.sort_by(|a, b| a.map_id.cmp(&b.map_id));
    ensure!(
        maps.windows(2).all(|p| p[0].map_id != p[1].map_id),
        "duplicate map ID"
    );
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut archive = bundle::Bundle::new();
    let mut campaign_maps = Vec::new();
    let mut schematic_outputs = Vec::new();

    let mut atlas_assets = BTreeMap::new();
    for map in &mut maps {
        for texture in std::mem::take(&mut map.textures) {
            ensure!(
                bundle::content_id(&texture.bytes) == texture.content_id,
                "logical texture content ID changed"
            );
            if let Some(old) = atlas_assets.insert(texture.content_id.clone(), texture.image) {
                ensure!(
                    old.as_raw() == atlas_assets[&texture.content_id].as_raw(),
                    "shared logical texture differs"
                );
            }
        }
    }
    let atlas_path = (!atlas_assets.is_empty()).then_some("atlas.json".to_string());
    if atlas_path.is_some() {
        let logical = atlas_assets
            .iter()
            .map(|(content_id, image)| atlas::LogicalTexture {
                content_id: content_id.clone(),
                width: image.width(),
                height: image.height(),
            })
            .collect::<Vec<_>>();
        let layout = atlas::pack(&logical)?;
        let images = atlas_assets
            .into_iter()
            .map(|(content_id, image)| atlas::ImageAsset { content_id, image })
            .collect::<Vec<_>>();
        let pages = atlas::build_pages(&layout, &images)?;
        let mut page_meta = Vec::with_capacity(pages.len());
        for (page, images) in pages.into_iter().enumerate() {
            let mut mips = Vec::with_capacity(images.mips.len());
            for (level, image) in images.mips.into_iter().enumerate() {
                let width = image.width();
                let height = image.height();
                let content_id =
                    archive.add_content("atlas", "png", crate::source::vtf::to_png(&image)?)?;
                mips.push(metadata::AtlasMip {
                    level: level as u8,
                    content_id,
                    width,
                    height,
                });
            }
            page_meta.push(metadata::AtlasPage {
                page: page as u32,
                mips,
            });
        }
        let textures = logical
            .into_iter()
            .map(|texture| metadata::AtlasTexture {
                regions: layout
                    .regions
                    .iter()
                    .filter(|r| r.content_id == texture.content_id)
                    .map(|r| metadata::AtlasRegion {
                        source: [r.source.x, r.source.y, r.source.width, r.source.height],
                        page: r.page,
                        allocation: [
                            r.allocation.x,
                            r.allocation.y,
                            r.allocation.width,
                            r.allocation.height,
                        ],
                    })
                    .collect(),
                content_id: texture.content_id,
                width: texture.width,
                height: texture.height,
            })
            .collect();
        archive.add(
            "atlas.json",
            metadata::AtlasMetadata {
                format: "src2mc-atlas",
                version: 1,
                page_size: atlas::PAGE_SIZE,
                max_mip_level: atlas::MAX_MIP_LEVEL,
                gutter: atlas::GUTTER,
                pages: page_meta,
                textures,
            }
            .encode()?,
        )?;
    }

    for map in maps {
        bundle::validate_id(&map.map_id, "map")?;
        let prefix = format!("maps/{}", map.map_id);
        archive.add(
            format!("{prefix}/surfaces.s2faces"),
            surface::encode(map.faces, surface::Limits::default())?,
        )?;
        let mut model_refs = Vec::new();
        for model in map.models {
            let content_id = archive.add_content("meshes", "s2mesh", model.bytes)?;
            ensure!(
                !model.source_model.is_empty(),
                "source model path must not be empty"
            );
            model_refs.push(metadata::ModelReference {
                content_id,
                source_model: model.source_model,
                materials: model.materials,
            });
        }
        model_refs.sort();
        model_refs.dedup();
        let model_ids: BTreeMap<_, _> = model_refs
            .iter()
            .enumerate()
            .map(|(index, model)| {
                (
                    (
                        model.content_id.clone(),
                        model.source_model.clone(),
                        model.materials.clone(),
                    ),
                    index as u32,
                )
            })
            .collect();

        let mut roots = Vec::new();
        let mut placements = Vec::new();
        let mut occupied: BTreeSet<IVec3> = map.blocks.iter().map(|(p, _)| *p).collect();
        ensure!(
            !occupied.contains(&map.anchor_cell),
            "map anchor cell {:?} is occupied",
            map.anchor_cell
        );
        let anchor_id = map
            .palette
            .names()
            .iter()
            .position(|v| v == "src2mc:map_anchor")
            .context("palette lacks src2mc:map_anchor")? as u16;
        let root_id = map
            .palette
            .names()
            .iter()
            .position(|v| v == "src2mc:prop_root")
            .context("palette lacks src2mc:prop_root")? as u16;
        let mut blocks = map.blocks;
        blocks.push((map.anchor_cell, anchor_id));
        roots.push(schem::ModBlockEntity::anchor(
            sub(map.anchor_cell, map.cell_min),
            campaign_id,
            &map.map_id,
            map.anchor_cell,
        )?);
        for prop in map.props {
            ensure!(
                occupied.insert(prop.root_cell),
                "prop root cell {:?} is occupied",
                prop.root_cell
            );
            let stable_id = stable_prop_id(&map.map_id, prop.source_ordinal, &prop.source_model)?;
            let model = *model_ids
                .get(&(
                    prop.model_content_id.clone(),
                    prop.source_model.clone(),
                    prop.material_ids.clone(),
                ))
                .with_context(|| format!("prop {stable_id:x?} references a missing model"))?;
            blocks.push((prop.root_cell, root_id));
            roots.push(schem::ModBlockEntity::prop_root(
                sub(prop.root_cell, map.cell_min),
                campaign_id,
                &map.map_id,
                stable_id,
                &prop.model_content_id,
                prop.root_cell,
                prop.translation,
                prop.rotation,
                prop.scale,
                prop.material_ids.clone(),
                &prop.source_model,
            )?);
            placements.push(placement::Placement {
                stable_id,
                model,
                root_cell: prop.root_cell,
                translation: prop.translation,
                rotation: prop.rotation,
                scale: prop.scale,
            });
        }
        archive.add(
            format!("{prefix}/props.s2props"),
            placement::encode(placements, model_refs.len() as u32)?,
        )?;
        let has_pvs = map.pvs.is_some();
        if let Some(pvs) = map.pvs {
            archive.add(format!("{prefix}/pvs.s2pvs"), pvs)?;
        }
        archive.add(
            format!("{prefix}/diagnostics.json"),
            map.diagnostics.encode()?,
        )?;
        let metadata_path = format!("maps/{}.json", map.map_id);
        let meta = metadata::MapMetadata {
            format: "src2mc-map",
            version: 1,
            map_id: map.map_id.clone(),
            source_name: map.source_name,
            units_per_block: UNITS_PER_BLOCK,
            cell_min: map.cell_min,
            cell_max: map.cell_max,
            anchor_cell: map.anchor_cell,
            surfaces: format!("{prefix}/surfaces.s2faces"),
            materials: map.materials,
            models: model_refs,
            props: format!("{prefix}/props.s2props"),
            pvs: has_pvs.then(|| format!("{prefix}/pvs.s2pvs")),
            diagnostics: format!("{prefix}/diagnostics.json"),
        };
        archive.add(&metadata_path, meta.encode()?)?;
        campaign_maps.push(metadata::CampaignMap {
            map_id: map.map_id.clone(),
            metadata: metadata_path,
        });
        let schematic_path = out.join(format!("{}.schem", map.map_id));
        schem::write_mod(
            &schematic_path,
            &blocks,
            &roots,
            &map.palette,
            map.cell_min,
            map.cell_max,
            &map.map_id,
        )?;
        schematic_outputs.push(schematic_path);
    }
    archive.add(
        metadata::CAMPAIGN_PATH,
        metadata::Campaign::with_atlas(campaign_id, atlas_path, campaign_maps)?.encode()?,
    )?;
    let bundle_path = out.join(format!("{campaign_id}.src2mc"));
    let manifest = archive.write(&bundle_path, campaign_id)?;
    Ok(WrittenCampaign {
        bundle: bundle_path,
        schematics: schematic_outputs,
        manifest,
    })
}

fn sub(a: IVec3, b: IVec3) -> IVec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// The 16-block section coordinate of a map-local block-space position,
/// matching the runtime's `floor(coordinate / 16.0)` section mapping.
fn section_coord(value: f64) -> i32 {
    (value / 16.0).floor() as i32
}

/// Adds every surface face's 16-block section to the PVS section set, the
/// same grid `surface::encode` buckets faces into, so sections with visible
/// geometry but no props still get a cluster-visibility entry.
fn insert_surface_sections(
    surfaces: &[crate::voxel::surface::VisibleFaceRecord],
    sections: &mut BTreeSet<[i32; 3]>,
) {
    for face in surfaces {
        sections.insert(crate::output::surface::section_of(face.cell));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bsp::texcoord::BlockTexCoord;
    use crate::output::mesh::{Mesh, Submesh, Vertex};
    use crate::voxel::surface::{FaceDirection, FacePatch, SourceProvenance};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "src2mc-mod-export-{label}-{}-{sequence}",
            std::process::id()
        ))
    }

    fn triangle_mesh() -> Vec<u8> {
        crate::output::mesh::encode(&Mesh {
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [1.0, 1.0, 0.0],
            vertices: vec![
                Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    position: [1.0, 0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    position: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 1.0],
                },
            ],
            indices: vec![0, 1, 2],
            submeshes: vec![Submesh {
                first_index: 0,
                index_count: 3,
                material_slot: 0,
            }],
        })
        .unwrap()
    }

    fn fixture_map(map_id: &str, source_model: &str, mesh_bytes: Vec<u8>) -> MapExport {
        let mut palette = Palette::new();
        let surface_block = palette.intern("src2mc:surface");
        palette.intern("src2mc:map_anchor");
        palette.intern("src2mc:prop_root");
        let material = metadata::MaterialReference {
            source_material: "fixture/grid".into(),
            source_material_raw: None,
            render_class: metadata::RenderClass::Fallback,
            texture: None,
            surface_prop: None,
            reflectivity: [0.25, 0.5, 0.75],
        };
        let content_id = bundle::content_id(&mesh_bytes);
        MapExport {
            map_id: map_id.into(),
            source_name: format!("{map_id}.bsp"),
            cell_min: [0, -1, 0],
            cell_max: [1, 0, 0],
            anchor_cell: [0, -1, 0],
            blocks: vec![([0, 0, 0], surface_block)],
            palette,
            faces: vec![surface::EncodedFace {
                cell: [0, 0, 0],
                patch: FacePatch {
                    direction: FaceDirection::Up,
                    min: [0, 2, 0],
                    max: [1, 2, 1],
                },
                material: surface::MaterialId(0),
                uv: BlockTexCoord {
                    u: [1.0, 0.0, 0.0, 0.0],
                    v: [0.0, 0.0, 1.0, 0.0],
                },
                provenance: SourceProvenance::Brush { brush: 0, side: 0 },
            }],
            materials: vec![material],
            textures: Vec::new(),
            models: vec![ModelAsset {
                source_model: source_model.into(),
                bytes: mesh_bytes,
                materials: vec![0],
            }],
            pvs: None,
            props: vec![Prop {
                source_ordinal: 0,
                source_model: source_model.into(),
                model_content_id: content_id,
                root_cell: [1, 0, 0],
                translation: [0.5, 0.0, 0.5],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: 1.0,
                material_ids: vec![0],
            }],
            diagnostics: metadata::Diagnostics::new(Vec::new()).unwrap(),
        }
    }

    fn face_at(
        material: usize,
        u: [f64; 4],
        v: [f64; 4],
    ) -> crate::voxel::surface::VisibleFaceRecord {
        crate::voxel::surface::VisibleFaceRecord {
            cell: [0, 0, 0],
            shape: crate::voxel::shapes::Shape::Full,
            patch: FacePatch {
                direction: FaceDirection::Up,
                min: [0, 0, 0],
                max: [16, 16, 16],
            },
            source: crate::voxel::surface::FaceSource {
                provenance: SourceProvenance::Brush { brush: 0, side: 0 },
                material,
                uv: BlockTexCoord { u, v },
            },
        }
    }

    fn face_in_cell(cell: IVec3) -> crate::voxel::surface::VisibleFaceRecord {
        crate::voxel::surface::VisibleFaceRecord {
            cell,
            shape: crate::voxel::shapes::Shape::Full,
            patch: FacePatch {
                direction: FaceDirection::Up,
                min: [0, 0, 0],
                max: [16, 16, 16],
            },
            source: crate::voxel::surface::FaceSource {
                provenance: SourceProvenance::Brush { brush: 0, side: 0 },
                material: 0,
                uv: BlockTexCoord {
                    u: [1.0, 0.0, 0.0, 0.0],
                    v: [0.0, 0.0, 1.0, 0.0],
                },
            },
        }
    }

    #[test]
    fn surface_only_sections_get_pvs_entries_without_any_prop() {
        // A wall-enclosed room with no props: one face near the origin, one
        // two sections away, and a third re-visiting the first section.
        let surfaces = vec![
            face_in_cell([0, 0, 0]),
            face_in_cell([40, 0, 0]),
            face_in_cell([1, 1, 1]),
        ];
        let mut sections = BTreeSet::new();
        insert_surface_sections(&surfaces, &mut sections);
        assert_eq!(
            sections,
            BTreeSet::from([[0, 0, 0], [2, 0, 0]]),
            "sections spanned by surface-only geometry must be present even without props"
        );
    }

    #[test]
    fn bucket_by_output_groups_contributions_sharing_a_resolution() {
        let buckets = bucket_by_output(
            [1024, 1024],
            [
                (Contrib::Face(0), [4.0, 4.0]),
                (Contrib::Face(1), [4.0, 4.0]),
                (Contrib::Face(2), [1.0, 1.0]),
                (Contrib::Prop, [64.0, 64.0]),
            ],
        );
        assert_eq!(buckets.len(), 3);
        assert_eq!(buckets[&[64, 64]].len(), 2);
        assert_eq!(buckets[&[16, 16]].len(), 1);
        assert_eq!(buckets[&[1024, 1024]].len(), 1);
    }

    #[test]
    fn bucket_by_output_clamps_to_original_and_ignores_bad_contributions() {
        let buckets = bucket_by_output(
            [32, 32],
            [
                (Contrib::Face(0), [64.0, 64.0]),
                (Contrib::Face(1), [f64::NAN, 1.0]),
            ],
        );
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[&[32, 32]].len(), 1);
    }

    #[test]
    fn bucket_by_output_of_no_contributions_is_empty() {
        assert!(bucket_by_output([32, 32], Vec::new()).is_empty());
    }

    #[test]
    fn per_face_rates_is_positional_and_flags_degenerate_faces() {
        let surfaces = vec![
            face_at(0, [1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]),
            face_at(0, [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]),
        ];
        let rates = per_face_rates(&surfaces);
        assert_eq!(rates.len(), 2);
        assert_eq!(rates[0], Some([1.0, 1.0]));
        assert_eq!(rates[1], None);
    }

    #[test]
    fn stable_identity_ignores_root_selection() {
        assert_eq!(
            stable_prop_id("map", 7, "models/a.mdl").unwrap(),
            stable_prop_id("map", 7, "models/a.mdl").unwrap()
        );
        assert_ne!(
            stable_prop_id("map", 7, "models/a.mdl").unwrap(),
            stable_prop_id("map", 8, "models/a.mdl").unwrap()
        );
    }

    fn block_solid(min: Vec3, max: Vec3) -> BlockSolid {
        let planes = crate::geom::box_planes(min, max).to_vec();
        let side_of_plane = (0..planes.len()).collect();
        BlockSolid {
            planes,
            bounds: Aabb::new(min, max),
            side_of_plane,
        }
    }

    fn solid_grid(cell: IVec3) -> VoxelGrid {
        let mut grid = VoxelGrid::new();
        grid.set(cell, 1);
        grid
    }

    /// A transform placed exactly on the block grid's own origin and axes, so
    /// world block-space points and the mesh's local space coincide. What the
    /// grid/brush collision tests want, since they are not exercising a real
    /// prop's rotation or scale.
    fn identity_transform() -> PropTransform {
        PropTransform {
            origin: Vec3::ZERO,
            columns: [
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            scale: 1.0,
        }
    }

    /// A cell's worth of visible face normal at `cell`, pointing `direction`
    /// — enough to give the sub-block correction an axis and side to move
    /// along without needing a full converted map in these unit tests.
    fn normals_at(cell: IVec3, direction: FaceDirection) -> BTreeMap<IVec3, Vec<FaceDirection>> {
        BTreeMap::from([(cell, vec![direction])])
    }

    #[test]
    fn overlap_backed_by_a_real_brush_is_left_untouched() {
        // The prop's bounds overlap a solid grid cell, but a brush exactly
        // matching that cell was in the original map: intentional clipping.
        let grid = solid_grid([0, 0, 0]);
        let solids = vec![block_solid(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        )];
        let mut solid_index = BTreeMap::new();
        solid_index.insert([0, 0, 0], vec![0]);
        let bounds = Aabb::new(Vec3::new(0.25, 0.25, 0.25), Vec3::new(0.75, 0.75, 0.75));
        let snap = snap_prop_bounds(
            &grid,
            &solids,
            &solid_index,
            &BTreeMap::new(),
            bounds,
            &PropOccupancy::always(),
            &identity_transform(),
            1.0,
        );
        assert_eq!(snap.offset, Vec3::ZERO);
        assert!(!snap.intersects);
    }

    #[test]
    fn overlap_with_no_backing_brush_is_corrected() {
        // The grid cell is solid but no brush anywhere near it accounts for
        // that: purely a voxelization artifact, so it gets nudged clear.
        let grid = solid_grid([0, 0, 0]);
        let solids: Vec<BlockSolid> = Vec::new();
        let solid_index = BTreeMap::new();
        let bounds = Aabb::new(Vec3::new(0.25, 0.25, 0.25), Vec3::new(0.75, 0.75, 0.75));
        let visible_normals = normals_at([0, 0, 0], FaceDirection::Up);
        let occupancy = PropOccupancy::always();
        let transform = identity_transform();
        let snap = snap_prop_bounds(
            &grid,
            &solids,
            &solid_index,
            &visible_normals,
            bounds,
            &occupancy,
            &transform,
            1.0,
        );
        assert_ne!(snap.offset, Vec3::ZERO);
        assert!(snap.resolved);
        assert!(
            unintentional_cells(
                &grid,
                &solids,
                &solid_index,
                translated_bounds(bounds, snap.offset),
                &occupancy,
                &transform,
            )
            .is_empty()
        );
    }

    #[test]
    fn a_grazing_corner_is_not_enough_to_call_overlap_intentional() {
        // A brush only clips a sliver of the overlap region (well under the
        // 20% volume threshold): mostly an artifact, so it still corrects.
        let grid = solid_grid([0, 0, 0]);
        let solids = vec![block_solid(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.05, 0.05, 0.05),
        )];
        let mut solid_index = BTreeMap::new();
        solid_index.insert([0, 0, 0], vec![0]);
        let bounds = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0));
        let snap = snap_prop_bounds(
            &grid,
            &solids,
            &solid_index,
            &normals_at([0, 0, 0], FaceDirection::Up),
            bounds,
            &PropOccupancy::always(),
            &identity_transform(),
            1.0,
        );
        assert_ne!(snap.offset, Vec3::ZERO);
    }

    #[test]
    fn a_shallow_true_surface_gives_the_exact_gap_not_a_whole_block() {
        // The grid reads all of cell [0,0,0] as solid floor, but the real
        // brush's top is at 0.90 — just under the cell boundary, the way
        // voxelization rounds a slightly-short floor up to a full cell. A
        // prop resting almost exactly on the true floor only pokes a sliver
        // into the grid's conservative cell, and the correction should
        // measure exactly that sliver, not guess a whole block the way the
        // old snap did.
        let grid = solid_grid([0, 0, 0]);
        let solids = vec![block_solid(
            Vec3::new(-10.0, -10.0, -10.0),
            Vec3::new(10.0, 0.90, 10.0),
        )];
        let mut solid_index = BTreeMap::new();
        solid_index.insert([0, 0, 0], vec![0]);
        let bounds = Aabb::new(Vec3::new(0.25, 0.89, 0.25), Vec3::new(0.75, 1.89, 0.75));
        let snap = snap_prop_bounds(
            &grid,
            &solids,
            &solid_index,
            &normals_at([0, 0, 0], FaceDirection::Up),
            bounds,
            &PropOccupancy::always(),
            &identity_transform(),
            1.0,
        );
        assert!(snap.resolved);
        assert!(
            (snap.offset.y - 0.01).abs() < 1e-9,
            "expected a sub-block gap, not a whole block: {:?}",
            snap.offset
        );
        assert!(snap.offset.y < 1.0);
    }

    #[test]
    fn a_gap_past_the_cap_is_left_unresolved_rather_than_clipped() {
        // The nearest real plane the broad-phase index turns up sits 5.5
        // blocks away — nowhere near what voxelization rounding could ever
        // produce — so this is not a rounding artifact and the prop is left
        // where the mapper put it rather than moved to an invented position.
        let grid = solid_grid([0, 0, 0]);
        let solids = vec![block_solid(
            Vec3::new(-10.0, 5.0, -10.0),
            Vec3::new(10.0, 5.5, 10.0),
        )];
        let mut solid_index = BTreeMap::new();
        solid_index.insert([0, 0, 0], vec![0]);
        let bounds = Aabb::new(Vec3::new(0.25, 0.0, 0.25), Vec3::new(0.75, 0.5, 0.75));
        let snap = snap_prop_bounds(
            &grid,
            &solids,
            &solid_index,
            &normals_at([0, 0, 0], FaceDirection::Up),
            bounds,
            &PropOccupancy::always(),
            &identity_transform(),
            1.0,
        );
        assert_eq!(snap.offset, Vec3::ZERO);
        assert!(snap.intersects && !snap.resolved);
    }

    #[test]
    fn a_diagonal_props_own_geometry_clears_a_box_its_aabb_does_not() {
        // A thin diagonal rod's bounding box overlaps a solid cell along its
        // full diagonal, but the rod itself only actually passes through a
        // sliver of it near one corner. Judged by the mesh rather than the
        // box, the far corner of that cell is not touched at all.
        let mesh = crate::output::mesh::Mesh {
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [1.0, 1.0, 1.0],
            vertices: vec![
                crate::output::mesh::Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
                crate::output::mesh::Vertex {
                    position: [0.1, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
                crate::output::mesh::Vertex {
                    position: [0.0, 0.1, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
            ],
            indices: vec![0, 1, 2],
            submeshes: Vec::new(),
        };
        let occupancy = PropOccupancy::build(&mesh);
        let transform = identity_transform();
        assert!(occupancy.hit(transform.to_local(Vec3::new(0.02, 0.02, 0.02))));
        assert!(!occupancy.hit(transform.to_local(Vec3::new(0.9, 0.9, 0.9))));
    }

    #[test]
    fn a_diagonal_rods_own_geometry_clears_a_cell_its_rotated_box_only_grazes() {
        // A diagonal rod (a pipe, a railing, a flush sign at an angle) has a
        // rotated bounding box far bigger than the rod itself. Here the rod
        // only ever reaches into cell [0,0,0], but its box, as extraction
        // computes it from the rotated corners, reaches on into [1,0,0] too.
        // Judged by the mesh instead of the box, only the first cell — the
        // one the rod actually passes through — counts as a real collision.
        let mesh = crate::output::mesh::Mesh {
            bounds_min: [0.0, 0.0, 0.0],
            bounds_max: [0.3, 0.3, 0.3],
            vertices: vec![
                crate::output::mesh::Vertex {
                    position: [0.0, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
                crate::output::mesh::Vertex {
                    position: [0.3, 0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
                crate::output::mesh::Vertex {
                    position: [0.0, 0.3, 0.3],
                    normal: [0.0, 1.0, 0.0],
                    uv: [0.0, 0.0],
                },
            ],
            indices: vec![0, 1, 2],
            submeshes: Vec::new(),
        };
        let occupancy = PropOccupancy::build(&mesh);
        let transform = identity_transform();
        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], 1);
        grid.set([1, 0, 0], 1);
        // The rotated bounding box, as extraction would compute it, reaches
        // well past the rod's real footprint into the neighbouring cell.
        let bounds = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.5, 0.3, 0.3));
        let cells = intersecting_cells(&grid, bounds, &occupancy, &transform);
        assert!(
            cells.contains(&[0, 0, 0]),
            "the cell the rod really passes through must still count"
        );
        assert!(
            !cells.contains(&[1, 0, 0]),
            "a cell the rod's box only reaches by rotation, not by geometry, must not"
        );
    }

    #[test]
    fn a_pipe_runs_identical_segments_all_receive_the_same_offset() {
        // Three touching segments of the same pipe: only the middle one was
        // found to clip, but the whole run must move together or the pipe
        // comes apart at the joins.
        let assembly_of = vec![0, 0, 0];
        let individual = vec![Vec3::ZERO, Vec3::new(0.0, 0.4, 0.0), Vec3::ZERO];
        let ordinals = vec![10, 11, 12];
        let offsets = assign_assembly_offsets(&assembly_of, &individual, &ordinals);
        assert_eq!(offsets, vec![Vec3::new(0.0, 0.4, 0.0); 3]);
    }

    #[test]
    fn an_assembly_with_no_correction_needed_leaves_every_member_alone() {
        let assembly_of = vec![0, 0];
        let individual = vec![Vec3::ZERO, Vec3::ZERO];
        let ordinals = vec![0, 1];
        let offsets = assign_assembly_offsets(&assembly_of, &individual, &ordinals);
        assert_eq!(offsets, vec![Vec3::ZERO; 2]);
    }

    #[test]
    fn an_unrelated_prop_does_not_inherit_a_neighbours_assembly_correction() {
        // Two separate singleton assemblies: only one of them needed a move.
        let assembly_of = vec![0, 1];
        let individual = vec![Vec3::new(0.2, 0.0, 0.0), Vec3::ZERO];
        let ordinals = vec![0, 1];
        let offsets = assign_assembly_offsets(&assembly_of, &individual, &ordinals);
        assert_eq!(offsets, vec![Vec3::new(0.2, 0.0, 0.0), Vec3::ZERO]);
    }

    #[test]
    fn a_stacked_pair_keeps_its_relative_offset() {
        // A pallet and a crate exactly touching in the source map (pallet top
        // at y=1, crate base at y=1). Each settled independently by a
        // slightly different amount, as the grid's own rounding does; the
        // crate must end up moved by exactly what the pallet was, so the two
        // stay in contact rather than sinking into or floating above it.
        let unsettled_bounds = vec![
            Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)),
            Aabb::new(Vec3::new(0.0, 1.0, 0.0), Vec3::new(1.0, 2.0, 1.0)),
        ];
        let mut settle_shift = [0.05, -0.2];
        let mut offset = [Vec3::ZERO, Vec3::new(0.0, 0.3, 0.0)];
        let ordinals = [0u64, 1u64];
        let order = [0usize, 1usize];
        apply_stacking_inheritance(
            &order,
            &unsettled_bounds,
            &mut settle_shift,
            &mut offset,
            &ordinals,
        );
        assert_eq!(settle_shift[1], settle_shift[0]);
        assert_eq!(offset[1], offset[0]);
    }

    #[test]
    fn a_prop_on_the_floor_is_unaffected_by_an_unrelated_prop_nearby() {
        // A second prop sitting on the floor a few blocks away must not be
        // mistaken for a support just because it was processed first.
        let unsettled_bounds = vec![
            Aabb::new(Vec3::new(10.0, 0.0, 10.0), Vec3::new(11.0, 1.0, 11.0)),
            Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)),
        ];
        let mut settle_shift = [0.3, 0.0];
        let mut offset = [Vec3::ZERO, Vec3::ZERO];
        let ordinals = [0u64, 1u64];
        let order = [0usize, 1usize];
        apply_stacking_inheritance(
            &order,
            &unsettled_bounds,
            &mut settle_shift,
            &mut offset,
            &ordinals,
        );
        assert_eq!(settle_shift[1], 0.0);
        assert_eq!(offset[1], Vec3::ZERO);
    }

    #[test]
    fn floating_prop_settles_onto_the_floor() {
        let mut grid = VoxelGrid::new();
        grid.set([0, 0, 0], 1);
        let bounds = Aabb::new(Vec3::new(0.25, 1.5, 0.25), Vec3::new(0.75, 2.0, 0.75));
        let shift = crate::voxel::settle::offset(&grid, bounds, 4.0);
        assert!(shift < 0.0, "expected the prop to be pulled down: {shift}");
    }

    #[test]
    fn synthetic_campaign_is_deterministic_and_deduplicates_shared_meshes() {
        let mesh = triangle_mesh();
        let first_dir = temp_dir("first");
        let second_dir = temp_dir("second");
        let first = write_campaign(
            &first_dir,
            "fixture",
            vec![
                fixture_map("map_b", "models/b.mdl", mesh.clone()),
                fixture_map("map_a", "models/a.mdl", mesh.clone()),
            ],
        )
        .unwrap();
        let second = write_campaign(
            &second_dir,
            "fixture",
            vec![
                fixture_map("map_a", "models/a.mdl", mesh.clone()),
                fixture_map("map_b", "models/b.mdl", mesh),
            ],
        )
        .unwrap();

        assert_eq!(first.manifest, second.manifest);
        assert_eq!(
            first.manifest.fingerprint,
            include_str!("../../tests/fixtures/mod_export_fingerprint.txt").trim()
        );
        assert_eq!(
            first
                .manifest
                .entries
                .iter()
                .filter(|entry| entry.path.starts_with("meshes/"))
                .count(),
            1
        );
        for map_id in ["map_a", "map_b"] {
            let first_schematic = std::fs::read(first_dir.join(format!("{map_id}.schem"))).unwrap();
            assert_eq!(
                &first_schematic[..2],
                &[0x1f, 0x8b],
                "WorldEdit requires GZIP NBT"
            );
            assert_eq!(
                first_schematic,
                std::fs::read(second_dir.join(format!("{map_id}.schem"))).unwrap()
            );
        }
        assert_eq!(
            std::fs::read(&first.bundle).unwrap(),
            std::fs::read(&second.bundle).unwrap()
        );

        std::fs::remove_dir_all(first_dir).unwrap();
        std::fs::remove_dir_all(second_dir).unwrap();
    }

    #[test]
    fn identical_mesh_bytes_keep_distinct_material_bindings() {
        let bytes = triangle_mesh();
        let content_id = bundle::content_id(&bytes);
        let mut map = fixture_map("map", "models/a.mdl", bytes.clone());
        map.cell_max[0] = 2;
        map.materials.push(metadata::MaterialReference {
            source_material: "fixture/alternate".into(),
            source_material_raw: None,
            render_class: metadata::RenderClass::Fallback,
            texture: None,
            surface_prop: None,
            reflectivity: [0.0; 3],
        });
        map.models.push(ModelAsset {
            source_model: "models/b.mdl".into(),
            bytes,
            materials: vec![1],
        });
        map.props.push(Prop {
            source_ordinal: 1,
            source_model: "models/b.mdl".into(),
            model_content_id: content_id,
            root_cell: [2, 0, 0],
            translation: [1.5, 0.0, 0.5],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            material_ids: vec![1],
        });
        let dir = temp_dir("material-bindings");
        let written = write_campaign(&dir, "fixture", vec![map]).unwrap();
        assert_eq!(
            written
                .manifest
                .entries
                .iter()
                .filter(|entry| entry.path.starts_with("meshes/"))
                .count(),
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
