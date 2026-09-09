// src2mc coding-task comparison.
//
// Compares model candidates on three real tasks extracted from this
// repository into standalone scratch crates. Grading is `cargo test` against
// hidden tests restored into the crate before grading — no judge, because the
// answers are exactly checkable.
//
//   Tasks (one scratch crate each, std-only):
//     color    — sRGB/Oklab conversion and perceptual distance (palette/color.rs)
//     polytope — convex polyhedron vertices and volume from brush half-spaces (geom.rs)
//     glob     — material path glob matching for the rule engine (palette/rules.rs)
//
//   Run:
//     ori eval evals/src2mc-coding.eval.ts --pilot 1          # price a sample first
//     ori eval evals/src2mc-coding.eval.ts --timeout 1200000  # full run, roomy ceiling
//
//   Cost: (candidates + incumbent) x cases agent turns. The slate is pinned
//   by name; every slug is asserted against the live catalog at load, so a
//   retirement or typo fails loudly instead of silently shrinking the
//   comparison. Edit CANDIDATES / INCUMBENT below to change it.

import { test } from "bun:test";
import {
  assertModelIsLive,
  pilotCases,
  rankedModels,
  setupAgent,
} from "ori/eval";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const INCUMBENT = "z-ai/glm-5.3-flash:floor";

// Named by the user, resolved to exact catalog slugs. Routing modifiers
// such as ":floor" are not catalog ids; setupAgent accepts them, the
// catalog does not.
const CANDIDATES = [
  "openai/gpt-oss-20b",
  "qwen/qwen3-235b-a22b-2507",
  "deepseek/deepseek-v4-flash",
  "openai/gpt-5.6-luna",
  "google/gemini-3.8-flash",
];

const RUN_TIMEOUT_MS = 900_000; // runaway guard per agent run; the slowest
// measured clean run was 536s (deepseek, pilot), so this is ~1.7x that
const CARGO_TIMEOUT_MS = 120_000; // a cold zero-dep crate tests in ~1s
const TEST_TIMEOUT_MS = 1_800_000; // one candidate races all cases in one
// test; the full run batches 3 cases, so the slowest candidate needs ~3x
// its slowest single case

interface Case {
  id: string;
  crate: string;
  lib: string;
  tests: string;
}

// ---------------------------------------------------------------------------
// Task: color — palette/color.rs
// ---------------------------------------------------------------------------

const COLOR_LIB = `//! Colour conversion and perceptual distance for nearest-block matching.
//!
//! src2mc maps a Source texture's average colour to the nearest Minecraft
//! block. Plain sRGB distance is dominated by brightness, so the search runs
//! in Oklab (Bjorn Ottosson's perceptual colour space): Euclidean distance
//! there behaves like "how different do these look".
//!
//! Only std is available; no external crates.

/// A colour in Oklab: \`l\` is lightness in 0..1, \`a\` and \`b\` are the opponent
/// axes, roughly -0.4..0.4.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklab {
    pub l: f64,
    pub a: f64,
    pub b: f64,
}

/// sRGB channel (0..1) to linear light, via the IEC 61966-2-1 transfer
/// function: a linear segment at or below 0.04045, a gamma 2.4 curve above.
pub fn srgb_to_linear(c: f64) -> f64 {
    todo!()
}

/// Linear light to an sRGB channel (0..1); exact inverse of srgb_to_linear.
pub fn linear_to_srgb(c: f64) -> f64 {
    todo!()
}

/// Linear RGB, each channel 0..1, to Oklab: the standard Ottosson
/// transformation (linear RGB through the LMS matrix, cube roots, then the
/// final 3x3 matrix).
pub fn oklab_from_linear(rgb: [f64; 3]) -> Oklab {
    todo!()
}

/// An 8-bit sRGB triple to Oklab.
pub fn oklab_from_srgb(rgb: [u8; 3]) -> Oklab {
    todo!()
}

/// Linear RGB to an 8-bit sRGB triple, for display. Clamp each channel to
/// 0..1, convert, and round to nearest.
pub fn srgb_from_linear(rgb: [f64; 3]) -> [u8; 3] {
    todo!()
}

/// Squared perceptual distance between two Oklab colours. Squared because
/// only the ordering matters and the square root would be pure cost.
pub fn distance_squared(a: Oklab, b: Oklab) -> f64 {
    todo!()
}
`;

