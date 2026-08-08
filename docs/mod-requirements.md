# src2mc companion mod — design requirements

Status: requirements, not a design that has been committed to. Nothing here has
been built. Every measurement and API reference was taken from the versions
installed in the E-Z instance (NeoForge 21.1.231, Minecraft 1.21.1, KubeJS
2101.7.2, Sodium 0.8.13-beta.1, Lithium 0.15.4, Sable 2.0.3, WorldEdit 7.3.8).

## 1. Why

src2mc currently expresses everything a Source map needs through blocks that
KubeJS registers. It works, and it does not scale. Ten maps come to about
1.7 GB and 130,000 registered blocks; a full Entropy: Zero 1 and 2 conversion
would be several times that.

`d1_trainstation_02` alone, converted with textures and props, writes:

| Part of the pack | Size | Files |
|---|---:|---:|
| Textures | 67 MB | 14,989 |
| Models (`.obj`, `.mtl`, block JSON) | 62 MB | 13,629 |
| Blockstates | 45 MB | 11,244 |
| Startup script | 8.1 MB | 1 |
| **Total** | **181 MB** | **~40,000** |

Almost none of that is content. The map uses **168 distinct materials** and
**95 distinct models**. Everything above that number is the same information
written out again under another name, for three reasons:

- **Texture tiling.** A Source wall texture covers several blocks, so it is cut
  into tiles and each tile is registered as its own 16×16 block. 135 of the 168
  materials are split, up to 64 tiles per axis. One texture becomes hundreds of
  files, each a worse copy of it.
- **Baked prop placements.** A prop's rotation and sub-block offset are baked
  into its mesh, so every distinct placement is a separate model, `.obj` and
  blockstate — 2195 of them for 325 props, because large props are split across
  several blocks as well.
- **Collision.** Every distinct collision box is a registered block: 8954 of
  them for this map, each with a blockstate and a model JSON.

Two further limits come from the same source. Registering blocks is a startup
operation, so **adding a map means restarting the game**; and a converted prop
is frozen at conversion time, so **a door that came out too small cannot be
fixed in the world** — it has to be reconverted.

A mod removes all three causes rather than trading them off. This document says
what it has to do, not how to write it.

## 2. Goals

1. Pack size proportional to the map's **content** — its distinct materials and
   models — not to its placements, tiles or collision cells.
2. **No restart** when a map, texture or model is added. A resource reload
   (`F3+T`) and a datapack reload (`/reload`) are acceptable and expected.
3. Props **movable, rotatable and rescalable in the world**, after the fact,
   without reconverting.
4. Rendering cost no worse than today: prop geometry stays in the chunk mesh
   rather than going through the entity renderer every frame.
5. Collision at least as good as today's per-cell shapes, and still visible to
   Sable and Create: Aeronautics.
6. Schematics remain the transport: what WorldEdit pastes must be complete.

## 3. Non-goals

- Replacing WorldEdit as the paste mechanism.
- Rendering Source's shaders, lightmaps or animated materials.
- Running the conversion inside Minecraft. src2mc stays a Rust tool that writes
  a bundle; the mod consumes it.
- Editing the map's brush geometry in-world. Only props are editable.
- Keeping the KubeJS output working. The mod replaces it.

## 4. Constraints that are already measured

These are not assumptions. Each was read out of the installed jars during the
work that led here, and each rules out an otherwise obvious design.

- **A chunk vertex reaches 8 blocks.** Sodium's `CompactChunkVertex` packs each
  coordinate into 20 bits spanning −8…+24 blocks from the section origin, and
  masks rather than clamps. A block model whose geometry reaches further is
  drawn correctly to the limit and then folds back over the map. Since a block
  may sit anywhere in its section, **±8 blocks is the safe reach**, and a prop
  bigger than that must be drawn from more than one block.
- **A collision shape reaches 1 block.** `BlockCollisions` scans
  `floor(min − 1e-7) − 1 … floor(max + 1e-7) + 1`. Vanilla names the concept —
  `BlockBehaviour$BlockStateBase$Cache.largeCollisionShape` is "any axis where
  the shape leaves the block" — and Lithium tracks the same as a per-section
  `OVERSIZED_SHAPE` flag with the same ±1 expansion. One prop-sized shape is
  therefore impossible; collision must be per cell.
