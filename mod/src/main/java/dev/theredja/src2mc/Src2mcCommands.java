package dev.theredja.src2mc;

import static net.minecraft.commands.Commands.literal;

import net.minecraft.network.chat.Component;
import net.neoforged.neoforge.event.RegisterCommandsEvent;

/** Common-side command entry point. Subcommands are added as their phases land. */
final class Src2mcCommands {
    private Src2mcCommands() {
    }

    static void register(RegisterCommandsEvent event) {
        event.getDispatcher().register(literal(Src2mc.MOD_ID)
            .then(literal("reload")
                .requires(source -> source.hasPermission(2))
                .executes(context -> reload(context.getSource())))
            .then(literal("validate")
                .requires(source -> source.hasPermission(2))
                .executes(context -> validate(context.getSource())))
            .then(literal("reconcile")
                .requires(source -> source.hasPermission(2))
                .executes(context -> reconcile(context.getSource())))
            .then(literal("status").executes(context -> {
                var generation = Src2mc.BUNDLES.active();
                context.getSource().sendSuccess(
                    () -> Component.literal(generation.sequence() == 0
                        ? "src2mc: no campaign generation loaded; folder=" + Src2mc.BUNDLES.directory()
                        : "src2mc: generation " + generation.sequence() + ", bundles="
                            + generation.bundles().size() + ", fingerprint=" + generation.fingerprint()),
                    false
                );
                return 1;
            })));
    }

    private static int validate(net.minecraft.commands.CommandSourceStack source) {
        try {
            var candidate = Src2mc.BUNDLES.validateCandidate();
            source.sendSuccess(
                () -> Component.literal("src2mc: valid bundles=" + candidate.bundles().size()
                    + ", fingerprint=" + candidate.fingerprint()),
                false
            );
            if (Src2mcConfig.verboseDiagnostics()) {
                candidate.bundles().forEach(bundle -> Src2mc.LOGGER.info(
                    "Validated src2mc bundle {}: campaign={}, entries={}, bytes={}, fingerprint={}",
                    bundle.path(), bundle.campaignId(), bundle.entries().size(), bundle.uncompressedBytes(), bundle.fingerprint()
                ));
            }
            warnForTallMaps(source, candidate);
            return 1;
        } catch (java.io.IOException exception) {
            Src2mc.LOGGER.error("src2mc validation failed; active generation was not changed", exception);
            source.sendFailure(Component.literal("src2mc: validation failed: " + exception.getMessage()));
            return 0;
        }
    }

    private static int reload(net.minecraft.commands.CommandSourceStack source) {
        long retained = Src2mc.BUNDLES.active().sequence();
        try {
            var generation = Src2mc.BUNDLES.reload();
            Src2mc.LOGGER.info(
                "Published src2mc generation {}: bundles={}, fingerprint={}",
                generation.sequence(), generation.bundles().size(), generation.fingerprint()
            );
            source.sendSuccess(
                () -> Component.literal("src2mc: loaded generation " + generation.sequence()
                    + " with " + generation.bundles().size() + " bundle(s)"),
                true
            );
            warnForTallMaps(source, generation);
            return 1;
        } catch (java.io.IOException exception) {
            Src2mc.LOGGER.error("src2mc reload failed; retaining generation {}", retained, exception);
            source.sendFailure(Component.literal("src2mc: reload failed; retained generation " + retained
                + ": " + exception.getMessage()));
            return 0;
        }
    }

    private static int reconcile(net.minecraft.commands.CommandSourceStack source) {
        var level = source.getLevel();
        var position = net.minecraft.core.BlockPos.containing(source.getPosition());
        var chunk = level.getChunk(position.getX() >> 4, position.getZ() >> 4);
        var counts = dev.theredja.src2mc.world.WorldReconciler.reconcileChunk(level, chunk);
        source.sendSuccess(() -> Component.literal("src2mc: reconciled current chunk; anchors=" + counts.anchors()
            + ", healed=" + counts.healed() + ", diagnostics=" + counts.failures()), false);
        return counts.failures() == 0 ? 1 : 0;
    }

    private static void warnForTallMaps(net.minecraft.commands.CommandSourceStack source,
                                         dev.theredja.src2mc.bundle.BundleGeneration generation) {
        generation.bundles().forEach(bundle -> bundle.maps().stream()
            .filter(dev.theredja.src2mc.bundle.BundleMap::exceedsVanillaBuildHeight)
            .forEach(map -> source.sendSystemMessage(Component.literal(
                "src2mc warning: map " + map.mapId() + " is taller than vanilla's 384-block build height; "
                    + "configure your user-managed KubeJS height datapack before pasting it (src2mc will not install one)."
            ))));
    }
}
