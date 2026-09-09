package dev.theredja.src2mc.client.render;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexBuffer;
import com.mojang.blaze3d.vertex.VertexFormat;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.client.GlStateBackup;
import org.joml.Matrix4f;
import org.lwjgl.opengl.GL;
import org.lwjgl.opengl.GL11C;
import org.lwjgl.opengl.GL15C;
import org.lwjgl.opengl.GL33C;

/**
 * Render-thread-owned, fail-open GPU occlusion queries. Results are never read
 * synchronously: a query only suppresses a draw when its completed result is
 * still compatible with the current camera view.
 */
final class OcclusionCuller<K> implements AutoCloseable {
    // These deliberately allow a little camera movement before reusing an
    // answer. The proxy's 0.5-block expansion below keeps that reuse safely
    // biased toward drawing rather than exposing a hidden prop suddenly.
    private static final int MAX_QUERIES_PER_FRAME = 24;
    private static final int QUERY_REFRESH_FRAMES = 16;
    private static final double MAX_CAMERA_DELTA = 0.25;
    private static final double MAX_VIEW_ANGLE_DEGREES = 1.5;
    private static final double BOUNDS_PADDING = 0.5;

    private final Map<K, State> states = new HashMap<>();
    private final Measurements measurements = new Measurements();
    private VertexBuffer proxy;
    private long frame;
    private boolean supported;
    private boolean capabilityChecked;
    private boolean enabled = true;
    private int queriesRemaining;

    void beginFrame(long frame) {
        this.frame = frame;
        measurements.beginFrame();
        queriesRemaining = MAX_QUERIES_PER_FRAME;
        ensureCapability();
        if (!supported) return;
        for (State state : states.values()) poll(state);
    }

    boolean shouldCull(K key, AABB bounds, CameraView view, long triangles) {
        if (!enabled || !supported) return false;
        State state = states.computeIfAbsent(key, ignored -> new State());
        if (!state.occluded) return false;
        if (!state.view.compatibleWith(view)) {
            state.occluded = false;
            measurements.current.invalidated++;
            return false;
        }
        measurements.current.rejectedDraws++;
        measurements.current.rejectedTriangles += triangles;
        return true;
    }

    /** Issues at most a bounded number of queries, prioritizing expensive batches. */
    void issue(List<Candidate<K>> input, CameraView view, Matrix4f modelView, Matrix4f projection) {
        if (!enabled || !supported || input.isEmpty()) return;
        measurements.current.candidates += input.size();
        List<Candidate<K>> candidates = new ArrayList<>(input);
        candidates.sort(Comparator.comparingLong(Candidate<K>::triangles).reversed());
        for (Candidate<K> candidate : candidates) {
            State state = states.computeIfAbsent(candidate.key(), ignored -> new State());
            if (state.pending || (state.lastIssuedFrame >= 0 && frame - state.lastIssuedFrame < QUERY_REFRESH_FRAMES && state.view.compatibleWith(view))) continue;
            if (queriesRemaining <= 0) { measurements.current.budgetSkipped++; continue; }
            queriesRemaining--;
            issue(state, candidate.bounds().inflate(BOUNDS_PADDING), view, modelView, projection);
        }
    }

    boolean toggleEnabled() {
        enabled = !enabled;
        if (!enabled) for (State state : states.values()) state.occluded = false;
        return enabled;
    }

    void invalidate(K key) {
        State state = states.remove(key);
        if (state != null) delete(state);
    }

    Stats stats() {
        long pendingCount = states.values().stream().filter(state -> state.pending).count();
        return new Stats(enabled, supported, pendingCount, queriesRemaining, measurements.last(), measurements.average(), measurements.totals());
    }

    @Override public void close() {
        for (State state : states.values()) delete(state);
        states.clear();
        if (proxy != null) { proxy.close(); proxy = null; }
        capabilityChecked = false;
        supported = false;
        enabled = true;
        queriesRemaining = 0;
        measurements.reset();
    }

    private void ensureCapability() {
        if (capabilityChecked) return;
        capabilityChecked = true;
        supported = GL.getCapabilities().OpenGL33 || GL.getCapabilities().GL_ARB_occlusion_query2;
    }

    private void poll(State state) {
        if (!state.pending || GL15C.glGetQueryObjecti(state.query, GL15C.GL_QUERY_RESULT_AVAILABLE) == 0) return;
        boolean samplesPassed = GL15C.glGetQueryObjecti(state.query, GL15C.GL_QUERY_RESULT) != 0;
        GL15C.glDeleteQueries(state.query);
        state.query = 0;
        state.pending = false;
        state.occluded = !samplesPassed;
        if (samplesPassed) measurements.current.visible++;
        else measurements.current.occluded++;
    }

    private void issue(State state, AABB bounds, CameraView view, Matrix4f modelView, Matrix4f projection) {
        ensureProxy();
        GlStateBackup backup = new GlStateBackup();
        RenderSystem.backupGlState(backup);
        boolean queryBegan = false;
        try {
            RenderSystem.enableDepthTest();
            RenderSystem.depthFunc(GL11C.GL_LEQUAL);
            RenderSystem.depthMask(false);
            RenderSystem.disableCull();
            RenderSystem.colorMask(false, false, false, false);
            state.query = GL33C.glGenQueries();
            GL33C.glBeginQuery(GL33C.GL_ANY_SAMPLES_PASSED, state.query);
            queryBegan = true;
            Matrix4f transform = new Matrix4f(modelView)
                .translate((float) (bounds.minX - view.position().x), (float) (bounds.minY - view.position().y), (float) (bounds.minZ - view.position().z))
                .scale((float) bounds.getXsize(), (float) bounds.getYsize(), (float) bounds.getZsize());
            proxy.bind();
            proxy.drawWithShader(transform, projection, GameRenderer.getPositionShader());
            GL33C.glEndQuery(GL33C.GL_ANY_SAMPLES_PASSED);
            queryBegan = false;
            VertexBuffer.unbind();
            state.pending = true;
            state.occluded = false;
            state.view = view;
            state.lastIssuedFrame = frame;
            measurements.current.issued++;
        } catch (RuntimeException exception) {
            if (queryBegan) {
                try { GL33C.glEndQuery(GL33C.GL_ANY_SAMPLES_PASSED); }
                catch (RuntimeException ignored) { }
            }
            if (state.query != 0) GL15C.glDeleteQueries(state.query);
            state.query = 0;
            state.pending = false;
            state.occluded = false;
            supported = false;
        } finally {
            RenderSystem.restoreGlState(backup);
        }
    }

