# Phase 0.5 feasibility evidence

Status: in progress. This records measured evidence and unresolved seams; it is
not a compatibility claim or a v1 format definition.

## Canonical visible faces

The converter retains contributing Source planes until final occupancy and
shape fitting are known. It then emits each actual exposed half-block patch
with its bounds, fitted shape, provenance, material and complete Source texel
projection. Candidate selection measures plane distance at each patch centre
and uses stable provenance as its tie-breaker. This prevents an internal slab
or stair patch from inheriting the side nearest an outer whole-face centre.

Automated adversarial fixtures cover:

- three materials and projections meeting at one voxel;
- overlap resolution in both merge orders;
- removal of stale records during hollowing;
- fitted stair provenance;
- partial-shape faces next to an occupied cell; and
- full-block shared-face culling.

Brushes and displacements both feed the records. Phase 1 still needs to assign
deduplicated material/UV-region IDs and choose their bounded integer widths.

## Representative HL2 inventory

Measured locally with:

```text
src2mc mod-inventory d1_trainstation_02.bsp
```

at the fixed 32 Source units per block:

| Measurement | Result |
| --- | ---: |
| Projected schematic bounds | 298 x 138 x 456 |
| Projected bounding volume | 18,752,544 cells |
| Output cells | 102,953 |
| Cells before hollowing | 243,845 |
| Canonical visible half-block patches | 777,359 |
| Source materials | 577 |
| Distinct resolved textures | 213 |
| Encoded VTF payload | 46.3 MiB |
| Decoded RGBA8 texture memory | 182.2 MiB |
| RGBA8 memory with complete mip chains | 242.9 MiB |
| Distinct prop models | 95 |
| Prop triangles, counted once per model | 39,724 |
| Mesh prop placements | 325 |
| Collision cells | 11,101 |

These are content measurements, not a residency budget. Texture memory is
deduplicated by resolved base-texture path. GPU estimates assume uncompressed
RGBA8 and a complete mip chain; driver allocation and page padding are not yet
included.

## Custom texture binding seam

The local instance contains Sodium `0.8.13-beta.2+mc1.21.1` and its Forgified
FRAPI implementation. Static inspection found only Sodium's standard solid,
cutout, and translucent terrain passes and no public arbitrary terrain-pass
registration API.

NeoForge 1.21.1 named model render types do not fill that gap: their chunk
render type must be one of Minecraft's existing chunk buffer layers. The
separately supplied entity render type can bind a mod-owned texture, but proving
that path would not prove chunk rendering. Therefore no entity or block-entity
renderer has been accepted as the Phase 0.5 spike.

The chosen backend is a parallel static section-mesh renderer owned by src2mc.

The first client probe partitions immutable VBOs by 16³ section and texture
page, binds two mod-owned textures outside the block atlas, and uses the
frustum supplied by NeoForge's `AFTER_SOLID_BLOCKS` render stage. Resource
replacement and teardown are covered by unit tests. On 2026-09-01 the complete
development instance reached the title screen with NeoForge 21.1.235, Sodium
0.8.13-beta.2 and Iris 1.8.14-beta.1; src2mc initialized without an exception.
That startup result does not establish visual correctness. The two-page probe
still requires the manual in-world inspection listed below before this spike
can be accepted.

The first in-world run exposed a missing per-draw `VertexBuffer.bind()` call:
OpenGL rejected every draw because no vertex-array object was active. The probe
rendered nothing and flooded the debug log with `GL_INVALID_OPERATION`. This is
fixed in the current probe, but the corrected path still requires the manual
inspection below.

Manual inspection command: `/src2mc_render_probe`. It creates a four-block-wide
wall six blocks ahead of the player, crossing the nearest section boundary. The
lower and upper rows use separate red and blue/green checkerboard pages. Verify:

- both rows appear at the coordinates printed in chat and remain stable while
  walking, turning, and crossing the section boundary;
