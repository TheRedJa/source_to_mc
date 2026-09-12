package dev.theredja.src2mc.client.render;

import static org.junit.jupiter.api.Assertions.assertEquals;

import net.minecraft.client.renderer.LightTexture;
import org.junit.jupiter.api.Test;

final class LightSamplerTest {
    /** Eight corners at one value each, indexed the way {@link LightSampler#blend} expects. */
    private static int[] corners(int... sky) {
        int[] packed = new int[8];
        for (int i = 0; i < 8; i++) packed[i] = sky[i] == LightSampler.SOLID ? LightSampler.SOLID : LightTexture.pack(0, sky[i]);
        return packed;
    }

    /** A point at a cell centre must keep that cell's value, so nothing moves where the old
     * per-face value was already the right one. */
    @Test
    void aPointAtACellCentreTakesThatCell() {
        int[] cells = corners(15, 0, 0, 0, 0, 0, 0, 0);

        assertEquals(15, LightTexture.sky(LightSampler.blend(0, 0, 0, cells)));
    }

    /** Halfway between a bright and a dark cell is the average of the two. */
    @Test
    void halfwayBetweenTwoCellsIsTheirAverage() {
        int[] cells = corners(15, 15, 15, 15, 7, 7, 7, 7);

        assertEquals(11, LightTexture.sky(LightSampler.blend(0.5, 0, 0, cells)));
    }

    /** The two channels must not bleed into each other. */
    @Test
    void skyAndBlockAreAveragedSeparately() {
        int[] cells = new int[8];
        for (int i = 0; i < 8; i++) cells[i] = i < 4 ? LightTexture.pack(12, 0) : LightTexture.pack(0, 8);

        int blended = LightSampler.blend(0.5, 0, 0, cells);

        assertEquals(6, LightTexture.block(blended));
        assertEquals(4, LightTexture.sky(blended));
    }

    /** A vertex flat against a wall must take the air beside it, not the wall's own darkness. */
    @Test
    void solidCornersAreLeftOutOfTheAverage() {
        int[] cells = corners(LightSampler.SOLID, LightSampler.SOLID, LightSampler.SOLID, LightSampler.SOLID,
            9, 9, 9, 9);

        assertEquals(9, LightTexture.sky(LightSampler.blend(0.5, 0.5, 0.5, cells)));
    }

    /** Nothing to average means the caller has to fall back to its flat sample. */
    @Test
    void anAllSolidNeighbourhoodReportsItself() {
        int[] cells = corners(LightSampler.SOLID, LightSampler.SOLID, LightSampler.SOLID, LightSampler.SOLID,
            LightSampler.SOLID, LightSampler.SOLID, LightSampler.SOLID, LightSampler.SOLID);

        assertEquals(LightSampler.SOLID, LightSampler.blend(0.5, 0.5, 0.5, cells));
    }
}
