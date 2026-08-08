package io.github.theredja.src2mc.dev;

import io.github.theredja.src2mc.Src2mc;
import net.minecraft.client.Minecraft;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientTickEvent;

/**
 * Opens a save on its own, so the dev client can be run without a hand on it.
 *
 * <p>{@code -Dsrc2mc.world=<save>} loads {@code saves/<save>} once the title
 * screen appears, through the same call the world list uses. Off unless the
 * property is set, and it does nothing at all after the first time.
 */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class DevAutoWorld {
    public static final String PROPERTY = "src2mc.world";

    private static boolean opened;

    private static int ticks;

    private DevAutoWorld() {}

    @SubscribeEvent
    public static void onClientTick(ClientTickEvent.Post event) {
        String world = System.getProperty(PROPERTY, "");
        if (opened || world.isBlank()) {
            return;
        }
        Minecraft minecraft = Minecraft.getInstance();
        // Whatever the first screen is — the title, or the accessibility
        // onboarding a fresh game directory shows instead — the world can be
        // opened once the loading overlay is gone and no level is up yet.
        if (minecraft.getOverlay() != null || minecraft.level != null || minecraft.screen == null) {
            return;
        }
        if (++ticks < 40) {
            return;
        }
        opened = true;
        Src2mc.LOG.info("dev harness: opening save '{}'", world);
        minecraft.createWorldOpenFlows()
                .openWorld(world, () -> Src2mc.LOG.error("dev harness: could not open save '{}'", world));
    }
}
