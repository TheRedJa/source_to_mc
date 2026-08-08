# Decisions

Append-only. One entry per question that has been settled, with the evidence
that settled it. Entries are not edited or deleted; a decision that turns out to
be wrong gets a later entry that supersedes it, naming the one it replaces.

The point of this file is that neither the converter nor the mod re-derives, or
quietly contradicts, something already measured. Anyone — human or agent —
picking up work on either side reads this before proposing a design. If a
proposal conflicts with an entry here, the entry wins until a new measurement
replaces it.

Every entry states its evidence. "It seemed reasonable" is not evidence.

---

## D1 — Geometry goes in the chunk mesh; nothing renders or ticks per placement

2026-08-08

Props are drawn as part of the chunk mesh, through a dynamic baked model. No
placement may add an entity, a `BlockEntityRenderer` or a `BlockEntityTicker`.

**Evidence.** An earlier display-entity version of this project cost 45 fps.
Measured since, on the author's machine: roughly 300 block displays cost about
10 fps, and past roughly 500 loaded entities the loss stops being linear and
grows sharply. A single map holds a few hundred props and maps are exported
stacked, so a per-placement entity crosses that knee at the second map.

**Consequence.** Placements are non-ticking, non-rendered block entities: NBT
attached to a position, one entry in the chunk's block entity map, read once on
chunk load. They contribute nothing to the entity count the measurement is
about.

## D2 — Triangle count is not a budget

2026-08-08

No mesh decimation, no level of detail, no prop merging for performance
reasons.

**Evidence.** A converted map with over 3000 props, running into millions of
triangles, rendered with virtually no frame cost under Sodium.

**Consequence.** Complexity is spent on keeping work out of the entity and tick
paths, never on reducing geometry. Geometry is still split across carrier cells,
but for Sodium's vertex encoding range (D3), not for speed.

## D3 — A chunk vertex reaches about 8 blocks

2026-08-08

A prop's geometry is split across carrier cells so no vertex sits more than
about 8 blocks from the block it belongs to.

**Evidence.** Sodium 0.8.13-beta.1 `CompactChunkVertex` encodes positions in 20
bits per axis with `MODEL_ORIGIN 8.0f` and `MODEL_RANGE 32.0f`.

**Consequence.** Carrier cells exist for rendering as well as collision. They
are derived state, computed at bake time and never written to a schematic
(`docs/format.md` §3).

## D4 — Collision reaches exactly one block, so one big shape is impossible

2026-08-08

A prop cannot be a single block carrying one prop-sized collision shape.

**Evidence.** Vanilla `BlockCollisions` builds its cursor from
`floor(min - 1e-7) - 1` to `floor(max + 1e-7) + 1` — one block of margin.
Lithium 0.15.4, which is installed, replaces that path with
`ChunkAwareBlockCollisionSweeper` and expands by `expandMin`/`expandMax` of -1
and +1. Both agree. Geometry described further away is simply never tested.
The shape itself is constructible — `Shapes.box` only asserts min <= max, and an
oversized box falls to the `ArrayVoxelShape` path rather than being rejected —
but nothing looks for it.

**Consequence.** Collision is per cell. Supersedes nothing; this closed the
question asked in `~/.claude/plans/i-want-to-brainstorm-proud-volcano.md`.

## D5 — Sable voxelizes the world per block, so per-cell collision is also the right shape

2026-08-08

Even if D4 did not hold, one big shape would be wrong.

**Evidence.** Create: Aeronautics bundles Sable 2.0.3, whose world collision is
a per-block voxel grid: `VoxelNeighborhoodState.getState` asks only
`getCollisionShape(...).isEmpty()` and `isCollisionShapeFullBlock(...)` per
cell, then classifies EMPTY / FACE / EDGE / CORNER / INTERIOR. One carrier with
a huge shape would leave 26 of every 27 cells EMPTY and ships would pass
through the prop.

**Consequence.** The per-cell design is what the physics mods this project
targets actually need. Sable is also data-driven —
`data/<ns>/physics_block_properties/*.json` with `sable:mass`, `sable:volume`,
`sable:friction`, `sable:restitution` and others, selected by block id or tag —
so the mod can declare prop mass from the Source `$surfaceprop` and make
collision-only cells weightless.

## D6 — Nothing content-dependent may be a registry entry

2026-08-08

The set of registered blocks, block entity types and items is fixed at build
time. Materials, models and placements are data.

**Evidence.** Registries freeze at startup, which is the entire reason the
current KubeJS output needs a restart when a map is added. Measured on one map
(`d1_trainstation_02`, KubeJS mode): 181 MB across about 40,000 files — 67 MB of
textures, 62 MB of models, 45 MB of blockstates — describing 168 distinct
materials and 95 distinct models.

