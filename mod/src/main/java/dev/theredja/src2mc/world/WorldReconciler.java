package dev.theredja.src2mc.world;

import dev.theredja.src2mc.Src2mc;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.chunk.LevelChunk;
import net.neoforged.neoforge.event.level.ChunkEvent;

/** Lazy server-side recovery and conservative anchorless diagnostics. */
public final class WorldReconciler {
    private static final Set<String> WARNED = ConcurrentHashMap.newKeySet();
    private WorldReconciler() {}

    public static void onChunkLoad(ChunkEvent.Load event) {
        if (!(event.getLevel() instanceof ServerLevel level) || !(event.getChunk() instanceof LevelChunk chunk)) return;
        level.getServer().execute(() -> reconcileChunk(level, chunk, false));
    }

    public static Counts reconcileChunk(ServerLevel level, LevelChunk chunk) {
        return reconcileChunk(level, chunk, true);
    }

    private static Counts reconcileChunk(ServerLevel level, LevelChunk chunk, boolean diagnoseWithoutPlacement) {
        if (Src2mc.bundles().active().sequence() == 0 || !level.hasChunk(chunk.getPos().x, chunk.getPos().z)) return new Counts(0, 0, 0);
        int anchors = 0, healed = 0, failures = 0;
        for (BlockEntity entity : java.util.List.copyOf(chunk.getBlockEntities().values())) {
            if (!(entity instanceof Src2mcDataBlockEntity data)) continue;
            Block block = entity.getBlockState().getBlock();
            CompoundTag payload = data.payload();
            boolean anchorPayload = validAnchorPayload(payload);
            if (block == Src2mcWorldContent.MAP_ANCHOR.get() || (block == Src2mcWorldContent.PLACEHOLDER.get() && anchorPayload)) {
                if (!anchorPayload) { failures++; warn(level, chunk, "invalid anchor NBT at " + entity.getBlockPos()); continue; }
                var map = Src2mc.bundles().active().findMap(payload.getString("campaign_id"), payload.getString("map_id"));
                if (map.isEmpty() || !java.util.Arrays.equals(payload.getIntArray("anchor_cell"), map.get().anchorCell())) {
                    failures++; replace(level, entity.getBlockPos(), Src2mcWorldContent.PLACEHOLDER.get(), payload);
                    warn(level, chunk, "missing or mismatched bundle map for anchor at " + entity.getBlockPos());
                    continue;
                }
                MapPlacement placement = MapPlacement.fromAnchor(payload.getString("campaign_id"), map.get(), entity.getBlockPos());
                var result = PlacementSavedData.get(level).register(placement);
                if (result == PlacementIndex.Registration.OVERLAP) {
                    failures++; warn(level, chunk, "overlapping map placement rejected at " + entity.getBlockPos()); continue;
                }
                anchors++;
                if (result == PlacementIndex.Registration.ADDED) dev.theredja.src2mc.network.PlacementNetwork.broadcast(level);
                if (block == Src2mcWorldContent.PLACEHOLDER.get()) { replace(level, entity.getBlockPos(), Src2mcWorldContent.MAP_ANCHOR.get(), payload); healed++; }
            } else if (block == Src2mcWorldContent.PROP_ROOT.get()
                || (block == Src2mcWorldContent.PLACEHOLDER.get() && payload.contains("stable_id", Tag.TAG_STRING))) {
                if (!validPropPayload(payload)) { failures++; warn(level, chunk, "invalid prop-root NBT at " + entity.getBlockPos()); continue; }
                var map = Src2mc.bundles().active().findMap(payload.getString("campaign_id"), payload.getString("map_id"));
                boolean available = map.isPresent() && map.get().modelContentIds().contains(payload.getString("model_content_id"));
                if (!available) {
                    failures++; replace(level, entity.getBlockPos(), Src2mcWorldContent.PLACEHOLDER.get(), payload);
                    warn(level, chunk, "missing bundle model for prop root at " + entity.getBlockPos());
                } else if (block == Src2mcWorldContent.PLACEHOLDER.get()) {
                    replace(level, entity.getBlockPos(), Src2mcWorldContent.PROP_ROOT.get(), payload); healed++;
                }
            }
        }
        boolean orphanSurface = false;
        BlockPos.MutableBlockPos cursor = new BlockPos.MutableBlockPos();
        outer: for (int sectionIndex = 0; sectionIndex < chunk.getSections().length; sectionIndex++) {
            var section = chunk.getSections()[sectionIndex];
            if (!section.maybeHas(state -> state.is(Src2mcWorldContent.SURFACE.get()))) continue;
            int sectionMinY = net.minecraft.core.SectionPos.sectionToBlockCoord(chunk.getSectionYFromSectionIndex(sectionIndex));
            for (int y = sectionMinY; y < sectionMinY + 16; y++)
                for (int z = chunk.getPos().getMinBlockZ(); z <= chunk.getPos().getMaxBlockZ(); z++)
                    for (int x = chunk.getPos().getMinBlockX(); x <= chunk.getPos().getMaxBlockX(); x++) {
                    cursor.set(x, y, z);
                    if (chunk.getBlockState(cursor).is(Src2mcWorldContent.SURFACE.get())
                        && PlacementSavedData.get(level).index().at(cursor).isEmpty()) { orphanSurface = true; break outer; }
                }
        }
        // During a large synchronous WorldEdit paste, surface chunks can load
        // before the anchor chunk. With no known placement yet, automatic
        // diagnosis would falsely label every early chunk as anchorless. The
        // manual command remains the authoritative zero-placement check.
        if (orphanSurface && (diagnoseWithoutPlacement || !PlacementSavedData.get(level).index().view().isEmpty())) {
            failures++;
            warnOnce(level, "anchorless_surface", "anchorless src2mc surface data exists outside every known placement"
                + " (first observed in chunk " + chunk.getPos() + ")");
        }
        return new Counts(anchors, healed, failures);
    }