const COLOR_TESTS = `use eval_color::*;

const TOL: f64 = 1e-6;

#[test]
fn endpoints_are_fixed() {
    assert_eq!(srgb_to_linear(0.0), 0.0);
    assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-12);
    assert_eq!(linear_to_srgb(0.0), 0.0);
    assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-12);
}

#[test]
fn transfer_function_matches_the_standard() {
    assert!((srgb_to_linear(0.5) - 0.214_041_140_482_232_55).abs() < TOL);
    // Below the join the curve is the exact linear segment c / 12.92.
    assert!((srgb_to_linear(0.02) - 0.02 / 12.92).abs() < 1e-12);
    assert!((linear_to_srgb(0.214) - 0.499_955_549_340_205_53).abs() < TOL);
    assert!((linear_to_srgb(0.001) - 0.001 * 12.92).abs() < 1e-12);
}

#[test]
fn srgb_and_linear_round_trip() {
    for step in 0..=1000 {
        let c = step as f64 / 1000.0;
        let back = linear_to_srgb(srgb_to_linear(c));
        assert!((back - c).abs() < 1e-9, "{c} -> {back}");
        let forth = srgb_to_linear(linear_to_srgb(c));
        assert!((forth - c).abs() < 1e-9, "{c} -> {forth}");
    }
}

/// The two transfer curves must agree where they change over, or mid-greys
/// come out visibly wrong.
#[test]
fn transfer_curve_is_continuous_at_the_join() {
    let below = srgb_to_linear(0.040_44);
    let above = srgb_to_linear(0.040_46);
    assert!((above - below).abs() < 1e-5);
}

#[test]
fn oklab_matches_the_reference_values() {
    let cases: [(&str, [u8; 3], [f64; 3]); 4] = [
        ("red", [255, 0, 0], [0.627_955_360_6, 0.224_863_061_1, 0.125_846_298_5]),
        ("green", [0, 255, 0], [0.866_439_611_5, -0.233_887_574_2, 0.179_498_479_9]),
        ("blue", [0, 0, 255], [0.452_013_718_4, -0.032_456_984_2, -0.311_528_147_7]),
        ("grey", [128, 128, 128], [0.599_870_801_7, 0.0, 0.0]),
    ];
    for (name, rgb, want) in cases {
        let got = oklab_from_srgb(rgb);
        assert!((got.l - want[0]).abs() < TOL, "{name} l: {got:?}");
        assert!((got.a - want[1]).abs() < TOL, "{name} a: {got:?}");
        assert!((got.b - want[2]).abs() < TOL, "{name} b: {got:?}");
    }
}

#[test]
fn white_is_light_and_neutral() {
    let white = oklab_from_srgb([255, 255, 255]);
    assert!((white.l - 1.0).abs() < 1e-3, "{white:?}");
    assert!(white.a.abs() < 1e-3 && white.b.abs() < 1e-3, "{white:?}");
}

#[test]
fn black_is_the_origin() {
    let black = oklab_from_srgb([0, 0, 0]);
    assert!(black.l.abs() < 1e-9 && black.a.abs() < 1e-9 && black.b.abs() < 1e-9);
}

/// The point of Oklab here: a grey must be nearer another grey than a
/// saturated colour of the same lightness.
#[test]
fn greys_are_nearer_greys_than_hues() {
    let grey = oklab_from_srgb([128, 128, 128]);
    let other_grey = oklab_from_srgb([150, 150, 150]);
    let red = oklab_from_srgb([190, 60, 60]);
    assert!(distance_squared(grey, other_grey) < distance_squared(grey, red));
}

#[test]
fn distance_is_symmetric_zero_on_itself_and_one_white_to_black() {
    let a = oklab_from_srgb([12, 200, 90]);
    let b = oklab_from_srgb([200, 12, 90]);
    assert!((distance_squared(a, b) - distance_squared(b, a)).abs() < 1e-12);
    assert!(distance_squared(a, a).abs() < 1e-12);
    let white = oklab_from_srgb([255, 255, 255]);
    let black = oklab_from_srgb([0, 0, 0]);
    // White's Oklab lightness is 1.0 only to within the constants' own
    // rounding, so this is 1e-6 rather than exact.
    assert!((distance_squared(white, black) - 1.0).abs() < 1e-6);
}

#[test]
fn display_round_trips_through_srgb() {
    let rgb = srgb_from_linear([
        srgb_to_linear(0.5),
        srgb_to_linear(0.25),
        srgb_to_linear(1.0),
    ]);
    assert_eq!(rgb, [128, 64, 255]);
}

#[test]
fn display_clamps_out_of_range_channels() {
    assert_eq!(srgb_from_linear([-0.5, 0.0, 1.5]), [0, 0, 255]);
}
`;

