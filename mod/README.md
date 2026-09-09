# src2mc NeoForge mod

The format, bundle loading, placement lookup, and mod-owned surface renderer
through Phase 4 are implemented and have passed real-map visual testing. Phase 5
static prop rendering is functional and under large-map performance work; root
lifecycle and missing-material handling remain incomplete. The mod registers
fixed generic world content plus `/src2mc status`, `validate`, `reload`, and
`reconcile`; client diagnostics include `/src2mc_render_status` and
`/src2mc_prop_status`. Surface rendering uses mod-owned paged textures rather
than Minecraft's block atlas. Collision comes in Phase 6.

The current implementation state, test paths, verified behavior, known defects,
and next work are recorded in [`../SESSION_HANDOFF.md`](../SESSION_HANDOFF.md).

This is a clean restart after an unplanned prototype was discarded. The current
requirements, architecture decisions and phased implementation sequence are:

- [`../docs/mod-requirements.md`](../docs/mod-requirements.md)
- [`../docs/decisions.md`](../docs/decisions.md)
- [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md)

The version-1 writer, converter `mod export` command, and Phase 2 mod-side
reader are implemented. The old surface-pool bundle format is
explicitly retired in [`../docs/format.md`](../docs/format.md).

Export one or several maps from the repository root with:

```sh
cargo run -- mod export --campaign hl2 --out out path/to/map1.bsp path/to/map2.bsp
```

## Requirements

- A Java 21 JDK.
- Network access the first time Gradle resolves the wrapper and NeoForge
  development dependencies.

## Build

From this directory:

```sh
./gradlew build
```

The development JAR is written to `build/libs/`.

Run the JVM unit tests with:

```sh
./gradlew test
```

## Run the development client

```sh
./gradlew runClient
```

The client uses `runs/client/` as its game directory. Create a new single-player
world there for the Phase 0 smoke test; the mod should load and make no changes.
This run intentionally contains only NeoForge and src2mc. It is not a
compatibility test for a real modpack.

Exact target versions and the testing arrangement for Sodium, Lithium, Create,
Create: Aeronautics, and Sable belong in
[`docs/compatibility-baseline.md`](docs/compatibility-baseline.md) once supplied
by the user.

Local screenshots, logs, and benchmark reports go in `local-reports/`, which is
ignored by Git. Do not add proprietary game assets to this project.
