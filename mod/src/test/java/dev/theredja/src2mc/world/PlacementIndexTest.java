package dev.theredja.src2mc.world;

import static org.junit.jupiter.api.Assertions.assertEquals;

import dev.theredja.src2mc.bundle.BundleMap;
import dev.theredja.src2mc.bundle.SurfaceTable;
import java.util.List;
import java.util.Map;
import net.minecraft.core.BlockPos;
import org.junit.jupiter.api.Test;

final class PlacementIndexTest {
    private static BundleMap map(String id, int width) {
        return new BundleMap(id, id + ".bsp", new int[]{0, 0, 0}, new int[]{width - 1, 3, 3},
            new int[]{0, -1, 0}, List.of(new dev.theredja.src2mc.bundle.BundleMaterial(
                "test", dev.theredja.src2mc.bundle.BundleMaterial.RenderClass.FALLBACK, null)),
            List.of(), List.of(), false, new SurfaceTable(List.of(), Map.of()), java.util.Set.of(), null, null, null);
    }

    @Test void twoInstancesOfOneMapResolveIndependently() {
        PlacementIndex index = new PlacementIndex();
        MapPlacement first = MapPlacement.fromAnchor("hl2", map("d1", 4), new BlockPos(10, 19, 10));
        MapPlacement second = MapPlacement.fromAnchor("hl2", map("d1", 4), new BlockPos(30, 19, 10));
        assertEquals(PlacementIndex.Registration.ADDED, index.register(first));
        assertEquals(PlacementIndex.Registration.ADDED, index.register(second));
        assertEquals(first, index.at(new BlockPos(12, 20, 12)).orElseThrow());
        assertEquals(second, index.at(new BlockPos(32, 20, 12)).orElseThrow());
    }

    @Test void twoDifferentMapsResolveIndependently() {
        PlacementIndex index = new PlacementIndex();
        MapPlacement first = MapPlacement.fromAnchor("hl2", map("d1", 4), new BlockPos(0, -1, 0));
        MapPlacement second = MapPlacement.fromAnchor("hl2", map("d2", 5), new BlockPos(20, -1, 0));
        index.register(first); index.register(second);
        assertEquals("d1", index.at(new BlockPos(1, 1, 1)).orElseThrow().mapId());
        assertEquals("d2", index.at(new BlockPos(22, 1, 1)).orElseThrow().mapId());
    }

    @Test void overlapIsRejectedWithoutDestroyingPriorPlacement() {
        PlacementIndex index = new PlacementIndex();
        MapPlacement first = MapPlacement.fromAnchor("hl2", map("d1", 8), new BlockPos(0, -1, 0));
        MapPlacement overlap = MapPlacement.fromAnchor("hl2", map("d2", 8), new BlockPos(4, -1, 0));
        assertEquals(PlacementIndex.Registration.ADDED, index.register(first));
        assertEquals(PlacementIndex.Registration.OVERLAP, index.register(overlap));
        assertEquals(List.of(first), index.view());
    }
}
