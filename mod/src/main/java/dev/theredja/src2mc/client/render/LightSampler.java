package dev.theredja.src2mc.client.render;

import java.util.Map;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.BlockAndTintGetter;
import net.minecraft.world.level.block.state.BlockState;

/**
 * Samples vanilla packed light for mesh vertices baked into static VBOs. Both
 * renderers upload once and rebuild on invalidation, so light is captured at
 * build time rather than read per frame.
 */
final class LightSampler {
    private LightSampler() {}

    /**
     * Samples at world position (x, y, z) offset half a block along the outward normal
     * (nx, ny, nz), so the read lands one block outside an opaque surface rather than inside it.
     * {@code cache}, keyed by {@link BlockPos#asLong()}, is owned by the caller for the duration
     * of one build; a region or prop section touches at most a few thousand distinct positions
     * and many triangles/faces share one.
     */
    static int sample(BlockAndTintGetter level, double x, double y, double z, float nx, float ny, float nz,
                      Map<Long, Integer> cache) {
        BlockPos pos = BlockPos.containing(x + nx * 0.5, y + ny * 0.5, z + nz * 0.5);
        long key = pos.asLong();
        Integer cached = cache.get(key);
        if (cached != null) return cached;
        int light = compute(level, pos);
        cache.put(key, light);
        return light;
    }

    /** Falls back to the brightest neighbour when the sampled cell is itself solid and dark,
     * which keeps geometry embedded in a solid block (common for props) from going black. */
    private static int compute(BlockAndTintGetter level, BlockPos pos) {
        BlockState state = level.getBlockState(pos);
        int packed = LevelRenderer.getLightColor(level, state, pos);
        if (packed != 0 || !state.isSolidRender(level, pos)) return packed;
        int best = 0;
        for (Direction direction : Direction.values()) {
            int neighbour = LevelRenderer.getLightColor(level, pos.relative(direction));
            if (neighbour > best) best = neighbour;
        }
        return best;
    }
}
