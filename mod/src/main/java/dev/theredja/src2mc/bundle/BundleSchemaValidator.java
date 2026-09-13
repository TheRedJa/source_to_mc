package dev.theredja.src2mc.bundle;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.ByteArrayOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Semantic and binary-schema validation after the ZIP has passed its hash envelope. */
final class BundleSchemaValidator {
    private static final byte[] FACE_MAGIC = {'S','2','F','A','C','E',0,0};
    private static final byte[] PROP_MAGIC = {'S','2','P','R','O','P',0,0};
    private static final byte[] MESH_MAGIC = {'S','2','M','E','S','H',0,0};
    private static final byte[] PVS_MAGIC = {'S','2','P','V','I','S',0,0};
    private static final byte[] OCCLUSION_MAGIC = {'S','2','O','C','C','L',0,0};
    private static final byte[] PNG_SIGNATURE = {(byte)137,80,78,71,13,10,26,10};

    private BundleSchemaValidator() {}

    /** One campaign map, after the campaign-level checks and before its own. */
    private record MapRef(String mapId, String metadata) {}

    static List<BundleMap> validate(ZipFile zip, Map<String, ZipEntry> entries, String manifestCampaign, List<BundleManifest.Entry> manifestEntries) throws IOException {
        Map<String, String> hashes = new java.util.HashMap<>();
        for (BundleManifest.Entry entry : manifestEntries) hashes.put(entry.path(), entry.sha256());
        JsonObject campaign = json(zip, required(entries, "campaign.json"), "campaign.json");
        if (campaign.has("atlas")) keys(campaign, "format", "version", "campaign_id", "atlas", "maps");
        else keys(campaign, "format", "version", "campaign_id", "maps");
        format(campaign, "src2mc-campaign-metadata", "campaign.json");
        if (!manifestCampaign.equals(string(campaign, "campaign_id"))) fail(BundleErrorCode.INVALID_REFERENCE, "campaign IDs differ");
        JsonArray maps = array(campaign, "maps");
        limit(maps.size(), BundleLimits.MAX_MAPS, "map count");
        // Everything a map validates is its own; the only shared state left is
        // the referenced-payload set, so the campaign-level checks that do care
        // about order -- sortedness, canonical metadata paths -- happen here,
        // before the maps themselves are handed to the pool.
        Set<String> referenced = ConcurrentHashMap.newKeySet();
        AtlasIndex atlas = campaign.has("atlas")
            ? validateAtlas(zip, entries, hashes, exactPath(campaign, "atlas", "atlas.json"), referenced)
            : null;
        String previous = null;
        List<MapRef> refs = new ArrayList<>(maps.size());
        for (JsonElement itemElement : maps) {
            JsonObject item = object(itemElement, "campaign map");
            keys(item, "map_id", "metadata");
            String mapId = id(string(item, "map_id"), "map");
            if (previous != null && previous.compareTo(mapId) >= 0) fail(BundleErrorCode.DUPLICATE_IDENTITY, "maps are not uniquely sorted");
            previous = mapId;
            String metadata = string(item, "metadata");
            if (!metadata.equals("maps/" + mapId + ".json")) fail(BundleErrorCode.INVALID_REFERENCE, "non-canonical metadata path for " + mapId);
            refs.add(new MapRef(mapId, metadata));
        }
        List<BundleMap> loadedMaps = BundleLoadPool.map(refs,
            ref -> validateMap(zip, entries, hashes, ref.mapId(), ref.metadata(), referenced, atlas));
        long modelCount = 0;
        for (BundleMap loaded : loadedMaps) {
            modelCount = Math.addExact(modelCount, loaded.modelCount());
            limit(modelCount, BundleLimits.MAX_MODELS_PER_CAMPAIGN, "campaign model count");
        }
        for (String path : entries.keySet()) {
            if (path.equals("manifest.json") || path.equals("campaign.json")) continue;
            if (!referenced.contains(path)) fail(BundleErrorCode.INVALID_REFERENCE, "unreferenced payload: " + path);
        }
        return List.copyOf(loadedMaps);
    }

