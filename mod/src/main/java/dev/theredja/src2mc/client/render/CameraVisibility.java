package dev.theredja.src2mc.client.render;

import dev.theredja.src2mc.bundle.BundleGeneration;
import dev.theredja.src2mc.bundle.BundleMap;
import dev.theredja.src2mc.bundle.PropVisibility;
import dev.theredja.src2mc.network.PlacementNetwork;
import dev.theredja.src2mc.world.MapPlacement;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;

/**
 * Resolves which map placement the camera is currently inside and that
 * placement's PVS row/cluster, once per camera block move. Shared by every
 * mod-owned renderer that wants to reject sections/props/faces the current
 * BSP leaf can't see; the resolution itself is placement-level, not tied to
 * props or surfaces specifically.
 *
 * <p>Every unresolved case — camera outside any map placement, no visibility
 * table, solid leaf, or unknown cluster — leaves the row null and callers
 * must render without PVS rejection (fail open).
 */
final class CameraVisibility {
    private static PropVisibility table;
    private static MapPlacement placement;
    private static byte[] row;
    private static int cluster = -1;
    private static int cameraX = Integer.MIN_VALUE;
    private static int cameraY = Integer.MIN_VALUE;
    private static int cameraZ = Integer.MIN_VALUE;

    private CameraVisibility() {}

    static void resolve(BundleGeneration generation, Minecraft minecraft) {
        var camera = minecraft.gameRenderer.getMainCamera().getPosition();
        BlockPos cameraBlock = BlockPos.containing(camera.x, camera.y, camera.z);
        if (cameraBlock.getX() == cameraX && cameraBlock.getY() == cameraY && cameraBlock.getZ() == cameraZ) return;
        cameraX = cameraBlock.getX(); cameraY = cameraBlock.getY(); cameraZ = cameraBlock.getZ();
        table = null; placement = null; row = null; cluster = -1;
        var resolvedPlacement = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).at(cameraBlock).orElse(null);
        if (resolvedPlacement == null) return;
        BundleMap map = generation.findMap(resolvedPlacement.campaignId(), resolvedPlacement.mapId()).orElse(null);
        PropVisibility resolvedTable = map == null ? null : map.pvs();
        if (resolvedTable == null) return;
        BlockPos local = resolvedPlacement.toLocal(cameraBlock);
        int resolvedCluster = resolvedTable.clusterAt(local.getX(), local.getY(), local.getZ());
        if (resolvedCluster < 0) return;
        byte[] resolvedRow = resolvedTable.row(resolvedCluster);
        if (resolvedRow == null) return;
        table = resolvedTable; placement = resolvedPlacement; row = resolvedRow; cluster = resolvedCluster;
    }

    static void reset() {
        table = null; placement = null; row = null; cluster = -1;
        cameraX = Integer.MIN_VALUE; cameraY = Integer.MIN_VALUE; cameraZ = Integer.MIN_VALUE;
    }

    static PropVisibility table() { return table; }
    static MapPlacement placement() { return placement; }
    static byte[] row() { return row; }
    static int cluster() { return cluster; }
}
