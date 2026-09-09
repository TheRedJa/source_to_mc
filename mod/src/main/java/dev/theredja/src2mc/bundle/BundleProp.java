package dev.theredja.src2mc.bundle;

/** Immutable exact Source-derived prop transform; rootCell is storage only. */
public record BundleProp(String stableId, int modelIndex, int[] rootCell,
                         double[] translation, double[] rotation, double scale) {
    public BundleProp {
        rootCell = rootCell.clone();
        translation = translation.clone();
        rotation = rotation.clone();
    }
    @Override public int[] rootCell() { return rootCell.clone(); }
    @Override public double[] translation() { return translation.clone(); }
    @Override public double[] rotation() { return rotation.clone(); }
}
