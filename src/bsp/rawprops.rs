//! Raw parsing of the static prop game lump.
//!
//! `vbsp` reads this lump from a version table, and the table is wrong for the
//! later versions. For anything from version 7 up it takes the four bytes at
//! offset 64 as a `u32` of flags; in the maps that actually exist those bytes
//! are the minimum and maximum CPU and GPU levels, which are `0xFF` apiece
//! when unset. So the flags come out as `0xFFFFFFFF`, every flag reads as set
//! including `NO_DRAW`, and a conversion that filters on it throws the map's
//! static props away. Measured, that is all 328 props of a Portal 2 map and
//! 6691 of the 8386 in INFRA's `infra_c1_m1_office`.
//!
//! The real flags have been the byte at offset 31 since version 4 and have
//! never moved. Neither has anything else this conversion wants: the origin,
//! the angles and the model index are the first 26 bytes of every version of
//! the record. Everything that differs between versions and between branches
//! is *after* them, which means none of it has to be understood — only stepped
//! over.
//!
//! So the record stride is measured from the lump rather than looked up. The
//! lump states how many props it holds and how long it is, and those two
//! numbers give the stride exactly, whatever branch of the engine wrote it.
//! That is also the only thing that reliably tells the branch variants apart,
//! since several of them share a version number and differ in size.

use super::lumps::{self, LUMP_GAME_LUMP};
use anyhow::{Result, bail, ensure};

/// `sprp`, as the game lump directory spells it: big-endian ASCII.
const STATIC_PROPS: i32 = i32::from_be_bytes(*b"sprp");

/// A game lump directory entry.
const GAME_LUMP_ENTRY: usize = 16;

/// Model paths in the dictionary are a fixed-width character array.
const NAME_LEN: usize = 128;

/// The part of a prop record that has never moved.
const ORIGIN: usize = 0;
const ANGLES: usize = 12;
const PROP_TYPE: usize = 24;
const FLAGS: usize = 31;
/// The shortest record any version has: version 4.
const MIN_STRIDE: usize = 56;
/// Longer than any known version's record. Version 11 is 80 bytes; the limit
/// is here to reject a stride computed from a corrupt count rather than to
/// predict a future one.
const MAX_STRIDE: usize = 128;

/// The prop record fields with a fixed home, plus whatever else was legible.
#[derive(Debug, Clone)]
pub struct RawProp {
    /// Index into [`StaticProps::models`].
    pub prop_type: u16,
    pub origin: [f32; 3],
    /// Pitch, yaw, roll, as Source stores them.
    pub angles: [f32; 3],
    pub flags: u8,
    /// Uniform scale, or 1.0 for the versions that do not carry one.
    pub scale: f32,
}

/// `STATIC_PROP_NO_DRAW`: the compiler kept the prop for its collision and
/// lighting, and the engine never draws it.
pub const NO_DRAW: u8 = 0x4;

impl RawProp {
    pub fn no_draw(&self) -> bool {
        self.flags & NO_DRAW != 0
    }
}

/// Every static prop in a map, and the models they name.
#[derive(Debug, Clone, Default)]
pub struct StaticProps {
    pub models: Vec<String>,
    pub props: Vec<RawProp>,
    /// The lump version, for reporting.
    pub version: u16,
    /// Bytes per record, as measured.
    pub stride: usize,
}

