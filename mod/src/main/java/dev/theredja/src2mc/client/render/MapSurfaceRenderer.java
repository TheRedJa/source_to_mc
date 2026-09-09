package dev.theredja.src2mc.client.render;

import static net.minecraft.commands.Commands.literal;

import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexBuffer;
import com.mojang.blaze3d.vertex.VertexFormat;
import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.bundle.AtlasIndex;
import dev.theredja.src2mc.bundle.BundleGeneration;
import dev.theredja.src2mc.bundle.BundleManifest;
import dev.theredja.src2mc.bundle.BundleMaterial;
import dev.theredja.src2mc.bundle.BundleMap;
import dev.theredja.src2mc.bundle.SurfaceTable;
import dev.theredja.src2mc.network.PlacementNetwork;
import dev.theredja.src2mc.world.MapPlacement;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.network.chat.Component;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.HitResult;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.EventPriority;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;
import net.neoforged.neoforge.client.event.RegisterClientCommandsEvent;
import org.joml.Matrix4f;

/** Mod-owned static section/page renderer; independent of Sodium terrain internals. */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class MapSurfaceRenderer {
    private static final int SECTION_BUILDS_PER_FRAME = 2;
    private static final long MESH_GRACE_FRAMES = 600;
    private static final AtlasPageResidency PAGES = new AtlasPageResidency();
    private static final Map<MeshKey, Mesh> MESHES = new LinkedHashMap<>();
    private static final Map<SectionKey, Long> BUILT_SECTIONS = new HashMap<>();
    private static ClientLevel level;
    private static long generationSequence = -1;
    private static List<MapPlacement> placementSnapshot = List.of();
    private static long frame;

    private MapSurfaceRenderer() {}

    /** Shared campaign atlas residency for every mod-owned static renderer. */
    static AtlasPageResidency atlasPages() {
        return PAGES;
    }

    @SubscribeEvent
    public static void registerCommand(RegisterClientCommandsEvent event) {
        event.getDispatcher().register(literal("src2mc_debug_face").executes(context -> inspectFace(context.getSource())));
        event.getDispatcher().register(literal("src2mc_render_status").executes(context -> renderStatus(context.getSource())));
    }

    private static int renderStatus(net.minecraft.commands.CommandSourceStack source) {
        AtlasPageResidency.Stats stats = PAGES.stats();
        source.sendSuccess(() -> Component.literal("src2mc render: sections=" + BUILT_SECTIONS.size()
            + ", meshes=" + MESHES.size() + ", atlas resident=" + stats.residentPages() + "/" + stats.trackedPages()
            + ", vram=" + formatBytes(stats.residentVramBytes()) + ", decode-pending=" + formatBytes(stats.pendingRamBytes())
            + ", requests=" + stats.requests() + " (hit=" + stats.hits() + ", miss=" + stats.misses()
            + ", denied=" + stats.denied() + ", evicted=" + stats.evictions() + ", failed=" + stats.decodeFailures() + ")"), false);
        return 1;
    }

    private static int inspectFace(net.minecraft.commands.CommandSourceStack source) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null || minecraft.hitResult == null
            || minecraft.hitResult.getType() != HitResult.Type.BLOCK) {
            source.sendFailure(Component.literal("Look directly at a src2mc surface block first"));
            return 0;
        }
        BlockHitResult hit = (BlockHitResult) minecraft.hitResult;
        BlockPos world = hit.getBlockPos();
        var placement = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).at(world).orElse(null);
        if (placement == null) {
            source.sendFailure(Component.literal("No unambiguous src2mc placement contains " + world.toShortString()));
            return 0;
        }
        BundleMap map = Src2mc.bundles().active().findMap(placement.campaignId(), placement.mapId()).orElse(null);
        if (map == null) {
            source.sendFailure(Component.literal("The placement's map is absent from the active generation"));
            return 0;
        }
        BlockPos local = placement.toLocal(world);
        List<SurfaceTable.Face> faces = map.surfaces().facesAt(local.getX(), local.getY(), local.getZ());
        source.sendSuccess(() -> Component.literal("src2mc face debug: world=" + world.toShortString()
            + ", local=" + local.toShortString() + ", hit=" + hit.getDirection() + ", records=" + faces.size()), false);
        for (SurfaceTable.Face face : faces) {
            BundleMaterial material = map.materials().get(face.materialId());
            String texture = material.texture() == null ? "none" : material.texture().contentId().substring(0, 12);
            SurfaceTable.UvRegion region = map.surfaces().uvRegions().get(face.uvRegionId());
            double[] uv = region.values();
            String projection = String.format(java.util.Locale.ROOT,
                "uv-rate=[%.4f,%.4f] output=%s",
                Math.sqrt(uv[0] * uv[0] + uv[1] * uv[1] + uv[2] * uv[2]),
                Math.sqrt(uv[4] * uv[4] + uv[5] * uv[5] + uv[6] * uv[6]),
                material.texture() == null ? "none" : material.texture().originalWidth() + "x" + material.texture().originalHeight()
                    + "->" + material.texture().outputWidth() + "x" + material.texture().outputHeight());
            source.sendSuccess(() -> Component.literal("  patch=" + face.patch() + " dir=" + patchDirection(face.patch())
                + " plane=" + patchPlane(face.patch())
                + " material=" + face.materialId() + " " + material.sourceMaterial()
                + " class=" + material.renderClass() + " texture=" + texture
                + " source=" + face.provenance() + ":" + Long.toUnsignedString(face.sourcePrimary())
                + ":" + Long.toUnsignedString(face.sourceSecondary()) + " " + projection), false);
        }
        return 1;
    }

    private static String patchDirection(int patch) {
        return switch (patch & 7) {
            case 0 -> "down"; case 1 -> "up"; case 2 -> "north";
            case 3 -> "south"; case 4 -> "west"; case 5 -> "east"; default -> "invalid";
        };
    }

    private static String patchPlane(int patch) {
        return String.format(java.util.Locale.ROOT, "%.1f", ((patch >>> 3) & 3) * 0.5);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void render(RenderLevelStageEvent event) {
        if (event.getStage() == RenderLevelStageEvent.Stage.AFTER_SOLID_BLOCKS) {
            renderOpaque(event);
        } else if (event.getStage() == RenderLevelStageEvent.Stage.AFTER_PARTICLES) {
            renderTranslucent(event);
        }
    }

    private static void renderOpaque(RenderLevelStageEvent event) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null) { clear(); return; }
        BundleGeneration generation = Src2mc.bundles().active();
        List<MapPlacement> placements = PlacementNetwork.clientIndex(minecraft.level.dimension().location()).view();
        if (minecraft.level != level || generation.sequence() != generationSequence || !placements.equals(placementSnapshot)) {
            clear();
            level = minecraft.level;
            generationSequence = generation.sequence();
            placementSnapshot = placements;
        }
        frame++;
        PAGES.pump(frame);
        invalidateReadyPages(PAGES.drainReadyPages());
        if (generation.sequence() == 0 || placements.isEmpty()) return;

        var camera = event.getCamera().getPosition();
        int cameraSectionX = SectionPos.blockToSectionCoord(camera.x);
        int cameraSectionZ = SectionPos.blockToSectionCoord(camera.z);
        int distance = minecraft.options.getEffectiveRenderDistance() + 1;
        int builds = 0;
        for (MapPlacement placement : placements) {
            var located = generation.findLocatedMap(placement.campaignId(), placement.mapId()).orElse(null);
            if (located == null || located.map().atlas() == null) continue;
            BundleMap map = located.map();
            for (var sectionEntry : map.surfaces().sections().entrySet()) {
                SurfaceTable.SectionPos localSection = sectionEntry.getKey();
                AABB bounds = sectionBounds(placement, localSection);
                int sectionX = SectionPos.blockToSectionCoord((bounds.minX + bounds.maxX) * 0.5);
                int sectionZ = SectionPos.blockToSectionCoord((bounds.minZ + bounds.maxZ) * 0.5);
                if (Math.abs(sectionX - cameraSectionX) > distance || Math.abs(sectionZ - cameraSectionZ) > distance) continue;
                SectionKey sectionKey = new SectionKey(placement, localSection);
                if (!BUILT_SECTIONS.containsKey(sectionKey) && builds < SECTION_BUILDS_PER_FRAME) {
                    buildSection(located.bundle(), map, placement, localSection, sectionEntry.getValue());
                    builds++;
                }
                if (BUILT_SECTIONS.containsKey(sectionKey)) BUILT_SECTIONS.put(sectionKey, frame);
                MESHES.forEach((key, mesh) -> { if (key.section.equals(sectionKey)) mesh.lastVisibleFrame = frame; });
            }
        }
        discardExpiredMeshes();
        prefetchNearMeshes(generation);
        PAGES.pump(frame);
        invalidateReadyPages(PAGES.drainReadyPages());

        draw(event, generation, false);
    }

    /** NeoForge documents AFTER_PARTICLES as the safe basic custom-translucency stage. */
    private static void renderTranslucent(RenderLevelStageEvent event) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft.level == null || generationSequence < 0) return;
        draw(event, Src2mc.bundles().active(), true);
    }

    private static void draw(RenderLevelStageEvent event, BundleGeneration generation, boolean translucent) {
        var camera = event.getCamera().getPosition();
        var drawItems = MESHES.entrySet().stream()
            .filter(item -> item.getValue().lastVisibleFrame == frame)
            .filter(item -> (item.getKey().renderClass == BundleMaterial.RenderClass.TRANSLUCENT) == translucent)
            .filter(item -> event.getFrustum().isVisible(item.getValue().bounds))
            .sorted(translucent ? Comparator.<Map.Entry<MeshKey, Mesh>>comparingDouble(item -> -distanceSquared(item.getValue().bounds, camera)) : (left, right) -> 0)
            .toList();
        for (var item : drawItems) {
            Mesh mesh = item.getValue();
            ResourceLocation texture = PAGES.request(generation.sequence(), mesh.bundle, mesh.atlas, item.getKey().page, frame)
                .orElseGet(PAGES::placeholderTexture);
            RenderType renderType = translucent ? RenderType.entityTranslucent(texture)
                : item.getKey().renderClass == BundleMaterial.RenderClass.SOLID ? RenderType.entitySolid(texture) : RenderType.entityCutout(texture);
            renderType.setupRenderState();
            Matrix4f modelView = new Matrix4f(event.getModelViewMatrix()).translate(
                (float) (mesh.origin.getX() - camera.x),
                (float) (mesh.origin.getY() - camera.y),
                (float) (mesh.origin.getZ() - camera.z));
            mesh.buffer.bind();
            mesh.buffer.drawWithShader(modelView, event.getProjectionMatrix(),
                translucent ? GameRenderer.getRendertypeEntityTranslucentShader()
                    : item.getKey().renderClass == BundleMaterial.RenderClass.SOLID
                        ? GameRenderer.getRendertypeEntitySolidShader() : GameRenderer.getRendertypeEntityCutoutShader());
            renderType.clearRenderState();
        }
        VertexBuffer.unbind();
    }

    private static void prefetchNearMeshes(BundleGeneration generation) {
        for (var item : MESHES.entrySet()) {
            Mesh mesh = item.getValue();
            if (mesh.lastVisibleFrame == frame) {
                PAGES.request(generation.sequence(), mesh.bundle, mesh.atlas, item.getKey().page, frame);
            }
        }
    }

    private static void invalidateReadyPages(List<AtlasPageResidency.PageKey> ready) {
        if (ready.isEmpty()) return;
        var affected = new HashSet<SectionKey>();
        for (var item : MESHES.entrySet()) for (AtlasPageResidency.PageKey page : ready) {
            if (item.getKey().page == page.page() && item.getValue().bundle.fingerprint().equals(page.fingerprint())) {
                affected.add(item.getKey().section);
            }
        }
        if (affected.isEmpty()) return;
        affected.forEach(BUILT_SECTIONS::remove);
        MESHES.entrySet().removeIf(item -> {
            if (!affected.contains(item.getKey().section)) return false;
            item.getValue().close();
            return true;
        });
    }

    private static double distanceSquared(AABB bounds, net.minecraft.world.phys.Vec3 camera) {
        double x = (bounds.minX + bounds.maxX) * 0.5 - camera.x;
        double y = (bounds.minY + bounds.maxY) * 0.5 - camera.y;
        double z = (bounds.minZ + bounds.maxZ) * 0.5 - camera.z;
        return x * x + y * y + z * z;
    }

    private static String formatBytes(long bytes) {
        if (bytes < 1024 * 1024) return (bytes / 1024) + " KiB";
        return String.format(java.util.Locale.ROOT, "%.1f MiB", bytes / (1024.0 * 1024.0));
    }

    private static void buildSection(BundleManifest bundle, BundleMap map, MapPlacement placement,
                                     SurfaceTable.SectionPos section, List<SurfaceTable.Face> faces) {
        Map<PageClass, List<SurfaceTessellator.Triangle>> triangles = new HashMap<>();
        int baseX = section.x() << 4, baseY = section.y() << 4, baseZ = section.z() << 4;
        for (SurfaceTable.Face face : faces) {
            if (face.materialId() < 0 || face.materialId() >= map.materials().size()
                || face.uvRegionId() < 0 || face.uvRegionId() >= map.surfaces().uvRegions().size()) continue;
            BundleMaterial material = map.materials().get(face.materialId());
            if (!material.textured() || material.renderClass() == BundleMaterial.RenderClass.FALLBACK) continue;
            AtlasIndex.Texture texture = map.atlas().textures().get(material.texture().contentId());
            if (texture == null) continue;
            int local = face.localCell();
            int x = baseX + (local & 15), y = baseY + (local >> 8 & 15), z = baseZ + (local >> 4 & 15);
            for (var triangle : SurfaceTessellator.tessellate(x, y, z, face,
                map.surfaces().uvRegions().get(face.uvRegionId()), material.texture(), texture, map.atlas().pageSize())) {
                triangles.computeIfAbsent(new PageClass(triangle.page(), material.renderClass()), ignored -> new ArrayList<>()).add(triangle);
            }
        }
        BlockPos origin = placement.translation().offset(baseX, baseY, baseZ);
        AABB bounds = sectionBounds(placement, section);
        SectionKey sectionKey = new SectionKey(placement, section);
        BUILT_SECTIONS.put(sectionKey, frame);
        triangles.forEach((pageClass, values) -> {
            MeshKey key = new MeshKey(sectionKey, pageClass.page, pageClass.renderClass);
            Mesh old = MESHES.put(key, upload(bundle, map.atlas(), origin, bounds, values, baseX, baseY, baseZ));
            if (old != null) old.close();
        });
    }

    private static Mesh upload(BundleManifest bundle, AtlasIndex atlas, BlockPos origin, AABB bounds,
                               List<SurfaceTessellator.Triangle> triangles, int baseX, int baseY, int baseZ) {
        int capacity = (int) Math.min(Integer.MAX_VALUE, Math.max(4096L, (long) triangles.size() * 3 * 36));
        try (var bytes = new ByteBufferBuilder(capacity)) {
            var builder = new BufferBuilder(bytes, VertexFormat.Mode.TRIANGLES, DefaultVertexFormat.NEW_ENTITY);
            for (var triangle : triangles) {
                vertex(builder, triangle.a(), baseX, baseY, baseZ, triangle);
                vertex(builder, triangle.b(), baseX, baseY, baseZ, triangle);
                vertex(builder, triangle.c(), baseX, baseY, baseZ, triangle);
            }
            try (var data = builder.buildOrThrow()) {
                var buffer = new VertexBuffer(VertexBuffer.Usage.STATIC);
                buffer.bind(); buffer.upload(data); VertexBuffer.unbind();
                return new Mesh(bundle, atlas, buffer, origin, bounds, frame);
            }
        }
    }

    private static void vertex(BufferBuilder builder, SurfaceTessellator.Vertex vertex, int baseX, int baseY, int baseZ,
                               SurfaceTessellator.Triangle triangle) {
        double abx = triangle.b().x() - triangle.a().x(), aby = triangle.b().y() - triangle.a().y(), abz = triangle.b().z() - triangle.a().z();
        double acx = triangle.c().x() - triangle.a().x(), acy = triangle.c().y() - triangle.a().y(), acz = triangle.c().z() - triangle.a().z();
        float nx = (float) (aby * acz - abz * acy), ny = (float) (abz * acx - abx * acz), nz = (float) (abx * acy - aby * acx);
        float length = (float) Math.sqrt(nx * nx + ny * ny + nz * nz);
        if (length > 0) { nx /= length; ny /= length; nz /= length; }
        builder.addVertex((float) (vertex.x() - baseX), (float) (vertex.y() - baseY), (float) (vertex.z() - baseZ))
            .setColor(255, 255, 255, 255).setUv((float) vertex.u(), (float) vertex.v()).setOverlay(OverlayTexture.NO_OVERLAY)
            .setLight(LightTexture.FULL_BRIGHT).setNormal(nx, ny, nz);
    }

    private static AABB sectionBounds(MapPlacement placement, SurfaceTable.SectionPos section) {
        BlockPos min = placement.translation().offset(section.x() << 4, section.y() << 4, section.z() << 4);
        return new AABB(min.getX(), min.getY(), min.getZ(), min.getX() + 16, min.getY() + 16, min.getZ() + 16).inflate(0.01);
    }

    private static void discardExpiredMeshes() {
        List<SectionKey> expired = BUILT_SECTIONS.entrySet().stream()
            .filter(item -> frame - item.getValue() > MESH_GRACE_FRAMES).map(Map.Entry::getKey).toList();
        for (SectionKey section : expired) {
            BUILT_SECTIONS.remove(section);
            MESHES.entrySet().removeIf(item -> {
                if (!item.getKey().section.equals(section)) return false;
                item.getValue().close();
                return true;
            });
        }
    }

    private static void clear() {
        MESHES.values().forEach(Mesh::close); MESHES.clear(); BUILT_SECTIONS.clear();
        PAGES.reset(-1);
        level = null; generationSequence = -1; placementSnapshot = List.of(); frame = 0;
    }

    private record SectionKey(MapPlacement placement, SurfaceTable.SectionPos local) {}
    private record MeshKey(SectionKey section, int page, BundleMaterial.RenderClass renderClass) {}
    private record PageClass(int page, BundleMaterial.RenderClass renderClass) {}
    private static final class Mesh implements AutoCloseable {
        final BundleManifest bundle; final AtlasIndex atlas; final VertexBuffer buffer; final BlockPos origin; final AABB bounds;
        long lastVisibleFrame;
        Mesh(BundleManifest bundle, AtlasIndex atlas, VertexBuffer buffer, BlockPos origin, AABB bounds, long frame) {
            this.bundle = bundle; this.atlas = atlas; this.buffer = buffer; this.origin = origin; this.bounds = bounds; this.lastVisibleFrame = frame;
        }
        @Override public void close() { buffer.close(); }
    }
}
