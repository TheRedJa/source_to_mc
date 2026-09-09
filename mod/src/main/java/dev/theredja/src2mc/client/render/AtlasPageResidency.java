package dev.theredja.src2mc.client.render;

import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.platform.TextureUtil;
import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.Src2mcConfig;
import dev.theredja.src2mc.Src2mcIds;
import dev.theredja.src2mc.bundle.AtlasIndex;
import dev.theredja.src2mc.bundle.BundleManifest;
import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.zip.ZipFile;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.util.FastColor;
import net.minecraft.server.packs.resources.ResourceManager;

/**
 * Render-thread-owned, demand-driven atlas residency. PNG decoding happens on
 * one daemon worker; GL allocation/upload and eviction only happen in pump().
 */
public final class AtlasPageResidency implements AutoCloseable {
    private static final HexFormat HEX = HexFormat.of();
    private static final long EVICTION_GRACE_FRAMES = 600;
    private final ExecutorService decoder = Executors.newSingleThreadExecutor(task -> {
        Thread thread = new Thread(task, "src2mc-atlas-decoder");
        thread.setDaemon(true);
        return thread;
    });
    private final Map<Key, Entry> entries = new LinkedHashMap<>();
    private long generation = -1;
    private long residentVram;
    private long pendingRam;
    private long requests;
    private long hits;
    private long misses;
    private long denied;
    private long evictions;
    private long decodeFailures;
    private final List<PageKey> readyPages = new ArrayList<>();
    private DynamicTexture placeholderTexture;

    /** Marks a page needed this frame and returns its texture once uploaded. */
    public Optional<ResourceLocation> request(long generation, BundleManifest bundle, AtlasIndex atlas,
                                               int page, long frame) {
        resetIfGenerationChanged(generation);
        if (page < 0 || page >= atlas.pages().size()) return Optional.empty();
        requests++;
        Key key = new Key(bundle.fingerprint(), page);
        Entry current = entries.get(key);
        if (current != null) {
            current.lastUsedFrame = frame;
            if (current.textureId != null) hits++;
            return Optional.ofNullable(current.textureId);
        }
        long decodedBytes = decodedBytes(atlas.pages().get(page));
        if (decodedBytes > Src2mcConfig.textureRamBudgetBytes() || pendingRam + decodedBytes > Src2mcConfig.textureRamBudgetBytes()) {
            denied++;
            return Optional.empty();
        }
        misses++;
        Entry created = new Entry(decodedBytes, frame);
        pendingRam += decodedBytes;
        created.decode = CompletableFuture.supplyAsync(() -> {
            DecodedPage decoded = decode(bundle, atlas.pages().get(page));
            if (created.abandoned) {
                decoded.images.forEach(NativeImage::close);
                throw new IllegalStateException("atlas page request abandoned");
            }
            return decoded;
        }, decoder);
        entries.put(key, created);
        return Optional.empty();
    }

    /** Completes uploads and evicts least-recently-used pages to the VRAM budget. */
    public void pump(long frame) {
        Minecraft minecraft = Minecraft.getInstance();
        for (var iterator = entries.entrySet().iterator(); iterator.hasNext();) {
            var item = iterator.next();
            Entry entry = item.getValue();
            if (entry.decode == null || !entry.decode.isDone()) continue;
            DecodedPage decoded;
            try {
                decoded = entry.decode.join();
            } catch (RuntimeException exception) {
                pendingRam -= entry.decodedBytes;
                iterator.remove();
                decodeFailures++;
                Src2mc.LOGGER.error("Failed to decode src2mc atlas page {}", item.getKey().page, exception.getCause());
                continue;
            }
            pendingRam -= entry.decodedBytes;
            entry.decode = null;
            ResourceLocation id = Src2mcIds.id("atlas/" + item.getKey().fingerprint.substring(0, 16) + "/" + item.getKey().page);
            minecraft.getTextureManager().register(id, new MipTexture(decoded.images));
            entry.textureId = id;
            entry.vramBytes = decoded.bytes;
            residentVram += decoded.bytes;
            readyPages.add(new PageKey(item.getKey().fingerprint, item.getKey().page));
        }
        while (residentVram > Src2mcConfig.textureVramBudgetBytes()) {
            Map.Entry<Key, Entry> victim = entries.entrySet().stream()
                .filter(item -> item.getValue().textureId != null
                    && frame - item.getValue().lastUsedFrame > EVICTION_GRACE_FRAMES)
                .min(Comparator.comparingLong(item -> item.getValue().lastUsedFrame)).orElse(null);
            if (victim == null) break;
            release(minecraft, victim.getValue());
            entries.remove(victim.getKey());
            evictions++;
        }
    }

    public long residentVramBytes() { return residentVram; }
    public long pendingRamBytes() { return pendingRam; }

    /** Pages that changed from a loading placeholder to a resident texture since the last call. */
    public List<PageKey> drainReadyPages() {
        List<PageKey> result = List.copyOf(readyPages);
        readyPages.clear();
        return result;
    }

