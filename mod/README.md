# src2mc NeoForge mod

Phase 0 is a deliberately empty NeoForge 1.21.1 / Java 21 project. It contains
no Source-map loading, rendering, converter integration, blocks, props, or
collision behavior.

This is a clean restart after an unplanned prototype was discarded. The current
requirements, architecture decisions and phased implementation sequence are:

- [`../docs/mod-requirements.md`](../docs/mod-requirements.md)
- [`../docs/decisions.md`](../docs/decisions.md)
- [`IMPLEMENTATION_PLAN.md`](IMPLEMENTATION_PLAN.md)

The new interchange format does not exist yet; defining it is Phase 1. The old
surface-pool bundle format is explicitly retired in
[`../docs/format.md`](../docs/format.md).

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
