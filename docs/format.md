# src2mc mod interchange format

Status: **version 1 work in progress**. Sections 1–12 are normative. Their
writer-side contracts are implemented by the converter and the Phase 2 mod
reader validates them. Rendering consumers are implemented in later phases.
The abandoned prototype is not an input format and has no migration path.

## 1. Container and names

A campaign bundle is a ZIP file with the extension `.src2mc`. Entry names are
UTF-8, relative, forward-slash-separated paths. They are at most 240 UTF-8
bytes. Empty components, `.`, `..`, backslashes, control characters, drive or
URI-style colon components, leading slashes, and trailing slashes are invalid.
Duplicate names are invalid after exact UTF-8 comparison.

`manifest.json` is the only bootstrap entry and is reserved. Directory entries
are unnecessary. Readers must use names, never ZIP entry order.

## 2. Canonical payloads

JSON owned by this format is UTF-8 without a byte-order mark, uses the declared
field order and lexicographically sorted object maps, contains no insignificant
whitespace, and ends in exactly one LF. Strings use JSON escaping as emitted by
Serde JSON. Optional fields are governed by each payload schema; exporters may
not interchange missing and `null`.

Numbers use JSON's shortest round-tripping decimal representation. Format
floats are finite IEEE-754 binary64 values. NaN and infinities are errors;
negative zero is normalized to positive zero before encoding. Payload-specific
schemas may narrow these rules. Binary integers and IEEE-754 fields are
little-endian. Binary payloads begin with their own magic and version rather
than relying only on the manifest version.

PNG texture bytes and binary mesh bytes are already their canonical payloads.
An exporter must control their encoder settings; decoded pixels or equivalent
triangles are not substituted when hashing.

## 3. Content IDs and hashes

`sha256` and all content IDs are 64 lowercase hexadecimal characters. A
content ID is SHA-256 over the exact canonical payload bytes and is used for
deduplicated assets, for example `textures/<id>.png` and `meshes/<id>.s2mesh`.
Human Source paths remain diagnostic provenance and never determine identity.

The manifest lists every payload entry except itself, sorted by entry path:

```json
{"format":"src2mc-campaign","version":1,"campaign_id":"hl2","fingerprint":"…","entries":[{"path":"maps/d1_trainstation_01.json","size":123,"sha256":"…"}]}
```

The fingerprint is SHA-256 over this unambiguous byte sequence:

1. ASCII `src2mc-manifest-v1` followed by a zero byte;
2. for each sorted entry: path byte length as `u32`, path UTF-8 bytes,
   uncompressed size as `u64`, and the 32 decoded hash bytes.

ZIP compression, entry order, timestamps, permissions, creator OS, extra
fields, comments, and the complete archive byte stream are outside identity.
Validation hashes uncompressed entry payloads and recomputes the fingerprint.

## 4. Writer requirements

The converter rejects a non-`.src2mc` output name, unsafe or duplicate entry
paths, a user-supplied `manifest.json`, non-finite format floats, and a
content-address collision whose bytes differ. It deduplicates identical
canonical content bytes. The current writer also emits sorted entries and
fixed permissions for reproducible diagnostics, but readers must not depend on
those incidental ZIP properties.

## 5. Map surface table

Each map's canonical visible faces are stored at
`maps/<map-id>/surfaces.s2faces`. The payload begins with this fixed little-endian
header:

| Field | Type | Value |
| --- | --- | --- |
| magic | 8 bytes | `S2FACE\0\0` |
| version | `u32` | 2 |
| UV-region count | `u32` | number of following UV records |
| section count | `u32` | number of following section buckets |
| face count | `u32` | total face records in all buckets |

UV regions follow in lexicographic order of their canonical IEEE-754 bit
patterns. Each consists of eight finite `f64` values: the four coefficients of
`s = ux*x + uy*y + uz*z + uoffset`, then the corresponding four coefficients
for `t`. Coordinates are map-local Minecraft block coordinates. Signed zero is
canonicalized to positive zero before deduplication and writing.

Each non-empty 16x16x16 map-local section then contains its signed `i32` X, Y,
and Z section coordinates, a `u32` face count, and that many fixed 20-byte face
records:

