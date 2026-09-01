# src2mc NeoForge mod — implementation plan

Status: current implementation sequence for the clean mod restart. Product
requirements live in [`../docs/mod-requirements.md`](../docs/mod-requirements.md)
and current architectural decisions in
[`../docs/decisions.md`](../docs/decisions.md). This file is planning only; it
does not authorize a phase until requested.

## Scope and fixed decisions

- Java 21, Minecraft 1.21.1, NeoForge only.
- Rust remains the only Source BSP conversion implementation.
- Mod export uses 32 Source units per Minecraft block and writes one WorldEdit
  schematic per map plus one versioned, custom-extension ZIP campaign bundle
  for each batch conversion. It preflights schematic limits and fails rather
  than silently tiling, truncating or rescaling an exceptional map.
- The mod has a fixed registry of generic blocks. Content never creates
  registry entries.
- The ordinary workflow is bundle installation/reload, then unrotated,
  unmirrored WorldEdit paste of the complete exported schematic. WorldEdit
  rotation, mirroring, partial copies and post-paste editing are out of scope.
- Props are non-ticking root block entities; rendering is chunk-baked. Derived
  carriers are invisible implementation state, rebuilt from roots.

## Phase 0 — Recreate the mod skeleton and lock the test environment

1. Scaffold a minimal NeoForge 1.21.1 / Java 21 project under `mod/`.
2. Add a documented local development launch configuration and a simple test
   world.
3. Add a compatibility manifest with placeholders for the exact versions of
   Sodium, Lithium, Create, Create: Aeronautics, and Sable.
4. Add a test-report directory ignored by Git for screenshots, logs, and
   benchmark results.

Exit criteria:

- `./gradlew build`, unit tests, and a dev client all start successfully.
- No compatibility claim is made until the user supplies the target modpack
  versions.

## Phase 0.5 — Prove the input truth and risky runtime seams

1. Define a canonical converter-side record for every visible Minecraft face:
   cell, block/shape face, Source provenance, material and full planar UV
   transform.
2. Prove that face provenance survives voxel overlap resolution, hollowing and
   slab/stair fitting with adversarial synthetic fixtures.
3. Inventory representative HL2 content at 32 units per block: distinct source
   textures and decoded dimensions, models/triangles, placements, visible
   faces, collision cells and projected schematic volume.
4. Build a disposable client spike that emits geometry through a named/custom
   render type which binds two mod-owned texture pages in one chunk. Test the
   actual binding path under vanilla and the Sodium version recorded in the
   compatibility baseline; ordinary vanilla-atlas quads do not satisfy this
   test.
5. Verify that Sodium retains the custom layer, texture binding and UVs, then
   extend the spike across a chunk/section boundary with one
   carrier-partitioned mesh and record chunk rebuild and culling behavior.
6. Run an early visual-only Create contraption experiment with a root and
   carriers. This is feasibility evidence, not a compatibility claim.
7. Record the benchmark machine, JVM heap, render settings and cold/warm
   measurement procedure.

Exit criteria:

- A multi-material corner and a fitted stair retain the correct material and UV
  projection on every visible face.
- Multi-page rendering works without entering the vanilla block atlas and its
  draw-call/page cost is measured.
- The content inventory gives encoded-size, decoded-RAM and mipmapped-VRAM
  estimates rather than inferring them from the old generated pack size.
- Any failed rendering or Create assumption is resolved in the decisions before
  the v1 format is locked.

## Phase 1 — Versioned bundle contract and converter export

1. Define the mod-export package extension and v1 manifest.
2. Define canonical entry payloads for every hashed type, including float
   normalization and non-finite-value rejection. Define stable content IDs,
   per-entry SHA-256 hashes and the sorted manifest fingerprint; ZIP container
   metadata and compressed bytes are outside identity.
3. Define map, material, model, canonical visible-face and diagnostics tables.
   Index faces by sparse 16x16x16 map-local section buckets. Each face stores
   compact references to deduplicated material and UV-region records rather
   than repeating a full transform.
4. Define compact binary runtime mesh v1: positions, normals, UVs, submesh
   ranges, material references, bounds, and versioning.
5. Define schematic NBT for the map anchor and prop-root block entity. Keep the
   root's storage cell separate from the exact Source-derived render transform.
6. Define validation limits for entry count, path length, table counts, mesh
   counts, texture dimensions, decoded bytes, nesting, ZIP expansion and total
   runtime allocation.
7. Implement converter `mod export` output for a single map and multi-map
   batches. A batch writes one campaign bundle and one 32-units-per-block
   schematic per map after dimension/volume preflight.
8. Place each prop root in the deterministically nearest free cell within or
   immediately around its transformed bounds. Never overwrite geometry or move
   the visible model; fail export with the stable prop ID if no safe cell exists.
9. Add deterministic Rust fixtures containing synthetic geometry plus a small
   real-map-derived fixture when permitted.

Exit criteria:

