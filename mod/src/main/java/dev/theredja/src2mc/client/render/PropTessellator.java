package dev.theredja.src2mc.client.render;

import dev.theredja.src2mc.bundle.AtlasIndex;
import dev.theredja.src2mc.bundle.BundleMaterial;
import dev.theredja.src2mc.bundle.BundleProp;
import dev.theredja.src2mc.bundle.RuntimeMesh;
import java.util.ArrayList;
import java.util.List;

/** CPU-side, lossless clipping of authored prop triangles into atlas pages and 16-cube sections. */
final class PropTessellator {
    private static final double EPSILON = 1.0e-8;
    private static final int MAX_POLYGONS_PER_TRIANGLE = 4096;

    private PropTessellator() {}

    static List<Triangle> tessellate(RuntimeMesh mesh, RuntimeMesh.Submesh submesh, BundleProp prop,
                                    BundleMaterial.TextureReference material, AtlasIndex.Texture texture,
                                    int pageSize) {
        float[] vertices = mesh.vertices();
        int[] indices = mesh.indices();
        List<Triangle> result = new ArrayList<>();
        int end = submesh.firstIndex() + submesh.indexCount();
        for (int index = submesh.firstIndex(); index < end; index += 3) {
            List<Vertex> triangle = new ArrayList<>(3);
            for (int corner = 0; corner < 3; corner++) {
                int vertex = indices[index + corner] * 8;
                triangle.add(transform(vertices, vertex, prop, material));
            }
            double minU = min(triangle, 3), maxU = max(triangle, 3);
            double minV = min(triangle, 4), maxV = max(triangle, 4);
            int repeatUMin = floorDiv(minU, texture.width()), repeatUMax = floorDiv(maxU - EPSILON, texture.width());
            int repeatVMin = floorDiv(minV, texture.height()), repeatVMax = floorDiv(maxV - EPSILON, texture.height());
            long candidates = (long) (repeatUMax - repeatUMin + 1) * (repeatVMax - repeatVMin + 1) * texture.regions().size();
            if (candidates > MAX_POLYGONS_PER_TRIANGLE) continue;
            for (int repeatV = repeatVMin; repeatV <= repeatVMax; repeatV++) {
                for (int repeatU = repeatUMin; repeatU <= repeatUMax; repeatU++) {
                    for (AtlasIndex.Region region : texture.regions()) {
                        int[] source = region.source(), allocation = region.allocation();
                        double left = (double) repeatU * texture.width() + source[0];
                        double top = (double) repeatV * texture.height() + source[1];
                        List<Vertex> clipped = clipRect(triangle, left, top, left + source[2], top + source[3]);
                        if (clipped.size() < 3) continue;
                        for (int i = 1; i + 1 < clipped.size(); i++) {
                            Vertex a = atlas(clipped.getFirst(), left, top, allocation, pageSize);
                            Vertex b = atlas(clipped.get(i), left, top, allocation, pageSize);
                            Vertex c = atlas(clipped.get(i + 1), left, top, allocation, pageSize);
                            result.add(new Triangle(region.page(), a, b, c));
                        }
                    }
                }
            }
        }
        return result;
    }

    static List<Triangle> clipSection(Triangle triangle, int x, int y, int z) {
        List<Vertex> polygon = List.of(triangle.a(), triangle.b(), triangle.c());
        polygon = clip(polygon, 0, x, true); polygon = clip(polygon, 0, x + 16, false);
        polygon = clip(polygon, 1, y, true); polygon = clip(polygon, 1, y + 16, false);
        polygon = clip(polygon, 2, z, true); polygon = clip(polygon, 2, z + 16, false);
        if (polygon.size() < 3) return List.of();
        List<Triangle> result = new ArrayList<>(polygon.size() - 2);
        for (int i = 1; i + 1 < polygon.size(); i++) result.add(new Triangle(triangle.page(), polygon.getFirst(), polygon.get(i), polygon.get(i + 1)));
        return result;
    }

