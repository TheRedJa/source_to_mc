package dev.theredja.src2mc.bundle;

import java.nio.file.Path;
import java.util.List;

/** Fully hash-verified bootstrap metadata for one bundle. */
public record BundleManifest(
    Path path,
    String campaignId,
    String fingerprint,
    List<Entry> entries,
    long uncompressedBytes,
    List<BundleMap> maps
) {
    public BundleManifest {
        path = path.toAbsolutePath().normalize();
        entries = List.copyOf(entries);
        maps = List.copyOf(maps);
    }

    public record Entry(String path, long size, String sha256) {
    }
}
