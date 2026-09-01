# src2mc mod — product and design requirements

Status: current requirements for the new mod. The implementation sequence is in
[`../mod/IMPLEMENTATION_PLAN.md`](../mod/IMPLEMENTATION_PLAN.md). The abandoned
first mod prototype and its interchange format are not a compatibility target.

## Purpose

The larger project is a recreation of the Half-Life 2 universe in Minecraft.
That still leaves a great deal of creative work to do, so producing the base
maps must be automatic rather than consuming days of manual blocking-out.

src2mc has developed toward that goal in stages:

1. WorldEdit schematics made entirely from stone.
2. Vanilla Minecraft blocks selected for approximate material colour.
3. KubeJS blocks carrying the maps' real Source textures.
4. Textures projected continuously across many Minecraft blocks instead of
   being squeezed onto each face.
5. Real Source prop meshes emitted through KubeJS.

That path proved the conversion and the desired visual result, but not a
scalable way to deliver it. Half-Life 2 alone produces a generated pack well
over 9 GB. The vanilla block texture atlas grows to roughly 16k x 16k, and
Minecraft can fail to start on weaker GPUs. The generated pack also duplicates
textures, models, blockstates and collision descriptions across tiles and prop
placements.

The first companion-mod prototype grew directly from that output without a
complete design. It used a fixed pool of material blocks and the vanilla block
atlas, accumulated correctness and lifecycle problems, and was scrapped. The
current mod is a clean restart. It must solve the scaling problem rather than
encode the KubeJS workaround in Java.

For a smaller measured example, `d1_trainstation_02` produced about 181 MB and
40,000 files for only 168 materials and 95 distinct models. The bulk came from
texture tiles, placement-specific prop meshes and registered collision shapes.

## Product goals

1. Convert Source BSP geometry, terrain, materials and static props into a
   useful Minecraft base map with minimal manual work.
2. Keep heavyweight immutable asset data proportional to distinct source
   content. Placement records may scale with placements, and rebuildable
   carrier/collision state may scale with occupied cells, but neither may
   duplicate textures or meshes.
3. Preserve the appearance that motivated the KubeJS path: real textures tile
   at the correct scale and phase, and props use their real meshes.
4. Load a whole campaign without depending on the vanilla block texture atlas
   or creating content-dependent registry entries.
5. Add or repair campaign data through a controlled reload, without restarting
   Minecraft or repasting a map.
6. Keep rendering chunk-baked. No permanent Minecraft entity, block entity
   renderer or ticker may be created per prop.
7. Keep unrotated, unmirrored WorldEdit schematic paste as the ordinary
   map-placement workflow. WorldEdit is not a map editor in the prototype.
8. Preserve enough source metadata and diagnostics to make conversion failures
   actionable instead of silently degrading the map.

## Scope

- Java 21, Minecraft 1.21.1 and NeoForge.
- The Rust CLI remains the only Source BSP conversion implementation.
- Mod export uses the project's fixed scale of 32 Source units per Minecraft
  block and emits one schematic per map. A versioned campaign bundle contains
  the shared materials, textures, meshes, UV data and metadata for one or more
  maps.
- The mod loads, validates, renders and reconciles that exported data.
- Surface blocks are generic. Their material and UV region are resolved from a
  map anchor and map-local position, without a block entity on every surface.
- Props are non-ticking root block entities. Invisible carrier blocks are
  derived implementation state and can always be rebuilt from the roots.

## Non-goals for the prototype

- Replacing WorldEdit or performing BSP conversion inside Minecraft.
- Reproducing Source gameplay logic, shaders, lightmaps, animation, decals or
  every translucent effect.
- A full in-game prop editor. Reliable placement, persistence and pick-block
  metadata come first.
- WorldEdit rotation, mirroring, partial-map copying or editing pasted maps
  through WorldEdit.
- Accurate collision while props move on Create, Sable or Aeronautics
  structures.
- Multiplayer asset synchronization, vanilla-client support or migration from
  the abandoned prototype format.
- Shipping Valve or other proprietary game assets in this repository.

## Requirements

### R1 — Fixed registries

The set of registered blocks, items and block-entity types is fixed by the mod
version. Maps, materials, textures, models, placements and collision shapes are
data and never create registry entries.

