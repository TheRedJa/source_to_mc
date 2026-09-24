package dev.theredja.src2mc.bundle;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.io.TempDir;

final class BundleValidatorTest {
    @TempDir
    Path directory;

    @Test
    void validatesExternalConverterBundleWhenRequested() throws Exception {
        String path = System.getProperty("src2mc.testBundle");
        Assumptions.assumeTrue(path != null, "set -Dsrc2mc.testBundle to exercise converter/mod compatibility");
        BundleManifest manifest = new BundleValidator().validate(Path.of(path));
        org.junit.jupiter.api.Assertions.assertFalse(manifest.maps().isEmpty());
        org.junit.jupiter.api.Assertions.assertNotNull(manifest.maps().getFirst().atlas());
    }

    @Test
    void validatesCanonicalManifestAndPayloadHash() throws Exception {
        Path bundle = directory.resolve("valid.src2mc");
        writeBundle(bundle, 1, "hl2", "campaign.json", campaign("hl2"), false);

        BundleManifest manifest = new BundleValidator().validate(bundle);

        assertEquals("hl2", manifest.campaignId());
        assertEquals(1, manifest.entries().size());
        assertEquals(campaign("hl2").length, manifest.uncompressedBytes());
    }

    @Test
    void rejectsCorruptionUnsupportedVersionsAndUnsafePaths() throws Exception {
        Path corrupt = directory.resolve("corrupt.src2mc");
        writeBundle(corrupt, 1, "hl2", "campaign.json", "bad\n".getBytes(StandardCharsets.UTF_8), true);
        assertCode(BundleErrorCode.HASH_MISMATCH, corrupt);

        Path unsupported = directory.resolve("unsupported.src2mc");
        writeBundle(unsupported, 2, "hl2", "campaign.json", campaign("hl2"), false);
        assertCode(BundleErrorCode.UNSUPPORTED_VERSION, unsupported);

        Path unsafe = directory.resolve("unsafe.src2mc");
        writeBundle(unsafe, 1, "hl2", "../campaign.json", "{}\n".getBytes(StandardCharsets.UTF_8), false);
        assertCode(BundleErrorCode.UNSAFE_PATH, unsafe);
    }

    @Test
    void failedReloadRetainsLastKnownGoodGeneration() throws Exception {
        Path bundle = directory.resolve("campaign.src2mc");
        writeBundle(bundle, 1, "hl2", "campaign.json", campaign("hl2"), false);
        var repository = new BundleRepository(() -> directory);
        BundleGeneration good = repository.reload();

        writeBundle(bundle, 1, "hl2", "campaign.json", "corrupt\n".getBytes(StandardCharsets.UTF_8), true);
        assertThrows(BundleValidationException.class, repository::reload);

        assertEquals(good, repository.active());
        assertEquals(1, repository.active().sequence());
    }

    @Test
    void loadsSeveralBundlesInParallelWithAStableResult() throws Exception {
        for (int i = 0; i < 6; i++) {
            String campaign = "hl" + i;
            writeBundle(directory.resolve(campaign + ".src2mc"), 1, campaign, "campaign.json", campaign(campaign), false);
        }
        var repository = new BundleRepository(() -> directory);

        BundleGeneration first = repository.reload();
        BundleGeneration second = repository.reload();

        assertEquals(6, first.bundles().size());
        assertEquals(first.fingerprint(), second.fingerprint());
        // Filename order, whatever order the threads finished in.
        assertEquals(List.of("hl0", "hl1", "hl2", "hl3", "hl4", "hl5"),
            second.bundles().stream().map(BundleManifest::campaignId).toList());
    }

