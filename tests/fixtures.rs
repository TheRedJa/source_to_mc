//! Golden fixtures for schematic output.
//!
//! The files under `tests/fixtures` are real converter output, committed. This
//! test regenerates them and fails if a byte moved.
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
         Regenerate deliberately with UPDATE_FIXTURES=1 cargo test --test fixtures",
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
