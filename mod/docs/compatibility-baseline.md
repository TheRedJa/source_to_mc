# Compatibility baseline

Status: test-instance inventory and narrowly scoped evidence. Unverified rows
make **no compatibility claim**.

The exact target modpack versions and test arrangement must be supplied before
compatibility verification begins. Record them here, then perform the later
phase-specific checks.

| Mod | Target version | Verification status |
| --- | --- | --- |
| Sodium | 0.8.13-beta.2+mc1.21.1 (NeoForge) | Installed and confirmed loading. The Phase 0.5 mod-owned two-page renderer passed the user's in-game seam, UV, occlusion and distance checks on 2026-09-02. |
| Iris | 1.8.14-beta.1+mc1.21.1 (NeoForge) | Installed alongside Sodium and confirmed loading; no shader-pack compatibility claim yet. |
| Lithium | 0.15.4+mc1.21.1 (NeoForge) | Installed and confirmed loading; no behavior-specific claim yet. |
| Create | 6.0.10 (NeoForge) | Installed and confirmed loading. Its public contraption coordinate API was inspected from the exact local JAR. The Phase 0.5 custom-mesh probe passed rigid motion, speed/direction changes, disassembly and automatic reattachment checks without jitter or visual artifacts. This is feasibility evidence, not full compatibility. |
| Create: Aeronautics | 1.3.0 bundled (NeoForge) | Installed and confirmed loading; no behavior-specific claim yet. |
| Sable | 2.0.3 (NeoForge) | Installed and confirmed loading; no behavior-specific claim yet. |

src2mc deliberately has no compile-time dependencies on these mods. The local
client instance supplies them only for compatibility testing.
