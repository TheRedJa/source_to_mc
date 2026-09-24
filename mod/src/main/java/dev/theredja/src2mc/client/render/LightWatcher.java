package dev.theredja.src2mc.client.render;

import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.bundle.BundleGeneration;
import dev.theredja.src2mc.network.PlacementNetwork;
import dev.theredja.src2mc.world.MapPlacement;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.SectionPos;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.chunk.DataLayer;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientTickEvent;

/**
 * Polls vanilla light data for world sections overlapping placed maps and invalidates the
 * affected surface regions/prop aggregates on change. NeoForge exposes no client-side
 * light-update event, and hooking one would need the render-pipeline mixin this project
 * deliberately avoids, so this trades an event for a bounded round-robin hash scan each tick.
 */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
final class LightWatcher {
    private static final long SCAN_BUDGET_NANOS = 1_000_000L;
    private static final int WATCH_RADIUS_SECTIONS = 6;
    private static final Map<Long, Watch> WATCHED = new LinkedHashMap<>();
    private static boolean enabled = true;
    private static List<Long> scanOrder = List.of();
    private static int scanCursor;
    private static ClientLevel level;
    private static long generationSequence = -1;
    private static List<MapPlacement> placementSnapshot = List.of();
    private static int lastCameraSectionX = Integer.MIN_VALUE;
    private static int lastCameraSectionY = Integer.MIN_VALUE;
    private static int lastCameraSectionZ = Integer.MIN_VALUE;
    private static long checksLastTick;
    private static long invalidatedLastTick;

    private LightWatcher() {}

