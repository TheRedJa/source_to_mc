package dev.theredja.src2mc.client.render;

import static net.minecraft.commands.Commands.literal;

import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexBuffer;
import com.mojang.blaze3d.vertex.VertexFormat;
import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.bundle.AtlasIndex;
import dev.theredja.src2mc.bundle.BundleGeneration;
import dev.theredja.src2mc.bundle.BundleManifest;
import dev.theredja.src2mc.bundle.BundleMap;
import dev.theredja.src2mc.bundle.BundleMaterial;
import dev.theredja.src2mc.bundle.BundleModel;
import dev.theredja.src2mc.bundle.BundleProp;
import dev.theredja.src2mc.bundle.PropVisibility;
import dev.theredja.src2mc.bundle.RuntimeMesh;
import dev.theredja.src2mc.network.PlacementNetwork;
import dev.theredja.src2mc.world.MapPlacement;
import dev.theredja.src2mc.world.Src2mcDataBlockEntity;
import dev.theredja.src2mc.world.Src2mcWorldContent;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.phys.AABB;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.EventPriority;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;
import net.neoforged.neoforge.client.event.RegisterClientCommandsEvent;
import org.joml.Matrix4f;

/**
 * Static prop VBOs derived from loaded prop-root blocks. Geometry is merged
 * into one aggregate batch per map placement, section, atlas page, and render
 * class so the draw count does not scale with prop placements. This
 * deliberately uses neither entities nor block-entity renderers, and never
 * touches Sodium's terrain pipeline.
 */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class PropRenderer {
    private static final int PROP_BUILDS_PER_FRAME = 32;
    private static final int PROP_DECODE_LOOKAHEAD = 256;
    private static final long PROP_BUILD_BUDGET_NANOS = 4_000_000L;
    private static final long PROP_REBUILD_BUDGET_NANOS = 4_000_000L;
    private static final int ROOT_RECHECK_FRAMES = 10;
    private static final long MESH_GRACE_FRAMES = 600;
    private static final RuntimeMeshResidency RUNTIME_MESHES = new RuntimeMeshResidency();
    private static final PropBatchAggregator<AggregateKey, PropKey, Mesh> AGGREGATES = new PropBatchAggregator<>();
    private static final Map<PropKey, PropSource> ROOT_DATA = new HashMap<>();
    private static final Map<PropKey, Set<AggregateKey>> PROP_CONTRIBUTIONS = new HashMap<>();
    private static final Map<PropKey, Boolean> ROOTS = new HashMap<>();
    private static final Map<PropKey, RootStatus> ROOT_STATUS = new HashMap<>();
    private static final Set<PropKey> BUILT_PROPS = new HashSet<>();
    private static final Map<PropKey, Long> LAST_VISIBLE = new HashMap<>();
    private static ClientLevel level;
    private static long generationSequence = -1;
    private static List<MapPlacement> placementSnapshot = List.of();
    private static long frame;
    private static int nearbyProps;
    private static int nearbyUnbuilt;
    private static int buildsLastFrame;
    private static boolean renderingEnabled = true;
    private static final PropRenderPerf PERF = new PropRenderPerf();
    private static final OcclusionCuller<AggregateKey> OCCLUSION = new OcclusionCuller<>();
    private static PropVisibility pvsTable;
    private static MapPlacement pvsPlacement;
    private static byte[] pvsRow;
    private static int pvsCluster = -1;
    private static int pvsCameraX = Integer.MIN_VALUE;
    private static int pvsCameraY = Integer.MIN_VALUE;
    private static int pvsCameraZ = Integer.MIN_VALUE;

    private PropRenderer() {}

    @SubscribeEvent
    public static void registerCommand(RegisterClientCommandsEvent event) {
        event.getDispatcher().register(literal("src2mc_prop_status").executes(context -> {
            for (Component line : statusLines()) context.getSource().sendSuccess(() -> line, false);
            return 1;
        }));
        event.getDispatcher().register(literal("src2mc_prop_toggle").executes(context -> {
            renderingEnabled = !renderingEnabled;
            context.getSource().sendSuccess(() -> Component.literal("src2mc prop rendering "
                + (renderingEnabled ? "enabled" : "disabled — built geometry is kept warm; run again to re-enable")), false);
            return 1;
        }));
        event.getDispatcher().register(literal("src2mc_prop_occlusion_toggle").executes(context -> {
            boolean enabled = OCCLUSION.toggleEnabled();
            context.getSource().sendSuccess(() -> Component.literal("src2mc prop occlusion "
                + (enabled ? "enabled" : "disabled — props remain rendered, PVS and frustum culling remain active")), false);
            return 1;
        }));
        event.getDispatcher().register(literal("src2mc_prop_overlay_toggle").executes(context -> {
            boolean enabled = PropStatusOverlay.toggle();
            context.getSource().sendSuccess(() -> Component.literal("src2mc prop status overlay " + (enabled ? "enabled" : "disabled")), false);
            return 1;
        }));
    }

    static List<Component> statusLines() {
        long active = ROOT_STATUS.values().stream().filter(status -> status == RootStatus.ACTIVE).count();
        long unloaded = ROOT_STATUS.values().stream().filter(status -> status == RootStatus.UNLOADED).count();
        long missing = ROOT_STATUS.values().stream().filter(status -> status == RootStatus.MISSING).count();
        long schema = ROOT_STATUS.values().stream().filter(status -> status == RootStatus.SCHEMA).count();
        long campaign = ROOT_STATUS.values().stream().filter(status -> status == RootStatus.CAMPAIGN).count();
        long map = ROOT_STATUS.values().stream().filter(status -> status == RootStatus.MAP).count();
        long identity = ROOT_STATUS.values().stream().filter(status -> status == RootStatus.IDENTITY).count();
        RuntimeMeshResidency.Stats runtimeMeshes = RUNTIME_MESHES.stats();
        PropRenderPerf.Snapshot perf = PERF.snapshot();
        PropRenderPerf.Frame last = perf.frame();
        PropRenderPerf.Frame average = perf.windowAverage();
        AtlasPageResidency.Stats atlas = MapSurfaceRenderer.atlasPages().stats();
        long propVbo = estimatedVboBytes();
        OcclusionCuller.Stats occlusion = OCCLUSION.stats();
        long invalidRoots = unloaded + missing + schema + campaign + map + identity;
        List<Component> lines = new ArrayList<>();
        lines.add(Component.literal("[src2mc] Prop renderer").withStyle(ChatFormatting.AQUA, ChatFormatting.BOLD));
        lines.add(statusLine("Status", renderingEnabled ? "ON" : "OFF", renderingEnabled ? ChatFormatting.GREEN : ChatFormatting.RED)
            .append(detail("  Roots " + active + "/" + ROOTS.size() + "  Batches " + AGGREGATES.size() + "  Built " + BUILT_PROPS.size())));
        lines.add(statusLine("Render (last frame)", last.drawCalls() + " draws  " + formatMillions(last.triangles()) + " triangles  " + formatMs(last.renderMs()) + " ms", ChatFormatting.YELLOW)
            .append(detail("  PVS " + (pvsRow != null ? "cluster " + pvsCluster + ", " + last.pvsRejected() + " rejected" : "off"))));
        String occlusionState = !occlusion.enabled() ? "OFF" : occlusion.supported() ? "ON" : "UNSUPPORTED";
        ChatFormatting occlusionColor = !occlusion.enabled() ? ChatFormatting.RED : occlusion.supported() ? ChatFormatting.GREEN : ChatFormatting.RED;
        lines.add(statusLine("GPU occlusion", occlusionState, occlusionColor)
            .append(detail("  Last: " + occlusion.last().rejectedDraws() + " draws rejected, " + formatMillions(occlusion.last().rejectedTriangles()) + " triangles saved"
                + "  |  Queries " + occlusion.last().issued() + " issued, " + occlusion.pending() + " pending")));
        lines.add(statusLine("120-frame average", occlusion.average().rejectedDraws() + " occlusion-rejected draws/frame", ChatFormatting.GOLD)
            .append(detail("  " + formatMillions(occlusion.average().rejectedTriangles()) + " triangles/frame saved"
                + "  |  " + average.drawCalls() + " draws, " + formatMillions(average.triangles()) + " triangles, " + formatMs(average.renderMs()) + " ms render")));
        lines.add(statusLine("Memory", "Props " + formatBytes(propVbo) + "  |  Atlas " + formatBytes(atlas.residentVramBytes()), ChatFormatting.LIGHT_PURPLE)
            .append(detail("  Meshes " + runtimeMeshes.ready() + " ready, " + runtimeMeshes.pending() + " loading")));
        if (invalidRoots != 0 || runtimeMeshes.failed() != 0) lines.add(statusLine("Warning", invalidRoots + " unavailable roots, " + runtimeMeshes.failed() + " failed meshes", ChatFormatting.RED)
            .append(detail(" (unloaded " + unloaded + ", missing " + missing + ", schema " + schema + ")")));
        return List.copyOf(lines);
    }

    @SubscribeEvent(priority = EventPriority.LOWEST)
    public static void render(RenderLevelStageEvent event) {
        if (event.getStage() == RenderLevelStageEvent.Stage.AFTER_SOLID_BLOCKS) renderOpaque(event);
        else if (event.getStage() == RenderLevelStageEvent.Stage.AFTER_PARTICLES) renderTranslucent(event);
    }

    private static void renderOpaque(RenderLevelStageEvent event) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null) { clear(); return; }
        BundleGeneration generation = Src2mc.bundles().active();
        List<MapPlacement> placements = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).view();
        if (minecraft.level != level || generation.sequence() != generationSequence || !placements.equals(placementSnapshot)) {
            clear(); level = minecraft.level; generationSequence = generation.sequence(); placementSnapshot = placements;
        }
        frame++;
        PERF.beginFrame();
        MapSurfaceRenderer.atlasPages().pump(frame);
        if (generation.sequence() == 0 || placements.isEmpty()) return;
        long scanStart = System.nanoTime();
        int scannedRoots = updateRoots(generation, minecraft.level, placements);
        if (scannedRoots >= 0) {
            PERF.add(PropRenderPerf.M_ROOT_SCAN_NANOS, System.nanoTime() - scanStart);
            PERF.add(PropRenderPerf.M_ROOT_SCAN_PROPS, scannedRoots);
        }
        Map<PropKey, Map<BatchKey, List<PropTessellator.Triangle>>> tessCache = new HashMap<>();
        long buildStart = System.nanoTime();
        buildVisibleProps(generation, minecraft, placements, tessCache);
        PERF.add(PropRenderPerf.M_BUILD_NANOS, System.nanoTime() - buildStart);
        discardExpiredMeshes();
        rebuildDirtyAggregates(generation, minecraft.gameRenderer.getMainCamera().getPosition(), tessCache);
        resolveCameraPvs(generation, minecraft);
        OCCLUSION.beginFrame(frame);
        if (renderingEnabled) draw(event, generation, false);
    }

    /**
     * Resolves the camera's visibility cluster once per camera block move.
     * Every unresolved case — camera outside any map placement, no visibility
     * table, solid leaf, or unknown cluster — leaves the row null and renders
     * without PVS rejection.
     */
    private static void resolveCameraPvs(BundleGeneration generation, Minecraft minecraft) {
        var camera = minecraft.gameRenderer.getMainCamera().getPosition();
        BlockPos cameraBlock = BlockPos.containing(camera.x, camera.y, camera.z);
        if (cameraBlock.getX() == pvsCameraX && cameraBlock.getY() == pvsCameraY && cameraBlock.getZ() == pvsCameraZ) return;
        pvsCameraX = cameraBlock.getX(); pvsCameraY = cameraBlock.getY(); pvsCameraZ = cameraBlock.getZ();
        pvsTable = null; pvsPlacement = null; pvsRow = null; pvsCluster = -1;
        var placement = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).at(cameraBlock).orElse(null);
        if (placement == null) return;
        var map = generation.findMap(placement.campaignId(), placement.mapId()).orElse(null);
        PropVisibility table = map == null ? null : map.pvs();
        if (table == null) return;
        BlockPos local = placement.toLocal(cameraBlock);
        int cluster = table.clusterAt(local.getX(), local.getY(), local.getZ());
        if (cluster < 0) return;
        byte[] row = table.row(cluster);
        if (row == null) return;
        pvsTable = table; pvsPlacement = placement; pvsRow = row; pvsCluster = cluster;
    }

    private static void renderTranslucent(RenderLevelStageEvent event) {
        if (Minecraft.getInstance().level != null && generationSequence >= 0 && renderingEnabled) draw(event, Src2mc.bundles().active(), true);
    }

    /** @return the number of roots inspected, or -1 when this frame skipped the periodic recheck. */
    private static int updateRoots(BundleGeneration generation, ClientLevel clientLevel, List<MapPlacement> placements) {
        if (frame % ROOT_RECHECK_FRAMES != 1) return -1;
        Set<PropKey> seen = new HashSet<>();
        for (MapPlacement placement : placements) {
            BundleMap map = generation.findMap(placement.campaignId(), placement.mapId()).orElse(null);
            if (map == null) continue;
            for (BundleProp prop : map.props()) {
                PropKey key = new PropKey(placement, prop.stableId()); seen.add(key);
                RootStatus status = rootStatus(clientLevel, placement, map, prop);
                boolean active = status == RootStatus.ACTIVE;
                Boolean wasActive = ROOTS.put(key, active);
                ROOT_STATUS.put(key, status);
                if (wasActive != null && wasActive != active) removeProp(key);
            }
        }
        ROOTS.keySet().removeIf(key -> {
            if (seen.contains(key)) return false;
            removeProp(key); ROOT_STATUS.remove(key); return true;
        });
        return seen.size();
    }

    private static RootStatus rootStatus(ClientLevel clientLevel, MapPlacement placement, BundleMap map, BundleProp prop) {
        int[] root = prop.rootCell();
        BlockPos position = placement.translation().offset(root[0], root[1], root[2]);
        // hasChunkAt avoids forcing a client chunk load merely to render a prop.
        if (!clientLevel.hasChunkAt(position)) return RootStatus.UNLOADED;
        if (!clientLevel.getBlockState(position).is(Src2mcWorldContent.PROP_ROOT.get())) return RootStatus.MISSING;
        BlockEntity blockEntity = clientLevel.getBlockEntity(position);
        if (!(blockEntity instanceof Src2mcDataBlockEntity data)) return RootStatus.SCHEMA;
        CompoundTag tag = data.payload();
        // The immutable, hash-validated bundle table is the authority for the
        // transform and model reference. WorldEdit preserves the identity but
        // may rewrite numeric NBT representation during its Sponge-v3 paste.
        // Requiring an exact re-serialization here would reject an otherwise
        // valid root. Its expected world cell still prevents moved/copied roots
        // from rendering as the original placement.
        if (tag.getInt("schema_version") != 1) return RootStatus.SCHEMA;
        if (!placement.campaignId().equals(tag.getString("campaign_id"))) return RootStatus.CAMPAIGN;
        if (!map.mapId().equals(tag.getString("map_id"))) return RootStatus.MAP;
        return prop.stableId().equals(tag.getString("stable_id")) ? RootStatus.ACTIVE : RootStatus.IDENTITY;
    }

    private static void buildVisibleProps(BundleGeneration generation, Minecraft minecraft, List<MapPlacement> placements,
                                          Map<PropKey, Map<BatchKey, List<PropTessellator.Triangle>>> tessCache) {
        var camera = minecraft.gameRenderer.getMainCamera().getPosition();
        int cameraSectionX = SectionPos.blockToSectionCoord(camera.x), cameraSectionZ = SectionPos.blockToSectionCoord(camera.z);
        int distance = minecraft.options.getEffectiveRenderDistance() + 1;
        List<BuildCandidate> candidates = new ArrayList<>();
        int nearbyBuilt = 0;
        for (MapPlacement placement : placements) {
            var located = generation.findLocatedMap(placement.campaignId(), placement.mapId()).orElse(null);
            if (located == null || located.map().atlas() == null) continue;
            BundleMap map = located.map();
            for (BundleProp prop : map.props()) {
                PropKey key = new PropKey(placement, prop.stableId());
                if (!ROOTS.getOrDefault(key, false)) continue;
                double[] translation = prop.translation();
                int sx = SectionPos.blockToSectionCoord(placement.translation().getX() + translation[0]);
                int sz = SectionPos.blockToSectionCoord(placement.translation().getZ() + translation[2]);
                if (Math.abs(sx - cameraSectionX) > distance || Math.abs(sz - cameraSectionZ) > distance) continue;
                // Residency follows render distance, not the camera frustum. Looking away must not
                // make a nearby prop expire and slowly rebuild when the player turns around.
                if (BUILT_PROPS.contains(key)) { LAST_VISIBLE.put(key, frame); nearbyBuilt++; continue; }
                double x = placement.translation().getX() + translation[0] - camera.x;
                double y = placement.translation().getY() + translation[1] - camera.y;
                double z = placement.translation().getZ() + translation[2] - camera.z;
                candidates.add(new BuildCandidate(new PropSource(located.bundle(), map, placement, prop), x * x + y * y + z * z));
            }
        }
        candidates.sort(Comparator.comparingDouble(BuildCandidate::distanceSquared));
        nearbyProps = nearbyBuilt + candidates.size();
        nearbyUnbuilt = candidates.size();
        long started = System.nanoTime();
        int builds = 0;
        for (int index = 0; index < Math.min(candidates.size(), PROP_DECODE_LOOKAHEAD); index++) {
            BuildCandidate candidate = candidates.get(index);
            if (buildProp(generation, candidate, tessCache)) builds++;
            if (builds >= PROP_BUILDS_PER_FRAME || (builds > 0 && System.nanoTime() - started >= PROP_BUILD_BUDGET_NANOS)) break;
        }
        buildsLastFrame = builds;
        PERF.add(PropRenderPerf.M_BUILDS, builds);
    }

    /**
     * Registers a prop's tessellated contributions on aggregate batches; the
     * affected aggregates are rebuilt by {@link #rebuildDirtyAggregates}.
     *
     * @return true only when this prop's mesh was decoded and the prop is built.
     */
    private static boolean buildProp(BundleGeneration generation, BuildCandidate candidate,
                                     Map<PropKey, Map<BatchKey, List<PropTessellator.Triangle>>> tessCache) {
        PropSource source = candidate.source();
        if (source.prop().modelIndex() < 0 || source.prop().modelIndex() >= source.map().models().size()) return false;
        RuntimeMesh mesh = RUNTIME_MESHES.request(generation.sequence(), source.bundle(), source.map().models().get(source.prop().modelIndex()).contentId()).orElse(null);
        if (mesh == null) return false;
        PropKey propKey = new PropKey(source.placement(), source.prop().stableId());
        Map<BatchKey, List<PropTessellator.Triangle>> triangles = tessellateProp(source, mesh);
        tessCache.put(propKey, triangles);
        ROOT_DATA.put(propKey, source);
        Set<AggregateKey> contributions = PROP_CONTRIBUTIONS.computeIfAbsent(propKey, ignored -> new HashSet<>());
        for (BatchKey batch : triangles.keySet()) {
            AggregateKey key = new AggregateKey(source.placement(), batch.section().x(), batch.section().y(), batch.section().z(), batch.page(), batch.renderClass());
            if (contributions.add(key)) AGGREGATES.register(key, propKey);
        }
        BUILT_PROPS.add(propKey);
        return true;
    }

    /** Tessellates one prop into map-local (section, page, render class) triangle batches. */
    private static Map<BatchKey, List<PropTessellator.Triangle>> tessellateProp(PropSource source, RuntimeMesh mesh) {
        BundleMap map = source.map();
        BundleProp prop = source.prop();
        BundleModel model = map.models().get(prop.modelIndex());
        Map<BatchKey, List<PropTessellator.Triangle>> triangles = new HashMap<>();
        for (RuntimeMesh.Submesh submesh : mesh.submeshes()) {
            if (submesh.materialSlot() < 0 || submesh.materialSlot() >= model.materialSlotCount()) continue;
            int materialId = model.materialIds()[submesh.materialSlot()];
            if (materialId < 0 || materialId >= map.materials().size()) continue;
            BundleMaterial material = map.materials().get(materialId);
            if (!material.textured() || material.renderClass() == BundleMaterial.RenderClass.FALLBACK) continue;
            AtlasIndex.Texture texture = map.atlas().textures().get(material.texture().contentId());
            if (texture == null) continue;
            for (PropTessellator.Triangle triangle : PropTessellator.tessellate(mesh, submesh, prop, material.texture(), texture, map.atlas().pageSize())) {
                for (Section section : coveredSections(triangle)) {
                    for (PropTessellator.Triangle clipped : PropTessellator.clipSection(triangle, section.x << 4, section.y << 4, section.z << 4)) {
                        triangles.computeIfAbsent(new BatchKey(section, clipped.page(), material.renderClass()), ignored -> new ArrayList<>()).add(clipped);
                    }
                }
            }
        }
        return triangles;
    }

    /** Rebuilds dirty aggregate batches nearest-first within the frame time budget. */
    private static void rebuildDirtyAggregates(BundleGeneration generation, net.minecraft.world.phys.Vec3 camera,
                                               Map<PropKey, Map<BatchKey, List<PropTessellator.Triangle>>> tessCache) {
        long started = System.nanoTime();
        List<RebuildCandidate> ordered = new ArrayList<>();
        for (AggregateKey key : AGGREGATES.dirtyKeys()) {
            BlockPos center = key.placement().translation().offset((key.sectionX() << 4) + 8, (key.sectionY() << 4) + 8, (key.sectionZ() << 4) + 8);
            double dx = center.getX() - camera.x, dy = center.getY() - camera.y, dz = center.getZ() - camera.z;
            ordered.add(new RebuildCandidate(key, dx * dx + dy * dy + dz * dz));
        }
        if (ordered.isEmpty()) return;
        ordered.sort(Comparator.comparingDouble(RebuildCandidate::distanceSquared));
        int rebuilt = 0;
        for (RebuildCandidate candidate : ordered) {
            if (rebuilt > 0 && System.nanoTime() - started >= PROP_REBUILD_BUDGET_NANOS) break;
            rebuildAggregate(generation, candidate.key(), tessCache);
            rebuilt++;
        }
        PERF.add(PropRenderPerf.M_REBUILT_AGGREGATES, rebuilt);
        PERF.add(PropRenderPerf.M_DIRTY_REMAINING, AGGREGATES.dirtyCount());
        PERF.add(PropRenderPerf.M_REBUILD_NANOS, System.nanoTime() - started);
    }

    private static void rebuildAggregate(BundleGeneration generation, AggregateKey key,
                                         Map<PropKey, Map<BatchKey, List<PropTessellator.Triangle>>> tessCache) {
        List<PropTessellator.Triangle> merged = new ArrayList<>();
        BundleManifest bundle = null;
        AtlasIndex atlas = null;
        for (PropKey contributor : AGGREGATES.contributors(key)) {
            PropSource source = ROOT_DATA.get(contributor);
            if (source == null) continue;
            Map<BatchKey, List<PropTessellator.Triangle>> tessellated = tessellateCached(generation, contributor, source, tessCache);
            merged.addAll(tessellated.getOrDefault(new BatchKey(new Section(key.sectionX(), key.sectionY(), key.sectionZ()), key.page(), key.renderClass()), List.of()));
            if (bundle == null) { bundle = source.bundle(); atlas = source.map().atlas(); }
        }
        Mesh old = AGGREGATES.value(key);
        OCCLUSION.invalidate(key);
        if (old != null) old.close();
        Mesh mesh = bundle == null || merged.isEmpty()
            ? null
            : upload(bundle, atlas, key.placement(), key.sectionX(), key.sectionY(), key.sectionZ(), merged);
        AGGREGATES.rebuildComplete(key, mesh);
    }

    /** @return the prop's per-batch triangles, re-tessellating from the cached decoded mesh when needed. */
    private static Map<BatchKey, List<PropTessellator.Triangle>> tessellateCached(BundleGeneration generation, PropKey propKey,
                                                                                  PropSource source,
                                                                                  Map<PropKey, Map<BatchKey, List<PropTessellator.Triangle>>> tessCache) {
        Map<BatchKey, List<PropTessellator.Triangle>> cached = tessCache.get(propKey);
        if (cached != null) return cached;
        RuntimeMesh mesh = null;
        if (source.prop().modelIndex() >= 0 && source.prop().modelIndex() < source.map().models().size()) {
            // Decoded meshes are cached per generation with no eviction, so a prop built once
            // can always be re-tessellated for aggregate rebuilds.
            mesh = RUNTIME_MESHES.request(generation.sequence(), source.bundle(), source.map().models().get(source.prop().modelIndex()).contentId()).orElse(null);
        }
        Map<BatchKey, List<PropTessellator.Triangle>> triangles = mesh == null ? Map.of() : tessellateProp(source, mesh);
        tessCache.put(propKey, triangles);
        return triangles;
    }

    private static List<Section> coveredSections(PropTessellator.Triangle triangle) {
        double minX = Math.min(triangle.a().x(), Math.min(triangle.b().x(), triangle.c().x()));
        double minY = Math.min(triangle.a().y(), Math.min(triangle.b().y(), triangle.c().y()));
        double minZ = Math.min(triangle.a().z(), Math.min(triangle.b().z(), triangle.c().z()));
        double maxX = Math.max(triangle.a().x(), Math.max(triangle.b().x(), triangle.c().x()));
        double maxY = Math.max(triangle.a().y(), Math.max(triangle.b().y(), triangle.c().y()));
        double maxZ = Math.max(triangle.a().z(), Math.max(triangle.b().z(), triangle.c().z()));
        int fromX = floorSection(minX), fromY = floorSection(minY), fromZ = floorSection(minZ);
        int toX = floorSection(maxX - 1.0e-8), toY = floorSection(maxY - 1.0e-8), toZ = floorSection(maxZ - 1.0e-8);
        List<Section> result = new ArrayList<>();
        for (int x = fromX; x <= toX; x++) for (int y = fromY; y <= toY; y++) for (int z = fromZ; z <= toZ; z++) result.add(new Section(x, y, z));
        return result;
    }

    private static int floorSection(double coordinate) { return (int) Math.floor(coordinate / 16.0); }

    private static Mesh upload(BundleManifest bundle, AtlasIndex atlas, MapPlacement placement, int sectionX, int sectionY, int sectionZ, List<PropTessellator.Triangle> triangles) {
        int baseX = sectionX << 4, baseY = sectionY << 4, baseZ = sectionZ << 4;
        int capacity = (int) Math.min(Integer.MAX_VALUE, Math.max(4096L, (long) triangles.size() * 3 * 36));
        try (var bytes = new ByteBufferBuilder(capacity)) {
            var builder = new BufferBuilder(bytes, VertexFormat.Mode.TRIANGLES, DefaultVertexFormat.NEW_ENTITY);
            for (PropTessellator.Triangle triangle : triangles) { vertex(builder, triangle.a(), baseX, baseY, baseZ); vertex(builder, triangle.b(), baseX, baseY, baseZ); vertex(builder, triangle.c(), baseX, baseY, baseZ); }
            try (var data = builder.buildOrThrow()) {
                var buffer = new VertexBuffer(VertexBuffer.Usage.STATIC);
                buffer.bind(); buffer.upload(data); VertexBuffer.unbind();
                BlockPos origin = placement.translation().offset(baseX, baseY, baseZ);
                long estimatedVboBytes = (long) triangles.size() * 3L * 36L;
                AABB bounds = null;
                for (PropTessellator.Triangle triangle : triangles) for (PropTessellator.Vertex vertex : List.of(triangle.a(), triangle.b(), triangle.c())) {
                    AABB point = new AABB(vertex.x(), vertex.y(), vertex.z(), vertex.x(), vertex.y(), vertex.z());
                    bounds = bounds == null ? point : bounds.minmax(point);
                }
                return new Mesh(bundle, atlas, buffer, origin, bounds.move(placement.translation()).inflate(0.01), estimatedVboBytes, triangles.size());
            }
        }
    }

    private static void vertex(BufferBuilder builder, PropTessellator.Vertex vertex, int baseX, int baseY, int baseZ) {
        builder.addVertex((float) (vertex.x() - baseX), (float) (vertex.y() - baseY), (float) (vertex.z() - baseZ))
            .setColor(255, 255, 255, 255).setUv((float) vertex.u(), (float) vertex.v()).setOverlay(OverlayTexture.NO_OVERLAY)
            .setLight(LightTexture.FULL_BRIGHT).setNormal((float) vertex.nx(), (float) vertex.ny(), (float) vertex.nz());
    }

    private static void draw(RenderLevelStageEvent event, BundleGeneration generation, boolean translucent) {
        long started = System.nanoTime();
        var camera = event.getCamera().getPosition();
        List<Map.Entry<AggregateKey, Mesh>> visible = new ArrayList<>();
        List<OcclusionCuller.Candidate<AggregateKey>> queryCandidates = new ArrayList<>();
        OcclusionCuller.CameraView view = new OcclusionCuller.CameraView(camera, event.getCamera().getXRot(), event.getCamera().getYRot());
        for (AggregateKey key : AGGREGATES.keys()) {
            Mesh mesh = AGGREGATES.value(key);
            if (mesh == null || (key.renderClass() == BundleMaterial.RenderClass.TRANSLUCENT) != translucent) continue;
            if (pvsRow != null && key.placement().equals(pvsPlacement)) {
                short[] clusters = pvsTable.sectionClusters(key.sectionX(), key.sectionY(), key.sectionZ());
                if (!pvsTable.visible(pvsRow, clusters)) { PERF.add(PropRenderPerf.M_PVS_REJECTED, 1); continue; }
            }
            PERF.add(PropRenderPerf.M_FRUSTUM_TESTS, 1);
            if (!event.getFrustum().isVisible(mesh.bounds)) continue;
            queryCandidates.add(new OcclusionCuller.Candidate<>(key, mesh.bounds, mesh.triangles));
            if (OCCLUSION.shouldCull(key, mesh.bounds, view, mesh.triangles)) continue;
            visible.add(Map.entry(key, mesh));
        }
        OCCLUSION.issue(queryCandidates, view, event.getModelViewMatrix(), event.getProjectionMatrix());
        if (translucent) visible.sort(Comparator.<Map.Entry<AggregateKey, Mesh>>comparingDouble(item -> -distanceSquared(item.getValue().bounds, camera)));
        else visible.sort(Comparator.<Map.Entry<AggregateKey, Mesh>>comparingInt(item -> item.getKey().renderClass().ordinal())
            .thenComparingInt(item -> item.getKey().page()));
        int drawCalls = 0;
        long triangles = 0;
        RenderType activeType = null;
        BundleMaterial.RenderClass activeClass = null;
        int activePage = -1;
        String activeFingerprint = null;
        AtlasIndex activeAtlas = null;
        for (var item : visible) {
            AggregateKey key = item.getKey();
            Mesh mesh = item.getValue();
            // Consecutive aggregates sharing bundle, atlas, page, and class reuse one render state.
            if (activeType == null || key.renderClass() != activeClass || key.page() != activePage
                || !activeFingerprint.equals(mesh.bundle.fingerprint()) || activeAtlas != mesh.atlas) {
                if (activeType != null) activeType.clearRenderState();
                var texture = MapSurfaceRenderer.atlasPages().request(generation.sequence(), mesh.bundle, mesh.atlas, key.page(), frame).orElseGet(MapSurfaceRenderer.atlasPages()::placeholderTexture);
                activeType = translucent ? RenderType.entityTranslucent(texture) : key.renderClass() == BundleMaterial.RenderClass.SOLID ? RenderType.entitySolid(texture) : RenderType.entityCutout(texture);
                activeType.setupRenderState();
                activeClass = key.renderClass();
                activePage = key.page();
                activeFingerprint = mesh.bundle.fingerprint();
                activeAtlas = mesh.atlas;
                PERF.add(PropRenderPerf.M_STATE_SWITCHES, 1);
            }
            Matrix4f modelView = new Matrix4f(event.getModelViewMatrix()).translate((float) (mesh.origin.getX() - camera.x), (float) (mesh.origin.getY() - camera.y), (float) (mesh.origin.getZ() - camera.z));
            mesh.buffer.bind(); mesh.buffer.drawWithShader(modelView, event.getProjectionMatrix(), translucent ? GameRenderer.getRendertypeEntityTranslucentShader() : key.renderClass() == BundleMaterial.RenderClass.SOLID ? GameRenderer.getRendertypeEntitySolidShader() : GameRenderer.getRendertypeEntityCutoutShader());
            drawCalls++;
            triangles += mesh.triangles;
        }
        if (activeType != null) activeType.clearRenderState();
        VertexBuffer.unbind();
        PERF.add(PropRenderPerf.M_VISIBLE_AGGREGATES, drawCalls);
        PERF.add(PropRenderPerf.M_DRAW_CALLS, drawCalls);
        if (translucent) PERF.add(PropRenderPerf.M_DRAW_CALLS_TRANSLUCENT, drawCalls);
        PERF.add(PropRenderPerf.M_TRIANGLES, triangles);
        PERF.add(PropRenderPerf.M_RENDER_NANOS, System.nanoTime() - started);
    }

    private static double distanceSquared(AABB bounds, net.minecraft.world.phys.Vec3 camera) { double x = bounds.getCenter().x - camera.x, y = bounds.getCenter().y - camera.y, z = bounds.getCenter().z - camera.z; return x * x + y * y + z * z; }
    private static long estimatedVboBytes() { return AGGREGATES.values().stream().mapToLong(mesh -> mesh.estimatedVboBytes).sum(); }
    private static String formatBytes(long bytes) { return String.format(java.util.Locale.ROOT, "%.1f MiB", bytes / (1024.0 * 1024.0)); }
    private static String formatMs(double millis) { return String.format(java.util.Locale.ROOT, "%.2f ms", millis); }
    private static String formatMillions(long value) { return String.format(java.util.Locale.ROOT, "%.2fM", value / 1_000_000.0); }
    private static MutableComponent statusLine(String label, String value, ChatFormatting valueColor) {
        return Component.literal(label + ": ").withStyle(ChatFormatting.DARK_GRAY)
            .append(Component.literal(value).withStyle(valueColor));
    }
    private static Component detail(String text) { return Component.literal(text).withStyle(ChatFormatting.GRAY); }
    /** @return the number of aggregate batches newly dirtied by removing this prop. */
    private static int removeProp(PropKey key) {
        int dirtied = 0;
        Set<AggregateKey> contributions = PROP_CONTRIBUTIONS.remove(key);
        if (contributions != null) for (AggregateKey aggregate : contributions) {
            OCCLUSION.invalidate(aggregate);
            if (AGGREGATES.unregister(aggregate, key)) dirtied++;
        }
        ROOT_DATA.remove(key);
        BUILT_PROPS.remove(key); LAST_VISIBLE.remove(key);
        return dirtied;
    }
    private static void discardExpiredMeshes() {
        List<PropKey> expired = LAST_VISIBLE.entrySet().stream().filter(item -> frame - item.getValue() > MESH_GRACE_FRAMES).map(Map.Entry::getKey).toList();
        if (expired.isEmpty()) return;
        int dirtied = 0;
        for (PropKey key : expired) dirtied += removeProp(key);
        PERF.add(PropRenderPerf.M_EVICTED_PROPS, expired.size());
        PERF.add(PropRenderPerf.M_EVICTED_AGGREGATES, dirtied);
    }
    private static void clear() {
        AGGREGATES.values().forEach(Mesh::close); AGGREGATES.clear();
        PROP_CONTRIBUTIONS.clear(); ROOT_DATA.clear();
        ROOTS.clear(); ROOT_STATUS.clear(); BUILT_PROPS.clear(); LAST_VISIBLE.clear(); PERF.reset(); OCCLUSION.close();
        pvsTable = null; pvsPlacement = null; pvsRow = null; pvsCluster = -1;
        pvsCameraX = Integer.MIN_VALUE; pvsCameraY = Integer.MIN_VALUE; pvsCameraZ = Integer.MIN_VALUE;
        level = null; generationSequence = -1; placementSnapshot = List.of(); frame = 0; nearbyProps = 0; nearbyUnbuilt = 0; buildsLastFrame = 0;
    }

    private record PropKey(MapPlacement placement, String stableId) {}
    private record PropSource(BundleManifest bundle, BundleMap map, MapPlacement placement, BundleProp prop) {}
    private record Section(int x, int y, int z) {}
    private record BatchKey(Section section, int page, BundleMaterial.RenderClass renderClass) {}
    private record AggregateKey(MapPlacement placement, int sectionX, int sectionY, int sectionZ, int page, BundleMaterial.RenderClass renderClass) {}
    private record BuildCandidate(PropSource source, double distanceSquared) {}
    private record RebuildCandidate(AggregateKey key, double distanceSquared) {}
    private enum RootStatus { ACTIVE, UNLOADED, MISSING, SCHEMA, CAMPAIGN, MAP, IDENTITY }
    private static final class Mesh implements AutoCloseable {
        final BundleManifest bundle; final AtlasIndex atlas; final VertexBuffer buffer; final BlockPos origin; final AABB bounds; final long estimatedVboBytes; final long triangles;
        Mesh(BundleManifest bundle, AtlasIndex atlas, VertexBuffer buffer, BlockPos origin, AABB bounds, long estimatedVboBytes, long triangles) { this.bundle = bundle; this.atlas = atlas; this.buffer = buffer; this.origin = origin; this.bounds = bounds; this.estimatedVboBytes = estimatedVboBytes; this.triangles = triangles; }
        @Override public void close() { buffer.close(); }
    }
}