    private static BundleMap validateMap(ZipFile zip, Map<String, ZipEntry> entries, Map<String, String> hashes, String mapId, String path, Set<String> referenced, AtlasIndex atlas) throws IOException {
        referenced.add(path);
        JsonObject map = json(zip, required(entries, path), path);
        // Both optional tables sit between props and diagnostics, in that order.
        List<String> expected = new ArrayList<>(List.of("format", "version", "map_id", "source_name", "units_per_block",
            "cell_min", "cell_max", "anchor_cell", "surfaces", "materials", "models", "props"));
        if (map.has("pvs")) expected.add("pvs");
        if (map.has("occlusion")) expected.add("occlusion");
        expected.add("diagnostics");
        keys(map, expected.toArray(String[]::new));
        format(map, "src2mc-map", path);
        if (!mapId.equals(string(map, "map_id"))) fail(BundleErrorCode.INVALID_REFERENCE, "map ID differs from path: " + path);
        String sourceName = string(map, "source_name");
        if (sourceName.isEmpty()) fail(BundleErrorCode.INVALID_SCHEMA, "empty source_name: " + path);
        double units = finiteNumber(map.get("units_per_block"), "units_per_block");
        if (units != 32.0) fail(BundleErrorCode.INVALID_SCHEMA, "mod bundle units_per_block must be 32");
        int[] min = vector3i(map.get("cell_min"), "cell_min");
        int[] max = vector3i(map.get("cell_max"), "cell_max");
        int[] anchor = vector3i(map.get("anchor_cell"), "anchor_cell");
        for (int axis = 0; axis < 3; axis++) if (min[axis] > max[axis]) fail(BundleErrorCode.INVALID_SCHEMA, "invalid map bounds");

        JsonArray materials = array(map, "materials");
        limit(materials.size(), BundleLimits.MAX_MATERIALS_PER_MAP, "material count");
        Map<String, int[]> textureIds = new java.util.HashMap<>();
        List<BundleMaterial> loadedMaterials = new ArrayList<>(materials.size());
        for (JsonElement value : materials) loadedMaterials.add(validateMaterial(object(value, "material"), textureIds));

        JsonArray models = array(map, "models");
        limit(models.size(), BundleLimits.MAX_MODELS_PER_CAMPAIGN, "model count");
        List<BundleModel> modelRefs = new ArrayList<>(models.size());
        BundleModel prior = null;
        for (JsonElement value : models) {
            JsonObject model = object(value, "model");
            keys(model, "content_id", "source_model", "materials");
            String contentId = digest(string(model, "content_id"), "model content ID");
            String source = string(model, "source_model");
            if (source.isEmpty()) fail(BundleErrorCode.INVALID_SCHEMA, "empty source model");
            JsonArray slots = array(model, "materials");
            if (slots.isEmpty()) fail(BundleErrorCode.INVALID_SCHEMA, "model has no material slots");
            int[] ids = new int[slots.size()];
            for (int i = 0; i < ids.length; i++) { ids[i] = uintIndex(slots.get(i), "material ID"); if (ids[i] >= materials.size()) fail(BundleErrorCode.INVALID_REFERENCE, "model material ID out of range"); }
            BundleModel current = new BundleModel(contentId, source, ids);
            if (prior != null && compareModels(prior, current) >= 0) fail(BundleErrorCode.DUPLICATE_IDENTITY, "model references are not uniquely sorted");
            prior = current;
            modelRefs.add(current);
        }

        String prefix = "maps/" + mapId + "/";
        String surfaces = exactPath(map, "surfaces", prefix + "surfaces.s2faces");
        String props = exactPath(map, "props", prefix + "props.s2props");
        String diagnostics = exactPath(map, "diagnostics", prefix + "diagnostics.json");
        PropVisibility pvs = null;
        if (map.has("pvs")) {
            String pvsPath = exactPath(map, "pvs", prefix + "pvs.s2pvs");
            referenced.add(pvsPath);
            pvs = validatePvs(zip, required(entries, pvsPath));
        }
        OcclusionTable occlusion = null;
        if (map.has("occlusion")) {
            String occlusionPath = exactPath(map, "occlusion", prefix + "occlusion.s2occl");
            referenced.add(occlusionPath);
            occlusion = validateOcclusion(zip, required(entries, occlusionPath));
        }
        referenced.addAll(List.of(surfaces, props, diagnostics));
        SurfaceTable surfaceTable = validateFaces(zip, required(entries, surfaces), materials.size());
        List<BundleProp> propRecords = validateProps(zip, required(entries, props), modelRefs.size());
        validateDiagnostics(json(zip, required(entries, diagnostics), diagnostics));

        // A map's meshes are the bulk of its validation and each is walked on
        // its own, so this is where the parallelism pays for a campaign that
        // holds a single large map.
        BundleLoadPool.forEach(modelRefs, model -> {
            String mesh = "meshes/" + model.contentId() + ".s2mesh";
            referenced.add(mesh);
            contentHash(hashes, mesh, model.contentId());
            validateMesh(zip, required(entries, mesh), model.materialSlotCount());
        });
        if (!textureIds.isEmpty() && atlas == null) fail(BundleErrorCode.MISSING_ENTRY, "textured materials require atlas.json");
        for (Map.Entry<String, int[]> textureRef : textureIds.entrySet()) {
            AtlasIndex.Texture texture = atlas.textures().get(textureRef.getKey());
            if (texture == null || texture.width() != textureRef.getValue()[0] || texture.height() != textureRef.getValue()[1])
                fail(BundleErrorCode.INVALID_REFERENCE, "material texture is absent from atlas or has conflicting dimensions");
        }
        long mapHeight = (long) max[1] - min[1] + 1;
        return new BundleMap(mapId, sourceName, min, max, anchor, loadedMaterials, modelRefs, propRecords, mapHeight > 384,
            surfaceTable, modelRefs.stream().map(BundleModel::contentId).collect(java.util.stream.Collectors.toUnmodifiableSet()), atlas, pvs, occlusion);
    }

