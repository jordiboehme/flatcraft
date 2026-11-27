//! The host blade offset calibration series (`tools.offset_series`): one
//! small test tile per offset, cut in a row, each tile host compensated
//! with its own offset. The person cuts the row and picks the tile that
//! came out best; the simulator ranks the tiles for a blade of the
//! profile's real offset (Plan decision R4: by `max_hausdorff_mm`, not by
//! the score, so there are no ties).
//!
//! Like `job.simulate`, a series takes three steps, so the session lock is
//! never held while the tiles are planned and simulated:
//!
//! 1. [`prepare`], under the lock: the parameters, the send gate, the host
//!    profile check and a snapshot of the document (its objects only say
//!    where the row may go) with the resolved profiles.
//! 2. [`compute`], without the lock: the tiles are planned, the series is
//!    emitted through [`job::emit_plan`] and every tile is simulated.
//! 3. [`finish`], under the lock again: the reply, and with `send` the job
//!    goes through [`job::send_built`] (the gate, the safety check and the
//!    pending confirmation of every send).

use std::fmt::Write;
use std::sync::Arc;

use flatcraft_core::document::{CutOptions, KeepMode, ObjectId, WeedSettings};
use flatcraft_core::transform::Transform;
use flatcraft_device::KnifeCompensation;
use flatcraft_path::{
    AppliedCompensation, Compensation, CutPlan, HostSettings, KeptMaterial, PipelineSettings,
    Toolpath, plan,
};
use flatcraft_sim::{Grade, JobMetrics, SimSettings, measure_kept, simulate};
use kurbo::{Point, Rect, Vec2};
use serde_json::{Value, json};

use crate::cutgeom;
use crate::gate::{self, GateDecision};
use crate::job::{self, JobInput, JobObject, JobResult};
use crate::session::{CommandError, Origin, Session};

/// Side of one test tile, mm.
pub const TILE_MM: f64 = 25.0;

/// Distance from one tile's left edge to the next one's, mm: 5 mm of
/// waste between neighbours.
pub const TILE_PITCH_MM: f64 = 30.0;

/// Fewest and most offsets of one series.
pub const MIN_TILES: usize = 2;
pub const MAX_TILES: usize = 7;

/// Smallest and largest offset a tile may test, mm.
pub const MIN_OFFSET_MM: f64 = 0.05;
pub const MAX_OFFSET_MM: f64 = 1.0;

/// How far the row keeps from every object of the document, mm: the
/// test shape's gap.
pub const GAP_MM: f64 = crate::troubleshoot::TEST_SHAPE_GAP_MM;

/// The job name of a series sent to the plotter.
pub const SERIES_NAME: &str = "offset-series";

/// The refusal for a plotter profile without host compensation.
pub const NEEDS_HOST: &str =
    "The offset series needs host knife compensation in the plotter profile.";

/// The tile's star tips: apex angle (degrees), apex and leg length, mm in
/// the tile, y down. The apex points up.
const TIPS: [(u32, Point, f64); 2] = [
    (90, Point::new(18.5, 2.5), 6.5),
    (30, Point::new(7.0, 14.0), 9.0),
];

/// The tile's circles: centre and radius, mm in the tile.
const CIRCLES: [(Point, f64); 2] = [(Point::new(16.0, 18.0), 0.5), (Point::new(20.5, 18.0), 1.0)];

/// The tile's square: top-left corner and side, mm in the tile.
const SQUARE: (Point, f64) = (Point::new(2.0, 2.0), 10.0);