    @SubscribeEvent
    static void onClientTick(ClientTickEvent.Post event) {
        Minecraft minecraft = Minecraft.getInstance();
        if (!enabled) return;
        if (minecraft.level == null) { clear(); return; }
        BundleGeneration generation = Src2mc.bundles().active();
        List<MapPlacement> placements = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).view();
        if (minecraft.level != level || generation.sequence() != generationSequence || !placements.equals(placementSnapshot)) {
            clear();
            level = minecraft.level;
            generationSequence = generation.sequence();
            placementSnapshot = placements;
        }
        if (generation.sequence() == 0 || placements.isEmpty()) return;
        var camera = minecraft.gameRenderer.getMainCamera().getPosition();
        int cameraSectionX = SectionPos.blockToSectionCoord(camera.x);
        int cameraSectionY = SectionPos.blockToSectionCoord(camera.y);
        int cameraSectionZ = SectionPos.blockToSectionCoord(camera.z);
        int distance = minecraft.options.getEffectiveRenderDistance() + 1;
        refreshWatchSet(placements, cameraSectionX, cameraSectionY, cameraSectionZ, distance);
        scanTick();
    }

    /** Rebuilds the watch set only when the camera moves to a new section, bounded to a cube of
     * {@link #WATCH_RADIUS_SECTIONS} around the camera. The bound is what keeps this cheap: a map's
     * own bounds span its whole Y column and thousands of sections, so tracking all of them
     * reallocated a six-figure map on every section crossing — a hitch every few blocks walked. */
    private static void refreshWatchSet(List<MapPlacement> placements, int cameraSectionX, int cameraSectionY,
                                        int cameraSectionZ, int distance) {
        if (cameraSectionX == lastCameraSectionX && cameraSectionY == lastCameraSectionY && cameraSectionZ == lastCameraSectionZ) return;
        lastCameraSectionX = cameraSectionX; lastCameraSectionY = cameraSectionY; lastCameraSectionZ = cameraSectionZ;
        int radius = Math.min(distance, WATCH_RADIUS_SECTIONS);
        Map<Long, Watch> next = new LinkedHashMap<>();
        for (MapPlacement placement : placements) {
            int minSX = SectionPos.blockToSectionCoord(placement.worldMin().getX());
            int minSY = SectionPos.blockToSectionCoord(placement.worldMin().getY());
            int minSZ = SectionPos.blockToSectionCoord(placement.worldMin().getZ());
            int maxSX = SectionPos.blockToSectionCoord(placement.worldMax().getX());
            int maxSY = SectionPos.blockToSectionCoord(placement.worldMax().getY());
            int maxSZ = SectionPos.blockToSectionCoord(placement.worldMax().getZ());
            for (int sx = Math.max(minSX, cameraSectionX - radius); sx <= Math.min(maxSX, cameraSectionX + radius); sx++) {
                for (int sz = Math.max(minSZ, cameraSectionZ - radius); sz <= Math.min(maxSZ, cameraSectionZ + radius); sz++) {
                    for (int sy = Math.max(minSY, cameraSectionY - radius); sy <= Math.min(maxSY, cameraSectionY + radius); sy++) {
                        long key = SectionPos.asLong(sx, sy, sz);
                        Watch existing = WATCHED.get(key);
                        next.put(key, existing != null && existing.placement.equals(placement) ? existing : new Watch(placement));
                    }
                }
            }
        }
        WATCHED.clear();
        WATCHED.putAll(next);
        scanOrder = new ArrayList<>(WATCHED.keySet());
        scanCursor = 0;
    }

    private static void scanTick() {
        checksLastTick = 0;
        invalidatedLastTick = 0;
        if (scanOrder.isEmpty()) return;
        long deadline = System.nanoTime() + SCAN_BUDGET_NANOS;
        int visited = 0;
        while (visited < scanOrder.size() && System.nanoTime() < deadline) {
            if (scanCursor >= scanOrder.size()) scanCursor = 0;
            long key = scanOrder.get(scanCursor);
            scanCursor++;
            visited++;
            Watch watch = WATCHED.get(key);
            if (watch == null) continue;
            SectionPos section = SectionPos.of(key);
            int hash = hashSection(section);
            checksLastTick++;
            if (!watch.initialized) {
                watch.hash = hash;
                watch.initialized = true;
            } else if (hash != watch.hash) {
                watch.hash = hash;
                MapSurfaceRenderer.invalidateLight(watch.placement, section);
                PropRenderer.invalidateLight(watch.placement, section);
                invalidatedLastTick++;
            }
        }
    }

    /** Day/night needs no invalidation here: sky light values are static per block, and
     * {@link net.minecraft.client.renderer.LightTexture} handles time of day when the shader
     * samples the lightmap. This only reacts to actual block/sky data-layer edits. */
    private static int hashSection(SectionPos section) {
        var lightEngine = level.getLightEngine();
        return hashLayer(lightEngine.getLayerListener(LightLayer.BLOCK).getDataLayerData(section)) * 31
            + hashLayer(lightEngine.getLayerListener(LightLayer.SKY).getDataLayerData(section));
    }

    /** Homogenous layers are hashed from their fill value: {@link DataLayer#getData()} would
     * otherwise materialize a 2 KB array inside vanilla's own object just to hash it. */
    private static int hashLayer(DataLayer layer) {
        if (layer == null) return 0;
        if (layer.isDefinitelyHomogenous()) {
            for (int value = 0; value <= 15; value++) if (layer.isDefinitelyFilledWith(value)) return ~value;
        }
        return Arrays.hashCode(layer.getData());
    }

    private static void clear() {
        WATCHED.clear();
        scanOrder = List.of();
        scanCursor = 0;
        level = null;
        generationSequence = -1;
        placementSnapshot = List.of();
        lastCameraSectionX = Integer.MIN_VALUE;
        lastCameraSectionY = Integer.MIN_VALUE;
        lastCameraSectionZ = Integer.MIN_VALUE;
        checksLastTick = 0;
        invalidatedLastTick = 0;
    }

    /** Kill switch for A/B measurement: with relight off, meshes keep whatever light they were
     * built with and nothing is ever invalidated. */
    static void setEnabled(boolean value) {
        enabled = value;
        if (!enabled) clear();
    }

    static boolean enabled() { return enabled; }

    static int watchedSections() { return WATCHED.size(); }
    static long checksLastTick() { return checksLastTick; }
    static long invalidatedLastTick() { return invalidatedLastTick; }

    private static final class Watch {
        final MapPlacement placement;
        int hash;
        boolean initialized;
        Watch(MapPlacement placement) { this.placement = placement; }
    }
}