| Field | Type | Meaning |
| --- | --- | --- |
| local cell | `u16` | bits 0–3 X, 4–7 Z, 8–11 Y; high bits zero |
| patch | `u8` | bits 0–2 direction, 3–4 plane, 5 A-min, 6 B-min; bit 7 zero |
| provenance kind | `u8` | 0 brush side, 1 displacement triangle |
| material ID | `u32` | index into the map's material-reference table |
| UV-region ID | `u32` | index into this file's UV table |
| provenance primary | `u32` | brush or displacement index |
| provenance secondary | `u32` | side or triangle index |

Directions 0–5 are down, up, north, south, west, and east. Plane and A/B
coordinates are in half-block units 0–2. For up/down A is X and B is Z; for
north/south A is X and B is Y; for west/east A is Z and B is Y. A face is one
0.5x0.5 micro-patch: each tangential maximum is its encoded minimum plus one.
This directly preserves the converter's fitted-shape geometry; v1 does not
silently coalesce patches into larger rectangles.

Sections and faces use canonical coordinate/record order. `u32` references and
counts avoid a campaign-wide `u16` ceiling; loaders must additionally enforce
the defensive allocation limits in section 10.

## 6. Campaign and map metadata

`campaign.json` is canonical JSON with `format`
`src2mc-campaign-metadata`, `version` 1, the same `campaign_id` as the
manifest, and a `maps` array. Each map item contains its `map_id` and canonical
metadata path `maps/<map-id>.json`. Map items are sorted by map ID and IDs are
unique. Campaign and map IDs consist only of lowercase ASCII letters, digits,
`_` and `-` so they are portable path and Minecraft resource-path components.
Display/source names are separate and may contain other UTF-8 characters.

Each `maps/<map-id>.json` has `format` `src2mc-map`, `version` 1, and these
fields in order:

| Field | Meaning |
| --- | --- |
| `map_id` | stable ID matching the entry name |
| `source_name` | human-readable Source map provenance; not identity |
| `units_per_block` | finite positive conversion scale; 32 for mod export |
| `cell_min`, `cell_max` | inclusive map-local schematic cell bounds |
| `anchor_cell` | map-local storage cell of the authoritative anchor |
| `surfaces` | canonical `maps/<map-id>/surfaces.s2faces` path |
| `materials` | map-local material-reference array |
| `models` | unique model-reference array sorted by content ID, source path, then material IDs |
| `props` | canonical `maps/<map-id>/props.s2props` path |
| `pvs` | optional canonical `maps/<map-id>/pvs.s2pvs` path; absent when the map has no usable PVS |
| `diagnostics` | canonical `maps/<map-id>/diagnostics.json` path |

Surface-table material IDs index `materials` directly. This array therefore
retains the converter/BSP material order and is not sorted during encoding.
Each material record contains the normalized diagnostic `source_material`, an
optional differing `source_material_raw`, `render_class` (`solid`, `cutout`,
`translucent`, or `fallback`), an optional texture reference, optional Source
`surface_prop`, and three finite reflectivity values. Cutout takes precedence
when Source declares both alpha-test and translucency, matching Source's hard
alpha test rather than turning grates into blended panes.

A texture reference contains its 64-character lowercase SHA-256 logical-content ID
and positive `original_width`, `original_height`, `output_width`, and
`output_height`. Output dimensions record resampling rather than hiding it.
The optional campaign `atlas` field is exactly `atlas.json`; it is required when
any material has a texture. The logical ID is retained in `atlas.json`, while
pixels live only in content-addressed `atlas/<page-mip-content-id>.png` pages.
Thus the bundle does not duplicate a logical image as both standalone and atlas
payloads. `atlas.json` records the fixed 4096 page size, mip levels 0 through 4,
16-pixel base gutter, every page mip and its dimensions, and every logical
texture's lossless source-to-page rectangles. A model reference
similarly contains its content ID, diagnostic normalized Source model path,
and a `materials` array mapping mesh material slots to map-local material IDs.
Model bytes live at `meshes/<content-id>.s2mesh`.

Multiple model references may name the same content ID when their Source-model
provenance or map-local material-slot arrays differ. References are uniquely
sorted by `(content_id, source_model, materials)`; prop placement records select
the reference index. The mesh payload still occurs only once.

`maps/<map-id>/diagnostics.json` has format `src2mc-diagnostics`, version 1,
and a canonically sorted `messages` array. Each message has severity `info`,
`warning`, or `error`; a stable uppercase ASCII/digit/underscore code; a human
message; and a lexicographically keyed string context map. Human wording is
not a programmatic error code. An empty message array is valid.

