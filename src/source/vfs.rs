//! Finding Source's content files.
//!
//! A material's texture is almost never in the map. A BSP's embedded pakfile
//! holds the cubemap patch stubs the compiler generated and little else —
//! `az_c4_4` ships 22 `.vmt` and 5 `.vtf` — while the textures those stubs
//! point at live in the game's VPK archives, or loose in a mod's `materials/`
//! folder. Reading a map's real textures therefore means reconstructing the
//! engine's own search path.
//!
//! Source describes that path in `gameinfo.txt`, so that is what we read. The
//! one place it cannot be followed literally is a mod mounting its base game:
//! Entropy: Zero's `gameinfo.txt` lists `hl2`, but resolves it through Steam
//! rather than as a sibling directory, so those names are also probed against
//! the Half-Life 2 install in the same Steam library.


use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One place content can be read from.
enum Source {
    /// A loose directory, indexed case-insensitively.
    Dir { root: PathBuf, files: HashMap<String, PathBuf> },
    /// A VPK archive, keyed case-insensitively.
    Vpk { path: PathBuf, archive: vpk::VPK, keys: HashMap<String, String> },
}

impl Source {
    fn open(&self, key: &str) -> Option<Vec<u8>> {
        match self {
            Source::Dir { files, .. } => files.get(key).and_then(|p| std::fs::read(p).ok()),
            Source::Vpk { archive, keys, .. } => {
                let entry = archive.tree.get(keys.get(key)?)?;
                entry.get().ok().map(|data| data.into_owned())
            }
        }
    }

    fn label(&self) -> String {
        match self {
            Source::Dir { root, .. } => root.display().to_string(),
            Source::Vpk { path, .. } => path.display().to_string(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Source::Dir { files, .. } => files.len(),
            Source::Vpk { keys, .. } => keys.len(),
        }
    }
}

/// An ordered content search path.
#[derive(Default)]
pub struct Vfs {
    sources: Vec<Source>,
}

/// Source paths are written in whatever case the mapper typed, VPK trees are
/// lowercase, and Linux filesystems are case-sensitive. Everything is compared
/// lowercased with forward slashes.
fn key(path: &str) -> String {
    path.to_ascii_lowercase().replace('\\', "/")
}

impl Vfs {
    /// Build the search path for a map: its game directory, whatever
    /// `gameinfo.txt` mounts, plus any extra directories given.
    pub fn for_map(map_path: &Path, extra_dirs: &[PathBuf]) -> Vfs {
        let mut mounts: Vec<Mount> = extra_dirs.iter().cloned().map(Mount::Dir).collect();

        if let Some(game) = game_dir(map_path) {
            mounts.push(Mount::Dir(game.clone()));
            mounts.extend(mounted(&game));
        }

        let mut vfs = Vfs::default();
        for mount in mounts {
            match mount {
                Mount::Dir(root) => {
                    vfs.add_dir(&root);
                    vfs.add_vpks(&root);
                }
                Mount::Vpk(path) => vfs.add_vpk(&path),
            }
        }
        vfs
    }

    /// Index a game directory's loose `materials/` tree.
    pub fn add_dir(&mut self, root: &Path) {
        let materials = root.join("materials");
        if !materials.is_dir() {
            return;
        }
        let mut files = HashMap::new();
        index_dir(&materials, root, &mut files);
        if !files.is_empty() {
            self.sources.push(Source::Dir { root: root.to_path_buf(), files });
        }
    }

