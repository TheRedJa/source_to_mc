package io.github.theredja.src2mc.schematic;

import io.github.theredja.src2mc.FormatVersion;
import java.io.BufferedInputStream;
import java.io.DataInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.PushbackInputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.zip.GZIPInputStream;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtAccounter;
import net.minecraft.nbt.NbtIo;

/**
 * What {@code docs/format.md} §1 says has to be read first.
 *
 * <p>A schematic is Sponge v3 and mostly WorldEdit's business, but the version
 * under {@code Metadata.src2mc:FormatVersion} is ours, and it is checked before
 * anything else in the file is believed.
 */
public record SchematicHeader(String name, int formatVersion, int width, int height, int length) {
    public static final String VERSION_KEY = "src2mc:FormatVersion";

    /**
     * Reads the header and refuses a version this mod does not know.
     *
     * @throws FormatVersion.UnsupportedVersion naming both version numbers
     */
    public static SchematicHeader read(CompoundTag root, String what) {
        CompoundTag schematic = root.getCompound("Schematic");
        CompoundTag metadata = schematic.getCompound("Metadata");

        int version = metadata.contains(VERSION_KEY) ? metadata.getInt(VERSION_KEY) : FormatVersion.ABSENT;
        FormatVersion.require(version, what);

        return new SchematicHeader(
                metadata.contains("Name") ? metadata.getString("Name") : "",
                version,
                schematic.getShort("Width") & 0xFFFF,
                schematic.getShort("Height") & 0xFFFF,
                schematic.getShort("Length") & 0xFFFF);
    }

    /** Reads a {@code .schem}, gzipped or not — the fixtures are not. */
    public static SchematicHeader read(Path file) throws IOException {
        try (InputStream raw = new BufferedInputStream(Files.newInputStream(file))) {
            CompoundTag root = NbtIo.read(new DataInputStream(maybeGunzip(raw)), NbtAccounter.unlimitedHeap());
            return read(root, file.getFileName().toString());
        }
    }

    private static InputStream maybeGunzip(InputStream in) throws IOException {
        PushbackInputStream pushback = new PushbackInputStream(in, 2);
        int first = pushback.read();
        int second = pushback.read();
        if (second >= 0) {
            pushback.unread(second);
        }
        if (first >= 0) {
            pushback.unread(first);
        }
        boolean gzipped = first == 0x1F && second == 0x8B;
        return gzipped ? new GZIPInputStream(pushback) : pushback;
    }
}