- **Sable voxelizes the world per block.** `VoxelNeighborhoodState.getState`
  asks only `getCollisionShape(...).isEmpty()` and `isCollisionShapeFullBlock`,
  per cell. Ships collide with cells, not with shapes, so every cell a prop
  occupies needs a block with non-empty collision or ships pass through it.
- **Registries freeze at startup.** This is the whole of the restart problem.
  Any design where content adds registry entries keeps it.
- **Model data is chunk-baked and Sodium supports it.** NeoForge's
  `IDynamicBakedModel` takes a `ModelData` argument, `IBlockEntityExtension`
  provides `getModelData()` and `requestModelDataUpdate()`, and Sodium's
  `BlockRenderer` and `SodiumModelDataContainer` read it. Per-position geometry
  can therefore be *baked into the chunk*, at no per-frame cost, without
  registering anything per position.
- **Schematics carry block entities.** WorldEdit's `SpongeSchematicV3Writer`
  and `Reader` both handle `BlockEntities`. Per-position NBT is a supported
  transport. src2mc does not write it yet — `src/output/schem.rs` emits only
  `Palette`, `Data` and `Entities`.
- **The reload hooks exist.** `AddPackFindersEvent` mounts a folder as a pack,
  `RegisterSpriteSourceTypesEvent` allows a sprite source that enumerates files
  at stitch time, and `RegisterClientReloadListenersEvent` runs mod code on
  `F3+T`.

## 5. Requirements

### R1 — A constant number of registry entries

The mod must register a fixed set of blocks, block entities and items that does
not grow with the number of maps, materials, models, placements or collision
shapes converted.

Everything content-dependent must be **data**: loaded from the bundle, replaced
on reload, and never a registry object. This is the requirement that delivers
both "no restart" and most of the size reduction; every other requirement is
subordinate to it.

The pool approach is the expected shape: a fixed number of indexed surface
blocks (say `src2mc:surface_0` … `src2mc:surface_4095`) whose meaning comes
from a material table in the bundle, plus a handful of purpose-built blocks for
props and collision. If a campaign needs more distinct materials than the pool
holds, that is an error the tool reports at conversion time, not a silent
collapse.

### R2 — One texture per material, tiled by projection

A Source material must appear in the pack **once**, at a resolution worth
having, and be tiled across a wall by computing texture coordinates rather than
by cutting it into blocks.

- The material table carries what src2mc already knows from the face's texture
  projection: how many world units one repeat of the texture covers, per axis.
- The block's dynamic model computes UVs from the block's world position and
  that scale, which is what Source itself does. Bricks line up across a wall
  because the projection is continuous, not because a tile was placed.
- Consequence: `[materials] tile_textures`, `tile_max` and `max_blocks` become
  meaningless for this output path, and the 16×16 downsample can go — the
  reason it exists is that a tile was a block face.

Expected effect on the sample map: 14,989 texture files at 16×16 become 168
textures at source resolution.

### R3 — Props are data, one mesh per model

A prop placement must not create a model, a file or a registry entry.

- A model is loaded once per `.mdl`, from the bundle, and shared by every
  placement in every map.
- A placement is a block entity at the prop's anchor cell holding: model index,
  rotation (quaternion), sub-block offset, uniform scale, and a stable id.
- Geometry is produced by a dynamic model from that block entity's model data,
  so it lands in the chunk mesh and costs nothing per frame.
- The block entity must register no `BlockEntityTicker` and no
  `BlockEntityRenderer`. Both are where block entities earn their reputation:
  a ticker runs every tick for every loaded instance, and a renderer draws
  outside the chunk mesh, unbatched and unculled, which is the cost this design
  exists to avoid. What is left is an NBT payload attached to a position — one
  entry in the chunk's block entity map, serialized with the region file, read
  once on chunk load. A map holds a few hundred of them, against the thousands
  of chests and signs a vanilla world carries without trouble.
- Surfaces, which are the bulk of a map, are never block entities. They are
  pool blocks (§R1), so the per-position cost applies only to props.
