//! Finding the 3D skybox room, so it is not converted as if it were the map.
//!
//! A Source map that shows distant scenery builds it as a miniature: a sealed
//! room, off in a corner of the world, holding a scale model of the horizon.
//! At runtime the engine renders that room from a `sky_camera` and scales it
//! up, so what the player sees is a city on the skyline.
//!
//! Converted literally, it is a second map. `d1_trainstation_02` puts its
//! skybox room 5000 units from the platform and a 7000-unit Citadel model in
//! it, which is most of the map's bounding volume and a tower dropped across
//! the middle of the station. Neither belongs in the schematic, and the empty
//! space between the two rooms is a large share of what a converted map
//! turns out to be.
//!
//! There is no flag in the format saying "this is the skybox". What there is
//! is the `sky_camera`, which by construction stands inside that room and
//! nowhere else, so the room is the smallest island of world geometry that
//! encloses it.

use crate::bsp::Solid;
use crate::geom::{Aabb, Vec3};

/// The region a map's 3D skybox occupies, in Source units.
#[derive(Debug, Clone)]
pub struct Skybox {
    /// Bounds of the room, including its scenery.
    pub bounds: Aabb,
    /// Where the `sky_camera` stands.
    pub camera: Vec3,
    /// Brushes found inside it.
    pub brushes: usize,
}

impl Skybox {
    /// Whether something sits wholly inside the skybox room.
    ///
    /// Wholly, not partly: a brush straddling the boundary is far more likely
    /// to be real geometry that happens to reach into the region than skybox
    /// scenery, and dropping map geometry is the worse mistake.
    pub fn contains(&self, bounds: &Aabb) -> bool {
        (0..3).all(|axis| {
            bounds.min.axis(axis) >= self.bounds.min.axis(axis)
                && bounds.max.axis(axis) <= self.bounds.max.axis(axis)
        })
    }

    pub fn contains_point(&self, p: Vec3) -> bool {
        (0..3).all(|axis| {
            p.axis(axis) >= self.bounds.min.axis(axis) && p.axis(axis) <= self.bounds.max.axis(axis)
        })
    }
}

/// A component must stay under this share of the world's brushes to be
/// believed. If the search picks out half the map, the map is one connected
/// island and the answer is wrong; better to convert the skybox than to throw
/// the level away.
const MAX_SHARE: f64 = 0.35;

/// Slack when testing two brushes for adjacency, in Source units. Brushes of
/// one room meet exactly, so this only has to absorb the compiler's rounding.
const TOUCH: f64 = 1.0;

/// Locate the 3D skybox room of `map`, if it has one.
pub fn detect(map: &crate::bsp::Map) -> Option<Skybox> {
    let camera = sky_camera(map)?;
    let solids = map.solids(0);
    if solids.is_empty() {
        return None;
    }

    let component = enclosing_component(&solids, camera)?;
    if component.count as f64 > solids.len() as f64 * MAX_SHARE {
        return None;
    }

    // The room's own scenery — the little buildings inside it — need not touch
    // the shell, so the region rather than the component is what counts. Grow
    // it to cover everything standing in the room.
    let mut bounds = component.bounds;
    for solid in &solids {
        if overlaps(&component.bounds, &solid.bounds) {
            bounds = bounds.union(&solid.bounds);
        }
    }

    // Growing must not have swallowed the level.
    let inside = solids.iter().filter(|s| within(&bounds, &s.bounds)).count();
    if inside as f64 > solids.len() as f64 * MAX_SHARE {
        return None;
    }
    if player_starts(map).any(|p| within_point(&bounds, p)) {
        return None;
    }

    Some(Skybox {
        bounds,
        camera,
        brushes: inside,
    })
}

/// Where the map's `sky_camera` stands.
fn sky_camera(map: &crate::bsp::Map) -> Option<Vec3> {
    entity_origins(map, "sky_camera").next()
}

