//! Deterministic writer for the version-1 `.src2mc` campaign container.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Seek, Write};
use std::path::Path;

pub const EXTENSION: &str = "src2mc";
pub const FORMAT_VERSION: u32 = 1;
pub const MANIFEST_PATH: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryDigest {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Manifest {
    pub format: &'static str,
    pub version: u32,
    pub campaign_id: String,
    pub fingerprint: String,
    pub entries: Vec<EntryDigest>,
}

/// Payload entries kept in path order. The generated manifest is deliberately
/// not part of its own fingerprint.
#[derive(Debug, Default)]
pub struct Bundle {
    entries: BTreeMap<String, Vec<u8>>,
    total_size: u64,
}

impl Bundle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, path: impl Into<String>, bytes: Vec<u8>) -> Result<()> {
        let path = path.into();
        validate_entry_path(&path)?;
        ensure!(path != MANIFEST_PATH, "manifest.json is reserved");
        crate::output::limits::check_count(
            "bundle entry",
            self.entries.len() as u64 + 1,
            crate::output::limits::MAX_ENTRY_COUNT,
        )?;
        ensure!(
            bytes.len() as u64 <= crate::output::limits::MAX_UNCOMPRESSED_ENTRY_BYTES,
            "{}: entry `{path}` is too large",
            crate::output::limits::ErrorCode::LimitExceeded.as_str()
        );
        let total = self.total_size.checked_add(bytes.len() as u64);
        ensure!(
            total.is_some_and(|size| size <= crate::output::limits::MAX_UNCOMPRESSED_BUNDLE_BYTES),
            "{}: bundle payload is too large",
            crate::output::limits::ErrorCode::LimitExceeded.as_str()
        );
        ensure!(
            !self.entries.contains_key(&path),
            "duplicate bundle entry `{path}`"
        );
        self.entries.insert(path, bytes);
        self.total_size = total.expect("validated bundle size");
        Ok(())
    }

    /// Add canonical bytes below a content-addressed path and return their ID.
    pub fn add_content(&mut self, directory: &str, suffix: &str, bytes: Vec<u8>) -> Result<String> {
        validate_component(directory)?;
        ensure!(
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
            "invalid content suffix `{suffix}`"
        );
        let id = sha256(&bytes);
        let path = format!("{directory}/{id}.{suffix}");
        validate_entry_path(&path)?;
        if let Some(existing) = self.entries.get(&path) {
            ensure!(existing == &bytes, "content hash collision at `{path}`");
        } else {
            self.add(path, bytes)?;
        }
        Ok(id)
    }

    pub fn manifest(&self, campaign_id: impl Into<String>) -> Result<Manifest> {
        let campaign_id = campaign_id.into();
        validate_id(&campaign_id, "campaign")?;
        let entries: Vec<_> = self
            .entries
            .iter()
            .map(|(path, bytes)| EntryDigest {
                path: path.clone(),
                size: bytes.len() as u64,
                sha256: sha256(bytes),
            })
            .collect();
        let fingerprint = fingerprint(&entries);
        Ok(Manifest {
            format: "src2mc-campaign",
            version: FORMAT_VERSION,
            campaign_id,
            fingerprint,
            entries,
        })
    }

    pub fn write(&self, path: &Path, campaign_id: impl Into<String>) -> Result<Manifest> {
        ensure!(
            path.extension().and_then(|v| v.to_str()) == Some(EXTENSION),
            "bundle output must use the .{EXTENSION} extension"
        );
        let manifest = self.manifest(campaign_id)?;
        let manifest_bytes = canonical_json(&manifest)?;
        let file =
            std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
        self.write_to(file, &manifest_bytes)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(manifest)
    }

    fn write_to<W: Write + Seek>(&self, writer: W, manifest: &[u8]) -> Result<()> {
        use zip::write::SimpleFileOptions;
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut zip = zip::ZipWriter::new(writer);
        zip.start_file(MANIFEST_PATH, options)?;
        zip.write_all(manifest)?;
        for (path, bytes) in &self.entries {
            zip.start_file(path, options)?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        Ok(())
    }
}

