package dev.theredja.src2mc.bundle;

import java.time.Instant;
import java.util.List;

/** Immutable, fully validated set published as one unit. */
public record BundleGeneration(long sequence, String fingerprint, Instant loadedAt, List<BundleManifest> bundles) {
    public BundleGeneration {
        bundles = List.copyOf(bundles);
    }

    public static BundleGeneration empty() {
        return new BundleGeneration(0, "none", Instant.EPOCH, List.of());
    }

    public java.util.Optional<BundleMap> findMap(String campaignId, String mapId) {
        return findLocatedMap(campaignId, mapId).map(LocatedMap::map);
    }

    public java.util.Optional<LocatedMap> findLocatedMap(String campaignId, String mapId) {
        for (BundleManifest bundle : bundles) {
            if (!bundle.campaignId().equals(campaignId)) continue;
            for (BundleMap map : bundle.maps()) {
                if (map.mapId().equals(mapId)) return java.util.Optional.of(new LocatedMap(bundle, map));
            }
        }
        return java.util.Optional.empty();
    }

    public record LocatedMap(BundleManifest bundle, BundleMap map) {}
}
