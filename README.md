# src2mc

Convert Source Engine maps (`.bsp`) into Minecraft 1.21.1 schematics, so that
recreating a game's geometry does not start with days of manual blocking-out.

Built for recreating **Entropy: Zero** and **Entropy: Zero 2**, and tested
against every stock map in both, plus Half-Life 2. Portal, Portal 2 and INFRA
convert as well.

## Status

Working today:

- Loads Source BSP v19/20/21, and v22 as used by INFRA's branch, from a file or
  from inside a VPK — some games ship no loose maps at all.
- Voxelizes brushes at a configurable scale (default 16 Source units per block).
- Maps brush contents to blocks: water, glass, grates, ladders; clip, areaportal
  and tool brushes are dropped.
- Chooses a block per surface from its material: glob rules first, then the
  texture's average colour. Ships with rules for Half-Life 2 and Entropy: Zero.
- Voxelizes displacement terrain, backed into solid so it is not a shell.
- Places **every prop** the map has — the compiled `prop_static` lump *and* the
  entity lump's `prop_physics`, `prop_dynamic` and friends: the fences,
  railings, catwalks, crates, cars, doors and lamps that fill a map's rooms,
  none of which is in any brush lump.
- Draws those props as their **real triangle mesh**, not as cubes, through
  NeoForge's OBJ model loader — so a forklift is a forklift. The map's rotation
  is baked into the mesh so the prop can be an ordinary block the chunk
  absorbs, rather than an entity redrawn every frame. Large ones get invisible
  barriers to stand on.
- Leaves out the **3D skybox room**, the scale model of the horizon that would
  otherwise convert into a second, wrongly-sized map.
- Optionally extracts the map's **real textures** and emits them as Minecraft
  blocks through a generated KubeJS pack, each texture **split across as many
  blocks as it really covers** in the map.
- Fits half-height and stepped geometry to slabs and stairs.
- Hollows out solid volumes so only surfaces are emitted.
- Writes Sponge Schematic **v3** `.schem` tiles plus a manifest and a WorldEdit
  paste script.
- Writes moving brush entities (doors, platforms, trains) to their own
  schematics, so they do not seal the openings they belong to.
- Dumps every entity to JSON with positions in Minecraft coordinates.
- Converts whole campaigns at once, laid out side by side, and emits a
  dimension datapack tall enough to paste them into.

Not implemented yet: Entropy: Zero 2's MapBase-specific entities.

## Usage

```sh
# What does this map contain, and what will converting it cost?
src2mc inspect  maps/ez2_c1_1.bsp

# What every material resolves to, and why.
src2mc materials maps/ez2_c1_1.bsp
src2mc materials maps/ez2_c1_1.bsp --stubs > my-rules.toml

# Which materials have a real Source texture behind them.
src2mc textures maps/ez2_c1_1.bsp
src2mc textures maps/ez2_c1_1.bsp --missing

# Entities only, no voxelization.
src2mc entities maps/ez2_c1_1.bsp --classname func_door -o doors.json

# Convert, split into 256-block tiles (the default).
src2mc convert  maps/ez2_c1_1.bsp -o out/ --units-per-block 16

# Bigger tiles, or the whole map as one schematic.
src2mc convert  maps/ez2_c1_1.bsp -o out/ --tile-size 1024
src2mc convert  maps/ez2_c1_1.bsp -o out/ --single

# A whole campaign, laid out side by side, with a dimension to paste it into.
src2mc batch    maps/*.bsp -o out/ --spacing 256 --emit-dimension

# With the map's own textures, as generated blocks (needs KubeJS).
src2mc convert  maps/ez2_c1_1.bsp -o out/ --textures kubejs

# Maps that ship only inside a VPK. `maps` prints each one already in the
# archive:map form every other command takes.
src2mc maps    infra/pak02_dir.vpk
src2mc convert infra/pak02_dir.vpk:maps/infra_c1_m1_office.bsp -o out/
```

