package io.github.theredja.src2mc.client;

import io.github.theredja.src2mc.Src2mc;
import io.github.theredja.src2mc.bundle.MaterialTable;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ModelEvent;
import net.neoforged.neoforge.client.event.RegisterClientReloadListenersEvent;

/** Client wiring: the geometry loader, and the material table on {@code F3+T}. */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class Src2mcClient {
    private Src2mcClient() {}

    @net.neoforged.bus.api.SubscribeEvent
    public static void onRegisterGeometryLoaders(ModelEvent.RegisterGeometryLoaders event) {
        event.register(SurfaceGeometry.ID, SurfaceGeometry.LOADER);
    }

    @net.neoforged.bus.api.SubscribeEvent
    public static void onRegisterReloadListeners(RegisterClientReloadListenersEvent event) {
        // Runs on F3+T, in the same reload that restitches the atlas, so a
        // changed texture and a changed material table arrive together.
        event.registerReloadListener(MaterialTable.clientReloadListener());
    }
}
