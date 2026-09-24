package dev.theredja.src2mc.bundle;

import java.util.Arrays;

/** Immutable, validated map index retained by a published generation. */
public record BundleMap(
    String mapId,
    String sourceName,
    int[] cellMin,
    int[] cellMax,
    int[] anchorCell,
    java.util.List<BundleMaterial> materials,
    java.util.List<BundleModel> models,
    java.util.List<BundleProp> props,
    boolean exceedsVanillaBuildHeight,
    SurfaceTable surfaces,
    java.util.Set<String> modelContentIds,
    AtlasIndex atlas,
    PropVisibility pvs,
    // Cells that block light without holding a block; null when the map has none.
    OcclusionTable occlusion
) {
    public BundleMap {
        cellMin = cellMin.clone();
        cellMax = cellMax.clone();
        anchorCell = anchorCell.clone();
        materials = java.util.List.copyOf(materials);
        models = java.util.List.copyOf(models);
        props = java.util.List.copyOf(props);
        modelContentIds = java.util.Set.copyOf(modelContentIds);
    }

    @Override public int[] cellMin() { return cellMin.clone(); }
    @Override public int[] cellMax() { return cellMax.clone(); }
    @Override public int[] anchorCell() { return anchorCell.clone(); }

    @Override
    public boolean equals(Object other) {
        return other instanceof BundleMap map
            && mapId.equals(map.mapId) && sourceName.equals(map.sourceName)
            && Arrays.equals(cellMin, map.cellMin) && Arrays.equals(cellMax, map.cellMax)
            && Arrays.equals(anchorCell, map.anchorCell)
            && materials.equals(map.materials) && models.equals(map.models) && props.equals(map.props)
            && exceedsVanillaBuildHeight == map.exceedsVanillaBuildHeight && surfaces.equals(map.surfaces)
            && modelContentIds.equals(map.modelContentIds) && java.util.Objects.equals(atlas, map.atlas)
            && pvs == map.pvs && occlusion == map.occlusion;
    }

    @Override
    public int hashCode() {
        int result = java.util.Objects.hash(mapId, sourceName, materials, models, props, exceedsVanillaBuildHeight);
        result = 31 * result + Arrays.hashCode(cellMin);
        result = 31 * result + Arrays.hashCode(cellMax);
        result = 31 * result + Arrays.hashCode(anchorCell);
        result = 31 * result + surfaces.hashCode();
        result = 31 * result + modelContentIds.hashCode();
        result = 31 * result + java.util.Objects.hashCode(atlas);
        result = 31 * result + System.identityHashCode(pvs);
        // Identity, like the visibility table: both are large and are never
        // rebuilt within one published generation, and this record is hashed
        // often enough that walking them would cost real frame time.
        return 31 * result + System.identityHashCode(occlusion);
    }

    public int materialCount() { return materials.size(); }
    public int modelCount() { return models.size(); }
}
