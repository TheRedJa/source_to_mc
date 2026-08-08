package io.github.theredja.src2mc.bundle;

import io.github.theredja.src2mc.Src2mc;
import java.io.ByteArrayInputStream;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.Set;
import javax.annotation.Nullable;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.metadata.MetadataSectionSerializer;
import net.minecraft.server.packs.resources.IoSupplier;

/**
 * The blockstate and model JSON for the surface pool, made up on the spot.
 *
 * <p>Every one of the {@link Src2mc#SURFACE_POOL_SIZE} blocks needs a blockstate
 * file and a model file or the model loader complains, and all of them say the
 * same two things with a different number in them. Committing that many
 * near-identical files would be the duplication this project exists to remove,
 * so they are generated here instead. The count is a compile-time constant, so
 * this stays within D6: no map, texture or material affects what this pack
 * serves.
 */
public final class GeneratedPack implements PackResources {
    private static final String BLOCKSTATE_PREFIX = "blockstates/surface_";
    private static final String MODEL_PREFIX = "models/block/surface_";
    private static final String SUFFIX = ".json";

    private final PackLocationInfo location;

    public GeneratedPack(PackLocationInfo location) {
        this.location = location;
    }

    /** The index in {@code surface_<n>.json}, or -1 if this is not one of ours. */
    private static int indexOf(String path, String prefix) {
        if (!path.startsWith(prefix) || !path.endsWith(SUFFIX)) {
            return -1;
        }
        String digits = path.substring(prefix.length(), path.length() - SUFFIX.length());
        try {
            int index = Integer.parseInt(digits);
            return index >= 0 && index < Src2mc.SURFACE_POOL_SIZE ? index : -1;
        } catch (NumberFormatException e) {
            return -1;
        }
    }

    private static IoSupplier<InputStream> of(String json) {
        byte[] bytes = json.getBytes(StandardCharsets.UTF_8);
        return () -> new ByteArrayInputStream(bytes);
    }

    private static String blockstate(int index) {
        return "{\"variants\":{\"\":{\"model\":\"" + Src2mc.MOD_ID + ":block/surface_" + index + "\"}}}";
    }

    /**
     * The model is a marker: the geometry loader registered under
     * {@code src2mc:surface} builds the quads, and the index is the only thing it
     * needs from the file.
     */
    private static String model(int index) {
        return "{\"loader\":\"" + Src2mc.MOD_ID + ":surface\",\"index\":" + index + "}";
    }

    @Nullable
    @Override
    public IoSupplier<InputStream> getRootResource(String... elements) {
        if (elements.length == 1 && elements[0].equals(PACK_META)) {
            return of("{\"pack\":{\"description\":\"src2mc surface pool\",\"pack_format\":0}}");
        }
        return null;
    }

    @Nullable
    @Override
    public IoSupplier<InputStream> getResource(PackType type, ResourceLocation id) {
        if (type != PackType.CLIENT_RESOURCES || !id.getNamespace().equals(Src2mc.MOD_ID)) {
            return null;
        }
        int blockstate = indexOf(id.getPath(), BLOCKSTATE_PREFIX);
        if (blockstate >= 0) {
            return of(blockstate(blockstate));
        }
        int model = indexOf(id.getPath(), MODEL_PREFIX);
        if (model >= 0) {
            return of(model(model));
        }
        return null;
    }

    @Override
    public void listResources(PackType type, String namespace, String path, ResourceOutput out) {
        if (type != PackType.CLIENT_RESOURCES || !namespace.equals(Src2mc.MOD_ID)) {
            return;
        }
        String wanted = path.endsWith("/") ? path : path + "/";
        for (int i = 0; i < Src2mc.SURFACE_POOL_SIZE; i++) {
            String blockstate = BLOCKSTATE_PREFIX + i + SUFFIX;
            if (blockstate.startsWith(wanted)) {
                out.accept(ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, blockstate), of(blockstate(i)));
            }
            String model = MODEL_PREFIX + i + SUFFIX;
            if (model.startsWith(wanted)) {
                out.accept(ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, model), of(model(i)));
            }
        }
    }

    @Override
    public Set<String> getNamespaces(PackType type) {
        return type == PackType.CLIENT_RESOURCES ? Set.of(Src2mc.MOD_ID) : Set.of();
    }

    @Nullable
    @Override
    public <T> T getMetadataSection(MetadataSectionSerializer<T> serializer) {
        return null;
    }

    @Override
    public PackLocationInfo location() {
        return this.location;
    }

    @Override
    public void close() {}
}
