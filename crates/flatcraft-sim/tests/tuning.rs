//! Task 10: the tuning sweeps. Each knife constant is decided here by a
//! measured rule, never by a guess (R7). Every sweep prints its table as
//! Markdown (run with `--nocapture`); `docs/quality/knife-compensation.md`
//! holds the tables of the last full run.
//!
//! The shapes are the test sheet's (`flatcraft_engine::patterns`): circles,
//! star tips, the 50 mm square, the square with a hole, a 10 mm round hole
//! and the S-curve, at their sheet positions. They are planned on the
//! cutting area of an uncalibrated 12x12 mat (the grid, inset by 5 mm) with
//! `Compensation::Host` at d = 0.25 and 0.3 mm, and simulated in float and
//! in device mode (R3): 20 units per mm, grid origin 0. Kept material is
//! the plan's own (`KeptMaterial::from_toolpaths` with the shared sheet).
//!
//! The full sweeps are `#[ignore]`d (about a minute in a dev build); each
//! has a fast twin that runs the same rule on the cases that decide it and
//! checks that the rule's answer is the pinned constant.

use flatcraft_core::document::{CutStrategy, ObjectId};
use flatcraft_core::geometry::RingDirection;
use flatcraft_path::knife::{self, ARC_STEP_DEG, HOLE_LEAD_IN_RADIUS_MM, SWIVEL_MAX_DEG};
use flatcraft_path::plan::DEFAULT_OVERCUT_MM;
use flatcraft_path::{
    AlignmentStroke, CORNER_DEG, Compensation, CutPlan, HostSettings, KeepMode, KeepSide,
    KeptMaterial, PathRef, PipelineSettings, PlacedPath, StrategySettings, Toolpath, ToolpathKind,
    WASTE_SEG, passes, plan, rings,
};
use flatcraft_sim::{
    ALGORITHM_LIMIT_MM, DEVICE_LIMIT_MM, DeviceGrid, END_TO_END_LIMIT_MM, JobMetrics,
    SAMPLE_STEP_FACTOR, SimSettings, SimTrace, measure_kept, simulate,
};
use kurbo::{Arc, BezPath, Circle, Point, Rect, Shape, Vec2};

/// The offsets every sweep plans and simulates with.
const OFFSETS: [f64; 2] = [0.25, 0.3];

/// The cutting area of the uncalibrated 12x12 mat: its grid, inset by 5 mm.
fn area() -> Rect {
    Rect::new(5.0, 5.0, 299.8, 299.8)
}

/// The device home: the media corner, at the grid origin.
const HOME: Point = Point::ZERO;

/// The Cameo 2 rounding: 20 units per mm from the grid origin.
fn grid() -> DeviceGrid {
    DeviceGrid::new(20.0, Point::ZERO)
}

/// A calibrated plotter on a mat with an odd grid origin: scale 1.002 and
/// 0.998, mirrored across the carriage, inside a 12x12 mat.
fn calibrated_grid() -> DeviceGrid {
    DeviceGrid {
        units_per_mm: 20.0,
        origin: Point::new(0.37, 0.013),
        media_size: kurbo::Size::new(304.8, 330.2),
        mirror_x: true,
        mirror_y: false,
        scale: Vec2::new(1.002, 0.998),
    }
}

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn heading(deg: f64) -> Vec2 {
    Vec2::from_angle(deg.to_radians())
}

fn source(object: u64) -> PathRef {
    PathRef {
        object: ObjectId(object),
        path_index: 0,
    }
}

fn placed(object: u64, path: BezPath) -> PlacedPath {
    PlacedPath {
        path,
        source: source(object),
    }
}

fn polygon(pts: &[Point]) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(pts[0]);
    for &q in &pts[1..] {
        b.line_to(q);
    }
    b.close_path();
    b
}

fn polyline(pts: &[Point]) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(pts[0]);
    for &q in &pts[1..] {
        b.line_to(q);
    }
    b
}

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    polygon(&[p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h)])
}

fn circle(c: Point, r: f64) -> BezPath {
    Circle::new(c, r).to_path(1e-4)
}

// ------------------------------------------------------------------ shapes

/// One job of the sweeps.
#[derive(Clone)]
struct Case {
    name: String,
    input: Vec<PlacedPath>,
}

fn case(name: impl Into<String>, input: Vec<PlacedPath>) -> Case {
    Case {
        name: name.into(),
        input,
    }
}

/// Apex angles of the sheet's star tips.
const STAR_ANGLES: [f64; 8] = [15.0, 30.0, 45.0, 60.0, 90.0, 120.0, 150.0, 170.0];

/// A closed V tip with its apex at `apex`, pointing up (-y), legs 20 mm.
fn star_tip(apex: Point, angle_deg: f64) -> BezPath {
    let half = angle_deg.to_radians() / 2.0;
    let (dx, dy) = (20.0 * half.sin(), 20.0 * half.cos());
    polygon(&[
        apex,
        p(apex.x + dx, apex.y + dy),
        p(apex.x - dx, apex.y + dy),
    ])
}

fn star_cases() -> Vec<Case> {
    STAR_ANGLES
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let apex = p(25.0 + 50.0 * (i % 4) as f64, [150.0, 190.0][i / 4]);
            case(format!("star-{a}"), vec![placed(1, star_tip(apex, a))])
        })
        .collect()
}

/// The sheet's circles, at their sheet positions.
fn circle_cases() -> Vec<Case> {
    let mut x = 10.0;
    let mut out = Vec::new();
    for r in [0.5, 1.0, 2.0, 5.0, 10.0, 25.0] {
        out.push(case(
            format!("circle-r{r}"),
            vec![placed(1, circle(p(x + r, 105.0), r))],
        ));
        x += 2.0 * r + 10.0;
    }
    out
}

fn s_curve() -> BezPath {
    let mut b = BezPath::new();
    b.move_to((190.0, 40.0));
    b.curve_to((200.0, 5.0), (225.0, 5.0), (225.0, 25.0));
    b.curve_to((225.0, 45.0), (250.0, 45.0), (260.0, 10.0));
    b
}

/// A 40 mm square piece with a 10 mm round hole in its middle.
fn round_hole(c: Point) -> Vec<PlacedPath> {
    vec![
        placed(1, rect_path(c.x - 20.0, c.y - 20.0, 40.0, 40.0)),
        placed(1, circle(c, 5.0)),
    ]
}

/// The test sheet's shapes, one job each.
fn sheet() -> Vec<Case> {
    let mut out = vec![
        case(
            "square-50",
            vec![placed(1, rect_path(10.0, 10.0, 50.0, 50.0))],
        ),
        case(
            "hole-in-square",
            vec![
                placed(1, rect_path(80.0, 10.0, 30.0, 30.0)),
                placed(1, rect_path(90.0, 20.0, 10.0, 10.0)),
            ],
        ),
        case("round-hole-10", round_hole(p(150.0, 230.0))),
        case("s-curve", vec![placed(1, s_curve())]),
    ];
    out.extend(circle_cases());
    out.extend(star_cases());
    out
}

/// A closed needle: a V tip of 2 deg, so its apex turns 178 deg.
fn needle() -> Case {
    case("needle-2", vec![placed(1, star_tip(p(250.0, 150.0), 2.0))])
}

/// An open hairpin: out and straight back, a 180 deg turn.
fn hairpin() -> Case {
    case(
        "hairpin-180",
        vec![placed(
            1,
            polyline(&[p(200.0, 270.0), p(240.0, 270.0), p(200.0, 270.0)]),
        )],
    )
}

/// Probes for the pivot threshold: the sheet's curves all turn more than
/// 2 deg per flattened vertex, so they cannot tell 0.5, 1 and 2 deg apart.
/// - `turns`: ten open lines, each 10 mm, then one turn of just under
///   0.5, 1, 2, 5 or 10 deg (the largest turn that threshold skips), either
///   way, then 150 mm straight on. A skipped turn leaves the axis
///   `d * sin(turn)` off the next segment's axis line, and the axis only
///   closes that gap over the segment, so a long segment is the worst case.
/// - `circle-r100` turns about 1.6 deg per vertex once flattened, and
///   `arc-r400` about 0.8 deg: the gentle curves of large designs.
fn threshold_probes() -> Vec<Case> {
    let turns = [0.5, -0.5, 1.0, -1.0, 2.0, -2.0, 5.0, -5.0, -10.0, 10.0]
        .iter()
        .enumerate()
        .map(|(k, &t)| {
            let a = p(20.0, 20.0 + 28.0 * k as f64);
            let b = a + Vec2::new(10.0, 0.0);
            let c = b + 150.0 * heading(t * (1.0 - 1e-6));
            placed(k as u64 + 1, polyline(&[a, b, c]))
        })
        .collect();
    let span = 60.0 / 400.0;
    let arc = Arc::new(
        p(150.0, 270.0 - 400.0),
        (400.0, 400.0),
        std::f64::consts::FRAC_PI_2 - span / 2.0,
        span,
        0.0,
    );
    vec![
        case("turns", turns),
        case(
            "circle-r100",
            vec![placed(1, circle(p(152.0, 152.0), 100.0))],
        ),
        case("arc-r400", vec![placed(1, arc.to_path(1e-4))]),
    ]
}

