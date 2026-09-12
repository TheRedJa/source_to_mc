package dev.theredja.src2mc.client.render;

import java.util.Map;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.util.Mth;
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
        return cached(level, BlockPos.containing(x + nx * 0.5, y + ny * 0.5, z + nz * 0.5), cache);
    }

    /**
     * Smooth light for one vertex: the eight cells around the sampled point,
     * weighed by how far the point sits into each.
     *
     * One value per face is vanilla's flat lighting, and it steps in whole
     * blocks, which is what our geometry looked like. Interpolating between
     * vertices is how vanilla's own smooth lighting works, so this changes
     * nothing about the vertex format or the GPU's work -- only which number
     * each vertex carries.
     *
     * Cells that render solid are dropped from the average and the remaining
     * weights renormalized: a wall's own blocks are unlit inside and would drag
     * every vertex on its face toward black. With nothing but solid cells
     * around the point there is nothing to average, and the flat sample -- which
     * has its own brightest-neighbour fallback -- is the better answer.
     */
    static int smooth(BlockAndTintGetter level, double x, double y, double z, float nx, float ny, float nz,
                      Map<Long, Integer> cache) {
        double px = x + nx * 0.5, py = y + ny * 0.5, pz = z + nz * 0.5;
        // Cell centres sit at +0.5, so the lower cell of each pair is found by
        // stepping back half a block from the sampled point.
        int x0 = Mth.floor(px - 0.5), y0 = Mth.floor(py - 0.5), z0 = Mth.floor(pz - 0.5);
        double fx = px - 0.5 - x0, fy = py - 0.5 - y0, fz = pz - 0.5 - z0;
        int[] corners = new int[8];
        BlockPos.MutableBlockPos cursor = new BlockPos.MutableBlockPos();
        for (int dx = 0; dx < 2; dx++) {
            for (int dy = 0; dy < 2; dy++) {
                for (int dz = 0; dz < 2; dz++) {
                    cursor.set(x0 + dx, y0 + dy, z0 + dz);
                    corners[dx << 2 | dy << 1 | dz] = level.getBlockState(cursor).isSolidRender(level, cursor)
                        ? SOLID : cached(level, cursor, cache);
                }
            }
        }
        int blended = blend(fx, fy, fz, corners);
        return blended == SOLID ? sample(level, x, y, z, nx, ny, nz, cache) : blended;
    }

    /** A corner that contributes nothing, and the answer when none of them does. */
    static final int SOLID = -1;

    /**
     * Trilinear blend of eight packed light values, indexed {@code dx<<2|dy<<1|dz}, with
     * {@link #SOLID} corners left out and the remaining weights renormalized. Sky and block
     * are separate channels and are averaged separately; blending the packed integers would
     * mix one into the other.
     */
    static int blend(double fx, double fy, double fz, int[] corners) {
        double sky = 0, block = 0, total = 0;
        for (int corner = 0; corner < 8; corner++) {
            if (corners[corner] == SOLID) continue;
            double weight = ((corner & 4) == 0 ? 1 - fx : fx)
                * ((corner & 2) == 0 ? 1 - fy : fy)
                * ((corner & 1) == 0 ? 1 - fz : fz);
            if (weight <= 0) continue;
            sky += weight * LightTexture.sky(corners[corner]);
            block += weight * LightTexture.block(corners[corner]);
            total += weight;
        }
        if (total <= 0) return SOLID;
        return LightTexture.pack((int) Math.round(block / total), (int) Math.round(sky / total));
    }

    private static int cached(BlockAndTintGetter level, BlockPos pos, Map<Long, Integer> cache) {
        long key = pos.asLong();
        Integer hit = cache.get(key);
        if (hit != null) return hit;
        int light = compute(level, pos.immutable());
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
