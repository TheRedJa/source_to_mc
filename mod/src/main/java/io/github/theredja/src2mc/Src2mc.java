package io.github.theredja.src2mc;

import io.github.theredja.src2mc.bundle.BundlePackFinder;
import io.github.theredja.src2mc.bundle.MaterialTable;
import io.github.theredja.src2mc.dev.DevHarness;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.AddReloadListenerEvent;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Entry point and the one place the interchange contract is stated in Java.
 *
 * <p>Everything this mod reads is described by {@code docs/format.md} in the
 * repository root. That document is normative: where it and this code disagree,
 * this code has a bug.
 */
@Mod(Src2mc.MOD_ID)
public final class Src2mc {
    /**
     * Also the namespace of every block this mod registers, so a surface block
     * is {@code src2mc:surface_<n>} and a prop block entity is
     * {@code src2mc:prop}.
     */
    public static final String MOD_ID = "src2mc";

    /**
     * Version of the interchange format in {@code docs/format.md}.
     *
     * <p>Must equal {@code src2mc::FORMAT_VERSION} in {@code src/lib.rs}. A test
     * checks that it does, so the two cannot drift silently. Bump both in the
     * same commit as the regenerated fixtures.
     */
    public static final int FORMAT_VERSION = 1;

    /**
     * How many surface blocks the mod registers, {@code src2mc:surface_0} to
     * {@code src2mc:surface_<SURFACE_POOL_SIZE-1>}.
     *
     * <p>A compile-time constant, deliberately: {@code docs/format.md} §2 and §5
     * and decision D6 all say the registry may not depend on what has been
     * converted. Growing this number is a mod update and a restart, and is the
     * one thing a resource reload cannot fix.
     *
     * <p>4096, the number {@code docs/mod-requirements.md} R1 sketches, chosen
     * to be above whatever a campaign needs rather than measured against one:
     * the measured sample map {@code d1_trainstation_02} uses 168 distinct
     * materials, and being wrong here costs a restart.
     */
    public static final int SURFACE_POOL_SIZE = 4096;

    public static final Logger LOG = LoggerFactory.getLogger(MOD_ID);

    public Src2mc(IEventBus modBus) {
        LOG.info("src2mc loaded, reading interchange format version {}", FORMAT_VERSION);

        SurfaceBlocks.register(modBus);
        modBus.addListener(BundlePackFinder::onAddPackFinders);

        // The server needs the material table too: sounds, and later the
        // collision that is derived from the same bundle.
        NeoForge.EVENT_BUS.addListener(Src2mc::onAddReloadListener);
        NeoForge.EVENT_BUS.addListener(Src2mcCommands::register);

        if (DevHarness.enabled()) {
            LOG.info("dev harness on ({})", DevHarness.PROPERTY);
            NeoForge.EVENT_BUS.addListener(DevHarness::onPlayerLoggedIn);
        }
    }

    private static void onAddReloadListener(AddReloadListenerEvent event) {
        event.addListener(MaterialTable.reloadListener());
    }
}
