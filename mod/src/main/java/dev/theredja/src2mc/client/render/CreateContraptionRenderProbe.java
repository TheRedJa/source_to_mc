package dev.theredja.src2mc.client.render;

import static net.minecraft.commands.Commands.literal;

import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexBuffer;
import com.mojang.blaze3d.vertex.VertexFormat;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.util.FastColor;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.phys.Vec3;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RegisterClientCommandsEvent;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;
import org.joml.Matrix4f;
import org.joml.Vector4f;

import dev.theredja.src2mc.Src2mc;
import dev.theredja.src2mc.Src2mcIds;

/** Disposable Phase 0.5 proof that mod-owned geometry can follow Create. */
@EventBusSubscriber(modid = Src2mc.MOD_ID, value = Dist.CLIENT)
public final class CreateContraptionRenderProbe {
    private static final String CREATE_ENTITY =
        "com.simibubi.create.content.contraptions.AbstractContraptionEntity";
    private static final ResourceLocation TEXTURE_ID = Src2mcIds.id("render_probe/create_markers");
    private static UUID target;
    private static VertexBuffer buffer;
    private static DynamicTexture texture;
    private static Access access;
    private static List<BlockPos> markerPositions = List.of();
    private static boolean failed;

    private CreateContraptionRenderProbe() {
    }

    @SubscribeEvent
    public static void registerCommand(RegisterClientCommandsEvent event) {
        event.getDispatcher().register(literal("src2mc_create_probe").executes(context -> {
            var minecraft = Minecraft.getInstance();
            if (minecraft.player == null || minecraft.level == null) {
                context.getSource().sendFailure(Component.literal("The Create probe requires a loaded world"));
                return 0;
            }
            if (target != null) {
                disable(minecraft);
                context.getSource().sendSuccess(() -> Component.literal("src2mc Create probe disabled"), false);
                return 1;
            }
            var entity = nearestContraption(minecraft);
            if (entity == null) {
                context.getSource().sendFailure(Component.literal(
                    "No assembled Create contraption found within 32 blocks"));
                return 0;
            }
            try {
                enable(minecraft, entity);
                context.getSource().sendSuccess(() -> Component.literal(
                    "src2mc Create probe attached to " + entity.getName().getString()), false);
                return 1;
            } catch (ReflectiveOperationException | RuntimeException exception) {
                disable(minecraft);
                Src2mc.LOGGER.error("Could not attach Create render probe", exception);
                context.getSource().sendFailure(Component.literal(
                    "Create probe failed; see latest.log for the single recorded error"));
                return 0;
            }
        }));
    }

    @SubscribeEvent
    public static void render(RenderLevelStageEvent event) {
        if (event.getStage() != RenderLevelStageEvent.Stage.AFTER_SOLID_BLOCKS || target == null || failed) {
            return;
        }
        var minecraft = Minecraft.getInstance();
        if (minecraft.level == null) {
            disable(minecraft);
            return;
        }
        Entity entity = null;
        for (var candidate : minecraft.level.entitiesForRendering()) {
            if (candidate.getUUID().equals(target)) {
                entity = candidate;
                break;
            }
        }
        if (entity == null || buffer == null || access == null) {
            try {
                entity = matchingReplacement(minecraft);
                if (entity == null) {
                    return;
                }
                target = entity.getUUID();
                access = Access.of(entity.getClass());
                Src2mc.LOGGER.info("Reattached Create probe to replacement entity {}", target);
            } catch (ReflectiveOperationException | RuntimeException exception) {
                failOnce(minecraft, "Could not reacquire reassembled Create contraption", exception);
                return;
            }
        }
        try {
            float partialTick = event.getPartialTick().getGameTimeDeltaPartialTick(false);
            var origin = access.toGlobal(entity, Vec3.ZERO, partialTick);
            var x = access.toGlobal(entity, new Vec3(1, 0, 0), partialTick).subtract(origin);
            var y = access.toGlobal(entity, new Vec3(0, 1, 0), partialTick).subtract(origin);
            var z = access.toGlobal(entity, new Vec3(0, 0, 1), partialTick).subtract(origin);
            var camera = event.getCamera().getPosition();
            var transform = new Matrix4f();
            transform.setColumn(0, new Vector4f((float) x.x, (float) x.y, (float) x.z, 0));
            transform.setColumn(1, new Vector4f((float) y.x, (float) y.y, (float) y.z, 0));
            transform.setColumn(2, new Vector4f((float) z.x, (float) z.y, (float) z.z, 0));
            transform.setColumn(3, new Vector4f(
                (float) (origin.x - camera.x), (float) (origin.y - camera.y),
                (float) (origin.z - camera.z), 1));
            var modelView = new Matrix4f(event.getModelViewMatrix()).mul(transform);
            var renderType = RenderType.entityCutout(TEXTURE_ID);
            var previousShaderColor = RenderSystem.getShaderColor().clone();
            RenderSystem.setShaderColor(1, 1, 1, 1);
            try {
                renderType.setupRenderState();
                buffer.bind();
                buffer.drawWithShader(modelView, event.getProjectionMatrix(),
                    GameRenderer.getPositionTexColorShader());
            } finally {
                renderType.clearRenderState();
                VertexBuffer.unbind();
                RenderSystem.setShaderColor(previousShaderColor[0], previousShaderColor[1],
                    previousShaderColor[2], previousShaderColor[3]);
            }
        } catch (ReflectiveOperationException | RuntimeException exception) {
            failOnce(minecraft, "Create render probe disabled after one transform failure", exception);
        }
    }

