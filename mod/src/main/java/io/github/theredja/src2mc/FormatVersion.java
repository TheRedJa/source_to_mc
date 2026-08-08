package io.github.theredja.src2mc;

/**
 * The one check that has to happen before anything else is read.
 *
 * <p>{@code docs/format.md} §1: the mod reads the version first and stops with a
 * clear message naming both versions if it does not match. Output written before
 * the document existed carries no version at all, which is version 0 and means
 * the converter has to be re-run.
 */
public final class FormatVersion {
    /** Output with no version key at all predates {@code docs/format.md}. */
    public static final int ABSENT = 0;

    private FormatVersion() {}

    public static boolean isSupported(int version) {
        return version == Src2mc.FORMAT_VERSION;
    }

    /**
     * Why {@code version} is unacceptable, naming both numbers, or empty if it is
     * acceptable. {@code what} names the thing being read, so the message points
     * at a file rather than at the mod.
     */
    public static java.util.Optional<String> reject(int version, String what) {
        if (isSupported(version)) {
            return java.util.Optional.empty();
        }
        if (version == ABSENT) {
            return java.util.Optional.of(what
                    + " carries no src2mc format version, so it was written before the format existed"
                    + " (version " + ABSENT + "); this mod reads version " + Src2mc.FORMAT_VERSION
                    + ". Re-run the converter.");
        }
        return java.util.Optional.of(what
                + " is src2mc format version " + version
                + "; this mod reads version " + Src2mc.FORMAT_VERSION
                + ". Re-run the converter, or use the mod build that matches it.");
    }

    /** As {@link #reject}, as an exception, for the paths that cannot continue. */
    public static void require(int version, String what) {
        reject(version, what).ifPresent(message -> {
            throw new UnsupportedVersion(message);
        });
    }

    public static final class UnsupportedVersion extends RuntimeException {
        public UnsupportedVersion(String message) {
            super(message);
        }
    }
}
