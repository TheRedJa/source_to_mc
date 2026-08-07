//! Direct access to the BSP lump directory.
//!
//! `vbsp` keeps its own lump reader private, and a couple of things need the
//! raw bytes: recovering original leaf order (see [`super::rawleaves`]) and
//! repairing text lumps that are not valid UTF-8.

use anyhow::{Result, bail, ensure};

/// Byte offset of the lump directory within the BSP header.
const DIRECTORY_OFFSET: usize = 8;
const ENTRY_SIZE: usize = 16;
pub const LUMP_COUNT: usize = 64;

pub const LUMP_ENTITIES: usize = 0;
pub const LUMP_LEAFS: usize = 10;
pub const LUMP_GAME_LUMP: usize = 35;

/// Byte offset of the BSP version word, straight after the `VBSP` magic.
const VERSION_OFFSET: usize = 4;

/// The version a BSP declares.
pub fn version(data: &[u8]) -> Result<i32> {
    ensure!(data.len() >= 8, "file is too small to be a BSP");
    ensure!(&data[0..4] == b"VBSP", "not a Source BSP file (bad magic)");
    read_i32(data, VERSION_OFFSET)
}

/// Present a version `vbsp` refuses as the nearest one it accepts, in place.
///
/// `vbsp` reads the version into an enum of 19, 20 and 21 and fails the whole
/// header on anything else. Version 22 is INFRA's custom Source branch, and it
/// is a branch marker rather than a format: the header is the usual 1036 bytes
/// of 64 lump entries and a revision, and every lump structure the conversion
/// reads is the same size it is in a version 21 map — planes 20 bytes, brushes
/// 12, brush sides 8, faces 56, texinfo 72, nodes 32, leaves 32, displacement
/// info 176, models 48, all dividing exactly into their lump lengths.
///
/// So the version word is rewritten rather than the parser taught a new
/// variant, in the same place and for the same reason the entity lump is
/// repaired: a byte the reader would refuse, edited before it sees it.
///
/// Returns the version the file actually declared.
pub fn present_as_known_version(data: &mut [u8]) -> Result<i32> {
    let declared = version(data)?;
    if declared == 22 {
        data[VERSION_OFFSET..VERSION_OFFSET + 4].copy_from_slice(&21i32.to_le_bytes());
    }
    Ok(declared)
}

#[derive(Debug, Clone, Copy)]
pub struct LumpEntry {
    pub offset: usize,
    pub length: usize,
    pub version: i32,
    /// Non-zero holds the uncompressed size of an LZMA-packed lump, which only
    /// console builds produce.
    pub four_cc: i32,
}

impl LumpEntry {
    pub fn range(&self) -> std::ops::Range<usize> {
        self.offset..self.offset + self.length
    }
}

/// Read one lump's directory entry, validating that it lies within the file.
pub fn lump_entry(data: &[u8], index: usize) -> Result<LumpEntry> {
    ensure!(index < LUMP_COUNT, "lump index {index} out of range");
    ensure!(
        data.len() >= DIRECTORY_OFFSET + LUMP_COUNT * ENTRY_SIZE,
        "file is too small to contain a BSP lump directory"
    );
    ensure!(&data[0..4] == b"VBSP", "not a Source BSP file (bad magic)");

    let at = DIRECTORY_OFFSET + index * ENTRY_SIZE;
    let entry = LumpEntry {
        offset: read_i32(data, at)? as usize,
        length: read_i32(data, at + 4)?.max(0) as usize,
        version: read_i32(data, at + 8)?,
        four_cc: read_i32(data, at + 12)?,
    };

    ensure!(
        entry
            .offset
            .checked_add(entry.length)
            .is_some_and(|end| end <= data.len()),
        "lump {index} extends past the end of the file"
    );
    Ok(entry)
}

