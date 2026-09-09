package dev.theredja.src2mc.world;

import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.state.BlockState;

/** Runtime-rendered data carrier; vanilla must never draw a fallback model. */
final class Src2mcInvisibleBlock extends Block {
    Src2mcInvisibleBlock(Properties properties) { super(properties); }
    @Override public RenderShape getRenderShape(BlockState state) { return RenderShape.INVISIBLE; }
}
