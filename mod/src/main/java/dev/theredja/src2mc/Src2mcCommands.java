package dev.theredja.src2mc;

import static net.minecraft.commands.Commands.argument;
import static net.minecraft.commands.Commands.literal;

import com.mojang.brigadier.arguments.StringArgumentType;
import java.nio.file.Path;
import net.minecraft.core.BlockPos;
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
            .then(literal("place")
                .requires(source -> source.hasPermission(2))
                .then(argument("mapId", StringArgumentType.word())
                    .executes(context -> place(context.getSource(), StringArgumentType.getString(context, "mapId")))))
            .then(literal("lightmask")
                .requires(source -> source.hasPermission(2))
                .executes(context -> lightMaskStatus(context.getSource()))
                .then(argument("enabled", com.mojang.brigadier.arguments.BoolArgumentType.bool())
                    .executes(context -> lightMask(context.getSource(),
                        com.mojang.brigadier.arguments.BoolArgumentType.getBool(context, "enabled")))))
            .then(literal("lightcolumn")
                .requires(source -> source.hasPermission(2))
                .executes(context -> lightColumn(context.getSource()))
                .then(argument("height", com.mojang.brigadier.arguments.IntegerArgumentType.integer(1, 512))
                    .executes(context -> lightColumn(context.getSource(),
                        com.mojang.brigadier.arguments.IntegerArgumentType.getInteger(context, "height")))))
            .then(literal("status").executes(context -> {
                var generation = Src2mc.BUNDLES.active();
                context.getSource().sendSuccess(
                    () -> Component.literal(generation.sequence() == 0
                        ? "src2mc: no campaign generation loaded (load state "
                            + dev.theredja.src2mc.bundle.BundleLoadProgress.snapshot().state()
                            + "); folder=" + Src2mc.BUNDLES.directory()
                        : "src2mc: generation " + generation.sequence() + ", bundles="
                            + generation.bundles().size() + ", fingerprint=" + generation.fingerprint()),
                    false
                );
                return 1;
            })));
    }

    /**
     * What the bake holds here: its own value at the player, the value the
     * light engine ended up with, and the column above. The two disagreeing is
     * the signature of light that was published and then recomputed away.
     */
    private static int lightMaskStatus(net.minecraft.commands.CommandSourceStack source) {
        var level = source.getLevel();
        net.minecraft.core.BlockPos at = net.minecraft.core.BlockPos.containing(source.getPosition());
        var baked = dev.theredja.src2mc.world.LightOcclusion.baked(level);

        StringBuilder column = new StringBuilder();
        for (int y = at.getY() + 10; y >= at.getY() - 2; y--) {
            net.minecraft.core.BlockPos probe = new net.minecraft.core.BlockPos(at.getX(), y, at.getZ());
            int sky = dev.theredja.src2mc.world.LightOcclusion.skyAt(level, at.getX(), y, at.getZ());
            column.append(!level.getBlockState(probe).isAir() ? '#' : sky < 0 ? '?' : Character.forDigit(sky, 16));
        }
        source.sendSuccess(() -> Component.literal("src2mc: sky-light bake "
            + (dev.theredja.src2mc.world.LightOcclusion.enabled() ? "on" : "off")
            + "; shaded=" + baked.darkCells() + " open=" + baked.litCells()
            + " sections=" + baked.sections().size() + " in " + baked.millis() + "ms"
            + "; y=" + at.getY()
            + " baked=" + describe(dev.theredja.src2mc.world.LightOcclusion.skyAt(level, at.getX(), at.getY(), at.getZ()), -1)
            + " engine sky=" + level.getBrightness(net.minecraft.world.level.LightLayer.SKY, at)
            + " block=" + level.getBrightness(net.minecraft.world.level.LightLayer.BLOCK, at)
            + "; column +10..-2 " + column), false);
        return 1;
    }

    private static int lightColumn(net.minecraft.commands.CommandSourceStack source) {
        return lightColumn(source, 96);
    }

    /**
     * Everything above the player in one column: real blocks, what the bake
     * says, and what the light engine answers. This is where a ceiling that
     * should stop daylight either shows up or does not.
     */
    private static int lightColumn(net.minecraft.commands.CommandSourceStack source, int height) {
        var level = source.getLevel();
        net.minecraft.core.BlockPos at = net.minecraft.core.BlockPos.containing(source.getPosition());
        var placement = dev.theredja.src2mc.world.PlacementSavedData.get(level).index().at(at);
        StringBuilder found = new StringBuilder();
        int blocks = 0, shaded = 0;
        for (int above = 0; above <= height; above++) {
            net.minecraft.core.BlockPos probe = at.above(above);
            var state = level.getBlockState(probe);
            int baked = dev.theredja.src2mc.world.LightOcclusion.skyAt(level, probe.getX(), probe.getY(), probe.getZ());
            if (baked >= 0 && baked < 15) shaded++;
            if (state.isAir()) continue;
            blocks++;
            if (found.length() < 180) {
                found.append(' ').append(probe.getY()).append('#')
                    .append(state.getBlock().getName().getString().replace("Block of ", ""))
                    .append("/sky").append(level.getBrightness(net.minecraft.world.level.LightLayer.SKY, probe));
            }
        }
        // Neighbouring columns: a ceiling built from plates can miss the one
        // column being stood in while sealing the ones beside it.
        StringBuilder around = new StringBuilder();
        for (net.minecraft.core.Direction side : net.minecraft.core.Direction.Plane.HORIZONTAL) {
            net.minecraft.core.BlockPos next = at.relative(side);
            around.append(' ').append(side.getName().charAt(0))
                .append('=').append(describe(dev.theredja.src2mc.world.LightOcclusion
                    .skyAt(level, next.getX(), next.getY(), next.getZ()), -1))
                .append('/').append(level.getBrightness(net.minecraft.world.level.LightLayer.SKY, next));
        }
        String local = placement.map(value -> {
            var cell = value.toLocal(at);
            return value.mapId() + " local=" + cell.getX() + "," + cell.getY() + "," + cell.getZ();
        }).orElse("no placement here");
        int blockCount = blocks, shadedCount = shaded;
        source.sendSuccess(() -> Component.literal("src2mc: column at " + at.getX() + "," + at.getY() + "," + at.getZ()
            + " (" + local + "); within " + height + " above: blocks=" + blockCount + " shaded=" + shadedCount
            + ";" + (found.isEmpty() ? " no blocks at all" : found)
            + "; here baked/engine" + around), false);
        return 1;
    }

    private static String describe(int value, int absent) {
        return value == absent ? "none" : String.valueOf(value);
    }

    /**
     * Turn the sky-light bake on or off and republish, so its effect can be
     * compared in place rather than across two exports. Off hands the same
     * sections full daylight, which is what vanilla alone produces here.
     */
    private static int lightMask(net.minecraft.commands.CommandSourceStack source, boolean enabled) {
        dev.theredja.src2mc.world.LightOcclusion.setEnabled(enabled);
        int shaded = dev.theredja.src2mc.world.LightOcclusion.rebuild(source.getLevel());
        source.sendSuccess(() -> Component.literal("src2mc: sky-light bake "
            + (enabled ? "on" : "off") + "; shaded cells=" + shaded), true);
        return 1;
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
            long millis = dev.theredja.src2mc.bundle.BundleLoadProgress.snapshot().millis();
            source.sendSuccess(
                () -> Component.literal("src2mc: loaded generation " + generation.sequence()
                    + " with " + generation.bundles().size() + " bundle(s) in " + millis + " ms"),
                true
            );
            warnForTallMaps(source, generation);
            // The bake is computed from the bundles, so a new generation can
            // change it — including removing it when a map is withdrawn.
            dev.theredja.src2mc.world.LightOcclusion.rebuild(source.getLevel());
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

    private static int place(net.minecraft.commands.CommandSourceStack source, String mapId) {
        var generation = Src2mc.BUNDLES.active();
        var matches = generation.bundles().stream()
            .flatMap(bundle -> bundle.maps().stream()
                .filter(map -> map.mapId().equals(mapId))
                .map(map -> new dev.theredja.src2mc.bundle.BundleGeneration.LocatedMap(bundle, map)))
            .toList();
        if (matches.isEmpty()) {
            source.sendFailure(Component.literal("src2mc: no loaded bundle has map `" + mapId + "`"));
            return 0;
        }
        if (matches.size() > 1) {
            source.sendFailure(Component.literal("src2mc: map `" + mapId + "` is ambiguous across "
                + matches.size() + " bundles; rename one campaign to disambiguate"));
            return 0;
        }
        var located = matches.get(0);
        Path schematicPath = Src2mcConfig.schematicDirectory().resolve(mapId + ".schem");
        dev.theredja.src2mc.world.SchematicReader.Result schematic;
        try {
            schematic = dev.theredja.src2mc.world.SchematicReader.read(schematicPath);
        } catch (java.io.IOException exception) {
            Src2mc.LOGGER.error("src2mc place failed to read {}", schematicPath, exception);
            source.sendFailure(Component.literal("src2mc: failed to read " + schematicPath + ": " + exception.getMessage()));
            return 0;
        }
        int[] anchorCell = located.map().anchorCell();
        BlockPos anchorWorld = BlockPos.containing(source.getPosition());
        BlockPos translation = anchorWorld.subtract(new BlockPos(anchorCell[0], anchorCell[1], anchorCell[2]));

        var level = source.getLevel();
        int[] offset = schematic.offset();
        int minWorldY = Integer.MAX_VALUE, maxWorldY = Integer.MIN_VALUE;
        for (var cell : schematic.cells()) {
            int worldY = translation.getY() + offset[1] + cell.y();
            minWorldY = Math.min(minWorldY, worldY);
            maxWorldY = Math.max(maxWorldY, worldY);
        }
        if (!schematic.cells().isEmpty()
            && (minWorldY < level.getMinBuildHeight() || maxWorldY > level.getMaxBuildHeight() - 1)) {
            source.sendFailure(Component.literal("src2mc: " + mapId + " needs y=" + minWorldY + ".." + maxWorldY
                + " from this position, but this world's build height is only " + level.getMinBuildHeight() + ".."
                + level.getMaxBuildHeight() + ". Placement would silently drop blocks/props outside that range. "
                + "Install a taller dimension_type (custom height datapack) for this world before placing, "
                + "or place from a lower anchor position."));
            return 0;
        }

        dev.theredja.src2mc.world.WorldPlacer.enqueue(level, source, mapId, translation, schematic);
        source.sendSuccess(() -> Component.literal("src2mc: placing " + mapId + " ("
            + schematic.cells().size() + " cells) at " + anchorWorld.toShortString()), true);
        return 1;
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