// ------------------------------------------------------------------ knobs

/// The knife values a sweep varies.
#[derive(Debug, Clone, Copy)]
struct Knob {
    threshold: f64,
    arc_step: f64,
    hole_radius: f64,
    swivel_max: f64,
    /// Plan for these device units (rounding-aware runs), or in float.
    grid: Option<DeviceGrid>,
}

impl Knob {
    /// The pinned values, with the Quality strategy's pivot threshold.
    fn pinned() -> Self {
        Self {
            threshold: StrategySettings::for_strategy(CutStrategy::Quality)
                .host_pivot_threshold_deg,
            arc_step: ARC_STEP_DEG,
            hole_radius: HOLE_LEAD_IN_RADIUS_MM,
            swivel_max: SWIVEL_MAX_DEG,
            grid: None,
        }
    }

    /// The same values, planned for the device units of `mode`.
    fn for_mode(self, mode: Mode) -> Self {
        Self {
            grid: (mode == Mode::Device).then(grid),
            ..self
        }
    }

    fn host(self, d: f64) -> Compensation {
        let mut h = HostSettings::new(d, self.threshold);
        h.arc_step_deg = self.arc_step;
        h.hole_lead_in_radius_mm = self.hole_radius;
        h.swivel_max_deg = self.swivel_max;
        h.device_grid = self.grid;
        Compensation::Host(h)
    }
}

fn settings(d: f64, k: Knob) -> PipelineSettings {
    PipelineSettings {
        compensation: k.host(d),
        ..PipelineSettings::with_defaults(area(), HOME)
    }
}

fn planned(c: &Case, s: &PipelineSettings) -> CutPlan {
    plan(&c.input, s).unwrap_or_else(|e| panic!("{}: {e}", c.name))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Float,
    Device,
}

/// The simulator settings for a real offset `d`, with the pinned sample
/// step or `d / factor`.
fn sim(d: f64, mode: Mode, factor: Option<f64>) -> SimSettings {
    let mut s = match mode {
        Mode::Float => SimSettings::float(d),
        Mode::Device => SimSettings::device(d, grid()),
    };
    if let Some(f) = factor {
        s.sample_step_mm = d / f;
    }
    s
}

/// `c` planned with `k` for `mode` (device mode plans for the device
/// units), simulated in `mode` and measured.
fn run_mode(c: &Case, d: f64, k: Knob, mode: Mode) -> (CutPlan, SimTrace, JobMetrics) {
    let out = planned(c, &settings(d, k.for_mode(mode)));
    let (trace, m) = measured(&out, &sim(d, mode, None));
    (out, trace, m)
}

/// A planned job simulated and measured with its own kept material.
fn measured(out: &CutPlan, s: &SimSettings) -> (SimTrace, JobMetrics) {
    let trace = simulate(&out.toolpaths, out.home, s).unwrap();
    let (m, _) = measure_kept(&out.toolpaths, &trace, s, &out.kept);
    (trace, m)
}

/// The deviation a limit is judged on: the larger of the two-sided
/// Hausdorff distance and the correspondence deviation.
fn deviation(m: &JobMetrics) -> f64 {
    m.max_hausdorff_mm.max(m.max_correspondence_mm)
}

/// The largest distance of the tip from a corner vertex while the axis
/// runs its pivot arc: from `v + d * t_in` to `v + d * t_out`. Float mode
/// only (the device moves the arc ends).
fn pivot_drift(paths: &[Toolpath], trace: &SimTrace, d: f64) -> f64 {
    let x = &trace.samples;
    let mut worst = 0.0f64;
    let mut from = 0;
    for (i, t) in paths.iter().enumerate() {
        if t.kind != ToolpathKind::Cut {
            continue;
        }
        let q = &t.points;
        for j in 1..q.len().saturating_sub(1) {
            if rings::turn_deg(q[j - 1], q[j], q[j + 1]) < CORNER_DEG {
                continue;
            }
            let (ti, to) = ((q[j] - q[j - 1]).normalize(), (q[j + 1] - q[j]).normalize());
            let (arrive_at, leave_at) = (q[j] + d * ti, q[j] + d * to);
            let Some(a) = x[from..].iter().position(|s| {
                s.down
                    && s.toolpath == i
                    && s.seg == (j - 1) as u32
                    && s.axis.distance(arrive_at) < 1e-9
            }) else {
                continue;
            };
            let a = from + a;
            let Some(l) = x[a..]
                .iter()
                .position(|s| s.toolpath == i && s.axis.distance(leave_at) < 1e-9)
            else {
                continue;
            };
            for s in &x[a..=a + l] {
                worst = worst.max(s.tip.distance(q[j]));
            }
            from = a;
        }
    }
    worst
}

/// Interior vertices of the planned paths that turn by more than 0 and at
/// most `threshold` degrees: the pivots the threshold skips.
fn skipped(out: &CutPlan, threshold: f64) -> usize {
    out.toolpaths
        .iter()
        .filter(|t| t.kind == ToolpathKind::Cut)
        .flat_map(|t| {
            t.points
                .windows(3)
                .map(|w| rings::turn_deg(w[0], w[1], w[2]))
                .collect::<Vec<_>>()
        })
        .filter(|&a| a > 1e-9 && a <= threshold)
        .count()
}

fn table_header(cols: &[&str]) {
    println!();
    println!("| {} |", cols.join(" | "));
    println!(
        "|{}|",
        cols.iter().map(|_| "---").collect::<Vec<_>>().join("|")
    );
}

fn row(cells: &[String]) {
    println!("| {} |", cells.join(" | "));
}

fn mm(v: f64) -> String {
    format!("{v:.4}")
}

// ------------------------------------------------------- sample step factor

const FACTORS: [f64; 4] = [10.0, 20.0, 40.0, 80.0];

/// How far a misaligned first start may move from its value at 80. Its
/// errors are 0.1 to 0.3 mm, so a 0.005 mm reading error is below 5 %.
const MISALIGNED_TOLERANCE_MM: f64 = 0.005;

/// The values the sample step must not change, in float mode.
fn step_values(out: &CutPlan, d: f64, factor: f64, start: f64) -> [f64; 5] {
    let s = SimSettings {
        start_heading: heading(start),
        ..sim(d, Mode::Float, Some(factor))
    };
    let (_, m) = measured(out, &s);
    [
        m.max_hausdorff_mm,
        m.max_correspondence_mm,
        m.max_corner_deviation_mm,
        m.max_start_deviation_mm,
        m.max_closure_gap_mm,
    ]
}

