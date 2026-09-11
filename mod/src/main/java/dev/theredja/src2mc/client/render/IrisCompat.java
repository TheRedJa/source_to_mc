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
    }

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
