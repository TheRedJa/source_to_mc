package dev.theredja.src2mc.network;

import dev.theredja.src2mc.world.PlacementIndex;
import dev.theredja.src2mc.world.PlacementSavedData;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.event.entity.player.PlayerEvent;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;

public final class PlacementNetwork {
    private static final Map<ResourceLocation, PlacementIndex> CLIENT = new ConcurrentHashMap<>();
    private PlacementNetwork() {}

    public static void register(RegisterPayloadHandlersEvent event) {
        event.registrar("1").playToClient(PlacementSyncPayload.TYPE, PlacementSyncPayload.STREAM_CODEC,
            (payload, context) -> {
                PlacementIndex index = new PlacementIndex();
                payload.placements().forEach(index::register);
                CLIENT.put(payload.dimension(), index);
            });
    }

    public static PlacementIndex clientIndex(ResourceLocation dimension) { return CLIENT.getOrDefault(dimension, new PlacementIndex()); }

    public static void onLogin(PlayerEvent.PlayerLoggedInEvent event) {
        if (event.getEntity() instanceof ServerPlayer player) send(player);
    }

    public static void send(ServerPlayer player) {
        ServerLevel level = player.serverLevel();
        PacketDistributor.sendToPlayer(player, payload(level));
    }

    public static void broadcast(ServerLevel level) {
        PacketDistributor.sendToPlayersInDimension(level, payload(level));
    }

    private static PlacementSyncPayload payload(ServerLevel level) {
        return new PlacementSyncPayload(level.dimension().location(), PlacementSavedData.get(level).index().view());
    }
}