- Re-running an export produces identical canonical entry payloads and the same
  manifest fingerprint: sorted entry paths, uncompressed sizes and SHA-256
  hashes. ZIP timestamps, permissions, host metadata and compressed byte stream
  are not identity and are ignored by validation.
- Shared textures/models occur once in a multi-map bundle.
- Malformed, unsupported, and missing files have documented error codes.
- Every exported surface face traces back to an explicit converter record; no
  material or UV is reconstructed from a single per-voxel material guess.
- Root placement is byte-stable, never overwrites another block, and does not
  change the prop's render transform.

## Phase 2 — Bundle discovery, validation, and diagnostics

1. Register the minimal fixed generic block and block-entity types needed for
   surface, anchor, prop-root, carrier/collision and placeholder persistence.
2. Implement configured bundle-folder discovery.
3. Implement `/src2mc reload`, `/src2mc validate`, `/src2mc status`, and
   `/src2mc reconcile` command shells.
4. Reject unsafe ZIP paths and enforce all format limits before allocating full
   tables, decoded textures or meshes.
5. Load manifests, hashes, schema versions, tables, meshes and textures into an
   unpublished immutable candidate generation on the appropriate side.
6. Publish a fully valid generation atomically; a failed reload retains the
   last known-good generation and stale chunk work cannot mix generations.
7. Add chat summaries, detailed logs, and opt-in verbose diagnostics.
8. Persist missing anchor/root data in placeholders; healing becomes testable
   when the corresponding surface and prop behavior exists in later phases.
9. Add map-height validation and a warning that recommends the user-managed
   KubeJS height datapack without installing one.

Exit criteria:

- Corruption/version fixtures fail legibly without crashing Minecraft.
- A failed replacement leaves the last known-good generation active.
- Ordinary reload does not change the registry size.

## Phase 3 — Generic blocks, map anchors, and surface lookup

1. Implement anchor persistence at the original schematic origin.
2. Support translation of the complete schematic only. Diagnose anchorless
   pasted surface data instead of guessing its coordinate mapping. Rotation and
   mirroring are unsupported and unchecked; no detection or correct rendering
   is promised for a deliberately transformed paste.
3. Build a persistent world-level placement index from anchors and map metadata
   that remains usable when the anchor chunk is unloaded. Synchronize only the
   small immutable placement-index view required by the rendering client;
   multiplayer bundle/asset synchronization remains deferred.
4. Resolve canonical visible-face records from that index without surface block
   entities.
5. Reconcile anchor/root/placeholder-derived state lazily when its server chunk
   loads, plus the manual `/src2mc reconcile` command. Do not depend on
   WorldEdit events.
6. Add fixtures for two independently pasted maps and two instances of one
   map in different world positions.

Exit criteria:

- Ordinary WorldEdit paste resolves surfaces after chunk load.
- A second untransformed paste at another position resolves independently.
- Anchorless pasted surface data produces a clear unsupported diagnostic rather
  than incorrect materials or UVs.
- Overlapping map placements produce a clear diagnostic rather than ambiguous
  rendering.

## Phase 4 — Mod-owned texture backend and surface rendering

1. Implement texture analysis that calculates the required effective source
   resolution for 16x16 output texels per projected world block, while recording
   the original source dimensions and every resampling decision.
2. Generate mod-owned atlas pages no larger than 4096x4096. Each texture
   allocation is wholly contained by one page and never spans a page boundary;
   an allocation too large for a page follows a documented reduce/split/reject
   rule. Add extruded gutters sufficient for every mip level, or prove an
   equivalent per-allocation mip-clamp policy, plus page-to-chunk dependency
   data.
3. Implement the first paged-atlas surface model renderer and bind it through
   the NeoForge/Sodium-compatible chunk-model path.
4. Render solid and alpha-cutout materials; add the documented basic
   translucent fallback.
5. Implement UV transforms from planar region metadata and local map
   coordinates.
6. Load metadata eagerly but decode/upload pages asynchronously as referencing
   chunks approach render distance. Track loaded-chunk references, apply a grace
   period before pages become evictable, and use LRU eviction under RAM/VRAM
   pressure.
7. Render a diagnostic placeholder while a page is loading and invalidate only
   its dependent chunk meshes when ready. Camera direction alone must not evict
   pages.
8. Enforce the agreed residency budget and log source/output dimensions, page
   allocations, cache hits/misses/evictions, encoded bytes, decoded RAM,
   mipmapped VRAM and timing.
9. Run an in-game screenshot test. If paged atlases visibly fail, stop and ask
   the user to confirm testing a texture-array backend before changing course.

Exit criteria:

- Texture orientation, phase, and repetition match the UV-region fixtures.
- More texture content than one 4096x4096 page renders without using or
  duplicating the vanilla block atlas.
- Approaching a textured chunk prefetches its pages; leaving all chunks that use
  them makes the pages evictable only after the grace period.
- Loading or evicting one page rebuilds only dependent chunks and stays within
  the agreed RAM/VRAM budgets.
- Solid/cutout real-map material fixtures render correctly.
- Every unsupported shader/material feature follows a documented deterministic
  fallback and contributes to a coverage/diagnostic report.

