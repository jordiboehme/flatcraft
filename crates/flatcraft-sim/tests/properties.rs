//! Task 16: simulated properties of the whole pipeline on random layouts.
//!
//! - `host_random_polygons_meet_algorithm_limit`: the layouts of
//!   `flatcraft-path/tests/support/shapes.rs` (star-shaped polygons,
//!   nestings, open polylines), planned with host compensation in floats
//!   and simulated with the same d: every path within
//!   `ALGORITHM_LIMIT_MM`, nothing uncut, pushed or cut into kept material.
//! - `no_cut_into_kept_material`: rows of pieces 0.5 mm apart, with star
//!   holes, each object in a random keep mode (Pieces, Stencil, Both on the
//!   shared sheet), random weeding, a mat or none, and lifted corners with
//!   a 0.1 mm overshoot (`CornerMode::LiftOvershoot`): no blade tip ever
//!   cuts into kept material, in float. The same on the device grid
//!   (`no_cut_into_kept_material_on_the_device_grid`) fails today and is
//!   ignored with the failing case in `properties.proptest-regressions`
//!   (Task 16 report).

#[path = "../../flatcraft-path/tests/support/shapes.rs"]
mod shapes;

use std::collections::BTreeMap;

use flatcraft_core::document::{ObjectId, WeedLines, WeedSettings};
use flatcraft_core::limits::{MIN_WEED_MARGIN_MM, MIN_WEED_SPACING_MM};
use flatcraft_path::{
    Compensation, CornerMode, CutPlan, HostSettings, KeepMode, PathRef, PipelineSettings,
    PlacedPath, ToolpathKind, plan,
};
use flatcraft_sim::{
    ALGORITHM_LIMIT_MM, DeviceGrid, JobMetrics, SimSettings, measure_kept, simulate,
};
use kurbo::{BezPath, Point, Rect};
use proptest::prelude::*;

/// The cutting area of the uncalibrated 12x12 mat: its grid, inset by 5 mm.
fn area() -> Rect {
    Rect::new(5.0, 5.0, 299.8, 299.8)
}

/// The Cameo 2 rounding: 20 units per mm from the grid origin.
fn grid() -> DeviceGrid {
    DeviceGrid::new(20.0, Point::ZERO)
}

fn host(d: f64, grid: Option<DeviceGrid>) -> Compensation {
    let mut h = HostSettings::new(d, 0.0);
    h.device_grid = grid;
    Compensation::Host(h)
}

/// A planned job simulated and measured with its own kept material.
fn measured(out: &CutPlan, s: &SimSettings) -> JobMetrics {
    let trace = simulate(&out.toolpaths, out.home, s).unwrap();
    measure_kept(&out.toolpaths, &trace, s, &out.kept).0
}

/// Cases of the simulated properties in CI. Each case plans and simulates
/// a whole layout: 256 cases took 16 s (`host_random_polygons_...`) and
/// 25 s (`no_cut_into_kept_material`) in the test profile, so CI runs 64
/// and the full 256 are the `_full` twins (`--ignored`).
const CI_CASES: u32 = 64;
/// Cases of the `_full` twins.
const FULL_CASES: u32 = 256;

/// The layout planned with host compensation in floats for a real d of
/// `d`: every path within `ALGORITHM_LIMIT_MM`, nothing uncut, pushed or
/// cut into kept material.
fn meets_algorithm_limit(layout: &[(usize, shapes::Item)], d: f64) -> Result<(), TestCaseError> {
    let built = shapes::build(layout);
    let s = PipelineSettings {
        compensation: host(d, None),
        ..PipelineSettings::with_defaults(area(), Point::ZERO)
    };
    let out = plan(&shapes::placed(&built), &s).unwrap();
    let m = measured(&out, &SimSettings::float(d));
    for p in &m.per_path {
        let what = format!(
            "d {d}, toolpath {} ({:?})",
            p.toolpath, out.toolpaths[p.toolpath].source
        );
        for (name, v) in [
            ("hausdorff", p.hausdorff_mm),
            ("correspondence", p.max_correspondence_mm),
            ("corner", p.max_corner_deviation_mm),
            ("span frechet", p.max_span_frechet_mm),
            ("start", p.max_start_deviation_mm),
            ("closure gap", p.closure_gap_mm.unwrap_or(0.0)),
        ] {
            prop_assert!(v <= ALGORITHM_LIMIT_MM, "STOP: {what}: {name} {v}");
        }
        prop_assert_eq!(p.uncut_mm, 0.0, "STOP: {}: uncut", what);
        prop_assert_eq!(p.pushed, 0, "STOP: {}: pushed", what);
    }
    prop_assert_eq!(
        m.max_into_kept_mm,
        0.0,
        "STOP: d {}: into kept {:?}",
        d,
        m.worst_spots
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CI_CASES))]

    #[test]
    fn host_random_polygons_meet_algorithm_limit(
        layout in shapes::layout(6),
        d in 0.25f64..=0.3,
    ) {
        meets_algorithm_limit(&layout, d)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(FULL_CASES))]

    #[test]
    #[ignore = "full run (256 cases, about 16 s); run with --ignored"]
    fn host_random_polygons_meet_algorithm_limit_full(
        layout in shapes::layout(6),
        d in 0.25f64..=0.3,
    ) {
        meets_algorithm_limit(&layout, d)?;
    }
}

// ------------------------------------------------------- kept material

/// The gap between neighbouring pieces, mm.
const GAP_MM: f64 = 0.5;

