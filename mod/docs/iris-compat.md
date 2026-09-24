# Iris compatibility

What a shaderpack does to the buffers this mod uploads, and what has to be done about it.

## The vertex format is not ours while a pack is loaded

Both renderers build their meshes with `DefaultVertexFormat.NEW_ENTITY` and keep the resulting
`VertexBuffer` for the life of a region (`MapSurfaceRenderer.upload`, `PropRenderer.upload`). With a
shaderpack in use, Iris changes that format out from under us in two separate places:

- `MixinBufferBuilder` — the `BufferBuilder` constructor substitutes `IrisVertexFormats.ENTITY` for
  `NEW_ENTITY`, adding a mid-texcoord, a tangent and an entity-id element.
- `MixinVertexFormat` — `NEW_ENTITY.setupBufferState()` binds the *extended* attribute layout for
  any buffer claiming that format, whether or not the buffer was written with the extra elements.

The second one is why Iris's own per-thread opt-out, `ImmediateState.skipExtension`, is a trap.
Setting it produces a 36-byte buffer that is later drawn with the 52-byte layout: every attribute
reads past its vertex and the map renders as stretched triangles across the whole screen. Do not
retry this. The extension has to be accepted; only the data Iris puts into it can be influenced.

## The entity id is captured, and it sticks

Iris fills the added entity-id element as each vertex is written, from
`CapturedRenderingState.INSTANCE` — the entity, block entity and item that were rendering at that
moment. For vanilla entity rendering that is correct and lasts one frame. For us it is written into
a static VBO that is never rebuilt, so a mesh built while, say, an armour stand was rendering is
labelled that entity for the rest of the session.

Shaderpacks act on that label. Complementary Reimagined with `ENTITY_SHADOW=-1` (entity shadows
off) drops entity-labelled geometry from the shadow map entirely, which showed up as: shadows
within a handful of blocks of the player and flat sunlight past that, permanently, healed only by
rebuilding every mesh with shaders disabled. Toggling shaders never helped because the bad data was
in our buffers, not in Iris's pipeline.

So both upload paths zero the three captured ids for the duration of a build and restore them
afterwards, through `IrisCompat.setCapturedIds` / `restoreCapturedIds`. `/src2mc_iris_entity_id
captured` restores the old behaviour for an A/B; `neutral` is the default.

## Everything here is reflective

`IrisCompat` binds Iris's public v0 API (`IrisApi.isShaderPackInUse`, `isRenderingShadowPass`) and
the two internals above by reflection, and fails soft to inert when Iris is absent or its shape
changed. Iris is a runtime mod with no artifact in this build, so there is no compile-time
dependency and no mixin of our own. `/src2mc_render_status` reports `(no hook)` when a binding did
not resolve, which is the first thing to check after an Iris update.

## Diagnosing the next one

The bugs in this area all look the same from the outside — geometry that draws but is shaded wrong
— so the status line carries the facts that separate them:

- `vertex strides={36=N}` — which layouts the built meshes actually hold. Anything other than a
  single value means meshes built under different pack states are in play at once.
- `built with a pack in use=N/M` — how many meshes were built with Iris active.
- `captured now=a/b/c` — the ids Iris would stamp into a buffer written this instant.
- `shadow draw set=drawn/considered (unbuilt N, too far M)` — what actually reached the shadow map
  last frame, which separates "the pack rejected it" from "we never submitted it".

Useful commands: `/src2mc_rebuild_meshes` drops every mesh without touching the generation, the sky
bake or the atlas, so a rebuild can be tested on its own — unlike `/src2mc reload`, which changes
all three at once. `/src2mc_client_light` reports the client's own sky-light bake, which the
server-side probes (`/src2mc lightmask`, `/src2mc lightcolumn`) never showed.
