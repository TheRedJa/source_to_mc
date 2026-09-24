package dev.theredja.src2mc.client.render;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import dev.theredja.src2mc.bundle.AtlasIndex;
import dev.theredja.src2mc.bundle.BundleMaterial;
import dev.theredja.src2mc.bundle.SurfaceTable;
import java.util.List;
import org.junit.jupiter.api.Test;

final class SurfaceTessellatorTest {
    @Test
    void clipsOneFaceAcrossLosslessRegionsAndRemapsEachPage() {
        var face = new SurfaceTable.Face(0, 3, 0, 0, 0, 0, 0); // south, lower-left micro-face
        var uv = new SurfaceTable.UvRegion(new double[]{40, 0, 0, 4054, 0, 40, 0, 0});
        var material = new BundleMaterial.TextureReference("id", 8192, 8192, 8192, 8192);
        var texture = new AtlasIndex.Texture("id", 8192, 8192, List.of(
            new AtlasIndex.Region(new int[]{0, 0, 4064, 8192}, 0, new int[]{16, 16, 4064, 8192}),
            new AtlasIndex.Region(new int[]{4064, 0, 4128, 8192}, 1, new int[]{16, 16, 4128, 8192})
        ));
        var triangles = SurfaceTessellator.tessellate(0, 0, 0, face, uv, material, texture, 16384);
        assertTrue(triangles.stream().anyMatch(triangle -> triangle.page() == 0));
        assertTrue(triangles.stream().anyMatch(triangle -> triangle.page() == 1));
        assertEquals(4, triangles.size());
        assertTrue(triangles.stream().flatMap(t -> List.of(t.a(), t.b(), t.c()).stream())
            .allMatch(vertex -> vertex.u() >= 0 && vertex.u() <= 1 && vertex.v() >= 0 && vertex.v() <= 1));
    }

    @Test
    void preservesNegativeTextureRepeats() {
        var face = new SurfaceTable.Face(0, 3, 0, 0, 0, 0, 0);
        var uv = new SurfaceTable.UvRegion(new double[]{16, 0, 0, -4, 0, 16, 0, -4});
        var material = new BundleMaterial.TextureReference("id", 16, 16, 16, 16);
        var texture = new AtlasIndex.Texture("id", 16, 16,
            List.of(new AtlasIndex.Region(new int[]{0, 0, 16, 16}, 0, new int[]{16, 16, 16, 16})));
        var triangles = SurfaceTessellator.tessellate(0, 0, 0, face, uv, material, texture, 4096);
        assertTrue(triangles.size() >= 2);
        assertTrue(triangles.stream().flatMap(t -> List.of(t.a(), t.b(), t.c()).stream())
            .allMatch(vertex -> vertex.u() >= 16.0 / 4096 && vertex.u() <= 32.0 / 4096));
    }
}