**Consequence.** Adding a map or changing a texture is a resource reload
(`docs/format.md` §5). Note that registration *count* was never the performance
problem — 27,000 KubeJS registrations load in 0.21 s. The problem is
duplication: file count, disk, and the restart.

## D7 — Dynamic per-position models work under the installed mods

2026-08-08

`IDynamicBakedModel` plus `ModelData` is the mechanism for prop geometry.

**Evidence.** NeoForge 21.1.231 ships `client/model/data/ModelData`,
`IDynamicBakedModel`, `AttachmentType`, `AddPackFindersEvent`,
`RegisterSpriteSourceTypesEvent`, `ModelEvent.{ModifyBakingResult,
BakingCompleted, RegisterGeometryLoaders}`, `RegisterClientReloadListenersEvent`
and `IBlockEntityExtension.getModelData()` / `requestModelDataUpdate()`. Sodium
0.8.13-beta.1 honours it — `BlockRenderer`, `SodiumModelDataContainer`,
`PlatformModelAccess.getModelData`.

**Consequence.** Per-placement geometry with no per-frame cost is available
without mixing into the renderer.

## D8 — Placements travel in the schematic as block entities

2026-08-08

Prop placements are `Blocks.BlockEntities` entries, not a side file and not a
chunk attachment.

**Evidence.** WorldEdit 7.3.8's `SpongeSchematicV3Writer` and `Reader` both
handle `BlockEntities`, so per-position NBT survives copy, paste and save. A
NeoForge `LevelChunk` `AttachmentType` would create no block entities at all,
but WorldEdit copies blocks and block entities, not attachments, so a copied
region would arrive without its props.

**Consequence.** Pasting a converted map is the workflow, so transport wins.
Revisit only if a few hundred non-ticking block entities per map measure badly,
which D1 says they will not.

## D9 — Surfaces are pool blocks, not block entities

2026-08-08

World geometry uses a fixed pool of registered blocks, `src2mc:surface_N`.

**Evidence.** Surfaces are the bulk of a map — hundreds of thousands of cells,
against a few hundred props. Block entities for those would be millions per
campaign, and blockstate transport is free.

**Consequence.** The pool size caps distinct materials per bundle. The cap must
be a hard, named error, and the number needs measuring across E:Z 1 and 2 before
it is fixed.

## D10 — Surface texture coordinates come from world position, not from stored data

2026-08-08

A surface block carries no per-position data. The mod derives its texture
coordinates triplanar from the block's world position and the face normal,
scaled by the material's `blocks_per_repeat`.

**Evidence.** The alternatives were counted. Storing a tile index in blockstate
properties multiplies the block state count by the tile grid: a 4096-entry pool
with a 16x16 grid is a million block states, each a real object. Folding the
tile into the pool index reintroduces exactly the duplication being removed —
the measured map has 168 materials split up to 64 tiles per axis. A block entity
per surface is ruled out by D9.

**Consequence.** The phase of the texture comes from world position rather than
from the face's `textureVecs`, so alignment with the original map is
approximate — a surface may be offset from Source by up to one repeat. Since the
geometry has already been snapped to a block grid, most of that alignment was
lost before this stage anyway. The converter's job is to measure
`blocks_per_repeat` honestly and emit one material per material; compensating
for the offset by emitting more materials is a format violation.

---

## Open

Questions that are not settled. Moving one of these into an entry above requires
evidence, not a preference.

- **Pool size.** How many surface indices does a campaign need? One data point,
  measured 2026-08-08 by converting `d1_trainstation_02` in bundle mode: **232
  distinct materials for a single map**, 168 of them from world geometry and the
  rest from props. A campaign shares a great many of those between maps, so the
  total is nowhere near 232 times the map count — but it is well above 232, and
  any pool in the low hundreds is too small for even one map plus its props. The
  converter currently uses 4096 as a placeholder. Measure across E:Z 1 and 2
  before fixing it, and note that the number must match in `src/output/bundle.rs`
  and in the mod.
- **Mesh container.** What `meshes/*.mesh` actually is. It must be readable
  without a model loader and carry positions, normals, UVs and a per-triangle
  material index. glTF, a trivial custom binary, and a packed vertex buffer are
  all candidates.
- **Where assets come from.** An exported bundle of PNGs and meshes, or the mod
  reading the user's own Source installation through `gameinfo.txt` and decoding
  VTF and MDL on the fly. The second ships no Valve content at all and collapses
  a campaign to almost nothing, at the cost of VTF and MDL readers in Java.
- **Whether the KubeJS path is kept.** It works today and needs no mod. Keeping
  it means maintaining two outputs.
