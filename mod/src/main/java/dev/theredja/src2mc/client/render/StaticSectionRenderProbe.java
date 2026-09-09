package dev.theredja.src2mc.client.render;

import static net.minecraft.commands.Commands.literal;

import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexBuffer;
import com.mojang.blaze3d.vertex.VertexFormat;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.SectionPos;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.util.FastColor;
import net.minecraft.world.phys.AABB;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RegisterClientCommandsEvent;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;
import org.joml.Matrix4f;

import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.Src2mcIds;

/** Disposable Phase 0.5 proof for mod-owned textures and static section VBOs. */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class StaticSectionRenderProbe {
    private static final int PAGES = 2;
    private static final SectionMeshStore<Mesh> MESHES = new SectionMeshStore<>();
    private static final ResourceLocation[] TEXTURE_IDS = {
        Src2mcIds.id("render_probe/page_0"),
        Src2mcIds.id("render_probe/page_1")
    };
    private static final DynamicTexture[] TEXTURES = new DynamicTexture[PAGES];
    private static ClientLevel level;

    private StaticSectionRenderProbe() {
    }

    @SubscribeEvent
    public static void registerCommand(RegisterClientCommandsEvent event) {
        event.getDispatcher().register(literal("src2mc_render_probe").executes(context -> {
            var minecraft = Minecraft.getInstance();
            if (minecraft.player == null || minecraft.level == null) {
                context.getSource().sendFailure(Component.literal("The render probe requires a loaded world"));
                return 0;
            }
            if (MESHES.snapshot().isEmpty()) {
                var bounds = enable(minecraft);
                context.getSource().sendSuccess(
                    () -> Component.literal("src2mc render probe enabled at " + describe(bounds)),
                    false
                );
            } else {
                disable(minecraft);
                context.getSource().sendSuccess(
                    () -> Component.literal("src2mc render probe disabled"),
                    false
                );
            }
            return 1;
        }));
    }

    @SubscribeEvent
    public static void render(RenderLevelStageEvent event) {
        if (event.getStage() != RenderLevelStageEvent.Stage.AFTER_SOLID_BLOCKS) {
            return;
        }
        var minecraft = Minecraft.getInstance();
        if (level != null && minecraft.level != level) {
            disable(minecraft);
            return;
        }
        if (MESHES.snapshot().isEmpty()) {
            return;
        }

        var camera = event.getCamera().getPosition();
        for (var mesh : MESHES.snapshot().values()) {
            if (!event.getFrustum().isVisible(mesh.bounds)) {
                continue;
            }
            var renderType = RenderType.entityCutout(TEXTURE_IDS[mesh.page]);
            renderType.setupRenderState();
            var modelView = new Matrix4f(event.getModelViewMatrix()).translate(
                (float) (mesh.origin.getX() - camera.x),
                (float) (mesh.origin.getY() - camera.y),
                (float) (mesh.origin.getZ() - camera.z)
            );
            mesh.buffer.bind();
            mesh.buffer.drawWithShader(
                modelView,
                event.getProjectionMatrix(),
                GameRenderer.getRendertypeEntityCutoutShader()
            );
            renderType.clearRenderState();
        }
        VertexBuffer.unbind();
    }

    private static AABB enable(Minecraft minecraft) {
        disable(minecraft);
        level = minecraft.level;
        createTextures(minecraft);

        var player = minecraft.player;
        var forward = player.getDirection();
        var alongX = forward.getAxis() == Direction.Axis.Z;
        var position = player.blockPosition();
        var baseY = safeBaseY(position.getY());
        var forwardCoordinate = alongX
            ? position.getZ() + forward.getStepZ() * 6
            : position.getX() + forward.getStepX() * 6;
        var crossCoordinate = alongX ? position.getX() : position.getZ();
        var boundary = nearestSectionBoundary(crossCoordinate);

        var quads = new ArrayList<Quad>();
        for (int column = 0; column < 4; column++) {
            var low = boundary - 2 + column;
            var u0 = column / 4.0F;
            var u1 = (column + 1) / 4.0F;
            for (int page = 0; page < PAGES; page++) {
                var y0 = baseY + page;
                var y1 = y0 + 1;
                quads.add(alongX
                    ? Quad.xy(low, low + 1, y0, y1, forwardCoordinate, u0, u1, page, forward.getOpposite())
                    : Quad.zy(low, low + 1, y0, y1, forwardCoordinate, u0, u1, page, forward.getOpposite()));
            }
        }

        var grouped = new LinkedHashMap<SectionMeshStore.Key, List<Quad>>();
        for (var quad : quads) {
            var section = SectionPos.of(BlockPos.containing(quad.centerX(), quad.centerY(), quad.centerZ()));
            var key = new SectionMeshStore.Key(section.x(), section.y(), section.z(), quad.page);
            grouped.computeIfAbsent(key, ignored -> new ArrayList<>()).add(quad);
        }

        var meshes = new LinkedHashMap<SectionMeshStore.Key, Mesh>();
        grouped.forEach((key, group) -> meshes.put(key, upload(key, group)));
        MESHES.replace(meshes);
        var bounds = quads.stream().map(Quad::bounds).reduce(AABB::minmax).orElseThrow();
        Src2mc.LOGGER.info("Enabled two-page static section render probe at {}", describe(bounds));
        return bounds;
    }

    private static void disable(Minecraft minecraft) {
        MESHES.close();
        for (int page = 0; page < PAGES; page++) {
            if (TEXTURES[page] != null) {
                minecraft.getTextureManager().release(TEXTURE_IDS[page]);
                TEXTURES[page] = null;
            }
        }
        level = null;
    }

    private static void createTextures(Minecraft minecraft) {
        for (int page = 0; page < PAGES; page++) {
            var image = new NativeImage(16, 16, false);
            for (int y = 0; y < 16; y++) {
                for (int x = 0; x < 16; x++) {
                    var alternate = ((x / 4) + (y / 4)) % 2 == 0;
                    var color = page == 0
                        ? rgba(alternate ? 210 : 115, alternate ? 70 : 25, 35)
                        : rgba(35, alternate ? 190 : 80, alternate ? 220 : 105);
                    image.setPixelRGBA(x, y, color);
                }
            }
            var texture = new DynamicTexture(image);
            texture.setFilter(false, false);
            minecraft.getTextureManager().register(TEXTURE_IDS[page], texture);
            texture.upload();
            TEXTURES[page] = texture;
        }
    }

    private static int rgba(int red, int green, int blue) {
        return FastColor.ABGR32.color(255, blue, green, red);
    }

    private static Mesh upload(SectionMeshStore.Key key, List<Quad> quads) {
        var origin = SectionPos.of(key.sectionX(), key.sectionY(), key.sectionZ()).origin();
        try (var bytes = new ByteBufferBuilder(4096)) {
            var builder = new BufferBuilder(bytes, VertexFormat.Mode.QUADS, DefaultVertexFormat.NEW_ENTITY);
            for (var quad : quads) {
                quad.write(builder, origin);
            }
            try (var data = builder.buildOrThrow()) {
                var buffer = new VertexBuffer(VertexBuffer.Usage.STATIC);
                buffer.bind();
                buffer.upload(data);
                VertexBuffer.unbind();
                var bounds = quads.stream().map(Quad::bounds).reduce(AABB::minmax).orElseThrow();
                return new Mesh(buffer, origin, bounds, key.page());
            }
        }
    }

    private static int safeBaseY(int playerY) {
        var sectionMin = SectionPos.sectionToBlockCoord(SectionPos.blockToSectionCoord(playerY));
        return Math.clamp(playerY, sectionMin + 1, sectionMin + 13);
    }

    private static int nearestSectionBoundary(int coordinate) {
        var lower = SectionPos.sectionToBlockCoord(SectionPos.blockToSectionCoord(coordinate));
        return coordinate - lower <= 8 ? lower : lower + 16;
    }

    private static String describe(AABB bounds) {
        return "[%.0f, %.0f, %.0f]..[%.0f, %.0f, %.0f]".formatted(
            bounds.minX, bounds.minY, bounds.minZ, bounds.maxX, bounds.maxY, bounds.maxZ
        );
    }

    private record Mesh(VertexBuffer buffer, BlockPos origin, AABB bounds, int page)
        implements AutoCloseable {
        @Override
        public void close() {
            buffer.close();
        }
    }

    private record Quad(
        float[] a,
        float[] b,
        float[] c,
        float[] d,
        float u0,
        float u1,
        int page,
        Direction normal
    ) {
        static Quad xy(float x0, float x1, float y0, float y1, float z, float u0, float u1, int page, Direction normal) {
            return new Quad(
                new float[] {x0, y0, z}, new float[] {x1, y0, z},
                new float[] {x1, y1, z}, new float[] {x0, y1, z}, u0, u1, page, normal
            );
        }

        static Quad zy(float z0, float z1, float y0, float y1, float x, float u0, float u1, int page, Direction normal) {
            return new Quad(
                new float[] {x, y0, z0}, new float[] {x, y1, z0},
                new float[] {x, y1, z1}, new float[] {x, y0, z1}, u0, u1, page, normal
            );
        }

        void write(BufferBuilder builder, BlockPos origin) {
            vertex(builder, a, origin, u0, 1.0F);
            vertex(builder, b, origin, u1, 1.0F);
            vertex(builder, c, origin, u1, 0.0F);
            vertex(builder, d, origin, u0, 0.0F);
            vertex(builder, d, origin, u0, 0.0F);
            vertex(builder, c, origin, u1, 0.0F);
            vertex(builder, b, origin, u1, 1.0F);
            vertex(builder, a, origin, u0, 1.0F);
        }

        private void vertex(BufferBuilder builder, float[] position, BlockPos origin, float u, float v) {
            builder.addVertex(position[0] - origin.getX(), position[1] - origin.getY(), position[2] - origin.getZ())
                .setColor(255, 255, 255, 255)
                .setUv(u, v)
                .setOverlay(OverlayTexture.NO_OVERLAY)
                .setLight(LightTexture.FULL_BRIGHT)
                .setNormal(normal.getStepX(), normal.getStepY(), normal.getStepZ());
        }

        double centerX() { return (a[0] + c[0]) * 0.5; }
        double centerY() { return (a[1] + c[1]) * 0.5; }
        double centerZ() { return (a[2] + c[2]) * 0.5; }

        AABB bounds() {
            return new AABB(
                Math.min(a[0], c[0]), Math.min(a[1], c[1]), Math.min(a[2], c[2]),
                Math.max(a[0], c[0]), Math.max(a[1], c[1]), Math.max(a[2], c[2])
            ).inflate(0.01);
        }
    }
}
