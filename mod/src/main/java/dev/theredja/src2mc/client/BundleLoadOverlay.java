package dev.theredja.src2mc.client;

import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.bundle.BundleLoadProgress;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RegisterGuiLayersEvent;
import net.neoforged.neoforge.client.event.ScreenEvent;
import net.neoforged.neoforge.client.gui.VanillaGuiLayers;
import net.neoforged.neoforge.common.NeoForge;

/**
 * What the background bundle load is doing, drawn wherever the player happens
 * to be while it runs.
 *
 * The load starts during game startup, so most of it happens behind the title
 * screen -- where GUI layers do not draw at all. Hence two registrations: the
 * layer for in-world, and a screen-render hook for every menu.
 */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT, bus = EventBusSubscriber.Bus.MOD)
public final class BundleLoadOverlay {
    private static final ResourceLocation LAYER = ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "bundle_load");
    /** How long a finished load keeps saying so before it gets out of the way. */
    private static final long DONE_DISPLAY_MILLIS = 6_000L;
    private static final int WHITE = 0xFFFFFF;
    private static final int RED = 0xFF5555;
    private static long doneAtMillis;

    private BundleLoadOverlay() {}

    @SubscribeEvent
    public static void register(RegisterGuiLayersEvent event) {
        event.registerAbove(VanillaGuiLayers.DEBUG_OVERLAY, LAYER,
            (graphics, deltaTracker) -> draw(graphics));
        NeoForge.EVENT_BUS.addListener((ScreenEvent.Render.Post screen) -> draw(screen.getGuiGraphics()));
    }

    private static void draw(GuiGraphics graphics) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.options.hideGui) return;
        BundleLoadProgress.Snapshot progress = BundleLoadProgress.snapshot();
        List<Component> lines = new ArrayList<>(2);
        int colour = WHITE;
        switch (progress.state()) {
            case LOADING -> {
                doneAtMillis = 0L;
                lines.add(Component.literal("src2mc: loading bundles "
                    + progress.done() + "/" + progress.total()));
                if (!progress.current().isEmpty()) lines.add(Component.literal("  " + progress.current()));
            }
            case DONE -> {
                if (doneAtMillis == 0L) doneAtMillis = System.currentTimeMillis();
                if (System.currentTimeMillis() - doneAtMillis > DONE_DISPLAY_MILLIS) return;
                lines.add(Component.literal("src2mc: " + progress.message()));
            }
            case FAILED -> {
                colour = RED;
                lines.add(Component.literal("src2mc: bundle load failed"));
                lines.add(Component.literal("  " + progress.message()));
            }
            case IDLE -> { return; }
        }
        int lineHeight = minecraft.font.lineHeight + 2;
        int width = lines.stream().mapToInt(minecraft.font::width).max().orElse(0);
        int x = 4;
        int y = graphics.guiHeight() - 4 - lines.size() * lineHeight;
        graphics.fill(x - 3, y - 3, x + width + 3, y + lines.size() * lineHeight + 1, 0xA0000000);
        for (Component line : lines) {
            graphics.drawString(minecraft.font, line, x, y, colour, true);
            y += lineHeight;
        }
    }
}
