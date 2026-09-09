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
        var campaigns = new HashSet<String>();
        var bundles = new ArrayList<BundleManifest>(paths.size());
        for (Path path : paths) {
            BundleManifest manifest = validator.validate(path);
            if (!campaigns.add(manifest.campaignId())) {
                throw new BundleValidationException(
                    BundleErrorCode.DUPLICATE_IDENTITY,
                    "campaign `" + manifest.campaignId() + "` occurs in more than one bundle"
                );
            }
            bundles.add(manifest);
        }
        return new BundleGeneration(nextSequence.get(), generationFingerprint(bundles), Instant.now(), bundles);
    }

    /** The atomic swap happens only after every bundle validates. */
    public BundleGeneration reload() throws IOException {
        BundleGeneration candidate = validateCandidate();
        BundleGeneration published = new BundleGeneration(
            nextSequence.getAndIncrement(),
            candidate.fingerprint(),
            candidate.loadedAt(),
            candidate.bundles()
        );
        active.set(published);
        return published;
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