// ---------------------------------------------------------------------------
// Task: polytope — geom.rs
// ---------------------------------------------------------------------------

const POLYTOPE_LIB = `//! Convex polyhedra from brush half-spaces.
//!
//! A Source brush is stored as its planes; the solid is the intersection of
//! the half-spaces \`normal · p <= dist\` (normals face outward). The voxeliser
//! needs the polyhedron's corners and its exact volume to decide how a brush
//! fills block cells.
//!
//! Vec3, Plane and box_planes below are provided scaffolding. The task is to
//! implement polyhedron_vertices and polyhedron_volume. Only std is
//! available; no external crates.

use std::ops::{Add, Div, Mul, Sub};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }

    pub fn splat(v: f64) -> Self {
        Vec3 { x: v, y: v, z: v }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn normalized(self) -> Vec3 {
        let len = self.length();
        if len == 0.0 { self } else { self / len }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Div<f64> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f64) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}

/// A plane in the form \`normal · p = dist\`, the normal facing outward, so a
/// point is inside the solid exactly when \`normal · p - dist <= 0\` holds for
/// every plane.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub normal: Vec3,
    pub dist: f64,
}

impl Plane {
    pub fn new(normal: Vec3, dist: f64) -> Self {
        Plane { normal, dist }
    }

    /// Signed distance from \`p\` to the plane; negative means inside.
    pub fn distance_to(&self, p: Vec3) -> f64 {
        self.normal.dot(p) - self.dist
    }
}

/// The six outward-facing planes of an axis-aligned box. Scaffolding; not
/// part of the task.
pub fn box_planes(min: Vec3, max: Vec3) -> [Plane; 6] {
    [
        Plane::new(Vec3::new(-1.0, 0.0, 0.0), -min.x),
        Plane::new(Vec3::new(1.0, 0.0, 0.0), max.x),
        Plane::new(Vec3::new(0.0, -1.0, 0.0), -min.y),
        Plane::new(Vec3::new(0.0, 1.0, 0.0), max.y),
        Plane::new(Vec3::new(0.0, 0.0, -1.0), -min.z),
        Plane::new(Vec3::new(0.0, 0.0, 1.0), max.z),
    ]
}

/// Vertices of the convex polyhedron formed by intersecting the half-spaces
/// \`normal · p <= dist\`.
///
/// Returns every corner of the solid once, in any order. \`epsilon\` absorbs
/// the floating-point slop in the plane equations: a point within
/// \`epsilon\` of satisfying a half-space counts as inside it, and two
/// candidate corners within \`epsilon\` of each other are the same vertex.
/// Near-parallel plane triples that share no single intersection point
/// contribute none. When the half-spaces enclose nothing the result is
/// empty.
pub fn polyhedron_vertices(planes: &[Plane], epsilon: f64) -> Vec<Vec3> {
    todo!()
}

/// Exact volume of the convex polyhedron defined by \`planes\`. Returns 0.0
/// when the half-spaces enclose no volume.
pub fn polyhedron_volume(planes: &[Plane], epsilon: f64) -> f64 {
    todo!()
}
`;

