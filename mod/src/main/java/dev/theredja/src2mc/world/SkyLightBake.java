package dev.theredja.src2mc.world;

import dev.theredja.src2mc.bundle.BundleGeneration;
import dev.theredja.src2mc.bundle.BundleMap;
import dev.theredja.src2mc.bundle.OcclusionTable;
import dev.theredja.src2mc.bundle.SurfaceTable;
import it.unimi.dsi.fastutil.ints.IntArrayFIFOQueue;
import it.unimi.dsi.fastutil.longs.Long2ObjectMap;
import it.unimi.dsi.fastutil.longs.Long2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.longs.LongOpenHashSet;
import it.unimi.dsi.fastutil.longs.LongSet;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.world.level.ChunkPos;

/**
 * Sky light for a placed map, computed from the bundle rather than the world.
 *
 * A converted map is mostly geometry that holds no block: brushes too thin to
 * voxelize are drawn as meshes, and every prop always was a mesh. Minecraft
 * blocks light with blocks alone, so daylight walks straight through ceilings
 * the source map has. Teaching vanilla's light engine to treat those cells as
 * opaque cannot work: {@code SkyLightSectionStorage.getLightValue} answers 15
 * for every cell in a column above the highest section that stores light data,
 * and a ceiling made of meshes lives in sections that hold no block at all and
 * therefore store nothing.
 *
 * So the light is computed here instead and handed to the engine as finished
 * data layers. What blocks daylight is the union of the map's voxelized cells —
 * every cell the surface table has a face in — and its occlusion mask, which
 * carries the drawn brushes and the props. Everything is map-local and static,
 * so one bake per placement serves until the placement or the bundle changes.
 */
public final class SkyLightBake {
    /**
     * Blocks of open world kept around a placement while the light is computed,
     * so daylight arrives at the map's edges from outside it rather than
     * starting at its boundary. One section is enough: light falls off in 15
     * steps and the map's own outer wall is what stops it.
     */
    public static final int MARGIN = 16;

    /** Full daylight, and the value everything outside a bake is assumed to hold. */
    public static final int FULL = 15;

    private SkyLightBake() {}

    /**
     * One dimension's finished sky light: a data layer per world section that a
     * placement covers, ready to hand to the light engine. Never mutated after
     * it is built.
     */
    public record Baked(Long2ObjectMap<byte[]> sections, LongSet chunks, int litCells, int darkCells, long millis) {
        public static final Baked EMPTY =
            new Baked(new Long2ObjectOpenHashMap<>(), LongSet.of(), 0, 0, 0L);

        public boolean isEmpty() { return sections.isEmpty(); }

        /** Sky light at a world cell, or -1 where this bake says nothing. */
        public int at(int x, int y, int z) {
            byte[] layer = sections.get(SectionPos.asLong(x >> 4, y >> 4, z >> 4));
            return layer == null ? -1 : nibble(layer, ((y & 15) << 8) | ((z & 15) << 4) | (x & 15));
        }

        public boolean covers(int chunkX, int chunkZ) { return chunks.contains(ChunkPos.asLong(chunkX, chunkZ)); }
    }

    /**
     * Compute the sky light of every placement, clamped to the world's own
     * height. Placements cannot overlap, so their sections never collide.
     */
    public static Baked bake(List<MapPlacement> placements, BundleGeneration generation, int minY, int maxY) {
        if (generation == null || placements.isEmpty()) return Baked.EMPTY;
        long started = System.nanoTime();
        Long2ObjectMap<byte[]> sections = new Long2ObjectOpenHashMap<>();
        LongSet chunks = new LongOpenHashSet();
        int lit = 0, dark = 0;
        for (MapPlacement placement : placements) {
            Optional<BundleMap> found = generation.findMap(placement.campaignId(), placement.mapId());
            if (found.isEmpty()) continue;
            Region region = bakeOne(found.get(), placement, minY, maxY);
            if (region == null) continue;
            lit += region.lit;
            dark += region.dark;
            region.emit(placement, minY, maxY, sections, chunks);
        }
        return new Baked(sections, chunks, lit, dark, (System.nanoTime() - started) / 1_000_000L);
    }

