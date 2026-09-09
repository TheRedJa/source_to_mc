package dev.theredja.src2mc.bundle;

/** Stable machine-readable v1 validation failures shared with the converter. */
public enum BundleErrorCode {
    UNSAFE_PATH,
    LIMIT_EXCEEDED,
    ZIP_EXPANSION_LIMIT,
    HASH_MISMATCH,
    MISSING_ENTRY,
    UNSUPPORTED_VERSION,
    INVALID_SCHEMA,
    INVALID_REFERENCE,
    DUPLICATE_IDENTITY
}
