package dev.theredja.src2mc.world;

import dev.theredja.src2mc.bundle.BundleMap;
import net.minecraft.core.BlockPos;

/** One authoritative, translation-only map placement. */
public record MapPlacement(String campaignId, String mapId, BlockPos anchorWorld, BlockPos translation,
                           BlockPos worldMin, BlockPos worldMax) {
    public static MapPlacement fromAnchor(String campaignId, BundleMap map, BlockPos anchorWorld) {
        int[] anchor = map.anchorCell();
        BlockPos translation = anchorWorld.subtract(new BlockPos(anchor[0], anchor[1], anchor[2]));
        int[] min = map.cellMin(), max = map.cellMax();
        return new MapPlacement(campaignId, map.mapId(), anchorWorld.immutable(), translation,
            translation.offset(min[0], min[1], min[2]), translation.offset(max[0], max[1], max[2]));
    }

    public boolean contains(BlockPos position) {
        return position.getX() >= worldMin.getX() && position.getX() <= worldMax.getX()
            && position.getY() >= worldMin.getY() && position.getY() <= worldMax.getY()
            && position.getZ() >= worldMin.getZ() && position.getZ() <= worldMax.getZ();
    }

    public boolean overlaps(MapPlacement other) {
        return worldMin.getX() <= other.worldMax.getX() && worldMax.getX() >= other.worldMin.getX()
            && worldMin.getY() <= other.worldMax.getY() && worldMax.getY() >= other.worldMin.getY()
            && worldMin.getZ() <= other.worldMax.getZ() && worldMax.getZ() >= other.worldMin.getZ();
    }

    public BlockPos toLocal(BlockPos world) { return world.subtract(translation); }
}
