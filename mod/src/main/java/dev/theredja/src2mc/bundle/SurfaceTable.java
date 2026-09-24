package dev.theredja.src2mc.bundle;

import java.util.List;
import java.util.Map;

/** Immutable sparse map-local surface lookup retained by a validated generation. */
public record SurfaceTable(List<UvRegion> uvRegions, Map<SectionPos, List<Face>> sections) {
    public SurfaceTable {
        uvRegions = List.copyOf(uvRegions);
        sections = sections.entrySet().stream().collect(java.util.stream.Collectors.toUnmodifiableMap(
            Map.Entry::getKey, entry -> List.copyOf(entry.getValue())));
    }

    public List<Face> facesAt(int x, int y, int z) {
        List<Face> bucket = sections.get(new SectionPos(x >> 4, y >> 4, z >> 4));
        if (bucket == null) return List.of();
        int local = ((y & 15) << 8) | ((z & 15) << 4) | (x & 15);
        return bucket.stream().filter(face -> face.localCell() == local).toList();
    }

    public record SectionPos(int x, int y, int z) {}
    public record Face(int localCell, int patch, int provenance, int materialId, int uvRegionId,
                       long sourcePrimary, long sourceSecondary) {}
    public record UvRegion(double[] values) {
        public UvRegion { values = values.clone(); }
        @Override public double[] values() { return values.clone(); }
        @Override public boolean equals(Object other) { return other instanceof UvRegion uv && java.util.Arrays.equals(values, uv.values); }
        @Override public int hashCode() { return java.util.Arrays.hashCode(values); }
    }
}