    @Test
    void reloadAsyncPublishesTheSameGenerationAndReportsProgress() throws Exception {
        writeBundle(directory.resolve("campaign.src2mc"), 1, "hl2", "campaign.json", campaign("hl2"), false);
        var repository = new BundleRepository(() -> directory);

        BundleGeneration published = repository.reloadAsync().get(30, java.util.concurrent.TimeUnit.SECONDS);

        assertEquals(published, repository.active());
        assertEquals(BundleLoadProgress.State.DONE, BundleLoadProgress.snapshot().state());
        assertEquals(1, BundleLoadProgress.snapshot().total());
    }

    @Test
    void aFailedAsyncLoadIsReportedWithoutReplacingTheActiveGeneration() throws Exception {
        Path bundle = directory.resolve("campaign.src2mc");
        writeBundle(bundle, 1, "hl2", "campaign.json", campaign("hl2"), false);
        var repository = new BundleRepository(() -> directory);
        BundleGeneration good = repository.reload();

        writeBundle(bundle, 1, "hl2", "campaign.json", "corrupt\n".getBytes(StandardCharsets.UTF_8), true);
        assertThrows(java.util.concurrent.ExecutionException.class,
            () -> repository.reloadAsync().get(30, java.util.concurrent.TimeUnit.SECONDS));

        assertEquals(good, repository.active());
        assertEquals(BundleLoadProgress.State.FAILED, BundleLoadProgress.snapshot().state());
    }

    @Test
    void validatesCompleteMapSchemasAndRejectsMalformedBinary() throws Exception {
        Path valid = directory.resolve("map.src2mc");
        Map<String, byte[]> payloads = emptyMapPayloads();
        writeBundle(valid, 1, "hl2", payloads);
        assertEquals(5, new BundleValidator().validate(valid).entries().size());

        Path malformed = directory.resolve("bad-face.src2mc");
        payloads = new TreeMap<>(payloads);
        payloads.put("maps/d1_01/surfaces.s2faces", new byte[24]);
        writeBundle(malformed, 1, "hl2", payloads);
        assertCode(BundleErrorCode.INVALID_SCHEMA, malformed);
    }

    private void assertCode(BundleErrorCode code, Path bundle) {
        BundleValidationException exception = assertThrows(
            BundleValidationException.class,
            () -> new BundleValidator().validate(bundle)
        );
        assertEquals(code, exception.code());
    }

    private static void writeBundle(
        Path output,
        int version,
        String campaign,
        String payloadPath,
        byte[] payload,
        boolean lieAboutPayload
    ) throws Exception {
        byte[] declared = lieAboutPayload ? "{}\n".getBytes(StandardCharsets.UTF_8) : payload;
        String payloadHash = sha256(declared);
        var entry = new BundleManifest.Entry(payloadPath, declared.length, payloadHash);
        String fingerprint = fingerprint(List.of(entry));
        String manifest = "{\"format\":\"src2mc-campaign\",\"version\":" + version
            + ",\"campaign_id\":\"" + campaign + "\",\"fingerprint\":\"" + fingerprint
            + "\",\"entries\":[{\"path\":\"" + payloadPath + "\",\"size\":" + declared.length
            + ",\"sha256\":\"" + payloadHash + "\"}]}\n";
        try (var zip = new ZipOutputStream(java.nio.file.Files.newOutputStream(output))) {
            put(zip, "manifest.json", manifest.getBytes(StandardCharsets.UTF_8));
            put(zip, payloadPath, payload);
        }
    }