const POLYTOPE_TESTS = `use eval_polytope::*;

const EPS: f64 = 1e-4;

fn box_planes(min: Vec3, max: Vec3) -> Vec<Plane> {
    eval_polytope::box_planes(min, max).to_vec()
}

/// Every got point within eps of a distinct want point, and the counts equal.
fn same_point_set(got: &[Vec3], want: &[Vec3], eps: f64) -> bool {
    if got.len() != want.len() {
        return false;
    }
    let mut used = vec![false; want.len()];
    'outer: for g in got {
        for (index, w) in want.iter().enumerate() {
            if !used[index] && (*g - *w).length() <= eps {
                used[index] = true;
                continue 'outer;
            }
        }
        return false;
    }
    true
}

#[test]
fn cube_has_exactly_its_eight_corners() {
    let planes = box_planes(Vec3::ZERO, Vec3::splat(16.0));
    let verts = polyhedron_vertices(&planes, EPS);
    let mut want: Vec<Vec3> = Vec::new();
    for x in [0.0, 16.0] {
        for y in [0.0, 16.0] {
            for z in [0.0, 16.0] {
                want.push(Vec3::new(x, y, z));
            }
        }
    }
    assert!(same_point_set(&verts, &want, EPS), "got {verts:?}");
}

#[test]
fn wedge_drops_the_clipped_corners() {
    // A cube sliced by a diagonal plane through two opposite edges: the two
    // corners on the far side of the cut disappear, leaving 6.
    let mut planes = box_planes(Vec3::ZERO, Vec3::splat(16.0));
    planes.push(Plane::new(
        Vec3::new(1.0, 1.0, 0.0).normalized(),
        16.0 / 2f64.sqrt(),
    ));
    let verts = polyhedron_vertices(&planes, EPS);
    assert_eq!(verts.len(), 6, "got {verts:?}");
}

/// A real vertex of a non-degenerate 3D polyhedron is where at least three
/// of its faces meet. This holds for any correct method, not just one
/// particular algorithm.
#[test]
fn every_vertex_lies_on_three_planes() {
    let mut planes = box_planes(Vec3::ZERO, Vec3::splat(1.0));
    planes.push(Plane::new(Vec3::new(1.0, 2.0, 0.5).normalized(), 1.1));
    let verts = polyhedron_vertices(&planes, EPS);
    assert!(!verts.is_empty());
    for v in verts {
        let on = planes
            .iter()
            .filter(|p| p.distance_to(v).abs() <= EPS)
            .count();
        assert!(on >= 3, "{v:?} lies on only {on} planes");
    }
}

#[test]
fn two_parallel_planes_enclose_nothing() {
    let planes = vec![
        Plane::new(Vec3::new(0.0, 0.0, 1.0), 16.0),
        Plane::new(Vec3::new(0.0, 0.0, -1.0), 0.0),
    ];
    assert!(polyhedron_vertices(&planes, EPS).is_empty());
}

#[test]
fn box_volume_is_the_product_of_its_sides() {
    let planes = box_planes(Vec3::new(-1.0, 2.0, 0.5), Vec3::new(3.0, 8.0, 2.5));
    let volume = polyhedron_volume(&planes, 1e-9);
    assert!((volume - 4.0 * 6.0 * 2.0).abs() < 1e-6, "got {volume}");
}

#[test]
fn a_cube_cut_corner_to_corner_has_half_the_volume() {
    let mut planes = box_planes(Vec3::ZERO, Vec3::splat(2.0));
    planes.push(Plane::new(
        Vec3::new(1.0, 1.0, 0.0).normalized(),
        2.0 / 2f64.sqrt(),
    ));
    let volume = polyhedron_volume(&planes, 1e-9);
    assert!((volume - 4.0).abs() < 1e-6, "got {volume}");
}

/// Slicing a unit cube at \`x + y + z = c\` removes a corner tetrahedron of
/// side \`3 - c\`, so long as that side fits within one edge.
#[test]
fn a_sliced_corner_matches_the_hand_computed_volume() {
    let cut = |c: f64| {
        let mut planes = box_planes(Vec3::ZERO, Vec3::splat(1.0));
        planes.push(Plane::new(Vec3::splat(1.0).normalized(), c / 3f64.sqrt()));
        polyhedron_volume(&planes, 1e-9)
    };

    let corner = (3.0f64 - 2.5).powi(3) / 6.0;
    assert!((cut(2.5) - (1.0 - corner)).abs() < 1e-6, "got {}", cut(2.5));

    // Cutting through the centre is exactly half, by symmetry.
    assert!((cut(1.5) - 0.5).abs() < 1e-6, "got {}", cut(1.5));
}

#[test]
fn degenerate_and_empty_polyhedra_have_no_volume() {
    assert_eq!(polyhedron_volume(&[], 1e-9), 0.0);
    // A box with min above max encloses nothing.
    let planes = box_planes(Vec3::splat(1.0), Vec3::ZERO);
    assert_eq!(polyhedron_volume(&planes, 1e-9), 0.0);
}

/// Volume must agree with a dense sampling of the same solid, which is the
/// property exact mode is built on.
#[test]
fn volume_agrees_with_dense_sampling() {
    let mut planes = box_planes(Vec3::ZERO, Vec3::splat(1.0));
    planes.push(Plane::new(Vec3::new(1.0, 2.0, 0.5).normalized(), 1.1));
    let volume = polyhedron_volume(&planes, 1e-9);

    let n = 100;
    let step = 1.0 / n as f64;
    let mut inside = 0u64;
    for i in 0..n {
        for j in 0..n {
            for k in 0..n {
                let p = Vec3::new(
                    (i as f64 + 0.5) * step,
                    (j as f64 + 0.5) * step,
                    (k as f64 + 0.5) * step,
                );
                if planes.iter().all(|pl| pl.distance_to(p) <= 0.0) {
                    inside += 1;
                }
            }
        }
    }
    let sampled = inside as f64 / (n * n * n) as f64;
    assert!(
        (volume - sampled).abs() < 3e-3,
        "exact {volume} vs sampled {sampled}"
    );
}
`;

