package io.github.theredja.src2mc.client;

import com.google.gson.JsonObject;
import io.github.theredja.src2mc.Src2mc;
import java.util.function.Function;
import net.minecraft.client.renderer.block.model.ItemOverrides;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.resources.model.BakedModel;
import net.minecraft.client.resources.model.Material;
import net.minecraft.client.resources.model.ModelBaker;
import net.minecraft.client.resources.model.ModelState;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.neoforge.client.model.geometry.IGeometryBakingContext;
import net.neoforged.neoforge.client.model.geometry.IGeometryLoader;
import net.neoforged.neoforge.client.model.geometry.IUnbakedGeometry;

/**
 * The model loader behind {@code {"loader": "src2mc:surface", "index": n}}.
 *
 * <p>The model files it reads are made up by {@code GeneratedPack}; the only
 * thing in one is the surface index, because everything else is in the bundle
 * and is read after baking, not during it.
 */
public record SurfaceGeometry(int index) implements IUnbakedGeometry<SurfaceGeometry> {
    public static final ResourceLocation ID = ResourceLocation.fromNamespaceAndPath(Src2mc.MOD_ID, "surface");

    public static final IGeometryLoader<SurfaceGeometry> LOADER = (json, context) -> new SurfaceGeometry(index(json));

    private static int index(JsonObject json) {
        if (!json.has("index")) {
            throw new com.google.gson.JsonParseException("a src2mc:surface model needs an index");
        }
        return json.get("index").getAsInt();
    }

    @Override
    public BakedModel bake(
            IGeometryBakingContext context,
            ModelBaker baker,
            Function<Material, TextureAtlasSprite> sprites,
            ModelState state,
            ItemOverrides overrides) {
        return new SurfaceModel(this.index);
    }
}
