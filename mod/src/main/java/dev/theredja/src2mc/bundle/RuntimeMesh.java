package dev.theredja.src2mc.bundle;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;

/** Decoded v1 runtime mesh. Only the client demand-loads these immutable arrays. */
public record RuntimeMesh(float[] boundsMin, float[] boundsMax, float[] vertices, int[] indices, Submesh[] submeshes) {
    private static final byte[] MAGIC = {'S','2','M','E','S','H',0,0};
    private static final int HEADER_BYTES = 48;
    public RuntimeMesh {
        boundsMin = boundsMin.clone(); boundsMax = boundsMax.clone(); vertices = vertices.clone();
        indices = indices.clone(); submeshes = submeshes.clone();
    }
    @Override public float[] boundsMin() { return boundsMin.clone(); }
    @Override public float[] boundsMax() { return boundsMax.clone(); }
    @Override public float[] vertices() { return vertices.clone(); }
    @Override public int[] indices() { return indices.clone(); }
    @Override public Submesh[] submeshes() { return submeshes.clone(); }
    public int vertexCount() { return vertices.length / 8; }

    public static RuntimeMesh decode(byte[] bytes) throws IOException {
        if (bytes.length < HEADER_BYTES) throw new IOException("truncated runtime mesh");
        ByteBuffer in = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN);
        byte[] magic = new byte[8]; in.get(magic);
        if (!Arrays.equals(magic, MAGIC) || in.getInt() != 1) throw new IOException("unsupported runtime mesh format");
        long vertexCount = Integer.toUnsignedLong(in.getInt()), indexCount = Integer.toUnsignedLong(in.getInt()), submeshCount = Integer.toUnsignedLong(in.getInt());
        long expected;
        try { expected = Math.addExact(HEADER_BYTES, Math.addExact(Math.multiplyExact(vertexCount, 32), Math.addExact(Math.multiplyExact(indexCount, 4), Math.multiplyExact(submeshCount, 12)))); }
        catch (ArithmeticException exception) { throw new IOException("runtime mesh length overflow", exception); }
        if (expected != bytes.length || vertexCount > Integer.MAX_VALUE || indexCount > Integer.MAX_VALUE || submeshCount > Integer.MAX_VALUE) throw new IOException("runtime mesh length differs from header");
        float[] min = {in.getFloat(), in.getFloat(), in.getFloat()};
        float[] max = {in.getFloat(), in.getFloat(), in.getFloat()};
        float[] vertices = new float[Math.toIntExact(vertexCount * 8)];
        for (int i = 0; i < vertices.length; i++) vertices[i] = in.getFloat();
        int[] indices = new int[(int) indexCount]; for (int i = 0; i < indices.length; i++) indices[i] = in.getInt();
        Submesh[] submeshes = new Submesh[(int) submeshCount];
        for (int i = 0; i < submeshes.length; i++) submeshes[i] = new Submesh(in.getInt(), in.getInt(), in.getInt());
        return new RuntimeMesh(min, max, vertices, indices, submeshes);
    }

    public record Submesh(int firstIndex, int indexCount, int materialSlot) {}
}
