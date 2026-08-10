# src2mc interchange format, version 1

This document is the contract between the converter in this repository and the
companion mod in `mod/`. It is normative: where this document and either
implementation disagree, this document is right and the implementation has a
bug.

Both sides carry the version number as a constant — `src2mc::FORMAT_VERSION` in
`src/lib.rs` and `Src2mc.FORMAT_VERSION` in
`mod/src/main/java/io/github/theredja/src2mc/Src2mc.java`. They must be equal.
Changing anything described here means, in one commit:

1. bumping both constants,
2. regenerating the fixtures under `tests/fixtures` (`UPDATE_FIXTURES=1 cargo
   test --test fixtures`),
3. updating this document,
4. adding a line to `docs/decisions.md` if the change settles a question rather
   than just moving bytes around.

Nothing else counts as an interface change. A change to Rust internals, to the
KubeJS output, or to the mod's rendering is not an interface change and must not
bump the version.

## 0. Status

Not all of this is built. Each section is marked:

- **shipped** — implemented, exercised by a fixture, safe to rely on.
- **specified** — agreed shape, not yet written on either side. Implement it as
  written; if it cannot be implemented as written, change this document first.

Version 1 is the first version, so the whole document describes what the mod
era looks like, and most of it is still **specified**. Output produced before
this document existed carries no `src2mc:FormatVersion` key at all; treat a
missing key as version 0 and refuse it with a message telling the user to
re-run the converter.

## 1. Transport: the schematic — **shipped** except where noted

Output is Sponge Schematic v3, gzipped, as written by `src/output/schem.rs` and
read by WorldEdit 7.3.8. The root compound holds one key, `Schematic`, with:

| Field | Type | Meaning |
| --- | --- | --- |
| `Version` | int | Always 3. Sponge schematic version, not ours. |
| `DataVersion` | int | 3955 (Minecraft 1.21.1). |
| `Width`, `Height`, `Length` | short | Region size. Read as unsigned. |
| `Offset` | int[3] | Region minimum in world space. |
| `Blocks.Palette` | compound | Block state string to palette index. |
| `Blocks.Data` | byte[] | Palette indices, unsigned LEB128 varints, ordered x fastest, then z, then y. |
| `Blocks.BlockEntities` | list | Prop placements. **specified**, see §3. |
| `Entities` | list | Display entities. Legacy, absent in mod-era output. |
| `Metadata.Name` | string | Map or tile name. |
| `Metadata.Author` | string | Always `src2mc`. |
| `Metadata.src2mc:FormatVersion` | int | The version this document describes. |

The mod must read `Metadata.src2mc:FormatVersion` before anything else and stop
with a clear message naming both versions if it does not match.

A map larger than one schematic is split into tiles; every tile carries the same
metadata and its own `Offset`. Tiles are independent — a prop never spans two of
them, because a placement is a single block entity at its anchor cell.

## 2. Surfaces — **shipped** on the converter side, **specified** on the mod side

World geometry is plain blocks from a fixed pool registered by the mod:

```
src2mc:surface_0 … src2mc:surface_<POOL_SIZE-1>
```

The pool size is a compile-time constant in the mod, is not content-dependent,
and never changes with the maps converted. A surface block has no block entity.
Its appearance and its collision come from the material table (§4) entry with
the same index, resolved at chunk bake time.

The converter assigns indices per bundle, densely from 0, and writes the mapping
into the material table. Running out of indices is a hard error naming the map
and the count; it must never silently reuse an index.

Blocks outside the pool (`minecraft:air`, `minecraft:barrier`, and anything the
converter places deliberately) appear in the palette as themselves and mean what
vanilla means.

## 3. Prop placements — **specified**

A prop is one entry in `Blocks.BlockEntities`. Sponge v3 requires `Id` and
`Pos`; everything else lives under `Data`.

| Key | Type | Meaning |
| --- | --- | --- |
| `Id` | string | Always `src2mc:prop`. |
| `Pos` | int[3] | Anchor cell, relative to the region, not the world. |
| `Data.model` | int | Index into the model table (§4). |
| `Data.rotation` | float[4] | Rotation as a quaternion, `x, y, z, w`. |
| `Data.offset` | float[3] | Offset from the anchor cell's minimum corner, in blocks. Each component is in `[0, 1)`. |
| `Data.scale` | float | Uniform scale. `1.0` means one Source unit maps at the bundle's configured scale. |
| `Data.id` | string | Stable identity, unique within a bundle. Survives editing. |
| `Data.flags` | int | Bit 0: collision enabled. Bits 1-31 reserved, must be written as 0 and ignored on read. |