    private static AtlasIndex validateAtlas(ZipFile zip, Map<String, ZipEntry> entries, Map<String, String> hashes, String path, Set<String> referenced) throws IOException {
        referenced.add(path);
        JsonObject root = json(zip, required(entries, path), path);
        keys(root, "format", "version", "page_size", "max_mip_level", "gutter", "pages", "textures");
        format(root, "src2mc-atlas", path);
        int pageSize = uintIndex(root.get("page_size"), "page_size");
        int maxMip = uintIndex(root.get("max_mip_level"), "max_mip_level");
        int gutter = uintIndex(root.get("gutter"), "gutter");
        if (pageSize != 4096 || maxMip != 4 || gutter != 16) fail(BundleErrorCode.INVALID_SCHEMA, "non-canonical atlas parameters");
        List<AtlasIndex.Page> pages = new ArrayList<>();
        JsonArray pageArray = array(root, "pages");
        for (int p = 0; p < pageArray.size(); p++) {
            JsonObject page = object(pageArray.get(p), "atlas page"); keys(page, "page", "mips");
            if (uintIndex(page.get("page"), "page") != p) fail(BundleErrorCode.INVALID_SCHEMA, "atlas pages are not ordered");
            JsonArray mipArray = array(page, "mips");
            if (mipArray.size() != maxMip + 1) fail(BundleErrorCode.INVALID_SCHEMA, "incomplete atlas mip chain");
            List<AtlasIndex.Mip> mips = new ArrayList<>();
            for (int level = 0; level < mipArray.size(); level++) {
                JsonObject mip = object(mipArray.get(level), "atlas mip"); keys(mip, "level", "content_id", "width", "height");
                if (uintIndex(mip.get("level"), "level") != level) fail(BundleErrorCode.INVALID_SCHEMA, "atlas mips are not ordered");
                String id = digest(string(mip, "content_id"), "atlas page content ID");
                int width = dimension(mip, "width", pageSize), height = dimension(mip, "height", pageSize);
                if (width != pageSize >> level || height != pageSize >> level) fail(BundleErrorCode.INVALID_SCHEMA, "invalid atlas mip dimensions");
                String png = "atlas/" + id + ".png"; referenced.add(png); contentHash(hashes, png, id);
                validatePng(zip, required(entries, png), new int[]{width, height});
                mips.add(new AtlasIndex.Mip(level, png, width, height));
            }
            pages.add(new AtlasIndex.Page(p, mips));
        }
        Map<String, AtlasIndex.Texture> textures = new java.util.LinkedHashMap<>();
        List<AtlasIndex.Region> allRegions = new ArrayList<>();
        String previous = null;
        for (JsonElement element : array(root, "textures")) {
            JsonObject texture = object(element, "atlas texture"); keys(texture, "content_id", "width", "height", "regions");
            String id = digest(string(texture, "content_id"), "logical texture content ID");
            if (previous != null && previous.compareTo(id) >= 0) fail(BundleErrorCode.DUPLICATE_IDENTITY, "atlas textures are not uniquely sorted"); previous = id;
            int width = dimension(texture, "width", BundleLimits.MAX_ORIGINAL_TEXTURE_AXIS), height = dimension(texture, "height", BundleLimits.MAX_ORIGINAL_TEXTURE_AXIS);
            List<AtlasIndex.Region> regions = new ArrayList<>(); long covered = 0;
            for (JsonElement value : array(texture, "regions")) {
                JsonObject region = object(value, "atlas region"); keys(region, "source", "page", "allocation");
                int[] source = rect(region.get("source"), width, height, "source");
                int page = uintIndex(region.get("page"), "page"); if (page >= pages.size()) fail(BundleErrorCode.INVALID_REFERENCE, "atlas region page out of range");
                int[] allocation = rect(region.get("allocation"), pageSize, pageSize, "allocation");
                if (source[2] != allocation[2] || source[3] != allocation[3]) fail(BundleErrorCode.INVALID_SCHEMA, "atlas region changes texture dimensions");
                if (allocation[0] < gutter || allocation[1] < gutter || (long)allocation[0] + allocation[2] + gutter > pageSize || (long)allocation[1] + allocation[3] + gutter > pageSize) fail(BundleErrorCode.INVALID_SCHEMA, "atlas region lacks mip gutter");
                covered = Math.addExact(covered, (long)source[2] * source[3]); regions.add(new AtlasIndex.Region(source, page, allocation));
            }
            if (regions.isEmpty() || covered != (long)width * height) fail(BundleErrorCode.INVALID_SCHEMA, "atlas regions do not cover logical texture");
            ensureNoOverlap(regions, true, 0); ensureNoOverlap(regions, false, gutter);
            allRegions.addAll(regions);
            textures.put(id, new AtlasIndex.Texture(id, width, height, regions));
        }
        if (textures.isEmpty() || pages.isEmpty()) fail(BundleErrorCode.INVALID_SCHEMA, "empty atlas must be omitted");
        ensureNoOverlap(allRegions, false, gutter);
        return new AtlasIndex(pageSize, maxMip, gutter, pages, textures);
    }

    private static int[] rect(JsonElement value, int width, int height, String label) throws BundleValidationException {
        if (value == null || !value.isJsonArray() || value.getAsJsonArray().size() != 4) fail(BundleErrorCode.INVALID_SCHEMA, label + " must be [x,y,width,height]");
        int[] r = new int[4]; for (int i=0;i<4;i++) r[i]=uintIndex(value.getAsJsonArray().get(i),label);
        if (r[2] == 0 || r[3] == 0 || (long)r[0]+r[2] > width || (long)r[1]+r[3] > height) fail(BundleErrorCode.INVALID_SCHEMA, "invalid " + label + " rectangle"); return r;
    }

    private static void ensureNoOverlap(List<AtlasIndex.Region> regions, boolean source, int padding) throws BundleValidationException {
        for (int i=0;i<regions.size();i++) for(int j=i+1;j<regions.size();j++) {
            if (!source && regions.get(i).page()!=regions.get(j).page()) continue;
            int[] a=source?regions.get(i).source():regions.get(i).allocation(), b=source?regions.get(j).source():regions.get(j).allocation();
            if ((long)a[0]-padding<b[0]+b[2]+padding && (long)b[0]-padding<a[0]+a[2]+padding && (long)a[1]-padding<b[1]+b[3]+padding && (long)b[1]-padding<a[1]+a[3]+padding) fail(BundleErrorCode.INVALID_SCHEMA, "overlapping atlas regions or gutters");
        }
    }

    private static BundleMaterial validateMaterial(JsonObject material, Map<String, int[]> textures) throws BundleValidationException {
        Set<String> allowed = Set.of("source_material", "source_material_raw", "render_class", "texture", "surface_prop", "reflectivity");
        if (!allowed.containsAll(material.keySet()) || !material.has("source_material") || !material.has("render_class") || !material.has("reflectivity")) fail(BundleErrorCode.INVALID_SCHEMA, "invalid material fields");
        String source = string(material, "source_material");
        if (source.isEmpty()) fail(BundleErrorCode.INVALID_SCHEMA, "empty source material");
        if (material.has("source_material_raw") && source.equals(string(material, "source_material_raw"))) fail(BundleErrorCode.INVALID_SCHEMA, "redundant raw material path");
        String renderClassName = string(material, "render_class");
        if (!Set.of("solid", "cutout", "translucent", "fallback").contains(renderClassName)) fail(BundleErrorCode.INVALID_SCHEMA, "invalid render class");
        JsonArray reflectivity = array(material, "reflectivity");
        if (reflectivity.size() != 3) fail(BundleErrorCode.INVALID_SCHEMA, "reflectivity must have three values");
        for (JsonElement value : reflectivity) finiteNumber(value, "reflectivity");
        BundleMaterial.TextureReference loadedTexture = null;
        if (material.has("texture")) {
            JsonObject texture = object(material.get("texture"), "texture reference");
            keys(texture, "content_id", "original_width", "original_height", "output_width", "output_height");
            String textureId = digest(string(texture, "content_id"), "texture content ID");
            int originalWidth = dimension(texture, "original_width", BundleLimits.MAX_ORIGINAL_TEXTURE_AXIS);
            int originalHeight = dimension(texture, "original_height", BundleLimits.MAX_ORIGINAL_TEXTURE_AXIS);
            int width = dimension(texture, "output_width", BundleLimits.MAX_ORIGINAL_TEXTURE_AXIS);
            int height = dimension(texture, "output_height", BundleLimits.MAX_ORIGINAL_TEXTURE_AXIS);
            int[] old = textures.putIfAbsent(textureId, new int[]{width, height});
            if (old != null && (old[0] != width || old[1] != height)) fail(BundleErrorCode.INVALID_REFERENCE, "shared texture has conflicting dimensions");
            loadedTexture = new BundleMaterial.TextureReference(textureId, originalWidth, originalHeight, width, height);
        }
        return new BundleMaterial(string(material, "source_material"), switch (renderClassName) {
            case "solid" -> BundleMaterial.RenderClass.SOLID;
            case "cutout" -> BundleMaterial.RenderClass.CUTOUT;
            case "translucent" -> BundleMaterial.RenderClass.TRANSLUCENT;
            default -> BundleMaterial.RenderClass.FALLBACK;
        }, loadedTexture);
    }

