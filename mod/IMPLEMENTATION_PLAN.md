# src2mc NeoForge mod — implementation plan

Status: current implementation sequence for the clean mod restart. Product
requirements live in [`../docs/mod-requirements.md`](../docs/mod-requirements.md)
and current architectural decisions in
[`../docs/decisions.md`](../docs/decisions.md). This file is planning only; it
does not authorize a phase until requested.

## Scope and fixed decisions

- Java 21, Minecraft 1.21.1, NeoForge only.
- Rust remains the only Source BSP conversion implementation.
- Converter output is one WorldEdit schematic per map plus one versioned,
  custom-extension ZIP campaign bundle for each batch conversion.
- The mod has a fixed registry of generic blocks. Content never creates
  registry entries.
- The ordinary workflow is bundle installation/reload, then WorldEdit paste.
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

## Phase 1 — Versioned bundle contract and converter export

1. Define the mod-export package extension and v1 manifest.
2. Specify stable content IDs and SHA-256 content hashes for deduplication.
3. Define JSON tables for maps, materials, models, planar UV regions, and
   diagnostics.
4. Define compact binary runtime mesh v1: positions, normals, UVs, submesh
   ranges, material references, bounds, and versioning.
5. Define schematic NBT for the map anchor and prop-root block entity.
6. Implement converter `mod export` output for a single map and multi-map
   batches. A batch writes one campaign bundle and a schematic per map.
7. Add deterministic Rust fixtures containing synthetic geometry plus a small
   real-map-derived fixture when permitted.

Exit criteria:

- Re-running an export produces byte-stable output where timestamps are
  excluded.
- Shared textures/models occur once in a multi-map bundle.
- Malformed, unsupported, and missing files have documented error codes.

## Phase 2 — Bundle discovery, validation, and diagnostics

1. Implement configured bundle-folder discovery.
2. Implement `/src2mc reload`, `/src2mc validate`, `/src2mc status`, and
   `/src2mc reconcile` command shells.
3. Load and validate ZIP manifests, hashes, schema versions, tables, meshes,
   and textures on the appropriate client/server side.
4. Add chat summaries, detailed logs, and opt-in verbose diagnostics.
5. Implement persistent missing-asset placeholders that retain original root
   and anchor data and resolve after a valid reload.
6. Add map-height validation and a warning that recommends the user-managed
   KubeJS height datapack without installing one.

Exit criteria:

- Corruption/version fixtures fail legibly without crashing Minecraft.
- A repaired/reloaded bundle heals placeholders without repasting.
- Ordinary reload does not change the registry size.

## Phase 3 — Generic blocks, map anchors, and surface lookup

1. Register the fixed generic block set: surface, map anchor, prop root,
   carrier/collision, and placeholder.
2. Implement anchor persistence at the original schematic origin.
3. Build a world-position-to-map-local-position index from anchors and map
   metadata.
4. Resolve surface material and planar UV-region data from that index without
   surface block entities.
5. Implement automatic and manual reconciliation of anchor/root-derived state
   after chunk load and WorldEdit operations.
6. Add fixtures for two independently pasted maps and two instances of one
   map in different world positions.

Exit criteria:

- Ordinary WorldEdit paste resolves surfaces after chunk load.
- WorldEdit copy/paste preserves anchors and roots; reconciliation recreates
  missing derived state.
- Overlapping map placements produce a clear diagnostic rather than ambiguous
  rendering.

## Phase 4 — Mod-owned texture backend and surface rendering

1. Implement texture analysis that calculates the required effective source
   resolution for 16x16 texels per projected world block.
2. Generate mod-owned atlas pages no larger than 4096x4096, including
   mipmaps/filtering and transparent-edge handling.
3. Implement the first paged-atlas surface model renderer and bind it through
   the NeoForge/Sodium-compatible chunk-model path.
4. Render solid and alpha-cutout materials; add the documented basic
   translucent fallback.
5. Implement UV transforms from planar region metadata and local map
   coordinates.
6. Log texture dimensions, page allocations, RAM/VRAM estimates, and timing.
7. Run an in-game screenshot test. If paged atlases visibly fail, stop and ask
   the user to confirm testing a texture-array backend before changing course.

Exit criteria:

- Texture orientation, phase, and repetition match the UV-region fixtures.
- More texture content than one 4096x4096 page renders without using or
  duplicating the vanilla block atlas.
- Solid/cutout real-map material fixtures render correctly.

## Phase 5 — Static prop rendering and lifecycle

1. Implement the non-ticking prop-root block entity schema: model ID,
   transform, material data, stable ID, and source metadata.
2. Implement the custom mesh loader and chunk-baked prop renderer, including
   arbitrary rotation, scale, normals, UVs, and multiple materials.
3. Partition large prop rendering into derived carrier coverage where Minecraft
   section/chunk rendering requires it, while retaining one logical root.
4. Implement root placement, break cleanup, and middle-click item metadata.
5. Implement placeholder props and detailed errors for missing model/material
   references.
6. Test WorldEdit paste/copy/cut, pick-block/re-place, reload, and reconciliation.

Exit criteria:

- Props create no permanent Minecraft `Entity`, no block entity renderer, and
  no block entity ticker.
- A multi-material rotated/scaled prop appears correctly after paste and after
  pick-block/re-place.
- Removing a root removes its carriers; placing a copied root recreates them.

## Phase 6 — Collision and static physics compatibility

1. Port/adapt the converter's mesh clipping logic into runtime carrier-cell
   collision generation.
2. Quantize generated collision conservatively to a 4x4x4 sub-block grid.
3. Use simple conservative boxes when mesh-derived collision is unnecessary.
4. Cache generated shapes by chunk/section and invalidate only affected props
   on reconciliation, placement, break, or reload.
5. Verify player collision against fence, railing, walkway, and container
   fixtures.
6. Verify static Sable collision uses the same carrier-cell representation.

Exit criteria:

- Players and static Sable test objects do not pass through the required
  fixtures.
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
5. Add end-to-end tests for converter fixture -> bundle -> reload -> WorldEdit
   paste -> copy -> reload -> reconcile.

Exit criteria:

- A representative prop moves with a Create contraption without corruption.
- Required test suite passes; optional dynamic-physics gaps are documented.

## Phase 8 — Real maps, benchmarks, and prototype release gate

1. With the user's local-game guidance, select representative real maps from
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
- Required tests in the current requirements and versioned format pass.

## Deferred work

- Source animations, gameplay entities, shader/lightmap fidelity, decals, and
  full prop-editing UI.
- Automatic glue command.
- Accurate collision while moving on Create/Sable/Aeronautics structures.
- Multiplayer asset synchronization, vanilla-client compatibility, migration
  tooling, streaming/unloading, and alternate licensed texture sources.

## Decision gates

Do not proceed past these gates without explicit confirmation:

1. Before compatibility verification: user supplies the exact target modpack
   versions and instance/testing arrangement.
2. If paged 4096x4096 custom atlases render incorrectly in-game: user confirms
   attempting texture arrays before a different rendering backend is chosen.
3. Before prototype acceptance: user reviews in-game FPS results on the
   agreed reference scenario.
