package io.github.theredja.src2mc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.theredja.src2mc.bundle.BundleIndex;
import io.github.theredja.src2mc.bundle.BundlePack;
import io.github.theredja.src2mc.bundle.MaterialTable;
import io.github.theredja.src2mc.schematic.SchematicHeader;
import java.io.IOException;
import java.io.StringReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.Optional;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.resources.IoSupplier;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/** Reading a bundle ({@code docs/format.md} §4) and refusing a wrong version (§1). */
class BundleReadingTest {
    private static final String MATERIALS =
            """
            [
              {
                "material": "concrete/concretefloor001a",
                "texture": "textures/concrete_concretefloor001a.png",
                "blocks_per_repeat": [4.0, 4.0],
                "render_type": "solid",
                "surface_prop": "concrete",
                "sound": "stone"
              },
              {
                "material": "glass/window01",
                "texture": "textures/glass_window01.png",
                "blocks_per_repeat": [2.0, 1.5],
                "render_type": "translucent",
                "surface_prop": "glass",
                "sound": "glass"
              }
            ]
            """;

    private static PackLocationInfo location() {
        return new PackLocationInfo("test", Component.literal("test"), PackSource.BUILT_IN, Optional.empty());
    }

    @Test
    void materialsAreIndexedBySurfaceIndex() {
        MaterialTable table = MaterialTable.parse(new StringReader(MATERIALS));

        assertEquals(2, table.size());
        MaterialTable.Entry first = table.get(0).orElseThrow();
        assertEquals("concrete/concretefloor001a", first.material());
        assertEquals(4.0F, first.blocksPerRepeatU());
        assertEquals(MaterialTable.RenderKind.SOLID, first.renderType());
        assertEquals(MaterialTable.RenderKind.TRANSLUCENT, table.get(1).orElseThrow().renderType());

        // The pool is bigger than any bundle, so asking past the end is normal.
        assertTrue(table.get(2).isEmpty());
        assertTrue(table.get(-1).isEmpty());
    }

    @Test
    void aTextureNamesTheSpriteItStitchesTo() {
        assertEquals(
                ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "bundle/concrete_floor"),
                BundlePack.spriteFor("textures/concrete_floor.png"));
    }

    @Test
    void aBadScaleIsRefusedRatherThanDrawnWrong() {
        String zero = MATERIALS.replace("[4.0, 4.0]", "[0.0, 4.0]");
        assertThrows(IllegalArgumentException.class, () -> MaterialTable.parse(new StringReader(zero)));

        String unknown = MATERIALS.replace("\"solid\"", "\"emissive\"");
        assertThrows(IllegalArgumentException.class, () -> MaterialTable.parse(new StringReader(unknown)));
    }

    @Test
    void theBundleDirectoryIsSeenAsResources(@TempDir Path bundle) throws IOException {
        Files.writeString(bundle.resolve("materials.json"), MATERIALS, StandardCharsets.UTF_8);
        Files.createDirectories(bundle.resolve("textures"));
        Files.write(bundle.resolve("textures/concrete_floor.png"), new byte[] {1, 2, 3});

        BundlePack pack = new BundlePack(bundle, location());

        assertNotNull(
                pack.getResource(PackType.CLIENT_RESOURCES, MaterialTable.LOCATION),
                "materials.json has to arrive as src2mc:bundle/materials.json");
        assertNotNull(
                pack.getResource(
                        PackType.CLIENT_RESOURCES,
                        ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "textures/bundle/concrete_floor.png")),
                "a bundle texture has to arrive where the atlas source looks for it");

        // A bundle is data the user points at, so it must not be a way to read
        // the rest of the disk.
        assertNull(pack.getResource(
                PackType.CLIENT_RESOURCES,
                ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "bundle/../../secret.txt")));

        Map<ResourceLocation, IoSupplier<java.io.InputStream>> listed = new HashMap<>();
        pack.listResources(PackType.CLIENT_RESOURCES, Src2mc.MOD_ID, "textures/bundle", listed::put);
        assertTrue(
                listed.containsKey(ResourceLocation.fromNamespaceAndPath(
                        Src2mc.MOD_ID, "textures/bundle/concrete_floor.png")),
                "the atlas finds textures by listing, so listing has to find them: " + listed.keySet());
    }

    @Test
    void aBundleOfTheWrongVersionIsRefused(@TempDir Path bundle) throws IOException {
        int wrong = Src2mc.FORMAT_VERSION + 1;
        Files.writeString(
                bundle.resolve("bundle.json"),
                "{\"format_version\": " + wrong + ", \"name\": \"later\"}",
                StandardCharsets.UTF_8);

        assertTrue(BundleIndex.read(bundle).isEmpty(), "a bundle from a later format must not be half-read");

        Files.writeString(
                bundle.resolve("bundle.json"),
                "{\"format_version\": " + Src2mc.FORMAT_VERSION + ", \"name\": \"entropy-zero\","
                        + " \"scale\": {\"units_per_block\": 16.0}}",
                StandardCharsets.UTF_8);
        BundleIndex index = BundleIndex.read(bundle).orElseThrow();
        assertEquals("entropy-zero", index.name());
        assertEquals(16.0F, index.unitsPerBlock());
    }

    /** {@code docs/format.md} §1: the version is read first and named in full when it is wrong. */
    @Test
    void aSchematicOfTheWrongVersionIsRefusedByName() {
        int wrong = Src2mc.FORMAT_VERSION + 7;
        CompoundTag root = schematic(wrong);

        FormatVersion.UnsupportedVersion refusal = assertThrows(
                FormatVersion.UnsupportedVersion.class, () -> SchematicHeader.read(root, "map.schem"));
        assertTrue(refusal.getMessage().contains(String.valueOf(wrong)), refusal.getMessage());
        assertTrue(refusal.getMessage().contains(String.valueOf(Src2mc.FORMAT_VERSION)), refusal.getMessage());
        assertTrue(refusal.getMessage().contains("map.schem"), refusal.getMessage());
    }

    /** Output older than the format carries no version at all, and that is version 0. */
    @Test
    void outputFromBeforeTheFormatIsVersionZero() {
        CompoundTag root = new CompoundTag();
        CompoundTag schematic = new CompoundTag();
        schematic.put("Metadata", new CompoundTag());
        root.put("Schematic", schematic);

        FormatVersion.UnsupportedVersion refusal = assertThrows(
                FormatVersion.UnsupportedVersion.class, () -> SchematicHeader.read(root, "old.schem"));
        assertTrue(refusal.getMessage().contains("Re-run the converter"), refusal.getMessage());
        assertFalse(FormatVersion.isSupported(FormatVersion.ABSENT));
    }

    @Test
    void theRightVersionReadsItsHeader() {
        SchematicHeader header = SchematicHeader.read(schematic(Src2mc.FORMAT_VERSION), "map.schem");
        assertEquals(Src2mc.FORMAT_VERSION, header.formatVersion());
        assertEquals("tiny", header.name());
        assertEquals(2, header.width());
    }

    private static CompoundTag schematic(int version) {
        CompoundTag metadata = new CompoundTag();
        metadata.putInt(SchematicHeader.VERSION_KEY, version);
        metadata.putString("Name", "tiny");

        CompoundTag schematic = new CompoundTag();
        schematic.put("Metadata", metadata);
        schematic.putShort("Width", (short) 2);
        schematic.putShort("Height", (short) 3);
        schematic.putShort("Length", (short) 4);

        CompoundTag root = new CompoundTag();
        root.put("Schematic", schematic);
        return root;
    }
}