/// The rule: the smallest factor whose float values are all within
/// 0.0005 mm of the values at 80 on the runs as planned (the real start
/// heading is the assumed one), and within `MISALIGNED_TOLERANCE_MM` with
/// the first start entered at 90 deg to the assumed heading. The quality
/// report shows misaligned first starts too, and the tip swings fastest
/// there, so it is the value most sensitive to the step. Float mode only:
/// device mode has its own rounding floor, and its rounded moves make the
/// trace jump between steps.
/// Prints the table, with the real steps (`MIN_SAMPLE_STEP_MM`
/// raises any step below it).
fn sample_step_rule(cases: &[Case]) -> f64 {
    let mut diff = [0.0f64; 4];
    let mut misaligned = [0.0f64; 4];
    table_header(&[
        "case",
        "d",
        "real start heading",
        "f 10",
        "f 20",
        "f 40",
        "f 80 (ref)",
        "real steps 10 / 20 / 40 / 80",
    ]);
    for c in cases {
        for d in OFFSETS {
            let out = planned(c, &settings(d, Knob::pinned()));
            // As planned (+x), and the first path entered with the blade
            // at 90 deg to the assumed heading: the unknown start heading,
            // where the tip swings round fastest.
            for start in [0.0, 90.0] {
                let v: Vec<[f64; 5]> = FACTORS
                    .iter()
                    .map(|&f| step_values(&out, d, f, start))
                    .collect();
                let reference = v[3];
                let mut cells = vec![c.name.clone(), format!("{d}"), format!("{start}")];
                for (k, vk) in v.iter().enumerate() {
                    let dk = vk
                        .iter()
                        .zip(&reference)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0, f64::max);
                    if start == 0.0 {
                        diff[k] = diff[k].max(dk);
                    } else {
                        misaligned[k] = misaligned[k].max(dk);
                    }
                    cells.push(if k == 3 {
                        format!("hd {:.4}", reference[0])
                    } else {
                        format!("{dk:.5}")
                    });
                }
                let steps: Vec<String> = FACTORS
                    .iter()
                    .map(|f| format!("{:.4}", (d / f).max(flatcraft_sim::MIN_SAMPLE_STEP_MM)))
                    .collect();
                cells.push(steps.join(" / "));
                row(&cells);
            }
        }
    }
    println!();
    println!("largest change from factor 80, as planned (the rule): {diff:?}");
    println!("largest change from factor 80, misaligned first start: {misaligned:?}");
    FACTORS
        .iter()
        .zip(diff.iter().zip(misaligned))
        .find(|&(_, (&dk, mk))| dk <= 0.0005 && mk <= MISALIGNED_TOLERANCE_MM)
        .map(|(&f, _)| f)
        .expect("factor 80 is within 0 of itself")
}

#[test]
#[ignore = "full sweep; run with --ignored --nocapture"]
fn sample_step_converges() {
    let mut cases = sheet();
    cases.push(needle());
    assert_eq!(sample_step_rule(&cases), SAMPLE_STEP_FACTOR);
}

/// The fast twin: the cases where the sample step matters most (the
/// tightest circle, the sharpest star tip, the round hole's lead-in, and
/// the square, whose misaligned first start decides between 10 and 20).
#[test]
fn sample_step_factor_is_the_rule() {
    let cases: Vec<Case> = sheet()
        .into_iter()
        .filter(|c| {
            ["circle-r0.5", "star-15", "round-hole-10", "square-50"].contains(&c.name.as_str())
        })
        .collect();
    assert_eq!(cases.len(), 4);
    assert_eq!(
        sample_step_rule(&cases),
        SAMPLE_STEP_FACTOR,
        "the rule moved, not a bug in the simulator: at factor 20 the misaligned \
         square-50 start measured 0.00495 mm, only 0.00005 under the 0.005 mm \
         tolerance (Task 10), so a small planner or metric change can tip it to 40"
    );
}

// ------------------------------------------------------- pivot threshold

/// The pivot threshold candidates: the brief's 0, 0.5, 1, 2 and 10 deg,
/// and 5 deg, the step between 2 and 10.
const THRESHOLDS: [f64; 6] = [0.0, 0.5, 1.0, 2.0, 5.0, 10.0];

/// The share of segments a Quality threshold above 0 must save to become
/// the default (ruling 2026-11-26: 0.5 deg saves 4 of 4804, not worth a
/// change of the default).
const MIN_SEGMENT_SAVING: f64 = 0.01;

/// The pivot threshold rules over `cases`: (Quality, Fast). Prints the
/// table.
fn pivot_threshold_rule(cases: &[Case]) -> (f64, f64) {
    let mut float_dev = [0.0f64; 6];
    let mut device_dev = [0.0f64; 6];
    let mut segments = [0usize; 6];
    let mut skips = [0usize; 6];
    table_header(&[
        "case",
        "d",
        "threshold",
        "skipped pivots",
        "segments",
        "float max dev",
        "device max dev",
    ]);
    for c in cases {
        for d in OFFSETS {
            for (k, &t) in THRESHOLDS.iter().enumerate() {
                let knob = Knob {
                    threshold: t,
                    ..Knob::pinned()
                };
                let (out, _, fm) = run_mode(c, d, knob, Mode::Float);
                let (_, _, dm) = run_mode(c, d, knob, Mode::Device);
                let sk = skipped(&out, t);
                float_dev[k] = float_dev[k].max(deviation(&fm));
                device_dev[k] = device_dev[k].max(deviation(&dm));
                segments[k] += fm.segments;
                skips[k] += sk;
                row(&[
                    c.name.clone(),
                    format!("{d}"),
                    format!("{t}"),
                    sk.to_string(),
                    fm.segments.to_string(),
                    mm(deviation(&fm)),
                    mm(deviation(&dm)),
                ]);
            }
        }
    }
    table_header(&[
        "threshold",
        "skipped pivots",
        "segments",
        "float max dev",
        "device max dev",
    ]);
    for (k, t) in THRESHOLDS.iter().enumerate() {
        row(&[
            format!("{t}"),
            skips[k].to_string(),
            segments[k].to_string(),
            mm(float_dev[k]),
            mm(device_dev[k]),
        ]);
    }
    assert!(
        float_dev[0] <= ALGORITHM_LIMIT_MM,
        "STOP: an arc at every corner misses ALGORITHM_LIMIT_MM in float mode"
    );
    // Quality: the largest threshold within the float limit, but a default
    // only moves off 0 (an arc at every corner, the simpler rule) when it
    // saves at least MIN_SEGMENT_SAVING of the segments. A tie goes to 0.
    let quality = THRESHOLDS
        .iter()
        .enumerate()
        .filter(|&(k, _)| float_dev[k] <= ALGORITHM_LIMIT_MM)
        .filter(|&(k, _)| 1.0 - segments[k] as f64 / segments[0] as f64 >= MIN_SEGMENT_SAVING)
        .map(|(_, &t)| t)
        .fold(0.0, f64::max);
    // Fast: the largest candidate within the device limit that also keeps
    // the float deviation within the algorithm limit, 0 if none does.
    let fast = THRESHOLDS
        .iter()
        .enumerate()
        .filter(|&(k, _)| device_dev[k] <= DEVICE_LIMIT_MM && float_dev[k] <= ALGORITHM_LIMIT_MM)
        .map(|(_, &t)| t)
        .fold(0.0, f64::max);
    // A candidate the shapes never exercise proves nothing.
    for (k, &t) in THRESHOLDS.iter().enumerate().skip(1) {
        assert!(
            skips[k] > skips[k - 1],
            "threshold {t} skips no more pivots than the one below it"
        );
    }
    println!();
    println!("Quality: {quality}, Fast: {fast}");
    (quality, fast)
}

fn pinned_thresholds() -> (f64, f64) {
    (
        StrategySettings::for_strategy(CutStrategy::Quality).host_pivot_threshold_deg,
        StrategySettings::for_strategy(CutStrategy::Fast).host_pivot_threshold_deg,
    )
}

#[test]
#[ignore = "full sweep; run with --ignored --nocapture"]
fn pivot_threshold_choice() {
    let mut cases = sheet();
    cases.extend(threshold_probes());
    assert_eq!(pivot_threshold_rule(&cases), pinned_thresholds());
}

/// The fast twin: the probes decide the rule (the sheet's curves turn more
/// than 2 deg per vertex), plus the S-curve. Its segment total is smaller
/// than the sheet's, so the Quality saving filter is weaker here (1 deg
/// saves 1.7 % of these segments, 0.6 % of the sheet's): the twin checks
/// the deviation part of the Quality rule, the full sweep both parts.
#[test]
fn pivot_thresholds_are_the_rule() {
    let mut cases = threshold_probes();
    cases.extend(sheet().into_iter().filter(|c| c.name == "s-curve"));
    assert_eq!(pivot_threshold_rule(&cases), pinned_thresholds());
    let outlines =
        StrategySettings::for_strategy(CutStrategy::OutlinesLast).host_pivot_threshold_deg;
    assert_eq!(
        outlines,
        pinned_thresholds().0,
        "Outlines last cuts like Quality"
    );
}

// ------------------------------------------------------- arc step

const ARC_STEPS: [f64; 5] = [30.0, 20.0, 15.0, 10.0, 5.0];

