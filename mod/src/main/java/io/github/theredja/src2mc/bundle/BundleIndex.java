package io.github.theredja.src2mc.bundle;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import io.github.theredja.src2mc.FormatVersion;
import io.github.theredja.src2mc.Src2mc;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Optional;

/**
 * {@code bundle.json} — the index at the root of a bundle
 * ({@code docs/format.md} §4).
 *
 * <p>Read before anything else in the bundle, because it carries the version
 * that says whether the rest can be believed.
 */
public record BundleIndex(String name, float unitsPerBlock) {
    public static final String FILE = "bundle.json";

    /**
     * Reads and accepts the bundle at {@code root}, or explains why not.
     *
     * <p>A version mismatch is a refusal, not a warning: reading half of a
     * bundle written by a different converter is how two implementations quietly
     * disagree.
     */
    public static Optional<BundleIndex> read(Path root) {
        Path file = root.resolve(FILE);
        if (!Files.isRegularFile(file)) {
            Src2mc.LOG.error("{} has no {} — that is not a bundle (docs/format.md §4)", root, FILE);
            return Optional.empty();
        }

        JsonObject json;
        try (Reader reader = Files.newBufferedReader(file, StandardCharsets.UTF_8)) {
            json = JsonParser.parseReader(reader).getAsJsonObject();
        } catch (Exception e) {
            Src2mc.LOG.error("cannot read {}: {}", file, e.toString());
            return Optional.empty();
        }

        int version = json.has("format_version") ? json.get("format_version").getAsInt() : FormatVersion.ABSENT;
        Optional<String> rejection = FormatVersion.reject(version, file.toString());
        if (rejection.isPresent()) {
            Src2mc.LOG.error("{}", rejection.get());
            return Optional.empty();
        }

        String name = json.has("name") ? json.get("name").getAsString() : root.getFileName().toString();
        float unitsPerBlock = 16.0F;
        if (json.has("scale")) {
            JsonObject scale = json.getAsJsonObject("scale");
            if (scale.has("units_per_block")) {
                unitsPerBlock = scale.get("units_per_block").getAsFloat();
            }
        }
        return Optional.of(new BundleIndex(name, unitsPerBlock));
    }
}
