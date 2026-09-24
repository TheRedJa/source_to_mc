package dev.theredja.src2mc.world;

import dev.theredja.src2mc.Src2mc;
import java.util.ArrayDeque;
import java.util.Deque;
import java.util.LinkedHashSet;
import java.util.Queue;
import java.util.Set;
import java.util.concurrent.ConcurrentLinkedQueue;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

/**
 * Places a parsed schematic directly, in place of a WorldEdit paste. WorldEdit's
 * bulk block-array write is reliable; its per-block tile-entity restore is not,
 * under memory/GC pressure on a long paste (observed: 13/1059 prop roots
 * surviving a 10+ minute paste). Writing blocks and their {@link Src2mcDataBlockEntity}
 * payloads together, the same way {@link WorldReconciler#replace} always has,
 * avoids that second, fragile path entirely.
 */
public final class WorldPlacer {
    private static final long BUDGET_NANOS = 4_000_000L;
    private static final Queue<Job> JOBS = new ConcurrentLinkedQueue<>();

    private WorldPlacer() {
    }

    public static void enqueue(ServerLevel level, CommandSourceStack source, String mapId,
                                BlockPos translation, SchematicReader.Result schematic) {
        Deque<Write> writes = new ArrayDeque<>(schematic.cells().size());
        for (SchematicReader.Cell cell : schematic.cells()) {
            int[] offset = schematic.offset();
            BlockPos world = translation.offset(offset[0] + cell.x(), offset[1] + cell.y(), offset[2] + cell.z());
            writes.add(new Write(world, resolve(cell.blockName()), cell.payload()));
        }
        JOBS.add(new Job(level, source, mapId, writes, writes.size()));
    }

    private static Block resolve(String blockName) {
        return switch (blockName) {
            case SchematicReader.SURFACE -> Src2mcWorldContent.SURFACE.get();
            case SchematicReader.MAP_ANCHOR -> Src2mcWorldContent.MAP_ANCHOR.get();
            case SchematicReader.PROP_ROOT -> Src2mcWorldContent.PROP_ROOT.get();
            default -> throw new IllegalStateException("schematic reader let through unsupported block " + blockName);
        };
    }

    public static void onServerTick(ServerTickEvent.Post event) {
        Job job = JOBS.peek();
        if (job == null) return;
        job.drain();
        if (job.isDone()) {
            JOBS.poll();
            job.finish();
        }
    }

    private record Write(BlockPos pos, Block block, CompoundTag payload) {
    }

    private static final class Job {
        private final ServerLevel level;
        private final CommandSourceStack source;
        private final String mapId;
        private final Deque<Write> writes;
        private final int total;
        private final Set<ChunkPos> touchedChunks = new LinkedHashSet<>();
        private int placed;
        private int outsideBuildHeight;

        Job(ServerLevel level, CommandSourceStack source, String mapId, Deque<Write> writes, int total) {
            this.level = level;
            this.source = source;
            this.mapId = mapId;
            this.writes = writes;
            this.total = total;
        }

        void drain() {
            long started = System.nanoTime();
            while (!writes.isEmpty()) {
                if (placed > 0 && System.nanoTime() - started >= BUDGET_NANOS) break;
                apply(writes.poll());
                placed++;
            }
        }

        boolean isDone() {
            return writes.isEmpty();
        }

        private void apply(Write write) {
            if (level.isOutsideBuildHeight(write.pos())) {
                outsideBuildHeight++;
                return;
            }
            touchedChunks.add(new ChunkPos(write.pos()));
            BlockState state = write.block().defaultBlockState();
            level.setBlock(write.pos(), state, Block.UPDATE_CLIENTS);
            if (write.payload() != null && level.getBlockEntity(write.pos()) instanceof Src2mcDataBlockEntity data) {
                data.replacePayload(write.payload());
            }
        }

        void finish() {
            int anchors = 0, healed = 0, failures = 0;
            for (ChunkPos pos : touchedChunks) {
                var chunk = level.getChunk(pos.x, pos.z);
                var counts = WorldReconciler.reconcileChunk(level, chunk);
                anchors += counts.anchors();
                healed += counts.healed();
                failures += counts.failures();
            }
            int finalAnchors = anchors, finalHealed = healed, finalFailures = failures, finalPlaced = placed, finalTotal = total;
            int finalOutside = outsideBuildHeight;
            String finalMapId = mapId;
            Src2mc.LOGGER.info("src2mc: placed {} blocks for map {}; anchors={}, healed={}, diagnostics={}, outsideBuildHeight={}",
                finalPlaced, finalMapId, finalAnchors, finalHealed, finalFailures, finalOutside);
            source.sendSuccess(() -> Component.literal("src2mc: placed " + finalMapId + "; blocks=" + finalPlaced
                + "/" + finalTotal + ", anchors=" + finalAnchors + ", healed=" + finalHealed
                + ", diagnostics=" + finalFailures), true);
            if (finalOutside > 0) {
                source.sendFailure(Component.literal("src2mc: " + finalOutside + " block(s) of " + finalMapId
                    + " fall outside this world's build height (" + level.getMinBuildHeight() + ".."
                    + level.getMaxBuildHeight() + ") and were NOT placed — props there will read as MISSING. "
                    + "This world needs a taller dimension_type (custom height datapack) to fit this map."));
            }
        }
    }
}
