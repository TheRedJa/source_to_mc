package dev.theredja.src2mc.bundle;

/** Validated map-local material entry addressed by surface and mesh records. */
public record BundleMaterial(String sourceMaterial, RenderClass renderClass, TextureReference texture) {
    public enum RenderClass { SOLID, CUTOUT, TRANSLUCENT, FALLBACK }

    public boolean textured() {
        return texture != null;
    }

    public record TextureReference(String contentId, int originalWidth, int originalHeight,
                                   int outputWidth, int outputHeight) {}
}