- Because a mesh may reach further than a chunk vertex can encode (§4), the mod
  splits a prop's geometry across carrier cells **at bake time, in memory**.
  Carrier cells are derived state: the mod places and removes them, they are
  never written to a schematic, and a prop that moves takes its carriers with
  it.

Expected effect on the sample map: 2195 prop variants, their `.obj` files and
their blockstates become 95 meshes.

### R4 — Collision computed, not registered

Collision must be derived from the prop's mesh and transform at runtime, per
cell, and served through the carrier block's `getShape`.

- No registered block per shape, no blockstate or model JSON per shape.
- Per-cell, non-empty shapes, so Sable's voxelization sees the prop (§4).
- The existing algorithm carries over unchanged in spirit: clip the triangles to
  each cell, take the bounds, round outward. It is already implemented in
  `src/output/collision.rs` and is the reference for what the mod should do.
- Shapes must be cached per section and invalidated when a prop moves.

Expected effect on the sample map: 8954 registered collision blocks become
zero.

### R5 — Reload without restart

Adding or changing a map's assets must require no game restart.

- Textures and meshes live in a folder the mod mounts through
  `AddPackFindersEvent`; new files appear on `F3+T`.
- The material table, model table and any per-map metadata reload with the
  datapack (`/reload`) or through a mod command, on both sides: the server needs
  meshes too, because collision is derived from them.
- The mod must state clearly what a reload cannot fix, and there should be as
  little of that as possible. Growing the block pool is the known case, and it
  is a config change plus a restart.

### R6 — Props editable in the world

A converted prop must be adjustable in place, because conversion cannot get
every prop right: a door that came out too small to walk through has to be
fixable without reconverting the map.

- Select a prop by looking at it — hit testing against the prop's own mesh, not
  just its anchor cell.
- Move, rotate and scale it, both by nudging (keyboard or a tool) and by typing
  exact numbers, in Source units as well as blocks. The numbers a mapper thinks
  in are Hammer units and degrees.
- Delete and duplicate.
- Every edit updates the block entity, re-bakes the carriers and recomputes
  collision. Undo for at least the last edit.
- Edits must survive a save and a copy: they live in the block entity, which is
  what the schematic carries.
- Editing must be gated — creative or a permission — so a survival player
  cannot rearrange the map.

### R7 — Transport through schematics stays complete

What WorldEdit pastes must be a working map.

- src2mc writes `Blocks.BlockEntities` into the Sponge v3 schematic, holding
  each prop's placement NBT. `src/output/schem.rs` gains that; the palette keeps
  doing what it does for surfaces.
- Pasting must reconstruct carriers and collision from the anchors alone.
- A paste into a world without the bundle must fail loudly and legibly, not
  silently produce a map of missing-texture cubes.

### R8 — Compatibility

The mod must work with what this project is actually played with: Sodium,
Lithium, Sable, Create: Aeronautics, WorldEdit and FAWE. Each has a known
interaction:

- **Sodium** — geometry must come through model data, which it supports; the
  ±8 block vertex reach is a hard constraint on how geometry is split.
- **Lithium** — replaces the collision sweeper; shapes must stay within the ±1
  block it and vanilla agree on.
- **Sable / Create: Aeronautics** — per-cell non-empty collision, and props
  should declare sensible physics properties (see §9).
- **WorldEdit / FAWE** — block entities must round-trip through copy, paste and
  schematic save.

### R9 — Performance budgets

Numbers to design against, taken from what the current output achieves:

- Chunk bake: prop geometry in the chunk mesh, no per-frame entity rendering.
  The regression this project already suffered — 45 fps from display entities —
  must not come back.
- Memory: per-section collision caches and model data must be bounded; a map is
  hundreds of thousands of prop cells.
- Load: mounting a campaign's bundle should be seconds, not minutes.
- Editing a prop must re-bake only the affected sections.

### R10 — Where the assets come from

Two options, and the mod should not foreclose either:

1. **Exported bundle** — src2mc writes PNGs and meshes. Self-contained, works
   without the game installed, costs disk.
2. **Read the game's own files** — the mod resolves materials and models out of
   the user's Source installation through the same `gameinfo.txt` search path
   src2mc already implements, decoding VTF and MDL on the fly. The bundle then
   holds only references, and a campaign costs almost nothing.

