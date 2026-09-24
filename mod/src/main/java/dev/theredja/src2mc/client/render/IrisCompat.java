package dev.theredja.src2mc.client.render;

import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;

/**
 * Reflective binding to Iris's public API v0 ({@code net.irisshaders.iris.api.v0.IrisApi}), used
 * to detect the shadow pass. Iris is an optional runtime mod with no maven artifact wired into
 * this build, so there is no compile-time dependency and no mixin; every method fails soft to
 * {@code false} when Iris is absent or the API shape changes.
 */
final class IrisCompat {
    private static final MethodHandle GET_INSTANCE;
    private static final MethodHandle IS_SHADER_PACK_IN_USE;
    private static final MethodHandle IS_RENDERING_SHADOW_PASS;
    /**
     * {@code CapturedRenderingState.INSTANCE} and its entity/block-entity/item setters. Iris stamps
     * these three ids into every vertex of an extended buffer as it is written, taking whatever was
     * rendering at the time, so a static mesh built mid-frame ends up permanently labelled as some
     * entity. Not part of the v0 API either.
     */
    private static final Object CAPTURED_STATE;
    private static final MethodHandle SET_ENTITY;
    private static final MethodHandle SET_BLOCK_ENTITY;
    private static final MethodHandle SET_ITEM;
    private static final MethodHandle GET_ENTITY;
    private static final MethodHandle GET_BLOCK_ENTITY;
    private static final MethodHandle GET_ITEM;

    static {
        MethodHandle getInstance = null, isShaderPackInUse = null, isRenderingShadowPass = null;
        try {
            var lookup = MethodHandles.publicLookup();
            Class<?> api = Class.forName("net.irisshaders.iris.api.v0.IrisApi");
            getInstance = lookup.findStatic(api, "getInstance", MethodType.methodType(api));
            isShaderPackInUse = lookup.findVirtual(api, "isShaderPackInUse", MethodType.methodType(boolean.class));
            isRenderingShadowPass = lookup.findVirtual(api, "isRenderingShadowPass", MethodType.methodType(boolean.class));
        } catch (ReflectiveOperationException | LinkageError ignored) {
            // Iris not installed, or its API shape changed; stay inert.
            getInstance = null; isShaderPackInUse = null; isRenderingShadowPass = null;
        }
        GET_INSTANCE = getInstance; IS_SHADER_PACK_IN_USE = isShaderPackInUse; IS_RENDERING_SHADOW_PASS = isRenderingShadowPass;
        Object captured = null;
        MethodHandle setEntity = null, setBlockEntity = null, setItem = null;
        MethodHandle getEntity = null, getBlockEntity = null, getItem = null;
        try {
            var lookup = MethodHandles.publicLookup();
            Class<?> state = Class.forName("net.irisshaders.iris.uniforms.CapturedRenderingState");
            captured = state.getField("INSTANCE").get(null);
            setEntity = lookup.findVirtual(state, "setCurrentEntity", MethodType.methodType(void.class, int.class));
            setBlockEntity = lookup.findVirtual(state, "setCurrentBlockEntity", MethodType.methodType(void.class, int.class));
            setItem = lookup.findVirtual(state, "setCurrentRenderedItem", MethodType.methodType(void.class, int.class));
            getEntity = lookup.findVirtual(state, "getCurrentRenderedEntity", MethodType.methodType(int.class));
            getBlockEntity = lookup.findVirtual(state, "getCurrentRenderedBlockEntity", MethodType.methodType(int.class));
            getItem = lookup.findVirtual(state, "getCurrentRenderedItem", MethodType.methodType(int.class));
        } catch (ReflectiveOperationException | LinkageError ignored) {
            captured = null; setEntity = null; setBlockEntity = null; setItem = null;
            getEntity = null; getBlockEntity = null; getItem = null;
        }
        CAPTURED_STATE = captured;
        SET_ENTITY = setEntity; SET_BLOCK_ENTITY = setBlockEntity; SET_ITEM = setItem;
        GET_ENTITY = getEntity; GET_BLOCK_ENTITY = getBlockEntity; GET_ITEM = getItem;
    }

    /** Whether the captured ids can be read and written at all. */
    static boolean canSetCapturedIds() {
        return CAPTURED_STATE != null;
    }

    /** The three ids Iris would stamp into a buffer written right now, for the status line. */
    static int[] capturedIds() {
        if (CAPTURED_STATE == null) return new int[] {-1, -1, -1};
        try {
            return new int[] {(int) GET_ENTITY.invoke(CAPTURED_STATE), (int) GET_BLOCK_ENTITY.invoke(CAPTURED_STATE),
                (int) GET_ITEM.invoke(CAPTURED_STATE)};
        } catch (Throwable ignored) {
            return new int[] {-1, -1, -1};
        }
    }

    /**
     * Sets the three captured ids, returning what they were so a caller can put them back.
     * Zeroing them around a static mesh's upload is what keeps the mesh from being labelled as
     * whatever entity happened to be rendering, which decides how a shaderpack treats it -- a pack
     * with entity shadows disabled drops entity-labelled geometry from the shadow map entirely.
     */
    static int[] setCapturedIds(int entity, int blockEntity, int item) {
        if (CAPTURED_STATE == null) return null;
        try {
            int[] previous = capturedIds();
            SET_ENTITY.invoke(CAPTURED_STATE, entity);
            SET_BLOCK_ENTITY.invoke(CAPTURED_STATE, blockEntity);
            SET_ITEM.invoke(CAPTURED_STATE, item);
            return previous;
        } catch (Throwable ignored) {
            return null;
        }
    }

    static void restoreCapturedIds(int[] previous) {
        if (previous == null) return;
        setCapturedIds(previous[0], previous[1], previous[2]);
    }

    /**
     * Iris has a per-thread opt-out from format extension
     * ({@code ImmediateState.skipExtension}), but it is not usable here: while a pack is in use
     * Iris also rewrites {@code NEW_ENTITY.setupBufferState()} to bind the extended layout for
     * every buffer that claims that format, so a buffer built without the extra elements is drawn
     * with the wrong stride and comes out as stretched garbage. The extension has to be accepted;
     * only the data Iris puts in it can be influenced.
     */

    private IrisCompat() {}

    static boolean shaderPackInUse() {
        if (GET_INSTANCE == null) return false;
        try {
            Object instance = GET_INSTANCE.invoke();
            return instance != null && (boolean) IS_SHADER_PACK_IN_USE.invoke(instance);
        } catch (Throwable ignored) {
            return false;
        }
    }

    static boolean renderingShadowPass() {
        if (GET_INSTANCE == null) return false;
        try {
            Object instance = GET_INSTANCE.invoke();
            return instance != null && (boolean) IS_RENDERING_SHADOW_PASS.invoke(instance);
        } catch (Throwable ignored) {
            return false;
        }
    }
}