/// The rule: the largest step with corner deviation (Task 3: the largest
/// correspondence deviation within 2d of a corner) and pivot drift within
/// `ALGORITHM_LIMIT_MM` on every case. Prints the table.
fn arc_step_rule(cases: &[Case]) -> f64 {
    let mut worst = [0.0f64; 5];
    table_header(&[
        "case", "d", "step 30", "step 20", "step 15", "step 10", "step 5",
    ]);
    for c in cases {
        for d in OFFSETS {
            let mut cells = vec![c.name.clone(), format!("{d}")];
            for (k, &step) in ARC_STEPS.iter().enumerate() {
                let knob = Knob {
                    arc_step: step,
                    ..Knob::pinned()
                };
                let out = planned(c, &settings(d, knob));
                let (trace, m) = measured(&out, &sim(d, Mode::Float, None));
                let drift = pivot_drift(&out.toolpaths, &trace, d);
                worst[k] = worst[k].max(m.max_corner_deviation_mm).max(drift);
                cells.push(format!(
                    "{} / {} ({})",
                    mm(m.max_corner_deviation_mm),
                    mm(drift),
                    m.segments
                ));
            }
            row(&cells);
        }
    }
    println!();
    println!("worst of corner deviation and pivot drift per step: {worst:?}");
    ARC_STEPS
        .iter()
        .zip(worst)
        .find(|&(_, v)| v <= ALGORITHM_LIMIT_MM)
        .map(|(&s, _)| s)
        .expect("STOP: no arc step meets ALGORITHM_LIMIT_MM")
}

#[test]
#[ignore = "full sweep; run with --ignored --nocapture"]
fn arc_step_choice() {
    let mut cases = star_cases();
    cases.push(needle());
    cases.push(hairpin());
    cases.push(case(
        "square-50",
        vec![placed(1, rect_path(10.0, 10.0, 50.0, 50.0))],
    ));
    assert_eq!(arc_step_rule(&cases), ARC_STEP_DEG);
}

/// The fast twin: the sharpest turns decide it.
#[test]
fn arc_step_is_the_rule() {
    let cases = vec![star_cases().remove(0), needle(), hairpin()];
    assert_eq!(arc_step_rule(&cases), ARC_STEP_DEG);
}

// ------------------------------------------------------- alignment

/// How a run starts (as in `tests/knife.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Branch {
    None,
    Swivel,
    LeadIn,
    HoleArc,
}

/// The vertex P0 the first path move of run `r` starts from, its tangent
/// t0 and the index of `P0 + d * t0` in the run.
fn run_start(t: &Toolpath, r: usize, d: f64) -> (Point, Vec2, usize) {
    let run = &t.axis.as_ref().unwrap()[r];
    let first = run.seg.iter().position(|&s| s != WASTE_SEG).unwrap();
    let j = run.seg[first] as usize;
    let (p0, t0) = (t.points[j], (t.points[j + 1] - t.points[j]).normalize());
    let target = p0 + d * t0;
    let at = run.points[..first]
        .iter()
        .position(|q| q.distance(target) < 1e-9)
        .unwrap();
    (p0, t0, at)
}

fn branch(t: &Toolpath, r: usize, d: f64) -> Branch {
    let (p0, t0, at) = run_start(t, r, d);
    let run = &t.axis.as_ref().unwrap()[r];
    if at == 0 {
        return Branch::None;
    }
    if run.points[..=at]
        .iter()
        .all(|q| (q.distance(p0) - d).abs() < 1e-9)
    {
        return Branch::Swivel;
    }
    let last = run.points[at] - run.points[at - 1];
    if last.cross(t0).abs() < 1e-9 && last.dot(t0) >= 2.0 * d - 1e-9 {
        Branch::LeadIn
    } else {
        Branch::HoleArc
    }
}

/// The axis length of run `r` before it reaches `P0 + d * t0`.
fn lead_length(t: &Toolpath, r: usize, d: f64) -> f64 {
    let (_, _, at) = run_start(t, r, d);
    let run = &t.axis.as_ref().unwrap()[r];
    run.points[..=at]
        .windows(2)
        .map(|w| w[0].distance(w[1]))
        .sum()
}

/// A job built without the order, so the start turn is exactly what the
/// sweep asks for: `ring` (closed, oriented counter-clockwise, starting at
/// its first point) with the default overcut, kept as `keep`, through
/// `knife::apply` with the blade coming in at heading `h`.
struct Direct {
    toolpaths: Vec<Toolpath>,
    kept: KeptMaterial,
    h: Vec2,
}

fn direct(rings_in: &[(Vec<Point>, KeepSide)], mode: KeepMode, h: Vec2, d: f64, k: Knob) -> Direct {
    let paths: Vec<Toolpath> = rings_in
        .iter()
        .map(|(ring, keep)| {
            let mut lap = rings::orient(ring, RingDirection::Ccw);
            lap.push(lap[0]);
            let mut t = Toolpath::new(lap, true, source(1));
            t.keep = *keep;
            t
        })
        .collect();
    let mut paths = passes::chain(paths, 1, false, DEFAULT_OVERCUT_MM);
    let kept = KeptMaterial::from_toolpaths(&paths, |_| mode, area());
    let s = PipelineSettings {
        compensation: k.host(d),
        start_heading: h,
        ..PipelineSettings::with_defaults(area(), HOME)
    };
    knife::apply(&mut paths, &s, &kept, h);
    Direct {
        toolpaths: paths,
        kept,
        h,
    }
}

fn direct_measured(j: &Direct, s: &SimSettings) -> (SimTrace, JobMetrics) {
    let s = SimSettings {
        start_heading: j.h,
        ..*s
    };
    let trace = simulate(&j.toolpaths, HOME, &s).unwrap();
    let (m, _) = measure_kept(&j.toolpaths, &trace, &s, &j.kept);
    (trace, m)
}

/// The square's corners from (100, 100), 20 mm, starting at its first
/// corner or in the middle of its first edge.
fn square(mid_edge: bool) -> Vec<Point> {
    let mut sq = vec![
        p(100.0, 100.0),
        p(120.0, 100.0),
        p(120.0, 120.0),
        p(100.0, 120.0),
    ];
    sq = rings::orient(&sq, RingDirection::Ccw);
    if mid_edge {
        let m = sq[0].midpoint(sq[1]);
        sq.insert(1, m);
        sq = rings::rotate_start(&sq, 1);
    }
    sq
}

/// A round ring of radius `r` around `c`, 180 vertices.
fn round(c: Point, r: f64) -> Vec<Point> {
    (0..180)
        .map(|k| c + r * Vec2::from_angle(std::f64::consts::TAU * k as f64 / 180.0))
        .collect()
}

/// The alignment layouts: a piece entered at a corner and mid-edge, a
/// stencil (kept outside), a Both outline, and a round hole in a piece.
/// Rings with their kept side.
type Rings = Vec<(Vec<Point>, KeepSide)>;

fn layouts() -> Vec<(&'static str, Rings, KeepMode)> {
    let c = p(150.0, 150.0);
    let frame = vec![
        p(130.0, 130.0),
        p(170.0, 130.0),
        p(170.0, 170.0),
        p(130.0, 170.0),
    ];
    vec![
        (
            "piece, corner",
            vec![(square(false), KeepSide::Inside)],
            KeepMode::Pieces,
        ),
        (
            "piece, mid-edge",
            vec![(square(true), KeepSide::Inside)],
            KeepMode::Pieces,
        ),
        (
            "stencil, corner",
            vec![(square(false), KeepSide::Outside)],
            KeepMode::Stencil,
        ),
        (
            "stencil, mid-edge",
            vec![(square(true), KeepSide::Outside)],
            KeepMode::Stencil,
        ),
        (
            "both, mid-edge",
            vec![(square(true), KeepSide::Both)],
            KeepMode::Both,
        ),
        (
            "round hole 10",
            vec![
                (round(c, 5.0), KeepSide::Outside),
                (frame, KeepSide::Inside),
            ],
            KeepMode::Pieces,
        ),
    ]
}

/// The start turns of the check: 0 to 165 deg in 15 deg steps, and 179.
fn start_turns() -> Vec<f64> {
    let mut v: Vec<f64> = (0..12).map(|k| 15.0 * k as f64).collect();
    v.push(179.0);
    v
}