    private static Vertex transform(float[] values, int offset, BundleProp prop, BundleMaterial.TextureReference material) {
        double scale = prop.scale(), px = values[offset] * scale, py = values[offset + 1] * scale, pz = values[offset + 2] * scale;
        double[] q = prop.rotation();
        double[] position = rotate(q, px, py, pz);
        double[] normal = rotate(q, values[offset + 3], values[offset + 4], values[offset + 5]);
        double[] translation = prop.translation();
        return new Vertex(position[0] + translation[0], position[1] + translation[1], position[2] + translation[2],
            normal[0], normal[1], normal[2], values[offset + 6] * material.outputWidth(), values[offset + 7] * material.outputHeight());
    }

    /** q * vector * inverse(q), with bundle quaternions stored as XYZW. */
    private static double[] rotate(double[] q, double x, double y, double z) {
        double qx = q[0], qy = q[1], qz = q[2], qw = q[3];
        double tx = 2.0 * (qy * z - qz * y), ty = 2.0 * (qz * x - qx * z), tz = 2.0 * (qx * y - qy * x);
        return new double[]{x + qw * tx + qy * tz - qz * ty, y + qw * ty + qz * tx - qx * tz, z + qw * tz + qx * ty - qy * tx};
    }

    private static Vertex atlas(Vertex vertex, double left, double top, int[] allocation, int pageSize) {
        return vertex.withUv((allocation[0] + vertex.u() - left) / pageSize, (allocation[1] + vertex.v() - top) / pageSize);
    }

    private static List<Vertex> clipRect(List<Vertex> input, double minU, double minV, double maxU, double maxV) {
        List<Vertex> out = clip(input, 3, minU, true); out = clip(out, 3, maxU, false);
        out = clip(out, 4, minV, true); return clip(out, 4, maxV, false);
    }

    private static List<Vertex> clip(List<Vertex> input, int axis, double boundary, boolean greater) {
        if (input.isEmpty()) return input;
        List<Vertex> out = new ArrayList<>(); Vertex previous = input.getLast(); boolean previousInside = inside(previous, axis, boundary, greater);
        for (Vertex current : input) {
            boolean currentInside = inside(current, axis, boundary, greater);
            if (currentInside != previousInside) {
                double before = value(previous, axis), after = value(current, axis);
                out.add(previous.interpolate(current, (boundary - before) / (after - before)));
            }
            if (currentInside) out.add(current);
            previous = current; previousInside = currentInside;
        }
        return out;
    }

    private static boolean inside(Vertex vertex, int axis, double boundary, boolean greater) {
        double value = value(vertex, axis); return greater ? value >= boundary - EPSILON : value <= boundary + EPSILON;
    }
    private static double value(Vertex v, int axis) { return switch (axis) { case 0 -> v.x(); case 1 -> v.y(); case 2 -> v.z(); case 3 -> v.u(); case 4 -> v.v(); default -> throw new IllegalArgumentException("axis"); }; }
    private static double min(List<Vertex> vertices, int axis) { double result = Double.POSITIVE_INFINITY; for (Vertex vertex : vertices) result = Math.min(result, value(vertex, axis)); return result; }
    private static double max(List<Vertex> vertices, int axis) { double result = Double.NEGATIVE_INFINITY; for (Vertex vertex : vertices) result = Math.max(result, value(vertex, axis)); return result; }
    private static int floorDiv(double value, int divisor) { return (int) Math.floor(value / divisor); }

    record Triangle(int page, Vertex a, Vertex b, Vertex c) {}
    record Vertex(double x, double y, double z, double nx, double ny, double nz, double u, double v) {
        Vertex withUv(double nextU, double nextV) { return new Vertex(x, y, z, nx, ny, nz, nextU, nextV); }
        Vertex interpolate(Vertex other, double t) { return new Vertex(x + (other.x - x) * t, y + (other.y - y) * t, z + (other.z - z) * t, nx + (other.nx - nx) * t, ny + (other.ny - ny) * t, nz + (other.nz - nz) * t, u + (other.u - u) * t, v + (other.v - v) * t); }
    }
}
