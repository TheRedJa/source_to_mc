# src2mc mod — working agreement

This is the Minecraft side of `src2mc`. The Rust converter in the parent
directory writes maps; this mod reads them. One repository, one history, so the
two halves cannot work from different assumptions.

## Read these first, every session, from the files — not from memory

1. `../docs/format.md` — the interchange contract. **Normative.** Where it and
   any code disagree, the document is right and the code has a bug.
2. `../docs/decisions.md` — settled questions and the evidence that settled
   them. If a design idea contradicts an entry there, the entry wins until a new
   measurement replaces it. Do not re-derive what has already been measured.
3. `../docs/mod-requirements.md` — what this mod is for, R1 to R10.

Never work from a summary of these documents, including one written by another
agent. Summaries are exactly where assumptions diverge. Open the files.

## Ownership

| Side | Owns |
| --- | --- |
| Rust (`../src`) | Everything that produces output: schematics, bundles, fixtures. |
| Mod (here) | Everything that consumes it: reading, rendering, collision, in-game editing. |
| `../docs/format.md` | Shared. Neither side changes it alone. |

Do not change the Rust converter to work around a problem here without saying
so plainly — it is the other side of a contract, not an implementation detail
you can adjust.

## Changing the format

An interface change means, in one commit:

1. bump `FORMAT_VERSION` in `Src2mc.java` **and** `src2mc::FORMAT_VERSION` in
   `../src/lib.rs` — a test fails if they disagree,
2. regenerate fixtures: `UPDATE_FIXTURES=1 cargo test --test fixtures` in the
   parent directory, and read the diff,
3. update `../docs/format.md`,
4. add an entry to `../docs/decisions.md` if it settles a question.

Changes to rendering, to internals, or to the KubeJS output are not interface
changes and must not bump the version.

## Non-negotiables, with the measurements behind them

These come from `../docs/decisions.md`. They are the constraints most likely to
be violated by an otherwise reasonable design:

- **No entity, no `BlockEntityTicker` and no `BlockEntityRenderer` per
  placement.** Past roughly 500 loaded entities the frame cost stops being
  linear. Geometry goes in the chunk mesh via `IDynamicBakedModel` and
  `ModelData`. (D1, D7)
- **Triangles are free.** A map with 3000+ props rendered with no meaningful
  cost under Sodium. Do not propose decimation, level of detail or prop
  merging. (D2)
- **A chunk vertex reaches about 8 blocks**, so prop geometry is split across
  carrier cells. Carrier cells are derived state, computed at bake time, never
  written to a schematic. (D3)
- **Collision reaches exactly one block.** One prop-sized shape on one block is
  impossible, in vanilla and under Lithium. Collision is per cell. (D4)
- **Sable voxelizes the world per block**, so per-cell collision is also what
  the physics mods this project targets need. (D5)
- **Nothing content-dependent may be registered.** Registries freeze at startup;
  that is the entire reason the current output needs a restart. Materials,
  models and placements are data, and adding a map is a resource reload. (D6)

## Build

Gradle wrapper, pinned. ModDevGradle, NeoForge 21.1.231, Java 21.

```sh
./gradlew build          # compile and run tests
./gradlew test           # tests only, including the format fixtures
./gradlew runClient      # dev client
```

The tests read `../tests/fixtures` through the `src2mc.fixtures` system
property, set in `build.gradle`. There is one copy of the fixtures, in the
converter's tree, so they cannot go stale.

## Target environment

The author runs this alongside Sodium, Lithium, Create: Aeronautics and its
bundled Sable, on NeoForge 21.1.x for Minecraft 1.21.1. Compatibility with those
is a requirement, not a nice-to-have — see R8.
