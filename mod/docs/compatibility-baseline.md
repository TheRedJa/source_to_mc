# Compatibility baseline

Status: placeholders only. This document makes **no compatibility claim**.

The exact target modpack versions and test arrangement must be supplied before
compatibility verification begins. Record them here, then perform the later
phase-specific checks.

| Mod | Target version | Verification status |
| --- | --- | --- |
| Sodium | 0.8.13-beta.2+mc1.21.1 (NeoForge) | Installed and confirmed loading in `runs/client` on 2026-08-11. Bundled Forgified FRAPI 3.4.1 and Sodium's FRAPI implementation are present. Static inspection found standard terrain buffers only for vanilla passes and no public custom terrain-pass registration API; custom mod-owned texture binding remains unverified pending the Phase 0.5 spike. |
| Iris | 1.8.14-beta.1+mc1.21.1 (NeoForge) | Installed alongside Sodium and confirmed loading in the same client log; src2mc rendering not tested. |
| Lithium | TBD | Not tested; no claim |
| Create | TBD | Not tested; no claim |
| Create: Aeronautics | TBD | Not tested; no claim |
| Sable | TBD | Not tested; no claim |

This Phase 0 project deliberately has no dependencies on these mods.
