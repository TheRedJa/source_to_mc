# src2mc mod

The Minecraft side of [src2mc](../README.md). The converter turns Source Engine
maps into schematics and a bundle of textures and meshes; this mod reads them.

It exists because the converter's KubeJS output does not scale. One map produced
181 MB across about 40,000 files to describe 168 materials and 95 models, and
every new map needed a restart. This mod's job is to make the cost proportional
to the content: a fixed set of registered blocks, everything else data.

Status: **surfaces**. The mod mounts a bundle, registers the surface pool, and
draws a surface block from its material table entry with the texture repetition
expressed as a texture coordinate. Props, collision and in-game editing are not
started.

## Running it

Point the mod at a bundle (`docs/format.md` §4) in either of two ways:

- put it in `<gamedir>/src2mc-bundle`, or
- set `-Dsrc2mc.bundle=/path/to/bundle`.

`/src2mc status` says what was found and how much of the pool it uses;
`/src2mc check <file>` reads a schematic's header and refuses one whose format
version is not this mod's, naming both numbers.

Changing a texture or the material table and pressing `F3+T` shows the change.
Adding a map does not need a restart. Growing the surface pool does — it is
`Src2mc.SURFACE_POOL_SIZE`, a compile-time constant, and that is deliberate
(D6).

The dev client can drive itself, which is how the reload claim is checked:

```sh
./gradlew runClient -Psrc2mc.dev=true -Psrc2mc.world=demo -Psrc2mc.wall=64x32
```

`src2mc.dev` builds a wall of surface blocks in front of the player,
`src2mc.world` opens a save without touching the menu, and `src2mc.autoshot`
screenshots the wall, reloads resources through the same call `F3+T` makes, and
screenshots it again. All three are off unless asked for.

## Known gaps

- **Cutout and translucent materials cull their neighbours wrongly.** Whether a
  block occludes the face next to it is fixed when the registry freezes, and the
  pool is one index space (`docs/format.md` §2), so it cannot come from the
  material table. Every pool block is registered as a solid full cube, which is
  right for the solid materials the converter emits today and wrong for the
  other two render types. The fix is either a non-occluding pool with
  `Block#skipRendering` driven by the material table, or a second index range,
  and it is a format question rather than an implementation detail.
- **Texture size.** Bundle textures go on the vanilla block atlas. A campaign of
  source-resolution textures has not been stitched yet, and the atlas has a
  size limit.

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