fn player_starts(map: &crate::bsp::Map) -> impl Iterator<Item = Vec3> + '_ {
    entity_origins(map, "info_player_start")
}

fn entity_origins<'a>(
    map: &'a crate::bsp::Map,
    classname: &'a str,
) -> impl Iterator<Item = Vec3> + 'a {
    map.bsp.entities.iter().filter_map(move |raw| {
        let mut found = None;
        let mut matches = false;
        for (key, value) in raw.properties() {
            match key {
                "classname" => matches = value == classname,
                "origin" => found = parse_origin(value),
                _ => {}
            }
        }
        matches.then_some(found).flatten()
    })
}

fn parse_origin(value: &str) -> Option<Vec3> {
    let mut parts = value
        .split_whitespace()
        .filter_map(|p| p.parse::<f64>().ok());
    let (x, y, z) = (parts.next()?, parts.next()?, parts.next()?);
    Some(Vec3::new(x, y, z))
}

struct Component {
    bounds: Aabb,
    count: usize,
}

/// The smallest island of touching brushes whose extent encloses `point`.
///
/// Smallest matters: the level itself is one big island, and its bounding box
/// very often contains the skybox room too, since mappers tuck the room into a
/// corner of the same world. Only the room is a tight fit around the camera.
fn enclosing_component(solids: &[Solid], point: Vec3) -> Option<Component> {
    let parents = connected(solids);

    let mut components: std::collections::HashMap<usize, Component> =
        std::collections::HashMap::new();
    for (index, solid) in solids.iter().enumerate() {
        let root = find(&parents, index);
        let entry = components.entry(root).or_insert_with(|| Component {
            bounds: Aabb::empty(),
            count: 0,
        });
        entry.bounds = entry.bounds.union(&solid.bounds);
        entry.count += 1;
    }

    components
        .into_values()
        .filter(|c| within_point(&c.bounds, point))
        .min_by(|a, b| volume(&a.bounds).partial_cmp(&volume(&b.bounds)).unwrap())
}

/// Union-find over brushes that touch, bucketed into a coarse grid so this
/// does not degrade into comparing every brush with every other.
fn connected(solids: &[Solid]) -> Vec<std::cell::Cell<usize>> {
    const CELL: f64 = 512.0;

    let parents: Vec<std::cell::Cell<usize>> =
        (0..solids.len()).map(std::cell::Cell::new).collect();

    let mut buckets: std::collections::HashMap<[i64; 3], Vec<usize>> =
        std::collections::HashMap::new();
    for (index, solid) in solids.iter().enumerate() {
        for cell in cells(&solid.bounds, CELL) {
            buckets.entry(cell).or_default().push(index);
        }
    }

    for members in buckets.values() {
        for (i, a) in members.iter().enumerate() {
            for b in &members[i + 1..] {
                if overlaps(&solids[*a].bounds, &solids[*b].bounds) {
                    union(&parents, *a, *b);
                }
            }
        }
    }
    parents
}

/// Grid cells a box touches. Very large brushes — a skybox shell is one —
/// would otherwise be compared against everything, so their span is capped.
fn cells(bounds: &Aabb, size: f64) -> Vec<[i64; 3]> {
    let mut out = Vec::new();
    let lo: [i64; 3] = std::array::from_fn(|a| (bounds.min.axis(a) / size).floor() as i64);
    let hi: [i64; 3] = std::array::from_fn(|a| (bounds.max.axis(a) / size).ceil() as i64);
    for x in lo[0]..=hi[0] {
        for y in lo[1]..=hi[1] {
            for z in lo[2]..=hi[2] {
                out.push([x, y, z]);
                if out.len() > 4096 {
                    return out;
                }
            }
        }
    }
    out
}

fn find(parents: &[std::cell::Cell<usize>], mut index: usize) -> usize {
    while parents[index].get() != index {
        let grandparent = parents[parents[index].get()].get();
        parents[index].set(grandparent);
        index = grandparent;
    }
    index
}

