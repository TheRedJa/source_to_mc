package dev.theredja.src2mc;

import java.nio.file.Path;
import net.neoforged.fml.loading.FMLPaths;
import net.neoforged.neoforge.common.ModConfigSpec;

public final class Src2mcConfig {
    static final ModConfigSpec SPEC;
    private static final ModConfigSpec.ConfigValue<String> BUNDLE_DIRECTORY;
    private static final ModConfigSpec.ConfigValue<String> SCHEMATIC_DIRECTORY;
    private static final ModConfigSpec.BooleanValue VERBOSE_DIAGNOSTICS;
    private static final ModConfigSpec.LongValue TEXTURE_RAM_BUDGET;
    private static final ModConfigSpec.LongValue TEXTURE_VRAM_BUDGET;

    static {
        var builder = new ModConfigSpec.Builder();
        BUNDLE_DIRECTORY = builder
            .comment("Game-directory-relative folder containing .src2mc campaign bundles")
            .define("bundleDirectory", "config/src2mc/bundles", Src2mcConfig::validRelativePath);
        SCHEMATIC_DIRECTORY = builder
            .comment("Game-directory-relative folder containing <mapId>.schem files for /src2mc place")
            .define("schematicDirectory", "config/src2mc/schematics", Src2mcConfig::validRelativePath);
        VERBOSE_DIAGNOSTICS = builder
            .comment("Include per-bundle validation details in logs")
            .define("verboseDiagnostics", false);
        TEXTURE_RAM_BUDGET = builder
            .comment("Maximum decoded src2mc texture-page RAM residency in bytes")
            .defineInRange("textureRamBudgetBytes", 2L * 1024 * 1024 * 1024,
                128L * 1024 * 1024, 16L * 1024 * 1024 * 1024);
        TEXTURE_VRAM_BUDGET = builder
            .comment("Maximum estimated src2mc texture-page GPU residency in bytes")
            .defineInRange("textureVramBudgetBytes", 2L * 1024 * 1024 * 1024,
                128L * 1024 * 1024, 16L * 1024 * 1024 * 1024);
        SPEC = builder.build();
    }

    private Src2mcConfig() {
    }

    static Path bundleDirectory() {
        return FMLPaths.GAMEDIR.get().resolve(BUNDLE_DIRECTORY.get()).normalize();
    }

    static Path schematicDirectory() {
        return FMLPaths.GAMEDIR.get().resolve(SCHEMATIC_DIRECTORY.get()).normalize();
    }

    static boolean verboseDiagnostics() {
        return VERBOSE_DIAGNOSTICS.get();
    }

    public static long textureRamBudgetBytes() {
        return TEXTURE_RAM_BUDGET.get();
    }

    public static long textureVramBudgetBytes() {
        return TEXTURE_VRAM_BUDGET.get();
    }

    private static boolean validRelativePath(Object value) {
        if (!(value instanceof String text) || text.isBlank()) return false;
        Path path;
        try {
            path = Path.of(text);
        } catch (RuntimeException exception) {
            return false;
        }
        return !path.isAbsolute() && path.normalize().startsWith("config") && !path.normalize().startsWith("..");
    }
}
