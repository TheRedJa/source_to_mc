package dev.theredja.src2mc;

import com.mojang.logging.LogUtils;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.config.ModConfig;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.common.NeoForge;
import org.slf4j.Logger;

/**
 * Common entry point for src2mc.
 */
@Mod(Src2mc.MOD_ID)
public final class Src2mc {
    public static final String MOD_ID = "src2mc";
    public static final Logger LOGGER = LogUtils.getLogger();
    static final dev.theredja.src2mc.bundle.BundleRepository BUNDLES =
        new dev.theredja.src2mc.bundle.BundleRepository(Src2mcConfig::bundleDirectory);

    public Src2mc(IEventBus modBus, ModContainer container) {
        container.registerConfig(ModConfig.Type.COMMON, Src2mcConfig.SPEC);
        dev.theredja.src2mc.world.Src2mcWorldContent.register(modBus);
        modBus.addListener(dev.theredja.src2mc.network.PlacementNetwork::register);
        NeoForge.EVENT_BUS.addListener(Src2mcCommands::register);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.world.WorldReconciler::onChunkLoad);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.network.PlacementNetwork::onLogin);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.world.WorldPlacer::onServerTick);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.world.LightOcclusion::onServerTick);
        // Baked light is keyed by dimension, so one left behind would light the
        // next world's overworld as if this world's maps were still in it.
        NeoForge.EVENT_BUS.addListener((net.neoforged.neoforge.event.server.ServerStoppedEvent event) ->
            dev.theredja.src2mc.world.LightOcclusion.clear());
        LOGGER.info("src2mc initialized; no campaign generation is loaded");
    }

    public static dev.theredja.src2mc.bundle.BundleRepository bundles() { return BUNDLES; }
}