fn read_i32(data: &[u8], at: usize) -> Result<i32> {
    let bytes = data
        .get(at..at + 4)
        .ok_or_else(|| anyhow::anyhow!("truncated BSP header at byte {at}"))?;
    Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Replace invalid UTF-8 bytes in a text lump, in place.
///
/// `vbsp` converts the entity and texture-string lumps with a strict
/// `String::from_utf8`, so a single stray byte fails the whole parse. Real maps
/// do contain them: E:Z2's `ez2_assassin_demo` has non-breaking spaces typed
/// into a light's `_ambient` value.
///
/// Substitutions are one byte for one byte, so every lump offset stays valid.
/// A `0xA0` becomes a space, since it is Latin-1's non-breaking space and
/// almost always stands in for one; anything else becomes `?`, which preserves
/// token boundaries in key/value text.
///
/// Returns how many bytes were replaced.
pub fn sanitize_text_lump(data: &mut [u8], index: usize) -> Result<usize> {
    let entry = lump_entry(data, index)?;
    if entry.four_cc != 0 {
        bail!("lump {index} is LZMA-compressed (console map?); not supported");
    }

    let lump = &mut data[entry.range()];
    let mut replaced = 0;
    let mut from = 0;
    while from < lump.len() {
        match std::str::from_utf8(&lump[from..]) {
            Ok(_) => break,
            Err(error) => {
                let bad = from + error.valid_up_to();
                // `None` means truncated trailing bytes: replace the rest.
                let len = error.error_len().unwrap_or(lump.len() - bad);
                for byte in &mut lump[bad..bad + len] {
                    *byte = if *byte == 0xA0 { b' ' } else { b'?' };
                    replaced += 1;
                }
                from = bad + len;
            }
        }
    }
    Ok(replaced)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BSP-shaped buffer whose entity lump holds `text`.
    fn bsp_with_entities(text: &[u8]) -> Vec<u8> {
        let header_len = DIRECTORY_OFFSET + LUMP_COUNT * ENTRY_SIZE + 4;
        let mut data = vec![0u8; header_len];
        data[0..4].copy_from_slice(b"VBSP");
        data[4..8].copy_from_slice(&20i32.to_le_bytes());

        let at = DIRECTORY_OFFSET + LUMP_ENTITIES * ENTRY_SIZE;
        data[at..at + 4].copy_from_slice(&(header_len as i32).to_le_bytes());
        data[at + 4..at + 8].copy_from_slice(&(text.len() as i32).to_le_bytes());
        data.extend_from_slice(text);
        data
    }

    fn entity_text(data: &[u8]) -> String {
        let entry = lump_entry(data, LUMP_ENTITIES).unwrap();
        String::from_utf8(data[entry.range()].to_vec()).unwrap()
    }

    #[test]
    fn a_version_22_map_is_presented_as_one_the_parser_knows() {
        let mut data = bsp_with_entities(b"{}");
        data[4..8].copy_from_slice(&22i32.to_le_bytes());
        assert_eq!(present_as_known_version(&mut data).unwrap(), 22);
        assert_eq!(version(&data).unwrap(), 21, "the parser still sees 22");
    }

    /// Everything the parser already accepts has to pass through untouched.
    #[test]
    fn a_version_it_already_accepts_is_left_alone() {
        for declared in [19i32, 20, 21] {
            let mut data = bsp_with_entities(b"{}");
            data[4..8].copy_from_slice(&declared.to_le_bytes());
            assert_eq!(present_as_known_version(&mut data).unwrap(), declared);
            assert_eq!(version(&data).unwrap(), declared);
        }
    }

    /// Presenting a map as version 21 puts it in reach of `vbsp`'s Left 4 Dead
    /// 2 lump-order heuristic, which reinterprets every lump entry. It decides
    /// by looking for a lump whose first word is too small to be a file offset,
    /// so a real map's first lump — always far into the file — settles it.
    #[test]
    fn a_rewritten_map_does_not_look_like_left_4_dead_2() {
        let mut data = bsp_with_entities(b"{}");
        data[4..8].copy_from_slice(&22i32.to_le_bytes());
        present_as_known_version(&mut data).unwrap();

        let mut small = 0;
        for index in 0..LUMP_COUNT {
            let offset = read_i32(&data, DIRECTORY_OFFSET + index * ENTRY_SIZE).unwrap();
            if offset > 20 {
                return; // The heuristic bails here, which is the point.
            }
            if offset != 0 && offset % 4 != 0 {
                small += 1;
            }
        }
        assert_eq!(small, 0, "the lump table would be read as Left 4 Dead 2's order");
    }

    #[test]
    fn a_file_that_is_not_a_bsp_is_refused() {
        let mut data = bsp_with_entities(b"{}");
        data[0..4].copy_from_slice(b"NOPE");
        assert!(present_as_known_version(&mut data).is_err());
        assert!(present_as_known_version(&mut Vec::new()).is_err());
    }

    #[test]
    fn valid_text_is_left_alone() {
        let mut data = bsp_with_entities(b"{\"classname\" \"worldspawn\"}");
        assert_eq!(sanitize_text_lump(&mut data, LUMP_ENTITIES).unwrap(), 0);
        assert_eq!(entity_text(&data), "{\"classname\" \"worldspawn\"}");
    }

    #[test]
    fn non_breaking_spaces_become_spaces() {
        // The exact shape found in ez2_assassin_demo.
        let mut data = bsp_with_entities(b"\"_ambient\" \"75\xa073\xa0114\xa025\"");
        assert_eq!(sanitize_text_lump(&mut data, LUMP_ENTITIES).unwrap(), 3);
        assert_eq!(entity_text(&data), "\"_ambient\" \"75 73 114 25\"");
    }

    #[test]
    fn other_invalid_bytes_become_question_marks() {
        let mut data = bsp_with_entities(b"\"targetname\" \"caf\xe9\"");
        assert_eq!(sanitize_text_lump(&mut data, LUMP_ENTITIES).unwrap(), 1);
        assert_eq!(entity_text(&data), "\"targetname\" \"caf?\"");
    }

    #[test]
    fn well_formed_multibyte_utf8_survives() {
        let mut data = bsp_with_entities("\"message\" \"café ünïcode\"".as_bytes());
        assert_eq!(sanitize_text_lump(&mut data, LUMP_ENTITIES).unwrap(), 0);
        assert_eq!(entity_text(&data), "\"message\" \"café ünïcode\"");
    }

    #[test]
    fn lump_length_is_preserved_so_offsets_stay_valid() {
        let original = b"\"a\" \"\xff\xfe\xfd\"".to_vec();
        let mut data = bsp_with_entities(&original);
        let before = data.len();
        sanitize_text_lump(&mut data, LUMP_ENTITIES).unwrap();
        assert_eq!(data.len(), before);
        assert_eq!(lump_entry(&data, LUMP_ENTITIES).unwrap().length, original.len());
    }

    #[test]
    fn trailing_truncated_sequence_is_replaced() {
        // A lead byte with no continuation byte, at the very end.
        let mut data = bsp_with_entities(b"ok\xc3");
        assert_eq!(sanitize_text_lump(&mut data, LUMP_ENTITIES).unwrap(), 1);
        assert_eq!(entity_text(&data), "ok?");
    }
}
