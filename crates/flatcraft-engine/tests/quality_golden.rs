//! Task 16: the golden quality suite. The test sheet of
//! `tools.test_patterns`, cut by a host compensated plotter (the File
//! Output preset with `Host { offset_mm = 0.3, pivot_threshold_deg = 0 }`
//! and a 0.3 mm blade), simulated through the engine's own three steps
//! (`simulate::snapshot`, `compute`) in float and on the device grid, and
//! measured per pattern group (the `<g id>` label of each path).
//!
//! - Float (the algorithm alone, planned and simulated without rounding):
//!   every group within `ALGORITHM_LIMIT_MM`.
//! - Device (planned for and snapped to the device units, what is sent):
//!   every group within `DEVICE_LIMIT_MM`, with every strategy.
//! - Upstream: the intended path against the source shapes, and the
//!   device cut against the source shapes end to end.
//!
//! Run with `--nocapture` for the tables; `task-16-report.md` holds them.

use std::collections::BTreeMap;
use std::sync::Arc;

use flatcraft_core::document::CutStrategy;
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use flatcraft_engine::simulate::{self, STRATEGIES, SimGrid, SimMode, SimRun, strategy_name};
use flatcraft_path::ToolpathKind;
use flatcraft_sim::{
    ALGORITHM_LIMIT_MM, DEVICE_LIMIT_MM, END_TO_END_LIMIT_MM, PathMetrics, SimResult, source,
};
use kurbo::{BezPath, Point};
use serde_json::{Value, json};

/// The extra the upstream error may have over the flatten tolerance: the
/// booleans of the cut geometry (at most t/2) and the joins.
const UPSTREAM_EXTRA_MM: f64 = 0.005;

/// The pattern groups with corners (turns of 30 deg and more).
fn is_corner_group(label: &str) -> bool {
    ["square-50", "hole-in-square", "f-glyph"].contains(&label) || label.starts_with("star-")
}

struct Sheet {
    _config: tempfile::TempDir,
    _out: tempfile::TempDir,
    session: Session,
}

