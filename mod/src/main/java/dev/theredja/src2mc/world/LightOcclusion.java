package dev.theredja.src2mc.world;

import dev.theredja.src2mc.Src2mc;
import it.unimi.dsi.fastutil.longs.Long2ObjectMap;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import net.minecraft.core.SectionPos;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.chunk.DataLayer;

/**
 * Holds each dimension's baked sky light and hands it to the light engine.
 *
 * See {@link SkyLightBake} for why the light is computed rather than left to
 * vanilla. Publishing it is the other half: a section only answers out of a
 * data layer once it has one, and an air-only section -- which is exactly what
 * a ceiling of drawn meshes sits in -- never gets one. Queueing the layer and
 * then declaring the section non-empty is what gives it one:
 * {@code LayerLightSectionStorage.initializeSection} builds the section's layer
 * with {@code createDataLayer}, which returns whatever is queued for it.
 *
 * Client and server bake separately from the same bundle, so they cannot
 * disagree: the server's copy is what gameplay and the saved chunk see, the
 * client's is what is drawn.
 */
public final class LightOcclusion {
    private static final Map<ResourceLocation, SkyLightBake.Baked> SERVER = new ConcurrentHashMap<>();
    private static final Map<ResourceLocation, SkyLightBake.Baked> CLIENT = new ConcurrentHashMap<>();
    // What the server's bake was last built from, so a change in either input
    // rebuilds it. Placements come back out of saved data already registered
    // and bundles are published by a command, so no single event can be trusted.
    private static final Map<ResourceLocation, long[]> SERVER_INPUTS = new ConcurrentHashMap<>();
    private static volatile boolean enabled = true;
    /** Bumped every time the client re-bakes, so a renderer can tell whether the light a mesh
     * captured came from the bake it is being drawn beside. */
    private static volatile long clientEpoch;
    private static volatile long clientGeneration = -1;
    /** A second between checks: nothing here changes faster than a command. */
    private static final int CHECK_INTERVAL_TICKS = 20;

    private LightOcclusion() {}

    public static boolean enabled() { return enabled; }

    /** How many times the client's bake has been published this session. */
    public static long clientEpoch() { return clientEpoch; }

    /** The bundle generation the client's current bake was computed from, or -1 before the first. */
    public static long clientGeneration() { return clientGeneration; }

    /** Turning the bake off republishes full daylight, which is what vanilla alone produces here. */
    public static void setEnabled(boolean value) { enabled = value; }

    private static Map<ResourceLocation, SkyLightBake.Baked> side(Level level) {
        return level.isClientSide() ? CLIENT : SERVER;
    }

    public static SkyLightBake.Baked baked(Level level) {
        return side(level).getOrDefault(level.dimension().location(), SkyLightBake.Baked.EMPTY);
    }

    /** Baked sky light at a world cell, or -1 where no bake covers it. */
    public static int skyAt(Level level, int x, int y, int z) {
        return baked(level).at(x, y, z);
    }

    /** Drop every dimension's bake, for a world unload or a bundle reload. */
    public static void clear() {
        SERVER.clear();
        CLIENT.clear();
        SERVER_INPUTS.clear();
        clientGeneration = -1;
        clientEpoch++;
    }

    /**
     * Rebuild one dimension's sky light from its placements and publish it to
     * every loaded chunk it covers. Returns how many cells came out darker than
     * open sky.
     */
    public static int rebuild(ServerLevel level) {
        PlacementIndex index = PlacementSavedData.get(level).index();
        SkyLightBake.Baked baked = SkyLightBake.bake(index.view(), Src2mc.bundles().active(),
            level.getMinBuildHeight(), level.getMaxBuildHeight());
        ResourceLocation dimension = level.dimension().location();
        if (baked.isEmpty()) SERVER.remove(dimension); else SERVER.put(dimension, baked);
        SERVER_INPUTS.put(dimension, inputs(level, index));
        int applied = publish(level, baked);
        Src2mc.LOGGER.info(
            "src2mc: baked sky light for {} from {} placement(s) in {} ms: {} shaded cell(s),"
                + " {} section(s), applied to {} loaded chunk(s), bundles={}",
            dimension, index.size(), baked.millis(), baked.darkCells(), baked.sections().size(), applied,
            Src2mc.bundles().active().sequence());
        return baked.darkCells();
    }

