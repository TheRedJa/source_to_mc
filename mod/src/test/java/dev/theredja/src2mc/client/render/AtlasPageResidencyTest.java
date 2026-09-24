package dev.theredja.src2mc.client.render;

import static org.junit.jupiter.api.Assertions.assertEquals;

import dev.theredja.src2mc.bundle.AtlasIndex;
import java.util.List;
import org.junit.jupiter.api.Test;

final class AtlasPageResidencyTest {
    @Test
    void accountsForEveryProvidedMipWithoutAssumingSquarePages() {
        var page = new AtlasIndex.Page(0, List.of(
            new AtlasIndex.Mip(0, "a", 4096, 2048),
            new AtlasIndex.Mip(1, "b", 2048, 1024),
            new AtlasIndex.Mip(2, "c", 1024, 512)
        ));
        assertEquals(44_040_192L, AtlasPageResidency.decodedBytes(page));
    }
}