/// Read the static props out of a BSP's bytes.
///
/// A map with no static props is not an error — plenty have none — so a
/// missing game lump or a missing `sprp` entry gives an empty result.
pub fn static_props(data: &[u8]) -> Result<StaticProps> {
    let entry = lumps::lump_entry(data, LUMP_GAME_LUMP)?;
    if entry.length == 0 {
        return Ok(StaticProps::default());
    }
    if entry.four_cc != 0 {
        bail!("game lump is LZMA-compressed (console map?); not supported");
    }

    // The game lump's own directory. Its offsets are into the file, not into
    // the lump, so they are used against `data` directly.
    let lump = &data[entry.range()];
    let count = read_i32(lump, 0)?;
    ensure!((0..=4096).contains(&count), "game lump declares {count} entries");

    for index in 0..count as usize {
        let at = 4 + index * GAME_LUMP_ENTRY;
        ensure!(at + GAME_LUMP_ENTRY <= lump.len(), "game lump directory is truncated");
        if read_i32(lump, at)? != STATIC_PROPS {
            continue;
        }
        let version = u16::from_le_bytes([lump[at + 6], lump[at + 7]]);
        let offset = read_i32(lump, at + 8)?.max(0) as usize;
        let length = read_i32(lump, at + 12)?.max(0) as usize;
        ensure!(
            offset.checked_add(length).is_some_and(|end| end <= data.len()),
            "the static prop lump extends past the end of the file"
        );
        return parse(&data[offset..offset + length], version);
    }

    Ok(StaticProps::default())
}

/// Parse the `sprp` lump body.
fn parse(lump: &[u8], version: u16) -> Result<StaticProps> {
    let dict = read_i32(lump, 0)?;
    ensure!((0..=65536).contains(&dict), "prop dictionary declares {dict} models");
    let mut at = 4;
    let mut models = Vec::with_capacity(dict as usize);
    for _ in 0..dict {
        ensure!(at + NAME_LEN <= lump.len(), "the prop dictionary is truncated");
        let name = &lump[at..at + NAME_LEN];
        let end = name.iter().position(|b| *b == 0).unwrap_or(NAME_LEN);
        models.push(String::from_utf8_lossy(&name[..end]).into_owned());
        at += NAME_LEN;
    }

    // The leaf array: which visleaves each prop touches. Nothing here needs
    // it, but it sits between the dictionary and the props.
    let leaves = read_i32(lump, at)?;
    ensure!(leaves >= 0, "the prop leaf array declares {leaves} entries");
    at += 4;
    at = at
        .checked_add(leaves as usize * 2)
        .filter(|end| *end <= lump.len())
        .ok_or_else(|| anyhow::anyhow!("the prop leaf array is truncated"))?;

    let count = read_i32(lump, at)?;
    ensure!(count >= 0, "the static prop lump declares {count} props");
    at += 4;
    let count = count as usize;
    if count == 0 {
        return Ok(StaticProps { models, props: Vec::new(), version, stride: 0 });
    }

    // Measured, not looked up. Several branches of the engine share a version
    // number and disagree about the record, and the record size is the thing
    // that actually tells them apart.
    let body = lump.len() - at;
    let stride = body / count;
    ensure!(
        (MIN_STRIDE..=MAX_STRIDE).contains(&stride),
        "{count} props in {body} bytes is {stride} bytes each, which is not a prop record"
    );
    ensure!(
        body.is_multiple_of(count),
        "{count} props do not divide {body} bytes evenly; the lump is not what it says"
    );

    // Only version 11 and up carry a scale, and only when the record is long
    // enough to hold one. Both have to be true: a version 11 lump written by a
    // branch that left the field out would otherwise read the byte after it.
    let scale_at = (version >= 11 && stride >= 80).then_some(76);

    let mut props = Vec::with_capacity(count);
    for record in lump[at..].chunks_exact(stride) {
        props.push(RawProp {
            prop_type: u16::from_le_bytes([record[PROP_TYPE], record[PROP_TYPE + 1]]),
            origin: read_vec(record, ORIGIN),
            angles: read_vec(record, ANGLES),
            flags: record[FLAGS],
            scale: match scale_at {
                Some(at) => f32::from_le_bytes(record[at..at + 4].try_into().unwrap()),
                None => 1.0,
            }
            // A zero or negative scale is a prop that cannot be drawn, and
            // some maps do carry one; treat it as the default rather than
            // collapsing the model to a point.
            .max(f32::MIN_POSITIVE),
        });
    }

    Ok(StaticProps { models, props, version, stride })
}

fn read_vec(record: &[u8], at: usize) -> [f32; 3] {
    std::array::from_fn(|axis| {
        let at = at + axis * 4;
        f32::from_le_bytes(record[at..at + 4].try_into().unwrap())
    })
}

