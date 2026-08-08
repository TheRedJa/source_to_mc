# src2mc mod

The Minecraft side of [src2mc](../README.md). The converter turns Source Engine
maps into schematics and a bundle of textures and meshes; this mod reads them.

It exists because the converter's KubeJS output does not scale. One map produced
181 MB across about 40,000 files to describe 168 materials and 95 models, and
every new map needed a restart. This mod's job is to make the cost proportional
to the content: a fixed set of registered blocks, everything else data.

Status: **skeleton**. The build, the mod entry point and the format fixture
tests are here. Nothing reads a bundle yet.

## What it must do

See [`../docs/mod-requirements.md`](../docs/mod-requirements.md). In short:

- surfaces as a fixed pool of blocks, textures tiled by UV scale rather than by
  splitting into more blocks,
- props as block entity data, geometry baked into the chunk mesh, collision
  computed per cell at runtime,
- props movable, rotatable and rescalable in game, with edits that survive a
  save and a copy,
- new maps and textures via a resource reload, never a restart.

## Contract

[`../docs/format.md`](../docs/format.md) is normative and versioned.
`Src2mc.FORMAT_VERSION` here must equal `src2mc::FORMAT_VERSION` in the
converter; a test enforces it, and the fixtures under `../tests/fixtures` are
read by both sides.

## Build

```sh
./gradlew build
./gradlew runClient
```

Java 21, NeoForge 21.1.231, Minecraft 1.21.1.

## Licence

PolyForm Noncommercial 1.0.0, the same as the converter. Ships no game content.