Every command that takes a map takes either form. The search path for textures
and models is rebuilt from the map's own `gameinfo.txt` either way, so a map
read out of an archive resolves its content exactly like a loose one.

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
| `entities/<class>_<name>_<n>.schem` | Moving brush entities, one file each |
| `dimension/` | A datapack dimension sized to the map (`--emit-dimension`) |
| `kubejs/` | Generated textured blocks and their script (`--textures kubejs`) |

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

## Terrain, doors and whole campaigns

**Displacements** are Source's terrain: a brush face subdivided into a grid of
displaced vertices. They are a heightfield rather than a solid, so they are
voxelized as triangles — using an exact separating-axis test against each voxel,
since sampling leaves holes where a triangle crosses a voxel corner, and holes
in terrain are what you notice by falling through them. The resulting surface is
one voxel thick, so `solidify` drives it a few voxels further in, along the
surface's own inward normal rather than downwards: displacements make cliffs and
ceilings as often as ground.

**Props** are everything a map puts *in* its rooms: the fences and railings
along a platform, the catwalks over the canals, the crates, radiators, lamps,
signs, cars and doors. None of the geometry is in any brush lump, so a map
converted from brushes alone is an accurate but empty shell. They arrive by two
routes: `prop_static` does not survive compilation as an entity — VBSP writes
the placements into the `sprp` game lump — while `prop_physics`, `prop_dynamic`
and their relatives stay in the entity lump as ordinary entities with a `model`
key, so anything naming a `.mdl` counts. `d1_trainstation_02` places 345
between them. Each model's `.mdl`, `.vvd` and `.dx90.vtx` are read off the same
search path the textures come from and LOD 0 is flattened to triangles.

The `sprp` lump is read out of the file directly rather than through `vbsp`,
because `vbsp`'s version table is wrong for the later versions: from version 7
up it takes the four bytes at offset 64 as a word of flags, and in the maps that
actually exist those bytes are the minimum and maximum CPU and GPU levels, which
are `0xFF` apiece when unset. Every flag then reads as set, `NO_DRAW` included,
and the map's static props are thrown away — all 328 of a Portal 2 map, and 6691
of INFRA's 8386 in `infra_c1_m1_office`. The real flags are the byte at offset
31 and have not moved since version 4; neither have the origin, the angles or
the model index, which are the first 26 bytes of every version. Everything that
differs between versions and between branches comes after them, so none of it
has to be understood — only stepped over, at a stride measured from the lump's
own count and length rather than looked up from a version. That is also the only
thing that reliably separates branches which share a version number and disagree
about the record.

A prop is the one thing in a Source map that was never designed for a grid, so
by default it is not put on one. Its triangles are written as a Wavefront
`.obj` and registered as a block whose model NeoForge's built-in OBJ loader
draws — so a car is a car rather than a lump of mismatched cubes. Two
conventions bite here and both are handled: one OBJ unit is one block, not the
1/16 a vanilla JSON model means, and a pack texture is a sprite on a shared
atlas where UVs past `0..1` read whatever was stitched next door rather than
wrapping, so a model that tiles its sheet gets the texture repeated into a
larger image and its coordinates divided to match.

Placing it at the map's own angle is a separate problem, since a block sits on
the grid facing one of four ways. A `minecraft:block_display` entity solves it
by rendering a blockstate under a free transformation, and that is what src2mc
used to do — at a cost that turned out to be the whole frame budget. A display
entity goes through the entity renderer every frame and is never baked into a
chunk's vertex buffer, so a few hundred props in view is a few hundred thousand
triangles resubmitted per frame, whatever culling mods are installed.

So the rotation is baked into the mesh instead. Each placement gets a model
whose coordinates already carry the map's angle and its position within a
block, and the prop becomes an ordinary block placed in a free cell inside its
own geometry — chunk-baked, free per frame, and lit face by face rather than by
the single cell it stands in. Placements that round to the same angle and
offset share one block, so a row of identical fence posts is one registration.

