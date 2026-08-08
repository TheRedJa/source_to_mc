package io.github.theredja.src2mc.dev;

import io.github.theredja.src2mc.Src2mc;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientTickEvent;

/**
 * Proves the reload claim without a pair of hands on the keyboard.
 *
 * <p>With {@code -Dsrc2mc.autoshot=true} the dev client takes a screenshot, then
 * reloads resources exactly as {@code F3+T} does — {@code F3+T} is bound to
 * {@link Minecraft#reloadResourcePacks()} and nothing else — then takes another.
 * Change a texture in the bundle between the two and the pair of images is the
 * evidence.
 *
 * <p>Off unless the property is set.
 */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class DevScreenshots {
    public static final String PROPERTY = "src2mc.autoshot";

    private static final int SECOND = 20;
    private static final int BEFORE = 12 * SECOND;
    private static final int RELOAD = 25 * SECOND;
    private static final int AFTER = 33 * SECOND;

    private static int ticks;

    private DevScreenshots() {}

    @SubscribeEvent
    public static void onClientTick(ClientTickEvent.Post event) {
        if (!Boolean.getBoolean(PROPERTY)) {
            return;
        }
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null || minecraft.player == null) {
            return;
        }

        ticks++;
        if (ticks % SECOND == 0) {
            // The frame rate, once a second, so "no worse than the baked-block
            // output" can be a number rather than an impression.
            Src2mc.LOG.info("dev harness: {}s, {} fps", ticks / SECOND, minecraft.getFps());
        }
        switch (ticks) {
            case BEFORE -> grab(minecraft, "src2mc-before.png");
            case RELOAD -> {
                Src2mc.LOG.info("dev harness: reloading resources, the F3+T path");
                minecraft.reloadResourcePacks();
            }
            case AFTER -> grab(minecraft, "src2mc-after.png");
            default -> {}
        }
    }

    private static void grab(Minecraft minecraft, String name) {
        Screenshot.grab(
                minecraft.gameDirectory,
                name,
                minecraft.getMainRenderTarget(),
                message -> Src2mc.LOG.info("dev harness: {}", message.getString()));
    }
}