    /// Mount every VPK archive in a game directory.
    ///
    /// Multi-part archives are split into `name_dir.vpk` plus numbered data
    /// files; only the directory half is an index, so opening `name_000.vpk`
    /// would find nothing.
    pub fn add_vpks(&mut self, root: &Path) {
        let Ok(entries) = std::fs::read_dir(root) else { return };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("vpk")))
            .filter(|p| {
                let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                // Keep `x_dir.vpk`, and standalone `x.vpk`, but never `x_000.vpk`.
                stem.ends_with("_dir")
                    || !stem
                        .rsplit_once('_')
                        .is_some_and(|(_, tail)| tail.len() == 3 && tail.bytes().all(|b| b.is_ascii_digit()))
            })
            .collect();
        paths.sort();
        for path in paths {
            self.add_vpk(&path);
        }
    }

    /// Mount one VPK archive.
    pub fn add_vpk(&mut self, path: &Path) {
        if self.sources.iter().any(|s| matches!(s, Source::Vpk { path: p, .. } if p == path)) {
            return;
        }
        if let Ok(archive) = vpk::from_path(path) {
            let keys = archive.tree.keys().map(|k| (key(k), k.clone())).collect();
            self.sources.push(Source::Vpk { path: path.to_path_buf(), archive, keys });
        }
    }

    /// Read a content-relative path such as `materials/concrete/wall.vtf`.
    pub fn open(&self, path: &str) -> Option<Vec<u8>> {
        let key = key(path);
        self.sources.iter().find_map(|source| source.open(&key))
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Where content will be looked for, in order, for the `textures` report.
    pub fn describe(&self) -> Vec<String> {
        self.sources
            .iter()
            .map(|s| format!("{} ({} files)", s.label(), s.len()))
            .collect()
    }
}

/// Walk a directory tree, keying every file by its path relative to `base`.
fn index_dir(dir: &Path, base: &Path, out: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            index_dir(&path, base, out);
        } else if let Ok(relative) = path.strip_prefix(base) {
            out.insert(key(&relative.to_string_lossy()), path);
        }
    }
}

/// The game directory a map belongs to: the parent of its `maps/` folder.
pub fn game_dir(map_path: &Path) -> Option<PathBuf> {
    let maps = map_path.parent()?;
    if !maps.file_name().is_some_and(|n| n.eq_ignore_ascii_case("maps")) {
        return None;
    }
    Some(maps.parent()?.to_path_buf())
}

/// One thing `gameinfo.txt` asks to be mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mount {
    Dir(PathBuf),
    Vpk(PathBuf),
}

/// Everything `gameinfo.txt` mounts, resolved to paths that exist.
fn mounted(game: &Path) -> Vec<Mount> {
    let Ok(text) = std::fs::read_to_string(game.join("gameinfo.txt")) else {
        return Vec::new();
    };

    // `|all_source_engine_paths|` is the directory holding the game folders.
    let engine_root = game.parent().unwrap_or(game).to_path_buf();
    // A mod may instead mount its base game through Steam, in which case the
    // sibling directory does not exist and the Steam library is where to look.
    let steam_hl2 = steam_library(game).map(|lib| lib.join("Half-Life 2"));

    let mut found = Vec::new();
    for value in search_path_values(&text) {
        for mount in resolve_search_path(&value, game, &engine_root, steam_hl2.as_deref()) {
            let path = match &mount {
                Mount::Dir(p) | Mount::Vpk(p) => p,
            };
            if path != game && !found.contains(&mount) {
                found.push(mount);
            }
        }
    }
    found
}

