package io.github.theredja.src2mc.client;

import io.github.theredja.src2mc.bundle.MaterialTable;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.block.model.ItemOverrides;
import net.minecraft.client.renderer.block.model.ItemTransforms;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.util.RandomSource;
import net.minecraft.world.inventory.InventoryMenu;
import net.minecraft.world.level.BlockAndTintGetter;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.client.ChunkRenderTypeSet;
import net.neoforged.neoforge.client.model.IDynamicBakedModel;
import net.neoforged.neoforge.client.model.data.ModelData;
import net.neoforged.neoforge.client.model.data.ModelProperty;
import org.jetbrains.annotations.NotNull;

/**
 * The model behind one surface block. This is what the mod exists for.
 *
 * <p>The block itself says nothing but its index. Everything visible — which
 * texture, how far it stretches, whether it is cut out — is the material table
 * entry with that index, read here at chunk bake time. Adding a map or changing
 * a texture therefore changes what this draws without changing anything that is
 * registered (D6).
 *
 * <p>The block's world position arrives through {@link ModelData}: NeoForge
 * calls {@link #getModelData} for every block it bakes, and Sodium's
 * {@code NeoForgeModelAccess} calls the same method, so the position is
 * available on both renderers without an entity, a block entity or a
 * {@code BlockEntityRenderer} (D1, D7, D9). The geometry lands in the chunk mesh
 * and costs nothing per frame.
 */
public final class SurfaceModel implements IDynamicBakedModel {
    /** Where a block's own position reaches the model. Texture coordinates need it (D10). */
    public static final ModelProperty<BlockPos> POSITION = new ModelProperty<>();

    private static final int VERTEX_STRIDE = 8;
    private static final int WHITE = 0xFFFFFFFF;

    private final int index;

    /** Resolved once per bake; the atlas is rebuilt on every reload anyway. */
    private volatile TextureAtlasSprite sprite;

    private volatile MaterialTable resolvedAgainst;

    public SurfaceModel(int index) {
        this.index = index;
    }

    private Optional<MaterialTable.Entry> entry() {
        return MaterialTable.client().get(this.index);
    }

    private TextureAtlasSprite sprite() {
        MaterialTable table = MaterialTable.client();
        TextureAtlasSprite cached = this.sprite;
        if (cached != null && this.resolvedAgainst == table) {
            return cached;
        }
        // getSprite answers with the missing-texture sprite rather than null, so
        // a bundle that names a texture it does not ship shows as missing rather
        // than failing the chunk bake.
        TextureAtlasSprite resolved = Minecraft.getInstance()
                .getModelManager()
                .getAtlas(InventoryMenu.BLOCK_ATLAS)
                .getSprite(table.get(this.index)
                        .map(MaterialTable.Entry::sprite)
                        .orElse(net.minecraft.client.renderer.texture.MissingTextureAtlasSprite.getLocation()));
        this.resolvedAgainst = table;
        this.sprite = resolved;
        return resolved;
    }

    @Override
    public @NotNull ModelData getModelData(
            @NotNull BlockAndTintGetter level,
            @NotNull BlockPos pos,
            @NotNull BlockState state,
            @NotNull ModelData data) {
        return data.derive().with(POSITION, pos.immutable()).build();
    }

    @Override
    public ChunkRenderTypeSet getRenderTypes(@NotNull BlockState state, @NotNull RandomSource rand, @NotNull ModelData data) {
        return entry().map(entry -> switch (entry.renderType()) {
                    case SOLID -> ChunkRenderTypeSet.of(RenderType.solid());
                    case CUTOUT -> ChunkRenderTypeSet.of(RenderType.cutoutMipped());
                    case TRANSLUCENT -> ChunkRenderTypeSet.of(RenderType.translucent());
                })
                .orElseGet(() -> ChunkRenderTypeSet.of(RenderType.solid()));
    }

    @Override
    public @NotNull List<BakedQuad> getQuads(
            BlockState state,
            Direction side,
            @NotNull RandomSource rand,
            @NotNull ModelData data,
            RenderType renderType) {
        if (side == null) {
            // Every face of a full cube is cullable, so there is nothing left
            // over for the null side.
            return List.of();
        }

        MaterialTable.Entry entry = entry().orElse(null);
        BlockPos pos = data.has(POSITION) ? data.get(POSITION) : BlockPos.ZERO;
        float repeatU = entry != null ? entry.blocksPerRepeatU() : 1.0F;
        float repeatV = entry != null ? entry.blocksPerRepeatV() : 1.0F;

        int worldU = pos.get(SurfaceUv.uAxis(side));
        int worldV = pos.get(SurfaceUv.vAxis(side));

        TextureAtlasSprite atlasSprite = sprite();
        List<SurfaceUv.Piece> pieces = SurfaceUv.pieces(side, worldU, worldV, repeatU, repeatV);
        List<BakedQuad> quads = new ArrayList<>(pieces.size());
        for (SurfaceUv.Piece piece : pieces) {
            quads.add(quad(side, piece, atlasSprite));
        }
        return quads;
    }

