package dev.theredja.src2mc.bundle;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import org.junit.jupiter.api.Test;

final class RuntimeMeshTest {
    @Test void decodesTheVersionOneLittleEndianLayout() throws Exception {
        ByteBuffer bytes = ByteBuffer.allocate(48 + 32 + 12 + 12).order(ByteOrder.LITTLE_ENDIAN);
        bytes.put(new byte[]{'S','2','M','E','S','H',0,0}).putInt(1).putInt(1).putInt(3).putInt(1);
        for (float value : new float[]{-1, -2, -3, 4, 5, 6}) bytes.putFloat(value);
        for (float value : new float[]{1, 2, 3, 0, 1, 0, .25f, .75f}) bytes.putFloat(value);
        bytes.putInt(0).putInt(0).putInt(0).putInt(0).putInt(3).putInt(0);
        RuntimeMesh mesh = RuntimeMesh.decode(bytes.array());
        assertEquals(1, mesh.vertexCount());
        assertArrayEquals(new float[]{-1, -2, -3}, mesh.boundsMin());
        assertArrayEquals(new int[]{0, 0, 0}, mesh.indices());
        assertEquals(new RuntimeMesh.Submesh(0, 3, 0), mesh.submeshes()[0]);
    }

    @Test void rejectsTruncatedAndInconsistentPayloads() {
        assertThrows(IOException.class, () -> RuntimeMesh.decode(new byte[0]));
        byte[] invalid = new byte[48];
        System.arraycopy(new byte[]{'S','2','M','E','S','H',0,0}, 0, invalid, 0, 8);
        ByteBuffer.wrap(invalid).order(ByteOrder.LITTLE_ENDIAN).putInt(8, 1).putInt(12, 1);
        assertThrows(IOException.class, () -> RuntimeMesh.decode(invalid));
    }
}
