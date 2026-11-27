//! The quality benchmark suite (Task 18): a fixed set of jobs, each
//! planned with every cut strategy and two comparison variants, simulated
//! on the device grid and measured. `tests/fixtures/bench/baseline.json`
//! holds the committed numbers, `crates/flatcraft-engine/tests/benchmarks.rs`
//! checks every run against them, and `cargo xtask bench-quality --write`
//! writes them again together with `docs/quality/benchmarks.md`.
//!
//! Every job runs on a host compensated Cameo 2 (File Output) with a
//! 0.3 mm blade: `Host { offset_mm = 0.3, pivot_threshold_deg = 0 }`.
//!
//! The compared numbers are deterministic: no wall clock, no thread order.
//! Only [`BenchRow::millis`] is measured time, and it is reported only.

use std::path::Path;
use std::time::Instant;

use flatcraft_core::document::{CutStrategy, ObjectId};
use flatcraft_path::{PathRef, PipelineSettings, PlacedPath, StrategySettings, plan};
use flatcraft_sim::{DEVICE_LIMIT_MM, SimSettings, measure, simulate as sim_trace};
use kurbo::{Point, Rect, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::prefs::Preferences;
use crate::session::{Origin, Session};
use crate::simulate::{self, SimGrid, grade_name, strategy_name};

/// The blade offset of every benchmark job, mm.
pub const BLADE_OFFSET_MM: f64 = 0.3;

/// The id of the benchmark's plotter preset.
pub const PLOTTER_ID: &str = "bench";

/// Stored numbers are rounded to this (mm, s and score alike), so the
/// committed file stays readable and a last-digit difference between
/// machines does not show as a change.
pub const ROUNDING: f64 = 1e-6;

/// The case whose dots are mostly below the material's minimum radius:
/// its rows are report only for the device limit.
pub const TINY_DOTS: &str = "tiny-dots";

/// The dense raster case: [`dense_trace_png`] traced with the defaults.
pub const DENSE_TRACE: &str = "dense-trace";

/// One job of the suite.
#[derive(Debug, Clone, Copy)]
pub struct BenchCase {
    pub name: &'static str,
    /// Puts the job's objects into the active document of a session made
    /// by [`session`].
    pub build: fn(&mut Session),
}

/// How a case is planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// One of the three cut strategies, as the person picks them.
    Strategy(CutStrategy),
    /// The order without the M2 improvements: nearest neighbour plus
    /// 2-opt only, no Or-opt, entry DP, turn or crossing terms (R8 check).
    /// Comparison only.
    Baseline,
    /// Two passes, closed rings chained without a lift (the default).
    /// Comparison only: the partner of [`Variant::PassesSeparate`].
    PassesChained,
    /// Two passes as separate lifted laps (R6 check: per-pass closure gap
    /// and start deviation against [`Variant::PassesChained`]).
    /// Comparison only.
    PassesSeparate,
}

impl Variant {
    /// Every variant, in the order of the tables.
    pub const ALL: [Variant; 6] = [
        Variant::Strategy(CutStrategy::Quality),
        Variant::Strategy(CutStrategy::Fast),
        Variant::Strategy(CutStrategy::OutlinesLast),
        Variant::Baseline,
        Variant::PassesChained,
        Variant::PassesSeparate,
    ];

    /// The name in the tables and in `baseline.json`.
    pub fn name(self) -> &'static str {
        match self {
            Variant::Strategy(s) => strategy_name(s),
            Variant::Baseline => "baseline",
            Variant::PassesChained => "passes_chained",
            Variant::PassesSeparate => "passes_separate",
        }
    }

    /// True for the three strategies: the rows the regression rule and
    /// the device limit apply to.
    pub fn is_strategy(self) -> bool {
        matches!(self, Variant::Strategy(_))
    }

    fn strategy(self) -> CutStrategy {
        match self {
            Variant::Strategy(s) => s,
            _ => CutStrategy::Quality,
        }
    }

    fn passes(self) -> Option<u32> {
        matches!(self, Variant::PassesChained | Variant::PassesSeparate).then_some(2)
    }

    /// What the variant changes in the pipeline settings of the job.
    fn tweak(self, s: &mut PipelineSettings) {
        match self {
            Variant::Baseline => {
                s.strategy = StrategySettings {
                    or_opt: false,
                    entry_dp: false,
                    crossing_penalty: false,
                    turn_weight_mm_per_rad: 0.0,
                    ..s.strategy
                };
            }
            Variant::PassesSeparate => s.chain_closed_passes = false,
            Variant::Strategy(_) | Variant::PassesChained => {}
        }
    }
}