// ---------------------------------------------------------------------------
// Task: glob — palette/rules.rs matching semantics
// ---------------------------------------------------------------------------

const GLOB_LIB = `//! Material path matching for src2mc's rule engine.
//!
//! Material rules map Source material paths (like "metal/metalgrate011a") to
//! Minecraft blocks, and their patterns are glob-like. One function decides
//! whether a pattern matches a path. Only std is available; no external
//! crates.

/// Does \`pattern\` match the whole of \`path\`?
///
/// - \`*\` matches any sequence of characters, including none, and crosses
///   \`/\` freely, so \`*grate*\` matches "metal/metalgrate011a" wherever the
///   grate part lives.
/// - \`?\` matches exactly one character, also including \`/\`.
/// - Everything else matches itself exactly.
/// - Matching is case-insensitive for ASCII letters only; every other
///   character compares exactly, so "Ä" does not match "ä".
/// - The match must cover the whole path: no prefix or suffix matches count.
/// - A pattern with many stars must still run in polynomial time: backtrack
///   on the most recent star rather than re-exploring earlier choices.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    todo!()
}
`;

const GLOB_TESTS = `use eval_glob::glob_match;

#[test]
fn exact_match_is_anchored_at_both_ends() {
    assert!(glob_match("metal/wall", "metal/wall"));
    assert!(!glob_match("metal/wall", "metal/wall_01"));
    assert!(!glob_match("metal/wall", "xmetal/wall"));
    assert!(!glob_match("metal/wall", "metal/wallx"));
}

#[test]
fn empty_pattern_and_path() {
    assert!(glob_match("", ""));
    assert!(!glob_match("", "x"));
    assert!(!glob_match("x", ""));
    assert!(glob_match("*", ""));
    assert!(glob_match("*", "anything/at/all"));
}

#[test]
fn star_crosses_slashes() {
    assert!(glob_match("*grate*", "metal/metalgrate011a"));
    assert!(glob_match("*grate*", "grates/deep/metalgrate011a.vmt"));
    assert!(glob_match("tools/*", "tools/dev/editor"));
    assert!(glob_match("metal/*grate*", "metal/hull/metalgrate011a"));
    assert!(!glob_match("metal/*grate*", "wood/grate"));
}

#[test]
fn star_is_greedy_but_anchored() {
    assert!(glob_match("a*b", "ab"));
    assert!(glob_match("a*b", "axxb"));
    assert!(!glob_match("a*b", "abx"));
    assert!(!glob_match("a*b", "xab"));
    assert!(glob_match("a*b*c", "abc"));
    assert!(!glob_match("a*b*c", "acb"));
    assert!(!glob_match("a*b*c", "acbb"));
    assert!(glob_match("a*b*c", "axxbyyc"));
    assert!(!glob_match("a*b*c", "accb"));
    assert!(glob_match("a*a*a", "aaaaaaaa"));
    assert!(!glob_match("a*a*a", "aa"));
    assert!(glob_match("abc*", "abc"));
    assert!(glob_match("*abc", "abc"));
    assert!(glob_match("**", "any/thing"));
}

#[test]
fn backtracking_uses_the_most_recent_star() {
    assert!(glob_match("*a*b*c*", "xaayybzzcqq"));
    assert!(!glob_match("*a*b*c*", "xaayyzzcqq"));
    // An implementation that re-explores earlier star choices
    // exponentially hangs here; a polynomial one answers instantly. The
    // twelve 'a' literals need twelve path 'a' characters and a final 'b',
    // and the path has no 'b', so the answer is false.
    let pattern = format!("{}b", "a*".repeat(12));
    let path = "a".repeat(80);
    assert!(!glob_match(&pattern, &path));
}

#[test]
fn question_mark_is_exactly_one_character() {
    assert!(glob_match("metal/?ool", "metal/tool"));
    assert!(!glob_match("metal/?ool", "metal/tttool"));
    assert!(!glob_match("metal/??l", "metal/tool"));
    assert!(glob_match("abc?", "abcd"));
    assert!(!glob_match("abc?", "abc"));
}

#[test]
fn question_mark_crosses_slashes() {
    assert!(glob_match("a?b", "a/b"));
    assert!(!glob_match("a?b", "a//b"));
}

#[test]
fn ascii_letters_only_are_case_insensitive() {
    assert!(glob_match("METAL/*", "metal/wall_01"));
    assert!(glob_match("metal/*", "METAL/WALL_01"));
    assert!(glob_match("MiXeD", "mIxEd"));
    // Non-ASCII compares exactly: no Unicode case folding.
    assert!(!glob_match("Ä*", "äx"));
    assert!(glob_match("Ä*", "Äx"));
}
`;

