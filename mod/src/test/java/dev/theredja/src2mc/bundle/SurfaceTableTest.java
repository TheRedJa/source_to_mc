package dev.theredja.src2mc.bundle;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Test;

final class SurfaceTableTest {
    @Test void resolvesSignedSectionAndPackedLocalCell() {
        var face = new SurfaceTable.Face((2 << 8) | (3 << 4) | 4, 1, 0, 7, 9, 11, 13);
        var table = new SurfaceTable(List.of(), Map.of(new SurfaceTable.SectionPos(-1, 1, -2), List.of(face)));
        assertEquals(List.of(face), table.facesAt(-12, 18, -29));
        assertEquals(List.of(), table.facesAt(-11, 18, -29));
    }
}