A block model may be drawn outside its own block, but not arbitrarily far.
Sodium packs each chunk vertex coordinate into 20 bits spanning −8 to +24
blocks from the section origin and masks away what does not fit, so a mesh
reaching past that is drawn correctly up to the limit and then folds back on
itself — which is what a gantry or a light shaft did, as a black sheet folded
over the map. A block can sit anywhere in its 16-block section, so 8 blocks
either way is the reach that is safe wherever it lands, and a prop bigger than
that is carried by several blocks instead, each drawing the part of the mesh
nearest it. Triangles too wide to fit in any one piece — a light shaft is often
a single pair of them — are split at their longest edge first, which is exact
on a flat triangle. `d1_trainstation_02` ends up with 321 of its 325 props
baked and 4 still entities; across 140 stock Half-Life 2 and Entropy: Zero
maps, 45,676 props bake and 531 do not, and no generated model reaches further
than the 8 blocks it may.

Splitting is what it costs: `d1_trainstation_02` registers 2195 blocks for its
props where one block per placement was 258, and its pack grows from 86 MB to
107 MB. Registration count itself is cheap — KubeJS loads tens of thousands of
blocks in a fraction of a second — so what grows is disk.

The block never replaces anything: it only ever takes a cell that is already
air, since taking one of the map's own would be a hole in whatever the prop
stands against. A prop with nowhere to put a block — and anything over
`bake_max_size` — keeps the old route, a display entity written into the
schematics' `Entities` list and into a `.mcfunction` of `summon` commands at
the same absolute coordinates, everything tagged `src2mc_<map>` so a bad paste
is one `/kill` away. `bake = false` puts every prop back on it.

Neither route has collision of its own, so props at least `collision_min_size`
units across (48 by default) are made solid separately: you can stand on a
container and walk through a traffic cone. That used to be a shell of invisible
barriers, one full cube per cell the surface passes through, which walks well
enough and is wrong in every detail — a catwalk floor three pixels thick
collides as a whole block, a railing as a wall. Physics mods make that worse
than untidy: Sable and Create: Aeronautics resolve contacts against block
shapes, so a map of cube-shelled props is a map of invisible boxes to catch on.

So each of those cells now gets a generated block shaped like the part of the
mesh inside it, found by clipping the prop's triangles to the cell and taking
what is left. Per cell rather than per prop, because Minecraft only tests blocks
within one block of whatever is moving: a shape describing geometry ten blocks
away is never consulted. The blocks are invisible — a blockstate pointing at a
model with no elements — and shared, since a shape is six numbers and thousands
of cells round to the same ones. `d1_trainstation_02` covers 44,762 cells with
8954 of them. Rounding is always outward, so a box is never smaller than the
geometry it stands for, and `collision_max_shapes` bounds how many distinct ones
a pack may register by rounding to a coarser grid until they fit — coarser being
more generous, never thinner. `[props] collision = "barrier"` asks for the old
cubes, `"none"` for nothing at all, and vanilla output uses cubes regardless,
having no pack to register a shape in.

Anything that cannot be
drawn as a mesh — a model heavier than `max_triangles`, a material with no
texture, or vanilla output, which has no pack to register meshes in — falls back
to the old behaviour of voxelizing the triangles, where a prop's material is a
material like any other and gets the same rules, colour matching and generated
block. Voxelized props are surfaces rather than solids, so a fence stays one
block thick.

`[props] min_size` drops anything under 12 units — maps are full of pebbles and
cans — and `max_size` is the lever for backdrop scenery, which is placed as
ordinary props thousands of units across and can be tens of thousands of blocks
of one dark material. It is off by default, because that scenery really is
there.

