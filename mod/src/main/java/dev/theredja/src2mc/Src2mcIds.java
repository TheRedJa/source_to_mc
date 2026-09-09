package dev.theredja.src2mc;

import net.minecraft.resources.ResourceLocation;

/** Creates resource identifiers owned by src2mc. */
public final class Src2mcIds {
    private Src2mcIds() {
    }

    public static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, path);
    }
}