    private static void writeBundle(Path output, int version, String campaign, Map<String, byte[]> payloads) throws Exception {
        List<BundleManifest.Entry> declared = payloads.entrySet().stream().map(entry -> {
            try {
                return new BundleManifest.Entry(entry.getKey(), entry.getValue().length, sha256(entry.getValue()));
            } catch (Exception exception) {
                throw new AssertionError(exception);
            }
        }).toList();
        StringBuilder manifest = new StringBuilder("{\"format\":\"src2mc-campaign\",\"version\":")
            .append(version).append(",\"campaign_id\":\"").append(campaign)
            .append("\",\"fingerprint\":\"").append(fingerprint(declared)).append("\",\"entries\":[");
        for (int i = 0; i < declared.size(); i++) {
            BundleManifest.Entry entry = declared.get(i);
            if (i != 0) manifest.append(',');
            manifest.append("{\"path\":\"").append(entry.path()).append("\",\"size\":")
                .append(entry.size()).append(",\"sha256\":\"").append(entry.sha256()).append("\"}");
        }
        manifest.append("]}\n");
        try (var zip = new ZipOutputStream(java.nio.file.Files.newOutputStream(output))) {
            put(zip, "manifest.json", manifest.toString().getBytes(StandardCharsets.UTF_8));
            for (Map.Entry<String, byte[]> entry : payloads.entrySet()) put(zip, entry.getKey(), entry.getValue());
        }
    }

    private static Map<String, byte[]> emptyMapPayloads() {
        Map<String, byte[]> payloads = new TreeMap<>();
        payloads.put("campaign.json", ("{\"format\":\"src2mc-campaign-metadata\",\"version\":1,\"campaign_id\":\"hl2\","
            + "\"maps\":[{\"map_id\":\"d1_01\",\"metadata\":\"maps/d1_01.json\"}]}\n").getBytes(StandardCharsets.UTF_8));
        payloads.put("maps/d1_01.json", ("{\"format\":\"src2mc-map\",\"version\":1,\"map_id\":\"d1_01\",\"source_name\":\"d1_01\","
            + "\"units_per_block\":32.0,\"cell_min\":[0,0,0],\"cell_max\":[0,0,0],\"anchor_cell\":[0,0,0],"
            + "\"surfaces\":\"maps/d1_01/surfaces.s2faces\",\"materials\":[],\"models\":[],"
            + "\"props\":\"maps/d1_01/props.s2props\",\"diagnostics\":\"maps/d1_01/diagnostics.json\"}\n").getBytes(StandardCharsets.UTF_8));
        payloads.put("maps/d1_01/diagnostics.json", "{\"format\":\"src2mc-diagnostics\",\"version\":1,\"messages\":[]}\n".getBytes(StandardCharsets.UTF_8));
        payloads.put("maps/d1_01/props.s2props", binaryHeader("S2PROP\0\0", 1, 0));
        payloads.put("maps/d1_01/surfaces.s2faces", binaryHeader("S2FACE\0\0", 1, 0, 0, 0));
        return payloads;
    }

    private static byte[] binaryHeader(String magic, int... values) {
        ByteBuffer output = ByteBuffer.allocate(8 + values.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        output.put(magic.getBytes(StandardCharsets.ISO_8859_1));
        for (int value : values) output.putInt(value);
        return output.array();
    }

    private static void put(ZipOutputStream zip, String path, byte[] bytes) throws IOException {
        zip.putNextEntry(new ZipEntry(path));
        zip.write(bytes);
        zip.closeEntry();
    }

    private static byte[] campaign(String id) {
        return ("{\"format\":\"src2mc-campaign-metadata\",\"version\":1,\"campaign_id\":\""
            + id + "\",\"maps\":[]}\n").getBytes(StandardCharsets.UTF_8);
    }

    private static String fingerprint(List<BundleManifest.Entry> entries) throws Exception {
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        digest.update("src2mc-manifest-v1\0".getBytes(StandardCharsets.US_ASCII));
        for (BundleManifest.Entry entry : entries) {
            byte[] path = entry.path().getBytes(StandardCharsets.UTF_8);
            digest.update(ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(path.length).array());
            digest.update(path);
            digest.update(ByteBuffer.allocate(8).order(ByteOrder.LITTLE_ENDIAN).putLong(entry.size()).array());
            digest.update(HexFormat.of().parseHex(entry.sha256()));
        }
        return HexFormat.of().formatHex(digest.digest());
    }

    private static String sha256(byte[] bytes) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }
}