/// The measured numbers of one case planned with one variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchRow {
    pub case: String,
    pub variant: String,
    pub score: f64,
    pub grade: String,
    pub max_hausdorff_mm: f64,
    /// Task 10's correspondence deviation; the device limit applies to
    /// the larger of this and `max_hausdorff_mm`.
    pub max_correspondence_mm: f64,
    /// The blade turn at the entries, summed over the cut toolpaths, deg:
    /// at each start, the angle between the heading the blade has (the
    /// last segment of the toolpath before, or the assumed start heading)
    /// and the first segment of the toolpath. What the Quality strategy's
    /// turn weight lowers.
    pub entry_turn_deg: f64,
    pub max_start_deviation_mm: f64,
    pub max_into_kept_mm: f64,
    pub max_closure_gap_mm: f64,
    pub uncut_mm: f64,
    pub pushed: usize,
    pub travel_mm: f64,
    pub travel_crossings: usize,
    pub segments: usize,
    pub lifts: usize,
    pub estimate_s: f64,
    /// Wall time of the plan and the simulation, ms. Report only: never
    /// stored, never compared.
    #[serde(skip)]
    pub millis: u64,
}

impl BenchRow {
    /// The deviation the device limit applies to.
    pub fn max_deviation_mm(&self) -> f64 {
        self.max_hausdorff_mm.max(self.max_correspondence_mm)
    }

    fn is_strategy(&self) -> bool {
        Variant::ALL
            .iter()
            .any(|v| v.is_strategy() && v.name() == self.variant)
    }
}

/// `v` to six decimals ([`ROUNDING`]), as the shortest float that prints
/// so: `0.0176`, not `0.017599999999999998`.
fn round(v: f64) -> f64 {
    format!("{v:.6}").parse().unwrap_or(v)
}

// ---------------------------------------------------------------- cases

const SMALL_TEXT: &str = include_str!("../../../tests/fixtures/bench/small-text.svg");
const STICKERS: &str = include_str!("../../../tests/fixtures/bench/stickers-with-holes.svg");
const TINY_DOTS_SVG: &str = include_str!("../../../tests/fixtures/bench/tiny-dots.svg");
const LONG_THIN: &str = include_str!("../../../tests/fixtures/bench/long-thin.svg");
const ROUND_HOLES: &str = include_str!("../../../tests/fixtures/bench/round-holes.svg");

fn run(s: &mut Session, id: &str, params: Value) {
    if let Err(e) = s.execute(id, params, Origin::User) {
        panic!("bench: {id} failed: {e}");
    }
}

fn import_svg(s: &mut Session, name: &str, svg: &str) {
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }));
}

/// The test sheet, the six fixtures, in the order of the tables.
pub fn cases() -> Vec<BenchCase> {
    vec![
        BenchCase {
            name: "test-sheet",
            build: |s| run(s, "tools.test_patterns", json!({})),
        },
        BenchCase {
            name: "small-text",
            build: |s| import_svg(s, "small-text", SMALL_TEXT),
        },
        BenchCase {
            name: "stickers-with-holes",
            build: |s| import_svg(s, "stickers-with-holes", STICKERS),
        },
        BenchCase {
            name: TINY_DOTS,
            build: |s| import_svg(s, TINY_DOTS, TINY_DOTS_SVG),
        },
        BenchCase {
            name: "long-thin",
            build: |s| import_svg(s, "long-thin", LONG_THIN),
        },
        BenchCase {
            name: "round-holes",
            build: |s| import_svg(s, "round-holes", ROUND_HOLES),
        },
        BenchCase {
            name: DENSE_TRACE,
            build: |s| {
                if let Err(e) = crate::commands::object::add_raster(
                    s,
                    DENSE_TRACE.to_string(),
                    dense_trace_png().into(),
                ) {
                    panic!("bench: the dense trace failed: {e}");
                }
            },
        },
    ]
}

