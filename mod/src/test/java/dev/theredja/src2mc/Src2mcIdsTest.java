package dev.theredja.src2mc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import org.junit.jupiter.api.Test;

final class Src2mcIdsTest {
    @Test
    void createsIdentifiersInTheModNamespace() {
        var id = Src2mcIds.id("texture_pages/page_0");

        assertEquals("src2mc", id.getNamespace());
        assertEquals("texture_pages/page_0", id.getPath());
    }

    @Test
    void rejectsInvalidResourcePaths() {
        assertThrows(RuntimeException.class, () -> Src2mcIds.id("Texture Page"));
    }
}
