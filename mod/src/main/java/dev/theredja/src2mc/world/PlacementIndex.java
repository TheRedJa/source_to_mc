package dev.theredja.src2mc.world;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Optional;
import net.minecraft.core.BlockPos;

/** Deterministic immutable-friendly placement set with ambiguity rejection. */
public final class PlacementIndex {
    private final List<MapPlacement> placements = new ArrayList<>();

    public Registration register(MapPlacement placement) {
        int replacement = -1;
        for (int i = 0; i < placements.size(); i++) {
            MapPlacement current = placements.get(i);
            if (current.anchorWorld().equals(placement.anchorWorld())) {
                if (current.equals(placement)) return Registration.UNCHANGED;
                replacement = i;
                break;
            }
        }
        for (int i = 0; i < placements.size(); i++) {
            if (i != replacement && placement.overlaps(placements.get(i))) return Registration.OVERLAP;
        }
        if (replacement >= 0) placements.remove(replacement);
        placements.add(placement);
        placements.sort(Comparator.comparingLong(value -> value.anchorWorld().asLong()));
        return Registration.ADDED;
    }

    public Optional<MapPlacement> at(BlockPos world) {
        MapPlacement found = null;
        for (MapPlacement placement : placements) if (placement.contains(world)) {
            if (found != null) return Optional.empty();
            found = placement;
        }
        return Optional.ofNullable(found);
    }

    public List<MapPlacement> view() { return List.copyOf(placements); }
    /** Count without copying, for per-tick change checks. */
    public int size() { return placements.size(); }
    public enum Registration { ADDED, UNCHANGED, OVERLAP }
}