**The 3D skybox** is a map's model of its own horizon: a sealed room off in a
corner holding a miniature of the skyline, which the engine renders scaled up
and far away. Converted literally it is a second, wrongly-sized map, and the
void between the two rooms is most of the schematic's volume. Nothing in the
format marks that room, but the `sky_camera` stands inside it and nowhere else,
so it is found as the smallest island of touching brushes enclosing the camera —
with a cap on how much of the map that island may be, and a check that no player
start is inside it, because dropping the level would be the worse failure. On
`d1_trainstation_02` leaving it out takes the bounding volume from 349M blocks
to 149M, halves the number of schematic tiles, and brings the map inside a
vanilla world's height. 123 of the 216 stock maps have one. Turn it off with
`[contents] skip_3d_skybox = false`.

**Moving brush entities** get their own schematic each, under `entities/`. A
`func_door` pasted into the world is a slab sealing the doorway it should open,
so doors, rotating doors, movelinears and tracktrains are pulled out by default.
Configure it per classname under `[entities.classname_modes]`.

**`batch`** converts several maps into one output directory, offsetting each so
none overlaps another, and writes a combined `paste_all.txt` and `batch.json`.
Offsets come from each map's own footprint rather than a fixed stride, because a
stride large enough for the biggest map strands everything else in empty space.
`--layout stacked` puts them all at the origin instead, for comparing versions of
one map.

**`--emit-dimension`** writes a datapack next to the schematics defining a
dimension with the `min_y` and `height` the map needs, plus a void generator, so
there is nothing to dig out before pasting. This matters because WorldEdit drops
out-of-range blocks *silently*: without it you paste a tall map, walk in, and
find the top missing with no error anywhere.

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

## Real textures, as real blocks

By default a wall becomes the vanilla block closest to its average colour. With
`--textures kubejs` it becomes the actual Half-Life 2 concrete.

