//! Reading `.vmt` material definitions.
//!
//! A VMT is a small VDF document: a shader name, then keys. All this needs
//! from it is which texture to draw and whether the surface is see-through:
//!
//! ```text
//! "LightmappedGeneric"
//! {
//!     "$basetexture" "Concrete/concretewall001a"
//!     "$surfaceprop" "concrete"
//! }
//! ```
//!
//! It is scanned rather than parsed into typed shaders. Source has dozens of
//! shaders and mods add their own, so a typed parser turns an unrecognised
//! shader into a missing texture; scanning for four keys cannot. It also has
//! to cope with `patch`, which is what the compiler writes into a map's
//! pakfile for every cubemap-lit surface, and which no VMT library models:
//!
//! ```text
//! "patch"
//! {
//!     "include" "materials/concrete/concretewall001a.vmt"
//!     "insert" { "$envmap" "env_cubemap" }
//! }
//! ```

use crate::source::vfs::Vfs;
use std::collections::HashMap;

/// How deep an `include` chain may go before we assume it is a cycle.
const MAX_INCLUDE_DEPTH: usize = 8;

/// What a material tells us about its appearance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialAssets {
    /// `$basetexture`, relative to `materials/` and without an extension.
    pub base_texture: String,
    /// The surface is see-through where its alpha channel says so.
    pub alpha_test: bool,
    /// The surface is blended, like glass.
    pub translucent: bool,
    /// `$surfaceprop`, e.g. `concrete` or `metalgrate`.
    pub surface_prop: Option<String>,
}

/// A parsed VMT: its shader and every scalar key found in it.
#[derive(Debug, Default, Clone)]
pub struct Vmt {
    pub shader: String,
    /// Keys lowercased; values as written. Keys inside `replace`/`insert`
    /// blocks override the ones they patch, so later wins.
    pub keys: HashMap<String, String>,
}

impl Vmt {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }

    fn flag(&self, key: &str) -> bool {
        // Source treats any non-zero value as set, and mappers write `1`,
        // `"1"` and occasionally `.5` for blend factors.
        self.get(key)
            .map(|v| v.trim_matches('"'))
            .is_some_and(|v| !v.is_empty() && v != "0")
    }

    pub fn is_patch(&self) -> bool {
        self.shader == "patch"
    }
}

/// Scan a VMT into its shader name and scalar keys.
pub fn parse(text: &str) -> Vmt {
    let mut vmt = Vmt::default();
    let mut tokens = tokenize(text).into_iter().peekable();

    // The first token is the shader, unless the file opens straight into a
    // block, which some hand-written materials do.
    if let Some(first) = tokens.peek()
        && first != "{"
    {
        vmt.shader = first.to_ascii_lowercase();
        tokens.next();
    }

    while let Some(token) = tokens.next() {
        if token == "{" || token == "}" {
            continue;
        }
        match tokens.peek() {
            // A key followed by a block: descend into it, so that `replace`
            // and `insert` contents land in the same flat map and override
            // whatever they were patching.
            Some(next) if next == "{" => continue,
            Some(_) => {
                let value = tokens.next().unwrap_or_default();
                vmt.keys.insert(token.to_ascii_lowercase(), value);
            }
            None => break,
        }
    }
    vmt
}

/// Split VDF text into tokens, dropping comments and platform conditionals.
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if c == b'[' {
            // `$basetexture "x" [!$X360]` — a conditional we never satisfy or
            // reject, so it is simply not a token.
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }
            i += 1;
        } else if c == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            tokens.push(String::from_utf8_lossy(&bytes[start..i]).into_owned());
            i += 1;
        } else if c == b'{' || c == b'}' {
            tokens.push((c as char).to_string());
            i += 1;
        } else {
            let start = i;
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && !matches!(bytes[i], b'{' | b'}' | b'"' | b'[')
            {
                i += 1;
            }
            tokens.push(String::from_utf8_lossy(&bytes[start..i]).into_owned());
        }
    }
    tokens
}

/// Resolves materials against a search path, following `patch` includes.
pub struct Materials<'a> {
    vfs: &'a Vfs,
    /// The map's own pakfile, tried first: it holds the compiler's patches.
    pak: Option<&'a vbsp::Packfile>,
}

