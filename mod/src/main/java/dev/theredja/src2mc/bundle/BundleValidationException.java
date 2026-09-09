package dev.theredja.src2mc.bundle;

import java.io.IOException;

/** Checked validation failure safe to report in logs and command output. */
public final class BundleValidationException extends IOException {
    private final BundleErrorCode code;

    public BundleValidationException(BundleErrorCode code, String message) {
        super(code + ": " + message);
        this.code = code;
    }

    public BundleValidationException(BundleErrorCode code, String message, Throwable cause) {
        super(code + ": " + message, cause);
        this.code = code;
    }

    public BundleErrorCode code() {
        return code;
    }
}
