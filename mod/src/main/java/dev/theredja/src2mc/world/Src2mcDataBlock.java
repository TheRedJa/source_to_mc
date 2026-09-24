package dev.theredja.src2mc.world;

import java.util.function.BiFunction;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;

final class Src2mcDataBlock extends Block implements EntityBlock {
    private final BiFunction<BlockPos, BlockState, BlockEntity> factory;
    private final boolean hidden;
    private final boolean nonColliding;

    Src2mcDataBlock(
        Properties properties,
        BiFunction<BlockPos, BlockState, BlockEntity> factory,
        boolean hidden,
        boolean nonColliding
    ) {
        super(properties);
        this.factory = factory;
        this.hidden = hidden;
        this.nonColliding = nonColliding;
    }

    @Override
    public BlockEntity newBlockEntity(BlockPos position, BlockState state) {
        return factory.apply(position, state);
    }

    @Override public RenderShape getRenderShape(BlockState state) {
        return hidden ? RenderShape.INVISIBLE : RenderShape.MODEL;
    }

    @Override public VoxelShape getShape(BlockState state, BlockGetter level, BlockPos position, CollisionContext context) {
        return nonColliding ? Shapes.empty() : super.getShape(state, level, position, context);
    }

    @Override public VoxelShape getCollisionShape(BlockState state, BlockGetter level, BlockPos position, CollisionContext context) {
        return nonColliding ? Shapes.empty() : super.getCollisionShape(state, level, position, context);
    }
}
