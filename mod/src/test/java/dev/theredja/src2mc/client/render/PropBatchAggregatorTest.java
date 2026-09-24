package dev.theredja.src2mc.client.render;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.Set;
import org.junit.jupiter.api.Test;

final class PropBatchAggregatorTest {
    @Test
    void registerCreatesAggregateTracksContributorsAndMarksDirty() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        aggregates.register("a", "p1");
        aggregates.register("a", "p2");
        aggregates.register("b", "p1");
        assertEquals(2, aggregates.size());
        assertEquals(Set.of("p1", "p2"), aggregates.contributors("a"));
        assertEquals(Set.of("p1"), aggregates.contributors("b"));
        assertEquals(List.of("a", "b"), aggregates.dirtyKeys().stream().sorted().toList());
        assertEquals(2, aggregates.dirtyCount());
        assertNull(aggregates.value("a"));
    }

    @Test
    void unknownKeyQueriesAreEmpty() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        assertEquals(Set.of(), aggregates.contributors("missing"));
        assertNull(aggregates.value("missing"));
        assertFalse(aggregates.unregister("missing", "p1"));
        assertEquals(0, aggregates.dirtyCount());
        aggregates.rebuildComplete("missing", 1);
        assertEquals(0, aggregates.size());
    }

    @Test
    void rebuildCompletePublishesValueAndClearsDirty() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        aggregates.register("a", "p1");
        aggregates.rebuildComplete("a", 7);
        assertEquals(7, aggregates.value("a"));
        assertEquals(0, aggregates.dirtyCount());
        assertTrue(aggregates.keys().contains("a"));
    }

    @Test
    void emptyAggregateIsDeletedOnlyAfterRebuildCompletes() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        aggregates.register("a", "p1");
        aggregates.rebuildComplete("a", 7);
        assertTrue(aggregates.unregister("a", "p1"));
        assertEquals(1, aggregates.size());
        assertEquals(7, aggregates.value("a"));
        assertTrue(aggregates.dirtyKeys().contains("a"));
        aggregates.rebuildComplete("a", null);
        assertEquals(0, aggregates.size());
        assertNull(aggregates.value("a"));
        assertFalse(aggregates.dirtyKeys().contains("a"));
    }

    @Test
    void aggregateWithRemainingContributorsSurvivesRebuild() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        aggregates.register("a", "p1");
        aggregates.register("a", "p2");
        aggregates.rebuildComplete("a", 3);
        assertTrue(aggregates.unregister("a", "p1"));
        aggregates.rebuildComplete("a", 4);
        assertEquals(Set.of("p2"), aggregates.contributors("a"));
        assertEquals(4, aggregates.value("a"));
        assertEquals(1, aggregates.size());
    }

    @Test
    void unregisterIsIdempotentPerFrame() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        aggregates.register("a", "p1");
        aggregates.rebuildComplete("a", 1);
        assertTrue(aggregates.unregister("a", "p1"));
        assertFalse(aggregates.unregister("a", "p1"));
        assertEquals(1, aggregates.dirtyCount());
    }

    @Test
    void reregisterAfterDeleteRecreatesTheAggregate() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        aggregates.register("a", "p1");
        aggregates.rebuildComplete("a", 1);
        aggregates.unregister("a", "p1");
        aggregates.rebuildComplete("a", null);
        aggregates.register("a", "p2");
        assertEquals(Set.of("p2"), aggregates.contributors("a"));
        assertNull(aggregates.value("a"));
        assertTrue(aggregates.dirtyKeys().contains("a"));
    }

    @Test
    void clearRemovesEverything() {
        var aggregates = new PropBatchAggregator<String, String, Integer>();
        aggregates.register("a", "p1");
        aggregates.rebuildComplete("a", 5);
        aggregates.clear();
        assertEquals(0, aggregates.size());
        assertEquals(0, aggregates.dirtyCount());
        assertTrue(aggregates.values().isEmpty());
    }
}
