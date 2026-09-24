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
    /** How long entering a world waits on a load that is somehow still running. */
    private static final long STARTUP_LOAD_TIMEOUT_SECONDS = 120;
    static final dev.theredja.src2mc.bundle.BundleRepository BUNDLES =
        new dev.theredja.src2mc.bundle.BundleRepository(Src2mcConfig::bundleDirectory);

    public Src2mc(IEventBus modBus, ModContainer container) {
        container.registerConfig(ModConfig.Type.COMMON, Src2mcConfig.SPEC);
        dev.theredja.src2mc.world.Src2mcWorldContent.register(modBus);
        modBus.addListener(dev.theredja.src2mc.network.PlacementNetwork::register);
        modBus.addListener((net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent event) -> startLoad());
        NeoForge.EVENT_BUS.addListener(Src2mc::onServerAboutToStart);
        NeoForge.EVENT_BUS.addListener(Src2mcCommands::register);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.world.WorldReconciler::onChunkLoad);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.network.PlacementNetwork::onLogin);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.world.WorldPlacer::onServerTick);
        NeoForge.EVENT_BUS.addListener(dev.theredja.src2mc.world.LightOcclusion::onServerTick);
        // Baked light is keyed by dimension, so one left behind would light the
        // next world's overworld as if this world's maps were still in it.
        NeoForge.EVENT_BUS.addListener((net.neoforged.neoforge.event.server.ServerStoppedEvent event) ->
            dev.theredja.src2mc.world.LightOcclusion.clear());
        LOGGER.info("src2mc initialized; bundles load in the background during startup");
    }

    /**
     * Bundles load by themselves, off-thread, as soon as the config that says
     * where they live has been read. Nothing has to wait for it: every consumer
     * watches the generation sequence and picks the load up when it lands.
     */
    private static void startLoad() {
        BUNDLES.reloadAsync().whenComplete((generation, error) -> {
            if (error != null) {
                LOGGER.error("src2mc: startup bundle load failed; no campaign generation is loaded", error);
                return;
            }
            LOGGER.info("src2mc: loaded generation {} at startup: bundles={}, fingerprint={}",
                generation.sequence(), generation.bundles().size(), generation.fingerprint());
        });
    }

    /**
     * Entering a world before the load has landed would leave every chunk that
     * loads meanwhile unreconciled -- {@code WorldReconciler} only sees a chunk
     * as it loads, once. On a normal start this join has nothing to wait for.
     */
    private static void onServerAboutToStart(net.neoforged.neoforge.event.server.ServerAboutToStartEvent event) {
        var load = BUNDLES.pending();
        if (load == null || load.isDone()) return;
        LOGGER.info("src2mc: waiting for the startup bundle load before the server starts");
        try {
            load.get(STARTUP_LOAD_TIMEOUT_SECONDS, java.util.concurrent.TimeUnit.SECONDS);
        } catch (java.util.concurrent.TimeoutException exception) {
            LOGGER.warn("src2mc: bundle load still running after {}s; starting without it",
                STARTUP_LOAD_TIMEOUT_SECONDS);
        } catch (InterruptedException exception) {
            Thread.currentThread().interrupt();
        } catch (java.util.concurrent.ExecutionException exception) {
            // Already logged where the load failed; the server starts with no generation.
        }
    }

    public static dev.theredja.src2mc.bundle.BundleRepository bundles() { return BUNDLES; }
}