#[test]
fn alignment_check() {
    table_header(&[
        "layout",
        "d",
        "turn",
        "branches",
        "lead length",
        "waste",
        "start dev",
        "pushed",
        "into kept",
    ]);
    let mut worst = 0.0f64;
    for (name, rs, mode) in layouts() {
        for d in OFFSETS {
            for turn in start_turns() {
                let mut cells_branch = Vec::new();
                let (mut lead, mut waste, mut dev) = (0.0f64, 0.0f64, 0.0f64);
                for sign in [1.0, -1.0] {
                    // The first tangent of the oriented ring.
                    let t0 = (rings::orient(&rs[0].0, RingDirection::Ccw)[1]
                        - rings::orient(&rs[0].0, RingDirection::Ccw)[0])
                        .normalize();
                    let h = rotate(t0, -sign * turn.to_radians());
                    let j = direct(&rs, mode, h, d, Knob::pinned());
                    let (_, m) = direct_measured(&j, &sim(d, Mode::Float, None));
                    let b = branch(&j.toolpaths[0], 0, d);
                    cells_branch.push(format!("{b:?}"));
                    lead = lead.max(lead_length(&j.toolpaths[0], 0, d));
                    waste = waste.max(m.waste_mm);
                    let sd = m.per_path[0].max_start_deviation_mm;
                    dev = dev.max(sd);
                    let case = format!("{name} d {d} turn {turn} sign {sign}");
                    assert!(sd <= ALGORITHM_LIMIT_MM, "{case}: start deviation {sd}");
                    assert_eq!(m.pushed, 0, "{case}");
                    assert_eq!(m.max_into_kept_mm, 0.0, "{case}");
                }
                worst = worst.max(dev);
                row(&[
                    name.to_string(),
                    format!("{d}"),
                    format!("{turn}"),
                    cells_branch.join(" / "),
                    mm(lead),
                    mm(waste),
                    mm(dev),
                    "0".into(),
                    "0".into(),
                ]);
            }
        }
    }
    println!();
    println!("worst start deviation: {worst:.5}");
}

fn rotate(v: Vec2, a: f64) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// Report only: how the swivel cut-off changes the starts of a piece
/// entered at a waste-side corner. The model cannot rate a swivel in
/// material against a lead-in, so 90 deg stays provisional (M3 test cuts).
#[test]
fn swivel_max_report() {
    table_header(&[
        "swivel max",
        "d",
        "swivels",
        "lead-ins",
        "hole arcs",
        "lead length (max)",
        "waste (sum)",
        "start dev (max)",
        "twist (max deg)",
    ]);
    for sm in [45.0, 60.0, 90.0, 120.0] {
        for d in OFFSETS {
            let (mut sw, mut li, mut ha) = (0, 0, 0);
            let (mut lead, mut waste, mut dev, mut twist) = (0.0f64, 0.0, 0.0f64, 0.0f64);
            for turn in start_turns() {
                for sign in [1.0, -1.0] {
                    let rs = vec![(square(false), KeepSide::Inside)];
                    let t0 = (rs[0].0[1] - rs[0].0[0]).normalize();
                    let h = rotate(t0, -sign * turn.to_radians());
                    let knob = Knob {
                        swivel_max: sm,
                        ..Knob::pinned()
                    };
                    let j = direct(&rs, KeepMode::Pieces, h, d, knob);
                    let (_, m) = direct_measured(&j, &sim(d, Mode::Float, None));
                    match branch(&j.toolpaths[0], 0, d) {
                        Branch::Swivel => sw += 1,
                        Branch::LeadIn => li += 1,
                        Branch::HoleArc => ha += 1,
                        Branch::None => {}
                    }
                    lead = lead.max(lead_length(&j.toolpaths[0], 0, d));
                    waste += m.waste_mm;
                    dev = dev.max(m.max_start_deviation_mm);
                    twist = twist.max(m.max_twist_deg);
                    assert_eq!(m.pushed, 0);
                    assert_eq!(m.max_into_kept_mm, 0.0);
                }
            }
            row(&[
                format!("{sm}"),
                format!("{d}"),
                sw.to_string(),
                li.to_string(),
                ha.to_string(),
                mm(lead),
                format!("{waste:.2}"),
                mm(dev),
                format!("{twist:.1}"),
            ]);
        }
    }
}

// ------------------------------------------------------- hole lead-in

const HOLE_RADII: [f64; 4] = [0.5, 1.0, 2.0, 3.0];

/// The rule: the smallest radius of the arc lead-in with start
/// deviation within `ALGORITHM_LIMIT_MM`, nothing pushed and nothing into
/// kept material, on round holes of radius `holes` entered at 12 headings,
/// both ways round. Every start must take the arc lead-in. Prints the table.
fn hole_lead_in_rule(holes: &[f64]) -> f64 {
    table_header(&[
        "radius",
        "hole r",
        "d",
        "arc lead-ins",
        "start dev (max)",
        "pushed",
        "into kept (max)",
        "waste (max)",
    ]);
    let mut ok = [true; 4];
    for (k, &rho) in HOLE_RADII.iter().enumerate() {
        for &hr in holes {
            for d in OFFSETS {
                let (mut dev, mut pushed, mut into, mut waste) = (0.0f64, 0, 0.0f64, 0.0f64);
                let (mut arcs, mut starts) = (0, 0);
                for deg in (0..12).map(|i| 15.0 + 30.0 * i as f64) {
                    for positive in [true, false] {
                        let c = p(150.0, 150.0);
                        let mut hole = round(c, hr);
                        if !positive {
                            hole.reverse();
                        }
                        let s = hr + 10.0;
                        let frame = vec![
                            p(c.x - s, c.y - s),
                            p(c.x + s, c.y - s),
                            p(c.x + s, c.y + s),
                            p(c.x - s, c.y + s),
                        ];
                        let knob = Knob {
                            hole_radius: rho,
                            ..Knob::pinned()
                        };
                        let rs = vec![(hole, KeepSide::Outside), (frame, KeepSide::Inside)];
                        let j = direct(&rs, KeepMode::Pieces, heading(deg), d, knob);
                        let t = &j.toolpaths[0];
                        let (_, m) = direct_measured(&j, &sim(d, Mode::Float, None));
                        starts += 1;
                        if branch(t, 0, d) == Branch::HoleArc {
                            arcs += 1;
                        }
                        dev = dev.max(m.per_path[0].max_start_deviation_mm);
                        pushed += m.pushed;
                        into = into.max(m.max_into_kept_mm);
                        waste = waste.max(m.per_path[0].waste_mm);
                    }
                }
                ok[k] &= dev <= ALGORITHM_LIMIT_MM && pushed == 0 && into == 0.0 && arcs == starts;
                row(&[
                    format!("{rho}"),
                    format!("{hr}"),
                    format!("{d}"),
                    format!("{arcs} of {starts}"),
                    mm(dev),
                    pushed.to_string(),
                    mm(into),
                    mm(waste),
                ]);
            }
        }
    }
    HOLE_RADII
        .iter()
        .zip(ok)
        .find(|&(_, o)| o)
        .map(|(&r, _)| r)
        .expect("STOP: no lead-in radius meets the rule")
}

#[test]
#[ignore = "full sweep; run with --ignored --nocapture"]
fn hole_lead_in_choice() {
    assert_eq!(
        hole_lead_in_rule(&[1.0, 2.0, 5.0, 10.0]),
        HOLE_LEAD_IN_RADIUS_MM
    );
}

/// The fast twin: the 10 mm round hole of the sheet.
#[test]
fn hole_lead_in_radius_is_the_rule() {
    assert_eq!(hole_lead_in_rule(&[5.0]), HOLE_LEAD_IN_RADIUS_MM);
}

// ------------------------------------------------------- device limit