    private static Entity matchingReplacement(Minecraft minecraft)
        throws ReflectiveOperationException {
        for (var entity : minecraft.level.entitiesForRendering()) {
            if (!inherits(entity.getClass(), CREATE_ENTITY)) {
                continue;
            }
            var candidateAccess = Access.of(entity.getClass());
            if (candidateAccess.blockPositions(entity).containsAll(markerPositions)) {
                return entity;
            }
        }
        return null;
    }

    private static void failOnce(Minecraft minecraft, String message, Exception exception) {
        failed = true;
        Src2mc.LOGGER.error(message, exception);
        if (minecraft.player != null) {
            minecraft.player.displayClientMessage(Component.literal(
                "src2mc Create probe failed; it has been stopped to prevent log flooding"), false);
        }
    }

    private static Entity nearestContraption(Minecraft minecraft) {
        Entity nearest = null;
        double nearestDistance = 32 * 32;
        for (var entity : minecraft.level.entitiesForRendering()) {
            if (!inherits(entity.getClass(), CREATE_ENTITY)) {
                continue;
            }
            double distance = entity.distanceToSqr(minecraft.player);
            if (distance < nearestDistance) {
                nearest = entity;
                nearestDistance = distance;
            }
        }
        return nearest;
    }

    private static boolean inherits(Class<?> type, String name) {
        for (var current = type; current != null; current = current.getSuperclass()) {
            if (current.getName().equals(name)) {
                return true;
            }
        }
        return false;
    }

    private static void enable(Minecraft minecraft, Entity entity)
        throws ReflectiveOperationException {
        disable(minecraft);
        access = Access.of(entity.getClass());
        var positions = access.blockPositions(entity).stream()
            .sorted(Comparator.comparingLong(BlockPos::asLong))
            .limit(3)
            .toList();
        if (positions.isEmpty()) {
            throw new IllegalStateException("Create contraption contains no blocks");
        }
        markerPositions = positions;
        createTexture(minecraft);
        buffer = upload(positions);
        target = entity.getUUID();
        failed = false;
        Src2mc.LOGGER.info("Attached Create probe to {} at local blocks {}", target, positions);
    }

    private static void disable(Minecraft minecraft) {
        target = null;
        access = null;
        markerPositions = List.of();
        failed = false;
        if (buffer != null) {
            buffer.close();
            buffer = null;
        }
        if (texture != null) {
            minecraft.getTextureManager().release(TEXTURE_ID);
            texture = null;
        }
    }

