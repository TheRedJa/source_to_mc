package dev.theredja.src2mc.client.render;

import java.util.Arrays;

/**
 * Render-thread-owned frame profiling counters for the prop renderer. Values
 * accumulate into the current frame; the next {@link #beginFrame()} publishes
 * them into a rolling window, so the status command can report last-frame,
 * window-average, and cumulative numbers without sampling or locking.
 */
final class PropRenderPerf {
    static final int WINDOW_FRAMES = 120;

    static final int M_VISIBLE_AGGREGATES = 0;
    static final int M_FRUSTUM_TESTS = 1;
    static final int M_DRAW_CALLS = 2;
    static final int M_DRAW_CALLS_TRANSLUCENT = 3;
    static final int M_TRIANGLES = 4;
    static final int M_RENDER_NANOS = 5;
    static final int M_BUILD_NANOS = 6;
    static final int M_BUILDS = 7;
    static final int M_REBUILT_AGGREGATES = 8;
    static final int M_EVICTED_PROPS = 9;
    static final int M_EVICTED_AGGREGATES = 10;
    static final int M_ROOT_SCAN_NANOS = 11;
    static final int M_ROOT_SCAN_PROPS = 12;
    static final int M_REBUILD_NANOS = 13;
    static final int M_DIRTY_REMAINING = 14;
    static final int M_STATE_SWITCHES = 15;
    static final int M_PVS_REJECTED = 16;
    private static final int METRICS = 17;

    private final long[] window = new long[WINDOW_FRAMES * METRICS];
    private final long[] current = new long[METRICS];
    private final long[] totals = new long[METRICS];
    private int cursor;
    private long frames;
    private boolean frameActive;

    /** Publishes the accumulated frame into the window and starts an empty one. */
    void beginFrame() {
        if (frameActive) {
            System.arraycopy(current, 0, window, cursor * METRICS, METRICS);
            cursor = (cursor + 1) % WINDOW_FRAMES;
            frames++;
        }
        frameActive = true;
        Arrays.fill(current, 0L);
    }

    void add(int metric, long amount) {
        current[metric] += amount;
        totals[metric] += amount;
    }

    void reset() {
        Arrays.fill(current, 0L);
        Arrays.fill(totals, 0L);
        Arrays.fill(window, 0L);
        cursor = 0;
        frames = 0;
        frameActive = false;
    }

    Snapshot snapshot() {
        long span = Math.min(frames, WINDOW_FRAMES);
        long[] averaged = new long[METRICS];
        for (int f = 0; f < span; f++) {
            int base = f * METRICS;
            for (int m = 0; m < METRICS; m++) averaged[m] += window[base + m];
        }
        if (span > 1) for (int m = 0; m < METRICS; m++) averaged[m] /= span;
        return new Snapshot(frameOf(current), frameOf(averaged), frameOf(totals), frames);
    }

    private Frame frameOf(long[] metrics) {
        return new Frame(metrics[M_VISIBLE_AGGREGATES], metrics[M_FRUSTUM_TESTS],
            metrics[M_DRAW_CALLS], metrics[M_DRAW_CALLS_TRANSLUCENT], metrics[M_TRIANGLES],
            metrics[M_RENDER_NANOS] / 1.0e6, metrics[M_BUILD_NANOS] / 1.0e6, metrics[M_REBUILD_NANOS] / 1.0e6,
            metrics[M_BUILDS], metrics[M_REBUILT_AGGREGATES], metrics[M_DIRTY_REMAINING],
            metrics[M_EVICTED_PROPS], metrics[M_EVICTED_AGGREGATES],
            metrics[M_ROOT_SCAN_NANOS] / 1.0e6, metrics[M_ROOT_SCAN_PROPS], metrics[M_STATE_SWITCHES],
            metrics[M_PVS_REJECTED]);
    }

    record Frame(long visibleAggregates, long frustumTests, long drawCalls,
                 long translucentDrawCalls, long triangles, double renderMs, double buildMs, double rebuildMs,
                 long builds, long rebuiltAggregates, long dirtyRemaining,
                 long evictedProps, long evictedAggregates,
                 double rootScanMs, long rootScanProps, long stateSwitches, long pvsRejected) {}

    record Snapshot(Frame frame, Frame windowAverage, Frame totals, long completedFrames) {}
}
