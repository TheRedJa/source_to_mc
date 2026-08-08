package io.github.theredja.src2mc;

import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.builder.LiteralArgumentBuilder;
import io.github.theredja.src2mc.bundle.BundleIndex;
import io.github.theredja.src2mc.bundle.BundlePackFinder;
import io.github.theredja.src2mc.bundle.MaterialTable;
import io.github.theredja.src2mc.schematic.SchematicHeader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Optional;
import net.minecraft.ChatFormatting;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.network.chat.Component;
import net.neoforged.fml.loading.FMLPaths;
import net.neoforged.neoforge.event.RegisterCommandsEvent;

/**
 * {@code /src2mc} — what the mod thinks it has loaded, and the version check on
 * a file, so both can be seen without reading a log.
 */
public final class Src2mcCommands {
    private Src2mcCommands() {}

    public static void register(RegisterCommandsEvent event) {
        LiteralArgumentBuilder<CommandSourceStack> root = Commands.literal(Src2mc.MOD_ID)
                .requires(source -> source.hasPermission(2))
                .then(Commands.literal("status").executes(Src2mcCommands::status))
                .then(Commands.literal("check")
                        .then(Commands.argument("file", StringArgumentType.greedyString())
                                .executes(Src2mcCommands::check)));
        event.getDispatcher().register(root);
    }

    private static int status(com.mojang.brigadier.context.CommandContext<CommandSourceStack> context) {
        CommandSourceStack source = context.getSource();
        Path root = BundlePackFinder.bundleRoot();
        source.sendSuccess(() -> Component.literal("format version " + Src2mc.FORMAT_VERSION
                + ", surface pool " + Src2mc.SURFACE_POOL_SIZE), false);
        source.sendSuccess(() -> Component.literal("bundle " + root), false);
        if (Files.isDirectory(root)) {
            Optional<BundleIndex> index = BundleIndex.read(root);
            source.sendSuccess(
                    () -> index.map(bundle -> Component.literal(
                                    "bundle '" + bundle.name() + "', " + bundle.unitsPerBlock() + " units per block"))
                            .orElseGet(() -> Component.literal("bundle rejected, see the log")
                                    .withStyle(ChatFormatting.RED)),
                    false);
        } else {
            source.sendSuccess(
                    () -> Component.literal("no bundle directory there").withStyle(ChatFormatting.RED), false);
        }
        source.sendSuccess(
                () -> Component.literal("materials loaded: " + MaterialTable.server().size()), false);
        return 1;
    }

    private static int check(com.mojang.brigadier.context.CommandContext<CommandSourceStack> context) {
        CommandSourceStack source = context.getSource();
        Path gameDir = FMLPaths.GAMEDIR.get().toAbsolutePath().normalize();
        Path file = gameDir.resolve(StringArgumentType.getString(context, "file"))
                .toAbsolutePath()
                .normalize();
        if (!file.startsWith(gameDir)) {
            source.sendFailure(Component.literal("that path leaves the game directory"));
            return 0;
        }
        try {
            SchematicHeader header = SchematicHeader.read(file);
            source.sendSuccess(
                    () -> Component.literal("'" + header.name() + "' format version " + header.formatVersion()
                            + ", " + header.width() + "x" + header.height() + "x" + header.length()),
                    false);
            return 1;
        } catch (FormatVersion.UnsupportedVersion e) {
            source.sendFailure(Component.literal(e.getMessage()).withStyle(ChatFormatting.RED));
            return 0;
        } catch (Exception e) {
            source.sendFailure(Component.literal("cannot read " + file + ": " + e));
            return 0;
        }
    }
}