    private static void createTexture(Minecraft minecraft) {
        var image = new NativeImage(4, 1, false);
        image.setPixelRGBA(0, 0, rgba(255, 190, 20));
        image.setPixelRGBA(1, 0, rgba(20, 220, 240));
        image.setPixelRGBA(2, 0, rgba(230, 40, 210));
        image.setPixelRGBA(3, 0, rgba(255, 255, 255));
        texture = new DynamicTexture(image);
        texture.setFilter(false, false);
        minecraft.getTextureManager().register(TEXTURE_ID, texture);
        texture.upload();
    }

    private static int rgba(int red, int green, int blue) {
        return FastColor.ABGR32.color(255, blue, green, red);
    }

    private static VertexBuffer upload(List<BlockPos> positions) {
        try (var bytes = new ByteBufferBuilder(4096)) {
            var builder = new BufferBuilder(
                bytes, VertexFormat.Mode.QUADS, DefaultVertexFormat.POSITION_TEX_COLOR);
            for (int index = 0; index < positions.size(); index++) {
                addMarker(builder, positions.get(index), index);
            }
            try (var data = builder.buildOrThrow()) {
                var result = new VertexBuffer(VertexBuffer.Usage.STATIC);
                result.bind();
                result.upload(data);
                VertexBuffer.unbind();
                return result;
            }
        }
    }

    private static void addMarker(BufferBuilder builder, BlockPos block, int color) {
        float x0 = block.getX() + 0.2F;
        float x1 = block.getX() + 0.8F;
        float middle = (x0 + x1) * 0.5F;
        float y = block.getY() + 1.05F;
        float z0 = block.getZ() + 0.2F;
        float z1 = block.getZ() + 0.8F;
        int[][] colors = {{255, 190, 20}, {20, 220, 240}, {230, 40, 210}};
        int[] rgb = colors[color];
        // One half gets its colour from a dedicated texture texel. The other
        // samples white and gets the same intended colour from vertex data.
        // Their comparison isolates texture upload from shader colour handling.
        float textureU = (color + 0.5F) / 4.0F;
        quad(builder, x0, middle, y, z0, z1, textureU, 255, 255, 255);
        quad(builder, middle, x1, y, z0, z1, 3.5F / 4.0F, rgb[0], rgb[1], rgb[2]);
    }

    private static void quad(BufferBuilder builder, float x0, float x1, float y,
        float z0, float z1, float u, int red, int green, int blue) {
        vertex(builder, x0, y, z0, u, red, green, blue);
        vertex(builder, x1, y, z0, u, red, green, blue);
        vertex(builder, x1, y, z1, u, red, green, blue);
        vertex(builder, x0, y, z1, u, red, green, blue);
        vertex(builder, x0, y, z1, u, red, green, blue);
        vertex(builder, x1, y, z1, u, red, green, blue);
        vertex(builder, x1, y, z0, u, red, green, blue);
        vertex(builder, x0, y, z0, u, red, green, blue);
    }

    private static void vertex(BufferBuilder builder, float x, float y, float z,
        float u, int red, int green, int blue) {
        builder.addVertex(x, y, z).setUv(u, 0.5F).setColor(red, green, blue, 255);
    }

    private record Access(Method toGlobal, Method getContraption, Method getBlocks) {
        static Access of(Class<?> entityType) throws ReflectiveOperationException {
            var toGlobal = entityType.getMethod("toGlobalVector", Vec3.class, float.class, boolean.class);
            var getContraption = entityType.getMethod("getContraption");
            var getBlocks = getContraption.getReturnType().getMethod("getBlocks");
            return new Access(toGlobal, getContraption, getBlocks);
        }

        Vec3 toGlobal(Entity entity, Vec3 local, float partialTick) throws ReflectiveOperationException {
            return (Vec3) toGlobal.invoke(entity, local, partialTick, true);
        }

        List<BlockPos> blockPositions(Entity entity) throws ReflectiveOperationException {
            var contraption = getContraption.invoke(entity);
            var blocks = (Map<?, ?>) getBlocks.invoke(contraption);
            var positions = new ArrayList<BlockPos>();
            for (var key : blocks.keySet()) {
                if (key instanceof BlockPos position) {
                    positions.add(position);
                }
            }
            return positions;
        }
    }
}