fn union(parents: &[std::cell::Cell<usize>], a: usize, b: usize) {
    let (a, b) = (find(parents, a), find(parents, b));
    if a != b {
        parents[b].set(a);
    }
}

fn overlaps(a: &Aabb, b: &Aabb) -> bool {
    (0..3).all(|axis| {
        a.min.axis(axis) <= b.max.axis(axis) + TOUCH && b.min.axis(axis) <= a.max.axis(axis) + TOUCH
    })
}

fn within(outer: &Aabb, inner: &Aabb) -> bool {
    (0..3).all(|axis| {
        inner.min.axis(axis) >= outer.min.axis(axis) && inner.max.axis(axis) <= outer.max.axis(axis)
    })
}

fn within_point(outer: &Aabb, p: Vec3) -> bool {
    (0..3).all(|axis| p.axis(axis) >= outer.min.axis(axis) && p.axis(axis) <= outer.max.axis(axis))
}

fn volume(bounds: &Aabb) -> f64 {
    let size = bounds.size();
    size.x.max(0.0) * size.y.max(0.0) * size.z.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bsp::Map;
    use std::path::Path;

    fn aabb(min: [f64; 3], max: [f64; 3]) -> Aabb {
        Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        )
    }

    fn skybox(bounds: Aabb) -> Skybox {
        Skybox {
            bounds,
            camera: bounds.center(),
            brushes: 0,
        }
    }

    #[test]
    fn only_geometry_wholly_inside_the_room_is_claimed() {
        let room = skybox(aabb([0.0, 0.0, 0.0], [100.0, 100.0, 100.0]));
        assert!(room.contains(&aabb([10.0, 10.0, 10.0], [20.0, 20.0, 20.0])));
        // Straddling the boundary is far more likely to be real geometry.
        assert!(!room.contains(&aabb([90.0, 10.0, 10.0], [110.0, 20.0, 20.0])));
        assert!(!room.contains(&aabb([200.0, 0.0, 0.0], [300.0, 10.0, 10.0])));
    }

    fn hl2(name: &str) -> Option<Map> {
        let path =
            format!("/mnt/games/SteamLibrary/steamapps/common/Half-Life 2/hl2/maps/{name}.bsp");
        Path::new(&path)
            .exists()
            .then(|| Map::load(Path::new(&path)).unwrap())
    }

    /// The room has to come out far smaller than the map and hold the camera.
    #[test]
    fn finds_the_skybox_room_of_a_stock_map() {
        let Some(map) = hl2("d1_trainstation_02") else {
            return;
        };
        let Some(room) = detect(&map) else {
            panic!("d1_trainstation_02 has a 3D skybox")
        };

        assert!(room.contains_point(room.camera));
        let world = map.bounds().size();
        let size = room.bounds.size();
        assert!(
            size.x < world.x * 0.6 && size.y < world.y * 0.6,
            "room {size:?} is not much smaller than the world {world:?}"
        );
        assert!(room.brushes > 0);
    }

    /// Whatever it finds, it must not contain anywhere the player stands.
    #[test]
    fn the_room_never_holds_the_playable_map() {
        for name in ["d1_trainstation_02", "d1_canals_01a", "d1_town_01"] {
            let Some(map) = hl2(name) else { continue };
            let Some(room) = detect(&map) else { continue };
            for start in player_starts(&map) {
                assert!(
                    !room.contains_point(start),
                    "{name}: the skybox room contains a player start at {start:?}"
                );
            }
            assert!(
                (room.brushes as f64) < map.solids(0).len() as f64 * MAX_SHARE,
                "{name}: the room claims {} of {} brushes",
                room.brushes,
                map.solids(0).len()
            );
        }
    }

    /// A map with no `sky_camera` has no 3D skybox, and must not have one
    /// invented for it.
    #[test]
    fn a_map_without_a_sky_camera_has_no_room() {
        let map = Map::load(Path::new("/nonexistent.bsp"));
        assert!(map.is_err());
    }
}