    private static void validateDiagnostics(JsonObject diagnostics) throws BundleValidationException {
        keys(diagnostics, "format", "version", "messages");
        format(diagnostics, "src2mc-diagnostics", "diagnostics");
        DiagnosticOrder previous = null;
        for (JsonElement value : array(diagnostics, "messages")) {
            JsonObject message = object(value, "diagnostic message");
            keys(message, "severity", "code", "message", "context");
            String severity = string(message, "severity");
            if (!Set.of("info", "warning", "error").contains(severity)) fail(BundleErrorCode.INVALID_SCHEMA, "invalid diagnostic severity");
            String code = string(message, "code");
            if (code.isEmpty() || !code.chars().allMatch(c -> c >= 'A' && c <= 'Z' || Character.isDigit(c) || c == '_')) fail(BundleErrorCode.INVALID_SCHEMA, "invalid diagnostic code");
            if (string(message, "message").isEmpty()) fail(BundleErrorCode.INVALID_SCHEMA, "empty diagnostic message");
            JsonObject context = object(message.get("context"), "diagnostic context");
            String contextPrevious = null;
            for (String key : context.keySet()) { if (contextPrevious != null && contextPrevious.compareTo(key) >= 0) fail(BundleErrorCode.INVALID_SCHEMA, "unsorted diagnostic context"); string(context, key); contextPrevious = key; }
            DiagnosticOrder order = new DiagnosticOrder(switch (severity) { case "info" -> 0; case "warning" -> 1; default -> 2; }, code, message.get("message").getAsString(), context.toString());
            if (previous != null && previous.compareTo(order) > 0) fail(BundleErrorCode.INVALID_SCHEMA, "diagnostics are not sorted");
            previous = order;
        }
    }

    private static SurfaceTable validateFaces(ZipFile zip, ZipEntry entry, int materialCount) throws IOException {
        try (Binary in = new Binary(zip.getInputStream(entry), entry.getSize())) {
            in.magic(FACE_MAGIC); in.version();
            long uvCount = in.count(BundleLimits.MAX_UV_REGIONS_PER_MAP, "UV region");
            long sectionCount = in.count(BundleLimits.MAX_SECTIONS_PER_MAP, "section");
            long faceCount = in.count(BundleLimits.MAX_FACES_PER_MAP, "face");
            in.requireRemaining(Math.addExact(Math.multiplyExact(uvCount, 64), Math.multiplyExact(sectionCount, 16)), "surface header counts");
            long[] priorUv = null;
            List<SurfaceTable.UvRegion> uvRegions = new ArrayList<>((int) uvCount);
            for (long i = 0; i < uvCount; i++) {
                long[] uv = new long[8];
                for (int n = 0; n < 8; n++) uv[n] = in.canonicalF64Bits("UV");
                if (priorUv != null && compareUnsigned(priorUv, uv) >= 0) fail(BundleErrorCode.INVALID_SCHEMA, "UV regions are not uniquely sorted");
                priorUv = uv;
                double[] values = new double[8];
                for (int n = 0; n < 8; n++) values[n] = Double.longBitsToDouble(uv[n]);
                uvRegions.add(new SurfaceTable.UvRegion(values));
            }
            int[] priorSection = null; long seenFaces = 0;
            Map<SurfaceTable.SectionPos, List<SurfaceTable.Face>> sections = new java.util.HashMap<>();
            for (long s = 0; s < sectionCount; s++) {
                int[] section = {in.i32(), in.i32(), in.i32()};
                if (priorSection != null && compare(priorSection, section) >= 0) fail(BundleErrorCode.INVALID_SCHEMA, "sections are not uniquely sorted");
                priorSection = section;
                long count = in.u32(); if (count == 0) fail(BundleErrorCode.INVALID_SCHEMA, "empty surface section");
                List<SurfaceTable.Face> faces = new ArrayList<>((int) count);
                seenFaces = Math.addExact(seenFaces, count); if (seenFaces > faceCount) fail(BundleErrorCode.INVALID_SCHEMA, "section face counts exceed header");
                in.requireRemaining(Math.multiplyExact(count, 20), "face records");
                FaceOrder prior = null;
                for (long f = 0; f < count; f++) {
                    byte[] record = in.bytes(20);
                    int local = u16(record, 0), patch = record[2] & 255, provenance = record[3] & 255;
                    if ((local & 0xf000) != 0 || (patch & 0x80) != 0 || (patch & 7) > 5 || ((patch >>> 3) & 3) > 2 || provenance > 1) fail(BundleErrorCode.INVALID_SCHEMA, "invalid face record bits");
                    if (u32(record, 4) >= Integer.toUnsignedLong(materialCount) || u32(record, 8) >= uvCount) fail(BundleErrorCode.INVALID_REFERENCE, "surface reference out of range");
                    FaceOrder order = new FaceOrder(local, patch, u32(record,4), u32(record,8), provenance, u32(record,12), u32(record,16));
                    if (prior != null && prior.compareTo(order) >= 0) fail(BundleErrorCode.INVALID_SCHEMA, "faces are not uniquely sorted");
                    prior = order;
                    faces.add(new SurfaceTable.Face(local, patch, provenance, (int)u32(record,4), (int)u32(record,8), u32(record,12), u32(record,16)));
                }
                sections.put(new SurfaceTable.SectionPos(section[0], section[1], section[2]), faces);
            }
            if (seenFaces != faceCount) fail(BundleErrorCode.INVALID_SCHEMA, "surface face count differs");
            in.end();
            return new SurfaceTable(uvRegions, sections);
        }
    }

