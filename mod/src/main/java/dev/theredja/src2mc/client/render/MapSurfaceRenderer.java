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
import dev.theredja.src2mc.bundle.BundleMaterial;
import dev.theredja.src2mc.bundle.BundleMap;
import dev.theredja.src2mc.bundle.SurfaceTable;
import dev.theredja.src2mc.network.PlacementNetwork;
import dev.theredja.src2mc.world.MapPlacement;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.SectionPos;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.network.chat.Component;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.HitResult;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.EventPriority;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;
import net.neoforged.neoforge.client.event.RegisterClientCommandsEvent;
import org.joml.Matrix4f;

/** Mod-owned static section/page renderer; independent of Sodium terrain internals. */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class MapSurfaceRenderer {
    private static final int REGION_SECTIONS = 4;
    private static final long LOAD_BUILD_BUDGET_NANOS = 150_000_000L;
    private static final long STEADY_BUILD_BUDGET_NANOS = 4_000_000L;
    private static final long MESH_GRACE_FRAMES = 600;
    private static final AtlasPageResidency PAGES = new AtlasPageResidency();
    private static final Map<MeshKey, Mesh> MESHES = new LinkedHashMap<>();
    private static final Map<RegionKey, Long> BUILT_REGIONS = new HashMap<>();
    /** Identity-keyed: {@link BundleMap#hashCode()}/{@code equals} deep-hash the whole surface
     * table, so a regular HashMap would redo that work on every lookup; the same instance is
     * reused for a generation's lifetime, so identity is both correct and cheap. */
    private static final Map<BundleMap, Map<RegionCoord, RegionGroup>> REGION_GROUPS = new java.util.IdentityHashMap<>();
    private static ClientLevel level;
    private static long generationSequence = -1;
    private static List<MapPlacement> placementSnapshot = List.of();
    private static long frame;
    private static long pvsRejectedRegions;
    private static boolean stillLoading = true;
    /** Set once a full build pass completes with nothing skipped; gates the large load budget so
     * a later relight (which always skips something at least once) never re-triggers it. */
    private static boolean firstPassComplete;
    private static long relightsQueued;
    private static long lastBuildNanos;
    private static long worstBuildNanos;
    private static long shadowPassCallsSinceMainPass;
    private static long shadowPassCallsLastFrame;
    /** Which stage the opaque draw runs in. Switchable at runtime because Iris picks a shaderpack
     * program from the rendering phase it is in, so the stage a mod draws from decides which
     * gbuffers program its geometry lands in — and the wrong one produces geometry that is drawn
     * but contributes nothing usable to the deferred pass. */
    private static RenderLevelStageEvent.Stage opaqueStage = RenderLevelStageEvent.Stage.AFTER_SOLID_BLOCKS;
    private static final java.util.Set<String> SHADOW_STAGES_SEEN = new java.util.LinkedHashSet<>();
    // One light per vertex rather than one per face. Kept as a toggle because
    // lighting is judged by looking at it, and the flat build is the only
    // honest comparison.
    private static boolean smoothLighting = true;
    private static boolean pvsCulling = true;
    private static boolean frustumCulling = true;
    /** Shadow-caster cutoff in blocks. The shaderpack's own shadow-distance setting only culls
     * Sodium's terrain, never mod-owned geometry, so without this the whole map is rasterized into
     * the shadow map every frame. */
    private static double shadowDistance = 75.0;

    /** Region builds in flight, oldest first; bounded so the accumulated triangle lists of
     * half-built regions cannot pile up. */
    private static final Map<RegionKey, PendingBuild> PENDING_BUILDS = new LinkedHashMap<>();
    private static final int MAX_PENDING_BUILDS = 4;

    private static final class PendingBuild {
        final BundleManifest bundle;
        final BundleMap map;
        final MapPlacement placement;
        final RegionGroup group;
        final Map<PageClass, List<LitTriangle>> triangles = new HashMap<>();
        final Map<Long, Integer> lightCache = new HashMap<>();
        int nextSection;

        PendingBuild(BundleManifest bundle, BundleMap map, MapPlacement placement, RegionGroup group) {
            this.bundle = bundle; this.map = map; this.placement = placement; this.group = group;
        }
    }

    private MapSurfaceRenderer() {}

    /** Shared campaign atlas residency for every mod-owned static renderer. */
    static AtlasPageResidency atlasPages() {
        return PAGES;
    }

    @SubscribeEvent
    public static void registerCommand(RegisterClientCommandsEvent event) {
        event.getDispatcher().register(literal("src2mc_debug_face").executes(context -> inspectFace(context.getSource())));
        event.getDispatcher().register(literal("src2mc_render_status").executes(context -> renderStatus(context.getSource())));
        event.getDispatcher().register(literal("src2mc_relight")
            .then(literal("on").executes(context -> setRelight(context.getSource(), true)))
            .then(literal("off").executes(context -> setRelight(context.getSource(), false))));
        var stageCommand = literal("src2mc_render_stage");
        for (RenderLevelStageEvent.Stage stage : List.of(
            RenderLevelStageEvent.Stage.AFTER_SOLID_BLOCKS,
            RenderLevelStageEvent.Stage.AFTER_CUTOUT_BLOCKS,
            RenderLevelStageEvent.Stage.AFTER_ENTITIES,
            RenderLevelStageEvent.Stage.AFTER_BLOCK_ENTITIES,
            RenderLevelStageEvent.Stage.AFTER_TRANSLUCENT_BLOCKS)) {
            stageCommand.then(literal(stage.toString()).executes(context -> setOpaqueStage(context.getSource(), stage)));
        }
        event.getDispatcher().register(stageCommand);
        event.getDispatcher().register(literal("src2mc_shadow_distance")
            .then(net.minecraft.commands.Commands.argument("blocks", com.mojang.brigadier.arguments.DoubleArgumentType.doubleArg(0))
                .executes(context -> setShadowDistance(context.getSource(),
                    com.mojang.brigadier.arguments.DoubleArgumentType.getDouble(context, "blocks")))));
        event.getDispatcher().register(literal("src2mc_smooth_light")
            .then(literal("on").executes(context -> setSmoothLighting(context.getSource(), true)))
            .then(literal("off").executes(context -> setSmoothLighting(context.getSource(), false))));
        event.getDispatcher().register(literal("src2mc_cull")
            .then(literal("pvs").then(literal("on").executes(context -> setPvsCulling(context.getSource(), true)))
                .then(literal("off").executes(context -> setPvsCulling(context.getSource(), false))))
            .then(literal("frustum").then(literal("on").executes(context -> setFrustumCulling(context.getSource(), true)))
                .then(literal("off").executes(context -> setFrustumCulling(context.getSource(), false)))));
    }

    private static int setShadowDistance(net.minecraft.commands.CommandSourceStack source, double blocks) {
        shadowDistance = blocks;
        source.sendSuccess(() -> Component.literal("src2mc shadow caster distance = "
            + (blocks <= 0 ? "unlimited" : blocks + " blocks")), false);
        return 1;
    }

    /** Drops every built mesh, since the light is baked into them at build time. */
    private static int setSmoothLighting(net.minecraft.commands.CommandSourceStack source, boolean value) {
        smoothLighting = value;
        MESHES.values().forEach(Mesh::close);
        MESHES.clear();
        BUILT_REGIONS.clear();
        PENDING_BUILDS.clear();
        PropRenderer.invalidateAllLight();
        source.sendSuccess(() -> Component.literal("src2mc smooth lighting " + (value ? "on" : "off")), false);
        return 1;
    }

    private static int setPvsCulling(net.minecraft.commands.CommandSourceStack source, boolean value) {
        pvsCulling = value;
        source.sendSuccess(() -> Component.literal("src2mc PVS culling " + (value ? "on" : "off")), false);
        return 1;
    }

    private static int setFrustumCulling(net.minecraft.commands.CommandSourceStack source, boolean value) {
        frustumCulling = value;
        source.sendSuccess(() -> Component.literal("src2mc frustum culling " + (value ? "on" : "off")), false);
        return 1;
    }

    private static int setOpaqueStage(net.minecraft.commands.CommandSourceStack source, RenderLevelStageEvent.Stage stage) {
        opaqueStage = stage;
        source.sendSuccess(() -> Component.literal("src2mc opaque draw stage = " + stage), false);
        return 1;
    }

    private static int setRelight(net.minecraft.commands.CommandSourceStack source, boolean value) {
        LightWatcher.setEnabled(value);
        relightsQueued = 0;
        worstBuildNanos = 0;
        source.sendSuccess(() -> Component.literal("src2mc relight " + (value ? "on" : "off")), false);
        return 1;
    }

    private static int renderStatus(net.minecraft.commands.CommandSourceStack source) {
        AtlasPageResidency.Stats stats = PAGES.stats();
        source.sendSuccess(() -> Component.literal("src2mc render: smooth-light " + (smoothLighting ? "on" : "off")
            + ", regions=" + BUILT_REGIONS.size()
            + ", meshes=" + MESHES.size() + ", atlas resident=" + stats.residentPages() + "/" + stats.trackedPages()
            + ", vram=" + formatBytes(stats.residentVramBytes()) + ", decode-pending=" + formatBytes(stats.pendingRamBytes())
            + ", requests=" + stats.requests() + " (hit=" + stats.hits() + ", miss=" + stats.misses()
            + ", denied=" + stats.denied() + ", evicted=" + stats.evictions() + ", failed=" + stats.decodeFailures() + ")"
            + ", PVS " + (CameraVisibility.row() != null ? "cluster " + CameraVisibility.cluster() + ", " + pvsRejectedRegions + " rejected" : "off")
            + (stillLoading ? ", loading" : "")
            + ", shaderpack=" + (IrisCompat.shaderPackInUse() ? "on" : "off")
            + ", shadow-pass invocations/frame=" + shadowPassCallsLastFrame + " (stages " + SHADOW_STAGES_SEEN + ")"
            + ", opaque stage=" + opaqueStage + ", shadow distance=" + shadowDistance
            + ", relight " + (LightWatcher.enabled() ? "on" : "off")
            + ": watched=" + LightWatcher.watchedSections() + ", checks/tick=" + LightWatcher.checksLastTick()
            + ", invalidated/tick=" + LightWatcher.invalidatedLastTick() + ", queued=" + relightsQueued
            + ", in-flight=" + PENDING_BUILDS.size()
            + ", build slice last=" + formatMillis(lastBuildNanos) + " worst=" + formatMillis(worstBuildNanos)), false);
        return 1;
    }

    private static int inspectFace(net.minecraft.commands.CommandSourceStack source) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null || minecraft.hitResult == null
            || minecraft.hitResult.getType() != HitResult.Type.BLOCK) {
            source.sendFailure(Component.literal("Look directly at a src2mc surface block first"));
            return 0;
        }
        BlockHitResult hit = (BlockHitResult) minecraft.hitResult;
        BlockPos world = hit.getBlockPos();
        var placement = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).at(world).orElse(null);
        if (placement == null) {
            source.sendFailure(Component.literal("No unambiguous src2mc placement contains " + world.toShortString()));
            return 0;
        }
        BundleMap map = Src2mc.bundles().active().findMap(placement.campaignId(), placement.mapId()).orElse(null);
        if (map == null) {
            source.sendFailure(Component.literal("The placement's map is absent from the active generation"));
            return 0;
        }
        BlockPos local = placement.toLocal(world);
        List<SurfaceTable.Face> faces = map.surfaces().facesAt(local.getX(), local.getY(), local.getZ());
        source.sendSuccess(() -> Component.literal("src2mc face debug: world=" + world.toShortString()
            + ", local=" + local.toShortString() + ", hit=" + hit.getDirection() + ", records=" + faces.size()), false);
        for (SurfaceTable.Face face : faces) {
            BundleMaterial material = map.materials().get(face.materialId());
            String texture = material.texture() == null ? "none" : material.texture().contentId().substring(0, 12);
            SurfaceTable.UvRegion region = map.surfaces().uvRegions().get(face.uvRegionId());
            double[] uv = region.values();
            String projection = String.format(java.util.Locale.ROOT,
                "uv-rate=[%.4f,%.4f] output=%s",
                Math.sqrt(uv[0] * uv[0] + uv[1] * uv[1] + uv[2] * uv[2]),
                Math.sqrt(uv[4] * uv[4] + uv[5] * uv[5] + uv[6] * uv[6]),
                material.texture() == null ? "none" : material.texture().originalWidth() + "x" + material.texture().originalHeight()
                    + "->" + material.texture().outputWidth() + "x" + material.texture().outputHeight());
            source.sendSuccess(() -> Component.literal("  patch=" + face.patch() + " dir=" + patchDirection(face.patch())
                + " plane=" + patchPlane(face.patch())
                + " material=" + face.materialId() + " " + material.sourceMaterial()
                + " class=" + material.renderClass() + " texture=" + texture
                + " source=" + face.provenance() + ":" + Long.toUnsignedString(face.sourcePrimary())
                + ":" + Long.toUnsignedString(face.sourceSecondary()) + " " + projection), false);
        }
        return 1;
    }

    private static String patchDirection(int patch) {
        return switch (patch & 7) {
            case 0 -> "down"; case 1 -> "up"; case 2 -> "north";
            case 3 -> "south"; case 4 -> "west"; case 5 -> "east"; default -> "invalid";
        };
    }

    private static String patchPlane(int patch) {
        return String.format(java.util.Locale.ROOT, "%.1f", ((patch >>> 3) & 3) * 0.5);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void render(RenderLevelStageEvent event) {
        boolean shadowPass = IrisCompat.renderingShadowPass();
        if (shadowPass) SHADOW_STAGES_SEEN.add(event.getStage().toString());
        if (event.getStage() == opaqueStage) {
            if (shadowPass) drawShadowPass(event, false); else renderOpaque(event);
        } else if (event.getStage() == RenderLevelStageEvent.Stage.AFTER_PARTICLES) {
            // Translucent geometry is skipped in the shadow pass; it would cast an opaque shadow.
            if (!shadowPass) renderTranslucent(event);
        }
    }

    static RenderLevelStageEvent.Stage opaqueStage() { return opaqueStage; }

    static boolean pvsCulling() { return pvsCulling; }

    static boolean smoothLighting() { return smoothLighting; }

    static boolean frustumCulling() { return frustumCulling; }

    /** True when {@code bounds} is close enough to the camera to be worth casting a shadow. */
    static boolean withinShadowDistance(AABB bounds, net.minecraft.world.phys.Vec3 camera) {
        if (shadowDistance <= 0) return true;
        double dx = Math.max(0, Math.max(bounds.minX - camera.x, camera.x - bounds.maxX));
        double dy = Math.max(0, Math.max(bounds.minY - camera.y, camera.y - bounds.maxY));
        double dz = Math.max(0, Math.max(bounds.minZ - camera.z, camera.z - bounds.maxZ));
        return dx * dx + dy * dy + dz * dz <= shadowDistance * shadowDistance;
    }

    /** Shadow pass: draw only, from whatever the main pass already built, frustum-tested against
     * the sun's frustum. No bookkeeping — advancing {@code frame}, building regions, evicting
     * meshes, or resolving PVS all assume a player camera, which this is not. */
    private static void drawShadowPass(RenderLevelStageEvent event, boolean translucent) {
        shadowPassCallsSinceMainPass++;
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null || generationSequence < 0) return;
        draw(event, Src2mc.bundles().active(), translucent, true);
    }

    private static void renderOpaque(RenderLevelStageEvent event) {
        shadowPassCallsLastFrame = shadowPassCallsSinceMainPass;
        shadowPassCallsSinceMainPass = 0;
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null) { clear(); return; }
        BundleGeneration generation = Src2mc.bundles().active();
        List<MapPlacement> placements = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).view();
        if (minecraft.level != level || generation.sequence() != generationSequence || !placements.equals(placementSnapshot)) {
            clear();
            level = minecraft.level;
            generationSequence = generation.sequence();
            placementSnapshot = placements;
        }
        frame++;
        PAGES.pump(frame);
        invalidateReadyPages(PAGES.drainReadyPages());
        if (generation.sequence() == 0 || placements.isEmpty()) return;

        CameraVisibility.resolve(generation, minecraft);
        pvsRejectedRegions = 0;
        var camera = event.getCamera().getPosition();
        int cameraSectionX = SectionPos.blockToSectionCoord(camera.x);
        int cameraSectionZ = SectionPos.blockToSectionCoord(camera.z);
        int distance = minecraft.options.getEffectiveRenderDistance() + 1;
        long buildBudget = stillLoading && !firstPassComplete ? LOAD_BUILD_BUDGET_NANOS : STEADY_BUILD_BUDGET_NANOS;
        long buildDeadline = System.nanoTime() + buildBudget;
        boolean skippedBuild = false;
        for (MapPlacement placement : placements) {
            var located = generation.findLocatedMap(placement.campaignId(), placement.mapId()).orElse(null);
            if (located == null || located.map().atlas() == null) continue;
            BundleMap map = located.map();
            for (var groupEntry : regionGroups(map).entrySet()) {
                RegionCoord coord = groupEntry.getKey();
                RegionGroup group = groupEntry.getValue();
                AABB bounds = regionBounds(placement, group);
                int regionX = SectionPos.blockToSectionCoord((bounds.minX + bounds.maxX) * 0.5);
                int regionZ = SectionPos.blockToSectionCoord((bounds.minZ + bounds.maxZ) * 0.5);
                if (Math.abs(regionX - cameraSectionX) > distance || Math.abs(regionZ - cameraSectionZ) > distance) continue;
                RegionKey regionKey = new RegionKey(placement, coord);
                if (!BUILT_REGIONS.containsKey(regionKey)) {
                    skippedBuild = true;
                    if (!PENDING_BUILDS.containsKey(regionKey) && PENDING_BUILDS.size() < MAX_PENDING_BUILDS) {
                        PENDING_BUILDS.put(regionKey, new PendingBuild(located.bundle(), map, placement, group));
                    }
                }
                if (BUILT_REGIONS.containsKey(regionKey)) BUILT_REGIONS.put(regionKey, frame);
                if (pvsCulling && !regionPvsVisible(map, placement, group.clusters())) { pvsRejectedRegions++; continue; }
                MESHES.forEach((key, mesh) -> { if (key.region.equals(regionKey)) mesh.lastVisibleFrame = frame; });
            }
        }
        drainPendingBuilds(buildDeadline);
        stillLoading = skippedBuild;
        if (!skippedBuild && PENDING_BUILDS.isEmpty()) firstPassComplete = true;
        discardExpiredMeshes();
        prefetchNearMeshes(generation);
        PAGES.pump(frame);
        invalidateReadyPages(PAGES.drainReadyPages());

        draw(event, generation, false, false);
    }

    /** NeoForge documents AFTER_PARTICLES as the safe basic custom-translucency stage. */
    private static void renderTranslucent(RenderLevelStageEvent event) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null || generationSequence < 0) return;
        draw(event, Src2mc.bundles().active(), true, false);
    }

    private static void draw(RenderLevelStageEvent event, BundleGeneration generation, boolean translucent, boolean shadowPass) {
        var camera = event.getCamera().getPosition();
        var drawItems = MESHES.entrySet().stream()
            .filter(item -> shadowPass ? BUILT_REGIONS.containsKey(item.getKey().region) : item.getValue().lastVisibleFrame == frame)
            .filter(item -> (item.getKey().renderClass == BundleMaterial.RenderClass.TRANSLUCENT) == translucent)
            // No frustum test in the shadow pass: the frustum there is the sun's, and rejecting a
            // mesh only keeps it out of the shadow map, which shows up as sunlight leaking through
            // sealed geometry rather than as a hole the player can see.
            .filter(item -> shadowPass
                ? withinShadowDistance(item.getValue().bounds, camera)
                : !frustumCulling || event.getFrustum().isVisible(item.getValue().bounds))
            .sorted(translucent ? Comparator.<Map.Entry<MeshKey, Mesh>>comparingDouble(item -> -distanceSquared(item.getValue().bounds, camera)) : (left, right) -> 0)
            .toList();
        for (var item : drawItems) {
            Mesh mesh = item.getValue();
            ResourceLocation texture = PAGES.request(generation.sequence(), mesh.bundle, mesh.atlas, item.getKey().page, frame)
                .orElseGet(PAGES::placeholderTexture);
            RenderType renderType = translucent ? RenderType.entityTranslucent(texture)
                : item.getKey().renderClass == BundleMaterial.RenderClass.SOLID ? RenderType.entitySolid(texture) : RenderType.entityCutout(texture);
            renderType.setupRenderState();
            Matrix4f modelView = new Matrix4f(event.getModelViewMatrix()).translate(
                (float) (mesh.origin.getX() - camera.x),
                (float) (mesh.origin.getY() - camera.y),
                (float) (mesh.origin.getZ() - camera.z));
            mesh.buffer.bind();
            mesh.buffer.drawWithShader(modelView, event.getProjectionMatrix(),
                translucent ? GameRenderer.getRendertypeEntityTranslucentShader()
                    : item.getKey().renderClass == BundleMaterial.RenderClass.SOLID
                        ? GameRenderer.getRendertypeEntitySolidShader() : GameRenderer.getRendertypeEntityCutoutShader());
            renderType.clearRenderState();
        }
        VertexBuffer.unbind();
    }

    private static void prefetchNearMeshes(BundleGeneration generation) {
        for (var item : MESHES.entrySet()) {
            Mesh mesh = item.getValue();
            if (mesh.lastVisibleFrame == frame) {
                PAGES.request(generation.sequence(), mesh.bundle, mesh.atlas, item.getKey().page, frame);
            }
        }
    }

    private static void invalidateReadyPages(List<AtlasPageResidency.PageKey> ready) {
        if (ready.isEmpty()) return;
        var affected = new HashSet<RegionKey>();
        for (var item : MESHES.entrySet()) for (AtlasPageResidency.PageKey page : ready) {
            if (item.getKey().page == page.page() && item.getValue().bundle.fingerprint().equals(page.fingerprint())) {
                affected.add(item.getKey().region);
            }
        }
        if (affected.isEmpty()) return;
        affected.forEach(BUILT_REGIONS::remove);
        MESHES.entrySet().removeIf(item -> {
            if (!affected.contains(item.getKey().region)) return false;
            item.getValue().close();
            return true;
        });
    }

    private static double distanceSquared(AABB bounds, net.minecraft.world.phys.Vec3 camera) {
        double x = (bounds.minX + bounds.maxX) * 0.5 - camera.x;
        double y = (bounds.minY + bounds.maxY) * 0.5 - camera.y;
        double z = (bounds.minZ + bounds.maxZ) * 0.5 - camera.z;
        return x * x + y * y + z * z;
    }

    private static String formatBytes(long bytes) {
        if (bytes < 1024 * 1024) return (bytes / 1024) + " KiB";
        return String.format(java.util.Locale.ROOT, "%.1f MiB", bytes / (1024.0 * 1024.0));
    }

    private static String formatMillis(long nanos) {
        return String.format(java.util.Locale.ROOT, "%.1fms", nanos / 1.0e6);
    }

    /**
     * Advances one region build by whole sections until {@code deadline}, returning true once every
     * section is tessellated. Region builds are split across frames because a 64-block region can
     * hold dozens of sections: run atomically, one relight after a torch placement stalled the frame
     * outright. The old meshes stay bound until the replacement uploads, so nothing flickers.
     */
    private static boolean advanceRegionBuild(PendingBuild build, long deadline) {
        BundleMap map = build.map;
        var sections = map.surfaces().sections();
        while (build.nextSection < build.group.sections().size()) {
            if (System.nanoTime() >= deadline) return false;
            SurfaceTable.SectionPos section = build.group.sections().get(build.nextSection++);
            List<SurfaceTable.Face> faces = sections.get(section);
            if (faces == null) continue;
            int baseX = section.x() << 4, baseY = section.y() << 4, baseZ = section.z() << 4;
            for (SurfaceTable.Face face : faces) {
                if (face.materialId() < 0 || face.materialId() >= map.materials().size()
                    || face.uvRegionId() < 0 || face.uvRegionId() >= map.surfaces().uvRegions().size()) continue;
                BundleMaterial material = map.materials().get(face.materialId());
                if (!material.textured() || material.renderClass() == BundleMaterial.RenderClass.FALLBACK) continue;
                AtlasIndex.Texture texture = map.atlas().textures().get(material.texture().contentId());
                if (texture == null) continue;
                int local = face.localCell();
                int x = baseX + (local & 15), y = baseY + (local >> 8 & 15), z = baseZ + (local >> 4 & 15);
                for (var triangle : SurfaceTessellator.tessellate(x, y, z, face,
                    map.surfaces().uvRegions().get(face.uvRegionId()), material.texture(), texture, map.atlas().pageSize())) {
                    build.triangles.computeIfAbsent(new PageClass(triangle.page(), material.renderClass()), ignored -> new ArrayList<>())
                        .add(new LitTriangle(triangle,
                            sampleVertexLight(build.placement, triangle.a(), face.patch(), build.lightCache),
                            sampleVertexLight(build.placement, triangle.b(), face.patch(), build.lightCache),
                            sampleVertexLight(build.placement, triangle.c(), face.patch(), build.lightCache)));
                }
            }
        }
        return true;
    }

    private static void finishRegionBuild(RegionKey regionKey, PendingBuild build) {
        RegionGroup group = build.group;
        BlockPos origin = build.placement.translation().offset(group.minX(), group.minY(), group.minZ());
        AABB bounds = regionBounds(build.placement, group);
        BUILT_REGIONS.put(regionKey, frame);
        build.triangles.forEach((pageClass, values) -> {
            MeshKey key = new MeshKey(regionKey, pageClass.page, pageClass.renderClass);
            Mesh old = MESHES.put(key, upload(build.bundle, build.map.atlas(), origin, bounds, values,
                group.minX(), group.minY(), group.minZ()));
            if (old != null) old.close();
        });
    }

    /** Drains in-flight region builds oldest-first, so a started region finishes before a new one
     * begins and no region is left half-tessellated for long. */
    private static void drainPendingBuilds(long deadline) {
        var iterator = PENDING_BUILDS.entrySet().iterator();
        while (iterator.hasNext() && System.nanoTime() < deadline) {
            var entry = iterator.next();
            long sliceStarted = System.nanoTime();
            boolean finished = advanceRegionBuild(entry.getValue(), deadline);
            lastBuildNanos = System.nanoTime() - sliceStarted;
            worstBuildNanos = Math.max(worstBuildNanos, lastBuildNanos);
            if (finished) {
                finishRegionBuild(entry.getKey(), entry.getValue());
                iterator.remove();
            }
        }
    }

    /**
     * One sample per vertex, at the vertex's own world position and along its
     * face's patch direction.
     *
     * One sample per face is what vanilla calls flat lighting, and it steps in
     * whole blocks -- a wall lit by one torch went from cell to cell rather
     * than fading. The eight cells behind a smooth sample all come out of the
     * build's own cache, so the extra cost is lookups in a hash map, not light
     * computations, and nothing changes per frame.
     */
    private static int sampleVertexLight(MapPlacement placement, SurfaceTessellator.Vertex vertex, int patch,
                                         Map<Long, Integer> cache) {
        int dirIndex = patch & 7;
        Direction direction = dirIndex < 6 ? Direction.values()[dirIndex] : null;
        float nx = direction == null ? 0 : direction.getStepX();
        float ny = direction == null ? 0 : direction.getStepY();
        float nz = direction == null ? 0 : direction.getStepZ();
        double worldX = placement.translation().getX() + vertex.x();
        double worldY = placement.translation().getY() + vertex.y();
        double worldZ = placement.translation().getZ() + vertex.z();
        return smoothLighting
            ? LightSampler.smooth(level, worldX, worldY, worldZ, nx, ny, nz, cache)
            : LightSampler.sample(level, worldX, worldY, worldZ, nx, ny, nz, cache);
    }

    private static Mesh upload(BundleManifest bundle, AtlasIndex atlas, BlockPos origin, AABB bounds,
                               List<LitTriangle> triangles, int baseX, int baseY, int baseZ) {
        int capacity = (int) Math.min(Integer.MAX_VALUE, Math.max(4096L, (long) triangles.size() * 3 * 36));
        try (var bytes = new ByteBufferBuilder(capacity)) {
            var builder = new BufferBuilder(bytes, VertexFormat.Mode.TRIANGLES, DefaultVertexFormat.NEW_ENTITY);
            for (var lit : triangles) {
                SurfaceTessellator.Triangle triangle = lit.triangle();
                vertex(builder, triangle.a(), baseX, baseY, baseZ, triangle, lit.lightA());
                vertex(builder, triangle.b(), baseX, baseY, baseZ, triangle, lit.lightB());
                vertex(builder, triangle.c(), baseX, baseY, baseZ, triangle, lit.lightC());
            }
            try (var data = builder.buildOrThrow()) {
                var buffer = new VertexBuffer(VertexBuffer.Usage.STATIC);
                buffer.bind(); buffer.upload(data); VertexBuffer.unbind();
                return new Mesh(bundle, atlas, buffer, origin, bounds, frame);
            }
        }
    }

    private static void vertex(BufferBuilder builder, SurfaceTessellator.Vertex vertex, int baseX, int baseY, int baseZ,
                               SurfaceTessellator.Triangle triangle, int light) {
        double abx = triangle.b().x() - triangle.a().x(), aby = triangle.b().y() - triangle.a().y(), abz = triangle.b().z() - triangle.a().z();
        double acx = triangle.c().x() - triangle.a().x(), acy = triangle.c().y() - triangle.a().y(), acz = triangle.c().z() - triangle.a().z();
        float nx = (float) (aby * acz - abz * acy), ny = (float) (abz * acx - abx * acz), nz = (float) (abx * acy - aby * acx);
        float length = (float) Math.sqrt(nx * nx + ny * ny + nz * nz);
        if (length > 0) { nx /= length; ny /= length; nz /= length; }
        builder.addVertex((float) (vertex.x() - baseX), (float) (vertex.y() - baseY), (float) (vertex.z() - baseZ))
            .setColor(255, 255, 255, 255).setUv((float) vertex.u(), (float) vertex.v()).setOverlay(OverlayTexture.NO_OVERLAY)
            .setLight(light).setNormal(nx, ny, nz);
    }

    /** Removes the built mesh for the region overlapping {@code worldSection} so the next frame's
     * build loop rebuilds it with fresh light. Covers the section itself and all 26 neighbours:
     * a smooth sample reads the eight cells around a point up to half a block outside the face,
     * so a change diagonally across a section corner does reach this region's vertices. */
    static void invalidateLight(MapPlacement placement, SectionPos worldSection) {
        BlockPos local = placement.toLocal(new BlockPos(SectionPos.sectionToBlockCoord(worldSection.x()),
            SectionPos.sectionToBlockCoord(worldSection.y()), SectionPos.sectionToBlockCoord(worldSection.z())));
        int sx = Math.floorDiv(local.getX(), 16), sy = Math.floorDiv(local.getY(), 16), sz = Math.floorDiv(local.getZ(), 16);
        for (int dx = -1; dx <= 1; dx++) {
            for (int dy = -1; dy <= 1; dy++) {
                for (int dz = -1; dz <= 1; dz++) invalidateRegionAt(placement, sx + dx, sy + dy, sz + dz);
            }
        }
    }

    private static void invalidateRegionAt(MapPlacement placement, int sectionX, int sectionY, int sectionZ) {
        RegionCoord coord = new RegionCoord(Math.floorDiv(sectionX, REGION_SECTIONS),
            Math.floorDiv(sectionY, REGION_SECTIONS), Math.floorDiv(sectionZ, REGION_SECTIONS));
        RegionKey key = new RegionKey(placement, coord);
        // A build already in flight sampled the pre-change light, so it has to restart.
        PENDING_BUILDS.remove(key);
        if (BUILT_REGIONS.remove(key) != null) relightsQueued++;
    }

    /** Groups a map's static sections into {@link #REGION_SECTIONS}-wide cubes, once per map
     * (surface geometry never changes after load), so building/culling/PVS work operates on far
     * fewer, larger units than one-section-at-a-time. */
    private static Map<RegionCoord, RegionGroup> regionGroups(BundleMap map) {
        return REGION_GROUPS.computeIfAbsent(map, MapSurfaceRenderer::buildRegionGroups);
    }

    private static Map<RegionCoord, RegionGroup> buildRegionGroups(BundleMap map) {
        Map<RegionCoord, List<SurfaceTable.SectionPos>> grouped = new HashMap<>();
        for (SurfaceTable.SectionPos section : map.surfaces().sections().keySet()) {
            RegionCoord coord = new RegionCoord(Math.floorDiv(section.x(), REGION_SECTIONS),
                Math.floorDiv(section.y(), REGION_SECTIONS), Math.floorDiv(section.z(), REGION_SECTIONS));
            grouped.computeIfAbsent(coord, ignored -> new ArrayList<>()).add(section);
        }
        Map<RegionCoord, RegionGroup> result = new HashMap<>();
        grouped.forEach((coord, sections) -> {
            int minX = Integer.MAX_VALUE, minY = Integer.MAX_VALUE, minZ = Integer.MAX_VALUE;
            int maxX = Integer.MIN_VALUE, maxY = Integer.MIN_VALUE, maxZ = Integer.MIN_VALUE;
            for (SurfaceTable.SectionPos section : sections) {
                minX = Math.min(minX, section.x() << 4); minY = Math.min(minY, section.y() << 4); minZ = Math.min(minZ, section.z() << 4);
                maxX = Math.max(maxX, (section.x() << 4) + 16); maxY = Math.max(maxY, (section.y() << 4) + 16); maxZ = Math.max(maxZ, (section.z() << 4) + 16);
            }
            result.put(coord, new RegionGroup(sections, unionSectionClusters(map, sections), minX, minY, minZ, maxX, maxY, maxZ));
        });
        return result;
    }

    /** Union of every member section's visible clusters; fails open (returns null, meaning
     * "always visible") if any member section lacks PVS coverage, matching
     * {@link dev.theredja.src2mc.bundle.PropVisibility#visible} treating null/empty as visible. */
    private static short[] unionSectionClusters(BundleMap map, List<SurfaceTable.SectionPos> sections) {
        var pvs = map.pvs();
        if (pvs == null) return null;
        var union = new java.util.TreeSet<Short>();
        for (SurfaceTable.SectionPos section : sections) {
            short[] clusters = pvs.sectionClusters(section.x(), section.y(), section.z());
            if (clusters == null || clusters.length == 0) return null;
            for (short cluster : clusters) union.add(cluster);
        }
        short[] result = new short[union.size()];
        int index = 0;
        for (short cluster : union) result[index++] = cluster;
        return result;
    }

    private static AABB regionBounds(MapPlacement placement, RegionGroup group) {
        BlockPos min = placement.translation().offset(group.minX(), group.minY(), group.minZ());
        BlockPos max = placement.translation().offset(group.maxX(), group.maxY(), group.maxZ());
        return new AABB(min.getX(), min.getY(), min.getZ(), max.getX(), max.getY(), max.getZ()).inflate(0.01);
    }

    private static void discardExpiredMeshes() {
        List<RegionKey> expired = BUILT_REGIONS.entrySet().stream()
            .filter(item -> frame - item.getValue() > MESH_GRACE_FRAMES).map(Map.Entry::getKey).toList();
        for (RegionKey region : expired) {
            BUILT_REGIONS.remove(region);
            MESHES.entrySet().removeIf(item -> {
                if (!item.getKey().region.equals(region)) return false;
                item.getValue().close();
                return true;
            });
        }
    }

    private static void clear() {
        MESHES.values().forEach(Mesh::close); MESHES.clear(); BUILT_REGIONS.clear(); REGION_GROUPS.clear();
        PENDING_BUILDS.clear();
        PAGES.reset(-1);
        CameraVisibility.reset();
        level = null; generationSequence = -1; placementSnapshot = List.of(); frame = 0; pvsRejectedRegions = 0; stillLoading = true;
        firstPassComplete = false; shadowPassCallsSinceMainPass = 0; shadowPassCallsLastFrame = 0;
    }

    /** Only rejects regions in the placement the camera is currently inside; other placements
     * keep today's distance+frustum-only behavior, matching how {@link PropRenderer} treats
     * props belonging to a placement other than the camera's. */
    private static boolean regionPvsVisible(BundleMap map, MapPlacement placement, short[] clusters) {
        if (CameraVisibility.row() == null || !placement.equals(CameraVisibility.placement())) return true;
        var pvs = map.pvs();
        if (pvs == null) return true;
        return pvs.visible(CameraVisibility.row(), clusters);
    }

    private record LitTriangle(SurfaceTessellator.Triangle triangle, int lightA, int lightB, int lightC) {}
    private record RegionCoord(int x, int y, int z) {}
    private record RegionGroup(List<SurfaceTable.SectionPos> sections, short[] clusters,
                                int minX, int minY, int minZ, int maxX, int maxY, int maxZ) {}
    private record RegionKey(MapPlacement placement, RegionCoord coord) {}
    private record MeshKey(RegionKey region, int page, BundleMaterial.RenderClass renderClass) {}
    private record PageClass(int page, BundleMaterial.RenderClass renderClass) {}
    private static final class Mesh implements AutoCloseable {
        final BundleManifest bundle; final AtlasIndex atlas; final VertexBuffer buffer; final BlockPos origin; final AABB bounds;
        long lastVisibleFrame;
        Mesh(BundleManifest bundle, AtlasIndex atlas, VertexBuffer buffer, BlockPos origin, AABB bounds, long frame) {
            this.bundle = bundle; this.atlas = atlas; this.buffer = buffer; this.origin = origin; this.bounds = bounds; this.lastVisibleFrame = frame;
        }
        @Override public void close() { buffer.close(); }
    }
}