Only the anchor cell appears in the schematic. Carrier cells — the extra blocks
the mod places so geometry stays inside Sodium's vertex range, and so collision
can be served per cell — are derived state. The mod computes them when the prop
is baked, removes them when it moves, and never writes them to a schematic. A
converter must not emit them and a schematic containing them is malformed.

An unknown `Data.model` is an error the mod surfaces once per bundle, not per
placement, and the prop renders as a marker rather than crashing the chunk bake.

## 4. The bundle — **shipped** for materials, **specified** for models

A bundle is a directory the mod mounts as a resource and data source. One bundle
covers a campaign, not a map.

```
<bundle>/
  bundle.json          index and version
  materials.json       material table
  models.json          model table
  textures/<n>.png     one file per material, power of two
  meshes/<n>.mesh      one file per model
```

`bundle.json`:

```json
{
  "format_version": 1,
  "name": "entropy-zero",
  "scale": { "units_per_block": 32.0 },
  "materials": "materials.json",
  "models": "models.json"
}
```

`materials.json` is an array indexed by surface index, so entry `i` describes
`src2mc:surface_i`:

```json
[
  {
    "material": "concrete/concretefloor001a",
    "texture": "textures/concrete_concretefloor001a.png",
    "blocks_per_repeat": [4.0, 4.0],
    "render_type": "solid",
    "surface_prop": "concrete",
    "sound": "stone"
  }
]
```

`material` is the original Source path, for diagnostics only. `render_type` is
one of `solid`, `cutout`, `translucent`. `sound` is a vanilla sound group name,
already mapped from `surface_prop` by the converter so the mod does not repeat
that table.

`blocks_per_repeat` is how many blocks one repeat of the texture covers along
each axis, measured from the map's own texture vectors. It is the whole point of
this format: a texture that spans eight blocks of wall is **one** texture with a
scale of 8, never eight textures and never eight blocks.

### Texture coordinates — **specified**

A surface block carries no per-position data. Its texture coordinates are
derived by the mod, from the block's world position and the face's normal:

- The face's two axes are the two world axes it does not point along —
  triplanar, picked by the largest component of the normal.
- `u = world[axis0] / blocks_per_repeat[0]`,
  `v = world[axis1] / blocks_per_repeat[1]`, taken across the face's own extent
  so neighbouring blocks continue the same repeat rather than each restarting
  it.

This is what removes the duplication the format exists to remove. It also means
alignment with the original Source UVs is approximate: the phase is taken from
world position rather than from the face's `textureVecs`, so a wall may be
offset from the original by up to one repeat. That trade is deliberate and
recorded as D10 in `docs/decisions.md`. A converter must not compensate by
emitting more materials.

`models.json` is an array indexed by `Data.model`:

```json
[
  {
    "mesh": "meshes/props_c17/bench01a.mesh",
    "materials": [3, 17],
    "bounds": { "min": [-0.5, 0.0, -0.5], "max": [0.5, 1.1, 0.5] }
  }
]
```

`bounds` is the model's axis-aligned bounds in blocks at scale 1.0, and exists
so the mod can pick carrier cells without decoding the mesh.

The mesh container is deliberately left open in version 1 — see
`docs/decisions.md`. Whatever it is, it must be readable without a full model
loader and must carry positions, normals, UVs, and a material index per
triangle.

## 5. Reload — **specified**

Adding a map to a bundle, or changing a texture, must require a resource reload
and nothing else. Concretely: the pool of registered blocks, the block entity
type, and the item set are all fixed at build time and must not depend on the
bundle's contents. Anything that would need a registry entry per material, per
model or per placement is a format bug, not an implementation detail.

## 6. Fixtures

`tests/fixtures` holds real converter output, committed. The Rust test
`tests/fixtures.rs` regenerates them and fails if the bytes differ; the mod's
tests read the same files. This is the mechanism that catches drift — a change
in either implementation that breaks the other fails in the same CI run.

Regenerate with:

```sh
UPDATE_FIXTURES=1 cargo test --test fixtures
```

Review the diff. A fixture change is an interface change unless you can say why
it is not.