/// The case called `name`.
pub fn case(name: &str) -> Option<BenchCase> {
    cases().into_iter().find(|c| c.name == name)
}

// ---------------------------------------------------------------- dense trace

/// Side of [`dense_trace_png`], px (300 dpi: 56.9 mm).
pub const DENSE_TRACE_PX: u32 = 672;

/// The cell of one shape in [`dense_trace_png`], px.
const CELL_PX: u32 = 48;

/// The dense-trace fixture: a 14 x 14 field of small shapes, black on
/// white: dots, squares, diamonds and plus signs of 2.4 to 3.4 mm, each
/// jittered in its cell by a fixed generator, and rings and framed squares
/// of 3.6 mm whose holes (2 mm) stay above the tracer's minimum feature
/// size. Drawn with plain arithmetic per pixel (no anti-aliasing, no
/// trigonometry), so the bytes are the same on every machine. The case
/// traces these bytes; `tests/fixtures/bench/dense-trace.png` is the same
/// image to look at, and a test keeps the two equal.
pub fn dense_trace_png() -> Vec<u8> {
    let n = DENSE_TRACE_PX;
    let cells = n / CELL_PX;
    let mut seed = 0x5EED_F1A7u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as i64
    };
    // (centre x, centre y, radius, kind) per cell, in px. A shape reaches
    // at most 23 px from its cell centre, so neighbours stay apart.
    let shapes: Vec<(i64, i64, i64, i64)> = (0..cells * cells)
        .map(|k| {
            let (cx, cy) = (i64::from(k % cells), i64::from(k / cells));
            let kind = next() % 6;
            let holed = kind == 2 || kind == 5;
            let (r, jitter) = if holed { (21, 1) } else { (14 + next() % 7, 3) };
            let jx = next() % (2 * jitter + 1) - jitter;
            let jy = next() % (2 * jitter + 1) - jitter;
            let half = i64::from(CELL_PX) / 2;
            (
                cx * i64::from(CELL_PX) + half + jx,
                cy * i64::from(CELL_PX) + half + jy,
                r,
                kind,
            )
        })
        .collect();
    let inside = |x: i64, y: i64| {
        let cell = (y / i64::from(CELL_PX)) * i64::from(cells) + x / i64::from(CELL_PX);
        let Some(&(cx, cy, r, kind)) = usize::try_from(cell).ok().and_then(|c| shapes.get(c))
        else {
            return false;
        };
        let (dx, dy) = (x - cx, y - cy);
        let d2 = dx * dx + dy * dy;
        match kind {
            0 => d2 <= r * r,
            1 => dx.abs() <= r * 4 / 5 && dy.abs() <= r * 4 / 5,
            2 => d2 <= r * r && d2 > 12 * 12,
            3 => dx.abs() + dy.abs() <= r,
            4 => (dx.abs() <= r / 3 && dy.abs() <= r) || (dy.abs() <= r / 3 && dx.abs() <= r),
            _ => {
                let m = dx.abs().max(dy.abs());
                m <= r && m > 12
            }
        }
    };
    let mut data = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..i64::from(n) {
        for x in 0..i64::from(n) {
            let v = if inside(x, y) { 0 } else { 255 };
            data.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let size = tiny_skia::IntSize::from_wh(n, n).expect("a valid size");
    tiny_skia::Pixmap::from_vec(data, size)
        .expect("the pixel data fits the size")
        .encode_png()
        .expect("a PNG in memory")
}

// ---------------------------------------------------------------- runs

/// A session for the suite: the system presets, plus the benchmark's
/// plotter preset (File Output with host compensation and a 0.3 mm blade)
/// written to `config_dir/plotters`. `config_dir` is a scratch folder the
/// caller owns (a tempdir); the output folder is a folder inside it.
pub fn session(config_dir: &Path) -> Session {
    let plotters = config_dir.join("plotters");
    std::fs::create_dir_all(&plotters).expect("bench: create the plotter folder");
    std::fs::write(
        plotters.join(format!("{PLOTTER_ID}.toml")),
        format!(
            "id = \"{PLOTTER_ID}\"\nname = \"Benchmark\"\ninherits = \"file-output-gpgl-cameo2\"\nknife_compensation = {{ Host = {{ offset_mm = {BLADE_OFFSET_MM:?}, pivot_threshold_deg = 0.0 }} }}\nblade_offset_mm = {BLADE_OFFSET_MM:?}\n"
        ),
    )
    .expect("bench: write the plotter preset");
    let profiles =
        flatcraft_device::ProfileStore::load(config_dir).expect("bench: load the presets");
    let prefs = Preferences {
        output_dir: Some(config_dir.join("output")),
        ..Preferences::default()
    };
    let mut s = Session::new(profiles, prefs, false, config_dir.to_path_buf());
    run(&mut s, "job.set_plotter", json!({ "id": PLOTTER_ID }));
    s
}

/// The snapshot of `case` (already built into `s`) for `variant`: what
/// [`run_variant`] computes, taken from the session so the computation
/// itself can run on another thread.
fn variant_input(s: &Session, case: &str, variant: Variant) -> simulate::SimInput {
    let mut input = simulate::snapshot(s, Some(variant.strategy()), SimGrid::Device)
        .unwrap_or_else(|e| panic!("bench: {case} snapshot failed: {e}"));
    if let Some(passes) = variant.passes() {
        input.input.job.passes = Some(passes);
    }
    input
}

/// Plans and simulates one snapshot of [`variant_input`] and measures it.
fn compute_row(input: simulate::SimInput, case: &str, variant: Variant) -> BenchRow {
    let started = Instant::now();
    let sim = simulate::compute_with(input, |p| variant.tweak(p))
        .unwrap_or_else(|e| panic!("bench: {case} {} failed: {e}", variant.name()));
    let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let r = sim
        .result
        .as_ref()
        .unwrap_or_else(|| panic!("bench: {case} was not simulated (host mode expected)"));
    let m = &r.metrics;
    BenchRow {
        case: case.to_string(),
        variant: variant.name().to_string(),
        score: round(m.score),
        grade: grade_name(m.grade).to_string(),
        max_hausdorff_mm: round(m.max_hausdorff_mm),
        max_correspondence_mm: round(m.max_correspondence_mm),
        entry_turn_deg: round(entry_turn_deg(
            sim.plan_index.iter().map(|&i| &sim.job.plan.toolpaths[i]),
        )),
        max_start_deviation_mm: round(m.max_start_deviation_mm),
        max_into_kept_mm: round(m.max_into_kept_mm),
        max_closure_gap_mm: round(m.max_closure_gap_mm),
        uncut_mm: round(m.uncut_mm),
        pushed: m.pushed,
        travel_mm: round(m.travel_mm),
        travel_crossings: m.travel_crossings,
        segments: m.segments,
        lifts: m.lifts,
        estimate_s: round(sim.estimate_s),
        millis,
    }
}

/// See [`BenchRow::entry_turn_deg`].
pub fn entry_turn_deg<'a>(toolpaths: impl Iterator<Item = &'a flatcraft_path::Toolpath>) -> f64 {
    let mut heading = flatcraft_path::ASSUMED_START_HEADING;
    let mut total = 0.0;
    for t in toolpaths {
        let p = &t.points;
        let Some(first) = p.windows(2).map(|w| w[1] - w[0]).find(|v| v.hypot2() > 0.0) else {
            continue;
        };
        total += flatcraft_path::order::turn_rad(heading, first).to_degrees();
        if let Some(last) = p
            .windows(2)
            .rev()
            .map(|w| w[1] - w[0])
            .find(|v| v.hypot2() > 0.0)
        {
            heading = last;
        }
    }
    total
}

