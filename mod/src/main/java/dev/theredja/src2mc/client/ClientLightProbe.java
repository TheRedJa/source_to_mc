package dev.theredja.src2mc.client;

import static net.minecraft.commands.Commands.literal;

import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.network.PlacementNetwork;
import dev.theredja.src2mc.world.LightOcclusion;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.world.level.LightLayer;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RegisterClientCommandsEvent;

/**
 * Reports the client's own sky-light bake.
 *
 * The existing probes ({@code /src2mc lightmask}, {@code /src2mc lightcolumn}) run on the server
 * and report the server's bake, which is not what gets drawn: the client bakes its own copy from
 * the same bundle when the placement payload arrives, and that copy is what the client light
 * engine — and therefore every mesh built from it — reads. The two can disagree, and nothing said
 * so before this command.
 */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class ClientLightProbe {
    private ClientLightProbe() {}

    @SubscribeEvent
    public static void registerCommand(RegisterClientCommandsEvent event) {
        event.getDispatcher().register(literal("src2mc_client_light").executes(context -> report(context.getSource())));
    }

    private static int report(net.minecraft.commands.CommandSourceStack source) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null) {
            source.sendFailure(Component.literal("No client level"));
            return 0;
        }
        var level = minecraft.level;
        BlockPos at = BlockPos.containing(minecraft.gameRenderer.getMainCamera().getPosition());
        var baked = LightOcclusion.baked(level);
        int here = LightOcclusion.skyAt(level, at.getX(), at.getY(), at.getZ());

        StringBuilder column = new StringBuilder();
        for (int y = at.getY() + 10; y >= at.getY() - 2; y--) {
            int sky = LightOcclusion.skyAt(level, at.getX(), y, at.getZ());
            column.append(sky < 0 ? '?' : Character.forDigit(sky, 16));
        }
        source.sendSuccess(() -> Component.literal("src2mc client light: bake "
            + (LightOcclusion.enabled() ? "on" : "off")
            + ", epoch=" + LightOcclusion.clientEpoch()
            + " from generation " + LightOcclusion.clientGeneration()
            + " (active generation " + Src2mc.bundles().active().sequence() + ")"
            + ", shaded=" + baked.darkCells() + " open=" + baked.litCells()
            + " sections=" + baked.sections().size() + " chunks=" + baked.chunks().size()
            + " in " + baked.millis() + "ms"
            + "; placements=" + PlacementNetwork.clientIndex(level.dimension().location()).size()
            + "; at " + at.toShortString()
            + " covered=" + baked.covers(at.getX() >> 4, at.getZ() >> 4)
            + " baked=" + (here < 0 ? "none" : here)
            + " engine sky=" + level.getBrightness(LightLayer.SKY, at)
            + " block=" + level.getBrightness(LightLayer.BLOCK, at)
            + "; column +10..-2 " + column), false);
        return 1;
    }
}