impl<'a> Materials<'a> {
    pub fn new(vfs: &'a Vfs, pak: Option<&'a vbsp::Packfile>) -> Materials<'a> {
        Materials { vfs, pak }
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(pak) = self.pak {
            // Pakfile entries are stored lowercase with forward slashes.
            let key = path.to_ascii_lowercase().replace('\\', "/");
            if let Ok(Some(data)) = pak.get(&key) {
                return Some(data);
            }
        }
        self.vfs.open(path)
    }

    /// Resolve a material name such as `concrete/concretewall001a`.
    ///
    /// `raw_name` is the name as the BSP stored it, which for a patched
    /// material is the pakfile stub; it is only consulted if the authored
    /// path is missing, since the stub adds a cubemap and nothing else.
    pub fn assets(&self, name: &str, raw_name: Option<&str>) -> Option<MaterialAssets> {
        let vmt = self.load(name, 0).or_else(|| {
            raw_name
                .filter(|r| *r != name)
                .and_then(|r| self.load(r, 0))
        })?;

        let base_texture = vmt
            .get("$basetexture")
            // Blend materials paint two textures across a displacement; the
            // first is the one the surface mostly reads as.
            .or_else(|| vmt.get("$basetexture2"))
            .map(|v| v.trim().trim_matches('"').replace('\\', "/"))
            .filter(|v| !v.is_empty())?;

        Some(MaterialAssets {
            base_texture,
            alpha_test: vmt.flag("$alphatest"),
            translucent: vmt.flag("$translucent"),
            surface_prop: vmt
                .get("$surfaceprop")
                .map(|v| v.trim_matches('"').to_string()),
        })
    }

    /// Load a VMT and merge in whatever it patches.
    fn load(&self, name: &str, depth: usize) -> Option<Vmt> {
        if depth > MAX_INCLUDE_DEPTH {
            return None;
        }
        let path = format!("materials/{}.vmt", name.trim_end_matches(".vmt"));
        let data = self.read(&path)?;
        let vmt = parse(&String::from_utf8_lossy(&data));

        let Some(include) = vmt.get("include") else {
            return Some(vmt);
        };

        // A patch's own keys are the overrides, so the included material is
        // loaded first and then written over.
        let included = include
            .trim_matches('"')
            .trim_start_matches("materials/")
            .trim_end_matches(".vmt")
            .to_string();
        let Some(mut base) = self.load(&included, depth + 1) else {
            return Some(vmt);
        };
        for (key, value) in vmt.keys {
            if key != "include" {
                base.keys.insert(key, value);
            }
        }
        Some(base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_material() {
        let vmt = parse(
            r#"
            "LightmappedGeneric"
            {
                "$basetexture" "Concrete/concretewall001a"
                "$surfaceprop" "concrete"
                "$detailblendfactor" .6
            }
            "#,
        );
        assert_eq!(vmt.shader, "lightmappedgeneric");
        assert_eq!(vmt.get("$basetexture"), Some("Concrete/concretewall001a"));
        assert_eq!(vmt.get("$surfaceprop"), Some("concrete"));
        assert_eq!(vmt.get("$detailblendfactor"), Some(".6"));
    }

    /// Real materials mix quoted and bare tokens and use `//` comments.
    #[test]
    fn handles_unquoted_values_and_comments() {
        let vmt = parse(
            r#"
            UnlitGeneric
            {
                // this is a comment with "quotes" in it
                $basetexture metal/metalwall001a
                $alphatest 1
            }
            "#,
        );
        assert_eq!(vmt.shader, "unlitgeneric");
        assert_eq!(vmt.get("$basetexture"), Some("metal/metalwall001a"));
        assert!(vmt.flag("$alphatest"));
    }

    /// `[$X360]` style conditionals must not be mistaken for values.
    #[test]
    fn platform_conditionals_are_dropped() {
        let vmt =
            parse(r#""LightmappedGeneric" { "$basetexture" "a/b" [!$X360] "$alphatest" "1" }"#);
        assert_eq!(vmt.get("$basetexture"), Some("a/b"));
        assert!(vmt.flag("$alphatest"));
    }

    /// Proxies and other nested blocks must not swallow the keys after them.
    #[test]
    fn nested_blocks_do_not_hide_later_keys() {
        let vmt = parse(
            r#"
            "LightmappedGeneric"
            {
                "$basetexture" "a/b"
                "Proxies"
                {
                    "TextureScroll"
                    {
                        "textureScrollVar" "$basetexturetransform"
                    }
                }
                "$translucent" "1"
            }
            "#,
        );
        assert_eq!(vmt.get("$basetexture"), Some("a/b"));
        assert!(vmt.flag("$translucent"));
    }

    #[test]
    fn flags_treat_zero_and_absent_as_off() {
        let vmt = parse(r#""x" { "$alphatest" "0" }"#);
        assert!(!vmt.flag("$alphatest"));
        assert!(!vmt.flag("$translucent"));
    }

    #[test]
    fn a_patch_is_recognised() {
        let vmt = parse(
            r#"
            "patch"
            {
                "include" "materials/concrete/concretewall001a.vmt"
                "insert" { "$envmap" "env_cubemap" }
            }
            "#,
        );
        assert!(vmt.is_patch());
        assert_eq!(
            vmt.get("include"),
            Some("materials/concrete/concretewall001a.vmt")
        );
        // Keys inside `insert` are flattened, which is what makes a patch's
        // overrides win over the material it includes.
        assert_eq!(vmt.get("$envmap"), Some("env_cubemap"));
    }

    #[test]
    fn empty_input_is_harmless() {
        let vmt = parse("");
        assert!(vmt.shader.is_empty());
        assert!(vmt.keys.is_empty());
    }

    // --- against real game content ---

    fn real_materials() -> Option<(Vfs, ())> {
        let map = std::path::Path::new(
            "/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/d1_trainstation_02.bsp",
        );
        map.exists().then(|| (Vfs::for_map(map, &[]), ()))
    }

    #[test]
    fn resolves_real_half_life_2_materials() {
        let Some((vfs, _)) = real_materials() else {
            return;
        };
        let materials = Materials::new(&vfs, None);

        let concrete = materials.assets("concrete/concretewall001a", None).unwrap();
        assert_eq!(concrete.base_texture, "Concrete/concretewall001a");
        assert_eq!(concrete.surface_prop.as_deref(), Some("concrete"));
        assert!(!concrete.alpha_test);

        // A grate is alpha-tested, which is how we know to make it cutout.
        let grate = materials.assets("metal/metalgrate011a", None).unwrap();
        assert!(grate.alpha_test, "metalgrate011a should be alpha tested");

        // Blend materials name two textures; the first is enough.
        let blend = materials
            .assets("nature/blendgrassgravel001a", None)
            .unwrap();
        assert_eq!(blend.base_texture, "nature/dirtfloor006a");
    }

    #[test]
    fn a_missing_material_resolves_to_nothing() {
        let Some((vfs, _)) = real_materials() else {
            return;
        };
        let materials = Materials::new(&vfs, None);
        assert!(materials.assets("nothing/at/all", None).is_none());
    }
}
