package dev.theredja.src2mc.bundle;

/** Immutable map-local binding of a shared runtime mesh to material slots. */
public record BundleModel(String contentId, String sourceModel, int[] materialIds) {
    public BundleModel { materialIds = materialIds.clone(); }
    @Override public int[] materialIds() { return materialIds.clone(); }
    public int materialSlotCount() { return materialIds.length; }
}
