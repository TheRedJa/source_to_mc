package dev.theredja.src2mc.world;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.IntArrayTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.NbtIo;
import org.junit.jupiter.api.Test;

/** Builds a minimal Sponge v3 schematic by hand and checks it decodes the way schem.rs encoded it. */
class SchematicReaderTest {

    @Test
    void decodesBlocksAndPayloadsAtTheirCells() throws IOException {
        // 2x1x1 region: (0,0,0) is src2mc:surface, (1,0,0) is src2mc:prop_root.
        Map<String, Integer> palette = new LinkedHashMap<>();
        palette.put("minecraft:air", 0);
        palette.put("src2mc:surface", 1);
        palette.put("src2mc:prop_root", 2);

        CompoundTag propData = new CompoundTag();
        propData.putInt("schema_version", 1);
        propData.putString("campaign_id", "hl2");
        propData.putString("map_id", "d1_01");
        propData.putString("stable_id", "ab".repeat(32));
        propData.putString("model_content_id", "cd".repeat(32));
        propData.put("root_cell", new IntArrayTag(new int[] {1, 0, 0}));
        propData.put("translation", doubleList(0.0, 0.0, 0.0));
        propData.put("rotation", doubleList(0.0, 0.0, 0.0, 1.0));
        propData.putDouble("scale", 1.0);

        CompoundTag propEntity = new CompoundTag();
        propEntity.putString("Id", "src2mc:prop_root");
        propEntity.put("Pos", new IntArrayTag(new int[] {1, 0, 0}));
        propEntity.put("Data", propData);

        ListTag blockEntities = new ListTag();
        blockEntities.add(propEntity);

        CompoundTag paletteTag = new CompoundTag();
        palette.forEach(paletteTag::putInt);

        // Cell order is x fastest: index(0)=surface, index(1)=prop_root.
        byte[] data = concatVarints(1, 2);

        CompoundTag blocksTag = new CompoundTag();
        blocksTag.put("Palette", paletteTag);
        blocksTag.putByteArray("Data", data);
        blocksTag.put("BlockEntities", blockEntities);

        CompoundTag schematicTag = new CompoundTag();
        schematicTag.putShort("Width", (short) 2);
        schematicTag.putShort("Height", (short) 1);
        schematicTag.putShort("Length", (short) 1);
        schematicTag.put("Offset", new IntArrayTag(new int[] {10, 20, 30}));
        schematicTag.put("Blocks", blocksTag);

        CompoundTag root = new CompoundTag();
        root.put("Schematic", schematicTag);

        Path path = Files.createTempFile("src2mc-schematic-reader-test", ".schem");
        try {
            NbtIo.writeCompressed(root, path);
            SchematicReader.Result result = SchematicReader.read(path);

            assertEquals(2, result.width());
            assertTrue(java.util.Arrays.equals(new int[] {10, 20, 30}, result.offset()));
            assertEquals(2, result.cells().size());

            var surface = result.cells().stream().filter(c -> c.x() == 0).findFirst().orElseThrow();
            assertEquals(SchematicReader.SURFACE, surface.blockName());
            assertEquals(null, surface.payload());

            var propRoot = result.cells().stream().filter(c -> c.x() == 1).findFirst().orElseThrow();
            assertEquals(SchematicReader.PROP_ROOT, propRoot.blockName());
            assertEquals("hl2", propRoot.payload().getString("campaign_id"));
        } finally {
            Files.deleteIfExists(path);
        }
    }

    @Test
    void unknownPaletteEntryIsAHardError() throws IOException {
        CompoundTag paletteTag = new CompoundTag();
        paletteTag.putInt("minecraft:stone", 0);

        CompoundTag blocksTag = new CompoundTag();
        blocksTag.put("Palette", paletteTag);
        blocksTag.putByteArray("Data", concatVarints(0));

        CompoundTag schematicTag = new CompoundTag();
        schematicTag.putShort("Width", (short) 1);
        schematicTag.putShort("Height", (short) 1);
        schematicTag.putShort("Length", (short) 1);
        schematicTag.put("Offset", new IntArrayTag(new int[] {0, 0, 0}));
        schematicTag.put("Blocks", blocksTag);

        CompoundTag root = new CompoundTag();
        root.put("Schematic", schematicTag);

        Path path = Files.createTempFile("src2mc-schematic-reader-test", ".schem");
        try {
            NbtIo.writeCompressed(root, path);
            assertThrows(IOException.class, () -> SchematicReader.read(path));
        } finally {
            Files.deleteIfExists(path);
        }
    }

    private static ListTag doubleList(double... values) {
        ListTag list = new ListTag();
        for (double value : values) list.add(net.minecraft.nbt.DoubleTag.valueOf(value));
        return list;
    }

    /** Matches {@code write_varint} in schem.rs: unsigned LEB128, one value per cell. */
    private static byte[] concatVarints(int... values) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (int value : values) {
            int remaining = value;
            while (true) {
                int b = remaining & 0x7F;
                remaining >>>= 7;
                if (remaining != 0) b |= 0x80;
                out.write(b);
                if (remaining == 0) break;
            }
        }
        return out.toByteArray();
    }
}
