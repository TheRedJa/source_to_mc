package io.github.theredja.src2mc.bundle;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import io.github.theredja.src2mc.Src2mc;
import java.io.IOException;
import java.io.Reader;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.resources.Resource;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.server.packs.resources.ResourceManagerReloadListener;

/**
 * {@code materials.json} from the bundle, indexed by surface index.
 *
 * <p>{@code docs/format.md} §4. Entry {@code i} describes
 * {@code src2mc:surface_i}; there is no other link between a block and its
 * appearance. The table is data, replaced on reload, and never a registry
 * object (D6).
 */
public final class MaterialTable {
    /** Where the bundle's material table lands once {@link BundlePack} has mapped it. */
    public static final ResourceLocation LOCATION =
            ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "bundle/materials.json");

    /** The client's copy, read from the resource pack. Written on {@code F3+T}. */
    private static volatile MaterialTable client = empty();

    /** The server's copy, read from the data pack. Written on {@code /reload}. */
    private static volatile MaterialTable server = empty();

    private final List<Entry> entries;

    private MaterialTable(List<Entry> entries) {
        this.entries = List.copyOf(entries);
    }

    public static MaterialTable empty() {
        return new MaterialTable(List.of());
    }

    public static MaterialTable client() {
        return client;
    }

    public static MaterialTable server() {
        return server;
    }

    public int size() {
        return this.entries.size();
    }

    /**
     * The entry for a surface index, or empty when the bundle describes fewer
     * materials than the pool holds — which is the normal case, since the pool
     * size does not depend on any map.
     */
    public Optional<Entry> get(int index) {
        return index >= 0 && index < this.entries.size()
                ? Optional.of(this.entries.get(index))
                : Optional.empty();
    }

    /** One material. {@code index} is its position in the file, so its surface block. */
    public record Entry(
            int index,
            String material,
            ResourceLocation sprite,
            float blocksPerRepeatU,
            float blocksPerRepeatV,
            RenderKind renderType,
            String surfaceProp,
            String sound) {}

    /** {@code render_type} in {@code docs/format.md} §4. */
    public enum RenderKind {
        SOLID,
        CUTOUT,
        TRANSLUCENT;

        static RenderKind parse(String name) {
            return switch (name) {
                case "solid" -> SOLID;
                case "cutout" -> CUTOUT;
                case "translucent" -> TRANSLUCENT;
                default -> throw new IllegalArgumentException(
                        "render_type must be solid, cutout or translucent, not " + name
                                + " (docs/format.md §4)");
            };
        }
    }

    public static MaterialTable parse(Reader reader) {
        JsonElement root = JsonParser.parseReader(reader);
        if (!root.isJsonArray()) {
            throw new IllegalArgumentException(
                    "materials.json is an array indexed by surface index (docs/format.md §4)");
        }
        JsonArray array = root.getAsJsonArray();
        List<Entry> entries = new ArrayList<>(array.size());
        for (int i = 0; i < array.size(); i++) {
            entries.add(parseEntry(i, array.get(i).getAsJsonObject()));
        }
        return new MaterialTable(entries);
    }

    private static Entry parseEntry(int index, JsonObject json) {
        JsonArray repeat = json.getAsJsonArray("blocks_per_repeat");
        if (repeat.size() != 2) {
            throw new IllegalArgumentException(
                    "blocks_per_repeat is two numbers, one per axis, at material " + index);
        }
        float u = repeat.get(0).getAsFloat();
        float v = repeat.get(1).getAsFloat();
        if (!(u > 0.0F) || !(v > 0.0F)) {
            throw new IllegalArgumentException(
                    "blocks_per_repeat must be positive at material " + index + ", got " + u + ", " + v);
        }
        return new Entry(
                index,
                json.get("material").getAsString(),
                BundlePack.spriteFor(json.get("texture").getAsString()),
                u,
                v,
                RenderKind.parse(json.get("render_type").getAsString()),
                json.has("surface_prop") ? json.get("surface_prop").getAsString() : "",
                json.has("sound") ? json.get("sound").getAsString() : "stone");
    }

    /**
     * Reads the table out of whichever pack stack the manager holds, so the same
     * code serves {@code F3+T} on the client and {@code /reload} on the server.
     */
    public static MaterialTable load(ResourceManager resources) {
        Optional<Resource> resource = resources.getResource(LOCATION);
        if (resource.isEmpty()) {
            Src2mc.LOG.warn(
                    "no {} — no bundle is mounted, so every surface block will render as missing texture",
                    LOCATION);
            return empty();
        }
        try (Reader reader = resource.get().openAsReader()) {
            MaterialTable table = parse(reader);
            Src2mc.LOG.info(
                    "material table: {} materials for a pool of {}",
                    table.size(),
                    Src2mc.SURFACE_POOL_SIZE);
            if (table.size() > Src2mc.SURFACE_POOL_SIZE) {
                Src2mc.LOG.error(
                        "bundle describes {} materials but this build registers {} surface blocks;"
                                + " materials from index {} up cannot be shown. Growing the pool is a"
                                + " mod update and a restart, not a reload.",
                        table.size(),
                        Src2mc.SURFACE_POOL_SIZE,
                        Src2mc.SURFACE_POOL_SIZE);
            }
            return table;
        } catch (IOException | RuntimeException e) {
            Src2mc.LOG.error("cannot read {}: {}", LOCATION, e.toString());
            return empty();
        }
    }

    public static ResourceManagerReloadListener reloadListener() {
        return resources -> server = load(resources);
    }

    public static ResourceManagerReloadListener clientReloadListener() {
        return resources -> client = load(resources);
    }
}