    private void ensureProxy() {
        if (proxy != null) return;
        try (var bytes = new ByteBufferBuilder(1024)) {
            var builder = new BufferBuilder(bytes, VertexFormat.Mode.TRIANGLES, DefaultVertexFormat.POSITION);
            cube(builder);
            try (var data = builder.buildOrThrow()) {
                proxy = new VertexBuffer(VertexBuffer.Usage.STATIC);
                proxy.bind();
                proxy.upload(data);
                VertexBuffer.unbind();
            }
        }
    }

    private static void cube(BufferBuilder builder) {
        face(builder, 0, 0, 0, 1, 0, 0, 1, 0, 1, 0, 0, 1);
        face(builder, 0, 1, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0);
        face(builder, 0, 0, 0, 0, 0, 1, 0, 1, 1, 0, 1, 0);
        face(builder, 1, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1);
        face(builder, 0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0);
        face(builder, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 0, 1);
    }

    private static void face(BufferBuilder b, float ax, float ay, float az, float bx, float by, float bz, float cx, float cy, float cz, float dx, float dy, float dz) {
        b.addVertex(ax, ay, az); b.addVertex(bx, by, bz); b.addVertex(cx, cy, cz);
        b.addVertex(ax, ay, az); b.addVertex(cx, cy, cz); b.addVertex(dx, dy, dz);
    }

    private void delete(State state) {
        if (state.query != 0) GL15C.glDeleteQueries(state.query);
        state.query = 0;
        state.pending = false;
    }

    record Candidate<K>(K key, AABB bounds, long triangles) {}
    record CameraView(Vec3 position, float xRot, float yRot) {
        boolean compatibleWith(CameraView other) {
            return position.distanceToSqr(other.position) <= MAX_CAMERA_DELTA * MAX_CAMERA_DELTA
                && angleDistance(xRot, other.xRot) <= MAX_VIEW_ANGLE_DEGREES
                && angleDistance(yRot, other.yRot) <= MAX_VIEW_ANGLE_DEGREES;
        }
        private static float angleDistance(float left, float right) {
            float delta = Math.abs(left - right) % 360.0f;
            return delta > 180.0f ? 360.0f - delta : delta;
        }
    }
    record Stats(boolean enabled, boolean supported, long pending, int queriesRemaining, Counters last, Counters average, Counters totals) {}
    record Counters(long candidates, long issued, long visible, long occluded, long invalidated,
                    long budgetSkipped, long rejectedDraws, long rejectedTriangles) {}

    private static final class Measurements {
        private static final int WINDOW_FRAMES = 120;
        private final CountersMutable current = new CountersMutable();
        private final List<Counters> window = new ArrayList<>();
        private final CountersMutable totals = new CountersMutable();
        private Counters last = new Counters(0, 0, 0, 0, 0, 0, 0, 0);
        private boolean started;

        void beginFrame() {
            if (started) {
                last = current.snapshot();
                totals.add(last);
                window.add(last);
                if (window.size() > WINDOW_FRAMES) window.removeFirst();
            }
            current.clear();
            started = true;
        }

        Counters last() { return last; }
        Counters totals() { return totals.snapshot(); }
        Counters average() {
            if (window.isEmpty()) return new Counters(0, 0, 0, 0, 0, 0, 0, 0);
            CountersMutable sum = new CountersMutable();
            for (Counters counters : window) sum.add(counters);
            return sum.divide(window.size());
        }
        void reset() { current.clear(); window.clear(); totals.clear(); last = new Counters(0, 0, 0, 0, 0, 0, 0, 0); started = false; }
    }

    private static final class CountersMutable {
        long candidates, issued, visible, occluded, invalidated, budgetSkipped, rejectedDraws, rejectedTriangles;
        void add(Counters counters) { candidates += counters.candidates(); issued += counters.issued(); visible += counters.visible(); occluded += counters.occluded(); invalidated += counters.invalidated(); budgetSkipped += counters.budgetSkipped(); rejectedDraws += counters.rejectedDraws(); rejectedTriangles += counters.rejectedTriangles(); }
        Counters snapshot() { return new Counters(candidates, issued, visible, occluded, invalidated, budgetSkipped, rejectedDraws, rejectedTriangles); }
        Counters divide(int divisor) { return new Counters(candidates / divisor, issued / divisor, visible / divisor, occluded / divisor, invalidated / divisor, budgetSkipped / divisor, rejectedDraws / divisor, rejectedTriangles / divisor); }
        void clear() { candidates = issued = visible = occluded = invalidated = budgetSkipped = rejectedDraws = rejectedTriangles = 0; }
    }
    private final class State {
        int query;
        boolean pending;
        boolean occluded;
        CameraView view = new CameraView(Vec3.ZERO, Float.NaN, Float.NaN);
        long lastIssuedFrame = -1;
    }
}
