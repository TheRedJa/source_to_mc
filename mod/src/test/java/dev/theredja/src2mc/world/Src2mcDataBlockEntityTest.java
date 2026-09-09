package dev.theredja.src2mc.world;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotSame;

import net.minecraft.nbt.CompoundTag;
import org.junit.jupiter.api.Test;

final class Src2mcDataBlockEntityTest {
    @Test void unwrapsWorldEditsSpongeV3DataEnvelope() {
        CompoundTag data = new CompoundTag(); data.putInt("schema_version", 1); data.putString("stable_id", "root");
        CompoundTag envelope = new CompoundTag(); envelope.put("Data", data); envelope.putString("id", "src2mc:prop_root");
        CompoundTag payload = Src2mcDataBlockEntity.normalizedPayload(envelope);
        assertEquals(1, payload.getInt("schema_version"));
        assertEquals("root", payload.getString("stable_id"));
        assertFalse(payload.contains("Data"));
    }

    @Test void retainsDirectMinecraftPayload() {
        CompoundTag direct = new CompoundTag(); direct.putInt("schema_version", 1); direct.putString("campaign_id", "hl2");
        assertEquals(direct, Src2mcDataBlockEntity.normalizedPayload(direct));
    }

    @Test void clientUpdatePayloadIsAnIndependentCopy() {
        CompoundTag original = new CompoundTag(); original.putInt("schema_version", 1);
        CompoundTag copy = Src2mcDataBlockEntity.copyPayloadForUpdate(original);
        assertNotSame(original, copy);
        copy.putInt("schema_version", 2);
        assertEquals(1, original.getInt("schema_version"));
    }
}
