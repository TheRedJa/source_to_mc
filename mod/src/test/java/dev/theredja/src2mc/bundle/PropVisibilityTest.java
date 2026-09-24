package dev.theredja.src2mc.bundle;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Map;
import org.junit.jupiter.api.Test;

final class PropVisibilityTest {
    private static PropVisibility table() {
        byte[][] rows = {
            {(byte) 0b0000_0101, 0},  // cluster 0 sees 0 and 2
            {(byte) 0b0000_0010, 0},  // cluster 1 sees 1
            {(byte) 0b0000_0001, 0},  // cluster 2 sees 0
        };
        PropVisibility.Node[] nodes = {
            // x > 15.5 is cluster 1; otherwise resolve low-X by Y.
            new PropVisibility.Node(1, 0, 0, 15.5f, new int[]{-2, 1}),
            new PropVisibility.Node(0, 1, 0, 15.5f, new int[]{-3, -1}),
        };
        Map<PropVisibility.SectionKey, short[]> sections = Map.of(
            new PropVisibility.SectionKey(0, 0, 0), new short[]{0},
            new PropVisibility.SectionKey(1, 0, 0), new short[]{0, 1},
            new PropVisibility.SectionKey(2, 0, 0), new short[0]);
        return new PropVisibility(3, nodes, 0, rows, sections);
    }

    @Test
    void clusterAtWalksTheExactBspTree() {
        var table = table();
        assertEquals(0, table.clusterAt(0, 0, 0));
        assertEquals(0, table.clusterAt(15, 15, 15));
        assertEquals(1, table.clusterAt(16, 0, 0));
        assertEquals(2, table.clusterAt(8, 16, 8));
        assertEquals(1, table.clusterAt(40, 0, 0));
    }

    @Test
    void visibleFailsOpenForMissingSectionsAndEmptySets() {
        var table = table();
        byte[] row = table.row(1);
        assertTrue(table.visible(row, null));
        assertTrue(table.visible(row, new short[0]));
    }

    @Test
    void visibleTestsLsbFirstClusterBits() {
        var table = table();
        assertTrue(table.visible(table.row(0), new short[]{0}));
        assertTrue(table.visible(table.row(0), new short[]{2}));
        assertFalse(table.visible(table.row(0), new short[]{1}));
        assertTrue(table.visible(table.row(1), new short[]{1}));
        assertFalse(table.visible(table.row(2), new short[]{1, 2}));
    }

    @Test
    void rowOutOfRangeReturnsNull() {
        assertNull(table().row(-1));
        assertNull(table().row(3));
    }

    @Test
    void sectionClusterSetsAreCheckedAgainstTheCameraRow() {
        var table = table();
        // Cluster 1's row only marks cluster 1 visible; section (0,0,0) is cluster 0 only.
        assertFalse(table.visible(table.row(1), table.sectionClusters(0, 0, 0)));
        // Section (1,0,0) spans clusters 0 and 1, so cluster 1's camera sees it.
        assertTrue(table.visible(table.row(1), table.sectionClusters(1, 0, 0)));
    }
}