/// The test sheet on a user plotter that inherits File Output with
/// `knife` compensation and a blade of `blade` mm.
fn sheet(knife: &str, blade: f64) -> Sheet {
    let config = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let plotters = config.path().join("plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(
        plotters.join("golden.toml"),
        format!(
            "id = \"golden\"\nname = \"golden\"\ninherits = \"file-output-gpgl-cameo2\"\nknife_compensation = {knife}\nblade_offset_mm = {blade:?}\n"
        ),
    )
    .unwrap();
    let profiles = ProfileStore::load(config.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(out.path().to_path_buf()),
        ..Preferences::default()
    };
    let mut session = Session::new(profiles, prefs, false, config.path().to_path_buf());
    run(&mut session, "job.set_plotter", json!({ "id": "golden" }));
    run(&mut session, "tools.test_patterns", json!({}));
    Sheet {
        _config: config,
        _out: out,
        session,
    }
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

const HOST: &str = "{ Host = { offset_mm = 0.3, pivot_threshold_deg = 0.0 } }";
const NONE: &str = "\"None\"";

/// One simulation of the sheet, with what the groups need.
struct Golden {
    run: Arc<SimRun>,
    /// The pattern group of each simulated toolpath.
    group: Vec<String>,
    /// The source shapes of each group, in mat mm.
    source: BTreeMap<String, Vec<BezPath>>,
    flatten_tolerance_mm: f64,
}

impl Golden {
    fn result(&self) -> &SimResult {
        self.run.result.as_ref().expect("simulated")
    }

    /// The simulated toolpaths (the ones the trace indexes).
    fn toolpaths(&self) -> Vec<flatcraft_path::Toolpath> {
        self.run
            .plan_index
            .iter()
            .map(|&i| self.run.job.plan.toolpaths[i].clone())
            .collect()
    }
}

fn simulate_sheet(s: &Session, strategy: Option<CutStrategy>, grid: SimGrid) -> Golden {
    let input = simulate::snapshot(s, strategy, grid).unwrap();
    let job_input = input.input.clone();
    let run = Arc::new(simulate::compute(input).unwrap());
    let cut = job_input.cut_geometry();
    let group = run
        .plan_index
        .iter()
        .map(|&i| {
            let t = &run.job.plan.toolpaths[i];
            cut.pieces(t.source.object)[t.source.path_index]
                .label
                .clone()
                .unwrap_or_default()
        })
        .collect();
    let mut source: BTreeMap<String, Vec<BezPath>> = BTreeMap::new();
    for o in &job_input.objects {
        let affine = o.transform.affine(o.local_bounds);
        for g in o.geometry.iter() {
            source
                .entry(g.label.clone().unwrap_or_default())
                .or_default()
                .push(affine * g.path.clone());
        }
    }
    Golden {
        run,
        group,
        source,
        flatten_tolerance_mm: job_input.plotter.flatten_tolerance_mm,
    }
}

/// The worst values of one pattern group.
#[derive(Debug, Default, Clone, Copy)]
struct Worst {
    /// The larger of the two-sided Hausdorff distance and the
    /// correspondence deviation (Task 10's `deviation`).
    deviation: f64,
    corner: f64,
    span_frechet: f64,
    start: f64,
    closure_gap: f64,
    uncut: f64,
    pushed: usize,
    paths: usize,
}

impl Worst {
    fn add(&mut self, m: &PathMetrics) {
        self.deviation = self
            .deviation
            .max(m.hausdorff_mm)
            .max(m.max_correspondence_mm);
        self.corner = self.corner.max(m.max_corner_deviation_mm);
        self.span_frechet = self.span_frechet.max(m.max_span_frechet_mm);
        self.start = self.start.max(m.max_start_deviation_mm);
        self.closure_gap = self.closure_gap.max(m.closure_gap_mm.unwrap_or(0.0));
        self.uncut += m.uncut_mm;
        self.pushed += m.pushed;
        self.paths += 1;
    }

    /// The limited values, as (name, value).
    fn limited(&self) -> [(&'static str, f64); 5] {
        [
            ("deviation", self.deviation),
            ("corner", self.corner),
            ("span frechet", self.span_frechet),
            ("start", self.start),
            ("closure gap", self.closure_gap),
        ]
    }
}

/// The worst values of each pattern group, over the `Cut` toolpaths.
fn by_group(g: &Golden) -> BTreeMap<String, Worst> {
    let toolpaths = g.toolpaths();
    let mut out: BTreeMap<String, Worst> = BTreeMap::new();
    for m in &g.result().metrics.per_path {
        if toolpaths[m.toolpath].kind != ToolpathKind::Cut {
            continue;
        }
        out.entry(g.group[m.toolpath].clone()).or_default().add(m);
    }
    out
}

fn mm(v: f64) -> String {
    format!("{v:.4}")
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

/// Prints the group table of `g` and returns every value over `limit`,
/// every uncut length, push and cut into kept material.
fn check_groups(what: &str, g: &Golden, limit: f64) -> Vec<String> {
    table_header(&[
        "group",
        "paths",
        "deviation",
        "corner",
        "span frechet",
        "start",
        "closure gap",
        "uncut",
        "pushed",
    ]);
    let groups = by_group(g);
    let mut fails = Vec::new();
    for (label, w) in &groups {
        row(&[
            label.clone(),
            w.paths.to_string(),
            mm(w.deviation),
            mm(w.corner),
            mm(w.span_frechet),
            mm(w.start),
            mm(w.closure_gap),
            mm(w.uncut),
            w.pushed.to_string(),
        ]);
        for (name, v) in w.limited() {
            if v > limit {
                fails.push(format!("{what} {label}: {name} {v:.5} > {limit}"));
            }
        }
        if w.uncut != 0.0 {
            fails.push(format!("{what} {label}: uncut {}", w.uncut));
        }
        if w.pushed != 0 {
            fails.push(format!("{what} {label}: pushed {}", w.pushed));
        }
    }
    let m = &g.result().metrics;
    println!(
        "\n{what}: max into kept {}, uncut {}, pushed {}, score {:.1}",
        m.max_into_kept_mm, m.uncut_mm, m.pushed, m.score
    );
    if m.max_into_kept_mm != 0.0 {
        fails.push(format!("{what}: into kept {}", m.max_into_kept_mm));
    }
    if m.uncut_mm != 0.0 {
        fails.push(format!("{what}: uncut {}", m.uncut_mm));
    }
    if m.pushed != 0 {
        fails.push(format!("{what}: pushed {}", m.pushed));
    }
    // Every group of the sheet is there.
    for label in g.source.keys() {
        if !groups.contains_key(label) {
            fails.push(format!("{what}: group {label} was not cut"));
        }
    }
    fails
}

/// The sheet's groups (`patterns::sheet_svg`): square-50, hole-in-square,
/// open-line, s-curve, circles, f-glyph, ruler-x, ruler-y and one per star
/// tip.
const GROUPS: usize = 8 + flatcraft_engine::patterns::STAR_ANGLES.len();

#[test]
fn test_sheet_meets_algorithm_limit() {
    let s = sheet(HOST, 0.3);
    let g = simulate_sheet(&s.session, None, SimGrid::Float);
    assert_eq!(g.run.mode, SimMode::Host);
    assert_eq!(g.source.len(), GROUPS, "{:?}", g.source.keys());
    // The float run plans in floats: points off the device grid, so it
    // measures the algorithm and not the rounding.
    let media = s
        .session
        .profiles
        .plotter("golden")
        .unwrap()
        .media_or_first(&s.session.doc().document.job.media_id)
        .unwrap()
        .0
        .clone();
    let grid = simulate::device_grid(&s.session.profiles.plotter("golden").unwrap(), &media);
    let off_grid = g.toolpaths().iter().flat_map(|t| t.runs()).any(|r| {
        r.points.iter().any(|&q| {
            let (u, v) = (grid.to_units(q), grid.rounded_units(q));
            (u.x - v.x).abs() > 1e-6 || (u.y - v.y).abs() > 1e-6
        })
    });
    assert!(off_grid, "the float run was planned for the device grid");
    let fails = check_groups("float", &g, ALGORITHM_LIMIT_MM);
    assert!(fails.is_empty(), "STOP: {fails:#?}");
}

#[test]
fn test_sheet_meets_device_limit() {
    let s = sheet(HOST, 0.3);
    let g = simulate_sheet(&s.session, None, SimGrid::Device);
    assert_eq!(g.source.len(), GROUPS);
    let fails = check_groups("device", &g, DEVICE_LIMIT_MM);
    assert!(fails.is_empty(), "STOP: {fails:#?}");
}

#[test]
fn every_strategy_meets_device_limit() {
    let s = sheet(HOST, 0.3);
    let mut fails = Vec::new();
    for strategy in STRATEGIES {
        let g = simulate_sheet(&s.session, Some(strategy), SimGrid::Device);
        assert_eq!(g.run.strategy, strategy);
        println!("\nstrategy {}", strategy_name(strategy));
        fails.extend(check_groups(strategy_name(strategy), &g, DEVICE_LIMIT_MM));
    }
    assert!(fails.is_empty(), "STOP: {fails:#?}");
}

/// The upstream error: the intended path (the first lap of each `Cut`
/// toolpath, without overcut) against the source shapes flattened at
/// 1e-4 mm, both ways, within the flatten tolerance plus
/// [`UPSTREAM_EXTRA_MM`]. Then the device cut against the source shapes,
/// both ways (Task 10's check), within `END_TO_END_LIMIT_MM`, and the
/// device deviation plus the upstream error within it too.
#[test]
fn upstream_error_fits_the_end_to_end_budget() {
    let s = sheet(HOST, 0.3);
    let g = simulate_sheet(&s.session, None, SimGrid::Device);
    let toolpaths = g.toolpaths();
    let device = by_group(&g);
    let upstream_limit = g.flatten_tolerance_mm + UPSTREAM_EXTRA_MM;
    table_header(&[
        "group",
        "upstream: intended to source",
        "upstream: source to intended",
        "device deviation",
        "device + upstream",
        "end to end: cut to source",
        "end to end: source to cut",
    ]);
    let mut fails = Vec::new();
    for (label, paths) in &g.source {
        let src = source::source_segments(paths);
        let of = |i: usize| g.group[i] == *label && toolpaths[i].kind == ToolpathKind::Cut;
        let intended: Vec<(Point, Point)> = (0..toolpaths.len())
            .filter(|&i| of(i) && toolpaths[i].pass == 1)
            .flat_map(|i| source::polyline_segments(toolpaths[i].lap()).collect::<Vec<_>>())
            .collect();
        let (up_over, up_under) = source::two_sided(&src, &intended);
        let upstream = up_over.max(up_under);
        let dev = device.get(label).map_or(f64::INFINITY, |w| w.deviation);
        let moves = source::cut_moves_where(&toolpaths, &g.result().trace, of);
        let (over, under) = source::two_sided(&src, &moves);
        row(&[
            label.clone(),
            mm(up_over),
            mm(up_under),
            mm(dev),
            mm(dev + upstream),
            mm(over),
            mm(under),
        ]);
        if upstream > upstream_limit {
            fails.push(format!(
                "{label}: upstream {upstream:.5} > {upstream_limit}"
            ));
        }
        if over.max(under) > END_TO_END_LIMIT_MM {
            fails.push(format!(
                "{label}: end to end {:.5} > {END_TO_END_LIMIT_MM}",
                over.max(under)
            ));
        }
        if dev + upstream > END_TO_END_LIMIT_MM {
            fails.push(format!(
                "{label}: device + upstream {:.5} > {END_TO_END_LIMIT_MM}",
                dev + upstream
            ));
        }
    }
    assert!(fails.is_empty(), "STOP: {fails:#?}");
}

/// Host compensation against none at all (a knife on a plotter set to
/// "None", so the tip trails 0.3 mm around every corner): on every corner
/// group the host corner deviation is at most a fifth of the other's.
#[test]
fn host_beats_uncompensated_on_every_corner_pattern() {
    let host = sheet(HOST, 0.3);
    let none = sheet(NONE, 0.3);
    let h = by_group(&simulate_sheet(&host.session, None, SimGrid::Device));
    let n_run = simulate_sheet(&none.session, None, SimGrid::Device);
    assert_eq!(n_run.run.mode, SimMode::Pen);
    let n = by_group(&n_run);
    table_header(&["group", "host corner", "none corner", "ratio"]);
    let mut corner_groups = 0;
    for (label, hw) in &h {
        if !is_corner_group(label) {
            continue;
        }
        corner_groups += 1;
        let nw = n[label];
        row(&[
            label.clone(),
            mm(hw.corner),
            mm(nw.corner),
            format!("{:.3}", hw.corner / nw.corner),
        ]);
        assert!(
            nw.corner > 0.0,
            "{label}: no corner deviation without compensation"
        );
        assert!(
            hw.corner <= nw.corner / 5.0,
            "{label}: host {} vs none {}",
            hw.corner,
            nw.corner
        );
    }
    assert_eq!(
        corner_groups,
        3 + flatcraft_engine::patterns::STAR_ANGLES.len()
    );
}

/// Report only (R4): the sheet planned for 0.3 mm, cut by a blade of
/// 0.27, 0.3 and 0.33 mm (the real d 10 % off either way).
#[test]
fn offset_sensitivity_table() {
    table_header(&[
        "group",
        "real d",
        "float deviation",
        "float corner",
        "float start",
        "device deviation",
        "device corner",
        "pushed (device)",
    ]);
    struct Real {
        d: f64,
        float: BTreeMap<String, Worst>,
        device: BTreeMap<String, Worst>,
        float_kept: f64,
        device_kept: f64,
    }
    let runs: Vec<Real> = [0.27, 0.3, 0.33]
        .into_iter()
        .map(|d| {
            let s = sheet(HOST, d);
            let f = simulate_sheet(&s.session, None, SimGrid::Float);
            let g = simulate_sheet(&s.session, None, SimGrid::Device);
            Real {
                d,
                float: by_group(&f),
                device: by_group(&g),
                float_kept: f.result().metrics.max_into_kept_mm,
                device_kept: g.result().metrics.max_into_kept_mm,
            }
        })
        .collect();
    for label in runs[0].float.keys() {
        for r in &runs {
            let (f, d) = (r.float[label], r.device[label]);
            row(&[
                label.clone(),
                format!("{}", r.d),
                mm(f.deviation),
                mm(f.corner),
                mm(f.start),
                mm(d.deviation),
                mm(d.corner),
                d.pushed.to_string(),
            ]);
        }
    }
    println!();
    for r in &runs {
        println!(
            "real d {}: max into kept {} (float), {} (device)",
            r.d, r.float_kept, r.device_kept
        );
    }
}
