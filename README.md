# src2mc

Convert Source Engine maps (`.bsp`) into Minecraft 1.21.1 schematics, so that
recreating a game's geometry does not start with days of manual blocking-out.

Built for recreating **Entropy: Zero** and **Entropy: Zero 2**, and tested
against every stock map in both, plus Half-Life 2.

## Status

Working today:

- Loads Source BSP v19/20/21 and extracts world and brush-entity geometry.
- Voxelizes brushes at a configurable scale (default 16 Source units per block).
- Maps brush contents to blocks: water, glass, grates, ladders; clip, areaportal
  and tool brushes are dropped.
- Chooses a block per surface from its material: glob rules first, then the
  texture's average colour. Ships with rules for Half-Life 2 and Entropy: Zero.
- Hollows out solid volumes so only surfaces are emitted.
- Writes Sponge Schematic **v3** `.schem` tiles plus a manifest and a WorldEdit
  paste script.
- Dumps every entity to JSON with positions in Minecraft coordinates.

Not implemented yet: displacements and static props.

## Usage

```sh
# What does this map contain, and what will converting it cost?
src2mc inspect  maps/ez2_c1_1.bsp

# What every material resolves to, and why.
src2mc materials maps/ez2_c1_1.bsp
src2mc materials maps/ez2_c1_1.bsp --stubs > my-rules.toml

# Entities only, no voxelization.
src2mc entities maps/ez2_c1_1.bsp --classname func_door -o doors.json

# Convert, split into 256-block tiles (the default).
src2mc convert  maps/ez2_c1_1.bsp -o out/ --units-per-block 16

# Bigger tiles, or the whole map as one schematic.
src2mc convert  maps/ez2_c1_1.bsp -o out/ --tile-size 1024
src2mc convert  maps/ez2_c1_1.bsp -o out/ --single
```

`--tile-size` accepts up to 32767, the schematic format's per-axis limit. The
practical ceiling is memory rather than the format: a schematic stores one entry
per cell including air, so a single file is capped at 400 million cells and
anything larger asks you to tile it. For reference, all of `d1_trainstation_02`
fits in one 596 x 196 x 912 schematic of 236 KB.

`inspect` first is the intended workflow: it is instant and tells you the block
dimensions, the Y range, and whether the map needs a custom dimension.

Output of `convert`:

| File | Contents |
|---|---|
| `<map>_x<i>_y<j>_z<k>.schem` | Sponge v3 tiles, aligned to a global lattice |
| `manifest.json` | Tile positions, sizes, block counts per block type |
| `paste.txt` | WorldEdit macro pasting every tile at its position |
| `entities.json` | Every Source entity, verbatim, with Minecraft coordinates |

Paste with WorldEdit or FAWE: `//schem load <tile>` then `//paste -a -o`.

The `-o` matters. Each tile's absolute corner is baked into the schematic's
`Offset`, and `-o` pastes it there. Plain `//paste` places the clipboard
relative to wherever you are standing, so if you move between tiles they end up
scattered at different positions and heights. `-a` skips air so tiles do not
erase their neighbours.

## Scale and world height

The default is 16 Source units per block, one Hammer grid square: the 72-unit
player becomes 4.5 blocks, so the world reads as roughly 2.5x upscaled but keeps
its detail. `--units-per-block 32` halves every dimension and looks closer to
vanilla proportions, at the cost of fine trim.

Many E:Z2 maps are tall. At 16 units/block `ez2_c4_1` needs 1090 blocks of
height and `ez2_c2_1` needs 1213 — far beyond vanilla's 384. This is not a
problem: a datapack `dimension_type` allows up to 4064 blocks
(`min_y` >= -2032, `height` <= 4064, both multiples of 16, `min_y + height - 1 <= 2031`).
Nothing is ever clamped or rescaled to fit; `inspect` reports the exact `min_y`
and `height` a map needs.

## Materials

Every surface gets its block from the material on the brush side that voxel is
nearest to. Two things decide it, in order.

**Rules** are ordered glob patterns; the first match wins. They exist for the
cases no colour could imply — bars must be bars, a ladder must be climbable,
glass must be see-through:

```toml
[[rule]]
match = ["*grate*", "*fence*", "*railing*"]
block = "minecraft:iron_bars"

[[rule]]
match = "wood/*"
auto = true          # match by colour, but only against wooden blocks
set = "wood"

[[rule]]
match = "tools/*"
skip = true
```

**Colour matching** handles everything else. It does not need a game install or
a VTF decoder: the map compiler already stores each texture's average colour in
the BSP, as the `reflectivity` radiosity bounces light with. That colour is
converted to Oklab and matched against a curated list of Minecraft blocks —
every one a full opaque cube with the same texture on all sides, that stays put
and does nothing on its own. No sand, no logs, no magma.

A rule's `set` narrows which blocks are eligible, which is what keeps wood
wooden while preserving how light or dark each texture is. That matters more
than it sounds: Entropy: Zero's metal ranges from near white to near black, and
a fixed `metal/* → iron_block` mapping flattens a whole map into one shade.

`src2mc materials <map>` shows every material with its colour, its block and
what decided it, busiest first. `--stubs` prints the colour-matched ones as
rules you can edit, and `--guessed` narrows the table to those. The built-in
rules route every material in all 170 stock Half-Life 2, Entropy: Zero and
Entropy: Zero 2 maps; only self-illuminated textures like monitors, which Source
stores no colour for, reach `fallback_block`.

Your own rules go before the built-in ones, so they win:

```sh
src2mc convert map.bsp --rules my-rules.toml -o out/
src2mc convert map.bsp --palette-set stone,concrete -o out/
src2mc convert map.bsp --no-builtin-rules -o out/
```

## Configuration

Every setting lives in one TOML file, and anything omitted keeps its default:

```sh
src2mc convert map.bsp -c my-config.toml -o out/
```

See `example-config.toml` for the full set with comments.

## Notes on Source BSP handling

Two things worth knowing if you work on this code:

- `vbsp` sorts its `leaves` vector by cluster, so its leaf indices do **not**
  match the indices BSP node children reference. Associating brushes with the
  model that owns them requires original leaf order, so `src/bsp/rawleaves.rs`
  reads that one lump directly.
- Shipped maps are not always valid UTF-8. `ez2_assassin_demo` has non-breaking
  spaces typed into a light's `_ambient` value, which `vbsp` rejects outright.
  The entity lump is repaired in memory, byte for byte, before parsing.
- Material paths in a compiled BSP are not the paths that were authored. A face
  lit by an `env_cubemap` is rewritten to `maps/<map>/<path>_<x>_<y>_<z>`, and a
  displacement blend texture gets a `_wvt_patch` suffix. Two thirds of Entropy:
  Zero's materials are patched this way, so rules would be useless without
  undoing it. The two nest, too: `d1_canals_01a` contains
  `maps/d1_canals_01a/maps/d1_canals_01a/nature/blendmudmud001a_wvt_patch_-1624_6208_7`.

## Building

```sh
cargo build --release
cargo test --release
```

Tests that need real maps look for an Entropy: Zero install and skip themselves
when it is absent.