The generic block set covers surfaces, map anchors, prop roots,
carrier/collision cells and missing-data placeholders. Registry size must not
change after loading or reloading a campaign.

### R2 — Versioned, deduplicated campaign bundles

The converter writes a custom-extension ZIP bundle for a campaign and a
schematic for each map. Shared content appears once. Stable content IDs and
SHA-256 hashes support deterministic export, validation and deduplication.

Bundle identity is a deterministic manifest fingerprint over sorted entry
paths, uncompressed sizes and SHA-256 hashes of canonical entry payloads. ZIP
timestamps, permissions, host metadata and compressed bytes are not content
identity and are ignored by validation.

Before conversion allocates a dense schematic buffer, it validates the map's
32-units-per-block dimensions against the Sponge v3 axis limits and the
converter's documented safe volume limit. An exceptional map that does not fit
fails with an actionable error; mod export never silently tiles, truncates or
rescales it.

The exact v1 manifest, tables, binary mesh layout, NBT and error codes are
Phase 1 work. [`format.md`](format.md) deliberately does not define them yet.

### R3 — Map identity survives WorldEdit

Every pasted map carries an anchor at its schematic origin. The anchor connects
world positions to the correct bundle, map and local coordinate system.

Two unrotated, unmirrored instances of the same map may be pasted at different
positions, and maps from different bundles may coexist. The complete exported
schematic contains its anchor and prop roots. Rotation and mirroring are
unsupported and unchecked; a deliberately transformed paste may render
incorrectly. Anchorless surface data is detectable and must produce a clear
diagnostic. Overlapping placements must likewise produce an explicit diagnostic
rather than an arbitrary material lookup.

The server persists authoritative placements and sends the rendering client the
small immutable placement-index view it needs. This metadata transfer is not
multiplayer bundle or asset synchronization, which remains out of scope.

### R4 — Source texture projection is preserved

A surface block has no block entity and no material-specific block ID. The
converter preserves a canonical record for every visible Minecraft face,
including its cell, face or fitted-shape surface, Source provenance, material
reference and full planar UV transform. This provenance must survive brush and
displacement voxelization, overlap resolution, hollowing, and slab/stair
fitting. The mod resolves those records from map-local coordinates while baking
the chunk mesh.

Face records are indexed in sparse 16x16x16 map-local section buckets. Each face
uses compact references to deduplicated material and UV-region records rather
than repeating its full planar transform. Exact integer widths are selected from
the Phase 0.5 inventory and bounded by the v1 format.

Texture orientation, phase and repetition must match converter fixtures.
Texture tiling is a coordinate operation, not duplicated images or blocks.

### R5 — Textures do not use the vanilla block atlas

The mod owns its texture backend. The first implementation uses paged atlases
no larger than 4096 x 4096, with mipmaps, filtering and transparent-edge
handling. More content than one page must allocate more pages rather than grow
one global atlas.

Each texture allocation is wholly contained by one page and never crosses a
page boundary. An allocation larger than a page follows a documented
reduce/split/reject rule. Atlas allocations use extruded gutters sufficient for
every generated mip level, or an equivalently proven per-allocation mip-clamp
policy, so filtering cannot sample neighboring textures.

Page dimensions alone are not a memory budget. Export and load diagnostics must
report encoded bytes, decoded RAM, mipmapped VRAM and page count. The prototype
must define and enforce an agreed texture-residency budget before whole-campaign
loading is accepted on weaker GPUs.

Campaign metadata and texture indices load on reload, but texture pixels are
demand-resident. Pages referenced by chunks approaching render distance are
decoded and uploaded asynchronously. A page with no loaded-chunk references is
evictable after a grace period, with least-recently-used eviction when the RAM
or VRAM budget is under pressure. A surface whose page is still loading uses a
diagnostic placeholder; completion invalidates only the affected chunk meshes.
Turning the camera does not immediately unload pages.

If paged atlases fail visually in the real client, testing a texture-array
backend is a user decision gate, not an automatic redesign.

### R6 — Props are shared data and chunk-baked geometry

A Source model is stored once and shared by all placements. Each placement is a
non-ticking prop-root block entity carrying a stable ID, model ID, transform,
material data and source metadata.

