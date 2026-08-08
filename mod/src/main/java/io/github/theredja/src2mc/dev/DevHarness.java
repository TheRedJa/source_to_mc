package io.github.theredja.src2mc.dev;

import io.github.theredja.src2mc.Src2mc;
import io.github.theredja.src2mc.SurfaceBlocks;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.neoforged.neoforge.event.entity.player.PlayerEvent;

/**
 * A wall of surface blocks, for looking at.
 *
 * <p>Nothing here runs unless {@code -Dsrc2mc.dev=true} is set, which the dev
 * client does and a real game never does. It exists so that "a bundle-supplied
 * surface shows up in game" can be checked by running the client rather than by
 * building the same wall by hand every time, and so the texture that appears
 * after {@code F3+T} can be compared against the one that was there before.
 */
public final class DevHarness {
    public static final String PROPERTY = "src2mc.dev";

    /**
     * How wide and tall the wall is, from {@code -Dsrc2mc.wall=<w>x<h>}. The
     * default is wide enough to see a repeat of eight blocks; a big one is for
     * measuring frame cost.
     */
    private static final String SIZE_PROPERTY = "src2mc.wall";

    private static final int WIDTH = size(0, 12);

    private static final int HEIGHT = size(1, 6);

    private static int size(int part, int fallback) {
        String[] parts = System.getProperty(SIZE_PROPERTY, "").split("x");
        try {
            return parts.length == 2 ? Integer.parseInt(parts[part].trim()) : fallback;
        } catch (NumberFormatException e) {
            return fallback;
        }
    }

    private DevHarness() {}

    public static boolean enabled() {
        return Boolean.getBoolean(PROPERTY);
    }

    public static void onPlayerLoggedIn(PlayerEvent.PlayerLoggedInEvent event) {
        if (!enabled() || !(event.getEntity() instanceof ServerPlayer player)) {
            return;
        }
        Level level = player.level();
        BlockPos feet = player.blockPosition();

        // A wall in front of wherever the player is looking, one band per
        // material the bundle describes, so a wrong index is obvious rather than
        // subtle.
        Direction facing = player.getDirection();
        BlockPos origin = feet.relative(facing, 6).relative(facing.getCounterClockWise(), WIDTH / 2);
        int materials = io.github.theredja.src2mc.bundle.MaterialTable.server().size();
        int used = Math.max(1, Math.min(materials, 4));

        Direction across = facing.getClockWise();
        for (int x = 0; x < WIDTH; x++) {
            for (int y = 0; y < HEIGHT; y++) {
                int index = (x * used) / WIDTH;
                BlockPos pos = origin.relative(across, x).above(y);
                level.setBlock(pos, SurfaceBlocks.get(index).defaultBlockState(), Block.UPDATE_ALL);
            }
        }

        Src2mc.LOG.info(
                "dev harness: {}x{} wall of surface blocks at {}, {} of {} materials loaded",
                WIDTH,
                HEIGHT,
                origin,
                used,
                materials);
    }
}