    private static List<BundleProp> validateProps(ZipFile zip, ZipEntry entry, int modelCount) throws IOException {
        try (Binary in = new Binary(zip.getInputStream(entry), entry.getSize())) {
            in.magic(PROP_MAGIC); in.version(); long count = in.count(BundleLimits.MAX_PROPS_PER_MAP, "prop");
            in.requireRemaining(Math.multiplyExact(count, 112), "prop records");
            List<BundleProp> props = new ArrayList<>((int) count);
            byte[] prior = null;
            for (long i = 0; i < count; i++) {
                byte[] id = in.bytes(32); if (prior != null && compareUnsignedBytes(prior, id) >= 0) fail(BundleErrorCode.DUPLICATE_IDENTITY, "prop IDs are not uniquely sorted"); prior = id;
                long model = in.u32(); if (model >= Integer.toUnsignedLong(modelCount)) fail(BundleErrorCode.INVALID_REFERENCE, "prop model out of range");
                int[] root = {in.i32(), in.i32(), in.i32()};
                double[] translation = new double[3]; for (int n = 0; n < 3; n++) translation[n] = Double.longBitsToDouble(in.canonicalF64Bits("translation"));
                double[] rotation = new double[4]; double length = 0; for (int n = 0; n < 4; n++) { double q = Double.longBitsToDouble(in.canonicalF64Bits("rotation")); rotation[n] = q; length += q*q; }
                double scale = Double.longBitsToDouble(in.canonicalF64Bits("scale"));
                if (Math.abs(length - 1.0) > 1e-9 || scale <= 0) fail(BundleErrorCode.INVALID_SCHEMA, "invalid prop transform");
                props.add(new BundleProp(java.util.HexFormat.of().formatHex(id), (int) model, root, translation, rotation, scale));
            }
            in.end();
            return List.copyOf(props);
        }
    }

    /** Parses and validates the optional prop visibility table (format.md section 12). */
    private static OcclusionTable validateOcclusion(ZipFile zip, ZipEntry entry) throws IOException {
        try (Binary in = new Binary(zip.getInputStream(entry), entry.getSize())) {
            in.magic(OCCLUSION_MAGIC);
            in.version();
            long sections = in.count(BundleLimits.MAX_SECTIONS_PER_MAP, "occlusion section");
            in.requireRemaining(Math.multiplyExact(sections, 12L + OcclusionTable.SECTION_BYTES), "occlusion payload");
            Map<SurfaceTable.SectionPos, byte[]> loaded = new java.util.HashMap<>();
            int[] previous = null;
            for (long i = 0; i < sections; i++) {
                int[] at = {in.i32(), in.i32(), in.i32()};
                if (previous != null && compare(previous, at) >= 0) {
                    fail(BundleErrorCode.DUPLICATE_IDENTITY, "occlusion sections are not uniquely sorted");
                }
                previous = at;
                byte[] bits = in.bytes(OcclusionTable.SECTION_BYTES);
                boolean empty = true;
                for (byte value : bits) if (value != 0) { empty = false; break; }
                if (empty) fail(BundleErrorCode.INVALID_SCHEMA, "occlusion section has no cells");
                loaded.put(new SurfaceTable.SectionPos(at[0], at[1], at[2]), bits);
            }
            in.end();
            return new OcclusionTable(loaded);
        }
    }

