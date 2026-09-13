# Bundle loading

How `.src2mc` bundles get from `config/src2mc/bundles/` into a published
generation, and why it is arranged the way it is.

## What changed, and why

Loading used to happen only when an operator typed `/src2mc reload`. Until then
the active generation was 0, `WorldReconciler` early-returned, and both
renderers drew nothing — so every session began by running the command and
waiting through it. The wait was long, and most of it was work that taught the
loader nothing.

Now bundles load by themselves during game startup, on several threads, while
the game is at the main menu. `/src2mc reload` still exists and is unchanged; it
is for picking up a bundle that changed on disk mid-session.

Measured on seven installed bundles (INFRA and Portal 2 campaigns, 50 MB each),
three runs apiece:

| | ms |
| --- | --- |
| before | 13695 / 13380 / 13310 |
| after dropping the redundant PNG decode, single-threaded | 7895 / 7844 / 7784 |
| after that, on the loader pool | 2066 / 1885 / 1790 |

Roughly half the win is deleted work and half is threads.

## When it runs

`Src2mc` starts the load from `FMLCommonSetupEvent` and returns immediately.
The mod constructor is too early: `Src2mcConfig.bundleDirectory()` reads a
`ModConfigSpec` value, and the COMMON config is not loaded when the constructor
runs.

Nothing waits on the load. Every consumer already polls the generation sequence
on its own schedule — `MapSurfaceRenderer.renderOpaque` and
`PropRenderer.renderOpaque` per frame, `LightOcclusion.onServerTick` every 20
ticks — and the published `BundleGeneration` is a deeply immutable record behind
an `AtomicReference`, so a load that lands later is picked up without any
coordination.

The one exception is `ServerAboutToStartEvent`, which joins the outstanding load
with a timeout. A chunk is reconciled when it loads, once; entering a world
before the generation exists would leave every chunk loaded meanwhile
unreconciled. On a normal start this join has nothing to wait for.

A failed load is logged and leaves the active generation alone, exactly as the
command's failure path does.

## Parallelism

`BundleLoadPool` owns a `ForkJoinPool` of `min(availableProcessors, 8)` daemon
threads, named `src2mc-bundle-loader-N`. Work is nested three levels deep:

- `BundleRepository.validateCandidate` over the bundles in the directory,
- `BundleSchemaValidator.validate` over the maps in a campaign,
- `BundleSchemaValidator.validateMap` over the meshes in a map, and
  `BundleValidator.verifyContents` over the entries it hashes.

The third level is the one that matters for a campaign holding a single large
map, which is the common case.

ForkJoinPool rather than a fixed pool is not a preference: with nested work a
fixed pool deadlocks the moment the outer level fills it with tasks that are all
blocked on inner ones. A ForkJoin worker steals other tasks while it waits.
`BundleLoadPool.map` forks when it is already running on the pool and submits
otherwise, because a bare `ForkJoinTask.fork()` from a foreign thread would hand
the work to the common pool.

Two properties are deliberate and tested:

- **Results keep input order.** The generation's bundle list and its fingerprint
  are what a serial load would have produced.
- **The earliest failure is the one reported.** Failures are carried back as
  values rather than thrown across the task boundary — `ForkJoinTask` wraps a
  checked exception in a `RuntimeException`, and the caller wants its
  `IOException` — and the lowest-indexed one is rethrown. Which bundle a user is
  told about does not depend on thread timing.

Shared state that made the map loop sequential was removed rather than locked:
the referenced-payload set became a `ConcurrentHashMap.newKeySet()`, and the
model-count accumulation and the sorted-order check were hoisted out of the loop,
where they belong anyway.

## Deleted work

`BundleSchemaValidator.validatePng` used to parse the PNG's IHDR — which carries
width, height, bit depth and colour type — and then run a full `ImageIO.read`
over the image anyway, for every atlas page times every mip level, mip 0 being
4096², discarding the `BufferedImage`. The decode established nothing the header
had not: the payload bytes are already proven against the manifest hash by
`verifyContents`, and `AtlasPageResidency` decodes for real at render time and
re-checks the dimensions before it uploads. Only the header check remains.

## Progress

`BundleLoadProgress` holds one `volatile` immutable snapshot — state, bundles
done, bundles total, current campaign, elapsed millis, message — rewritten as
each bundle finishes. Readers never block and the loader pays one store per
bundle.

`BundleLoadOverlay` draws it, registered twice: as a GUI layer above the debug
overlay for in-world, and on `ScreenEvent.Render.Post` for menus, because GUI
layers do not draw at the title screen and the load mostly happens behind it. It
shows progress while loading, the finished line for a few seconds, and a
persistent red line on failure. `/src2mc status` reports the load state when no
generation is loaded.

## Memory

Parallel loading raises peak memory: several bundles inflate and build their
tables at once. A default-heap (512 MB) JVM runs out loading seven 50 MB bundles
together; the retained `BundleMap` tables dominate that and the old serial path
retained them too, but the transient part is new. Game heaps are far larger, so
this is a note rather than a limit.

## Verifying a change here

- `gradlew test` — `BundleLoadPoolTest` covers ordering, deterministic failure
  selection under a timing skew, nested progress and the trivial sizes;
  `BundleValidatorTest` covers a multi-bundle parallel load's fingerprint
  stability and filename ordering, the async publish, and a failed async load
  retaining the previous generation.
- `gradlew test --rerun -Dsrc2mc.testBundle=<path>` exercises a real exported
  bundle through the whole validator.
- In game: start the client with bundles installed and do not run
  `/src2mc reload`. The overlay should appear during startup, the log should
  carry `loaded generation N at startup`, and entering a world should show the
  map already placed and lit.
