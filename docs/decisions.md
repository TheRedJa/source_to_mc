# Current mod architecture decisions

Status: decisions for the clean mod restart described by
[`mod/IMPLEMENTATION_PLAN.md`](../mod/IMPLEMENTATION_PLAN.md). The abandoned
prototype is not a compatibility target. Historical measurements are retained
only where they constrain the new design.

When new evidence changes a decision, add a superseding entry that names the
old one. Do not silently make code and documentation disagree.

## D1 — Rust converts; Minecraft consumes

The Rust CLI remains the only Source BSP conversion implementation. The mod
does not parse BSP, VPK, VTF or MDL files and does not depend on the user's game
installation at runtime. It consumes exported schematics and a campaign bundle.

This keeps Source-specific parsing in the mature converter and makes installed
campaigns self-contained.

## D2 — One schematic per map and one ZIP bundle per campaign

WorldEdit schematics remain the transport for world placement. Shared assets
and metadata live in a versioned, custom-extension ZIP campaign bundle.

A batch conversion emits one bundle plus one schematic per map. Content uses
stable IDs and hashes so textures and models shared by several maps occur once.
The exact v1 layout is deliberately deferred to Phase 1.

## D3 — Content never creates registry entries

The mod registers a small fixed set of generic blocks, items and block-entity
types. Materials, models, maps, placements and collision shapes are runtime
data.

Minecraft registries freeze at startup. The earlier KubeJS route generated
content-specific blocks and therefore required restarts, huge file counts and
large registries. Reproducing that model inside the mod would preserve the
problem the mod exists to solve.

## D4 — A map anchor defines map-local lookup

Each schematic carries a map anchor at its original origin. At runtime anchors
map world positions back to bundle ID, map ID and map-local coordinates.

Surface cells are generic blocks with no per-cell block entity. Their material
and planar UV region come from the bundle's spatial data. This supports several
maps, and several independently pasted instances of one map, without encoding
content identity in registered block IDs.

Carriers are derived state and are not authoritative. Anchors and prop roots
are authoritative and must survive WorldEdit copy/paste.

## D5 — Preserve Source UV projection data

Surface texture coordinates come from planar UV regions exported by the
converter and evaluated in map-local space. Orientation, scale and phase are
part of the data.

The abandoned prototype derived triplanar UVs from world position and stored
only `blocks_per_repeat`. That could shift a material by up to one repetition
and could not reproduce arbitrary Source texture vectors. The new format must
not inherit that approximation.

## D6 — The mod owns paged textures

Generated Source textures do not enter Minecraft's vanilla block texture
atlas. The first backend uses mod-owned atlas pages no larger than 4096 x 4096.

Half-Life 2 alone grew the KubeJS pack beyond 9 GB and drove the block atlas to
roughly 16k x 16k, causing startup failures on weaker GPUs. Multiple bounded
pages make capacity explicit and prevent one global texture from growing with
the campaign.

A texture-array backend is a gated fallback if paged atlases fail the in-game
visual test; it is not silently substituted during implementation.

## D7 — Prop geometry is chunk-baked

Props are non-ticking root block entities whose model data feeds the chunk
mesh. No placement creates a permanent Minecraft entity, block entity renderer
or block entity ticker.

Earlier display-entity experiments lost substantial frame rate at a few hundred
loaded props, with cost worsening beyond roughly 500 entities. By contrast, a
converted map with more than 3,000 props and millions of triangles showed no
meaningful comparable frame cost once the geometry reached the chunk mesh.

Triangle count is therefore not an arbitrary acceptance budget. Correct
lifecycle and keeping work out of per-frame entity rendering are the priorities.

## D8 — Large prop rendering uses derived carriers

A prop has one authoritative root. Invisible carrier blocks divide rendering
across chunk/section space where needed and are rebuilt from the root after
placement, load or reconciliation.

In Sodium 0.8.13-beta.1, compact chunk vertex coordinates make approximately
eight blocks in either direction the safe reach regardless of the carrier's
position in its section. Geometry beyond that can wrap rather than merely clip.
Compatibility testing must confirm the constraint for the actual target
version before relying on the exact number.

## D9 — Prop collision is derived per cell

Collision is generated from the prop mesh and transform, conservatively
quantized, cached per chunk or section and exposed through carrier cells.

Vanilla and the tested Lithium implementation search only near blocks crossed
by the moving shape, so one oversized collision shape at the prop root is not
discovered across a large prop. The tested Sable implementation also
voxelizes world collision per block. Per-cell collision satisfies both.

Collision shapes are runtime data, never registered block variants.

## D10 — Roots and anchors degrade to repairable placeholders

Missing, corrupt or unsupported bundle data must not destroy the information
needed to recover. Map anchors and prop roots become persistent placeholders
that retain their IDs and source metadata.

After corrected content is installed, reload plus reconciliation heals them
without repasting the schematic. Diagnostics identify the bundle, map or asset
and use stable error codes for format-boundary failures.

## D11 — Compatibility claims require the actual target environment

Minecraft 1.21.1, NeoForge and Java 21 are fixed. Claims involving Sodium,
Lithium, WorldEdit/FAWE, Create, Create: Aeronautics or Sable require their exact
target versions and the agreed test instance.

The placeholder table in `mod/docs/compatibility-baseline.md` is intentionally
not evidence. Manual screenshots, logs and benchmark reports remain local and
must not include proprietary game assets.

## D12 — The new format starts at Phase 1

The directory bundle, `surface_N` pool, `src2mc:FormatVersion` schematic field
and shared fixtures from the first prototype are abandoned. They impose no
compatibility or migration requirement on the new implementation.

Phase 1 defines the new ZIP manifest, JSON tables, binary mesh, schematic NBT,
version rules and error codes together. Until then, no checked-in document or
dead source file is a normative interchange contract.
