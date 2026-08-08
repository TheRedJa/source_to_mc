package io.github.theredja.src2mc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.io.IOException;
import java.io.Reader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;

/**
 * Reads the bundle the converter writes, from committed output.
 *
 * <p>Nothing in the mod loads a bundle yet. These tests exist first on purpose:
 * they pin what {@code docs/format.md} §4 promises, so the loader is written
 * against real bytes rather than against a reading of the document, and so a
 * change on the converter side shows up here immediately.
 */
class BundleFixtureTest {
    private static final Gson GSON = new Gson();

    private static JsonObject object(String name) throws IOException {
        return GSON.fromJson(read(name), JsonObject.class);
    }

    private static JsonArray array(String name) throws IOException {
        return GSON.fromJson(read(name), JsonArray.class);
    }

    private static Reader read(String name) throws IOException {
        Path path = Path.of(System.getProperty("src2mc.fixtures"), "bundle", name);
        assertTrue(
                Files.exists(path),
                "missing fixture "
                        + path
                        + " — generate it with: UPDATE_FIXTURES=1 cargo test --test fixtures");
        return Files.newBufferedReader(path, StandardCharsets.UTF_8);
    }

    @Test
    void theIndexDeclaresAVersionWeUnderstand() throws IOException {
        JsonObject index = object("bundle.json");

        assertEquals(
                Src2mc.FORMAT_VERSION,
                index.get("format_version").getAsInt(),
                "a bundle the mod cannot read must be refused, not guessed at");
        assertEquals("materials.json", index.get("materials").getAsString());
        assertEquals("models.json", index.get("models").getAsString());
        assertTrue(index.getAsJsonObject("scale").get("units_per_block").getAsDouble() > 0);
    }

    /**
     * The index into this array is the pool index: entry {@code i} is
     * {@code src2mc:surface_i}. Order is the contract, not a detail.
     */
    @Test
    void materialsAreIndexedByPoolPosition() throws IOException {
        JsonArray materials = array("materials.json");
        assertEquals(2, materials.size());

        JsonObject first = materials.get(0).getAsJsonObject();
        assertEquals("concrete/concretewall001a", first.get("material").getAsString());
        assertEquals(
                "textures/concrete_concretewall001a.png", first.get("texture").getAsString());
        assertEquals("solid", first.get("render_type").getAsString());
        assertEquals("stone", first.get("sound").getAsString());
    }

    /**
     * The number that removes the duplication. A texture spanning eight blocks
     * is one texture with a repeat of eight, and the axes are measured
     * separately — reading it as a single scalar would squash half the map.
     */
    @Test
    void repeatIsPerAxis() throws IOException {
        JsonArray repeat =
                array("materials.json")
                        .get(0)
                        .getAsJsonObject()
                        .getAsJsonArray("blocks_per_repeat");

        assertEquals(2, repeat.size());
        assertEquals(8.0, repeat.get(0).getAsDouble(), 1e-9);
        assertEquals(4.0, repeat.get(1).getAsDouble(), 1e-9);
    }

    /** Props are not in the bundle yet, but the table is always present. */
    @Test
    void theModelTableExistsEvenWhileEmpty() throws IOException {
        JsonArray models = array("models.json");
        assertNotNull(models, "models.json must be an array, even with no props in it");
        assertTrue(models.isEmpty(), "the converter does not write models yet");
    }

    /** A surface block id is derived from the index, with no lookup table. */
    @Test
    void surfaceIdsFollowFromTheIndex() throws IOException {
        JsonArray materials = array("materials.json");
        for (int i = 0; i < materials.size(); i++) {
            String id = Src2mc.MOD_ID + ":surface_" + i;
            assertFalse(id.contains("null"), id);
        }
        assertEquals("src2mc:surface_0", Src2mc.MOD_ID + ":surface_" + 0);
    }
}
