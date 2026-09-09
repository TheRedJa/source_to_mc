package dev.theredja.src2mc.world;

import dev.theredja.src2mc.bundle.BundleGeneration;
import dev.theredja.src2mc.bundle.SurfaceTable;
import java.util.List;
import net.minecraft.core.BlockPos;

/** Common-side lookup used by the future client renderer and diagnostics. */
public final class SurfaceLookup {
    private SurfaceLookup() {}

    public static Result resolve(BundleGeneration generation, PlacementIndex placements, BlockPos world) {
        MapPlacement placement = placements.at(world).orElse(null);
        if (placement == null) return new Result(Status.NO_PLACEMENT, null, List.of());
        var map = generation.findMap(placement.campaignId(), placement.mapId()).orElse(null);
        if (map == null) return new Result(Status.MISSING_MAP, placement, List.of());
        BlockPos local = placement.toLocal(world);
        return new Result(Status.RESOLVED, placement, map.surfaces().facesAt(local.getX(), local.getY(), local.getZ()));
    }

    public enum Status { RESOLVED, NO_PLACEMENT, MISSING_MAP }
    public record Result(Status status, MapPlacement placement, List<SurfaceTable.Face> faces) {
        public Result { faces = List.copyOf(faces); }
    }
}
