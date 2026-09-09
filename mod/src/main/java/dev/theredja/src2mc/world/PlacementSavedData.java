package dev.theredja.src2mc.world;

import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.saveddata.SavedData;

/** Dimension-scoped authoritative placement persistence. */
public final class PlacementSavedData extends SavedData {
    private static final String FILE = "src2mc_placements";
    private final PlacementIndex index = new PlacementIndex();

    public static PlacementSavedData get(ServerLevel level) {
        return level.getDataStorage().computeIfAbsent(new Factory<>(PlacementSavedData::new, PlacementSavedData::load), FILE);
    }

    private static PlacementSavedData load(CompoundTag tag, HolderLookup.Provider registries) {
        PlacementSavedData data = new PlacementSavedData();
        ListTag entries = tag.getList("placements", Tag.TAG_COMPOUND);
        for (int i = 0; i < entries.size(); i++) {
            CompoundTag entry = entries.getCompound(i);
            MapPlacement placement = new MapPlacement(entry.getString("campaign_id"), entry.getString("map_id"),
                BlockPos.of(entry.getLong("anchor")), BlockPos.of(entry.getLong("translation")),
                BlockPos.of(entry.getLong("world_min")), BlockPos.of(entry.getLong("world_max")));
            if (data.index.register(placement) == PlacementIndex.Registration.OVERLAP) {
                throw new IllegalStateException("overlapping persisted src2mc placements");
            }
        }
        return data;
    }

    public PlacementIndex index() { return index; }
    public PlacementIndex.Registration register(MapPlacement placement) {
        PlacementIndex.Registration result = index.register(placement);
        if (result == PlacementIndex.Registration.ADDED) setDirty();
        return result;
    }

    @Override public CompoundTag save(CompoundTag tag, HolderLookup.Provider registries) {
        ListTag entries = new ListTag();
        for (MapPlacement placement : index.view()) {
            CompoundTag entry = new CompoundTag();
            entry.putString("campaign_id", placement.campaignId()); entry.putString("map_id", placement.mapId());
            entry.putLong("anchor", placement.anchorWorld().asLong()); entry.putLong("translation", placement.translation().asLong());
            entry.putLong("world_min", placement.worldMin().asLong()); entry.putLong("world_max", placement.worldMax().asLong());
            entries.add(entry);
        }
        tag.put("placements", entries);
        return tag;
    }
}
