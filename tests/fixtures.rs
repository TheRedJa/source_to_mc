//! Golden fixtures for the interchange format described in `docs/format.md`.
//!
//! The files under `tests/fixtures` are real converter output, committed. This
//! test regenerates them and fails if a byte moved; the companion mod's tests
//! read the same files. That is the point — a change on either side that breaks
//! the other fails in the same CI run, which no amount of prose in the format
//! document can achieve.
//!
//! Fixtures are stored as uncompressed NBT rather than as gzipped `.schem`, so
//! the bytes depend only on what we serialize and not on which version of the
//! compressor happens to be in the lock file.
//!
//! Regenerate deliberately, and read the diff:
//!
//! ```sh
//! UPDATE_FIXTURES=1 cargo test --test fixtures
//! ```

use src2mc::output::schem;
use src2mc::voxel::grid::Palette;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Compare `actual` against the committed fixture, or rewrite it when
/// `UPDATE_FIXTURES` is set.
fn golden(name: &str, actual: &[u8]) {
    let path = fixtures().join(name);

    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }

    let expected = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read fixture {}: {e}\n\
             If this is a new fixture, create it with UPDATE_FIXTURES=1 cargo test --test fixtures",
            path.display()
        )
    });

    assert!(
        expected == actual,
        "fixture {} is out of date ({} bytes committed, {} bytes produced).\n\
         This is an interface change: see docs/format.md. Bump FORMAT_VERSION on \
         both sides, then regenerate with UPDATE_FIXTURES=1 cargo test --test fixtures",
        path.display(),
        expected.len(),
        actual.len()
    );
}

/// A deliberately tiny scene, chosen so the whole fixture can be reasoned about
/// by hand: two named blocks in a 4x3x5 region at a non-zero offset.
fn tiny() -> Vec<u8> {
    let mut palette = Palette::new();
    let stone = palette.intern("minecraft:stone");
    let glass = palette.intern("minecraft:glass");

    let blocks = vec![([0, 0, 0], stone), ([2, 1, 3], glass)];

    schem::encode_blocks(&blocks, &palette, [0, 0, 0], [3, 2, 4], "tiny").unwrap()
}

#[test]
fn tiny_schematic_matches_the_fixture() {
    golden("tiny.nbt", &tiny());
}

/// Serializing the same scene twice must produce identical bytes, or the
/// fixture above would fail at random rather than when something changed.
#[test]
fn encoding_is_deterministic() {
    assert!(tiny() == tiny(), "schematic encoding is not reproducible");
}

/// The mod reads this key before anything else, so make its absence loud here
/// rather than in a Minecraft log.
#[test]
fn the_format_version_is_written() {
    let root: fastnbt::Value = fastnbt::from_bytes(&tiny()).unwrap();
    let fastnbt::Value::Compound(root) = root else {
        panic!("root is not a compound")
    };
    let fastnbt::Value::Compound(schematic) = &root["Schematic"] else {
        panic!("missing Schematic")
    };
    let fastnbt::Value::Compound(metadata) = &schematic["Metadata"] else {
        panic!("missing Metadata")
    };

    assert_eq!(
        metadata["src2mc:FormatVersion"],
        fastnbt::Value::Int(src2mc::FORMAT_VERSION),
        "docs/format.md requires Metadata.src2mc:FormatVersion"
    );
}