/// Stable JSON for format-owned structs: compact UTF-8, declared field order,
/// sorted maps, no insignificant whitespace, and one trailing LF.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).context("encoding canonical JSON")?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// V1 preserves finite IEEE-754 values but maps both signed zeros to +0.
pub fn canonical_f64(value: f64) -> Result<f64> {
    ensure!(
        value.is_finite(),
        "non-finite float is not valid in format v1"
    );
    Ok(if value == 0.0 { 0.0 } else { value })
}

pub fn content_id(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256(bytes: &[u8]) -> String {
    content_id(bytes)
}

fn fingerprint(entries: &[EntryDigest]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"src2mc-manifest-v1\0");
    for entry in entries {
        hash.update((entry.path.len() as u32).to_le_bytes());
        hash.update(entry.path.as_bytes());
        hash.update(entry.size.to_le_bytes());
        hash.update(hex_to_bytes(&entry.sha256));
    }
    format!("{:x}", hash.finalize())
}

fn hex_to_bytes(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |b: u8| match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                _ => 0,
            };
            digit(pair[0]) << 4 | digit(pair[1])
        })
        .collect()
}

fn validate_component(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'),
        "invalid bundle path component `{value}`"
    );
    Ok(())
}

/// IDs are safe, portable bundle path components and Minecraft resource-path
/// components. Human-facing names are stored separately in metadata.
pub fn validate_id(value: &str, kind: &str) -> Result<()> {
    validate_component(value).with_context(|| format!("invalid {kind} ID `{value}`"))
}

pub fn validate_entry_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 240
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
    {
        bail!("unsafe bundle entry path `{path}`");
    }
    for part in path.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains(':')
            || part.bytes().any(|b| b < 0x20 || b == 0x7f)
        {
            bail!("unsafe bundle entry path `{path}`");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn insertion_order_does_not_change_identity() {
        let mut a = Bundle::new();
        a.add("maps/z.json", b"z\n".to_vec()).unwrap();
        a.add("maps/a.json", b"a\n".to_vec()).unwrap();
        let mut b = Bundle::new();
        b.add("maps/a.json", b"a\n".to_vec()).unwrap();
        b.add("maps/z.json", b"z\n".to_vec()).unwrap();
        assert_eq!(
            a.manifest("campaign").unwrap(),
            b.manifest("campaign").unwrap()
        );
    }

    #[test]
    fn content_is_deduplicated_by_canonical_bytes() {
        let mut bundle = Bundle::new();
        let a = bundle
            .add_content("textures", "png", vec![1, 2, 3])
            .unwrap();
        let b = bundle
            .add_content("textures", "png", vec![1, 2, 3])
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(bundle.entries.len(), 1);
    }

    #[test]
    fn rejected_duplicate_does_not_replace_the_first_payload() {
        let mut bundle = Bundle::new();
        bundle.add("maps/a.json", b"first\n".to_vec()).unwrap();
        assert!(bundle.add("maps/a.json", b"second\n".to_vec()).is_err());
        assert_eq!(bundle.entries["maps/a.json"], b"first\n");
    }

    #[test]
    fn rejects_zip_slip_and_ambiguous_paths() {
        for path in [
            "../x", "a/../x", "/x", "a\\x", "a//x", "a/./x", "C:/x", "x/",
        ] {
            assert!(validate_entry_path(path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn float_rules_reject_nonfinite_and_collapse_negative_zero() {
        assert_eq!(canonical_f64(-0.0).unwrap().to_bits(), 0.0f64.to_bits());
        assert!(canonical_f64(f64::NAN).is_err());
        assert!(canonical_f64(f64::INFINITY).is_err());
    }

    #[test]
    fn writer_emits_a_zip_without_changing_identity() {
        let mut bundle = Bundle::new();
        bundle.add("maps/a.json", b"{}\n".to_vec()).unwrap();
        let manifest = bundle.manifest("test").unwrap();
        let bytes = canonical_json(&manifest).unwrap();
        let mut output = Cursor::new(Vec::new());
        bundle.write_to(&mut output, &bytes).unwrap();
        assert!(!output.into_inner().is_empty());
        assert_eq!(manifest, bundle.manifest("test").unwrap());
    }
}