    /** One placement's light, in its own dense world-space box. */
    private static final class Region {
        final int x0, y0, z0, dx, dy, dz;
        final byte[] light;
        int lit, dark;

        Region(int x0, int y0, int z0, int dx, int dy, int dz) {
            this.x0 = x0; this.y0 = y0; this.z0 = z0;
            this.dx = dx; this.dy = dy; this.dz = dz;
            this.light = new byte[dx * dy * dz];
        }

        int index(int x, int y, int z) { return ((y - y0) * dz + (z - z0)) * dx + (x - x0); }

        boolean holds(int x, int y, int z) {
            return x >= x0 && x < x0 + dx && y >= y0 && y < y0 + dy && z >= z0 && z < z0 + dz;
        }

        /**
         * Write a data layer for every section the placement itself covers. The
         * margin is deliberately left out: outside the map our answer is a
         * guess about a world we did not convert, and vanilla's is right.
         */
        void emit(MapPlacement placement, int minY, int maxY, Long2ObjectMap<byte[]> sections, LongSet chunks) {
            int sectionMinY = Math.max(SectionPos.blockToSectionCoord(minY), SectionPos.blockToSectionCoord(placement.worldMin().getY()));
            int sectionMaxY = Math.min(SectionPos.blockToSectionCoord(maxY - 1), SectionPos.blockToSectionCoord(placement.worldMax().getY()));
            for (int sx = SectionPos.blockToSectionCoord(placement.worldMin().getX());
                 sx <= SectionPos.blockToSectionCoord(placement.worldMax().getX()); sx++) {
                for (int sz = SectionPos.blockToSectionCoord(placement.worldMin().getZ());
                     sz <= SectionPos.blockToSectionCoord(placement.worldMax().getZ()); sz++) {
                    chunks.add(ChunkPos.asLong(sx, sz));
                    for (int sy = sectionMinY; sy <= sectionMaxY; sy++) {
                        sections.put(SectionPos.asLong(sx, sy, sz), layer(sx, sy, sz));
                    }
                }
            }
        }

        private byte[] layer(int sectionX, int sectionY, int sectionZ) {
            byte[] bytes = new byte[2048];
            int baseX = sectionX << 4, baseY = sectionY << 4, baseZ = sectionZ << 4;
            for (int y = 0; y < 16; y++) {
                for (int z = 0; z < 16; z++) {
                    for (int x = 0; x < 16; x++) {
                        int world = holds(baseX + x, baseY + y, baseZ + z)
                            ? light[index(baseX + x, baseY + y, baseZ + z)]
                            : FULL;
                        setNibble(bytes, (y << 8) | (z << 4) | x, world);
                    }
                }
            }
            return bytes;
        }
    }

    private static Region bakeOne(BundleMap map, MapPlacement placement, int minY, int maxY) {
        BlockPos min = placement.worldMin(), max = placement.worldMax();
        int x0 = min.getX() - MARGIN, x1 = max.getX() + MARGIN;
        int z0 = min.getZ() - MARGIN, z1 = max.getZ() + MARGIN;
        int y0 = Math.max(minY, min.getY() - MARGIN), y1 = Math.min(maxY - 1, max.getY() + MARGIN);
        if (y1 <= y0) return null;
        Region region = new Region(x0, y0, z0, x1 - x0 + 1, y1 - y0 + 1, z1 - z0 + 1);
        boolean[] opaque = new boolean[region.light.length];
        fillOpaque(map, placement, region, opaque);
        flood(region, opaque);
        for (int i = 0; i < region.light.length; i++) {
            if (opaque[i]) continue;
            if (region.light[i] == FULL) region.lit++; else region.dark++;
        }
        return region;
    }

