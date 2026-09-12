package dev.theredja.src2mc.client;

import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.world.LightOcclusion;
import net.minecraft.client.Minecraft;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.event.level.ChunkEvent;

/**
 * Redraws the chunk whose sky light we just replaced.
 *
 * Vanilla rebuilds a section's mesh when its light changes because it is told
 * so by a light update; light handed straight to the engine arrives without
 * one, so the blocks in a placed map would keep the shading they were built
 * with until something else disturbed them. The mod's own surface and prop
 * renderers sample light per frame and need no help.
 */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class ClientLightRefresh {
    private ClientLightRefresh() {}

    /**
     * A chunk arriving from the server carries the light the server had when it
     * was sent, which for a chunk loaded before the bake finished is vanilla's.
     */
    @SubscribeEvent
    static void onChunkLoad(ChunkEvent.Load event) {
        if (!event.getLevel().isClientSide() || !(event.getLevel() instanceof net.minecraft.world.level.Level level)) return;
        LightOcclusion.applyChunk(level, event.getChunk().getPos().x, event.getChunk().getPos().z);
    }

    public static void markChunkDirty(int chunkX, int chunkZ) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null) return;
        for (int sectionY = minecraft.level.getMinSection(); sectionY < minecraft.level.getMaxSection(); sectionY++) {
            minecraft.levelRenderer.setSectionDirtyWithNeighbors(chunkX, sectionY, chunkZ);
        }
    }
}