/// `case` planned with `variant` on a session made by [`session`] (the
/// case already built into it), simulated on the device grid.
pub fn run_variant(s: &Session, case: &str, variant: Variant) -> BenchRow {
    compute_row(variant_input(s, case, variant), case, variant)
}

/// `case` with each of `variants`, in that order, on a fresh session in
/// `config_dir`. The snapshots are taken one after the other; the plans
/// and simulations run on a thread each, and the rows keep the order of
/// `variants`.
pub fn run_case(config_dir: &Path, case: &BenchCase, variants: &[Variant]) -> Vec<BenchRow> {
    let mut s = session(config_dir);
    (case.build)(&mut s);
    let inputs: Vec<_> = variants
        .iter()
        .map(|&v| (variant_input(&s, case.name, v), v))
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .into_iter()
            .map(|(input, v)| scope.spawn(move || compute_row(input, case.name, v)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    })
}

/// Every case with every variant (host, d = 0.3, device grid). Each case
/// gets its own session in a folder under `config_dir` and its own thread;
/// the rows come back in case order, so the result does not depend on
/// which thread finishes first.
pub fn run_all(config_dir: &Path) -> Vec<BenchRow> {
    let cases = cases();
    std::thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|c| scope.spawn(move || run_case(&config_dir.join(c.name), c, &Variant::ALL)))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    })
}