    private static PropVisibility validatePvs(ZipFile zip, ZipEntry entry) throws IOException {
        try (Binary in = new Binary(zip.getInputStream(entry), entry.getSize())) {
            in.magic(PVS_MAGIC);
            if (in.u32() != 2) fail(BundleErrorCode.UNSUPPORTED_VERSION, "unsupported PVS binary version");
            long clusters = in.count(BundleLimits.MAX_PVS_CLUSTERS, "pvs cluster");
            if (clusters == 0) fail(BundleErrorCode.INVALID_SCHEMA, "pvs table has no clusters");
            long rowBytes = in.u32();
            if (rowBytes != (clusters + 7) / 8) fail(BundleErrorCode.INVALID_SCHEMA, "pvs row bytes do not cover the cluster count");
            if (Math.multiplyExact(clusters, rowBytes) > BundleLimits.MAX_PVS_BITSET_BYTES) fail(BundleErrorCode.LIMIT_EXCEEDED, "pvs bitset payload exceeds limit");
            long nodes = in.count(BundleLimits.MAX_PVS_LEAVES, "pvs node");
            int root = in.i32();
            if (root < 0 || root >= nodes) fail(BundleErrorCode.INVALID_SCHEMA, "pvs root node is invalid");
            long sections = in.count(BundleLimits.MAX_SECTIONS_PER_MAP, "pvs section");
            in.requireRemaining(Math.addExact(Math.addExact(Math.multiplyExact(nodes, 24), Math.multiplyExact(sections, 16)), Math.multiplyExact(clusters, rowBytes)), "pvs payload");
            PropVisibility.Node[] nodeList = new PropVisibility.Node[(int) nodes];
            for (int i = 0; i < nodes; i++) {
                float nx = in.canonicalF32("pvs node normal"), ny = in.canonicalF32("pvs node normal"), nz = in.canonicalF32("pvs node normal"), dist = in.canonicalF32("pvs node distance");
                int[] children = {in.i32(), in.i32()};
                for (int child : children) {
                    if (child >= nodes || (child < 0 && child != Integer.MIN_VALUE && -1 - child >= clusters)) {
                        fail(BundleErrorCode.INVALID_SCHEMA, "pvs node child is invalid");
                    }
                }
                nodeList[i] = new PropVisibility.Node(nx, ny, nz, dist, children);
            }
            Map<PropVisibility.SectionKey, short[]> sectionMap = new HashMap<>((int) Math.min(sections, 65_536));
            int[] priorSection = null;
            for (long i = 0; i < sections; i++) {
                int[] section = {in.i32(), in.i32(), in.i32()};
                long clusterCount = in.count(BundleLimits.MAX_PVS_CLUSTERS, "pvs section cluster");
                short[] clusterSet = new short[(int) clusterCount];
                short prior = -1;
                for (int c = 0; c < clusterCount; c++) {
                    short cluster = (short) in.i16();
                    if (cluster < 0 || (c > 0 && cluster <= prior)) fail(BundleErrorCode.INVALID_SCHEMA, "pvs section clusters are not uniquely sorted");
                    clusterSet[c] = cluster; prior = cluster;
                }
                if (priorSection != null && compare(priorSection, section) >= 0) fail(BundleErrorCode.INVALID_SCHEMA, "pvs sections are not uniquely sorted");
                priorSection = section;
                sectionMap.put(new PropVisibility.SectionKey(section[0], section[1], section[2]), clusterSet);
            }
            byte[][] rows = new byte[(int) clusters][];
            for (int c = 0; c < clusters; c++) rows[c] = in.bytes((int) rowBytes);
            in.end();
            for (int c = 0; c < clusters; c++) if ((rows[c][c >> 3] & (1 << (c & 7))) == 0) fail(BundleErrorCode.INVALID_SCHEMA, "pvs row does not contain itself");
            return new PropVisibility((int) clusters, nodeList, root, rows, sectionMap);
        }
    }

    private static void validateMesh(ZipFile zip, ZipEntry entry, int materialSlots) throws IOException {        try (Binary in = new Binary(zip.getInputStream(entry), entry.getSize())) {
            in.magic(MESH_MAGIC); in.version(); long vertices = in.count(BundleLimits.MAX_VERTICES_PER_MESH, "vertex"), indices = in.count(BundleLimits.MAX_INDICES_PER_MESH, "index"), submeshes = in.count(BundleLimits.MAX_SUBMESHES_PER_MESH, "submesh");
            if (vertices == 0 || indices == 0 || indices % 3 != 0 || submeshes == 0) fail(BundleErrorCode.INVALID_SCHEMA, "empty or non-triangular mesh");
            float[] min = new float[3], max = new float[3]; for (int i=0;i<3;i++) min[i]=in.canonicalF32("bounds"); for (int i=0;i<3;i++) max[i]=in.canonicalF32("bounds"); for(int i=0;i<3;i++) if(min[i]>max[i]) fail(BundleErrorCode.INVALID_SCHEMA,"invalid mesh bounds");
            in.requireRemaining(Math.addExact(Math.addExact(Math.multiplyExact(vertices,32),Math.multiplyExact(indices,4)),Math.multiplyExact(submeshes,12)), "mesh arrays");
            for(long v=0;v<vertices;v++){ for(int n=0;n<3;n++) in.canonicalF32("position"); double normal=0; for(int n=0;n<3;n++){float x=in.canonicalF32("normal");normal+=x*x;} if(normal<=1e-12) fail(BundleErrorCode.INVALID_SCHEMA,"zero mesh normal"); for(int n=0;n<2;n++)in.canonicalF32("UV"); }
            for(long i=0;i<indices;i++) if(in.u32()>=vertices) fail(BundleErrorCode.INVALID_REFERENCE,"mesh index out of range");
            long next=0; for(long s=0;s<submeshes;s++){long first=in.u32(), count=in.u32(), slot=in.u32(); if(first!=next||count==0||count%3!=0||(next=Math.addExact(next,count))>indices) fail(BundleErrorCode.INVALID_SCHEMA,"invalid submesh ranges"); if(slot>=materialSlots) fail(BundleErrorCode.INVALID_REFERENCE,"mesh material slot out of range");} if(next!=indices) fail(BundleErrorCode.INVALID_SCHEMA,"submeshes do not cover indices"); in.end();
        }
    }

    /**
     * The IHDR already carries everything the bundle claims about a page, so
     * the header check is the whole check. Decoding the image here would cost a
     * full 4096-square inflate per mip per page and learn nothing: the payload
     * bytes are already proven against the manifest hash, and the real decode
     * happens at render time in AtlasPageResidency, which checks the dimensions
     * again before it uploads.
     */
    private static void validatePng(ZipFile zip, ZipEntry entry, int[] expectedDimensions) throws IOException {
        try (Binary in = new Binary(zip.getInputStream(entry), entry.getSize())) {
            in.magic(PNG_SIGNATURE); if(in.u32be()!=13 || !Arrays.equals(in.bytes(4),new byte[]{'I','H','D','R'})) fail(BundleErrorCode.INVALID_SCHEMA,"PNG lacks canonical IHDR");
            long width=in.u32be(),height=in.u32be(); if(width==0||height==0||width>BundleLimits.MAX_OUTPUT_TEXTURE_AXIS||height>BundleLimits.MAX_OUTPUT_TEXTURE_AXIS||width*height*4>BundleLimits.MAX_DECODED_TEXTURE_BYTES) fail(BundleErrorCode.LIMIT_EXCEEDED,"PNG dimensions exceed limits");
            if (width != expectedDimensions[0] || height != expectedDimensions[1]) fail(BundleErrorCode.INVALID_REFERENCE, "PNG dimensions differ from texture metadata");
        }
    }