- the middle section seam has no gap, overlap, flicker, or UV discontinuity;
- both sides render, with no mirrored/rotated checkerboard or swapped page;
- ordinary blocks correctly occlude the wall and the wall does not draw through
  the world; and
- disabling the probe removes every part without a crash or stale texture.

The probe uses NeoForge's world-render stage and camera frustum and does not mix
into Sodium's internal terrain pipeline.

## Manual render result

The user tested the corrected probe in the full development instance on
2026-09-02. Both texture pages rendered as distinct two-colour striped/checker
patterns on both sides. The wall showed no seam, flicker, stretching, filtering
artifact, distance instability, or unexpected occlusion, and toggling it off
removed all geometry. The deliberately selected diagnostic hues are not a
colour-fidelity reference; they prove independent page binding.

An ordinary block placed directly behind the zero-thickness test wall produced
z-fighting because its face was exactly coplanar. Converted surface/carrier
blocks must therefore remain visually empty, with the src2mc mesh as the sole
visual surface. The normal no-edit schematic workflow will not place a vanilla
face underneath it. The renderer will not hide deliberately coplanar user
geometry by shifting the Source surface or applying global depth bias.

The accepted probe partitions this wall into four section/page buffers: two
sections multiplied by two texture pages. It therefore submits four draw calls
when every part is in the frustum, and fewer when section culling removes one.

## Create contraption seam

The exact local Create 6.0.10 JAR exposes public contraption-local/world
coordinate transforms. `/src2mc_create_probe` finds the nearest assembled
contraption within 32 blocks and attaches gold, cyan and magenta mod-owned mesh
markers to up to three of its local block positions. It does not mix into
Create or make Create a hard dependency. A transform error disables the probe
after one log entry. Full-client startup passes; motion, rotation and
disassembly behavior require the user's in-game inspection.

The user verified that all three markers remained rigidly attached while the
contraption changed speed and rotation direction, with no jitter or rendering
artifact. Disassembly correctly removed them, but reassembly created a new
contraption entity and the first probe remained tied to the old UUID. The probe
now searches for a replacement containing the same saved local block positions
and reattaches automatically; this correction still needs an in-game retest.

The first marker colours also differed materially from the requested gold,
cyan and magenta. Minecraft's local `FastColor.ABGR32` source confirms the CPU
channel packing is correct. The three-texel diagnostic had placed UVs directly
on texel boundaries, so the corrected version samples only each texel's centre.
That did not alter the result. A follow-up comparison split each marker between
texture colour and vertex colour; both halves were identically wrong, proving
the shared entity shader path was responsible rather than texture upload.

The probe then switched to Minecraft's unlit position/texture/colour shader.
Its first attempt accidentally retained the larger `NEW_ENTITY` VBO format,
which made the shader read attributes at incorrect offsets and caused corrupted
colours plus screen-aligned fuzzy lines. Pairing the shader with its exact
`POSITION_TEX_COLOR` format fixed both defects. The user saw clean magenta,
cyan and gold markers with no visible boundary between the independently
coloured halves, no artifact lines, correct rigid motion, and automatic
reattachment after reassembly. The Create Phase 0.5 visual seam therefore
passes on the recorded test instance.

## Benchmark baseline

The feasibility machine is an Intel Core i7-13700K with 64 GiB system RAM and
an NVIDIA GeForce RTX 3060 (OpenGL driver string `610.57.04`). Current settings
are 16-chunk render distance, 8-chunk simulation distance, fancy graphics, four
mip levels, 180 FPS cap and VSync disabled. The 2 GiB Gradle-daemon cap is not
the client heap; the client heap must be explicit for a reproducible benchmark.

A cold run starts a fresh JVM, loads the fixed benchmark world, waits for chunk
loading to settle, and traverses a fixed route once. A warm run repeats that
route after all required sections and textures have been seen. Every result
records client `-Xms`/`-Xmx`, resolution, shader state, page residency,
frame-time percentiles, peak RAM and peak VRAM.
