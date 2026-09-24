package dev.theredja.src2mc.bundle;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Supplier;

/** Discovers bundles and atomically publishes only complete validated candidates. */
public final class BundleRepository {
    private static final byte[] GENERATION_DOMAIN = "src2mc-generation-v1\0".getBytes(StandardCharsets.US_ASCII);
    private final Supplier<Path> directory;
    private final BundleValidator validator;
    private final AtomicReference<BundleGeneration> active = new AtomicReference<>(BundleGeneration.empty());
    private final AtomicLong nextSequence = new AtomicLong(1);
    /** One load at a time: a startup load and a `/src2mc reload` must not interleave. */
    private final Object lock = new Object();
    private volatile CompletableFuture<BundleGeneration> pending;

    public BundleRepository(Supplier<Path> directory) {
        this(directory, new BundleValidator());
    }

    BundleRepository(Supplier<Path> directory, BundleValidator validator) {
        this.directory = directory;
        this.validator = validator;
    }

    public Path directory() {
        return directory.get().toAbsolutePath().normalize();
    }

    public BundleGeneration active() {
        return active.get();
    }

    /** Validate a candidate without changing active state. */
    public BundleGeneration validateCandidate() throws IOException {
        Path folder = directory();
        Files.createDirectories(folder);
        List<Path> paths;
        try (var stream = Files.list(folder)) {
            paths = stream
                .filter(Files::isRegularFile)
                .filter(path -> path.getFileName().toString().endsWith(".src2mc"))
                .sorted(Comparator.comparing(path -> path.getFileName().toString()))
                .toList();
        }
        BundleLoadProgress.started(paths.size());
        AtomicInteger done = new AtomicInteger();
        // Bundles are independent and the pool keeps their results in filename
        // order, so what is published is what a serial load would have published.
        List<BundleManifest> bundles = BundleLoadPool.map(paths, path -> {
            BundleManifest manifest = validator.validate(path);
            BundleLoadProgress.bundleDone(done.incrementAndGet(), manifest.campaignId());
            return manifest;
        });
        var campaigns = new HashSet<String>();
        for (BundleManifest manifest : bundles) {
            if (!campaigns.add(manifest.campaignId())) {
                throw new BundleValidationException(
                    BundleErrorCode.DUPLICATE_IDENTITY,
                    "campaign `" + manifest.campaignId() + "` occurs in more than one bundle"
                );
            }
        }
        return new BundleGeneration(nextSequence.get(), generationFingerprint(bundles), Instant.now(), bundles);
    }

    /** The atomic swap happens only after every bundle validates. */
    public BundleGeneration reload() throws IOException {
        synchronized (lock) {
            long started = System.nanoTime();
            try {
                BundleGeneration candidate = validateCandidate();
                BundleGeneration published = new BundleGeneration(
                    nextSequence.getAndIncrement(),
                    candidate.fingerprint(),
                    candidate.loadedAt(),
                    candidate.bundles()
                );
                active.set(published);
                BundleLoadProgress.finished(published.bundles().size(), published.sequence(), millisSince(started));
                return published;
            } catch (IOException | RuntimeException exception) {
                BundleLoadProgress.failed(String.valueOf(exception.getMessage()), millisSince(started));
                throw exception;
            }
        }
    }

    /**
     * Load in the background, so a game start does not wait on it. A load
     * already running is returned rather than started again; the active
     * generation is only ever replaced by a complete one, so a caller that
     * never joins still ends up with either the old bundles or the new.
     */
    public CompletableFuture<BundleGeneration> reloadAsync() {
        synchronized (lock) {
            CompletableFuture<BundleGeneration> running = pending;
            if (running != null && !running.isDone()) return running;
            CompletableFuture<BundleGeneration> started = CompletableFuture.supplyAsync(() -> {
                try {
                    return reload();
                } catch (IOException exception) {
                    throw new CompletionException(exception);
                }
            }, BundleLoadPool.executor());
            pending = started;
            return started;
        }
    }

    /** The background load in flight, if there is one. */
    public CompletableFuture<BundleGeneration> pending() {
        return pending;
    }

    private static long millisSince(long startedNanos) {
        return (System.nanoTime() - startedNanos) / 1_000_000L;
    }

    private static String generationFingerprint(List<BundleManifest> bundles) {
        MessageDigest digest;
        try {
            digest = MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException exception) {
            throw new AssertionError(exception);
        }
        digest.update(GENERATION_DOMAIN);
        for (BundleManifest bundle : bundles) {
            byte[] campaign = bundle.campaignId().getBytes(StandardCharsets.UTF_8);
            digest.update(ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(campaign.length).array());
            digest.update(campaign);
            digest.update(HexFormat.of().parseHex(bundle.fingerprint()));
        }
        return HexFormat.of().formatHex(digest.digest());
    }
}
