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

A batch conversion at the fixed mod-export scale of 32 Source units per block
emits one bundle plus one schematic per map. Content uses stable IDs and hashes
so textures and models shared by several maps occur once. Export preflights the
dense schematic dimensions and fails explicitly if an exceptional map cannot
fit; it does not silently tile, truncate or rescale the map. The exact v1 layout
is deliberately deferred to Phase 1.

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
maps, and several independently translated instances of one map, without
encoding content identity in registered block IDs.

Carriers are derived state and are not authoritative. Anchors and prop roots
are authoritative and are included in the complete exported schematic.

The prototype supports ordinary unrotated, unmirrored WorldEdit paste only.
Rotation, mirroring, partial-map copy and post-paste WorldEdit editing are out of
scope. Rotation and mirroring are not detected and may render incorrectly if
deliberately used. Anchorless surface data is detectable and must diagnose
itself rather than guessing a coordinate transform.

The server owns and persists placement identity. The client receives the small
immutable placement-index view required for chunk rendering. This metadata sync
is required even in an integrated single-player game and is distinct from the
deferred problem of distributing campaign bundles or assets to multiplayer
clients.

## D5 — Preserve Source UV projection data

Surface texture coordinates come from canonical visible-face records exported
by the converter and evaluated in map-local space. Each record identifies its
cell and rendered face or fitted-shape surface, Source provenance, material and
full planar transform. Orientation, scale and phase are part of the data.

A single material or block value per voxel is not sufficient: one cell may
expose faces originating from different Source sides. Face provenance must
survive overlap resolution, hollowing and shape fitting before the bundle is
written.

Runtime lookup uses sparse 16x16x16 map-local section buckets. Faces carry
compact references to deduplicated material and UV-region records instead of
repeating a full planar transform. The inventory determines bounded integer
widths before format v1 is locked.

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

Bounded dimensions do not by themselves bound total VRAM. The backend is not
accepted until a real-content inventory establishes decoded RAM, mipmapped VRAM
and page-count costs and the prototype has an explicit residency budget.

A texture-array backend is a gated fallback if paged atlases fail the in-game
visual test; it is not silently substituted during implementation.

Each texture allocation lies wholly on one page. Oversized allocations follow a
documented reduce/split/reject rule. Extruded gutters cover every mip level
unless the implementation proves an equivalent per-allocation mip clamp;
ordinary adjacent atlas pixels must never bleed together under filtering.

Campaign metadata loads eagerly, while texture pixels and GPU pages are
demand-resident. Chunks approaching render distance acquire their pages; pages
with no loaded-chunk references become evictable after a grace period. RAM and
VRAM budgets use least-recently-used eviction under pressure. An asynchronously
loading page renders as a diagnostic placeholder and invalidates only its
dependent chunks when ready. Camera direction alone does not control residency.

## D7 — Prop geometry is chunk-baked

Props are non-ticking root block entities whose model data feeds the chunk
mesh. No placement creates a permanent Minecraft entity, block entity renderer
or block entity ticker.

The root is invisible, non-colliding authoritative storage, not the visible
model and not the model's transform origin. During export it is placed in the
nearest free cell within or immediately around the transformed model bounds,
using a deterministic candidate order. The exact Source-derived transform stays
on the placement and is evaluated independently of the root cell. Export fails
with the prop's stable ID if no safe cell exists; roots never overwrite map
geometry, move the visible prop or disappear silently.

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

Automatic reconciliation is lazy on authoritative server chunk load; the
manual `/src2mc reconcile` command provides an explicit repair pass. Neither
path depends on WorldEdit events or client render distance.

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

## D13 — Content identity is derived from canonical entry payloads

Human-readable source names are provenance, not deduplication keys. Every
hashed table or asset type defines a canonical byte representation, including
float normalization and rejection of non-finite values. Content IDs derive from
those bytes. Bundle identity is a fingerprint of sorted entry paths,
uncompressed sizes and SHA-256 payload hashes. ZIP timestamps, permissions,
host metadata and compressed bytes are ignored by validation; whole-archive byte
identity is not a compatibility requirement.

## D14 — Reload publishes an immutable generation atomically

A candidate bundle generation is parsed, bounded, validated and prepared before
it becomes visible to lookup or rendering. Failure leaves the last known-good
generation active. Runtime handles identify their generation so asynchronous
chunk work cannot combine old tables with new textures or meshes.

## D15 — Per-cell collision preserves disconnected occupancy

The fixed 4x4x4 collision quantization is represented as subcell occupancy, not
as one bounding box around every triangle fragment in a block. Disconnected
occupied regions remain disconnected. Runtime shapes may merge adjacent
occupied subcells into fewer boxes only when the represented volume is
unchanged.
