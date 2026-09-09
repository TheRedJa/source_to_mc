package dev.theredja.src2mc.client.render;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Owns one immutable generation of section/page resources. Callers perform all
 * mutations on the render thread; publishing a replacement closes every
 * resource that is no longer retained.
 */
public final class SectionMeshStore<T extends AutoCloseable> implements AutoCloseable {
    public record Key(int sectionX, int sectionY, int sectionZ, int page) {
        public Key {
            if (page < 0) {
                throw new IllegalArgumentException("texture page must be non-negative");
            }
        }
    }

    private Map<Key, T> meshes = Map.of();

    public Map<Key, T> snapshot() {
        return meshes;
    }

    public void replace(Map<Key, T> replacement) {
        var next = Map.copyOf(new LinkedHashMap<>(replacement));
        var previous = meshes;
        meshes = next;
        previous.forEach((key, resource) -> {
            if (next.get(key) != resource) {
                close(resource);
            }
        });
    }

    @Override
    public void close() {
        replace(Map.of());
    }

    private static void close(AutoCloseable resource) {
        try {
            resource.close();
        } catch (Exception exception) {
            throw new IllegalStateException("closing section mesh", exception);
        }
    }
}