#[test]
fn quantized_output_meets_device_limit() {
    table_header(&[
        "case",
        "d",
        "float max dev",
        "device max dev, plain rounding",
        "pushed, plain rounding",
        "device max dev",
        "device hausdorff / correspondence",
        "device corner dev",
        "device start dev",
        "pushed",
        "into kept",
        "worst spot",
        "calibrated: device max dev",
        "calibrated: pushed",
    ]);
    let mut cases = sheet();
    cases.push(whole_sheet());
    cases.extend(threshold_probes());
    let mut worst = 0.0f64;
    let (mut pushed, mut over_device, mut over_float) = (Vec::new(), Vec::new(), Vec::new());
    let mut into_kept = Vec::new();
    for c in &cases {
        for d in OFFSETS {
            // The float plan, rounded only by the simulator: what plain
            // rounding of the exact runs would send.
            let (out, _, fm) = run_mode(c, d, Knob::pinned(), Mode::Float);
            let (_, plain) = measured(&out, &sim(d, Mode::Device, None));
            // The plan for the device units: rounding-aware runs.
            let (_, _, dm) = run_mode(c, d, Knob::pinned(), Mode::Device);
            // The same on a calibrated, mirrored plotter with a grid origin.
            let cal = Knob {
                grid: Some(calibrated_grid()),
                ..Knob::pinned()
            };
            let out_cal = planned(c, &settings(d, cal));
            let (_, cm) = measured(&out_cal, &SimSettings::device(d, calibrated_grid()));
            worst = worst.max(deviation(&dm)).max(deviation(&cm));
            row(&[
                c.name.clone(),
                format!("{d}"),
                mm(deviation(&fm)),
                mm(deviation(&plain)),
                plain.pushed.to_string(),
                mm(deviation(&dm)),
                format!(
                    "{} / {}",
                    mm(dm.max_hausdorff_mm),
                    mm(dm.max_correspondence_mm)
                ),
                mm(dm.max_corner_deviation_mm),
                mm(dm.max_start_deviation_mm),
                dm.pushed.to_string(),
                mm(dm.max_into_kept_mm),
                dm.worst_spots.first().map_or(String::new(), |w| {
                    format!("{:?} {:.1},{:.1}", w.kind, w.at.x, w.at.y)
                }),
                mm(deviation(&cm)),
                cm.pushed.to_string(),
            ]);
            if deviation(&cm) > DEVICE_LIMIT_MM {
                over_device.push(format!("{} d {d} calibrated: {}", c.name, deviation(&cm)));
            }
            if cm.pushed > 0 {
                pushed.push(format!("{} d {d} calibrated: {}", c.name, cm.pushed));
            }
            if deviation(&dm) > DEVICE_LIMIT_MM {
                over_device.push(format!("{} d {d}: {}", c.name, deviation(&dm)));
            }
            if deviation(&fm) > ALGORITHM_LIMIT_MM {
                over_float.push(format!("{} d {d}: {}", c.name, deviation(&fm)));
            }
            for (label, m) in [("float", &fm), ("device", &dm), ("calibrated", &cm)] {
                if m.max_into_kept_mm > 0.0 {
                    into_kept.push(format!("{} d {d} {label}: {}", c.name, m.max_into_kept_mm));
                }
            }
            if dm.pushed > 0 || fm.pushed > 0 {
                pushed.push(format!("{} d {d}: {}", c.name, dm.pushed));
            }
        }
    }
    println!();
    println!("worst device deviation: {worst:.5}");
    assert!(
        over_device.is_empty(),
        "STOP: over DEVICE_LIMIT_MM: {over_device:?}"
    );
    assert!(
        over_float.is_empty(),
        "over ALGORITHM_LIMIT_MM: {over_float:?}"
    );
    assert!(pushed.is_empty(), "pushed in device mode: {pushed:?}");
    assert!(
        into_kept.is_empty(),
        "STOP: waste moves into kept material: {into_kept:?}"
    );
}

// ------------------------------------------------------- start heading

/// The whole sheet as one job (each shape its own object).
fn whole_sheet() -> Case {
    let mut input = Vec::new();
    for (k, c) in sheet().into_iter().enumerate() {
        for mut pp in c.input {
            pp.source.object = ObjectId(k as u64 + 1);
            input.push(pp);
        }
    }
    case("sheet", input)
}

/// Eight headings, none along +x (the planner's assumption) and none
/// opposite the alignment stroke (+x): the pursuit model has no answer to
/// an exact reversal.
fn other_headings() -> Vec<f64> {
    (0..8).map(|k| 22.5 + 45.0 * k as f64).collect()
}

#[test]
fn unknown_start_heading_only_affects_the_first_path() {
    let job = whole_sheet();
    table_header(&[
        "d",
        "real start heading",
        "first path max dev",
        "first path start dev",
        "first path pushed",
        "later paths max dev",
        "with stroke: cuts max dev",
        "stroke pushed",
    ]);
    for d in OFFSETS {
        let out = planned(&job, &settings(d, Knob::pinned()));
        let stroke = PipelineSettings {
            alignment_stroke: Some(AlignmentStroke {
                start: p(270.0, 280.0),
                direction: Vec2::new(1.0, 0.0),
                length_mm: 10.0,
            }),
            ..settings(d, Knob::pinned())
        };
        let with_stroke = planned(&job, &stroke);
        assert_eq!(with_stroke.toolpaths[0].kind, ToolpathKind::Alignment);
        let line = &with_stroke.toolpaths[0].points;
        for k in 0..=100 {
            let q = line[0].lerp(line[1], k as f64 / 100.0);
            assert!(
                !with_stroke.kept.is_kept(q),
                "the stroke crosses kept material at {q:?}"
            );
        }
        for deg in other_headings() {
            let s = SimSettings {
                start_heading: heading(deg),
                ..sim(d, Mode::Float, None)
            };
            let (_, m) = measured(&out, &s);
            let per = &m.per_path;
            let first = per[0].hausdorff_mm.max(per[0].max_correspondence_mm);
            let later = per[1..]
                .iter()
                .map(|q| q.hausdorff_mm.max(q.max_correspondence_mm))
                .fold(0.0, f64::max);
            assert!(later <= ALGORITHM_LIMIT_MM, "d {d} heading {deg}: {later}");
            let (_, ms) = measured(&with_stroke, &s);
            let all = ms
                .per_path
                .iter()
                .filter(|q| with_stroke.toolpaths[q.toolpath].kind == ToolpathKind::Cut)
                .map(|q| q.hausdorff_mm.max(q.max_correspondence_mm))
                .fold(0.0, f64::max);
            assert!(
                all <= ALGORITHM_LIMIT_MM,
                "stroke, d {d} heading {deg}: {all}"
            );
            // The stroke may be pushed, since turning the blade is its
            // job, but only because it lies entirely in waste: nothing of
            // the job, the stroke included, touches kept material. The
            // cuts after it are never pushed.
            assert_eq!(ms.max_into_kept_mm, 0.0, "stroke, d {d} heading {deg}");
            let cut_pushed: usize = ms
                .per_path
                .iter()
                .filter(|q| with_stroke.toolpaths[q.toolpath].kind == ToolpathKind::Cut)
                .map(|q| q.pushed)
                .sum();
            assert_eq!(cut_pushed, 0, "stroke, d {d} heading {deg}");
            let stroke_pushed = ms.per_path[0].pushed;
            row(&[
                format!("{d}"),
                format!("{deg}"),
                mm(first),
                mm(per[0].max_start_deviation_mm),
                per[0].pushed.to_string(),
                mm(later),
                mm(all),
                stroke_pushed.to_string(),
            ]);
        }
    }
}

// ------------------------------------------------------- reports

/// Report only (R4): the job planned for d, cut by a blade of 0.9 d and
/// 1.1 d.
#[test]
fn offset_sensitivity_report() {
    table_header(&[
        "case",
        "assumed d",
        "real d",
        "max dev",
        "corner dev",
        "start dev",
        "pushed",
        "into kept",
    ]);
    let mut cases = sheet();
    cases.retain(|c| {
        [
            "square-50",
            "hole-in-square",
            "round-hole-10",
            "s-curve",
            "circle-r2",
            "star-15",
            "star-90",
        ]
        .contains(&c.name.as_str())
    });
    for c in &cases {
        for d in OFFSETS {
            let out = planned(c, &settings(d, Knob::pinned()));
            for f in [0.9, 1.0, 1.1] {
                let real = d * f;
                let (_, m) = measured(&out, &sim(real, Mode::Float, None));
                row(&[
                    c.name.clone(),
                    format!("{d}"),
                    format!("{real:.3}"),
                    mm(deviation(&m)),
                    mm(m.max_corner_deviation_mm),
                    mm(m.max_start_deviation_mm),
                    m.pushed.to_string(),
                    mm(m.max_into_kept_mm),
                ]);
            }
        }
    }
}