The converter places that invisible, non-colliding root in the nearest free
schematic cell within or immediately around the transformed model bounds. The
root cell is storage only: the model retains its exact Source-derived position,
rotation and scale independently of where the root block sits. Candidate cells
are ordered deterministically. A root never replaces map geometry or another
root; if the bounded search finds no safe cell, mod export fails with the prop's
stable ID and an actionable diagnostic instead of dropping or moving the prop.

Rendering is part of the chunk mesh. A prop creates no permanent Minecraft
entity, block entity renderer or block entity ticker. Large meshes may use
derived carriers so their vertices remain within the renderer's safe range.
Removing or moving a root must clean up and rebuild its carriers.

### R7 — Collision is derived per occupied cell

Collision is computed from the prop mesh and transform and exposed through
carrier cells. Shapes are conservative, cached by chunk or section, and
invalidated only for affected props.

The fixed 4x4x4 quantization preserves subcell occupancy. It must not replace
disconnected mesh fragments in one cell with a single bounding box that fills
the gap. Adjacent occupied subcells may be merged only without changing the
represented volume.

Collision data must not create registry entries or have a silent global shape
limit. Player collision and static Sable collision use the same carrier-cell
representation.

### R8 — Reload, repair and diagnostics

The mod validates versions, hashes, tables, meshes, textures and map-height
requirements against explicit size/count/depth limits before allocating their
full runtime representation. ZIP paths cannot escape the bundle, and compressed
inputs cannot expand without a documented bound. Missing or corrupt content
becomes a persistent placeholder that retains its original anchor or root data.

After the bundle is repaired, reload and reconciliation must heal placeholders
without repasting. Errors need concise chat summaries, detailed logs and stable
error codes where data crosses the converter/mod boundary.

Reload is staged and atomic: parse, validate and prepare a new immutable
generation before publishing it. A failed reload keeps the last known-good
generation active, and chunk builds cannot mix data from two generations.

### R9 — Compatibility and performance are measured

The target environment includes WorldEdit or FAWE and, once exact versions are
provided, Sodium, Lithium, Create, Create: Aeronautics and Sable. Compatibility
claims are recorded in `mod/docs/compatibility-baseline.md`, not inferred.

Important measured constraints retained from the prototype work:

- Sodium's compact chunk vertices give a safe reach of about eight blocks from
  a carrier in the tested version.
- Vanilla and Lithium discover collision only near the queried block, so large
  props need per-cell collision.
- The tested Sable version classifies world collision per block.
- Display entities caused severe frame-time loss at map-scale counts, while
  millions of triangles in chunk-baked prop geometry did not show comparable
  per-frame cost.

Bundle load/reload time, RAM, texture-page use and dense-view frame time must be
logged and tested on representative real maps before prototype acceptance.
The benchmark record fixes the machine, JVM heap, mod versions, render distance,
resolution, map position, visible scenario and warm/cold state so results can be
reproduced.

### R10 — Ownership and licensing

The converter reads maps and assets from the user's own Source installations.
The repository and distributable test fixtures contain no proprietary game
content unless permission explicitly allows a particular derived fixture.

The Rust converter owns export. The mod owns validation, runtime lookup,
rendering, collision and reconciliation. The versioned format is their only
shared contract.

## Prototype acceptance

The prototype is ready for user evaluation when:

- deterministic converter fixtures round-trip through bundle loading,
  untransformed WorldEdit paste, reload and reconciliation;
- multiple maps and multiple instances resolve independently;
- solid and cutout surfaces retain correct scale, orientation, phase and
  repetition across multiple texture pages;
- rotated, scaled and multi-material props survive paste and reload with no
  permanent entities or ticking/rendering block entities;
- moving a prop root to its deterministic free storage cell does not alter the
  rendered mesh transform;
- player and static Sable collision work for representative fences, railings,
  walkways and containers without closing intended openings or creating
  materially oversized invisible obstacles;
- ordinary bundle loading stays within one minute on the agreed reference
  machine, and reload adds no more than 30 seconds beyond NeoForge's baseline;
- dense-view frame time is measured against the agreed Minecraft/modpack
  baseline and reviewed in game by the user.

The eventual measure of success is practical: the generated base maps save
enough time that the user's effort can go into recreating the Half-Life 2
universe rather than rebuilding existing geometry by hand.