// ---------------------------------------------------------------- crossings

/// Task 6's crossing sweep: small jobs of 40 or 150 squares of 0.5 to
/// 4.5 mm in 60 x 60 mm (seeds 1 to 59, the generator of
/// `order_improvement_lowers_simulated_travel_crossings`), planned with
/// the order improvement and with the construction alone (no improvement
/// rounds), and counted by the float simulation (`SimSettings::float(0.0)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossingSweep {
    pub jobs: usize,
    /// Jobs where the improvement left fewer travel crossings.
    pub fewer: usize,
    /// Jobs where it left more.
    pub more: usize,
    /// Jobs where the count stayed.
    pub same: usize,
    /// Travel crossings of all jobs, construction alone.
    pub construction_crossings: usize,
    /// Travel crossings of all jobs, with the improvement.
    pub improved_crossings: usize,
}

/// How many jobs of the sweep may move between fewer, more and same
/// before it counts as a regression. One float decision of the order
/// (an atan2 or a sin one bit off on another OS's maths library) can flip
/// a job; the counts are only checked for a real trend. `pushed` and the
/// cut into kept material keep no slack: they are safety numbers.
pub const SWEEP_SLACK: usize = 2;

/// The seeds of the sweep.
pub const SWEEP_SEEDS: std::ops::RangeInclusive<u64> = 1..=59;

/// The job sizes of the sweep.
pub const SWEEP_SIZES: [usize; 2] = [40, 150];

fn sweep_job(seed: u64, n: usize) -> Vec<PlacedPath> {
    let mut seed = seed;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..n as u64)
        .map(|k| {
            let (x, y) = (10.0 + next() * 60.0, 10.0 + next() * 60.0);
            let (w, h) = (0.5 + next() * 4.0, 0.5 + next() * 4.0);
            PlacedPath {
                path: Rect::new(x, y, x + w, y + h).to_path(0.1),
                source: PathRef {
                    object: ObjectId(k),
                    path_index: 0,
                },
            }
        })
        .collect()
}

fn sweep_crossings(input: &[PlacedPath], rounds: Option<u32>) -> usize {
    let mut strategy = StrategySettings::default();
    if let Some(rounds) = rounds {
        strategy.rounds = rounds;
    }
    let settings = PipelineSettings {
        strategy,
        ..PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::ZERO)
    };
    let out = plan(input, &settings).expect("bench: the sweep job plans");
    let s = SimSettings::float(0.0);
    let trace = sim_trace(&out.toolpaths, out.home, &s).expect("bench: the sweep job simulates");
    measure(&out.toolpaths, &trace, &s).0.travel_crossings
}

