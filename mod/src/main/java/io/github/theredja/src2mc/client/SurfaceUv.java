package io.github.theredja.src2mc.client;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.Direction;

/**
 * Texture coordinates for a surface block face — the arithmetic of
 * {@code docs/format.md} §4 and D10, with no rendering in it.
 *
 * <p>A material's texture covers {@code blocks_per_repeat} blocks. The face of
 * one block therefore shows a sub-rectangle of the texture, chosen by where the
 * block is in the world, and neighbouring blocks continue the same repeat. That
 * is the whole trick: one texture, one block, and the repetition lives in the
 * coordinates.
 *
 * <p>When a repeat boundary falls inside a block face — which needs a texture
 * covering less than a whole block, or a scale that is not a whole number — the
 * face is cut into more <em>quads</em>. Never more textures and never more
 * blocks. Extra triangles are free (D2); extra registry entries are not (D6).
 */
public final class SurfaceUv {
    /**
     * Most repeats one block face is allowed to show per axis.
     *
     * <p>A guard against a bundle asking for a texture repeated a thousand times
     * inside one block, which is a converter bug rather than a map. Sixteen is
     * already far past anything a Source material does.
     */
    public static final int MAX_REPEATS_PER_BLOCK = 16;

    private SurfaceUv() {}

    /**
     * A piece of a face: where it sits in the block, and what part of the texture
     * it shows. Positions are block-local, {@code 0} to {@code 1}. Texture
     * coordinates are sprite-local, {@code 0} to {@code 1}.
     */
    public record Piece(float uMin, float uMax, float vMin, float vMax, float su0, float su1, float sv0, float sv1) {}

    /** Which world axis supplies the face's horizontal texture coordinate. */
    public static Direction.Axis uAxis(Direction face) {
        return switch (face.getAxis()) {
            case X -> Direction.Axis.Z;
            case Y, Z -> Direction.Axis.X;
        };
    }

    /** …and its vertical one. */
    public static Direction.Axis vAxis(Direction face) {
        return face.getAxis() == Direction.Axis.Y ? Direction.Axis.Z : Direction.Axis.Y;
    }

    /**
     * Texture v runs down the screen, and on a wall that has to mean down in the
     * world, so the vertical coordinate of a side face is negated. On a floor or
     * ceiling there is no down to match and the world axis is used as it is.
     */
    public static int vSign(Direction face) {
        return face.getAxis() == Direction.Axis.Y ? 1 : -1;
    }

    /**
     * The pieces one face is cut into.
     *
     * @param face which face of the block
     * @param worldU the block's world coordinate along {@link #uAxis}
     * @param worldV its world coordinate along {@link #vAxis}
     * @param blocksPerRepeatU how many blocks one repeat covers, first axis
     * @param blocksPerRepeatV …and second
     */
    public static List<Piece> pieces(
            Direction face, int worldU, int worldV, float blocksPerRepeatU, float blocksPerRepeatV) {
        List<Cut> alongU = cut(worldU, blocksPerRepeatU, 1);
        List<Cut> alongV = cut(worldV, blocksPerRepeatV, vSign(face));

        List<Piece> pieces = new ArrayList<>(alongU.size() * alongV.size());
        for (Cut u : alongU) {
            for (Cut v : alongV) {
                pieces.add(new Piece(u.min, u.max, v.min, v.max, u.sprite0, u.sprite1, v.sprite0, v.sprite1));
            }
        }
        return pieces;
    }

    /** One piece along one axis. */
    private record Cut(float min, float max, float sprite0, float sprite1) {}

    /**
     * Cuts the block's one-block extent along an axis wherever the texture
     * restarts.
     *
     * <p>The texture coordinate of a point at world position {@code w} is
     * {@code sign * w / blocksPerRepeat}. Every integer of that is a boundary,
     * because past it the texture has started again and the sprite coordinate
     * has to jump back.
     */
    private static List<Cut> cut(int world, float blocksPerRepeat, int sign) {
        float scale = Math.max(blocksPerRepeat, 1.0F / MAX_REPEATS_PER_BLOCK);

        // Texture coordinate as a function of the block-local position l:
        //   t(l) = sign * (world + l) / scale
        // and back again, using sign * sign == 1:
        //   l(t) = sign * scale * t - world
        float t0 = sign * world / scale;
        float t1 = sign * (world + 1) / scale;
        float low = Math.min(t0, t1);
        float high = Math.max(t0, t1);

        List<Float> stops = new ArrayList<>();
        stops.add(0.0F);
        for (int boundary = (int) Math.floor(low) + 1; boundary <= Math.ceil(high) - 1; boundary++) {
            float local = sign * scale * boundary - world;
            if (local > 1.0E-4F && local < 1.0F - 1.0E-4F) {
                stops.add(local);
            }
        }
        stops.add(1.0F);
        stops.sort(Float::compare);

        List<Cut> cuts = new ArrayList<>(stops.size() - 1);
        for (int i = 0; i + 1 < stops.size(); i++) {
            float min = stops.get(i);
            float max = stops.get(i + 1);
            float ta = sign * (world + min) / scale;
            float tb = sign * (world + max) / scale;
            // The repeat this piece is inside. Taken from the middle so that a
            // boundary exactly on the piece's edge cannot pick the wrong one.
            float middle = (ta + tb) * 0.5F;
            float repeat = (float) Math.floor(middle);
            cuts.add(new Cut(min, max, clamp01(ta - repeat), clamp01(tb - repeat)));
        }
        return cuts;
    }

    private static float clamp01(float value) {
        return value < 0.0F ? 0.0F : (value > 1.0F ? 1.0F : value);
    }
}
