package dev.theredja.src2mc.bundle;

import java.util.Map;

/** Validated Source PVS data. Camera lookup walks exported BSP planes: leaf
 * AABBs are deliberately not used because conservative boxes can overlap. */
public final class PropVisibility {
    private static final float PLANE_EPSILON = 0.001f;
    private final int clusterCount;
    private final Node[] nodes;
    private final int root;
    private final byte[][] rows;
    private final Map<SectionKey, short[]> sections;

    public PropVisibility(int clusterCount, Node[] nodes, int root, byte[][] rows, Map<SectionKey, short[]> sections) {
        this.clusterCount = clusterCount;
        this.nodes = nodes.clone();
        this.root = root;
        this.rows = rows.clone();
        this.sections = Map.copyOf(sections);
    }

    public byte[] row(int cluster) { return cluster < 0 || cluster >= clusterCount ? null : rows[cluster]; }

    /** Exact point-leaf traversal. Points on a split fail open rather than
     * selecting an arbitrary side and incorrectly rejecting visible props. */
    public int clusterAt(int x, int y, int z) {
        int cursor = root;
        for (int steps = 0; steps <= nodes.length; steps++) {
            if (cursor < 0) {
                if (cursor == Integer.MIN_VALUE) return -1;
                int cluster = -1 - cursor;
                return cluster >= 0 && cluster < clusterCount ? cluster : -1;
            }
            if (cursor >= nodes.length) return -1;
            Node node = nodes[cursor];
            float distance = node.nx * x + node.ny * y + node.nz * z - node.dist;
            if (!Float.isFinite(distance) || Math.abs(distance) <= PLANE_EPSILON) return -1;
            cursor = node.children[distance > 0.0f ? 0 : 1];
        }
        return -1;
    }

    public short[] sectionClusters(int x, int y, int z) { return sections.get(new SectionKey(x, y, z)); }

    public boolean visible(byte[] row, short[] clusters) {
        if (clusters == null || clusters.length == 0) return true;
        for (short cluster : clusters) {
            int index = cluster & 0xFFFF;
            if ((row[index >> 3] & (1 << (index & 7))) != 0) return true;
        }
        return false;
    }

    public record Node(float nx, float ny, float nz, float dist, int[] children) {
        public Node { children = children.clone(); }
    }
    public record SectionKey(int x, int y, int z) {}
}