/// Runs the crossing sweep. The jobs are spread over the machine's
/// threads; the counts do not depend on the order they finish in.
pub fn crossing_sweep() -> CrossingSweep {
    let jobs: Vec<(u64, usize)> = SWEEP_SIZES
        .iter()
        .flat_map(|&n| SWEEP_SEEDS.map(move |seed| (seed, n)))
        .collect();
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let per = jobs.len().div_ceil(threads.max(1));
    let counts: Vec<(usize, usize)> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(per.max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&(seed, n)| {
                            let job = sweep_job(seed, n);
                            (sweep_crossings(&job, Some(0)), sweep_crossings(&job, None))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    let mut out = CrossingSweep {
        jobs: counts.len(),
        fewer: 0,
        more: 0,
        same: 0,
        construction_crossings: 0,
        improved_crossings: 0,
    };
    for (built, improved) in counts {
        out.construction_crossings += built;
        out.improved_crossings += improved;
        match improved.cmp(&built) {
            std::cmp::Ordering::Less => out.fewer += 1,
            std::cmp::Ordering::Greater => out.more += 1,
            std::cmp::Ordering::Equal => out.same += 1,
        }
    }
    out
}

// ---------------------------------------------------------------- baseline

/// The committed numbers: `tests/fixtures/bench/baseline.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub rows: Vec<BenchRow>,
    pub crossings: CrossingSweep,
}

impl Baseline {
    /// The file text: pretty JSON with a final newline.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("rows serialize");
        s.push('\n');
        s
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// What a row may lose against its baseline before it counts as a
/// regression: the plan decision of Task 18.
pub mod rule {
    /// The score may drop by this.
    pub const SCORE_DROP: f64 = 1.0;
    /// A mm metric may grow by this factor ...
    pub const MM_FACTOR: f64 = 1.05;
    /// ... plus this, mm.
    pub const MM_SLACK: f64 = 0.002;
    /// Travel may grow by this factor.
    pub const TRAVEL_FACTOR: f64 = 1.05;
    /// Segments may grow by this factor.
    pub const SEGMENTS_FACTOR: f64 = 1.10;
}

/// Every way `now` is worse than `base` beyond the rule, as text.
pub fn row_regressions(base: &BenchRow, now: &BenchRow) -> Vec<String> {
    let mut out = Vec::new();
    let id = format!("{} {}", now.case, now.variant);
    if now.score < base.score - rule::SCORE_DROP {
        out.push(format!(
            "{id}: score {} < {} - {}",
            now.score,
            base.score,
            rule::SCORE_DROP
        ));
    }
    for (name, b, n) in [
        (
            "max_hausdorff_mm",
            base.max_hausdorff_mm,
            now.max_hausdorff_mm,
        ),
        (
            "max_correspondence_mm",
            base.max_correspondence_mm,
            now.max_correspondence_mm,
        ),
        (
            "max_start_deviation_mm",
            base.max_start_deviation_mm,
            now.max_start_deviation_mm,
        ),
        (
            "max_closure_gap_mm",
            base.max_closure_gap_mm,
            now.max_closure_gap_mm,
        ),
        ("uncut_mm", base.uncut_mm, now.uncut_mm),
    ] {
        let limit = b * rule::MM_FACTOR + rule::MM_SLACK;
        if n > limit {
            out.push(format!("{id}: {name} {n} > {limit:.6} (baseline {b})"));
        }
    }
    // Stored values are rounded to ROUNDING; half of it covers the last digit.
    let eps = ROUNDING / 2.0;
    if now.travel_mm > base.travel_mm * rule::TRAVEL_FACTOR + eps {
        out.push(format!(
            "{id}: travel_mm {} > {} x {}",
            now.travel_mm,
            base.travel_mm,
            rule::TRAVEL_FACTOR
        ));
    }
    if now.segments as f64 > base.segments as f64 * rule::SEGMENTS_FACTOR {
        out.push(format!(
            "{id}: segments {} > {} x {}",
            now.segments,
            base.segments,
            rule::SEGMENTS_FACTOR
        ));
    }
    if now.pushed > base.pushed {
        out.push(format!("{id}: pushed {} > {}", now.pushed, base.pushed));
    }
    if now.max_into_kept_mm > base.max_into_kept_mm + eps {
        out.push(format!(
            "{id}: max_into_kept_mm {} > {}",
            now.max_into_kept_mm, base.max_into_kept_mm
        ));
    }
    out
}

/// Every regression of `rows` and `sweep` against `base`: the rule per
/// strategy row (comparison rows are not checked), a strategy row without
/// a baseline row, and a crossing sweep with more jobs that got more
/// crossings or fewer jobs that got fewer.
pub fn regressions(base: &Baseline, rows: &[BenchRow], sweep: &CrossingSweep) -> Vec<String> {
    let mut out = Vec::new();
    for now in rows.iter().filter(|r| r.is_strategy()) {
        match base
            .rows
            .iter()
            .find(|b| b.case == now.case && b.variant == now.variant)
        {
            Some(b) => out.extend(row_regressions(b, now)),
            None => out.push(format!(
                "{} {}: no baseline row (run cargo xtask bench-quality --write)",
                now.case, now.variant
            )),
        }
    }
    let b = &base.crossings;
    if sweep.jobs != b.jobs {
        out.push(format!(
            "crossing sweep: {} jobs, the baseline has {}",
            sweep.jobs, b.jobs
        ));
    }
    if sweep.more > b.more + SWEEP_SLACK {
        out.push(format!(
            "crossing sweep: the improvement adds crossings on {} jobs (baseline {})",
            sweep.more, b.more
        ));
    }
    if sweep.fewer + SWEEP_SLACK < b.fewer {
        out.push(format!(
            "crossing sweep: the improvement removes crossings on {} jobs (baseline {})",
            sweep.fewer, b.fewer
        ));
    }
    out
}

/// The strategy rows (not [`TINY_DOTS`]) whose deviation, start deviation
/// or closure gap is above `DEVICE_LIMIT_MM`, as text.
pub fn device_limit_misses(rows: &[BenchRow]) -> Vec<String> {
    rows.iter()
        .filter(|r| r.is_strategy() && r.case != TINY_DOTS)
        .flat_map(|r| {
            [
                ("deviation", r.max_deviation_mm()),
                ("start deviation", r.max_start_deviation_mm),
                ("closure gap", r.max_closure_gap_mm),
            ]
            .into_iter()
            .filter(|(_, v)| *v > DEVICE_LIMIT_MM)
            .map(|(name, v)| {
                format!(
                    "{} {}: {name} {v:.4} mm > {DEVICE_LIMIT_MM} mm",
                    r.case, r.variant
                )
            })
            .collect::<Vec<_>>()
        })
        .collect()
}

// ---------------------------------------------------------------- report

fn mm(v: f64) -> String {
    format!("{v:.4}")
}

/// The table of `rows` as Markdown, one line per row. With `millis`, a
/// time column too (the console report; the committed page has none, as
/// times differ per machine).
pub fn table(rows: &[BenchRow], millis: bool) -> String {
    let mut s = String::from(
        "| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | entry turn deg | segments | lifts | time s |",
    );
    if millis {
        s.push_str(" ms |");
    }
    s.push('\n');
    s.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    if millis {
        s.push_str("---|");
    }
    s.push('\n');
    for r in rows {
        s.push_str(&format!(
            "| {} | {:.1} | {} | {} | {} | {} | {} | {} | {} | {:.1} | {} | {:.0} | {} | {} | {:.1} |",
            r.variant,
            r.score,
            r.grade,
            mm(r.max_deviation_mm()),
            mm(r.max_start_deviation_mm),
            mm(r.max_closure_gap_mm),
            mm(r.max_into_kept_mm),
            mm(r.uncut_mm),
            r.pushed,
            r.travel_mm,
            r.travel_crossings,
            r.entry_turn_deg,
            r.segments,
            r.lifts,
            r.estimate_s,
        ));
        if millis {
            s.push_str(&format!(" {} |", r.millis));
        }
        s.push('\n');
    }
    s
}

/// The tables of every case, under a `###` heading each.
pub fn tables(rows: &[BenchRow], millis: bool) -> String {
    let mut s = String::new();
    for c in cases() {
        let of: Vec<BenchRow> = rows.iter().filter(|r| r.case == c.name).cloned().collect();
        if of.is_empty() {
            continue;
        }
        s.push_str(&format!("### {}\n\n{}\n", c.name, table(&of, millis)));
    }
    s
}

/// The crossing sweep as a Markdown table.
pub fn sweep_table(c: &CrossingSweep) -> String {
    format!(
        "| jobs | fewer crossings | more | same | crossings, construction alone | crossings, improved |\n|---|---|---|---|---|---|\n| {} | {} | {} | {} | {} | {} |\n",
        c.jobs, c.fewer, c.more, c.same, c.construction_crossings, c.improved_crossings
    )
}

/// `docs/quality/benchmarks.md`.
pub fn markdown(base: &Baseline) -> String {
    let mut s = String::from(PAGE_INTRO);
    s.push_str(&PAGE_RULE.replace("@LIMIT@", &DEVICE_LIMIT_MM.to_string()));
    s.push_str(&tables(&base.rows, false));
    s.push_str(PAGE_SWEEP);
    s.push_str(&sweep_table(&base.crossings));
    s
}

const PAGE_INTRO: &str = r#"# Quality benchmarks

"Measure twice, cut once." FlatCraft measures every cut before the blade
touches the material: the knife simulator follows the blade tip along the
planned path and checks how far it strays. This page holds those numbers for
a fixed set of jobs, so a change that makes the cuts worse shows up before
it ships.

This page is written by `cargo xtask bench-quality --write`. Do not edit it
by hand.

## The jobs

Every job is cut by a Silhouette Cameo 2 (File Output) with a 0.3 mm blade
and host compensation (FlatCraft steers the blade around every corner), on
the 12 x 12 inch mat, and simulated on the plotter's own grid of 20 steps
per mm, so the numbers include the rounding the plotter really gets.

- **test-sheet**: the built-in test patterns (Test patterns in the app).
- **small-text**: block letters 4.8 mm high, drawn as filled shapes.
- **stickers-with-holes**: six stickers with windows; the donut has an
  island in its hole.
- **tiny-dots**: 96 dots with a radius of 0.1 to 0.6 mm. Most of them are
  smaller than the blade can turn around, so their numbers are a report, not
  a limit.
- **long-thin**: strips, a wave, a frame and a needle, up to 250 mm long.
- **round-holes**: a plate with 27 round holes of 1 to 10 mm, and three
  washers.
- **dense-trace**: a picture of 196 small shapes, some with holes, traced
  into cut lines with the default trace settings.

The files are in `tests/fixtures/bench`. They were drawn for FlatCraft.

## The variants

- **quality**, **fast**, **outlines_last**: the three cut strategies, as you
  pick them in the job settings.
- **baseline**: the cut order without the newer tricks (nearest neighbour
  plus 2-opt only). It shows what the better order buys.
- **passes_chained** and **passes_separate**: two passes, once as one
  ring that goes round twice without lifting the blade (the default), once
  as two separate laps with a lift between them.

## What the columns mean

- **score** and **grade**: the quality report's summary (100 is perfect).
  On these jobs the score is 100 everywhere: every row stays well inside
  the limits, so the score cannot tell the rows apart. The other columns
  can.
- **deviation**: how far the blade tip strays from the planned line at its
  worst, mm.
- **start**: the worst stray where a cut begins, mm.
- **closure gap**: the worst gap where a ring should meet its start, mm.
- **into kept**: how deep a waste move cuts into material you keep, mm.
- **uncut**: line length the blade never cuts through, mm.
- **pushed**: how often the blade is pushed from behind.
- **travel mm** and **crossings**: blade-up moves, and how often they cross
  a line that is cut already.
- **entry turn deg**: how far the blade has to turn at the starts of all
  cuts together, in degrees. The quality strategy spends some travel to
  keep this low.
- **segments** and **lifts**: straight moves with the blade down, and
  how often it goes up.
- **time s**: the time estimate in seconds.

## The rule

Every strategy row must stay as good as its row in
`tests/fixtures/bench/baseline.json`:

- the score drops by at most 1,
- each mm number grows by at most 5 % plus 0.002 mm,
- travel grows by at most 5 %,
- segments grow by at most 10 %,
- pushes and the cut into kept material never grow, not even a little.

The strategy rows (except tiny-dots) also stay within the device limit of
"#;

/// `@LIMIT@` is the device limit.
const PAGE_RULE: &str = r#"@LIMIT@ mm. The baseline and passes rows are for comparison only.
A change that makes the numbers better writes the new baseline in the same
commit, with the difference in the commit message.

Two more checks keep the cut order honest:

- **fast** has no more travel than **baseline** on every job: the newer
  order tricks never cost travel.
- **quality** turns the blade less at the starts than **baseline** and
  has no more crossings, on every job: what it pays in travel, it gets
  back where it counts.

The test `cargo test -p flatcraft-engine --test benchmarks` checks all of
this in CI. `cargo xtask bench-quality` prints the tables with the time each
run took on your machine.

## Results

"#;

const PAGE_SWEEP: &str = r#"## Travel crossings of the order improvement

A blade-up move that crosses a finished cut can snag a loose piece. The
order improvement (2-opt, Or-opt and the entry search) avoids crossings
but does not count them exactly. This sweep plans 118 small jobs (40 or 150
squares of 0.5 to 4.5 mm in 60 x 60 mm, seeds 1 to 59) with the improvement
and with the first nearest-neighbour order alone, and counts the crossings
in the simulation. The rule: the number of jobs the improvement makes
worse never grows, and the number it makes better never drops, give or
take 2 jobs (another computer's maths library can tip one close call the
other way).

"#;
