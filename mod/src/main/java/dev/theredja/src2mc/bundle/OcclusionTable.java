package dev.theredja.src2mc.bundle;

import java.util.Map;

/**
 * Map-local cells that block light while holding no block.
 *
 * A brush too thin to voxelize honestly is drawn as geometry instead, which
 * leaves its cells empty. Minecraft only ever blocks light with a block, so
 * without this a ceiling built from thin plates casts no shade at all. One bit
 * per cell, in the same 16-block sections the surface table uses.
 */
public record OcclusionTable(Map<SurfaceTable.SectionPos, byte[]> sections) {
    /** 16x16x16 bits. */
    public static final int SECTION_BYTES = 512;

    public OcclusionTable {
        sections = sections.entrySet().stream().collect(java.util.stream.Collectors.toUnmodifiableMap(
            Map.Entry::getKey, entry -> entry.getValue().clone()));
    }

    /** Whether the map-local cell blocks light. */
    public boolean opaqueAt(int x, int y, int z) {
        byte[] bits = sections.get(new SurfaceTable.SectionPos(x >> 4, y >> 4, z >> 4));
        return bits != null && test(bits, x, y, z);
    }

    /** Whether the bit for a cell is set in an already-resolved section. */
    public static boolean test(byte[] bits, int x, int y, int z) {
        int index = ((y & 15) << 8) | ((z & 15) << 4) | (x & 15);
        return (bits[index >> 3] & (1 << (index & 7))) != 0;
    }

    public int cellCount() {
        int total = 0;
        for (byte[] bits : sections.values()) for (byte value : bits) total += Integer.bitCount(value & 255);
        return total;
    }

    @Override
    public boolean equals(Object other) {
        if (!(other instanceof OcclusionTable table) || !sections.keySet().equals(table.sections.keySet())) return false;
        for (Map.Entry<SurfaceTable.SectionPos, byte[]> entry : sections.entrySet()) {
            if (!java.util.Arrays.equals(entry.getValue(), table.sections.get(entry.getKey()))) return false;
        }
        return true;
    }

    @Override
    public int hashCode() {
        int result = 0;
        // Order-independent, because the section map is unordered.
        for (Map.Entry<SurfaceTable.SectionPos, byte[]> entry : sections.entrySet()) {
            result += entry.getKey().hashCode() * 31 + java.util.Arrays.hashCode(entry.getValue());
        }
        return result;
    }
}
