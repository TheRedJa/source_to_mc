package io.github.theredja.src2mc.bundle;

import io.github.theredja.src2mc.Src2mc;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.Set;
import java.util.stream.Stream;
import javax.annotation.Nullable;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.metadata.MetadataSectionSerializer;
import net.minecraft.server.packs.resources.IoSupplier;

/**
 * A bundle directory ({@code docs/format.md} §4) seen as a pack.
 *
 * <p>The bundle is a plain directory the user points at, laid out for the
 * converter's convenience, not Minecraft's. This class is the whole of the
 * translation between the two:
 *
 * <table>
 *   <tr><th>Resource</th><th>File</th></tr>
 *   <tr><td>{@code src2mc:bundle/materials.json}</td><td>{@code <bundle>/materials.json}</td></tr>
 *   <tr><td>{@code src2mc:textures/bundle/<p>.png}</td><td>{@code <bundle>/textures/<p>.png}</td></tr>
 * </table>
 *
 * <p>Textures land under {@code textures/bundle/} because that is where the
 * atlas source in {@code assets/minecraft/atlases/blocks.json} looks for them,
 * and the sprite they stitch to is {@code src2mc:bundle/<p>} — see
 * {@link #spriteFor}.
 *
 * <p>Nothing is cached. Every read goes to disk, so editing a texture and
 * pressing {@code F3+T} shows the new one.
 */
public final class BundlePack implements PackResources {
    /** Everything under here in the resource tree comes straight out of the bundle root. */
    private static final String DIRECT = "bundle/";

    /** …and everything under here out of the bundle's own {@code textures/}. */
    private static final String TEXTURES = "textures/bundle/";

    private final Path root;
    private final PackLocationInfo location;

    public BundlePack(Path root, PackLocationInfo location) {
        this.root = root.toAbsolutePath().normalize();
        this.location = location;
    }

    /**
     * The atlas sprite a material table {@code texture} field names.
     *
     * <p>{@code "textures/concrete_floor.png"} in the bundle is
     * {@code src2mc:bundle/concrete_floor} on the block atlas.
     */
    public static ResourceLocation spriteFor(String texture) {
        String path = texture;
        if (path.startsWith("textures/")) {
            path = path.substring("textures/".length());
        }
        if (path.endsWith(".png")) {
            path = path.substring(0, path.length() - ".png".length());
        }
        return ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "bundle/" + path.toLowerCase(Locale.ROOT));
    }

    /** The file a resource path names, or null if it is not one this pack serves. */
    @Nullable
    private Path fileFor(String resourcePath) {
        String relative;
        if (resourcePath.startsWith(TEXTURES)) {
            relative = "textures/" + resourcePath.substring(TEXTURES.length());
        } else if (resourcePath.startsWith(DIRECT)) {
            relative = resourcePath.substring(DIRECT.length());
        } else {
            return null;
        }

        Path file = this.root.resolve(relative).normalize();
        // A resource path may contain dots, so a bundle cannot be a way to read
        // files outside itself.
        return file.startsWith(this.root) ? file : null;
    }

    @Nullable
    @Override
    public IoSupplier<InputStream> getRootResource(String... elements) {
        if (elements.length == 1 && elements[0].equals(PACK_META)) {
            return () -> new java.io.ByteArrayInputStream(
                    ("{\"pack\":{\"description\":\"src2mc bundle\",\"pack_format\":0}}")
                            .getBytes(java.nio.charset.StandardCharsets.UTF_8));
        }
        return null;
    }

    @Nullable
    @Override
    public IoSupplier<InputStream> getResource(PackType type, ResourceLocation id) {
        if (!id.getNamespace().equals(Src2mc.MOD_ID)) {
            return null;
        }
        Path file = fileFor(id.getPath());
        if (file == null || !Files.isRegularFile(file)) {
            return null;
        }
        return () -> Files.newInputStream(file);
    }

    @Override
    public void listResources(PackType type, String namespace, String path, ResourceOutput out) {
        if (!namespace.equals(Src2mc.MOD_ID)) {
            return;
        }
        // `path` is a prefix in the resource tree. Only the two prefixes this
        // pack maps can produce anything, so walk both and keep what matches.
        String wanted = path.endsWith("/") ? path : path + "/";
        for (String prefix : new String[] {DIRECT, TEXTURES}) {
            Path dir = fileFor(prefix);
            if (dir == null || !Files.isDirectory(dir)) {
                continue;
            }
            try (Stream<Path> walk = Files.walk(dir)) {
                walk.filter(Files::isRegularFile).forEach(file -> {
                    String suffix = dir.relativize(file).toString().replace('\\', '/');
                    String resourcePath = prefix + suffix;
                    if (!resourcePath.startsWith(wanted)) {
                        return;
                    }
                    ResourceLocation id = ResourceLocation.tryBuild(Src2mc.MOD_ID, resourcePath);
                    if (id != null) {
                        out.accept(id, () -> Files.newInputStream(file));
                    }
                });
            } catch (IOException e) {
                Src2mc.LOG.error("cannot list {}: {}", dir, e.toString());
            }
        }
    }

    @Override
    public Set<String> getNamespaces(PackType type) {
        return Set.of(Src2mc.MOD_ID);
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
