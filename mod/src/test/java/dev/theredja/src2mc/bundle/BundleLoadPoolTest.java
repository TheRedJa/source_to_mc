package dev.theredja.src2mc.bundle;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.stream.IntStream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;

final class BundleLoadPoolTest {
    private static void sleep(long millis) {
        try {
            Thread.sleep(millis);
        } catch (InterruptedException exception) {
            Thread.currentThread().interrupt();
        }
    }

    @Test
    void keepsResultsInInputOrderWhateverOrderTheyFinishIn() throws Exception {
        List<Integer> items = IntStream.range(0, 200).boxed().toList();
        List<Integer> results = BundleLoadPool.map(items, item -> {
            // Later items finish first, so an out-of-order collection shows up.
            sleep(items.size() - item);
            return item * 2;
        });
        assertEquals(items.stream().map(item -> item * 2).toList(), results);
    }

    @Test
    void reportsTheEarliestFailureEvenWhenALaterOneHappensFirst() throws Exception {
        // Which bundle a user is told about must not depend on thread timing.
        for (int attempt = 0; attempt < 20; attempt++) {
            IOException thrown = assertThrows(IOException.class, () ->
                BundleLoadPool.map(IntStream.range(0, 8).boxed().toList(), item -> {
                    if (item == 3) {
                        sleep(40);
                        throw new IOException("three");
                    }
                    if (item == 5) throw new IOException("five");
                    return item;
                }));
            assertEquals("three", thrown.getMessage());
        }
    }

    @Test
    @Timeout(30)
    void nestedParallelismMakesProgressWhateverThePoolsWidth() throws Exception {
        AtomicInteger leaves = new AtomicInteger();
        List<List<Integer>> outer = BundleLoadPool.map(IntStream.range(0, 16).boxed().toList(), item ->
            BundleLoadPool.map(IntStream.range(0, 16).boxed().toList(), inner -> {
                leaves.incrementAndGet();
                return item * 16 + inner;
            }));
        assertEquals(256, leaves.get());
        List<Integer> flattened = new ArrayList<>();
        outer.forEach(flattened::addAll);
        assertEquals(IntStream.range(0, 256).boxed().toList(), flattened);
    }

    @Test
    void runsSingletonAndEmptyListsWithoutTouchingThePool() throws Exception {
        assertEquals(List.of(), BundleLoadPool.map(List.<String>of(), item -> item));
        assertEquals(List.of("a"), BundleLoadPool.map(List.of("a"), item -> item));
        assertTrue(BundleLoadPool.parallelism() >= 1);
    }
}
