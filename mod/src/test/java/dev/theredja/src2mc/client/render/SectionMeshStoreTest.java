package dev.theredja.src2mc.client.render;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.util.Map;
import org.junit.jupiter.api.Test;

final class SectionMeshStoreTest {
    @Test
    void replacementClosesOnlyResourcesThatAreNoLongerRetained() {
        var store = new SectionMeshStore<Tracked>();
        var retained = new Tracked();
        var removed = new Tracked();
        var added = new Tracked();
        var retainedKey = new SectionMeshStore.Key(0, 4, 0, 0);
        var removedKey = new SectionMeshStore.Key(1, 4, 0, 1);

        store.replace(Map.of(retainedKey, retained, removedKey, removed));
        store.replace(Map.of(retainedKey, retained, new SectionMeshStore.Key(2, 4, 0, 0), added));

        assertEquals(0, retained.closed);
        assertEquals(1, removed.closed);
        assertEquals(0, added.closed);
        store.close();
        assertEquals(1, retained.closed);
        assertEquals(1, added.closed);
    }

    @Test
    void texturePageCannotBeNegative() {
        assertThrows(IllegalArgumentException.class, () -> new SectionMeshStore.Key(0, 0, 0, -1));
    }

    private static final class Tracked implements AutoCloseable {
        int closed;

        @Override
        public void close() {
            closed++;
        }
    }
}