/// Report only: the closure gap of an uncompensated ring (tip mode: the
/// commanded path as the tip path, no firmware model) against the overcut,
/// next to the tractrix value `d * sin(2 * atan(exp(-o / d)))`. It does not
/// model FC18, so it is no measured reason to change the default overcut.
#[test]
fn overcut_closure_report() {
    table_header(&[
        "case",
        "d",
        "overcut",
        "closure gap",
        "closure gap, blade aligned at start",
        "tractrix",
    ]);
    for c in sheet()
        .into_iter()
        .filter(|c| ["square-50", "star-90"].contains(&c.name.as_str()))
    {
        for d in OFFSETS {
            for o in [0.0, 0.5, 0.75, 1.0, 1.5] {
                let s = PipelineSettings {
                    overcut_mm: o,
                    ..PipelineSettings::with_defaults(area(), HOME)
                };
                let out = planned(&c, &s);
                let (_, m) = measured(&out, &sim(d, Mode::Float, None));
                // The same ring with the blade already along its first
                // segment: no start misalignment.
                let t = &out.toolpaths[0];
                let aligned = SimSettings {
                    start_heading: (t.points[1] - t.points[0]).normalize(),
                    ..sim(d, Mode::Float, None)
                };
                let (_, ma) = measured(&out, &aligned);
                let tractrix = d * (2.0 * (-o / d).exp().atan()).sin();
                row(&[
                    c.name.clone(),
                    format!("{d}"),
                    format!("{o}"),
                    mm(m.max_closure_gap_mm),
                    mm(ma.max_closure_gap_mm),
                    mm(tractrix),
                ]);
            }
        }
    }
}

/// Report only: the device deviation of the sheet at finer device units,
/// as if `D` took decimals (research open question), planned for those
/// units. 20 units per mm is the Cameo 2 today.
#[test]
fn device_resolution_report() {
    table_header(&[
        "units per mm",
        "d",
        "device max dev (worst case)",
        "worst case",
        "pushed (sum)",
    ]);
    let mut cases = sheet();
    cases.extend(threshold_probes());
    for upm in [20.0, 40.0, 100.0, 200.0] {
        for d in OFFSETS {
            let (mut worst, mut at, mut pushed) = (0.0f64, String::new(), 0);
            for c in &cases {
                let g = DeviceGrid::new(upm, Point::ZERO);
                let knob = Knob {
                    grid: Some(g),
                    ..Knob::pinned()
                };
                let out = planned(c, &settings(d, knob));
                let (_, m) = measured(&out, &SimSettings::device(d, g));
                pushed += m.pushed;
                if deviation(&m) > worst {
                    worst = deviation(&m);
                    at = c.name.clone();
                }
            }
            row(&[
                format!("{upm}"),
                format!("{d}"),
                mm(worst),
                at,
                pushed.to_string(),
            ]);
        }
    }
}

// The knife stage's drop bound stays within the device limit.
const _: () = assert!(flatcraft_path::quantize::MAX_DROP_MM <= DEVICE_LIMIT_MM);

/// The points the knife stage rounds go through the emitter's transform
/// unchanged: on a calibrated (scale 1.002 and 0.998), mirrored plotter
/// with a grid origin, every axis point is already on a device unit, so
/// the emitter rounds it to itself. The device simulation of that job
/// pushes nothing.
#[test]
fn rounded_points_go_through_the_emitter_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let store = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
    for mirror in [false, true] {
        let mut plotter = store.plotter("file-output-gpgl-cameo2").unwrap();
        plotter.scale_x = 1.002;
        plotter.scale_y = 0.998;
        plotter.axis.mirror_x = mirror;
        let mut media = plotter.media[0].clone();
        media.grid.origin_x_mm = 0.37;
        media.grid.origin_y_mm = 0.013;
        let g = flatcraft_device::gpgl::quantize::device_grid(&plotter, &media);
        for d in OFFSETS {
            let knob = Knob {
                grid: Some(g),
                ..Knob::pinned()
            };
            let out = planned(&whole_sheet(), &settings(d, knob));
            let mut points = 0;
            for t in &out.toolpaths {
                for q in t.runs().iter().flat_map(|r| r.points.clone()) {
                    let dev = flatcraft_device::gpgl::quantize::to_device(q, &plotter, &media);
                    let back = g.from_units(Point::new(f64::from(dev.x), f64::from(dev.y)));
                    assert!(
                        back.distance(q) < 1e-9,
                        "mirror {mirror}: {q:?} goes to {back:?}"
                    );
                    let u = g.to_units(q);
                    assert!((u.x - u.x.round()).abs() < 1e-6 && (u.y - u.y.round()).abs() < 1e-6);
                    points += 1;
                }
            }
            assert!(points > 1000);
            let (_, m) = measured(&out, &SimSettings::device(d, g));
            assert_eq!(m.pushed, 0, "mirror {mirror} d {d}");
            assert!(
                deviation(&m) <= DEVICE_LIMIT_MM,
                "mirror {mirror} d {d}: {}",
                deviation(&m)
            );
        }
    }
}

/// Host mode sends exactly the rounding-aware knife output: the test sheet,
/// planned with host compensation on the device grid of a calibrated,
/// mirrored plotter with a grid origin, emitted as host axis paths (one path
/// per run). The `M` and `D` coordinates are the planned device-grid points
/// (in order, a point equal to the one before it merged), so the emitter adds
/// no second rounding that could bring back a blade push; the device
/// simulation of those runs pushes nothing.
#[test]
fn emitted_host_job_is_the_planned_device_grid() {
    use flatcraft_device::KnifeCompensation;
    use flatcraft_device::gpgl::{CutSettings, EmitPath, GeometryKind, decode, emit_job};
    let dir = tempfile::tempdir().unwrap();
    let store = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
    for mirror in [false, true] {
        for d in OFFSETS {
            let mut plotter = store.plotter("file-output-gpgl-cameo2").unwrap();
            plotter.scale_x = 1.002;
            plotter.scale_y = 0.998;
            plotter.axis.mirror_x = mirror;
            plotter.media[0].grid.origin_x_mm = 0.37;
            plotter.media[0].grid.origin_y_mm = 0.013;
            plotter.knife_compensation = KnifeCompensation::Host {
                offset_mm: d,
                pivot_threshold_deg: 0.0,
            };
            let media = plotter.media[0].clone();
            let g = flatcraft_device::gpgl::quantize::device_grid(&plotter, &media);
            let knob = Knob {
                grid: Some(g),
                ..Knob::pinned()
            };
            let out = planned(&whole_sheet(), &settings(d, knob));
            let runs: Vec<Vec<Point>> = out
                .toolpaths
                .iter()
                .flat_map(|t| t.runs())
                .map(|r| r.points)
                .collect();
            let paths: Vec<EmitPath> = runs
                .iter()
                .map(|points| EmitPath {
                    points: points.clone(),
                    note: "run".into(),
                })
                .collect();
            let job = emit_job(
                &paths,
                GeometryKind::HostAxis { offset_mm: d },
                &plotter,
                &media.id,
                &CutSettings {
                    speed: 3,
                    force: 10,
                    track_enhancing: false,
                },
            )
            .unwrap();
            let mut want: Vec<Vec<(i64, i64)>> = Vec::new();
            for points in &runs {
                let mut units: Vec<(i64, i64)> = points
                    .iter()
                    .map(|&q| {
                        let u = g.to_units(q);
                        let r = g.rounded_units(q);
                        assert!(
                            (u.x - r.x).abs() < 1e-6 && (u.y - r.y).abs() < 1e-6,
                            "mirror {mirror} d {d}: {q:?} is not on the device grid"
                        );
                        (r.y as i64, r.x as i64)
                    })
                    .collect();
                units.dedup();
                if units.len() >= 2 {
                    want.push(units);
                }
            }
            let sent: Vec<Vec<(i64, i64)>> = {
                let mut paths: Vec<Vec<(i64, i64)>> = Vec::new();
                for c in decode::decode(&job.bytes) {
                    let (head, rest) = c.raw.split_at(1);
                    if head != "M" && head != "D" {
                        continue;
                    }
                    let n: Vec<i64> = rest.split(',').map(|t| t.parse().unwrap()).collect();
                    let pts = n.chunks(2).map(|p| (p[0], p[1]));
                    if head == "M" {
                        paths.push(pts.collect());
                    } else {
                        paths.last_mut().unwrap().extend(pts);
                    }
                }
                paths
            };
            assert_eq!(sent.len(), want.len(), "mirror {mirror} d {d}");
            assert_eq!(sent, want, "mirror {mirror} d {d}");
            let (_, m) = measured(&out, &SimSettings::device(d, g));
            assert_eq!(m.pushed, 0, "mirror {mirror} d {d}");
        }
    }
}

/// The distance of `q` from the source geometry of `c` (before flattening).
fn source_distance(c: &Case, q: Point) -> f64 {
    use kurbo::ParamCurveNearest as _;
    c.input
        .iter()
        .flat_map(|pp| pp.path.segments())
        .map(|seg| seg.nearest(q, 1e-9).distance_sq.sqrt())
        .fold(f64::INFINITY, f64::min)
}

