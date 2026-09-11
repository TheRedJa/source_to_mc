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
        LOGGER.info("src2mc initialized; no campaign generation is loaded");
    }

    public static dev.theredja.src2mc.bundle.BundleRepository bundles() { return BUNDLES; }
}
