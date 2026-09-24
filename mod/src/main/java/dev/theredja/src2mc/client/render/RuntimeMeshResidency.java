package dev.theredja.src2mc.client.render;

import dev.theredja.src2mc.bundle.BundleManifest;
import dev.theredja.src2mc.bundle.RuntimeMesh;
import java.io.IOException;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.zip.ZipFile;

/** Hash-rechecks and decodes shared prop meshes off the render thread. */
final class RuntimeMeshResidency implements AutoCloseable {
    private static final HexFormat HEX = HexFormat.of();
    private final ExecutorService decoder = Executors.newFixedThreadPool(2, task -> {
        Thread thread = new Thread(task, "src2mc-mesh-decoder"); thread.setDaemon(true); return thread;
    });
    private final Map<Key, CompletableFuture<RuntimeMesh>> meshes = new ConcurrentHashMap<>();
    private long generation = -1;

    Optional<RuntimeMesh> request(long nextGeneration, BundleManifest bundle, String contentId) {
        if (generation != nextGeneration) { meshes.clear(); generation = nextGeneration; }
        Key key = new Key(bundle.fingerprint(), contentId);
        CompletableFuture<RuntimeMesh> future = meshes.computeIfAbsent(key,
            ignored -> CompletableFuture.supplyAsync(() -> load(bundle, contentId), decoder));
        if (!future.isDone()) return Optional.empty();
        try { return Optional.of(future.join()); }
        catch (RuntimeException exception) { return Optional.empty(); }
    }

    Stats stats() {
        long pending = meshes.values().stream().filter(future -> !future.isDone()).count();
        long failed = meshes.values().stream().filter(CompletableFuture::isCompletedExceptionally).count();
        return new Stats(meshes.size() - pending - failed, pending, failed);
    }

    private static RuntimeMesh load(BundleManifest bundle, String contentId) {
        String path = "meshes/" + contentId + ".s2mesh";
        try (ZipFile zip = new ZipFile(bundle.path().toFile())) {
            BundleManifest.Entry entry = bundle.entries().stream().filter(candidate -> candidate.path().equals(path)).findFirst()
                .orElseThrow(() -> new IOException("mesh absent from validated manifest"));
            var zipEntry = zip.getEntry(path);
            if (zipEntry == null || zipEntry.getSize() != entry.size() || entry.size() > Integer.MAX_VALUE) throw new IOException("mesh changed after validation");
            byte[] bytes;
            try (var input = zip.getInputStream(zipEntry)) { bytes = input.readNBytes((int) entry.size() + 1); }
            if (bytes.length != entry.size() || !HEX.formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)).equals(entry.sha256())) throw new IOException("mesh hash changed after validation");
            return RuntimeMesh.decode(bytes);
        } catch (Exception exception) { throw new IllegalStateException("failed to load " + path, exception); }
    }

    @Override public void close() { meshes.clear(); decoder.shutdownNow(); }
    record Stats(long ready, long pending, long failed) {}
    private record Key(String fingerprint, String contentId) {}
}