// ---------------------------------------------------------------------------
// Cases and scratch crates
// ---------------------------------------------------------------------------

const TASKS: Case[] = [
  { id: "color", crate: "eval_color", lib: COLOR_LIB, tests: COLOR_TESTS },
  { id: "polytope", crate: "eval_polytope", lib: POLYTOPE_LIB, tests: POLYTOPE_TESTS },
  { id: "glob", crate: "eval_glob", lib: GLOB_LIB, tests: GLOB_TESTS },
];

const CASES = pilotCases(TASKS);

const cargoToml = (crate: string): string =>
  `[package]\nname = "${crate}"\nversion = "0.1.0"\nedition = "2024"\n`;

const makeScratch = (kase: Case): string => {
  const dir = mkdtempSync(path.join(tmpdir(), `src2mc-eval-${kase.id}-`));
  writeFileSync(path.join(dir, "Cargo.toml"), cargoToml(kase.crate));
  mkdirSync(path.join(dir, "src"));
  mkdirSync(path.join(dir, "tests"));
  writeFileSync(path.join(dir, "src", "lib.rs"), kase.lib);
  writeFileSync(path.join(dir, "tests", "eval.rs"), kase.tests);
  return dir;
};

const tail = (text: string, lines = 40): string => {
  const all = text.trimEnd().split("\n");
  return all.slice(-lines).join("\n");
};

const gradeScratch = (
  dir: string,
  kase: Case
): { ok: boolean; passed: number; detail: string } => {
  // The prompt forbids touching Cargo.toml and the tests; grading restores
  // pristine copies anyway, so a "cheating" edit cannot survive.
  writeFileSync(path.join(dir, "Cargo.toml"), cargoToml(kase.crate));
  writeFileSync(path.join(dir, "tests", "eval.rs"), kase.tests);

  const result = spawnSync("cargo", ["test", "--quiet"], {
    cwd: dir,
    encoding: "utf8",
    timeout: CARGO_TIMEOUT_MS,
    killSignal: "SIGKILL",
  });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  if (result.error !== undefined && result.status === null) {
    return {
      ok: false,
      passed: 0,
      detail: `cargo could not run: ${result.error.message}`,
    };
  }
  if (result.status !== 0) {
    return {
      ok: false,
      passed: 0,
      detail: `cargo test exited ${result.status ?? result.signal}:\n${tail(output)}`,
    };
  }
  // cargo prints one result line per target — the lib, each integration
  // test, doc-tests. The hidden suite lives in tests/eval.rs, so the count
  // is the sum across targets, not the first line (an empty lib target
  // prints "0 passed" first).
  const passed = [...output.matchAll(/test result: ok\. (\d+) passed/g)].reduce(
    (sum, match) => sum + Number(match[1]),
    0
  );
  if (passed < 3) {
    return {
      ok: false,
      passed,
      detail: `cargo test succeeded but only ${passed} tests ran; the hidden suite has more:\n${tail(output)}`,
    };
  }
  return { ok: true, passed, detail: "" };
};