    /**
     * Everything in the map that stops daylight: the cells the voxelizer filled
     * — recognized by the surface table having a face in them — and the cells
     * the exporter recorded as occluding without holding a block.
     */
    private static void fillOpaque(BundleMap map, MapPlacement placement, Region region, boolean[] opaque) {
        BlockPos translation = placement.translation();
        for (Map.Entry<SurfaceTable.SectionPos, List<SurfaceTable.Face>> entry : map.surfaces().sections().entrySet()) {
            SurfaceTable.SectionPos at = entry.getKey();
            for (SurfaceTable.Face face : entry.getValue()) {
                int local = face.localCell();
                mark(region, opaque, translation.getX() + (at.x() << 4) + (local & 15),
                    translation.getY() + (at.y() << 4) + ((local >> 8) & 15),
                    translation.getZ() + (at.z() << 4) + ((local >> 4) & 15));
            }
        }
        OcclusionTable occlusion = map.occlusion();
        if (occlusion == null) return;
        for (Map.Entry<SurfaceTable.SectionPos, byte[]> entry : occlusion.sections().entrySet()) {
            SurfaceTable.SectionPos at = entry.getKey();
            byte[] bits = entry.getValue();
            for (int local = 0; local < OcclusionTable.SECTION_BYTES * 8; local++) {
                if ((bits[local >> 3] & (1 << (local & 7))) == 0) continue;
                mark(region, opaque, translation.getX() + (at.x() << 4) + (local & 15),
                    translation.getY() + (at.y() << 4) + ((local >> 8) & 15),
                    translation.getZ() + (at.z() << 4) + ((local >> 4) & 15));
            }
        }
    }

    private static void mark(Region region, boolean[] opaque, int x, int y, int z) {
        if (region.holds(x, y, z)) opaque[region.index(x, y, z)] = true;
    }

    /**
     * Vanilla's own sky rules: the top of the box is open sky, light falls
     * straight down at full strength and loses one step in every other
     * direction. Matching them is what makes the map's edge meet the daylight
     * around it without a seam.
     */
    private static void flood(Region region, boolean[] opaque) {
        IntArrayFIFOQueue[] queues = new IntArrayFIFOQueue[FULL + 1];
        for (int level = 1; level <= FULL; level++) queues[level] = new IntArrayFIFOQueue();
        int top = region.y0 + region.dy - 1;
        for (int z = region.z0; z < region.z0 + region.dz; z++) {
            for (int x = region.x0; x < region.x0 + region.dx; x++) {
                int index = region.index(x, top, z);
                if (opaque[index]) continue;
                region.light[index] = FULL;
                queues[FULL].enqueue(index);
            }
        }
        int plane = region.dx * region.dz;
        for (int level = FULL; level >= 1; level--) {
            IntArrayFIFOQueue queue = queues[level];
            while (!queue.isEmpty()) {
                int index = queue.dequeueInt();
                if (region.light[index] != level) continue;
                int x = index % region.dx;
                int z = (index / region.dx) % region.dz;
                int y = index / plane;
                // Straight down keeps full daylight; everything else costs a step.
                spread(region, opaque, queues, index - plane, y > 0, level == FULL ? FULL : level - 1);
                spread(region, opaque, queues, index + plane, y < region.dy - 1, level - 1);
                spread(region, opaque, queues, index - 1, x > 0, level - 1);
                spread(region, opaque, queues, index + 1, x < region.dx - 1, level - 1);
                spread(region, opaque, queues, index - region.dx, z > 0, level - 1);
                spread(region, opaque, queues, index + region.dx, z < region.dz - 1, level - 1);
            }
        }
    }

    private static void spread(Region region, boolean[] opaque, IntArrayFIFOQueue[] queues,
                               int index, boolean inside, int level) {
        if (!inside || level < 1 || opaque[index] || region.light[index] >= level) return;
        region.light[index] = (byte) level;
        queues[level].enqueue(index);
    }

    static void setNibble(byte[] layer, int index, int value) {
        int at = index >> 1;
        if ((index & 1) == 0) layer[at] = (byte) ((layer[at] & 0xF0) | (value & 15));
        else layer[at] = (byte) ((layer[at] & 0x0F) | ((value & 15) << 4));
    }

    static int nibble(byte[] layer, int index) {
        int packed = layer[index >> 1] & 255;
        return (index & 1) == 0 ? packed & 15 : packed >> 4;
    }
}