    private static boolean validAnchorPayload(CompoundTag tag) {
        return tag.getInt("schema_version") == 1 && tag.contains("campaign_id", Tag.TAG_STRING)
            && !tag.getString("campaign_id").isBlank() && tag.contains("map_id", Tag.TAG_STRING)
            && !tag.getString("map_id").isBlank() && tag.contains("anchor_cell", Tag.TAG_INT_ARRAY)
            && tag.getIntArray("anchor_cell").length == 3;
    }

    private static boolean validPropPayload(CompoundTag tag) {
        if (tag.getInt("schema_version") != 1 || tag.getString("campaign_id").isBlank() || tag.getString("map_id").isBlank()
            || !tag.getString("stable_id").matches("[0-9a-f]{64}") || !tag.getString("model_content_id").matches("[0-9a-f]{64}")
            || tag.getIntArray("root_cell").length != 3 || !tag.contains("translation", Tag.TAG_LIST)
            || !tag.contains("rotation", Tag.TAG_LIST) || !tag.contains("scale", Tag.TAG_DOUBLE)) return false;
        var translation = tag.getList("translation", Tag.TAG_DOUBLE);
        var rotation = tag.getList("rotation", Tag.TAG_DOUBLE);
        if (translation.size() != 3 || rotation.size() != 4 || !Double.isFinite(tag.getDouble("scale")) || tag.getDouble("scale") <= 0) return false;
        double length = 0;
        for (int i = 0; i < translation.size(); i++) if (!Double.isFinite(translation.getDouble(i))) return false;
        for (int i = 0; i < rotation.size(); i++) { double value = rotation.getDouble(i); if (!Double.isFinite(value)) return false; length += value * value; }
        return Math.abs(length - 1.0) <= 1.0e-9;
    }

    private static void replace(ServerLevel level, BlockPos position, Block target, CompoundTag payload) {
        if (level.getBlockState(position).is(target)) return;
        level.setBlock(position, target.defaultBlockState(), Block.UPDATE_ALL);
        if (level.getBlockEntity(position) instanceof Src2mcDataBlockEntity replacement) replacement.replacePayload(payload);
    }

    private static void warn(ServerLevel level, LevelChunk chunk, String message) {
        String key = level.dimension().location() + ":" + chunk.getPos().toLong() + ":" + message;
        if (WARNED.add(key)) Src2mc.LOGGER.warn("src2mc [{} {}]: {}", level.dimension().location(), chunk.getPos(), message);
    }

    private static void warnOnce(ServerLevel level, String category, String message) {
        String key = level.dimension().location() + ":" + category;
        if (WARNED.add(key)) Src2mc.LOGGER.warn("src2mc [{}]: {}", level.dimension().location(), message);
    }

    public record Counts(int anchors, int healed, int failures) {}
}