## 7. Runtime prop mesh

Each deduplicated model mesh is `meshes/<content-id>.s2mesh`. Its little-endian
header contains the 8-byte magic `S2MESH\0\0`; `u32` version 1; `u32` vertex,
index, and submesh counts; then minimum and maximum bounds as six finite `f32`
values. Positions and bounds are model-local Minecraft block coordinates.

Each vertex is eight `f32` values: XYZ position, XYZ normal, and UV. Values are
finite, signed zero is normalized, and normals must be nonzero. The vertex
array is followed by `u32` triangle indices. Each following submesh contains
`u32` first-index, index-count, and material-slot fields. Ordered, contiguous,
nonempty submesh ranges cover the complete index buffer and contain whole
triangles. Material slots index the map model reference's `materials` array.
This preserves Source vertex normals; exporters must not silently replace them
with reconstructed flat triangle normals.

## 8. Prop placement table

Each map's roots are `maps/<map-id>/props.s2props`: magic `S2PROP\0\0`, `u32`
version 1, and `u32` record count, followed by fixed 112-byte records sorted by
stable ID:

| Field | Type | Meaning |
| --- | --- | --- |
| stable ID | 32 bytes | SHA-256-derived placement identity |
| model | `u32` | map model-reference index |
| root cell | 3 x `i32` | authoritative map-local storage cell |
| translation | 3 x `f64` | exact map-local visible model origin |
| rotation | 4 x `f64` | unit quaternion XYZW in Minecraft axes |
| scale | `f64` | finite positive uniform scale |

Stable IDs are unique and independent of nearest-free root-cell selection.
Moving the root therefore never moves the visible mesh or changes placement
identity. Signed-zero transform components are normalized when encoded.

## 9. Schematic anchor and prop-root NBT

Mod export writes Sponge schematic version 3. The complete map is one
schematic and its generic surface blocks use `src2mc:surface`. Its authoritative
map anchor and prop roots use `src2mc:map_anchor` and `src2mc:prop_root`.
The schematic bounds extend one cell below the converted map and the anchor is
placed at that new minimum-Y corner. It is therefore relative position
`[0,0,0]`, while never replacing map geometry.
`Blocks.BlockEntities` is a list of compounds in lexicographic XYZ position
order. Each compound uses WorldEdit 7.3.8's v3 envelope:

- `Id`: the namespaced block-entity type string;
- `Pos`: three `int`s relative to the schematic minimum, not world position;
- `Data`: the block entity's persistent payload.

Anchor `Data` contains `schema_version` int 1, `campaign_id`, `map_id`, and
`anchor_cell` as three map-local ints. The anchor cell is also recorded in map
metadata. This intentional redundancy lets validation reject a mismatched
schematic and lets the pasted world position establish the one supported
translation from map-local to world coordinates.

Prop-root `Data` contains `schema_version` int 1, `campaign_id`, `map_id`, the
64-lowercase-hex `stable_id`, `model_content_id`, map-local `root_cell`, exact
map-local `translation` as three doubles, unit-quaternion `rotation` as four
doubles in Minecraft XYZW axes, positive double `scale`, map-local
`material_ids` as an int array, and diagnostic `source_model`. Doubles are
finite and signed zero is canonicalized. The visible world transform is
derived from the pasted root position and `translation - root_cell`; moving or
copying a root therefore preserves the model's offset from its storage cell.
The bundle placement table remains the canonical batch copy of the same data.

Duplicate block-entity positions, positions outside the schematic bounds,
invalid identifiers, invalid references, non-finite transforms and non-unit
quaternions fail export. An anchor/root block and its matching block entity are
both required. Legacy display entities are never emitted by mod export.

## 10. Defensive limits and error codes

All counts and sizes are checked before allocating their complete payload.
These are hard corruption guards, distinct from configurable client RAM/VRAM
residency budgets:

| Resource | v1 maximum |
| --- | ---: |
| ZIP entries | 100,000 |
| entry path | 240 UTF-8 bytes |
| manifest bootstrap payload | 64 MiB |
| maps | 4,096 |
| JSON nesting | 64 |
| materials per map | 1,000,000 |
| models per campaign | 1,000,000 |
| props per map | 10,000,000 |
| UV regions per map | 10,000,000 |
| nonempty sections per map | 4,000,000 |
| face records per map | 100,000,000 |
| vertices / indices / submeshes per mesh | 10,000,000 / 30,000,000 / 65,536 |
| original texture axis | 16,384 |
| output texture axis | 4,096 |
| decoded bytes per texture | 256 MiB |
| PVS clusters per map | 65,536 |
| PVS leaves per map | 4,000,000 |
| PVS bitset payload | 256 MiB |
| uncompressed entry | 2 GiB |
| uncompressed bundle payload | 64 GiB |
| ZIP expansion ratio | 200:1 after a 1 MiB small-entry allowance |
| total runtime allocation | 16 GiB |

The reader additionally detects integer overflow and verifies that accumulated
sizes never exceed their enclosing entry. A lower configured runtime or
texture-residency budget may reject otherwise structurally valid content.

Stable machine error codes are `UNSAFE_PATH`, `LIMIT_EXCEEDED`,
`ZIP_EXPANSION_LIMIT`, `HASH_MISMATCH`, `MISSING_ENTRY`,
`UNSUPPORTED_VERSION`, `INVALID_SCHEMA`, `INVALID_REFERENCE`,
`DUPLICATE_IDENTITY`, `NO_FREE_PROP_ROOT`, and `SCHEMATIC_TOO_LARGE`.
Diagnostics may add context and human wording without changing these tokens.

## 11. Phase 1 export lock

The schemas above, single-map and batch `mod export` commands, deterministic
synthetic fixture, and local real-map stability check are complete. Texture
payload creation is owned by Phase 4. Mod export now resolves world materials,
records original and effective dimensions, and writes deduplicated logical PNG
payloads. Replacing those temporary standalone payloads with the final paged
representation remains an atomic format step so readers never accept an
unreferenced or half-migrated atlas.

## 12. Prop visibility table

`maps/<map-id>/pvs.s2pvs` optionally records the Source BSP visibility
clusters (PVS) that static-prop rendering uses to reject map regions the
camera cannot see. Map metadata references it through the optional `pvs`
field; a map without usable visibility data simply omits both, and readers
must render unfiltered in that case. The payload begins with this fixed
little-endian header:

| Field | Type | Value |
| --- | --- | --- |
| magic | 8 bytes | `S2PVIS\0\0` |
| version | `u32` | 1 |
| cluster count | `u32` | PVS rows and cluster IDs |
| row bytes | `u32` | `(cluster count + 7) / 8` |
| node count | `u32` | following BSP node records |
| root node | `i32` | world-model BSP root index |
| section count | `u32` | following section records |

BSP node records follow in original BSP node order. Each is 24 bytes: four
finite `f32` values `(normal_x, normal_y, normal_z, distance)` for the
map-local plane `normal · point = distance`, then two `i32` children. A
non-negative child is another node index. A negative child encodes a leaf
cluster as `-1 - cluster`; `i32::MIN` denotes a solid or outside leaf. The
runtime walks this tree for the camera point, selecting child zero for a
positive plane distance and child one for a negative distance. A point on a
plane, invalid tree, or terminal solid leaf fails open. This exact traversal
is necessary: outward-rounded leaf AABBs overlap and cannot safely select a
camera cluster.

Section records follow in lexicographic order of their map-local 16-block
section coordinates: three `i32` values, a `u32` cluster count, and that many
ascending `u16` cluster IDs. Each record lists every leaf cluster whose
outward-rounded block-space box intersects the section box. Sections with an
empty set are omitted; a section absent from the table must fail open. The
converter derives the table from the transformed bounds of the map's props,
so every runtime prop section is covered.

The payload ends with `cluster count` PVS rows of `row bytes` each. Bit `c`
of row `k` (LSB-first within each byte) set means cluster `c` is potentially
visible from cluster `k`. The rows use the Quake II run-length scheme as
decoded from the BSP: a nonzero byte copies eight clusters, and a `0` byte
followed by a count skips that many zero bytes. An aggregate batch whose
section intersects at least one visible cluster renders; otherwise it is
rejected for the frame. Every row must include its own cluster; a table that
violates this integrity invariant is invalid. A camera cluster of `-1`, a
missing tree, or any other unresolved lookup renders without rejection.
