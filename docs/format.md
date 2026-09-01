# src2mc mod interchange format

Status: **not defined yet**. Defining version 1 is Phase 1 of
[`mod/IMPLEMENTATION_PLAN.md`](../mod/IMPLEMENTATION_PLAN.md).

The previous document at this path described the abandoned first mod
prototype: a directory bundle, a fixed `src2mc:surface_N` material pool,
world-position triplanar UVs and a `src2mc:FormatVersion` schematic field. That
contract is retired. Existing prototype bundles and fixtures are not valid
inputs to the new mod, and migration compatibility is not required.

This file is intentionally a boundary rather than a speculative schema. Code
must not invent a format before Phase 1 defines and tests all of these together:

- the custom bundle extension and deterministic ZIP rules;
- manifest and schema versioning;
- stable content IDs and SHA-256 hashes;
- map, material, model, planar UV-region and diagnostics tables;
- texture entries and compact binary runtime meshes;
- map-anchor and prop-root NBT in Sponge v3 schematics;
- client/server ownership of each datum;
- validation limits and stable error codes;
- fixtures for valid, malformed, corrupt and unsupported inputs.

The fixed architectural constraints that the format must express are recorded
in [`mod-requirements.md`](mod-requirements.md) and
[`decisions.md`](decisions.md). Once Phase 1 is authorized, replace this file
with the normative version 1 specification in the same change that adds its
fixtures and implementations.