/// The corner mode of cardstock and the like.
const LIFT: CornerMode = CornerMode::LiftOvershoot { overshoot_mm: 0.1 };

#[derive(Debug, Clone)]
struct Piece {
    w: f64,
    h: f64,
    mode: KeepMode,
    /// A star hole in the middle, at most 0.35 of the smaller side.
    hole: Option<shapes::Star>,
}

fn keep_mode() -> impl Strategy<Value = KeepMode> {
    prop_oneof![
        Just(KeepMode::Pieces),
        Just(KeepMode::Stencil),
        Just(KeepMode::Both),
    ]
}

fn piece() -> impl Strategy<Value = Piece> {
    (
        5.0f64..40.0,
        5.0f64..40.0,
        keep_mode(),
        proptest::option::of(shapes::star()),
    )
        .prop_map(|(w, h, mode, hole)| Piece { w, h, mode, hole })
}

fn weed() -> impl Strategy<Value = WeedSettings> {
    (
        any::<bool>(),
        MIN_WEED_MARGIN_MM..10.0,
        any::<bool>(),
        prop_oneof![
            Just(WeedLines::Off),
            Just(WeedLines::Horizontal),
            Just(WeedLines::Vertical),
            Just(WeedLines::Both),
        ],
        MIN_WEED_SPACING_MM..30.0,
    )
        .prop_map(
            |(weed_box, margin_mm, per_object, lines, spacing_mm)| WeedSettings {
                weed_box,
                margin_mm,
                per_object,
                lines,
                spacing_mm,
            },
        )
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    shapes::polygon(&[
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ])
}

/// Up to three rows of pieces from (20, 20), each piece `GAP_MM` from the
/// next in its row and each row `GAP_MM` below the tallest piece of the
/// row above. Piece `k` is object `k + 1`, its hole a second path.
fn rows(rows: &[Vec<Piece>]) -> (Vec<PlacedPath>, BTreeMap<ObjectId, KeepMode>) {
    let (mut input, mut modes) = (Vec::new(), BTreeMap::new());
    let mut y = 20.0;
    for row in rows {
        let mut x = 20.0;
        for p in row {
            let object = ObjectId(input.len() as u64 + 1);
            modes.insert(object, p.mode);
            let mut path = rect_path(x, y, p.w, p.h);
            if let Some(star) = &p.hole {
                let c = Point::new(x + p.w / 2.0, y + p.h / 2.0);
                path.extend(shapes::polygon(&star.points(c, 0.35 * p.w.min(p.h))).iter());
            }
            input.push(PlacedPath {
                path,
                source: PathRef {
                    object,
                    path_index: 0,
                },
            });
            x += p.w + GAP_MM;
        }
        y += row.iter().map(|p| p.h).fold(0.0, f64::max) + GAP_MM;
    }
    (input, modes)
}

/// The rows planned with host compensation (lifted corners) for a real d
/// of `d`, rounded for `grid` or in floats, and simulated the same way: no
/// blade tip cuts into kept material.
fn nothing_cut_into_kept(
    layout: &[Vec<Piece>],
    weed: WeedSettings,
    no_mat: bool,
    d: f64,
    grid: Option<DeviceGrid>,
) -> Result<(), TestCaseError> {
    let (input, keep_modes) = rows(layout);
    let s = PipelineSettings {
        compensation: host(d, grid),
        corner_mode: LIFT,
        weed,
        monotonic_feed: no_mat,
        keep_modes,
        ..PipelineSettings::with_defaults(area(), Point::ZERO)
    };
    let out = plan(&input, &s).unwrap();
    prop_assert!(out.toolpaths.iter().any(|t| t.kind == ToolpathKind::Cut));
    let sim = match grid {
        Some(g) => SimSettings::device(d, g),
        None => SimSettings::float(d),
    };
    let m = measured(&out, &sim);
    prop_assert_eq!(
        m.max_into_kept_mm,
        0.0,
        "STOP: grid {:?} d {}: into kept {:?}",
        grid,
        d,
        m.worst_spots
    );
    Ok(())
}

fn pieces() -> impl Strategy<Value = Vec<Vec<Piece>>> {
    proptest::collection::vec(proptest::collection::vec(piece(), 1..=5), 1..=3)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CI_CASES))]

    #[test]
    fn no_cut_into_kept_material(
        layout in pieces(),
        weed in weed(),
        no_mat in any::<bool>(),
        d in 0.25f64..=0.3,
    ) {
        nothing_cut_into_kept(&layout, weed, no_mat, d, None)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(FULL_CASES))]

    #[test]
    #[ignore = "full run (256 cases, about 25 s); run with --ignored"]
    fn no_cut_into_kept_material_full(
        layout in pieces(),
        weed in weed(),
        no_mat in any::<bool>(),
        d in 0.25f64..=0.3,
    ) {
        nothing_cut_into_kept(&layout, weed, no_mat, d, None)?;
    }

    /// The same rows planned for and snapped to the Cameo 2 device grid.
    #[test]
    #[ignore = "STOP (Task 16): fails on the device grid, a hole start goes 0.057 mm into the piece; see task-16-report.md"]
    fn no_cut_into_kept_material_on_the_device_grid(
        layout in pieces(),
        weed in weed(),
        no_mat in any::<bool>(),
        d in 0.25f64..=0.3,
    ) {
        nothing_cut_into_kept(&layout, weed, no_mat, d, Some(grid()))?;
    }
}
