package dev.theredja.src2mc.world;

import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtAccounter;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.Tag;

/**
 * Parses a Sponge Schematic v3 {@code .schem} with vanilla's own NBT reader,
 * standing in for WorldEdit's paste. Format is fixed by {@code src/output/schem.rs}:
 * gzipped NBT, {@code Blocks.Data} is a varint-per-cell array indexed by
 * {@code x + z*Width + y*Width*Length}, and the only palette entries a src2mc
 * schematic ever contains are air, surface, map_anchor and prop_root. Kept free
 * of block-registry lookups so it can run in a plain JVM unit test; {@link WorldPlacer}
 * resolves block names against the registry.
 */
public final class SchematicReader {
    public static final String AIR = "minecraft:air";
    public static final String SURFACE = "src2mc:surface";
    public static final String MAP_ANCHOR = "src2mc:map_anchor";
    public static final String PROP_ROOT = "src2mc:prop_root";

    private SchematicReader() {
    }

    /** One non-air cell, with its block-entity payload if the block carries one. */
    public record Cell(int x, int y, int z, String blockName, CompoundTag payload) {
    }

    public record Result(int[] offset, int width, int height, int length, List<Cell> cells) {
    }

    public static Result read(Path path) throws IOException {
        CompoundTag root = NbtIo.readCompressed(path, NbtAccounter.unlimitedHeap());
        CompoundTag schematic = root.getCompound("Schematic");
        int width = schematic.getShort("Width");
        int height = schematic.getShort("Height");
        int length = schematic.getShort("Length");
        int[] offset = schematic.getIntArray("Offset");
        if (offset.length != 3) throw new IOException("schematic Offset must have 3 components");

        CompoundTag blocksTag = schematic.getCompound("Blocks");
        String[] palette = readPalette(blocksTag.getCompound("Palette"));
        byte[] data = blocksTag.getByteArray("Data");
        int volume = width * height * length;
        int[] ids = decodeVarints(data, volume);

        Map<Long, CompoundTag> payloads = new HashMap<>();
        if (blocksTag.contains("BlockEntities", Tag.TAG_LIST)) {
            var entities = blocksTag.getList("BlockEntities", Tag.TAG_COMPOUND);
            for (int i = 0; i < entities.size(); i++) {
                CompoundTag entity = entities.getCompound(i);
                int[] pos = entity.getIntArray("Pos");
                if (pos.length != 3) throw new IOException("block entity Pos must have 3 components");
                payloads.put(cellKey(pos[0], pos[1], pos[2]), entity.getCompound("Data"));
            }
        }

        List<Cell> cells = new ArrayList<>();
        for (int i = 0; i < volume; i++) {
            int id = ids[i];
            if (id < 0 || id >= palette.length) throw new IOException("schematic block id " + id + " has no palette entry");
            String name = palette[id];
            if (name.equals(AIR)) continue;
            if (!name.equals(SURFACE) && !name.equals(MAP_ANCHOR) && !name.equals(PROP_ROOT)) {
                throw new IOException("schematic contains unsupported block " + name);
            }
            int y = i / (width * length);
            int remainder = i % (width * length);
            int z = remainder / width;
            int x = remainder % width;
            CompoundTag payload = payloads.get(cellKey(x, y, z));
            if ((name.equals(MAP_ANCHOR) || name.equals(PROP_ROOT)) && payload == null) {
                throw new IOException("schematic cell (" + x + "," + y + "," + z + ") is " + name + " but has no block-entity payload");
            }
            cells.add(new Cell(x, y, z, name, payload));
        }
        return new Result(offset, width, height, length, cells);
    }

    private static String[] readPalette(CompoundTag paletteTag) throws IOException {
        var keys = paletteTag.getAllKeys();
        String[] palette = new String[keys.size()];
        for (String key : keys) {
            int id = paletteTag.getInt(key);
            if (id < 0 || id >= palette.length) throw new IOException("schematic palette id " + id + " out of range");
            palette[id] = key;
        }
        for (String name : palette) if (name == null) throw new IOException("schematic palette has a gap");
        return palette;
    }

    /** Unsigned LEB128, matching {@code write_varint} in schem.rs. */
    private static int[] decodeVarints(byte[] data, int count) throws IOException {
        int[] out = new int[count];
        int position = 0;
        for (int i = 0; i < count; i++) {
            int value = 0;
            int shift = 0;
            while (true) {
                if (position >= data.length) throw new IOException("schematic block data ends mid-varint");
                int b = data[position++] & 0xFF;
                value |= (b & 0x7F) << shift;
                if ((b & 0x80) == 0) break;
                shift += 7;
            }
            out[i] = value;
        }
        return out;
    }

    private static long cellKey(int x, int y, int z) {
        return (((long) x & 0xFFFFF) << 42) | (((long) y & 0xFFFFF) << 21) | ((long) z & 0x1FFFFF);
    }
}