/// One test tile as SVG text, 25 x 25 mm: a 10 mm square, a 90 and a 30
/// degree star tip and circles of radius 0.5 and 1 mm. Everything stays at
/// least 1.5 mm inside the tile, so a lead-in of the largest offset never
/// reaches the next tile.
pub fn offset_tile_svg() -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{TILE_MM}mm" height="{TILE_MM}mm" viewBox="0 0 {TILE_MM} {TILE_MM}" fill="none" stroke="#000000" stroke-width="0.1">"##
    );
    let (corner, side) = SQUARE;
    let _ = writeln!(
        s,
        r#"<g id="square-10"><rect x="{}" y="{}" width="{side}" height="{side}"/></g>"#,
        corner.x, corner.y
    );
    for (angle, apex, leg) in TIPS {
        let half = f64::from(angle).to_radians() / 2.0;
        let (dx, dy) = (leg * half.sin(), leg * half.cos());
        let _ = writeln!(
            s,
            r#"<g id="star-{angle}"><path d="M{:.6} {:.6} L{:.6} {:.6} L{:.6} {:.6} Z"/></g>"#,
            apex.x,
            apex.y,
            apex.x + dx,
            apex.y + dy,
            apex.x - dx,
            apex.y + dy
        );
    }
    let mut circles = String::new();
    for (c, r) in CIRCLES {
        let _ = write!(circles, r#"<circle cx="{}" cy="{}" r="{r}"/>"#, c.x, c.y);
    }
    let _ = writeln!(s, r#"<g id="circles">{circles}</g>"#);
    s.push_str("</svg>\n");
    s
}

/// The name of the tile for `offset_mm`, as the listing notes show it:
/// the offset rounded to a micrometre, without trailing zeros ("Offset
/// 0.3 mm", also for 0.1 + 0.2).
pub fn tile_name(offset_mm: f64) -> String {
    format!("Offset {} mm", offset_text(offset_mm))
}

/// `mm` rounded to three decimals, without trailing zeros.
fn offset_text(mm: f64) -> String {
    let text = format!("{mm:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Checks the offsets of a series: 2 to 7 of them, each from 0.05 to
/// 1.0 mm.
pub fn check_offsets(offsets_mm: &[f64]) -> Result<(), CommandError> {
    if !(MIN_TILES..=MAX_TILES).contains(&offsets_mm.len()) {
        return Err(CommandError::InvalidParams(format!(
            "offsets_mm needs {MIN_TILES} to {MAX_TILES} offsets, not {}",
            offsets_mm.len()
        )));
    }
    if let Some(bad) = offsets_mm
        .iter()
        .find(|o| !(MIN_OFFSET_MM..=MAX_OFFSET_MM).contains(*o))
    {
        return Err(CommandError::InvalidParams(format!(
            "each offset must be from {MIN_OFFSET_MM} to {MAX_OFFSET_MM} mm, not {bad}"
        )));
    }
    Ok(())
}

/// Everything a series reads, taken under the session lock.
#[derive(Debug, Clone)]
pub struct SeriesInput {
    /// The document's objects (where the row may go), its job settings
    /// and the resolved profiles, with the cached cut geometry as a seed.
    pub input: JobInput,
    pub offsets_mm: Vec<f64>,
}

/// Step 1, under the lock: checks the offsets, the send gate (with
/// `send`, before anything is built, as `job.send` does) and the host
/// profile, and takes the snapshot of the active document.
pub fn prepare(
    s: &Session,
    offsets_mm: &[f64],
    send: bool,
    origin: Origin,
) -> Result<SeriesInput, CommandError> {
    check_offsets(offsets_mm)?;
    let st = s.doc();
    let doc = &st.document;
    let plotter = s
        .profiles
        .plotter(&doc.job.plotter_id)
        .map_err(crate::commands::profile_error)?;
    if send
        && let GateDecision::Refuse(reason) =
            gate::check_send(origin, &plotter.transport, &s.prefs, s.has_ui)
    {
        return Err(CommandError::Refused(reason));
    }
    if !matches!(plotter.knife_compensation, KnifeCompensation::Host { .. }) {
        return Err(CommandError::Refused(NEEDS_HOST.into()));
    }
    let material = s
        .profiles
        .material(&doc.job.material_id)
        .map_err(crate::commands::profile_error)?;
    Ok(SeriesInput {
        input: JobInput::from_parts(doc, plotter, &material).with_seed(st.cached_cut_geometry()),
        offsets_mm: offsets_mm.to_vec(),
    })
}

/// One planned tile.
#[derive(Debug, Clone)]
pub struct Tile {
    pub offset_mm: f64,
    /// The tile's object id in the series' listing notes.
    pub object: ObjectId,
    /// Where the tile lies on the mat (the whole 25 mm tile), mm.
    pub bounds_mm: Rect,
    /// The blade heading the tile's plan starts with: the previous tile's
    /// last heading.
    pub start_heading: Vec2,
    /// The tile's own plan, from the previous tile's end.
    pub plan: CutPlan,
}

/// A planned and emitted series.
#[derive(Debug, Clone)]
pub struct Series {
    /// The whole row as one job: its plan's compensation is
    /// [`AppliedCompensation::HostCalibration`].
    pub job: JobResult,
    pub tiles: Vec<Tile>,
}

/// The refusal when the row does not fit.
fn no_room(size: Vec2) -> CommandError {
    CommandError::Refused(format!(
        "There is no free spot for the offset series: it needs a row of {} x {} with {} around it inside the cutting area. Use fewer offsets, move your design aside or pick a larger mat.",
        crate::report::mm(size.x),
        crate::report::mm(size.y),
        crate::report::mm(GAP_MM)
    ))
}

/// Where the row of `size` goes: the first free spot of `area`, keeping
/// [`GAP_MM`] from every box of `taken`. With feed bands (no mat) the row
/// lies inside one band ([`flatcraft_path::feed::BAND_MM`]), so cutting
/// tile after tile never feeds the sheet back.
fn row_spot(area: Rect, taken: &[Rect], size: Vec2, banded: bool) -> Option<Point> {
    use flatcraft_path::feed::BAND_MM;
    if !banded {
        return crate::placement::free_spot(area, taken, size, GAP_MM);
    }
    let first = (area.y0 / BAND_MM).floor().max(0.0) as usize;
    (first..)
        .map(|k| k as f64 * BAND_MM)
        .take_while(|y| *y < area.y1)
        .find_map(|y| {
            let band = Rect::new(area.x0, area.y0.max(y), area.x1, area.y1.min(y + BAND_MM));
            crate::placement::free_spot(band, taken, size, GAP_MM)
        })
}

/// The tile objects of a series whose row starts at `origin`.
fn tile_objects(offsets_mm: &[f64], origin: Point) -> Result<Vec<JobObject>, CommandError> {
    let svg = offset_tile_svg();
    let (import, paths) = crate::memo::svg(svg.as_bytes(), crate::memo::Fonts::ForDocument)
        .map_err(|e| CommandError::Failed(format!("internal: the offset tile is broken: {e}")))?;
    let frame = import.page_frame_mm;
    Ok(offsets_mm
        .iter()
        .enumerate()
        .map(|(i, &offset)| {
            let id = ObjectId(i as u64 + 1);
            let left = origin + Vec2::new(TILE_PITCH_MM * i as f64, 0.0);
            JobObject {
                id,
                name: tile_name(offset),
                visible: true,
                cut_enabled: true,
                transform: Transform {
                    center_mm: left + frame.center().to_vec2(),
                    ..Transform::default()
                },
                local_bounds: frame,
                cut: CutOptions::default(),
                keep: KeepMode::Pieces,
                geometry: paths.clone(),
                link_group: None,
                unit: id,
            }
        })
        .collect())
}

/// The plan of the whole row: the tiles' toolpaths in order, with the
/// compensation of a calibration series.
fn merge(tiles: &[Tile], area: Rect) -> CutPlan {
    let toolpaths: Vec<Toolpath> = tiles
        .iter()
        .flat_map(|t| t.plan.toolpaths.iter().cloned())
        .collect();
    // The tiles lie apart, so the kept material of the row is that of
    // each tile.
    let kept = KeptMaterial::from_toolpaths(&toolpaths, |_| KeepMode::Pieces, area);
    let (first, last) = (&tiles[0].plan, &tiles[tiles.len() - 1].plan);
    CutPlan {
        toolpaths,
        // Each tile's travel starts where the one before ended.
        travel_mm: tiles.iter().map(|t| t.plan.travel_mm).sum(),
        cut_mm: tiles.iter().map(|t| t.plan.cut_mm).sum(),
        home: first.home,
        end: last.end,
        end_heading: last.end_heading,
        compensation: AppliedCompensation::HostCalibration,
        dropped_short: tiles.iter().map(|t| t.plan.dropped_short).sum(),
        dropped_weed_boxes: 0,
        blocked_weed_boxes: 0,
        kept: Arc::new(kept),
    }
}

/// Plans and emits the series of `input`: the tiles in a row from the
/// first free spot of the cutting area (its top-left when nothing is
/// there), each planned with host compensation at its own offset, from
/// where the tile before ended and with its last heading.
pub fn build(input: &SeriesInput) -> Result<Series, CommandError> {
    check_offsets(&input.offsets_mm)?;
    let job_input = &input.input;
    let plotter = &job_input.plotter;
    if !matches!(plotter.knife_compensation, KnifeCompensation::Host { .. }) {
        return Err(CommandError::Refused(NEEDS_HOST.into()));
    }
    let (media, mat_warning) = crate::commands::effective_media(plotter, &job_input.job)?;
    let area = plotter
        .cutting_area(&media.id)
        .ok_or_else(|| CommandError::Failed("the mat has no cutting area".into()))?;
    let base = job::pipeline_settings(job_input, media, area)?;
    let Compensation::Host(host) = base.compensation else {
        return Err(CommandError::Failed(job::PLAN_MISMATCH.into()));
    };

    let n = input.offsets_mm.len();
    let size = Vec2::new(TILE_PITCH_MM * (n - 1) as f64 + TILE_MM, TILE_MM);
    // Every object counts, hidden and skipped ones too: they may be cut
    // later.
    let taken: Vec<Rect> = job_input
        .objects
        .iter()
        .filter_map(|o| crate::report::object_bounds(job_input, o))
        .collect();
    let origin = row_spot(area, &taken, size, base.monotonic_feed).ok_or_else(|| no_room(size))?;

    let objects = tile_objects(&input.offsets_mm, origin)?;
    let cut = cutgeom::objects_cut_geometry(
        &objects,
        job_input.tolerance_mm(),
        job_input.min_width_mm(),
        None,
    );
    let mut tiles = Vec::with_capacity(n);
    let (mut home, mut heading) = (base.home, base.start_heading);
    for (o, &offset_mm) in objects.iter().zip(&input.offsets_mm) {
        let mut placed = Vec::new();
        job::place_pieces(o.id, cut.pieces(o.id), &mut placed);
        let settings = PipelineSettings {
            compensation: Compensation::Host(HostSettings { offset_mm, ..host }),
            home,
            start_heading: heading,
            // A calibration row is cut on its own: no weeding, every
            // shape a piece.
            weed: WeedSettings::default(),
            weed_units: Default::default(),
            keep_modes: Default::default(),
            ..base.clone()
        };
        let p = plan(&placed, &settings).map_err(|e| CommandError::Refused(e.to_string()))?;
        let left = origin + Vec2::new(TILE_PITCH_MM * tiles.len() as f64, 0.0);
        tiles.push(Tile {
            offset_mm,
            object: o.id,
            bounds_mm: Rect::from_origin_size(left, (TILE_MM, TILE_MM)),
            start_heading: heading,
            plan: p,
        });
        let last = &tiles[tiles.len() - 1].plan;
        (home, heading) = (last.end, last.end_heading);
    }

    let merged = merge(&tiles, area);
    let settings = job::cut_settings(job_input);
    let emitted = job::emit_plan(&merged, &objects, &cut, plotter, &media.id, &settings)?;
    let mut warnings: Vec<String> = mat_warning.into_iter().collect();
    if merged.dropped_short > 0 {
        warnings.push(job::dropped_short_warning(merged.dropped_short));
    }
    let not_cut = job::paths_not_cut(&merged, &emitted.collapsed);
    if not_cut > 0 {
        warnings.push(job::collapsed_warning(not_cut));
    }
    Ok(Series {
        job: JobResult {
            plan: merged,
            job: emitted,
            settings,
            warnings,
            choice_warnings: 0,
            issues: Vec::new(),
            covered_warnings: Vec::new(),
            sent: None,
            built_for: plotter.clone(),
            calibration: Some(crate::cutlog::Calibration::OffsetSeries {
                offsets_mm: input.offsets_mm.clone(),
            }),
            strategy: job_input.job.strategy,
        },
        tiles,
    })
}

/// The simulation of one tile.
#[derive(Debug, Clone)]
pub struct TileSim {
    pub offset_mm: f64,
    pub bounds_mm: Rect,
    /// The device-mode metrics for a blade of the profile's real offset.
    pub metrics: JobMetrics,
}

/// Simulates every tile of `series` on the device grid with the
/// profile's real blade offset, each from where the tile before ended and
/// with the heading its plan started with. Only the toolpaths that are
/// really cut are simulated. The tiles run side by side.
pub fn simulate_tiles(input: &SeriesInput, series: &Series) -> Result<Vec<TileSim>, CommandError> {
    let plotter = &input.input.plotter;
    let (media, _) = crate::commands::effective_media(plotter, &input.input.job)?;
    let grid = crate::simulate::device_grid(plotter, media);
    let d = plotter.blade_offset_mm;
    let mask = job::cut_mask(&series.job);
    let mut first = 0;
    let jobs: Vec<(&Tile, Vec<Toolpath>)> = series
        .tiles
        .iter()
        .map(|t| {
            let n = t.plan.toolpaths.len();
            let cut = t
                .plan
                .toolpaths
                .iter()
                .zip(&mask[first..first + n])
                .filter(|(_, cut)| **cut)
                .map(|(tp, _)| tp.clone())
                .collect();
            first += n;
            (t, cut)
        })
        .collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(t, cut)| {
                scope.spawn(move || {
                    let settings = SimSettings {
                        start_heading: t.start_heading,
                        ..SimSettings::device(d, grid)
                    };
                    let trace = simulate(cut, t.plan.home, &settings)
                        .map_err(crate::simulate::sim_error)?;
                    let (metrics, _) = measure_kept(cut, &trace, &settings, &t.plan.kept);
                    Ok(TileSim {
                        offset_mm: t.offset_mm,
                        bounds_mm: t.bounds_mm,
                        metrics,
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(crate::session::internal_error("tools.offset_series")))
            })
            .collect()
    })
}

/// A planned and simulated series.
#[derive(Debug, Clone)]
pub struct SeriesRun {
    pub series: Series,
    pub sims: Vec<TileSim>,
    /// The profile's real blade offset the tiles were simulated with.
    pub blade_offset_mm: f64,
}

/// Step 2, without the lock: plans, emits and simulates the series.
pub fn compute(input: SeriesInput) -> Result<SeriesRun, CommandError> {
    let series = build(&input)?;
    let sims = simulate_tiles(&input, &series)?;
    Ok(SeriesRun {
        series,
        sims,
        blade_offset_mm: input.input.plotter.blade_offset_mm,
    })
}

/// The series of the active document's plotter with `offsets_mm`, built
/// like [`compute`] builds it (without the simulation): what
/// `tools.offset_series` would cut.
pub fn offset_series(s: &Session, offsets_mm: &[f64]) -> Result<JobResult, CommandError> {
    Ok(build(&prepare(s, offsets_mm, false, Origin::User)?)?.job)
}

/// The tiles of `sims` from best to worst: by `max_hausdorff_mm`, the
/// first of equals first; a tile with nothing measured comes last.
pub fn ranking(sims: &[TileSim]) -> Vec<usize> {
    let key = |i: usize| {
        let h = sims[i].metrics.max_hausdorff_mm;
        if h.is_finite() { h } else { f64::INFINITY }
    };
    let mut order: Vec<usize> = (0..sims.len()).collect();
    order.sort_by(|&a, &b| key(a).total_cmp(&key(b)));
    order
}

/// The name of a grade.
fn grade(g: Grade) -> &'static str {
    crate::simulate::grade_name(g)
}

/// The reply of `tools.offset_series` without the send.
pub fn report_json(run: &SeriesRun) -> Value {
    let order = ranking(&run.sims);
    let mut rank = vec![0; run.sims.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r + 1;
    }
    let tiles: Vec<Value> = run
        .sims
        .iter()
        .zip(&rank)
        .map(|(t, rank)| {
            json!({
                "offset_mm": t.offset_mm,
                "bounds_mm": crate::commands::rect_json(t.bounds_mm),
                "max_hausdorff_mm": t.metrics.max_hausdorff_mm,
                "grade": grade(t.metrics.grade),
                "score": t.metrics.score,
                "rank": rank,
            })
        })
        .collect();
    let sum = job::summary(&run.series.job);
    json!({
        "blade_offset_mm": run.blade_offset_mm,
        "best_offset_mm": order.first().map(|&i| run.sims[i].offset_mm),
        "tiles": tiles,
        "toolpaths": sum.toolpaths,
        "cut_mm": sum.cut_mm,
        "travel_mm": sum.travel_mm,
        "speed": sum.speed,
        "force": sum.force,
        "passes": sum.passes,
        "warnings": sum.warnings,
        "sent": null,
        "waiting_for_confirmation": false,
    })
}

/// Step 3, under the lock: the reply for the active document, and with
/// `send` the series goes through [`job::send_built`] for the document's
/// plotter as it is now (a series built for other settings is never
/// sent).
pub fn finish(
    s: &mut Session,
    run: SeriesRun,
    send: bool,
    origin: Origin,
) -> Result<Value, CommandError> {
    let mut out = report_json(&run);
    if send {
        let plotter = s
            .profiles
            .plotter(&s.doc().document.job.plotter_id)
            .map_err(crate::commands::profile_error)?;
        match job::send_built(s, &plotter, run.series.job, SERIES_NAME.into(), origin) {
            Ok(sent) => out["sent"] = sent,
            // The send waits for the person in the window: the ranking
            // still comes back, with the wait as a field.
            Err(CommandError::Refused(text)) if text == job::WAITING => {
                out["waiting_for_confirmation"] = true.into();
                out["note"] = text.into();
            }
            Err(e) => return Err(e),
        }
    }
    Ok(crate::commands::landed(s, out))
}
