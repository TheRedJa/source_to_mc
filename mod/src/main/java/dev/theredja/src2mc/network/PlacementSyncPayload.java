package dev.theredja.src2mc.network;

import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.world.MapPlacement;
import io.netty.handler.codec.DecoderException;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.BlockPos;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

/** Complete small immutable placement-index snapshot for one dimension. */
public record PlacementSyncPayload(ResourceLocation dimension, List<MapPlacement> placements) implements CustomPacketPayload {
    public static final Type<PlacementSyncPayload> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "placements"));
    private static final int MAX_PLACEMENTS = 4096;
    public static final StreamCodec<RegistryFriendlyByteBuf, PlacementSyncPayload> STREAM_CODEC = new StreamCodec<>() {
        @Override public PlacementSyncPayload decode(RegistryFriendlyByteBuf buffer) {
            ResourceLocation dimension = buffer.readResourceLocation();
            int count = buffer.readVarInt();
            if (count < 0 || count > MAX_PLACEMENTS) throw new DecoderException("invalid src2mc placement count " + count);
            List<MapPlacement> placements = new ArrayList<>(count);
            for (int i = 0; i < count; i++) placements.add(new MapPlacement(buffer.readUtf(128), buffer.readUtf(128),
                BlockPos.of(buffer.readLong()), BlockPos.of(buffer.readLong()), BlockPos.of(buffer.readLong()), BlockPos.of(buffer.readLong())));
            return new PlacementSyncPayload(dimension, placements);
        }
        @Override public void encode(RegistryFriendlyByteBuf buffer, PlacementSyncPayload payload) {
            buffer.writeResourceLocation(payload.dimension()); buffer.writeVarInt(payload.placements().size());
            for (MapPlacement placement : payload.placements()) {
                buffer.writeUtf(placement.campaignId(), 128); buffer.writeUtf(placement.mapId(), 128);
                buffer.writeLong(placement.anchorWorld().asLong()); buffer.writeLong(placement.translation().asLong());
                buffer.writeLong(placement.worldMin().asLong()); buffer.writeLong(placement.worldMax().asLong());
            }
        }
    };
    public PlacementSyncPayload { placements = List.copyOf(placements); }
    @Override public Type<? extends CustomPacketPayload> type() { return TYPE; }
}
