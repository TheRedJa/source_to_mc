package io.github.theredja.src2mc;

import java.util.ArrayList;
import java.util.List;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.material.MapColor;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * The fixed surface pool of {@code docs/format.md} §2.
 *
 * <p>{@link Src2mc#SURFACE_POOL_SIZE} blocks, named {@code surface_0} upwards,
 * registered unconditionally at startup. Nothing here looks at a bundle: the
 * pool exists before any map does, which is what makes adding a map a resource
 * reload rather than a restart (D6).
 *
 * <p>A surface block is a plain full cube with no block entity (D9). Everything
 * that varies between materials — texture, texture scale, render type, sound —
 * is the material table entry with the same index, read at bake time.
 */
public final class SurfaceBlocks {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(Src2mc.MOD_ID);

    private static final List<DeferredBlock<SurfaceBlock>> POOL = new ArrayList<>(Src2mc.SURFACE_POOL_SIZE);

    static {
        for (int i = 0; i < Src2mc.SURFACE_POOL_SIZE; i++) {
            final int index = i;
            POOL.add(BLOCKS.registerBlock(
                    "surface_" + i,
                    props -> new SurfaceBlock(index, props),
                    // Deliberately uniform across the pool. Occlusion and light
                    // blocking are cached per block state when the registry
                    // freezes, so they cannot come from the material table; only
                    // what is read at bake time can. See the note in
                    // mod/README.md about cutout and translucent materials.
                    BlockBehaviour.Properties.of()
                            .mapColor(MapColor.STONE)
                            .strength(1.5F, 6.0F)
                            .sound(SoundType.STONE)));
        }
    }

    private SurfaceBlocks() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
    }

    public static Block get(int index) {
        return POOL.get(index).get();
    }

    public static int size() {
        return POOL.size();
    }

    /** Carries nothing but its index; the index is the whole state. */
    public static final class SurfaceBlock extends Block {
        private final int index;

        public SurfaceBlock(int index, Properties properties) {
            super(properties);
            this.index = index;
        }

        public int index() {
            return this.index;
        }
    }
}
