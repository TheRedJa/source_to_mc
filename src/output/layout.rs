//! Placing several converted maps side by side.
//!
//! Offsets come from each map's own converted footprint rather than a fixed
//! stride. Source map sizes vary by more than an order of magnitude — a
//! corridor map next to a coastline — so a stride large enough for the biggest
//! would strand the rest in empty space, and one sized for the average would
//! overlap.

/// Footprint of one map in blocks, on the horizontal axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footprint {
    pub width: i32,
    pub depth: i32,
}

impl Footprint {
    pub fn new(width: i32, depth: i32) -> Footprint {
        Footprint {
            width: width.max(0),
            depth: depth.max(0),
        }
    }
}

/// Number of columns that makes the grid roughly square.
pub fn columns_for(count: usize) -> usize {
    (count as f64).sqrt().ceil().max(1.0) as usize
}

/// Block-space offsets placing every map in a grid with `spacing` between them.
///
/// Rows are packed left to right; each row is as deep as its tallest map, so a
/// row of small maps does not inherit the depth of a large one elsewhere.
pub fn grid(footprints: &[Footprint], columns: usize, spacing: i32) -> Vec<[i32; 3]> {
    let columns = columns.max(1);
    let spacing = spacing.max(0);

    let mut offsets = Vec::with_capacity(footprints.len());
    let (mut x, mut z, mut row_depth) = (0, 0, 0);

    for (index, footprint) in footprints.iter().enumerate() {
        if index > 0 && index % columns == 0 {
            x = 0;
            z += row_depth + spacing;
            row_depth = 0;
        }
        offsets.push([x, 0, z]);
        x += footprint.width + spacing;
        row_depth = row_depth.max(footprint.depth);
    }

    offsets
}

/// Every map at the origin, for comparing versions of one map.
pub fn stacked(count: usize) -> Vec<[i32; 3]> {
    vec![[0, 0, 0]; count]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlaps(a: ([i32; 3], Footprint), b: ([i32; 3], Footprint)) -> bool {
        let x = a.0[0] < b.0[0] + b.1.width && b.0[0] < a.0[0] + a.1.width;
        let z = a.0[2] < b.0[2] + b.1.depth && b.0[2] < a.0[2] + a.1.depth;
        x && z
    }

    /// The whole point: whatever the mix of sizes, no two maps may share
    /// ground. A campaign pasted into one world has to be walkable.
    #[test]
    fn no_two_maps_overlap_whatever_their_sizes() {
        let footprints: Vec<Footprint> = [
            (1200, 900),
            (80, 60),
            (4000, 300),
            (500, 2500),
            (150, 150),
            (900, 900),
            (30, 4000),
        ]
        .iter()
        .map(|(w, d)| Footprint::new(*w, *d))
        .collect();

        for spacing in [0, 1, 64, 512] {
            let columns = columns_for(footprints.len());
            let offsets = grid(&footprints, columns, spacing);
            assert_eq!(offsets.len(), footprints.len());

            for i in 0..footprints.len() {
                for j in (i + 1)..footprints.len() {
                    assert!(
                        !overlaps((offsets[i], footprints[i]), (offsets[j], footprints[j])),
                        "maps {i} and {j} overlap at spacing {spacing}: \
                         {:?} {:?} vs {:?} {:?}",
                        offsets[i],
                        footprints[i],
                        offsets[j],
                        footprints[j],
                    );
                }
            }
        }
    }

    #[test]
    fn maps_are_separated_by_at_least_the_spacing() {
        let footprints = vec![Footprint::new(100, 100); 4];
        let offsets = grid(&footprints, 2, 50);
        // Two per row, so the second starts a map-width plus the gap along x.
        assert_eq!(offsets[0], [0, 0, 0]);
        assert_eq!(offsets[1], [150, 0, 0]);
        assert_eq!(offsets[2], [0, 0, 150]);
        assert_eq!(offsets[3], [150, 0, 150]);
    }

    /// A row's depth must come from that row alone, or one tall map pushes
    /// every later row away from it.
    #[test]
    fn a_row_is_only_as_deep_as_its_own_maps() {
        let footprints = vec![
            Footprint::new(100, 100),
            Footprint::new(100, 100),
            Footprint::new(100, 5000),
            Footprint::new(100, 100),
        ];
        let offsets = grid(&footprints, 2, 0);
        assert_eq!(
            offsets[2][2], 100,
            "second row should follow the first's depth"
        );
        // The tall map is in the second row, so the third row clears it.
        assert_eq!(offsets[3][2], 100);
    }

    #[test]
    fn nothing_in_never_places_anything() {
        assert!(grid(&[], 3, 64).is_empty());
        assert!(stacked(0).is_empty());
    }

    #[test]
    fn a_single_map_sits_at_the_origin() {
        assert_eq!(grid(&[Footprint::new(500, 500)], 1, 64), vec![[0, 0, 0]]);
    }

    #[test]
    fn stacking_puts_every_map_at_the_origin() {
        assert_eq!(stacked(3), vec![[0, 0, 0]; 3]);
    }

    #[test]
    fn the_grid_stays_roughly_square() {
        assert_eq!(columns_for(0), 1);
        assert_eq!(columns_for(1), 1);
        assert_eq!(columns_for(4), 2);
        assert_eq!(columns_for(5), 3);
        assert_eq!(columns_for(92), 10);
    }

    /// Degenerate inputs must not produce negative offsets or a panic.
    #[test]
    fn zero_sized_maps_are_handled() {
        let offsets = grid(&[Footprint::new(0, 0), Footprint::new(-5, -5)], 1, 0);
        assert_eq!(offsets, vec![[0, 0, 0], [0, 0, 0]]);
    }
}
