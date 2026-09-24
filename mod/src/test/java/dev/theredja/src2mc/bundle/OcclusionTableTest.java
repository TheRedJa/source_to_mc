package dev.theredja.src2mc.bundle;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Map;
import org.junit.jupiter.api.Test;

final class OcclusionTableTest {
    /** The exporter's bit order: X low, then Z, then Y. */
    private static byte[] withCell(int x, int y, int z) {
        byte[] bits = new byte[OcclusionTable.SECTION_BYTES];
        int index = ((y & 15) << 8) | ((z & 15) << 4) | (x & 15);
        bits[index >> 3] |= (byte) (1 << (index & 7));
        return bits;
    }

    @Test
    void aCellReadsBackSetAndItsNeighboursDoNot() {
        OcclusionTable table = new OcclusionTable(Map.of(
            new SurfaceTable.SectionPos(0, -2, 1), withCell(3, -20, 17)));

        assertTrue(table.opaqueAt(3, -20, 17));
        assertFalse(table.opaqueAt(4, -20, 17));
        assertFalse(table.opaqueAt(3, -19, 17));
        assertFalse(table.opaqueAt(3, -20, 18));
        assertEquals(1, table.cellCount());
    }

    /** Cells of an unlisted section must not fall through to another's bits. */
    @Test
    void anAbsentSectionIsTransparent() {
        OcclusionTable table = new OcclusionTable(Map.of(
            new SurfaceTable.SectionPos(0, 0, 0), withCell(1, 2, 3)));

        assertTrue(table.opaqueAt(1, 2, 3));
        assertFalse(table.opaqueAt(17, 2, 3));
        assertFalse(table.opaqueAt(1, 18, 3));
    }

    @Test
    void theTableCopiesTheBitsItIsGiven() {
        byte[] bits = withCell(0, 0, 0);
        OcclusionTable table = new OcclusionTable(Map.of(new SurfaceTable.SectionPos(0, 0, 0), bits));
        bits[0] = 0;

        assertTrue(table.opaqueAt(0, 0, 0));
    }
}