/// Turn one `SearchPaths` value into the paths it names, keeping only those
/// that exist.
fn resolve_search_path(
    value: &str,
    game: &Path,
    engine_root: &Path,
    steam_hl2: Option<&Path>,
) -> Vec<Mount> {
    // `|gameinfo_path|` is the game directory; `|all_source_engine_paths|` the
    // directory above it. Anything else is relative to the engine root.
    let (bases, rest): (Vec<PathBuf>, &str) = if let Some(rest) = value.strip_prefix("|gameinfo_path|") {
        (vec![game.to_path_buf()], rest)
    } else if let Some(rest) = value.strip_prefix("|all_source_engine_paths|") {
        (vec![engine_root.to_path_buf()], rest)
    } else if value.contains('|') {
        return Vec::new();
    } else {
        let mut bases = vec![engine_root.to_path_buf()];
        bases.extend(steam_hl2.map(Path::to_path_buf));
        (bases, value)
    };

    let rest = rest.trim_matches('/');
    if rest.is_empty() || rest == "." {
        return Vec::new();
    }

    let mut out = Vec::new();
    for base in bases {
        // A trailing `*` means "every subdirectory of this", which is how
        // `custom/` add-ons and Entropy: Zero 2's own `ez2/*` are mounted.
        if let Some(parent) = rest.strip_suffix("/*").or_else(|| rest.strip_suffix('*')) {
            let dir = base.join(parent.trim_end_matches('/'));
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let mut children: Vec<PathBuf> =
                    entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                children.sort();
                out.extend(children.into_iter().map(Mount::Dir));
            }
            // `ez2/*` also mounts `ez2` itself in practice, since Source
            // treats the wildcard as "this and its children".
            if dir.is_dir() {
                out.push(Mount::Dir(dir));
            }
        } else {
            let path = base.join(rest);
            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("vpk")) {
                // A multi-part archive is indexed by its `_dir` half.
                let indexed = dir_vpk(&path);
                if indexed.is_file() {
                    out.push(Mount::Vpk(indexed));
                }
            } else if path.is_dir() {
                out.push(Mount::Dir(path));
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// `hl2_textures.vpk` names a multi-part archive whose index is
/// `hl2_textures_dir.vpk`; a single-part archive is itself.
fn dir_vpk(path: &Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    path.with_file_name(format!("{stem}_dir.vpk"))
}

/// The values in `gameinfo.txt`'s `SearchPaths` block.
fn search_path_values(gameinfo: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut in_block = false;
    let mut depth = 0;

    for line in gameinfo.lines() {
        // Quotes are optional throughout VDF, and mods differ: Half-Life 2
        // writes `SearchPaths`, Entropy: Zero 2 writes `"SearchPaths"`.
        let line = line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if !in_block {
            if line.trim_matches('"').eq_ignore_ascii_case("searchpaths") {
                in_block = true;
            }
            continue;
        }
        if line.starts_with('{') {
            depth += 1;
            continue;
        }
        if line.starts_with('}') {
            depth -= 1;
            if depth <= 0 {
                break;
            }
            continue;
        }

        // `"game+mod"  "hl2/custom/*"` — the value is the last field.
        let Some(value) = line.split_whitespace().next_back() else { continue };
        let value = value.trim_matches('"').replace('\\', "/");
        if !value.is_empty() && !values.contains(&value) {
            values.push(value);
        }
    }
    values
}

/// The `steamapps/common` directory a path sits under, if any.
fn steam_library(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(dir) = current {
        if dir.file_name().is_some_and(|n| n.eq_ignore_ascii_case("common"))
            && dir.parent().is_some_and(|p| {
                p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
            })
        {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const HL2: &str = "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2";
    const EZ: &str =
        "/mnt/games/SteamLibrary/steamapps/common/Entropy Zero/Entropy Zero/EntropyZero";

    fn hl2_map() -> Option<PathBuf> {
        let path = PathBuf::from(HL2).join("maps/d1_trainstation_02.bsp");
        path.exists().then_some(path)
    }

    fn ez_map() -> Option<PathBuf> {
        let path = PathBuf::from(EZ).join("maps/az_c4_4.bsp");
        path.exists().then_some(path)
    }

    #[test]
    fn keys_ignore_case_and_slash_direction() {
        assert_eq!(key("Materials\\Concrete\\Wall.VTF"), "materials/concrete/wall.vtf");
    }

    #[test]
    fn a_maps_game_directory_is_the_parent_of_maps() {
        let game = game_dir(Path::new("/games/hl2/maps/d1_town_01.bsp")).unwrap();
        assert_eq!(game, Path::new("/games/hl2"));
        // A path not inside a `maps` folder has no game directory.
        assert!(game_dir(Path::new("/games/hl2/d1_town_01.bsp")).is_none());
    }

    #[test]
    fn gameinfo_search_paths_are_read_verbatim() {
        let gameinfo = r#"
            "GameInfo"
            {
                FileSystem
                {
                    SearchPaths
                    {
                        // a comment naming ep2 should be ignored
                        game+mod            ep2/custom/*
                        game+mod+default_write_path      |gameinfo_path|.
                        game                hl2/hl2_textures.vpk
                        game                episodic
                    }
                }
            }
        "#;
        assert_eq!(
            search_path_values(gameinfo),
            vec!["ep2/custom/*", "|gameinfo_path|.", "hl2/hl2_textures.vpk", "episodic"]
        );
    }

    /// Quoting is optional in VDF and mods differ: Half-Life 2 writes
    /// `SearchPaths`, Entropy: Zero 2 writes `"SearchPaths"`. Missing the
    /// quoted form finds no content at all.
    #[test]
    fn a_quoted_block_name_is_recognised() {
        let gameinfo = r#"
            "SearchPaths"
            {
                "game+mod" "|gameinfo_path|ez2/*"
                "game+mod" "|all_source_engine_paths|mapbase/hl2/*"
            }
        "#;
        assert_eq!(
            search_path_values(gameinfo),
            vec!["|gameinfo_path|ez2/*", "|all_source_engine_paths|mapbase/hl2/*"]
        );
    }

    /// The block must end at its own closing brace, not swallow the rest.
    #[test]
    fn parsing_stops_at_the_end_of_the_block() {
        let gameinfo = r#"
            SearchPaths
            {
                game    hl2
            }
            SomethingElse
            {
                game    should_not_appear
            }
        "#;
        assert_eq!(search_path_values(gameinfo), vec!["hl2"]);
    }

    #[test]
    fn a_multipart_vpk_resolves_to_its_directory_half() {
        let path = Path::new("/games/hl2/hl2_textures.vpk");
        // The bare name does not exist, so the `_dir` half is used.
        assert_eq!(dir_vpk(path), Path::new("/games/hl2/hl2_textures_dir.vpk"));
    }

    #[test]
    fn a_steam_library_is_found_by_walking_up() {
        let lib = steam_library(Path::new("/x/SteamLibrary/steamapps/common/Game/mod/maps"));
        assert_eq!(lib.unwrap(), Path::new("/x/SteamLibrary/steamapps/common"));
        assert!(steam_library(Path::new("/home/user/maps")).is_none());
    }

    #[test]
    fn finds_half_life_2_content() {
        let Some(map) = hl2_map() else { return };
        let vfs = Vfs::for_map(&map, &[]);
        assert!(!vfs.is_empty(), "no content sources found");

        // A texture every Half-Life 2 map uses, which lives in the VPKs.
        let data = vfs
            .open("materials/concrete/concretewall001a.vmt")
            .expect("concretewall001a.vmt should be in hl2_textures");
        assert!(!data.is_empty());
        assert!(vfs.open("materials/concrete/concretewall001a.vtf").is_some());
    }

    /// Source material paths are stored in whatever case the mapper used.
    #[test]
    fn lookups_ignore_case() {
        let Some(map) = hl2_map() else { return };
        let vfs = Vfs::for_map(&map, &[]);
        assert!(vfs.open("MATERIALS/CONCRETE/CONCRETEWALL001A.VTF").is_some());
        assert!(vfs.open("materials\\concrete\\concretewall001a.vtf").is_some());
    }

    /// Entropy: Zero keeps its own content loose and mounts Half-Life 2's
    /// through Steam, so both have to resolve from one map's search path.
    #[test]
    fn a_mod_reaches_both_its_own_content_and_its_base_game() {
        let Some(map) = ez_map() else { return };
        let vfs = Vfs::for_map(&map, &[]);
        assert!(
            vfs.open("materials/az_skybox/mun_glow.vtf").is_some(),
            "Entropy: Zero's own loose materials should be found"
        );
        assert!(
            vfs.open("materials/concrete/concretewall001a.vtf").is_some(),
            "Half-Life 2's textures should be reachable from an E:Z map"
        );
    }

    #[test]
    fn a_missing_file_is_none_rather_than_an_error() {
        let Some(map) = hl2_map() else { return };
        let vfs = Vfs::for_map(&map, &[]);
        assert!(vfs.open("materials/nothing/at/all.vtf").is_none());
    }

    #[test]
    fn an_empty_search_path_is_harmless() {
        let vfs = Vfs::default();
        assert!(vfs.is_empty());
        assert!(vfs.open("materials/x.vtf").is_none());
        assert!(vfs.describe().is_empty());
    }
}