Minecraft cannot add blocks from a resource pack alone — a pack only retextures
blocks that already exist — so something has to register them, and
[KubeJS](https://modrinth.com/mod/kubejs) is the least intrusive way: its
`kubejs/assets/` folder loads exactly like a resource pack, and a generated
startup script registers each block under its own namespace, so nothing vanilla
is overwritten. Both halves are written into the output directory:

```
kubejs/assets/kubejs/textures/block/<id>.png
kubejs/startup_scripts/src2mc_blocks.js
```

Copy that `kubejs` folder into a NeoForge 1.21.1 instance next to `mods`, and
restart — KubeJS cannot hot-reload registrations. **A schematic converted this
way will not paste correctly without its pack**, so the required block ids are
listed in `manifest.json` and `paste.txt` says so.

The textures are not in the maps. A BSP's pakfile holds mostly the cubemap
*patch* stubs the compiler generated — `az_c4_4` ships 22 `.vmt` and 5 `.vtf` —
while the real textures live in the game's VPKs and a mod's loose `materials/`
folder. The search path is rebuilt from the map's own `gameinfo.txt`, so
Entropy: Zero 2's chain through `ez2/`, `mapbase/` and `hl2/` resolves without
configuration. `src2mc textures <map>` shows what was found and where — for
brush materials and for the ones static props bring with them, with how far
each texture will be split — and it resolves nearly every material in use, the
rest being render targets like `_rt_Camera` and water shaders that have no
`$basetexture` at all.

Rules still win where the *kind* of block matters: a grate stays `iron_bars`
rather than becoming an opaque cube with a grate painted on it. Alpha-tested
materials are registered `cutout` and translucent ones `translucent`, and an
alpha-tested texture is re-thresholded when downsampled — averaging a grate's
alpha to 16x16 otherwise makes every texel part-transparent, which cutout
rendering draws as a solid block.

### One texture, many blocks

A Source wall texture is not sized for one block. A 512-pixel concrete texture
at Hammer's default scale of 0.25 covers 2048 units of wall, which at 16 units
per block is eight blocks. Squeezing all 512 pixels onto every block face is
what made converted walls look like a smear.

A face does not store UVs; it stores two 4-vectors projecting a world position
straight into texel coordinates, and the length of each is texels per unit. So
the map itself says how many blocks one repeat of a texture covers. Each
texture is cut into that many pieces, one registered block apiece, and every
voxel takes the piece that really is in front of it — so the bricks line up
across the wall again.

**One tile is always one block.** A cap on tiles per axis is met by shortening
the *window* into the texture, never by widening the tiles. The alternative is
worse than the problem it solves: Highway 17's cliff blend spans 77 blocks, so
eight tiles stretched to fit would be flat ten-by-ten patches of identical
stone with a hard seam between them, which the eye finds instantly. Past the
cap only the first few blocks' worth of texels is used and that window repeats
— detail per block stays exactly right, and what is lost is the part of the
texture that never repeats anyway.

**The cost is capped by a block budget, not by a tile count.** What splitting
textures really costs is not disk — 100k blocks is about 60 MB of 16x16 PNGs —
but what a KubeJS instance pays to register them at startup. So the control is
`[materials] max_blocks`, default 100,000: textures are cut as finely as that
allows and no finer, which makes the same setting sensible for a single room
and for a whole campaign. `batch` plans one cap across every map before
cutting anything, since it merges them into a single pack.

Quality saturates well before the budget usually binds, because a texture is
never cut finer than its own resolution can feed — below one source texel per
output pixel a tile is upscaled mush rather than recovered detail. That is what
makes a generous ceiling safe: cost stops rising exactly where quality stops
improving. Entropy: Zero's 17 maps come to about 84k blocks with every texture
at full resolution, so the default lets the whole campaign through untouched.

**The tile size comes from the face, not the material.** One material is used
at several scales in the same map — Highway 17's `nature/cliffface001a` at six
of them, from a third of a texel per unit to two — so sizing tiles from the
material's typical scale leaves them too wide for every face using a larger
one, and that face comes out in 2x2 blocks of the same picture. Faces wanting
fewer texels per block than the tiles were cut at get the tile resized to the
block; faces wanting more are left exact, since advancing by more than one tile
per block shows no repeat.

Models have no texture scale to read — their UVs are an unwrap of the whole
sheet — so a prop's tile is interpolated across the triangle from its corner
coordinates, and how finely the sheet is cut is measured off the model's own
geometry rather than assumed. That matters more than it sounds:
`props_wasteland/rockcliff02a` stretches one sheet over 39 blocks of cliff, so
a fixed guess is out by a factor of several. A model's sheet is an atlas rather
than a repeating texture, so it is never windowed — a prop stretched past
`tile_max` keeps some repetition, which is what raising the cap buys.

Turn the whole thing off with `[materials] tile_textures = false`.

On `d1_trainstation_02` this is 198 materials registering about 20k blocks and
11 MB of 16x16 PNGs, with no measurable conversion cost.

## Sub-block detail

A block is a 1 m cube, so at 16 units/block every 8-unit step and kerb rounds
away. `[shapes] enabled` fits half-height and stepped geometry to **slabs and
stairs**, from a 2x2x2 occupancy mask recorded during voxelization and applied
after hollowing. On `d1_trainstation_02` that recovers about 7% of blocks as
slabs or stairs, with no measurable cost, and every schematic tool handles them
natively.

The mask is stored in the same sparse 16-cubed sections as the block grid.
Keying it per voxel in a hash map instead cost about 48 bytes each, which on
E:Z2's largest map was a 6 GB peak against 657 MB for the whole rest of the
conversion.

Only block families that have vanilla slab and stair variants can change;
anything else stays a full cube, and an unrecognised mask always stays a full
cube too — losing a step is far less noticeable than opening a hole in a wall.

This is vanilla-only. KubeJS 2101 registers blocks through exactly two
builders, `basic` and `detector`, so a generated textured block cannot be a
slab or a stair: `--textures kubejs` gives you the real textures and full cubes,
and vanilla mode gives you sub-block shapes.

Chisels & Bits was considered and rejected: every C&B block shares one id with
its shape in block-entity NBT, and WorldEdit copy/paste renders them invisible
([WorldEdit #2390](https://github.com/EngineHub/WorldEdit/issues/2390)). For
true detail everywhere, `--units-per-block 8` still works — E:Z2's tallest map
then needs ~2426 blocks of height, inside the 4064 a dimension allows.

## Configuration

Every setting lives in one TOML file, and anything omitted keeps its default:

```sh
src2mc convert map.bsp -c my-config.toml -o out/
```

See `example-config.toml` for the full set with comments.

## Notes on Source BSP handling

Five things worth knowing if you work on this code:

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
- A brush entity's geometry is **not in world space**. VBSP rewrites it to be
  relative to the entity's `origin` keyvalue and leaves the model's own stored
  origin at zero, so the coordinates in the plane lump have to have the entity
  origin added back. This is not a rare case: 103 of the 115 brush entity models
  in `d1_trainstation_02` are stored this way, and taken at face value every
  door, button, trigger and `func_brush` in a map piles up around wherever
  Source's origin happens to land.
- `gameinfo.txt` is not uniform across mods. Half-Life 2 writes `SearchPaths`
  unquoted, Entropy: Zero 2 writes `"SearchPaths"`, and E:Z2's content sits
  behind `|gameinfo_path|ez2/*` and `|all_source_engine_paths|mapbase/*`.
  Missing any of those finds nothing at all: before the parser handled them,
  E:Z2 resolved 0 of 127 materials rather than 125.

## The companion mod

The KubeJS output above works, but its cost is proportional to the wrong thing:
one map produced 181 MB across about 40,000 files to describe 168 materials and
95 models, and adding a map needs a restart. `mod/` holds a NeoForge mod that
fixes that by registering a fixed set of blocks and treating materials, models
and placements as data. It is a skeleton — the build and the format tests are
there, nothing reads a bundle yet.

The two halves are coordinated by three documents rather than by good
intentions:

- [`docs/format.md`](docs/format.md) — the interchange contract, versioned.
  Normative: where it and either implementation disagree, the document is right.
- [`docs/decisions.md`](docs/decisions.md) — settled questions and the
  measurement behind each, so neither side re-derives or contradicts them.
- [`docs/mod-requirements.md`](docs/mod-requirements.md) — what the mod is for.

`src2mc::FORMAT_VERSION` and `Src2mc.FORMAT_VERSION` must agree, and the golden
files under `tests/fixtures` are written by the converter's tests and read by
the mod's, so a change on either side that breaks the other fails in the same CI
run.

## Building

```sh
cargo build --release
cargo test --release
```

The mod builds separately, with Java 21:

```sh
cd mod && ./gradlew build
```

Tests that need real maps look for an Entropy: Zero install and skip themselves
when it is absent.

A prebuilt x86-64 Linux binary is on the
[releases page](https://github.com/TheRedJa/source_to_mc/releases), zipped with
the licence, the third-party notices, this README and `example-config.toml`.

## License

Copyright 2026 TheRedJa. [PolyForm Noncommercial 1.0.0](LICENSE): free to use,
modify and share for any noncommercial purpose. Redistributing any part of it
means passing on the licence and the notice in [NOTICE](NOTICE).

The crates src2mc is built from are compiled into its executable and keep their
own licences — all permissive, none copyleft. Their terms are collected in
[THIRD-PARTY.md](THIRD-PARTY.md), regenerated for every release by:

```sh
cargo install cargo-about --locked --features cli
cargo about generate about.hbs -o THIRD-PARTY.md
```

src2mc ships no game content. Textures, models and maps are read out of your own
installation of the game, and nothing of Valve's is redistributed with the tool
or with this repository.
