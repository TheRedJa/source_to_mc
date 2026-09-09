package dev.theredja.src2mc.world;

import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.Tag;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;

/** Lossless persistence envelope used before later phases interpret its schema. */
public final class Src2mcDataBlockEntity extends BlockEntity {
    private CompoundTag payload = new CompoundTag();

    Src2mcDataBlockEntity(BlockEntityType<?> type, BlockPos position, BlockState state) {
        super(type, position, state);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        payload = normalizedPayload(tag);
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.merge(payload.copy());
    }

    @Override
    public ClientboundBlockEntityDataPacket getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }

    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        return updatePayload();
    }

    public CompoundTag payload() {
        return payload.copy();
    }

    public void replacePayload(CompoundTag payload) {
        this.payload = payload.copy();
        setChanged();
        if (level != null && !level.isClientSide) {
            level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), 3);
        }
    }

    CompoundTag updatePayload() {
        return copyPayloadForUpdate(payload);
    }

    static CompoundTag copyPayloadForUpdate(CompoundTag payload) {
        return payload.copy();
    }

    /** WorldEdit v3 schematics retain custom payload under their {@code Data} envelope. */
    static CompoundTag normalizedPayload(CompoundTag tag) {
        if (tag.contains("Data", Tag.TAG_COMPOUND)) {
            CompoundTag data = tag.getCompound("Data");
            if (data.contains("schema_version", Tag.TAG_INT)) return data.copy();
        }
        return tag.copy();
    }
}
