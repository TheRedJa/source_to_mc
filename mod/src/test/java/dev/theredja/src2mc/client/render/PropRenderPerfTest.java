package dev.theredja.src2mc.client.render;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

final class PropRenderPerfTest {
    @Test
    void recordsAccumulateWithinAFrameAndPublishOnTheNextBegin() {
        var perf = new PropRenderPerf();
        perf.beginFrame();
        perf.add(PropRenderPerf.M_DRAW_CALLS, 7);
        perf.add(PropRenderPerf.M_DRAW_CALLS, 3);
        perf.add(PropRenderPerf.M_TRIANGLES, 30);
        assertEquals(10, perf.snapshot().frame().drawCalls());
        assertEquals(30, perf.snapshot().frame().triangles());
        assertEquals(0, perf.snapshot().completedFrames());
        perf.beginFrame();
        assertEquals(0, perf.snapshot().frame().drawCalls());
        assertEquals(10, perf.snapshot().windowAverage().drawCalls());
        assertEquals(10, perf.snapshot().totals().drawCalls());
        assertEquals(1, perf.snapshot().completedFrames());
    }

    @Test
    void windowAverageAveragesFramesInsideTheWindow() {
        var perf = new PropRenderPerf();
        perf.beginFrame();
        perf.add(PropRenderPerf.M_DRAW_CALLS, 4);
        perf.beginFrame();
        perf.add(PropRenderPerf.M_DRAW_CALLS, 10);
        perf.beginFrame();
        assertEquals(7, perf.snapshot().windowAverage().drawCalls());
        assertEquals(14, perf.snapshot().totals().drawCalls());
    }

    @Test
    void windowAverageDropsSamplesBeyondTheWindow() {
        var perf = new PropRenderPerf();
        perf.beginFrame();
        perf.add(PropRenderPerf.M_DRAW_CALLS, 1);
        perf.beginFrame();
        for (int i = 0; i < PropRenderPerf.WINDOW_FRAMES; i++) {
            perf.add(PropRenderPerf.M_DRAW_CALLS, 10);
            perf.beginFrame();
        }
        assertEquals(10, perf.snapshot().windowAverage().drawCalls());
        assertEquals(PropRenderPerf.WINDOW_FRAMES + 1, perf.snapshot().completedFrames());
        assertEquals(1 + 10L * PropRenderPerf.WINDOW_FRAMES, perf.snapshot().totals().drawCalls());
    }

    @Test
    void windowAverageHandlesFullWrapWithoutOldestSamples() {
        var perf = new PropRenderPerf();
        perf.beginFrame();
        for (int i = 0; i < PropRenderPerf.WINDOW_FRAMES; i++) {
            perf.add(PropRenderPerf.M_TRIANGLES, 1);
            perf.beginFrame();
        }
        for (int i = 0; i < PropRenderPerf.WINDOW_FRAMES; i++) {
            perf.add(PropRenderPerf.M_TRIANGLES, 3);
            perf.beginFrame();
        }
        assertEquals(3, perf.snapshot().windowAverage().triangles());
    }

    @Test
    void nanosConvertToMillis() {
        var perf = new PropRenderPerf();
        perf.beginFrame();
        perf.add(PropRenderPerf.M_RENDER_NANOS, 1_500_000);
        assertEquals(1.5, perf.snapshot().frame().renderMs(), 1.0e-9);
    }

    @Test
    void resetClearsEverything() {
        var perf = new PropRenderPerf();
        perf.beginFrame();
        perf.add(PropRenderPerf.M_DRAW_CALLS, 5);
        perf.beginFrame();
        perf.reset();
        var snapshot = perf.snapshot();
        assertEquals(0, snapshot.frame().drawCalls());
        assertEquals(0, snapshot.windowAverage().drawCalls());
        assertEquals(0, snapshot.totals().drawCalls());
        assertEquals(0, snapshot.completedFrames());
        perf.beginFrame();
        perf.add(PropRenderPerf.M_DRAW_CALLS, 2);
        perf.beginFrame();
        assertEquals(2, perf.snapshot().windowAverage().drawCalls());
    }
}