## Phase 5 — Static prop rendering and lifecycle

1. Implement the invisible, non-colliding, non-ticking prop-root block entity
   schema: model ID, exact render transform, material data, stable ID and source
   metadata. The root cell is authoritative storage, not the model origin.
2. Implement the custom mesh loader and chunk-baked prop renderer, including
   arbitrary rotation, scale, normals, UVs, and multiple materials.
3. Partition large prop rendering into derived carrier coverage where Minecraft
   section/chunk rendering requires it, while retaining one logical root.
4. Implement root placement, break cleanup, and middle-click item metadata.
5. Implement placeholder props and detailed errors for missing model/material
   references.
6. Test ordinary WorldEdit paste, pick-block/re-place, reload, and
   reconciliation. WorldEdit copy/cut and transformed paste are not supported.

Exit criteria:

- Props create no permanent Minecraft `Entity`, no block entity renderer, and
  no block entity ticker.
- A multi-material rotated/scaled prop appears correctly after paste and after
  pick-block/re-place.
- A root stored away from the model origin renders the mesh at its exact
  Source-derived transform.
- Removing a root removes its carriers; placing a copied root recreates them.

## Phase 6 — Collision and static physics compatibility

1. Port/adapt the converter's mesh clipping logic into runtime carrier-cell
   collision generation.
2. Quantize generated collision conservatively to a 4x4x4 sub-block grid without
   joining disconnected geometry into unintended solid obstacles.
3. Use simple conservative boxes when mesh-derived collision is unnecessary.
4. Cache generated shapes by chunk/section and invalidate only affected props
   on reconciliation, placement, break, or reload.
5. Verify player collision against fence, railing, walkway, and container
   fixtures.
6. Verify static Sable collision uses the same carrier-cell representation.

Exit criteria:

- Players and static Sable test objects do not pass through the required
  fixtures.
- Intended fence/railing openings remain passable, thin walkways do not become
  full-height obstacles, and oversized collision error is measured.
- Collision has no global arbitrary shape-count limit.
- Resource exhaustion is measured and logged; it does not silently corrupt or
  discard props.

## Phase 7 — Create baseline and regression coverage

1. Test manually gluing representative root/carrier props into Create
   contraptions.
2. Ensure assembly/disassembly does not corrupt roots, metadata, or world
   blocks.
3. Accept temporarily absent/imperfect collision while moving, but record it
   visibly in the compatibility report.
4. Add a safe unsupported-mode diagnostic for unimplemented Sable/Aeronautics
   dynamic behavior.
5. Add end-to-end tests for converter fixture -> bundle -> reload -> ordinary
   WorldEdit paste -> reload -> reconcile.

Exit criteria:

- A representative prop moves with a Create contraption without corruption.
- Required test suite passes; optional dynamic-physics gaps are documented.

## Phase 8 — Real maps, benchmarks, and prototype release gate

1. Using the benchmark definition established in Phase 0.5 and the user's
   local-game guidance, select representative real maps from
   HL2, Portal 1/2, INFRA, Entropy: Zero 1/2. Keep proprietary assets out of
   distributable fixtures unless explicitly permitted.
2. Measure single-map and campaign-bundle load/reload time, RAM, texture-page
   use, and dense-view FPS/frame time.
3. Test height warnings with a tall map.
4. Run manual in-game checks for WorldEdit, Sodium, Lithium, Create, Sable,
   and the supplied modpack versions.
5. Record known visual fallbacks: translucent behavior, deferred Source
   effects, and moving-physics collision limitations.

Prototype release criteria:

- Ordinary bundle load is at most one minute on the agreed reference machine.
- An ordinary reload adds no more than 30 seconds beyond NeoForge's baseline.
- Dense-view FPS/frame time is measured against vanilla/modpack baseline; the
  approximate 20% target is reviewed by the user in-game.
- The report also records bundle size, decoded RAM, mipmapped VRAM, page count,
  world-save growth, roots/carriers, chunk-build time and reload pause.
- Required tests in the current requirements and versioned format pass.

## Deferred work

- Source animations, gameplay entities, shader/lightmap fidelity, decals, and
  full prop-editing UI.
- Automatic glue command.
- Accurate collision while moving on Create/Sable/Aeronautics structures.
- Multiplayer asset synchronization, vanilla-client compatibility, migration
  tooling, non-texture asset streaming/unloading, and alternate licensed
  texture sources.

## Decision gates

Do not proceed past these gates without explicit confirmation:

1. Before compatibility verification: user supplies the exact target modpack
   versions and instance/testing arrangement.
2. Before Phase 1 locks the format: the Phase 0.5 rendering and content-inventory
   evidence supports the chosen texture backend and memory-residency policy.
3. If paged 4096x4096 custom atlases render incorrectly in-game: user confirms
   attempting texture arrays before a different rendering backend is chosen.
4. Before prototype acceptance: user reviews in-game FPS results on the
   agreed reference scenario.
