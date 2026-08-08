package io.github.theredja.src2mc;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import io.github.theredja.src2mc.client.SurfaceUv;
import java.util.List;
import net.minecraft.core.Direction;
import org.junit.jupiter.api.Test;

/**
 * The arithmetic of {@code docs/format.md} §4: texture repetition is a
 * coordinate, not another texture and not another block.
 */
class SurfaceUvTest {
    private static final float EPSILON = 1.0E-4F;

    /**
     * The claim the format is built on. A texture covering four blocks shows a
     * quarter of itself on each, and four blocks in a row show the whole thing
     * once — one texture, four blocks, no tiles.
     */
    @Test
    void aFourBlockTextureIsOneTextureAcrossFourBlocks() {
        for (int block = 0; block < 4; block++) {
            List<SurfaceUv.Piece> pieces = SurfaceUv.pieces(Direction.NORTH, block, 0, 4.0F, 4.0F);
            assertEquals(1, pieces.size(), "a whole-number scale needs no cutting at block " + block);
            SurfaceUv.Piece piece = pieces.get(0);
            assertEquals(block * 0.25F, piece.su0(), EPSILON, "block " + block + " starts a quarter later");
            assertEquals((block + 1) * 0.25F, piece.su1(), EPSILON);
        }
    }

    /** Neighbouring blocks continue the same repeat rather than each restarting it. */
    @Test
    void neighbouringBlocksContinueTheRepeat() {
        SurfaceUv.Piece left = SurfaceUv.pieces(Direction.NORTH, 7, 0, 8.0F, 8.0F).get(0);
        SurfaceUv.Piece right = SurfaceUv.pieces(Direction.NORTH, 8, 0, 8.0F, 8.0F).get(0);
        assertEquals(1.0F, left.su1(), EPSILON, "block 7 ends at the end of the repeat");
        assertEquals(0.0F, right.su0(), EPSILON, "block 8 starts the next one");
    }

    /**
     * A repeat boundary inside a block face is more quads, never more blocks and
     * never more textures.
     */
    @Test
    void aBoundaryInsideAFaceCutsTheQuadNotTheBlock() {
        // Two thirds of a block per repeat: the face crosses a boundary.
        List<SurfaceUv.Piece> pieces = SurfaceUv.pieces(Direction.NORTH, 0, 0, 2.0F / 3.0F, 1.0F);
        assertTrue(pieces.size() > 1, "expected the face to be cut, got " + pieces.size() + " piece");

        float covered = 0.0F;
        for (SurfaceUv.Piece piece : pieces) {
            covered += piece.uMax() - piece.uMin();
            assertWithinSprite(piece);
        }
        assertEquals(1.0F, covered, EPSILON, "the pieces still cover exactly one block");
    }

    /** Even a nonsense scale cannot make one block face into unbounded geometry. */
    @Test
    void repeatsPerBlockAreCapped() {
        List<SurfaceUv.Piece> pieces = SurfaceUv.pieces(Direction.NORTH, 0, 0, 1.0F / 4096.0F, 1.0F);
        assertTrue(
                pieces.size() <= SurfaceUv.MAX_REPEATS_PER_BLOCK + 1,
                "expected at most " + SurfaceUv.MAX_REPEATS_PER_BLOCK + " repeats, got " + pieces.size());
    }

    /** Texture v runs down the screen, so on a wall it has to run down the world. */
    @Test
    void wallTexturesAreNotUpsideDown() {
        SurfaceUv.Piece low = SurfaceUv.pieces(Direction.NORTH, 0, 0, 1.0F, 1.0F).get(0);
        assertTrue(
                low.sv0() > low.sv1(),
                "sprite v must decrease as the world goes up, got " + low.sv0() + " to " + low.sv1());
    }

    /** Below y=0 is ordinary world, not a special case. */
    @Test
    void negativeCoordinatesRepeatLikeAnyOther() {
        SurfaceUv.Piece piece = SurfaceUv.pieces(Direction.NORTH, -1, 0, 4.0F, 4.0F).get(0);
        assertWithinSprite(piece);
        assertEquals(0.75F, piece.su0(), EPSILON, "block -1 is the last quarter of the repeat below");
        assertEquals(1.0F, piece.su1(), EPSILON);
    }

    @Test
    void everyFaceHasAxesAndTheyAreNotItsOwn() {
        for (Direction face : Direction.values()) {
            assertTrue(SurfaceUv.uAxis(face) != face.getAxis(), face + " must not read its own axis for u");
            assertTrue(SurfaceUv.vAxis(face) != face.getAxis(), face + " must not read its own axis for v");
            assertTrue(SurfaceUv.uAxis(face) != SurfaceUv.vAxis(face), face + " needs two different axes");
        }
    }

    private static void assertWithinSprite(SurfaceUv.Piece piece) {
        for (float coordinate : new float[] {piece.su0(), piece.su1(), piece.sv0(), piece.sv1()}) {
            assertTrue(
                    coordinate >= -EPSILON && coordinate <= 1.0F + EPSILON,
                    "sprite coordinate " + coordinate + " leaves its own texture, which would sample a neighbour"
                            + " on the atlas");
        }
    }
}