    /** One rectangle of one face, in the block-local space the chunk mesh wants. */
    private static BakedQuad quad(Direction side, SurfaceUv.Piece piece, TextureAtlasSprite sprite) {
        Direction.Axis uAxis = SurfaceUv.uAxis(side);
        Direction.Axis vAxis = SurfaceUv.vAxis(side);
        Direction.Axis normalAxis = side.getAxis();
        float plane = side.getAxisDirection() == Direction.AxisDirection.POSITIVE ? 1.0F : 0.0F;

        float[][] corners = new float[4][5];
        fill(corners[0], uAxis, vAxis, normalAxis, piece.uMin(), piece.vMin(), plane, piece.su0(), piece.sv0());
        fill(corners[1], uAxis, vAxis, normalAxis, piece.uMax(), piece.vMin(), plane, piece.su1(), piece.sv0());
        fill(corners[2], uAxis, vAxis, normalAxis, piece.uMax(), piece.vMax(), plane, piece.su1(), piece.sv1());
        fill(corners[3], uAxis, vAxis, normalAxis, piece.uMin(), piece.vMax(), plane, piece.su0(), piece.sv1());

        // Minecraft culls back faces, so the four corners have to be wound
        // counter-clockwise seen from outside. Rather than keeping a table of
        // which faces need reversing, measure it.
        if (!facesOutward(corners, side)) {
            float[] swap = corners[1];
            corners[1] = corners[3];
            corners[3] = swap;
        }

        int[] vertices = new int[VERTEX_STRIDE * 4];
        int normal = packNormal(side);
        for (int i = 0; i < 4; i++) {
            int base = i * VERTEX_STRIDE;
            vertices[base] = Float.floatToRawIntBits(corners[i][0]);
            vertices[base + 1] = Float.floatToRawIntBits(corners[i][1]);
            vertices[base + 2] = Float.floatToRawIntBits(corners[i][2]);
            vertices[base + 3] = WHITE;
            vertices[base + 4] = Float.floatToRawIntBits(sprite.getU(corners[i][3]));
            vertices[base + 5] = Float.floatToRawIntBits(sprite.getV(corners[i][4]));
            vertices[base + 6] = 0;
            vertices[base + 7] = normal;
        }
        return new BakedQuad(vertices, -1, side, sprite, true);
    }

    private static void fill(
            float[] corner,
            Direction.Axis uAxis,
            Direction.Axis vAxis,
            Direction.Axis normalAxis,
            float u,
            float v,
            float plane,
            float spriteU,
            float spriteV) {
        corner[axisIndex(uAxis)] = u;
        corner[axisIndex(vAxis)] = v;
        corner[axisIndex(normalAxis)] = plane;
        corner[3] = spriteU;
        corner[4] = spriteV;
    }

    private static int axisIndex(Direction.Axis axis) {
        return switch (axis) {
            case X -> 0;
            case Y -> 1;
            case Z -> 2;
        };
    }

    private static boolean facesOutward(float[][] corners, Direction side) {
        float ax = corners[1][0] - corners[0][0];
        float ay = corners[1][1] - corners[0][1];
        float az = corners[1][2] - corners[0][2];
        float bx = corners[2][0] - corners[1][0];
        float by = corners[2][1] - corners[1][1];
        float bz = corners[2][2] - corners[1][2];
        float cx = ay * bz - az * by;
        float cy = az * bx - ax * bz;
        float cz = ax * by - ay * bx;
        var normal = side.getNormal();
        return cx * normal.getX() + cy * normal.getY() + cz * normal.getZ() > 0.0F;
    }

    private static int packNormal(Direction side) {
        var normal = side.getNormal();
        int x = (byte) (normal.getX() * 127) & 0xFF;
        int y = (byte) (normal.getY() * 127) & 0xFF;
        int z = (byte) (normal.getZ() * 127) & 0xFF;
        return x | (y << 8) | (z << 16);
    }

    @Override
    public boolean useAmbientOcclusion() {
        return true;
    }

    @Override
    public boolean isGui3d() {
        return true;
    }

    @Override
    public boolean usesBlockLight() {
        return true;
    }

    @Override
    public boolean isCustomRenderer() {
        return false;
    }

    @SuppressWarnings("deprecation")
    @Override
    public @NotNull TextureAtlasSprite getParticleIcon() {
        return sprite();
    }

    @Override
    public @NotNull TextureAtlasSprite getParticleIcon(@NotNull ModelData data) {
        return sprite();
    }

    @Override
    public @NotNull ItemOverrides getOverrides() {
        return ItemOverrides.EMPTY;
    }

    @Override
    public @NotNull ItemTransforms getTransforms() {
        return ItemTransforms.NO_TRANSFORMS;
    }

    public int index() {
        return this.index;
    }
}