    private static void contentHash(Map<String,String> hashes,String path,String contentId)throws BundleValidationException{if(!contentId.equals(hashes.get(path)))fail(BundleErrorCode.HASH_MISMATCH,"content-addressed path does not match payload hash: "+path);}

    private static void validateJsonEnvelope(byte[] bytes,String label)throws BundleValidationException{int nesting=0;boolean string=false,escaped=false;for(int i=0;i<bytes.length;i++){int c=bytes[i]&255;if(string){if(escaped)escaped=false;else if(c=='\\')escaped=true;else if(c=='"')string=false;continue;}if(c=='"')string=true;else if(c=='{'||c=='['){if(++nesting>BundleLimits.MAX_JSON_NESTING)fail(BundleErrorCode.LIMIT_EXCEEDED,label+" nesting exceeds limit");}else if(c=='}'||c==']'){if(--nesting<0)fail(BundleErrorCode.INVALID_SCHEMA,"unbalanced "+label);}else if((c==' '||c=='\t'||c=='\r'||c=='\n')&&i!=bytes.length-1)fail(BundleErrorCode.INVALID_SCHEMA,label+" contains non-canonical whitespace");}if(string||nesting!=0)fail(BundleErrorCode.INVALID_SCHEMA,"unterminated "+label);}

    private static JsonObject json(ZipFile zip, ZipEntry entry, String label) throws IOException {
        if (entry.getSize() > Integer.MAX_VALUE) fail(BundleErrorCode.LIMIT_EXCEEDED, label + " is too large for JSON allocation");
        byte[] bytes; try(InputStream input=zip.getInputStream(entry); ByteArrayOutputStream out=new ByteArrayOutputStream((int)entry.getSize())){input.transferTo(out);bytes=out.toByteArray();}
        if(bytes.length==0||bytes[bytes.length-1]!='\n'||bytes.length>=3&&bytes[0]==(byte)0xef&&bytes[1]==(byte)0xbb&&bytes[2]==(byte)0xbf) fail(BundleErrorCode.INVALID_SCHEMA,label+" is not canonical JSON");
        validateJsonEnvelope(bytes, label);
        try { var decoder=StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT); JsonElement root=JsonParser.parseString(decoder.decode(ByteBuffer.wrap(bytes)).toString()); return object(root,label); }
        catch(BundleValidationException e){throw e;} catch(Exception e){throw new BundleValidationException(BundleErrorCode.INVALID_SCHEMA,"invalid JSON: "+label,e);}
    }

    private static ZipEntry required(Map<String,ZipEntry> entries,String path)throws BundleValidationException{ZipEntry e=entries.get(path);if(e==null)fail(BundleErrorCode.MISSING_ENTRY,"missing "+path);return e;}
    private static void format(JsonObject o,String expected,String label)throws BundleValidationException{if(!expected.equals(string(o,"format")))fail(BundleErrorCode.INVALID_SCHEMA,"invalid format: "+label);if(uintIndex(o.get("version"),"version")!=1)fail(BundleErrorCode.UNSUPPORTED_VERSION,"unsupported version: "+label);}
    private static void keys(JsonObject o,String... expected)throws BundleValidationException{if(!o.keySet().equals(Set.of(expected))||!new ArrayList<>(o.keySet()).equals(List.of(expected)))fail(BundleErrorCode.INVALID_SCHEMA,"unexpected or non-canonical JSON fields");}
    private static JsonObject object(JsonElement e,String label)throws BundleValidationException{if(e==null||!e.isJsonObject())fail(BundleErrorCode.INVALID_SCHEMA,label+" must be an object");return e.getAsJsonObject();}
    private static JsonArray array(JsonObject o,String key)throws BundleValidationException{JsonElement e=o.get(key);if(e==null||!e.isJsonArray())fail(BundleErrorCode.INVALID_SCHEMA,key+" must be an array");return e.getAsJsonArray();}
    private static String string(JsonObject o,String key)throws BundleValidationException{JsonElement e=o.get(key);if(e==null||!e.isJsonPrimitive()||!e.getAsJsonPrimitive().isString())fail(BundleErrorCode.INVALID_SCHEMA,key+" must be a string");return e.getAsString();}
    private static String id(String value,String label)throws BundleValidationException{if(value.isEmpty()||!value.chars().allMatch(c->c>='a'&&c<='z'||Character.isDigit(c)||c=='_'||c=='-'))fail(BundleErrorCode.INVALID_SCHEMA,"invalid "+label+" ID");return value;}
    private static String digest(String value,String label)throws BundleValidationException{if(value.length()!=64||!value.chars().allMatch(c->Character.isDigit(c)||c>='a'&&c<='f'))fail(BundleErrorCode.INVALID_SCHEMA,"invalid "+label);return value;}
    private static int dimension(JsonObject o,String key,int max)throws BundleValidationException{int v=uintIndex(o.get(key),key);if(v==0||v>max)fail(BundleErrorCode.LIMIT_EXCEEDED,key+" exceeds limit");return v;}
    private static int uintIndex(JsonElement e,String label)throws BundleValidationException{try{if(e==null||!e.isJsonPrimitive()||!e.getAsJsonPrimitive().isNumber())throw new NumberFormatException();String s=e.getAsString();if(s.isEmpty()||(s.length()>1&&s.charAt(0)=='0')||!s.chars().allMatch(Character::isDigit))throw new NumberFormatException();long v=Long.parseLong(s);if(v>Integer.MAX_VALUE)throw new NumberFormatException();return(int)v;}catch(NumberFormatException x){fail(BundleErrorCode.INVALID_SCHEMA,label+" must be a canonical bounded integer");return 0;}}
    private static double finiteNumber(JsonElement e,String label)throws BundleValidationException{try{if(e==null||!e.isJsonPrimitive()||!e.getAsJsonPrimitive().isNumber())throw new NumberFormatException();double v=Double.parseDouble(e.getAsString());if(!Double.isFinite(v)||Double.doubleToRawLongBits(v)==Double.doubleToRawLongBits(-0.0))throw new NumberFormatException();return v;}catch(NumberFormatException x){fail(BundleErrorCode.INVALID_SCHEMA,"invalid "+label);return 0;}}
    private static int[] vector3i(JsonElement e,String label)throws BundleValidationException{if(e==null||!e.isJsonArray()||e.getAsJsonArray().size()!=3)fail(BundleErrorCode.INVALID_SCHEMA,label+" must have three integers");int[] out=new int[3];for(int i=0;i<3;i++){try{out[i]=e.getAsJsonArray().get(i).getAsInt();}catch(Exception x){fail(BundleErrorCode.INVALID_SCHEMA,"invalid "+label);}}return out;}
    private static String exactPath(JsonObject o,String key,String expected)throws BundleValidationException{String actual=string(o,key);if(!actual.equals(expected))fail(BundleErrorCode.INVALID_REFERENCE,"non-canonical "+key+" path");return actual;}
    private static void limit(long value,long max,String label)throws BundleValidationException{if(value>max)fail(BundleErrorCode.LIMIT_EXCEEDED,label+" exceeds limit");}
    private static int compare(int[]a,int[]b){for(int i=0;i<a.length;i++){int c=Integer.compare(a[i],b[i]);if(c!=0)return c;}return 0;}
    private static int compareUnsigned(long[]a,long[]b){for(int i=0;i<a.length;i++){int c=Long.compareUnsigned(a[i],b[i]);if(c!=0)return c;}return 0;}
    private static int compareUnsignedBytes(byte[]a,byte[]b){for(int i=0;i<Math.min(a.length,b.length);i++){int c=Integer.compare(a[i]&255,b[i]&255);if(c!=0)return c;}return Integer.compare(a.length,b.length);}
    private static int u16(byte[]b,int o){return(b[o]&255)|(b[o+1]&255)<<8;}
    private static long u32(byte[]b,int o){return Integer.toUnsignedLong(ByteBuffer.wrap(b,o,4).order(ByteOrder.LITTLE_ENDIAN).getInt());}
    private static void fail(BundleErrorCode code,String message)throws BundleValidationException{throw new BundleValidationException(code,message);}

    private static int compareModels(BundleModel left, BundleModel right) { int c=left.contentId().compareTo(right.contentId());if(c==0)c=left.sourceModel().compareTo(right.sourceModel());if(c==0){int[] a=left.materialIds(),b=right.materialIds();for(int i=0;i<Math.min(a.length,b.length);i++){c=Integer.compareUnsigned(a[i],b[i]);if(c!=0)return c;}c=Integer.compare(a.length,b.length);}return c; }
    private record FaceOrder(int local,int patch,long material,long uv,int provenance,long primary,long secondary) implements Comparable<FaceOrder>{public int compareTo(FaceOrder o){int c=Integer.compare(local,o.local);if(c==0)c=Integer.compare(patch,o.patch);if(c==0)c=Long.compareUnsigned(material,o.material);if(c==0)c=Long.compareUnsigned(uv,o.uv);if(c==0)c=Integer.compare(provenance,o.provenance);if(c==0)c=Long.compareUnsigned(primary,o.primary);if(c==0)c=Long.compareUnsigned(secondary,o.secondary);return c;}}
    private record DiagnosticOrder(int severity,String code,String message,String context) implements Comparable<DiagnosticOrder>{public int compareTo(DiagnosticOrder o){int c=Integer.compare(severity,o.severity);if(c==0)c=code.compareTo(o.code);if(c==0)c=message.compareTo(o.message);if(c==0)c=context.compareTo(o.context);return c;}}

    private static final class Binary implements AutoCloseable {
        private final InputStream in; private long remaining;
        Binary(InputStream in,long size){this.in=in;this.remaining=size;}
        byte[] bytes(int n)throws IOException{if(n<0||remaining<n)fail(BundleErrorCode.INVALID_SCHEMA,"truncated binary payload");byte[] b=in.readNBytes(n);if(b.length!=n)throw new EOFException();remaining-=n;return b;}
        void skip(int n)throws IOException{bytes(n);} void magic(byte[] expected)throws IOException{if(!Arrays.equals(bytes(expected.length),expected))fail(BundleErrorCode.INVALID_SCHEMA,"invalid binary magic");}
        void version()throws IOException{long v=u32();if(v!=1)fail(BundleErrorCode.UNSUPPORTED_VERSION,"unsupported binary version "+v);}
        int i32()throws IOException{return ByteBuffer.wrap(bytes(4)).order(ByteOrder.LITTLE_ENDIAN).getInt();} long u32()throws IOException{return Integer.toUnsignedLong(i32());}
        int i16()throws IOException{return ByteBuffer.wrap(bytes(2)).order(ByteOrder.LITTLE_ENDIAN).getShort();}
        long u32be()throws IOException{return Integer.toUnsignedLong(ByteBuffer.wrap(bytes(4)).getInt());}
        long count(long max,String label)throws IOException{long v=u32();limit(v,max,label);return v;}
        long canonicalF64Bits(String label)throws IOException{long bits=ByteBuffer.wrap(bytes(8)).order(ByteOrder.LITTLE_ENDIAN).getLong();double v=Double.longBitsToDouble(bits);if(!Double.isFinite(v)||bits==Double.doubleToRawLongBits(-0.0))fail(BundleErrorCode.INVALID_SCHEMA,"non-canonical "+label);return bits;}
        float canonicalF32(String label)throws IOException{int bits=i32();float v=Float.intBitsToFloat(bits);if(!Float.isFinite(v)||bits==Float.floatToRawIntBits(-0.0f))fail(BundleErrorCode.INVALID_SCHEMA,"non-canonical "+label);return v;}
        void requireRemaining(long required,String label)throws BundleValidationException{if(required<0||required>remaining)fail(BundleErrorCode.INVALID_SCHEMA,"invalid "+label+" length");}
        void end()throws BundleValidationException{if(remaining!=0)fail(BundleErrorCode.INVALID_SCHEMA,"trailing binary payload bytes");}
        public void close()throws IOException{in.close();}
    }
}
