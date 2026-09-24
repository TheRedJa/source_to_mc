package dev.theredja.src2mc.world;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import dev.theredja.src2mc.bundle.BundleGeneration;
import dev.theredja.src2mc.bundle.BundleManifest;
import dev.theredja.src2mc.bundle.BundleMap;
import dev.theredja.src2mc.bundle.OcclusionTable;
import dev.theredja.src2mc.bundle.SurfaceTable;
import java.nio.file.Path;
import java.time.Instant;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.core.BlockPos;
import org.junit.jupiter.api.Test;

final class SkyLightBakeTest {
    private static final int SIZE = 16;

    /** A map 16 cells on a side, anchored at its own corner. */
    private static BundleMap map(Map<SurfaceTable.SectionPos, byte[]> occlusion) {
        return new BundleMap("m", "m.bsp", new int[]{0, 0, 0}, new int[]{SIZE - 1, SIZE - 1, SIZE - 1},
            new int[]{0, 0, 0}, List.of(), List.of(), List.of(), false,
            new SurfaceTable(List.of(), Map.of()), java.util.Set.of(), null, null,
            occlusion == null ? null : new OcclusionTable(occlusion));
    }

    /** Occluding cells, map-local, in the exporter's bit order. */
    private static Map<SurfaceTable.SectionPos, byte[]> occluding(int[]... cells) {
        Map<SurfaceTable.SectionPos, byte[]> sections = new HashMap<>();
        for (int[] cell : cells) {
            byte[] bits = sections.computeIfAbsent(
                new SurfaceTable.SectionPos(cell[0] >> 4, cell[1] >> 4, cell[2] >> 4),
                key -> new byte[OcclusionTable.SECTION_BYTES]);
            int index = ((cell[1] & 15) << 8) | ((cell[2] & 15) << 4) | (cell[0] & 15);
            bits[index >> 3] |= (byte) (1 << (index & 7));
        }
        return sections;
    }

    private static SkyLightBake.Baked bake(BundleMap map) {
        MapPlacement placement = MapPlacement.fromAnchor("c", map, new BlockPos(0, 0, 0));
        BundleGeneration generation = new BundleGeneration(1, "f", Instant.EPOCH, List.of(
            new BundleManifest(Path.of("bundle.src2mc"), "c", "f", List.of(), 0, List.of(map))));
        return SkyLightBake.bake(List.of(placement), generation, -64, 320);
    }

    /**
     * A sealed room: floor, four walls and a ceiling of occluding cells, with
     * an optional hole in the ceiling. Nothing here holds a block, which is the
     * whole point -- this is a map built entirely of drawn geometry.
     */
    private static BundleMap room(boolean hole) {
        java.util.List<int[]> cells = new java.util.ArrayList<>();
        for (int x = 0; x < SIZE; x++) {
            for (int z = 0; z < SIZE; z++) {
                cells.add(new int[]{x, 0, z});
                if (!hole || x != 8 || z != 8) cells.add(new int[]{x, 10, z});
                for (int y = 1; y < 10; y++) {
                    if (x == 0 || x == SIZE - 1 || z == 0 || z == SIZE - 1) cells.add(new int[]{x, y, z});
                }
            }
        }
        return map(occluding(cells.toArray(new int[0][])));
    }

    /** A ceiling of occluding cells must shade the room under it. */
    @Test
    void aSealedRoomOfOccludingCellsIsDark() {
        SkyLightBake.Baked baked = bake(room(false));

        assertEquals(0, baked.at(8, 9, 8), "directly under the ceiling");
        assertEquals(0, baked.at(1, 1, 1), "the far corner of the floor");
        assertEquals(15, baked.at(8, 11, 8), "above the ceiling is open sky");
        assertTrue(baked.darkCells() > 0);
    }

    /** A hole in the ceiling lets daylight in, falling off one step per cell. */
    @Test
    void aHoleInTheCeilingLetsLightThrough() {
        SkyLightBake.Baked baked = bake(room(true));

        assertEquals(15, baked.at(8, 9, 8), "straight down through the hole keeps full daylight");
        assertEquals(14, baked.at(7, 9, 8), "one step sideways costs one level");
        assertEquals(13, baked.at(6, 9, 8));
        // Fourteen steps from the hole, which is as far as daylight reaches.
        assertEquals(1, baked.at(1, 1, 1), "the far corner is barely lit");
    }

    /** Open ground beside a map must keep the daylight vanilla would give it. */
    @Test
    void daylightOutsideTheRoomIsUntouched() {
        SkyLightBake.Baked baked = bake(room(false));

        assertEquals(15, baked.at(0, 12, 0), "above the roof line, clear of the walls");
    }

    /** A map with nothing to stop daylight must leave every cell at full sky. */
    @Test
    void aMapWithNoOccludersBakesToOpenSky() {
        SkyLightBake.Baked baked = bake(map(null));

        assertEquals(0, baked.darkCells());
        assertEquals(15, baked.at(8, 8, 8));
        assertTrue(baked.covers(0, 0), "the placement's own chunk is still published");
    }

    /**
     * The real thing, when one is offered: a converted map must bake without
     * running out of time or memory, and must come out with rooms in it.
     */
    @Test
    void aConvertedBundleBakes() throws Exception {
        String path = System.getProperty("src2mc.testBundle");
        org.junit.jupiter.api.Assumptions.assumeTrue(path != null,
            "set -Dsrc2mc.testBundle to bake a converted map");
        BundleManifest manifest = new dev.theredja.src2mc.bundle.BundleValidator().validate(Path.of(path));
        BundleGeneration generation = new BundleGeneration(1, manifest.fingerprint(), Instant.EPOCH, List.of(manifest));
        BundleMap first = manifest.maps().getFirst();
        MapPlacement placement = MapPlacement.fromAnchor(manifest.campaignId(), first,
            new BlockPos(first.anchorCell()[0], first.anchorCell()[1], first.anchorCell()[2]));

        SkyLightBake.Baked baked = SkyLightBake.bake(List.of(placement), generation, -64, 320);

        System.out.println("baked " + first.mapId() + ": " + baked.darkCells() + " shaded, "
            + baked.litCells() + " open, " + baked.sections().size() + " sections, " + baked.millis() + " ms");
        assertTrue(baked.darkCells() > 0, "a converted map with a roof must shade something");
        assertTrue(baked.sections().size() > 0);
    }

    /** The nibble packing must match the data layer the light engine reads. */
    @Test
    void sectionsAreWrittenInDataLayerOrder() {
        SkyLightBake.Baked baked = bake(room(false));
        byte[] layer = baked.sections().get(net.minecraft.core.SectionPos.asLong(0, 0, 0));

        assertEquals(baked.at(3, 9, 5), new net.minecraft.world.level.chunk.DataLayer(layer.clone()).get(3, 9, 5));
    }
}