/// The 16 grid positions a quarter unit apart, plain and on a calibrated
/// (scale 1.002 and 0.998), mirrored plotter: where the grid lies under
/// the shapes changes how the pivot arcs round.
fn grid_positions() -> Vec<DeviceGrid> {
    let mut out = Vec::new();
    for (sx, sy, mx) in [(1.0, 1.0, false), (1.002, 0.998, true)] {
        for ox in [0.0, 0.0125, 0.025, 0.0375] {
            for oy in [0.0, 0.0125, 0.025, 0.0375] {
                out.push(DeviceGrid {
                    units_per_mm: 20.0,
                    origin: Point::new(ox, oy),
                    media_size: kurbo::Size::new(304.8, 330.2),
                    mirror_x: mx,
                    mirror_y: false,
                    scale: Vec2::new(sx, sy),
                });
            }
        }
    }
    out
}

/// The cut moves of `trace`: each move into a blade-down sample that cuts
/// a `Cut` toolpath (not waste), as tip segments.
fn cut_moves(out: &CutPlan, trace: &SimTrace) -> Vec<(Point, Point)> {
    trace
        .samples
        .windows(2)
        .filter(|w| {
            let x = &w[1];
            x.down
                && w[0].down
                && x.seg != WASTE_SEG
                && out.toolpaths[x.toolpath].kind == ToolpathKind::Cut
        })
        .map(|w| (w[0].tip, w[1].tip))
        .collect()
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + t * ab)
}

/// The cell size of [`CutIndex`]: a query looks one cell round its own, so
/// any cut within this distance is found.
const CELL_MM: f64 = 0.1;

/// The cut moves in square cells of [`CELL_MM`], to find the cut near a
/// point.
struct CutIndex {
    cells: std::collections::HashMap<(i64, i64), Vec<usize>>,
    moves: Vec<(Point, Point)>,
}

impl CutIndex {
    fn new(moves: Vec<(Point, Point)>) -> Self {
        let cell = |v: f64| (v / CELL_MM).floor() as i64;
        let mut cells: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (k, &(a, b)) in moves.iter().enumerate() {
            for x in cell(a.x.min(b.x))..=cell(a.x.max(b.x)) {
                for y in cell(a.y.min(b.y))..=cell(a.y.max(b.y)) {
                    cells.entry((x, y)).or_default().push(k);
                }
            }
        }
        Self { cells, moves }
    }

    /// The distance from `p` to the nearest cut move, or infinity when no
    /// cut lies within [`CELL_MM`].
    fn distance(&self, p: Point) -> f64 {
        let (cx, cy) = (
            (p.x / CELL_MM).floor() as i64,
            (p.y / CELL_MM).floor() as i64,
        );
        let mut best = f64::INFINITY;
        for x in cx - 1..=cx + 1 {
            for y in cy - 1..=cy + 1 {
                for &k in self.cells.get(&(x, y)).into_iter().flatten() {
                    let (a, b) = self.moves[k];
                    best = best.min(segment_distance(p, a, b));
                }
            }
        }
        if best <= CELL_MM { best } else { f64::INFINITY }
    }
}

/// The end-to-end deviation of a simulated job from the source geometry of
/// `c`, both ways (Hausdorff): the largest distance of a cut tip from the
/// source shapes (an over-cut), and the largest distance of a point of the
/// source shapes, sampled every 0.01 mm, from the cut (an under-cut, such
/// as a corner the tip never reaches). Returns (cut to source, source to
/// cut).
fn end_to_end(c: &Case, out: &CutPlan, trace: &SimTrace) -> (f64, f64) {
    use kurbo::ParamCurveArclen as _;
    let moves = cut_moves(out, trace);
    let cut_to_source = moves
        .iter()
        .map(|&(_, b)| source_distance(c, b))
        .fold(0.0, f64::max);
    let index = CutIndex::new(moves);
    let mut source_to_cut = 0.0f64;
    for seg in c.input.iter().flat_map(|pp| pp.path.segments()) {
        let n = (seg.arclen(1e-6) / 0.01).ceil().max(1.0) as usize;
        for k in 0..=n {
            let q = kurbo::ParamCurve::eval(&seg, k as f64 / n as f64);
            source_to_cut = source_to_cut.max(index.distance(q));
        }
    }
    (cut_to_source, source_to_cut)
}

/// The device output on `grids`, for the sheet and the probes at d 0.25
/// and 0.3, planned for each grid. Per grid: the device deviation against
/// the planned path, the end-to-end deviation against the source shapes
/// (both ways, see [`end_to_end`])
/// and the pushed blades. Asserts, collected over all grids: end to end
/// within `END_TO_END_LIMIT_MM` (the guarantee that counts), the device
/// deviation within `DEVICE_LIMIT_MM`, and nothing pushed.
fn grid_position_rule(grids: &[DeviceGrid]) {
    table_header(&[
        "scale",
        "mirror x",
        "grid origin",
        "device max dev",
        "worst case",
        "end-to-end max",
        "worst case",
        "pushed",
        "into kept",
    ]);
    let mut cases = sheet();
    cases.extend(threshold_probes());
    let mut fails = Vec::new();
    let (mut worst_dev, mut worst_e2e) = (0.0f64, 0.0f64);
    let (mut worst_over, mut worst_under) = (0.0f64, 0.0f64);
    for g in grids {
        let (mut dev, mut e2e, mut pushed) = ((0.0f64, String::new()), (0.0f64, String::new()), 0);
        let mut into = 0.0f64;
        for c in &cases {
            for d in OFFSETS {
                let k = Knob {
                    grid: Some(*g),
                    ..Knob::pinned()
                };
                let out = planned(c, &settings(d, k));
                let (trace, m) = measured(&out, &SimSettings::device(d, *g));
                pushed += m.pushed;
                into = into.max(m.max_into_kept_mm);
                if deviation(&m) > dev.0 {
                    dev = (deviation(&m), format!("{} {d}", c.name));
                }
                let (over, under) = end_to_end(c, &out, &trace);
                let e = over.max(under);
                if e > e2e.0 {
                    e2e = (e, format!("{} {d}", c.name));
                }
                worst_over = worst_over.max(over);
                worst_under = worst_under.max(under);
            }
        }
        let at = format!(
            "scale {:?} mirror {} origin {:?}",
            g.scale, g.mirror_x, g.origin
        );
        if e2e.0 > END_TO_END_LIMIT_MM {
            fails.push(format!("STOP: {at}: end to end {} at {}", e2e.0, e2e.1));
        }
        if dev.0 > DEVICE_LIMIT_MM {
            fails.push(format!("{at}: device {} at {}", dev.0, dev.1));
        }
        if pushed > 0 {
            fails.push(format!("{at}: {pushed} pushed"));
        }
        if into > 0.0 {
            fails.push(format!("STOP: {at}: into kept {into}"));
        }
        worst_dev = worst_dev.max(dev.0);
        worst_e2e = worst_e2e.max(e2e.0);
        row(&[
            format!("{}, {}", g.scale.x, g.scale.y),
            g.mirror_x.to_string(),
            format!("{}, {}", g.origin.x, g.origin.y),
            mm(dev.0),
            dev.1,
            mm(e2e.0),
            e2e.1,
            pushed.to_string(),
            mm(into),
        ]);
    }
    println!();
    println!(
        "worst device deviation {worst_dev:.5}, worst end to end {worst_e2e:.5} \
         (cut to source {worst_over:.5}, source to cut {worst_under:.5})"
    );
    assert!(fails.is_empty(), "{fails:#?}");
}

/// The device output end to end at every grid position (32 grids).
#[test]
#[ignore = "full sweep; run with --ignored --nocapture"]
fn device_output_end_to_end_at_every_grid_position() {
    grid_position_rule(&grid_positions());
}

/// The fast twin: origin 0, the review's odd calibrated grid and the two
/// worst positions of the full sweep.
#[test]
fn end_to_end_on_curves_stays_within_the_limit() {
    let all = grid_positions();
    let pick = |mirror: bool, x: f64, y: f64| {
        *all.iter()
            .find(|g| g.mirror_x == mirror && g.origin == Point::new(x, y))
            .unwrap()
    };
    grid_position_rule(&[
        grid(),
        calibrated_grid(),
        pick(true, 0.0125, 0.0125),
        pick(false, 0.025, 0.0375),
    ]);
}
