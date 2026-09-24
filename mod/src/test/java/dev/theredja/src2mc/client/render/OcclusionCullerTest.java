package dev.theredja.src2mc.client.render;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;

class OcclusionCullerTest {
    @Test
    void cameraViewOnlyReusesACompletedQueryInsideItsConservativeWindow() {
        var queried = new OcclusionCuller.CameraView(new Vec3(10, 70, -3), 12.0f, -40.0f);
        assertTrue(queried.compatibleWith(new OcclusionCuller.CameraView(new Vec3(10.24, 70, -3), 13.4f, -38.6f)));
        assertFalse(queried.compatibleWith(new OcclusionCuller.CameraView(new Vec3(10.26, 70, -3), 12.0f, -40.0f)));
        assertFalse(queried.compatibleWith(new OcclusionCuller.CameraView(new Vec3(10, 70, -3), 13.6f, -40.0f)));
    }

    @Test
    void cameraViewHandlesRotationAcrossThePlusMinus180Boundary() {
        var queried = new OcclusionCuller.CameraView(Vec3.ZERO, 0.0f, 179.9f);
        assertTrue(queried.compatibleWith(new OcclusionCuller.CameraView(Vec3.ZERO, 0.0f, -179.95f)));
    }
}