    /**
     * Rebuild the server's light when what it is built from has changed.
     * Watching the two inputs costs a comparison per second and cannot be
     * out-ordered by an event.
     */
    public static void onServerTick(net.neoforged.neoforge.event.tick.ServerTickEvent.Post event) {
        if (event.getServer().getTickCount() % CHECK_INTERVAL_TICKS != 0) return;
        for (ServerLevel level : event.getServer().getAllLevels()) {
            long[] now = inputs(level, PlacementSavedData.get(level).index());
            long[] before = SERVER_INPUTS.get(level.dimension().location());
            if (before == null || before[0] != now[0] || before[1] != now[1]) rebuild(level);
        }
    }

    private static long[] inputs(ServerLevel level, PlacementIndex index) {
        return new long[] {Src2mc.bundles().active().sequence(), index.size()};
    }

    /** Bake and publish the client's own copy, which is what gets drawn. */
    public static int publishClient(Level level, ResourceLocation dimension, PlacementIndex index) {
        SkyLightBake.Baked baked = SkyLightBake.bake(index.view(), Src2mc.bundles().active(),
            level.getMinBuildHeight(), level.getMaxBuildHeight());
        if (baked.isEmpty()) CLIENT.remove(dimension); else CLIENT.put(dimension, baked);
        clientGeneration = Src2mc.bundles().active().sequence();
        clientEpoch++;
        publish(level, baked);
        return baked.darkCells();
    }

    /**
     * Hand a bake to one level's light engine, for the chunks it has loaded.
     * Returns how many chunks were reached; anything else picks the light up as
     * it loads.
     */
    private static int publish(Level level, SkyLightBake.Baked baked) {
        int chunks = 0;
        for (long chunk : baked.chunks()) {
            int chunkX = ChunkPos.getX(chunk), chunkZ = ChunkPos.getZ(chunk);
            if (!level.hasChunk(chunkX, chunkZ)) continue;
            applyChunk(level, chunkX, chunkZ);
            chunks++;
        }
        return chunks;
    }

    /**
     * Publish the light for one chunk. Called as chunks load, because light is
     * saved with the chunk and comes back off disk as it was written -- before
     * the bake existed, or from a different bundle.
     */
    public static void applyChunk(Level level, int chunkX, int chunkZ) {
        SkyLightBake.Baked baked = baked(level);
        if (!baked.covers(chunkX, chunkZ)) return;
        var engine = level.getLightEngine();
        Long2ObjectMap<byte[]> sections = baked.sections();
        boolean applied = false;
        for (int sectionY = level.getMinSection(); sectionY < level.getMaxSection(); sectionY++) {
            byte[] layer = sections.get(SectionPos.asLong(chunkX, sectionY, chunkZ));
            if (layer == null) continue;
            SectionPos at = SectionPos.of(chunkX, sectionY, chunkZ);
            engine.queueSectionData(LightLayer.SKY, at, new DataLayer(enabled ? layer.clone() : fullDaylight()));
            // An air-only section holds no light data at all, and a section
            // with no data answers full daylight whatever stands in the way.
            // This is what gives it data; the queued layer is that data.
            engine.updateSectionStatus(at, false);
            applied = true;
        }
        if (applied && level.isClientSide()) {
            dev.theredja.src2mc.client.ClientLightRefresh.markChunkDirty(chunkX, chunkZ);
        }
    }

    private static byte[] fullDaylight() {
        byte[] bytes = new byte[2048];
        java.util.Arrays.fill(bytes, (byte) 0xFF);
        return bytes;
    }
}