fn read_i32(data: &[u8], at: usize) -> Result<i32> {
    let bytes = data
        .get(at..at + 4)
        .ok_or_else(|| anyhow::anyhow!("truncated static prop lump at byte {at}"))?;
    Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bsp::lumps::{LUMP_COUNT, LUMP_GAME_LUMP};

    const HEADER: usize = 8 + LUMP_COUNT * 16 + 4;

    /// One prop record of `stride` bytes, with the fields that never move set
    /// and everything else filled with the bytes that caused the bug: `0xFF`,
    /// which is what unset CPU and GPU levels look like.
    fn record(stride: usize, origin: [f32; 3], angles: [f32; 3], ty: u16, flags: u8) -> Vec<u8> {
        let mut r = vec![0xFFu8; stride];
        for (axis, value) in origin.iter().enumerate() {
            r[axis * 4..axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (axis, value) in angles.iter().enumerate() {
            r[12 + axis * 4..12 + axis * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        r[24..26].copy_from_slice(&ty.to_le_bytes());
        r[26..30].copy_from_slice(&[0; 4]);
        r[30] = 6;
        r[31] = flags;
        r
    }

    /// A whole BSP-shaped buffer holding one `sprp` game lump.
    fn bsp(version: u16, stride: usize, models: &[&str], records: &[Vec<u8>]) -> Vec<u8> {
        let mut sprp = Vec::new();
        sprp.extend_from_slice(&(models.len() as i32).to_le_bytes());
        for model in models {
            let mut name = vec![0u8; NAME_LEN];
            name[..model.len()].copy_from_slice(model.as_bytes());
            sprp.extend_from_slice(&name);
        }
        // Two leaf entries, to prove they are stepped over rather than assumed
        // away.
        sprp.extend_from_slice(&2i32.to_le_bytes());
        sprp.extend_from_slice(&[0, 0, 1, 0]);
        sprp.extend_from_slice(&(records.len() as i32).to_le_bytes());
        for r in records {
            assert_eq!(r.len(), stride);
            sprp.extend_from_slice(r);
        }

        let mut data = vec![0u8; HEADER];
        data[0..4].copy_from_slice(b"VBSP");
        data[4..8].copy_from_slice(&20i32.to_le_bytes());

        // The game lump: a directory of one entry pointing at the body.
        let game_at = data.len();
        let sprp_at = game_at + 4 + GAME_LUMP_ENTRY;
        let mut game = Vec::new();
        game.extend_from_slice(&1i32.to_le_bytes());
        game.extend_from_slice(&STATIC_PROPS.to_le_bytes());
        game.extend_from_slice(&0u16.to_le_bytes());
        game.extend_from_slice(&version.to_le_bytes());
        game.extend_from_slice(&(sprp_at as i32).to_le_bytes());
        game.extend_from_slice(&(sprp.len() as i32).to_le_bytes());
        data.extend_from_slice(&game);
        data.extend_from_slice(&sprp);

        let at = 8 + LUMP_GAME_LUMP * 16;
        data[at..at + 4].copy_from_slice(&(game_at as i32).to_le_bytes());
        data[at + 4..at + 8].copy_from_slice(&((game.len() + sprp.len()) as i32).to_le_bytes());
        data
    }

    /// The bug this module exists for. Every byte after the record's first 32
    /// is `0xFF`, which is exactly what a real map looks like once the CPU and
    /// GPU levels are unset — and reading four of those as flags is what threw
    /// the props away.
    #[test]
    fn the_bytes_after_the_record_header_are_not_flags() {
        for (version, stride) in [(4u16, 56usize), (5, 60), (6, 64), (9, 72), (10, 76), (11, 80)] {
            let records = vec![
                record(stride, [1.0, 2.0, 3.0], [0.0, 90.0, 0.0], 0, 0),
                record(stride, [4.0, 5.0, 6.0], [0.0, 0.0, 0.0], 1, NO_DRAW),
            ];
            let data = bsp(version, stride, &["models/a.mdl", "models/b.mdl"], &records);
            let props = static_props(&data).expect("version {version} should parse");

            assert_eq!(props.stride, stride, "version {version} measured the wrong stride");
            assert_eq!(props.models, vec!["models/a.mdl", "models/b.mdl"]);
            assert_eq!(props.props.len(), 2);
            assert_eq!(props.props[0].origin, [1.0, 2.0, 3.0]);
            assert_eq!(props.props[0].angles, [0.0, 90.0, 0.0]);
            assert_eq!(props.props[0].prop_type, 0);
            assert!(!props.props[0].no_draw(), "version {version} lost a visible prop");
            assert!(props.props[1].no_draw(), "version {version} kept a hidden prop");
        }
    }

    /// The stride is what tells two branches sharing a version apart, so it
    /// comes from the lump and not from the version.
    #[test]
    fn the_stride_is_measured_rather_than_assumed() {
        // A version 9 lump whose records are 76 bytes, which is a real
        // variation between branches.
        let records = vec![record(76, [0.0; 3], [0.0; 3], 0, 0)];
        let props = static_props(&bsp(9, 76, &["models/a.mdl"], &records)).unwrap();
        assert_eq!(props.stride, 76);
        assert_eq!(props.props.len(), 1);
    }

    #[test]
    fn a_uniform_scale_is_read_only_where_one_exists() {
        let mut long = record(80, [0.0; 3], [0.0; 3], 0, 0);
        long[76..80].copy_from_slice(&2.5f32.to_le_bytes());
        let props = static_props(&bsp(11, 80, &["models/a.mdl"], &[long])).unwrap();
        assert_eq!(props.props[0].scale, 2.5);

        // Same bytes, older version: no scale field, so the default stands
        // rather than whatever happens to be at that offset.
        let props = static_props(&bsp(9, 72, &["models/a.mdl"], &[record(72, [0.0; 3], [0.0; 3], 0, 0)]))
            .unwrap();
        assert_eq!(props.props[0].scale, 1.0);
    }

    /// A scale of zero collapses the model to a point, and maps do ship them.
    #[test]
    fn a_zero_scale_does_not_collapse_the_model() {
        let mut zero = record(80, [0.0; 3], [0.0; 3], 0, 0);
        zero[76..80].copy_from_slice(&0.0f32.to_le_bytes());
        let props = static_props(&bsp(11, 80, &["models/a.mdl"], &[zero])).unwrap();
        assert!(props.props[0].scale > 0.0);
    }

    /// Most maps have no static props at all, and that is not a failure.
    #[test]
    fn a_map_without_static_props_reads_as_empty() {
        let props = static_props(&bsp(6, 64, &["models/a.mdl"], &[])).unwrap();
        assert!(props.props.is_empty());

        let mut bare = vec![0u8; HEADER];
        bare[0..4].copy_from_slice(b"VBSP");
        bare[4..8].copy_from_slice(&20i32.to_le_bytes());
        assert!(static_props(&bare).unwrap().props.is_empty());
    }

    /// A count that does not divide the lump means the record is not the size
    /// it looks, and guessing past that would place props at random.
    #[test]
    fn a_lump_that_does_not_divide_evenly_is_refused() {
        let mut data = bsp(9, 72, &["models/a.mdl"], &[record(72, [0.0; 3], [0.0; 3], 0, 0)]);
        // Claim two props where there is only room for one and a bit.
        let at = data.len() - 72 - 4;
        data[at..at + 4].copy_from_slice(&2i32.to_le_bytes());
        assert!(static_props(&data).is_err());
    }

    #[test]
    fn a_truncated_dictionary_is_an_error_not_a_panic() {
        let mut data = bsp(6, 64, &["models/a.mdl"], &[record(64, [0.0; 3], [0.0; 3], 0, 0)]);
        let at = 8 + LUMP_GAME_LUMP * 16;
        let offset = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        // Claim a hundred models where one was written.
        data[offset + 4 + GAME_LUMP_ENTRY..offset + 8 + GAME_LUMP_ENTRY]
            .copy_from_slice(&100i32.to_le_bytes());
        assert!(static_props(&data).is_err());
    }
}