Option 2 is the smaller and more honest of the two — it ships no Valve content
at all — but it needs a VTF and MDL reader in Java and only works where the
game is installed. Option 1 is the fallback and the default.

## 6. Data the bundle must carry

Sketch, to be pinned down during design:

- **Material table** — per material: texture reference, render type (solid,
  cutout, translucent), texture scale in world units per repeat, sound group,
  and the surface properties Source knows about.
- **Model table** — per `.mdl`: mesh reference, material references, bounds,
  triangle count.
- **Placements** — carried in schematic block entities, not in the bundle:
  model index, rotation, offset, scale, id.
- **Block pool mapping** — index to material, merged across the maps in a
  world, versioned so an older schematic still resolves.
- **Meshes** — a binary format rather than `.obj`; `.obj` is 62 MB of ASCII for
  one map today, and the mod has no reason to parse text.

## 7. What src2mc has to change

- Emit a bundle instead of a KubeJS pack: material table, model table, textures
  at source resolution, binary meshes.
- Write `Blocks.BlockEntities` in the schematic writer, with prop placements.
- Stop baking rotation into meshes, stop splitting textures into tiles and stop
  generating collision blocks for this output path. The code that does all three
  stays for the KubeJS path if that is kept.
- Keep the vanilla path exactly as it is: it needs no mod and should stay the
  no-dependency option.

## 8. Acceptance criteria

The mod is worth building only if it hits numbers like these. They are the
point of the exercise, so they should be measured, not assumed:

- `d1_trainstation_02` converts to a bundle **under 25 MB** with no loss of
  visible detail, against 181 MB today.
- Entropy: Zero 1 and 2 together stay **under 1 GB**, against a projection of
  several gigabytes.
- **Zero** registry entries per map. Adding a map needs a reload, not a restart.
- Frame rate in a converted map no worse than the current baked-block output.
- A prop can be moved, rotated and scaled in-game, and the change survives a
  save, a reload and a schematic round-trip.
- Sable ships collide with props exactly as they do now.

## 9. Open questions

- **Block pool size.** How many surface indices does a campaign need? E:Z 1 and
  2 together should be measured before a number is picked, and the failure mode
  when it is exceeded has to be a clear error.
- **Surfaces as pool blocks or as block entities.** The pool keeps blockstate
  transport free; block entities would remove the pool limit entirely but at
  millions of block entities per map. The pool is the recommendation.
- **Prop placements as block entities or as chunk attachments.** A NeoForge
  `AttachmentType` on `LevelChunk` would hold placements as sparse per-chunk
  data and create no block entities at all. It costs the transport: WorldEdit
  copies blocks and block entities, not attachments, so a copied region would
  arrive without its props and R7 would have no carrier. Block entities are the
  recommendation because pasting a converted map is the workflow. An attachment
  is worth revisiting only if a few hundred non-ticking block entities per map
  measure badly, which is not expected.
- **Physics properties.** Sable reads
  `data/<ns>/physics_block_properties/*.json` with `sable:mass`,
  `sable:volume`, `sable:friction`, `sable:restitution` and others, selected by
  block id or tag. Prop carriers should declare mass from the Source
  `$surfaceprop` and collision-only cells should be weightless, so a ship built
  around a prop is not dragged down by invisible blocks.
- **Lighting.** Chunk-baked prop faces are lit per face today. Whether the mod
  can do better, and whether it should, is unexplored.
- **Does the KubeJS path stay?** It has one advantage: it needs no mod of ours.
  Keeping both costs maintenance in the tool.

## 10. Risks

- **Two moving targets.** Sodium's chunk format and Lithium's collision path are
  both replacements for vanilla internals, and both have already forced design
  decisions here. A mod that leans on model data leans on Sodium continuing to
  support it.
- **Scope.** This is a NeoForge mod in Java, a language this project does not
  use, with a renderer, an editor and a data pipeline. It is larger than the
  converter that feeds it.
- **The bundle is a new format** that both sides have to agree on, versioned
  from the start, or every schematic ever pasted becomes unreadable.
- **Option 2 in R10 needs a VTF and MDL reader in Java**, duplicating what
  src2mc has in Rust. Sharing that code across languages is its own project.