    /** A deliberately obvious texture for faces whose real atlas page is still loading. */
    public ResourceLocation placeholderTexture() {
        Minecraft minecraft = Minecraft.getInstance();
        if (placeholderTexture == null) {
            var image = new NativeImage(16, 16, false);
            for (int y = 0; y < 16; y++) for (int x = 0; x < 16; x++) {
                boolean bright = ((x >> 2) + (y >> 2)) % 2 == 0;
                image.setPixelRGBA(x, y, FastColor.ABGR32.color(255,
                    bright ? 255 : 24, bright ? 0 : 0, bright ? 255 : 24));
            }
            placeholderTexture = new DynamicTexture(image);
            placeholderTexture.setFilter(false, false);
            minecraft.getTextureManager().register(Src2mcIds.id("atlas_loading"), placeholderTexture);
            placeholderTexture.upload();
        }
        return Src2mcIds.id("atlas_loading");
    }

    public Stats stats() {
        return new Stats(entries.size(), entries.values().stream().filter(entry -> entry.textureId != null).count(),
            residentVram, pendingRam, requests, hits, misses, denied, evictions, decodeFailures);
    }

    private void resetIfGenerationChanged(long next) {
        if (generation == next) return;
        closeEntries();
        generation = next;
    }

    public void reset(long nextGeneration) {
        resetIfGenerationChanged(nextGeneration);
    }

    private static DecodedPage decode(BundleManifest bundle, AtlasIndex.Page page) {
        List<NativeImage> images = new ArrayList<>(page.mips().size());
        try (var zip = new ZipFile(bundle.path().toFile())) {
            for (AtlasIndex.Mip mip : page.mips()) {
                var zipEntry = zip.getEntry(mip.path());
                BundleManifest.Entry declared = bundle.entries().stream()
                    .filter(entry -> entry.path().equals(mip.path())).findFirst()
                    .orElseThrow(() -> new IOException("page mip absent from validated manifest"));
                if (zipEntry == null || zipEntry.getSize() != declared.size() || declared.size() > Integer.MAX_VALUE) {
                    throw new IOException("atlas page changed after generation validation");
                }
                byte[] png;
                try (var input = zip.getInputStream(zipEntry)) { png = input.readNBytes((int) declared.size() + 1); }
                if (png.length != declared.size() || !sha256(png).equals(declared.sha256())) {
                    throw new IOException("atlas page hash changed after generation validation");
                }
                NativeImage image = NativeImage.read(new ByteArrayInputStream(png));
                if (image.getWidth() != mip.width() || image.getHeight() != mip.height()) {
                    image.close();
                    throw new IOException("decoded atlas dimensions changed");
                }
                images.add(image);
            }
            return new DecodedPage(images, decodedBytes(page));
        } catch (Exception exception) {
            images.forEach(NativeImage::close);
            throw new IllegalStateException(exception);
        }
    }

    private static String sha256(byte[] bytes) throws Exception {
        return HEX.formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }

    static long decodedBytes(AtlasIndex.Page page) {
        return page.mips().stream().mapToLong(mip -> Math.multiplyExact(4L, Math.multiplyExact(mip.width(), mip.height()))).sum();
    }

    private void closeEntries() {
        Minecraft minecraft = Minecraft.getInstance();
        entries.values().forEach(entry -> {
            entry.abandoned = true;
            if (entry.decode != null && entry.decode.isDone() && !entry.decode.isCompletedExceptionally()) {
                entry.decode.join().images.forEach(NativeImage::close);
            }
            release(minecraft, entry);
        });
        entries.clear();
        residentVram = 0;
        pendingRam = 0;
        readyPages.clear();
    }

    private void release(Minecraft minecraft, Entry entry) {
        if (entry.textureId != null) minecraft.getTextureManager().release(entry.textureId);
        residentVram -= entry.vramBytes;
    }

    @Override public void close() {
        closeEntries();
        decoder.shutdownNow();
    }

    public record PageKey(String fingerprint, int page) {}
    public record Stats(long trackedPages, long residentPages, long residentVramBytes, long pendingRamBytes,
                        long requests, long hits, long misses, long denied, long evictions, long decodeFailures) {}
    private record Key(String fingerprint, int page) {}
    private static final class Entry {
        final long decodedBytes; long lastUsedFrame; long vramBytes;
        CompletableFuture<DecodedPage> decode; ResourceLocation textureId; volatile boolean abandoned;
        Entry(long decodedBytes, long frame) { this.decodedBytes = decodedBytes; this.lastUsedFrame = frame; }
    }
    private record DecodedPage(List<NativeImage> images, long bytes) {}

    private static final class MipTexture extends AbstractTexture {
        private List<NativeImage> images;
        MipTexture(List<NativeImage> images) { this.images = images; }
        @Override public void load(ResourceManager ignored) {
            NativeImage base = images.getFirst();
            TextureUtil.prepareImage(getId(), images.size() - 1, base.getWidth(), base.getHeight());
            bind();
            for (int level = 0; level < images.size(); level++) {
                NativeImage image = images.get(level);
                image.upload(level, 0, 0, 0, 0, image.getWidth(), image.getHeight(), false, false, false, false);
            }
            setFilter(false, true);
            images.forEach(NativeImage::close);
            images = List.of();
        }
        @Override public void close() {
            images.forEach(NativeImage::close);
            images = List.of();
            releaseId();
        }
    }
}