const promptFor = (kase: Case, dir: string): string => `You are implementing a small, precisely specified piece of Rust for src2mc — a tool that converts Source Engine maps (.bsp) into Minecraft schematics. The task was extracted from its codebase into a standalone scratch crate; nothing outside the crate matters.

Scratch crate: ${dir}
- src/lib.rs — the scaffold below, with the full contract in its doc comments; every function body is a todo!() stub.
- tests/eval.rs — hidden acceptance tests. Grading restores pristine copies of that file and of Cargo.toml, then runs \`cargo test\`. The hidden tests are the authority.

The scaffold, exactly as it exists on disk:

--- src/lib.rs ---
${kase.lib}
------------------

Your job:
1. Replace ${dir}/src/lib.rs with the completed implementation: same public API, every body implemented to the doc-comment contract, no todo!() or unimplemented!() left.
2. std only. The pristine Cargo.toml declares no dependencies, so anything that needs an external crate fails to compile at grading.
3. Do not modify tests/eval.rs or Cargo.toml; they are restored before grading, so it cannot help.
4. Write the file with your usual file-writing tooling, using the absolute path above — your working directory is not the crate.
5. You may run \`cargo test\` inside ${dir} to check yourself; it needs no network. If a sandboxed shell command fails with a read-only or bwrap error, retry that same command with escalation enabled.
6. Done means the implementation is complete and you believe it satisfies the contract. Reply with a one-sentence summary.`;

// ---------------------------------------------------------------------------
// Slate: five candidates from the catalog plus the pinned incumbent
// ---------------------------------------------------------------------------

const incumbent = await (async (): Promise<string> => {
  try {
    await assertModelIsLive(INCUMBENT);
    return INCUMBENT;
  } catch (error) {
    // A routing modifier such as ":floor" resolves through its base slug.
    const base = INCUMBENT.split(":")[0];
    console.warn(
      `[src2mc-coding] ${INCUMBENT} is not in the catalog (${String(error)}); pinning its base ${base} instead.`
    );
    await assertModelIsLive(base);
    return base;
  }
})();

for (const slug of CANDIDATES) {
  await assertModelIsLive(slug);
}

const RUNNERS = [...CANDIDATES, incumbent];

const prices = new Map(
  (await rankedModels({ slugs: CANDIDATES })).map((model) => [model.slug, model])
);
const fmtPrice = (value: number | undefined): string =>
  value === undefined ? "?" : `$${(value * 1e6).toFixed(2)}/M`;
console.warn(
  `[src2mc-coding] slate: ${RUNNERS.map((slug) => {
    const row = prices.get(slug);
    return row
      ? `${slug} (${fmtPrice(row.promptPrice)} in, ${fmtPrice(row.completionPrice)} out)`
      : slug;
  }).join(", ")}`
);

test.concurrent.each(RUNNERS)(
  "src2mc coding tasks: %s",
  async (model) => {
    const agent = setupAgent({ model });
    const failures: string[] = [];
    const passes: string[] = [];

    // All cases race inside one candidate test: bun keeps concurrent tests
    // independent, so a failure here still lets every case finish and book
    // its row.
    await Promise.allSettled(
      CASES.map(async (kase) => {
        const dir = makeScratch(kase);
        let keep = false;
        try {
          const run = await agent.run({ prompt: promptFor(kase, dir) });
          run.toComplete();
          run.toFinishWithin(RUN_TIMEOUT_MS);
          const grade = gradeScratch(dir, kase);
          if (grade.ok) {
            const cost = run.costUsd === undefined ? "unknown" : `$${run.costUsd.toFixed(4)}`;
            passes.push(`${kase.id} (${grade.passed} tests, ${cost})`);
          } else {
            keep = true;
            failures.push(`${kase.id} — ${grade.detail}\n    scratch kept at ${dir}`);
          }
        } catch (error) {
          keep = true;
          failures.push(
            `${kase.id} — ${error instanceof Error ? error.message : String(error)}\n    scratch kept at ${dir}`
          );
        } finally {
          if (!keep) {
            rmSync(dir, { recursive: true, force: true });
          }
        }
      })
    );

    const summary = `${model}: ${passes.length}/${CASES.length} passed (${passes.join("; ") || "none"})`;
    if (failures.length > 0) {
      throw new Error(`${summary}\nfailures:\n${failures.join("\n\n")}`);
    }
    console.log(summary);
  },
  TEST_TIMEOUT_MS
);
