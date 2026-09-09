package dev.theredja.src2mc.bundle;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.HashMap;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

/** Streaming structural and cryptographic validation of a v1 campaign bundle. */
public final class BundleValidator {
    private static final String MANIFEST_PATH = "manifest.json";
    private static final byte[] FINGERPRINT_DOMAIN = "src2mc-manifest-v1\0".getBytes(StandardCharsets.US_ASCII);
    private static final HexFormat HEX = HexFormat.of();

    public BundleManifest validate(Path path) throws IOException {
        if (!path.getFileName().toString().endsWith(".src2mc")) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "bundle filename must end in .src2mc");
        }
        try (var zip = new ZipFile(path.toFile(), StandardCharsets.UTF_8)) {
            Map<String, ZipEntry> actual = inspectDirectory(zip);
            ZipEntry manifestEntry = actual.get(MANIFEST_PATH);
            if (manifestEntry == null) {
                throw failure(BundleErrorCode.MISSING_ENTRY, "missing manifest.json");
            }
            byte[] bytes = readBounded(zip, manifestEntry, BundleLimits.MAX_MANIFEST_BYTES);
            ParsedManifest parsed = parseManifest(bytes);
            verifyContents(zip, actual, parsed.entries());
            String fingerprint = fingerprint(parsed.entries());
            if (!fingerprint.equals(parsed.fingerprint())) {
                throw failure(BundleErrorCode.HASH_MISMATCH, "manifest fingerprint differs");
            }
            List<BundleMap> maps = BundleSchemaValidator.validate(zip, actual, parsed.campaignId(), parsed.entries());
            long total = parsed.entries().stream().mapToLong(BundleManifest.Entry::size).sum();
            return new BundleManifest(path, parsed.campaignId(), fingerprint, parsed.entries(), total, maps);
        } catch (BundleValidationException exception) {
            throw exception;
        } catch (RuntimeException exception) {
            throw new BundleValidationException(BundleErrorCode.INVALID_SCHEMA, "invalid manifest JSON", exception);
        }
    }

    private static Map<String, ZipEntry> inspectDirectory(ZipFile zip) throws BundleValidationException {
        Map<String, ZipEntry> entries = new HashMap<>();
        long total = 0;
        int count = 0;
        Enumeration<? extends ZipEntry> enumeration = zip.entries();
        while (enumeration.hasMoreElements()) {
            ZipEntry entry = enumeration.nextElement();
            String name = entry.getName();
            validatePath(name);
            if (entry.isDirectory()) {
                throw failure(BundleErrorCode.INVALID_SCHEMA, "directory ZIP entry is not permitted: " + name);
            }
            if (++count > BundleLimits.MAX_ENTRY_COUNT + 1) {
                throw failure(BundleErrorCode.LIMIT_EXCEEDED, "too many ZIP entries");
            }
            if (entries.putIfAbsent(name, entry) != null) {
                throw failure(BundleErrorCode.DUPLICATE_IDENTITY, "duplicate ZIP entry: " + name);
            }
            long size = entry.getSize();
            long compressed = entry.getCompressedSize();
            if (size < 0 || compressed < 0) {
                throw failure(BundleErrorCode.INVALID_SCHEMA, "ZIP entry has unknown size: " + name);
            }
            checkExpansion(name, compressed, size);
            try {
                total = Math.addExact(total, size);
            } catch (ArithmeticException exception) {
                throw failure(BundleErrorCode.LIMIT_EXCEEDED, "bundle size overflow");
            }
            if (total > BundleLimits.MAX_UNCOMPRESSED_BUNDLE_BYTES) {
                throw failure(BundleErrorCode.LIMIT_EXCEEDED, "bundle payload exceeds limit");
            }
        }
        return entries;
    }

    private static ParsedManifest parseManifest(byte[] bytes) throws BundleValidationException {
        if (bytes.length == 0 || bytes[bytes.length - 1] != '\n') {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "manifest must end in LF");
        }
        validateJsonEnvelope(bytes);
        JsonElement root = JsonParser.parseString(decodeUtf8(bytes));
        if (!root.isJsonObject()) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "manifest root must be an object");
        }
        JsonObject object = root.getAsJsonObject();
        requireKeys(object, Set.of("format", "version", "campaign_id", "fingerprint", "entries"));
        if (!"src2mc-campaign".equals(string(object, "format"))) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "unexpected manifest format");
        }
        int version = integer(object, "version");
        if (version != 1) {
            throw failure(BundleErrorCode.UNSUPPORTED_VERSION, "manifest version " + version);
        }
        String campaignId = string(object, "campaign_id");
        validateId(campaignId, "campaign");
        String claimedFingerprint = digestString(object, "fingerprint");
        JsonElement entriesElement = object.get("entries");
        if (entriesElement == null || !entriesElement.isJsonArray()) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "entries must be an array");
        }
        JsonArray array = entriesElement.getAsJsonArray();
        if (array.size() > BundleLimits.MAX_ENTRY_COUNT) {
            throw failure(BundleErrorCode.LIMIT_EXCEEDED, "too many manifest entries");
        }
        List<BundleManifest.Entry> entries = new ArrayList<>(array.size());
        String previous = null;
        for (JsonElement element : array) {
            if (!element.isJsonObject()) {
                throw failure(BundleErrorCode.INVALID_SCHEMA, "manifest entry must be an object");
            }
            JsonObject entry = element.getAsJsonObject();
            requireKeys(entry, Set.of("path", "size", "sha256"));
            String entryPath = string(entry, "path");
            validatePath(entryPath);
            if (MANIFEST_PATH.equals(entryPath)) {
                throw failure(BundleErrorCode.INVALID_SCHEMA, "manifest cannot hash itself");
            }
            if (previous != null && previous.compareTo(entryPath) >= 0) {
                throw failure(BundleErrorCode.DUPLICATE_IDENTITY, "manifest entries are not uniquely sorted");
            }
            previous = entryPath;
            long size = nonnegativeLong(entry, "size");
            if (size > BundleLimits.MAX_UNCOMPRESSED_ENTRY_BYTES) {
                throw failure(BundleErrorCode.LIMIT_EXCEEDED, "entry exceeds size limit: " + entryPath);
            }
            entries.add(new BundleManifest.Entry(entryPath, size, digestString(entry, "sha256")));
        }
        return new ParsedManifest(campaignId, claimedFingerprint, entries);
    }

    private static void verifyContents(ZipFile zip, Map<String, ZipEntry> actual, List<BundleManifest.Entry> declared)
        throws IOException {
        if (actual.size() != declared.size() + 1) {
            throw failure(BundleErrorCode.INVALID_REFERENCE, "ZIP entries do not match manifest");
        }
        for (BundleManifest.Entry expected : declared) {
            ZipEntry entry = actual.get(expected.path());
            if (entry == null) {
                throw failure(BundleErrorCode.MISSING_ENTRY, "missing " + expected.path());
            }
            if (entry.getSize() != expected.size()) {
                throw failure(BundleErrorCode.HASH_MISMATCH, "size differs for " + expected.path());
            }
            MessageDigest digest = sha256();
            long read = 0;
            try (InputStream input = zip.getInputStream(entry)) {
                byte[] buffer = new byte[64 * 1024];
                for (int count; (count = input.read(buffer)) >= 0;) {
                    if (count == 0) continue;
                    read = Math.addExact(read, count);
                    if (read > expected.size()) {
                        throw failure(BundleErrorCode.HASH_MISMATCH, "entry expands beyond declared size: " + expected.path());
                    }
                    digest.update(buffer, 0, count);
                }
            }
            if (read != expected.size() || !HEX.formatHex(digest.digest()).equals(expected.sha256())) {
                throw failure(BundleErrorCode.HASH_MISMATCH, "payload differs for " + expected.path());
            }
        }
    }

    private static String fingerprint(List<BundleManifest.Entry> entries) throws BundleValidationException {
        MessageDigest digest = sha256();
        digest.update(FINGERPRINT_DOMAIN);
        for (BundleManifest.Entry entry : entries) {
            byte[] path = entry.path().getBytes(StandardCharsets.UTF_8);
            digest.update(littleEndian(path.length, 4));
            digest.update(path);
            digest.update(littleEndian(entry.size(), 8));
            digest.update(HEX.parseHex(entry.sha256()));
        }
        return HEX.formatHex(digest.digest());
    }

    private static byte[] littleEndian(long value, int bytes) {
        ByteBuffer buffer = ByteBuffer.allocate(bytes).order(ByteOrder.LITTLE_ENDIAN);
        if (bytes == 4) buffer.putInt(Math.toIntExact(value)); else buffer.putLong(value);
        return buffer.array();
    }

    private static byte[] readBounded(ZipFile zip, ZipEntry entry, int maximum) throws IOException {
        if (entry.getSize() > maximum) {
            throw failure(BundleErrorCode.LIMIT_EXCEEDED, "manifest exceeds bootstrap limit");
        }
        try (InputStream input = zip.getInputStream(entry); var output = new ByteArrayOutputStream((int) entry.getSize())) {
            byte[] buffer = new byte[8192];
            for (int count; (count = input.read(buffer)) >= 0;) {
                if (count == 0) continue;
                if (output.size() + count > maximum) {
                    throw failure(BundleErrorCode.LIMIT_EXCEEDED, "manifest exceeds bootstrap limit");
                }
                output.write(buffer, 0, count);
            }
            return output.toByteArray();
        }
    }

    private static void checkExpansion(String name, long compressed, long size) throws BundleValidationException {
        if (size > BundleLimits.MAX_UNCOMPRESSED_ENTRY_BYTES) {
            throw failure(BundleErrorCode.LIMIT_EXCEEDED, "entry exceeds size limit: " + name);
        }
        long allowed;
        try {
            allowed = Math.multiplyExact(compressed, BundleLimits.MAX_ZIP_EXPANSION_RATIO);
        } catch (ArithmeticException exception) {
            allowed = Long.MAX_VALUE;
        }
        allowed = Math.max(allowed, BundleLimits.SMALL_ENTRY_ALLOWANCE);
        if (size > allowed) {
            throw failure(BundleErrorCode.ZIP_EXPANSION_LIMIT, "suspicious expansion for " + name);
        }
    }

    static void validatePath(String path) throws BundleValidationException {
        byte[] utf8 = path.getBytes(StandardCharsets.UTF_8);
        if (path.isEmpty() || utf8.length > BundleLimits.MAX_ENTRY_PATH_BYTES || path.startsWith("/")
            || path.endsWith("/") || path.indexOf('\\') >= 0) {
            throw failure(BundleErrorCode.UNSAFE_PATH, "unsafe entry path: " + path);
        }
        for (String part : path.split("/", -1)) {
            if (part.isEmpty() || part.equals(".") || part.equals("..") || part.indexOf(':') >= 0
                || part.chars().anyMatch(character -> character < 0x20 || character == 0x7f)) {
                throw failure(BundleErrorCode.UNSAFE_PATH, "unsafe entry path: " + path);
            }
        }
    }

    private static void validateId(String id, String label) throws BundleValidationException {
        if (id.isEmpty() || !id.chars().allMatch(c -> c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '_' || c == '-')) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "invalid " + label + " ID: " + id);
        }
    }

    private static String digestString(JsonObject object, String key) throws BundleValidationException {
        String value = string(object, key);
        if (value.length() != 64 || !value.chars().allMatch(c -> c >= '0' && c <= '9' || c >= 'a' && c <= 'f')) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "invalid digest in " + key);
        }
        return value;
    }

    private static String string(JsonObject object, String key) throws BundleValidationException {
        JsonElement value = object.get(key);
        if (value == null || !value.isJsonPrimitive() || !value.getAsJsonPrimitive().isString()) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, key + " must be a string");
        }
        return value.getAsString();
    }

    private static int integer(JsonObject object, String key) throws BundleValidationException {
        long value = nonnegativeLong(object, key);
        if (value > Integer.MAX_VALUE) throw failure(BundleErrorCode.INVALID_SCHEMA, key + " is out of range");
        return (int) value;
    }

    private static long nonnegativeLong(JsonObject object, String key) throws BundleValidationException {
        JsonElement value = object.get(key);
        try {
            if (value == null || !value.isJsonPrimitive() || !value.getAsJsonPrimitive().isNumber()) {
                throw failure(BundleErrorCode.INVALID_SCHEMA, key + " must be an integer");
            }
            String text = value.getAsString();
            if (text.isEmpty() || (text.length() > 1 && text.charAt(0) == '0') || !text.chars().allMatch(Character::isDigit)) {
                throw failure(BundleErrorCode.INVALID_SCHEMA, key + " must be a canonical nonnegative integer");
            }
            return Long.parseLong(text);
        } catch (NumberFormatException exception) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, key + " is out of range");
        }
    }

    private static void requireKeys(JsonObject object, Set<String> expected) throws BundleValidationException {
        if (!object.keySet().equals(expected)) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "unexpected or missing JSON fields");
        }
    }

    private static String decodeUtf8(byte[] bytes) throws BundleValidationException {
        try {
            return StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes))
                .toString();
        } catch (CharacterCodingException exception) {
            throw new BundleValidationException(BundleErrorCode.INVALID_SCHEMA, "manifest is not valid UTF-8", exception);
        }
    }

    private static void validateJsonEnvelope(byte[] bytes) throws BundleValidationException {
        if (bytes.length >= 3 && bytes[0] == (byte) 0xef && bytes[1] == (byte) 0xbb && bytes[2] == (byte) 0xbf) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "manifest must not contain a UTF-8 BOM");
        }
        int nesting = 0;
        boolean string = false;
        boolean escaped = false;
        for (byte value : bytes) {
            int character = value & 0xff;
            if (string) {
                if (escaped) escaped = false;
                else if (character == '\\') escaped = true;
                else if (character == '"') string = false;
                continue;
            }
            if (character == '"') string = true;
            else if (character == '{' || character == '[') {
                if (++nesting > BundleLimits.MAX_JSON_NESTING) {
                    throw failure(BundleErrorCode.LIMIT_EXCEEDED, "manifest JSON nesting exceeds limit");
                }
            } else if ((character == '}' || character == ']') && --nesting < 0) {
                throw failure(BundleErrorCode.INVALID_SCHEMA, "unbalanced manifest JSON");
            }
        }
        if (string || nesting != 0) {
            throw failure(BundleErrorCode.INVALID_SCHEMA, "unterminated manifest JSON");
        }
    }

    private static MessageDigest sha256() {
        try {
            return MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException exception) {
            throw new AssertionError(exception);
        }
    }

    private static BundleValidationException failure(BundleErrorCode code, String message) {
        return new BundleValidationException(code, message);
    }

    private record ParsedManifest(String campaignId, String fingerprint, List<BundleManifest.Entry> entries) {
    }
}
