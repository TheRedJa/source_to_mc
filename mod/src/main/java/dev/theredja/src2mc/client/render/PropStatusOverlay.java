package dev.theredja.src2mc.client.render;

import dev.theredja.src2mc.Src2mc;
import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RegisterGuiLayersEvent;
import net.neoforged.neoforge.client.gui.VanillaGuiLayers;

/** A deliberately opt-in, cached HUD view of {@link PropRenderer#statusLines()}. */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT, bus = EventBusSubscriber.Bus.MOD)
public final class PropStatusOverlay {
    private static final ResourceLocation LAYER = ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "prop_status");
    private static final int REFRESH_TICKS = 10;
    private static boolean enabled;
    private static long lastRefreshTick = Long.MIN_VALUE;
    private static List<Component> lines = List.of();

    private PropStatusOverlay() {}

    @SubscribeEvent
    public static void register(RegisterGuiLayersEvent event) {
        event.registerAbove(VanillaGuiLayers.DEBUG_OVERLAY, LAYER, PropStatusOverlay::render);
    }

    static boolean toggle() {
        enabled = !enabled;
        lastRefreshTick = Long.MIN_VALUE;
        return enabled;
    }

    private static void render(GuiGraphics graphics, net.minecraft.client.DeltaTracker deltaTracker) {
        Minecraft minecraft = Minecraft.getInstance();
        if (!enabled || minecraft.level == null || minecraft.options.hideGui) return;
        long gameTick = minecraft.level.getGameTime();
        if (lastRefreshTick == Long.MIN_VALUE || gameTick - lastRefreshTick >= REFRESH_TICKS) {
            lines = PropRenderer.statusLines();
            lastRefreshTick = gameTick;
        }
        if (lines.isEmpty()) return;
        int x = 4, y = 4, lineHeight = minecraft.font.lineHeight + 2;
        int width = lines.stream().mapToInt(minecraft.font::width).max().orElse(0);
        graphics.fill(x - 3, y - 3, x + width + 3, y + lines.size() * lineHeight + 1, 0xA0000000);
        for (Component line : lines) {
            graphics.drawString(minecraft.font, line, x, y, 0xFFFFFF, true);
            y += lineHeight;
        }
    }
}
