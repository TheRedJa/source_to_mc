package dev.theredja.src2mc.client.render;

import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;

/**
 * Render-thread-owned bookkeeping for merged prop batches. Contributors are
 * registered per aggregate key; mesh values are assigned only by the
 * renderer's budgeted rebuild pass, so GL resources never enter this class
 * and the lifecycle logic stays unit-testable without Minecraft.
 */
final class PropBatchAggregator<K, P, M> {
    private final Map<K, Entry<P, M>> entries = new HashMap<>();
    private final Set<K> dirty = new HashSet<>();

    private static final class Entry<P, M> {
        final Set<P> contributors = new HashSet<>();
        M value;
    }

    /** Registers a contributor, creating the aggregate when absent, and marks it dirty. */
    void register(K key, P contributor) {
        entries.computeIfAbsent(key, ignored -> new Entry<>()).contributors.add(contributor);
        dirty.add(key);
    }

    /** Removes a contributor and marks the aggregate dirty. @return true when newly dirtied. */
    boolean unregister(K key, P contributor) {
        Entry<P, M> entry = entries.get(key);
        if (entry == null) return false;
        entry.contributors.remove(contributor);
        return dirty.add(key);
    }

    /** Marks an existing aggregate dirty without changing its contributors, e.g. on a relight. */
    void markDirty(K key) {
        if (entries.containsKey(key)) dirty.add(key);
    }

    Set<P> contributors(K key) {
        Entry<P, M> entry = entries.get(key);
        return entry == null ? Set.of() : entry.contributors;
    }

    M value(K key) {
        Entry<P, M> entry = entries.get(key);
        return entry == null ? null : entry.value;
    }

    /** Publishes a rebuilt value, clears the dirty mark, and deletes aggregates with no contributors. */
    void rebuildComplete(K key, M value) {
        dirty.remove(key);
        Entry<P, M> entry = entries.get(key);
        if (entry == null) return;
        entry.value = value;
        if (entry.contributors.isEmpty()) entries.remove(key);
    }

    List<K> dirtyKeys() { return new ArrayList<>(dirty); }
    int dirtyCount() { return dirty.size(); }
    Set<K> keys() { return entries.keySet(); }

    Collection<M> values() {
        return entries.values().stream().map(entry -> entry.value).filter(Objects::nonNull).toList();
    }

    int size() { return entries.size(); }

    void clear() {
        entries.clear();
        dirty.clear();
    }
}
