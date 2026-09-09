//! Hard v1 parsing/allocation ceilings. These are corruption guards, not the
//! configurable runtime texture-residency budget used by the client.

use anyhow::{Result, ensure};

pub const MAX_ENTRY_COUNT: u64 = 100_000;
pub const MAX_ENTRY_PATH_BYTES: usize = 240;
pub const MAX_MAPS: u64 = 4_096;
pub const MAX_JSON_NESTING: usize = 64;
pub const MAX_MATERIALS_PER_MAP: u64 = 1_000_000;
pub const MAX_MODELS_PER_CAMPAIGN: u64 = 1_000_000;
pub const MAX_PROPS_PER_MAP: u64 = 10_000_000;
pub const MAX_UV_REGIONS_PER_MAP: u64 = 10_000_000;
pub const MAX_SECTIONS_PER_MAP: u64 = 4_000_000;
pub const MAX_FACES_PER_MAP: u64 = 100_000_000;
pub const MAX_VERTICES_PER_MESH: u64 = 10_000_000;
pub const MAX_INDICES_PER_MESH: u64 = 30_000_000;
pub const MAX_SUBMESHES_PER_MESH: u64 = 65_536;
pub const MAX_TEXTURE_DIMENSION: u64 = 16_384;
pub const MAX_OUTPUT_TEXTURE_DIMENSION: u64 = 16_384;
pub const MAX_DECODED_TEXTURE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_UNCOMPRESSED_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_UNCOMPRESSED_BUNDLE_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_ZIP_EXPANSION_RATIO: u64 = 200;
pub const MAX_RUNTIME_ALLOCATION_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    UnsafePath,
    LimitExceeded,
    ZipExpansion,
    HashMismatch,
    MissingEntry,
    UnsupportedVersion,
    InvalidSchema,
    InvalidReference,
    DuplicateIdentity,
    NoFreePropRoot,
    SchematicTooLarge,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsafePath => "UNSAFE_PATH",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::ZipExpansion => "ZIP_EXPANSION_LIMIT",
            Self::HashMismatch => "HASH_MISMATCH",
            Self::MissingEntry => "MISSING_ENTRY",
            Self::UnsupportedVersion => "UNSUPPORTED_VERSION",
            Self::InvalidSchema => "INVALID_SCHEMA",
            Self::InvalidReference => "INVALID_REFERENCE",
            Self::DuplicateIdentity => "DUPLICATE_IDENTITY",
            Self::NoFreePropRoot => "NO_FREE_PROP_ROOT",
            Self::SchematicTooLarge => "SCHEMATIC_TOO_LARGE",
        }
    }
}

pub fn check_count(label: &str, count: u64, maximum: u64) -> Result<()> {
    ensure!(
        count <= maximum,
        "{}: {label} count {count} exceeds {maximum}",
        ErrorCode::LimitExceeded.as_str()
    );
    Ok(())
}

pub fn check_zip_entry(compressed: u64, uncompressed: u64) -> Result<()> {
    ensure!(
        uncompressed <= MAX_UNCOMPRESSED_ENTRY_BYTES,
        "{}: entry expands to {uncompressed} bytes",
        ErrorCode::LimitExceeded.as_str()
    );
    let allowed = compressed
        .saturating_mul(MAX_ZIP_EXPANSION_RATIO)
        .max(1 << 20);
    ensure!(
        uncompressed <= allowed,
        "{}: {compressed} compressed bytes expand to {uncompressed}",
        ErrorCode::ZipExpansion.as_str()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_allows_small_files_but_rejects_bombs() {
        check_zip_entry(1, 1024 * 1024).unwrap();
        assert!(check_zip_entry(1024, 2 * 1024 * 1024).is_err());
    }

    #[test]
    fn error_codes_are_stable_machine_tokens() {
        for code in [
            ErrorCode::UnsafePath,
            ErrorCode::LimitExceeded,
            ErrorCode::ZipExpansion,
            ErrorCode::HashMismatch,
            ErrorCode::MissingEntry,
            ErrorCode::UnsupportedVersion,
            ErrorCode::InvalidSchema,
            ErrorCode::InvalidReference,
            ErrorCode::DuplicateIdentity,
            ErrorCode::NoFreePropRoot,
            ErrorCode::SchematicTooLarge,
        ] {
            assert!(
                code.as_str()
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b == b'_')
            );
        }
    }
}
