package dev.theredja.src2mc.bundle;

/**
 * Where the current bundle load has got to, for anyone who wants to say so.
 *
 * A load now runs on its own threads while the game is at the menu, so it needs
 * somewhere to report from that costs nothing to read every frame. One volatile
 * reference to an immutable snapshot, rewritten as each bundle finishes, is
 * enough: readers never block and the loader pays one store per bundle.
 */
public final class BundleLoadProgress {
    public enum State { IDLE, LOADING, DONE, FAILED }

    public record Snapshot(State state, int done, int total, String current, long millis, String message) {
        public static final Snapshot IDLE = new Snapshot(State.IDLE, 0, 0, "", 0L, "");
    }

    private static volatile Snapshot current = Snapshot.IDLE;

    private BundleLoadProgress() {}

    public static Snapshot snapshot() { return current; }

    static void started(int total) {
        current = new Snapshot(State.LOADING, 0, total, "", 0L, "");
    }

    static void bundleDone(int done, String name) {
        Snapshot before = current;
        current = new Snapshot(State.LOADING, done, before.total(), name, before.millis(), "");
    }

    static void finished(int bundles, long sequence, long millis) {
        current = new Snapshot(State.DONE, bundles, bundles, "", millis,
            "generation " + sequence + ", " + bundles + " bundle(s), " + millis + " ms");
    }

    static void failed(String message, long millis) {
        Snapshot before = current;
        current = new Snapshot(State.FAILED, before.done(), before.total(), before.current(), millis, message);
    }
}
