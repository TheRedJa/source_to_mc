package io.github.theredja.src2mc.bundle;

import io.github.theredja.src2mc.Src2mc;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Optional;
import net.minecraft.network.chat.Component;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.PackSelectionConfig;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.Pack;
import net.minecraft.server.packs.repository.PackCompatibility;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.world.flag.FeatureFlagSet;
import net.neoforged.fml.loading.FMLPaths;
import net.neoforged.neoforge.event.AddPackFindersEvent;

/**
 * Mounts the bundle, and the generated pool assets, as packs.
 *
 * <p>Both are hidden and always on: they are not optional content the user picks
 * in a list, they are how the mod reads its own data. The bundle is mounted for
 * both pack types because the server needs the material table as much as the
 * client does — sounds now, collision later.
 *
 * <p>Where the bundle is, in order:
 *
 * <ol>
 *   <li>the {@code src2mc.bundle} system property, which is how the dev client
 *       and the tests point at one,
 *   <li>{@code <gamedir>/src2mc-bundle}.
 * </ol>
 */
public final class BundlePackFinder {
    public static final String BUNDLE_PROPERTY = "src2mc.bundle";
    public static final String DEFAULT_DIRECTORY = "src2mc-bundle";

    private BundlePackFinder() {}

    public static Path bundleRoot() {
        String override = System.getProperty(BUNDLE_PROPERTY);
        return override != null && !override.isBlank()
                ? Path.of(override)
                : FMLPaths.GAMEDIR.get().resolve(DEFAULT_DIRECTORY);
    }

    public static void onAddPackFinders(AddPackFindersEvent event) {
        if (event.getPackType() == PackType.CLIENT_RESOURCES) {
            PackLocationInfo where = info("src2mc_pool", "src2mc surface pool");
            event.addRepositorySource(consumer -> consumer.accept(
                    pack(where, new PackSelectionConfig(true, Pack.Position.BOTTOM, false), GeneratedPack::new)));
        }

        Path root = bundleRoot();
        if (!Files.isDirectory(root)) {
            Src2mc.LOG.info("no bundle at {} — surfaces will have no textures until one is there", root);
            return;
        }

        Optional<BundleIndex> index = BundleIndex.read(root);
        if (index.isEmpty()) {
            // BundleIndex has already said why, naming both version numbers if
            // that is the reason. Refusing to mount is the point: a bundle of
            // the wrong version half-read is worse than no bundle.
            return;
        }

        Src2mc.LOG.info(
                "mounting bundle '{}' from {} as {}", index.get().name(), root, event.getPackType());
        PackLocationInfo where = info("src2mc_bundle", "src2mc bundle: " + index.get().name());
        event.addRepositorySource(consumer -> consumer.accept(pack(
                where,
                new PackSelectionConfig(true, Pack.Position.TOP, false),
                info -> new BundlePack(root, info))));
    }

    private static PackLocationInfo info(String id, String title) {
        return new PackLocationInfo(id, Component.literal(title), PackSource.BUILT_IN, Optional.empty());
    }

    private static Pack pack(
            PackLocationInfo where,
            PackSelectionConfig selection,
            java.util.function.Function<PackLocationInfo, PackResources> open) {
        Pack.ResourcesSupplier supplier = new Pack.ResourcesSupplier() {
            @Override
            public PackResources openPrimary(PackLocationInfo info) {
                return open.apply(info);
            }

            @Override
            public PackResources openFull(PackLocationInfo info, Pack.Metadata metadata) {
                return open.apply(info);
            }
        };
        // Built by hand rather than through readMetaAndCreate: neither pack has a
        // pack.mcmeta worth reading, and a pack_format number that has to track
        // the game version is one more thing to get wrong.
        Pack.Metadata metadata = new Pack.Metadata(
                where.title(), PackCompatibility.COMPATIBLE, FeatureFlagSet.of(), java.util.List.of(), true);
        return new Pack(where, supplier, metadata, selection);
    }
}
