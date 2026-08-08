package io.github.theredja.src2mc;

import net.neoforged.fml.common.Mod;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Entry point and the one place the interchange contract is stated in Java.
 *
 * <p>Everything this mod reads is described by {@code docs/format.md} in the
 * repository root. That document is normative: where it and this code disagree,
 * this code has a bug.
 */
@Mod(Src2mc.MOD_ID)
public final class Src2mc {
    /**
     * Also the namespace of every block this mod registers, so a surface block
     * is {@code src2mc:surface_<n>} and a prop block entity is
     * {@code src2mc:prop}.
     */
    public static final String MOD_ID = "src2mc";

    /**
     * Version of the interchange format in {@code docs/format.md}.
     *
     * <p>Must equal {@code src2mc::FORMAT_VERSION} in {@code src/lib.rs}. A test
     * checks that it does, so the two cannot drift silently. Bump both in the
     * same commit as the regenerated fixtures.
     */
    public static final int FORMAT_VERSION = 1;

    public static final Logger LOG = LoggerFactory.getLogger(MOD_ID);

    public Src2mc() {
        LOG.info("src2mc loaded, reading interchange format version {}", FORMAT_VERSION);
    }
}
