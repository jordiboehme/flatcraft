//! The import report and the reasons a job has nothing to cut.
//!
//! Each [`Issue`] has a stable [`Code`], a [`Severity`], the objects it is
//! about, a plain message (the text the window shows) and its fixes: ready
//! to run registry commands with their arguments. The window's buttons run
//! exactly these commands, and agents get the same issues in the results of
//! `object.import`, `object.import_svg`, `document.check` and
//! `job.preview`. Every fix is idempotent (absolute sizes, positions and
//! settings), so running it again after the document changed does no harm.
//!
//! The thresholds live here only ([`min_feature_mm`], [`gap_limit_mm`]), so
//! a later tuning of the minimum feature size changes every check at once.

use flatcraft_core::document::{CutMode, CutOptions, ObjectId};
use flatcraft_core::geometry::CutPath;
use flatcraft_import::ImportWarning;
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cutgeom::{self, CLOSE_TOLERANCE_MM};
use crate::job::{JobInput, JobObject};

/// What an issue is about. The names are stable: agents match on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    /// Shapes that are not closed, so they are cut as lines and nothing
    /// comes out: filled paths cut as lines, and outlines with a small gap.
    OpenPaths,
    /// Strokes cut around both edges that are thinner than the minimum
    /// feature size: the strip between the two cuts tears.
    Hairline,
    /// Closed shapes smaller than the minimum feature size.
    BelowMinFeature,
    /// Filled paths without area (or thinner than the cut tolerance) that
    /// the cut mode "object outlines" cuts nothing of.
    FillIgnored,
    /// The object fits the cutting area but does not lie inside it.
    OutsideArea,
    /// The object is larger than the cutting area.
    LargerThanArea,
    /// Text whose font is not installed: it was not drawn, so not cut.
    MissingFont,
    /// Clip paths or masks were ignored; the full shapes are cut.
    ClipIgnored,
    /// Embedded images were skipped; only vector paths are cut.
    ImagesSkipped,
    /// Dashed strokes are cut as solid lines.
    DashesIgnored,
    /// The object has no visible paths.
    NoPaths,
    /// Tracing found no shape.
    NothingTraced,
    /// The image was traced at a lower resolution.
    TraceDownscaled,
    /// The dpi in the image file was not plausible and was ignored.
    DpiIgnored,
    /// The trace dropped shapes below the minimum feature size.
    ShapesDropped,
    /// Nothing to cut: the document has no objects.
    EmptyDocument,
    /// Nothing to cut: these objects are hidden.
    Hidden,
    /// Nothing to cut: these objects are skipped.
    Skipped,
    /// Nothing to cut: every cut path is shorter than half a plotter step.
    TooSmall,
}

impl Code {
    /// Every code, in the order the tool description lists them.
    pub const ALL: [Code; 19] = [
        Code::OpenPaths,
        Code::Hairline,
        Code::BelowMinFeature,
        Code::FillIgnored,
        Code::OutsideArea,
        Code::LargerThanArea,
        Code::MissingFont,
        Code::ClipIgnored,
        Code::ImagesSkipped,
        Code::DashesIgnored,
        Code::NoPaths,
        Code::NothingTraced,
        Code::TraceDownscaled,
        Code::DpiIgnored,
        Code::ShapesDropped,
        Code::EmptyDocument,
        Code::Hidden,
        Code::Skipped,
        Code::TooSmall,
    ];

    /// The code as agents see it, like `open_paths`.
    pub fn as_str(self) -> &'static str {
        match self {
            Code::OpenPaths => "open_paths",
            Code::Hairline => "hairline",
            Code::BelowMinFeature => "below_min_feature",
            Code::FillIgnored => "fill_ignored",
            Code::OutsideArea => "outside_area",
            Code::LargerThanArea => "larger_than_area",
            Code::MissingFont => "missing_font",
            Code::ClipIgnored => "clip_ignored",
            Code::ImagesSkipped => "images_skipped",
            Code::DashesIgnored => "dashes_ignored",
            Code::NoPaths => "no_paths",
            Code::NothingTraced => "nothing_traced",
            Code::TraceDownscaled => "trace_downscaled",
            Code::DpiIgnored => "dpi_ignored",
            Code::ShapesDropped => "shapes_dropped",
            Code::EmptyDocument => "empty_document",
            Code::Hidden => "hidden",
            Code::Skipped => "skipped",
            Code::TooSmall => "too_small",
        }
    }

    /// A short name for people, for the title of a hover card.
    pub fn title(self) -> &'static str {
        match self {
            Code::OpenPaths => "Open shapes",
            Code::Hairline => "Thin double cuts",
            Code::BelowMinFeature => "Tiny shapes",
            Code::FillIgnored => "Fills that are not cut",
            Code::OutsideArea => "Off the mat",
            Code::LargerThanArea => "Bigger than the mat",
            Code::MissingFont => "Missing font",
            Code::ClipIgnored => "Clip or mask ignored",
            Code::ImagesSkipped => "Pictures skipped",
            Code::DashesIgnored => "Dashes cut solid",
            Code::NoPaths => "No paths",
            Code::NothingTraced => "Nothing traced",
            Code::TraceDownscaled => "Traced at a lower resolution",
            Code::DpiIgnored => "Odd dpi ignored",
            Code::ShapesDropped => "Specks dropped",
            Code::EmptyDocument => "Empty document",
            Code::Hidden => "Hidden objects",
            Code::Skipped => "Skipped objects",
            Code::TooSmall => "Too tiny to cut",
        }
    }

    /// True if the import commands find it in the file itself, so
    /// `document.check` cannot see it again later.
    pub fn import_only(self) -> bool {
        matches!(
            self,
            Code::MissingFont
                | Code::ClipIgnored
                | Code::ImagesSkipped
                | Code::DashesIgnored
                | Code::NoPaths
                | Code::NothingTraced
                | Code::TraceDownscaled
                | Code::DpiIgnored
                | Code::ShapesDropped
        )
    }
}

/// How bad an issue is. `error`: the job cannot be cut like this.
/// `warning`: it cuts, but probably not as the person wants. `info`: worth
/// knowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// One fix: a registry command with its arguments. The window's button
/// runs exactly this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fix {
    /// The button text, like "Move onto the mat".
    pub label: String,
    /// What the fix does, for the button's hover card.
    pub description: String,
    /// The registry command, like `object.move`.
    pub command: String,
    /// Its arguments, ready to pass as `params`.
    pub args: Value,
}

/// One line of a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub code: Code,
    pub severity: Severity,
    /// The objects it is about (empty for the document as a whole).
    pub object_ids: Vec<u64>,
    /// How many paths or shapes it is about (0 when it does not count).
    pub paths: usize,
    /// What is wrong, in plain words: the text the window shows.
    pub message: String,
    /// What to do about it, in plain words; with no fix, how to fix it by
    /// hand.
    pub hint: String,
    /// Ready-to-run fixes; empty when there is no automatic fix.
    pub fixes: Vec<Fix>,
}

/// The report of an import or a check: how many shapes the objects cut
/// and what is worth knowing about them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Cut shapes (closed or open lines) of the objects, as the job cuts them.
    pub shapes: usize,
    pub issues: Vec<Issue>,
}

impl Report {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Adds another report's shapes and issues.
    pub fn extend(&mut self, other: Report) {
        self.shapes += other.shapes;
        self.issues.extend(other.issues);
    }
}

/// Issues as JSON, for command results.
pub fn issues_json(issues: &[Issue]) -> Value {
    serde_json::to_value(issues).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------- thresholds

/// The smallest shape the knife cuts cleanly, mm: the tracer's default
/// minimum feature size for this blade offset. Every size check of the
/// report uses it.
pub fn min_feature_mm(blade_offset_mm: f64) -> f64 {
    let offset = if blade_offset_mm.is_finite() && blade_offset_mm > 0.0 {
        blade_offset_mm
    } else {
        0.25
    };
    flatcraft_import::trace::MIN_FEATURE_BLADE_OFFSETS * offset
}

/// An open outline whose ends are closer than this (and farther apart than
/// the join tolerance) has a gap: the piece hangs on a thin bridge.
pub fn gap_limit_mm(min_feature: f64) -> f64 {
    min_feature
}

/// Margin a "Scale to fit" leaves inside the cutting area, mm, so the
/// stroke outlines (measured to 0.01 mm) never touch over its edge.
const FIT_MARGIN_MM: f64 = 0.05;

/// A shape scaled up to the minimum feature size gets this much more, so
/// rounding never leaves it just below.
const SCALE_UP_SLACK: f64 = 1.02;

// ---------------------------------------------------------------- text

/// A length for a message: "254 mm", "1.5 mm", "0.25 mm".
pub fn mm(v: f64) -> String {
    let text = if v.abs() >= 10.0 {
        format!("{v:.0}")
    } else if v.abs() >= 1.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    };
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    };
    format!("{text} mm")
}

/// "1 shape" or "3 shapes".
fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn quoted(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    format!("'{name}'")
}

/// "'a'", "'a' and 'b'", "'a', 'b' and 'c'", "'a', 'b' and 3 more".
fn names(list: &[&str]) -> String {
    let q: Vec<String> = list.iter().map(|n| quoted(n)).collect();
    match q.len() {
        0 => String::new(),
        1 => q[0].clone(),
        2 | 3 => format!("{} and {}", q[..q.len() - 1].join(", "), q[q.len() - 1]),
        n => format!("{} and {} more", q[..2].join(", "), n - 2),
    }
}

/// "A sentence." from a lower-case phrase.
fn sentence(text: &str) -> String {
    let mut c = text.chars();
    let mut out: String = match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    };
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

fn ids_value(ids: &[u64]) -> Value {
    Value::Array(ids.iter().map(|i| (*i).into()).collect())
}

// ---------------------------------------------------------------- import-time issues

/// The issues an SVG import found in the file itself. Their messages are
/// the import warnings (`warning_text`), as sentences.
pub fn svg_issues(id: ObjectId, warnings: &[ImportWarning]) -> Vec<Issue> {
    warnings
        .iter()
        .map(|w| {
            let message = sentence(&crate::commands::object::warning_text(w));
            let (code, severity, paths, hint) = match w {
                ImportWarning::TextSkipped => (
                    Code::MissingFont,
                    Severity::Warning,
                    0,
                    "Install the font, restart FlatCraft and reload the file, or turn the text into paths in your drawing app (in Inkscape: Path, Object to Path) and import it again.",
                ),
                ImportWarning::ClipOrMaskIgnored { count } => (
                    Code::ClipIgnored,
                    Severity::Info,
                    *count,
                    "A clip or mask only hides part of a shape on screen; the knife does not know about it. Apply the clip in your drawing app (in Inkscape: Path, Intersection) if only the visible part should be cut.",
                ),
                ImportWarning::ImagesSkipped { count } => (
                    Code::ImagesSkipped,
                    Severity::Info,
                    *count,
                    "Pictures inside an SVG are not traced. Import the picture file on its own to trace it into outlines.",
                ),
                ImportWarning::DashesIgnored { count } => (
                    Code::DashesIgnored,
                    Severity::Info,
                    *count,
                    "A knife cannot skip along a dash pattern yet. Turn the dashes into separate paths in your drawing app if you want a perforated line.",
                ),
                ImportWarning::NoPaths => (
                    Code::NoPaths,
                    Severity::Warning,
                    0,
                    "Check that the file has visible shapes (not only text without its font or pictures).",
                ),
            };
            Issue {
                code,
                severity,
                object_ids: vec![id.0],
                paths,
                message,
                hint: hint.into(),
                fixes: Vec::new(),
            }
        })
        .collect()
}

/// One finding of a trace (`code` is one of `nothing_traced`,
/// `trace_downscaled`, `dpi_ignored` and `shapes_dropped`) with the
/// trace warning `text` as its message.
pub fn trace_issue(id: ObjectId, code: Code, paths: usize, text: &str) -> Issue {
    let (severity, hint) = match code {
        Code::NothingTraced => (
            Severity::Warning,
            "Open Trace and try another threshold, or invert to cut the light parts.",
        ),
        Code::TraceDownscaled => (
            Severity::Info,
            "Fine details may be a little softer. Use a smaller image if they matter.",
        ),
        Code::DpiIgnored => (
            Severity::Info,
            "Check the size on the mat. Open Trace and set the dpi if it is wrong.",
        ),
        _ => (
            Severity::Info,
            "They would tear rather than cut. Open Trace and lower the minimum feature size to keep them.",
        ),
    };
    Issue {
        code,
        severity,
        object_ids: vec![id.0],
        paths,
        message: sentence(text),
        hint: hint.into(),
        fixes: Vec::new(),
    }
}

// ---------------------------------------------------------------- geometry checks

/// The measured shapes of one object.
#[derive(Debug, Clone, Default)]
struct Shapes {
    total: usize,
    /// Open shapes that should be closed (see `Code::OpenPaths`), and how
    /// many of them have a gap (object outlines do not close those).
    open: usize,
    gaps: usize,
    /// Closed shapes and the larger side of the smallest one.
    closed: usize,
    smallest: Option<f64>,
    small: usize,
    /// Strokes cut around both edges below the minimum feature size, with
    /// the thinnest and widest such stroke.
    hairline: usize,
    hairline_widths: (f64, f64),
    /// Filled paths that give nothing to cut.
    no_area: usize,
}

/// One subpath of a cut path: whether it is closed, the distance of its
/// ends and its bounding box.
struct Sub {
    closed: bool,
    gap: f64,
    bounds: Rect,
}

fn subs(path: &BezPath) -> Vec<Sub> {
    let mut out = Vec::new();
    let mut cur: Option<(Point, Point, BezPath, bool)> = None;
    let finish = |cur: Option<(Point, Point, BezPath, bool)>, out: &mut Vec<Sub>| {
        if let Some((start, end, p, closed)) = cur
            && p.elements().len() > 1
        {
            let gap = end.distance(start);
            out.push(Sub {
                closed: closed || gap < CLOSE_TOLERANCE_MM,
                gap,
                bounds: p.bounding_box(),
            });
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                finish(cur.take(), &mut out);
                let mut b = BezPath::new();
                b.move_to(p);
                cur = Some((p, p, b, false));
            }
            PathEl::ClosePath => {
                if let Some(c) = cur.as_mut() {
                    c.3 = true;
                }
            }
            other => {
                if let Some(c) = cur.as_mut() {
                    c.2.push(other);
                    if let Some(end) = other.end_point() {
                        c.1 = end;
                    }
                }
            }
        }
    }
    finish(cur, &mut out);
    out
}

/// True if a source path gives nothing to cut although it has a shape: a
/// fill without stroke whose area is empty or thinner than the tolerance.
/// The job build counts the same paths for its warning.
pub(crate) fn fill_without_area(p: &CutPath, cut: &[BezPath]) -> bool {
    cut.is_empty() && p.paint.stroke.is_none() && !p.path.elements().is_empty()
}

/// Counts the open shapes that should be closed: open cut lines of a
/// filled path with area (`filled`: all lines cuts it as drawn, object
/// outlines would cut the edge of its fill), and open lines with a small
/// gap between their ends. Returns (filled, gaps).
fn open_count(cut: &[BezPath], filled: bool, gap_limit: f64) -> (usize, usize) {
    let mut out = (0, 0);
    for s in cut.iter().flat_map(subs).filter(|s| !s.closed) {
        if filled {
            out.0 += 1;
        } else if s.gap < gap_limit && s.bounds.width().max(s.bounds.height()) > gap_limit {
            out.1 += 1;
        }
    }
    out
}

fn measure(
    o: &JobObject,
    options: &CutOptions,
    affine: Affine,
    tolerance: f64,
    min_feature: f64,
) -> Shapes {
    let mut m = Shapes {
        hairline_widths: (f64::INFINITY, 0.0),
        ..Shapes::default()
    };
    let gap_limit = gap_limit_mm(min_feature);
    let outlines = CutOptions {
        mode: CutMode::ObjectOutlines,
        ..*options
    };
    for p in o.geometry.iter() {
        let cut = cutgeom::cut_path(p, options, affine, tolerance);
        if fill_without_area(p, &cut) {
            m.no_area += 1;
        }
        // A filled path cut as drawn should be closed only if its fill
        // has area: a filled straight line is a line.
        let filled = options.mode == CutMode::AllLines
            && p.paint.fill.is_some()
            && !cutgeom::cut_path(p, &outlines, affine, tolerance).is_empty();
        let (open, gaps) = open_count(&cut, filled, gap_limit);
        m.open += open + gaps;
        m.gaps += gaps;
        for s in cut.iter().flat_map(subs) {
            m.total += 1;
            if s.closed {
                let side = s.bounds.width().max(s.bounds.height());
                m.closed += 1;
                m.smallest = Some(m.smallest.map_or(side, |a: f64| a.min(side)));
                if side < min_feature {
                    m.small += 1;
                }
            }
        }
        if let Some(w) = cutgeom::outline_width(p.paint, options, affine)
            && w < min_feature
        {
            m.hairline += 1;
            m.hairline_widths.0 = m.hairline_widths.0.min(w);
            m.hairline_widths.1 = m.hairline_widths.1.max(w);
        }
    }
    m
}

/// The cut bounds of an object in mat mm, as `placed_bounds` measures them.
pub fn object_bounds(o: &JobObject) -> Option<Rect> {
    let affine = o.transform.affine(o.local_bounds);
    cutgeom::cut_bounds_of(&o.geometry, &o.cut, affine)
}

/// What the report checks with: the cutting area, the flatten tolerance
/// and the minimum feature size of the job's plotter and mat.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub area: Option<Rect>,
    pub tolerance_mm: f64,
    pub min_feature_mm: f64,
}

impl Limits {
    pub fn of(input: &JobInput) -> Self {
        let area = crate::commands::effective_media(&input.plotter, &input.job)
            .ok()
            .and_then(|(m, _)| input.plotter.cutting_area(&m.id));
        Self {
            area,
            tolerance_mm: input.plotter.flatten_tolerance_mm,
            min_feature_mm: min_feature_mm(input.plotter.blade_offset_mm),
        }
    }
}

/// The geometry checks of the objects `ids` (every object when `None`):
/// open paths, hairlines, shapes below the minimum feature size, fills the
/// cut mode ignores, and objects outside or larger than the cutting area.
/// Hidden and skipped objects are checked too: they may be cut later.
/// Linked copies share their cut options and scale, so an issue of those
/// names every member once.
pub fn check(input: &JobInput, ids: Option<&[ObjectId]>) -> Report {
    let limits = Limits::of(input);
    let mut report = Report::default();
    let mut seen: Vec<(Code, Vec<u64>)> = Vec::new();
    for o in &input.objects {
        if ids.is_some_and(|ids| !ids.contains(&o.id)) {
            continue;
        }
        let r = check_object(input, o, &limits);
        report.shapes += r.shapes;
        for i in r.issues {
            if i.object_ids.len() > 1 {
                let key = (i.code, i.object_ids.clone());
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
            }
            report.issues.push(i);
        }
    }
    report
}

/// The ids of `o` and its linked copies (only `o` when it is not linked),
/// in document order: a fix of shared settings changes all of them.
fn members(input: &JobInput, o: &JobObject) -> Vec<u64> {
    match o.link_group {
        None => vec![o.id.0],
        Some(g) => input
            .objects
            .iter()
            .filter(|x| x.link_group == Some(g))
            .map(|x| x.id.0)
            .collect(),
    }
}

/// The end of a fix description when the fix changes linked copies too.
fn shared_note(members: &[u64]) -> &'static str {
    if members.len() > 1 {
        " This object and its linked copies change together."
    } else {
        ""
    }
}

fn with_mode(o: &JobObject, mode: CutMode) -> CutOptions {
    CutOptions { mode, ..o.cut }
}

fn check_object(input: &JobInput, o: &JobObject, limits: &Limits) -> Report {
    let affine = o.transform.affine(o.local_bounds);
    let measure_as = |options: &CutOptions| {
        measure(
            o,
            options,
            affine,
            limits.tolerance_mm,
            limits.min_feature_mm,
        )
    };
    let m = measure_as(&o.cut);
    let shared = members(input, o);
    let mut issues = Vec::new();
    let name = quoted(&o.name);
    let id = o.id.0;
    if m.open > 0 {
        // In all lines a filled path is cut as drawn; object outlines cut
        // the edge of its fill, which is closed. They do not close a gap.
        // The fix is withheld when object outlines would then cut nothing
        // of a filled line of the object (its fix would switch back).
        let filled = m.open - m.gaps;
        let outlines_ok = o.cut.mode == CutMode::AllLines
            && filled > 0
            && measure_as(&with_mode(o, CutMode::ObjectOutlines)).no_area == 0;
        let fixes = if outlines_ok {
            let rest = if m.gaps > 0 {
                format!(
                    " {} with a gap stay open; close them in your drawing app.",
                    count(m.gaps, "shape", "shapes")
                )
            } else {
                String::new()
            };
            vec![Fix {
                label: "Use object outlines".into(),
                description: format!(
                    "Cuts around the edge of every filled shape instead of along the lines as drawn, so each filled shape is closed and comes out.{rest}{}",
                    shared_note(&shared)
                ),
                command: "object.set_cut_options".into(),
                args: json!({ "ids": [id], "mode": "object_outlines" }),
            }]
        } else {
            Vec::new()
        };
        let what = count(m.open, "shape is", "shapes are");
        let hint = if outlines_ok && m.gaps == 0 {
            "Object outlines cut around the filled shapes, which closes them.".to_string()
        } else if o.cut.mode == CutMode::AllLines && filled > 0 && !outlines_ok {
            "Object outlines would close the filled shapes, but then cut nothing of the filled lines in this object. Close the shapes in your drawing app, or move the lines to their own object.".to_string()
        } else {
            "Close the gaps in your drawing app (in Inkscape: select both end nodes and join them), then reload the file.".to_string()
        };
        issues.push(Issue {
            code: Code::OpenPaths,
            severity: Severity::Warning,
            object_ids: if outlines_ok { shared.clone() } else { vec![id] },
            paths: m.open,
            message: format!(
                "{name}: {what} not closed, so the knife cuts a line there and the piece stays on the sheet."
            ),
            hint,
            fixes,
        });
    }
    if m.hairline > 0 {
        let (lo, hi) = m.hairline_widths;
        let widths = if mm(lo) == mm(hi) {
            mm(lo)
        } else {
            format!("{} to {}", mm(lo).trim_end_matches(" mm"), mm(hi))
        };
        let what = count(m.hairline, "line is", "lines are");
        issues.push(Issue {
            code: Code::Hairline,
            severity: Severity::Warning,
            object_ids: shared.clone(),
            paths: m.hairline,
            message: format!(
                "{name}: {what} cut along both edges but only {widths} wide. A strip thinner than {} tears.",
                mm(limits.min_feature_mm)
            ),
            hint: "Cut thin lines once, along their middle.".into(),
            fixes: vec![Fix {
                label: "Cut as single lines".into(),
                description: format!(
                    "Cuts every stroke of this object once, along its middle, instead of around both edges.{}",
                    shared_note(&shared)
                ),
                command: "object.set_cut_options".into(),
                args: json!({ "ids": [id], "strokes": "centre_line" }),
            }],
        });
    }
    let bounds = object_bounds(o);
    if m.small > 0 {
        let all_small = m.small == m.closed;
        let fix = match (all_small, m.smallest, bounds) {
            (true, Some(smallest), Some(b)) if smallest > 0.0 => scale_up_fix(
                id,
                b,
                limits.min_feature_mm / smallest,
                limits.area,
                &shared,
            ),
            _ => None,
        };
        let what = count(m.small, "shape is", "shapes are");
        issues.push(Issue {
            code: Code::BelowMinFeature,
            severity: Severity::Warning,
            object_ids: if fix.is_some() { shared.clone() } else { vec![id] },
            paths: m.small,
            message: format!(
                "{name}: {what} smaller than {}, the smallest shape this blade cuts cleanly. They may tear or not come off.",
                mm(limits.min_feature_mm)
            ),
            hint: if fix.is_some() {
                "Scale the object up until its smallest shape is big enough.".into()
            } else {
                "Scale the object up, or remove the tiny shapes in your drawing app.".into()
            },
            fixes: fix.into_iter().collect(),
        });
    }
    if m.no_area > 0 {
        issues.push(fill_ignored(
            input,
            o,
            m.no_area,
            limits.tolerance_mm,
            Severity::Warning,
        ));
    }
    if let (Some(area), Some(b)) = (limits.area, bounds) {
        issues.extend(area_issue(o, b, area, &shared));
    }
    Report {
        shapes: m.total,
        issues,
    }
}

/// `object.scale` to a size `factor` times the object's, by its larger
/// side, as an absolute size (so running it twice does no more). `None`
/// when it would not grow it, or when the result would not fit the
/// cutting area (then "Scale to fit" would shrink it back).
fn scale_up_fix(id: u64, b: Rect, factor: f64, area: Option<Rect>, shared: &[u64]) -> Option<Fix> {
    let factor = factor * SCALE_UP_SLACK;
    if !factor.is_finite() || factor <= 1.0 {
        return None;
    }
    if let Some(area) = area
        && (b.width() * factor > area.width() || b.height() * factor > area.height())
    {
        return None;
    }
    let args = if b.width() >= b.height() {
        json!({ "ids": [id], "width_mm": b.width() * factor })
    } else {
        json!({ "ids": [id], "height_mm": b.height() * factor })
    };
    Some(Fix {
        label: "Scale up".into(),
        description: format!(
            "Scales the object {} times, keeping its proportions and its centre, so its smallest shape reaches the minimum feature size.{}",
            round_factor(factor),
            shared_note(shared)
        ),
        command: "object.scale".into(),
        args,
    })
}

fn round_factor(f: f64) -> String {
    if f >= 10.0 {
        format!("{f:.0}")
    } else {
        format!("{f:.1}")
    }
}

/// The `fill_ignored` issue of an object the job does cut.
pub(crate) fn fill_ignored_issue(
    input: &JobInput,
    o: &JobObject,
    n: usize,
    tolerance: f64,
) -> Option<Issue> {
    (n > 0).then(|| fill_ignored(input, o, n, tolerance, Severity::Warning))
}

/// The `fill_ignored` issue. Its fix (all lines) is withheld when all
/// lines would cut filled shapes of the object as open lines (the
/// `open_paths` fix would switch back).
fn fill_ignored(
    input: &JobInput,
    o: &JobObject,
    n: usize,
    tolerance: f64,
    severity: Severity,
) -> Issue {
    let what = count(
        n,
        "filled shape has no area or is",
        "filled shapes have no area or are",
    );
    let them = if n == 1 { "it" } else { "them" };
    let limits = Limits::of(input);
    let affine = o.transform.affine(o.local_bounds);
    let as_lines = measure(
        o,
        &with_mode(o, CutMode::AllLines),
        affine,
        tolerance,
        limits.min_feature_mm,
    );
    let lines_ok = as_lines.open == as_lines.gaps;
    let shared = members(input, o);
    Issue {
        code: Code::FillIgnored,
        severity,
        object_ids: if lines_ok {
            shared.clone()
        } else {
            vec![o.id.0]
        },
        paths: n,
        message: format!(
            "{}: {what} thinner than {}, so the cut mode Object outlines cuts nothing of {them}.",
            quoted(&o.name),
            mm(tolerance)
        ),
        hint: if lines_ok {
            "The cut mode All lines cuts every path as drawn, these too.".into()
        } else {
            "All lines would cut these, but cut the filled shapes of this object as open lines. Draw these lines as strokes, or move them to their own object.".into()
        },
        fixes: if lines_ok {
            vec![Fix {
                label: "Cut all lines".into(),
                description: format!(
                    "Switches this object to the cut mode All lines: every path is cut as drawn, filled or not.{}",
                    shared_note(&shared)
                ),
                command: "object.set_cut_options".into(),
                args: json!({ "ids": [o.id.0], "mode": "all_lines" }),
            }]
        } else {
            Vec::new()
        },
    }
}

/// The issue of an object with bounds `b` that is larger than `area` or
/// does not lie inside it.
fn area_issue(o: &JobObject, b: Rect, area: Rect, shared: &[u64]) -> Option<Issue> {
    // The slack of `inside` on one edge: anything not larger than this
    // fits, and clamped to the left (top) edge it passes `inside`.
    // Anything larger gets "Scale to fit".
    const EPS: f64 = crate::commands::AREA_SLACK_MM;
    let name = quoted(&o.name);
    let id = o.id.0;
    if b.width() > area.width() + EPS || b.height() > area.height() + EPS {
        let wf = (area.width() - FIT_MARGIN_MM) / b.width();
        let hf = (area.height() - FIT_MARGIN_MM) / b.height();
        let f = wf.min(hf);
        let mut args = if wf <= hf {
            json!({ "ids": [id], "width_mm": b.width() * f })
        } else {
            json!({ "ids": [id], "height_mm": b.height() * f })
        };
        let anchor = fit_anchor(b, f, area);
        args["anchor"] = json!(anchor.0);
        let and_move = if anchor.1 {
            ""
        } else {
            " If it then sticks out, Move onto the mat puts it on."
        };
        return Some(Issue {
            code: Code::LargerThanArea,
            severity: Severity::Error,
            object_ids: shared.to_vec(),
            paths: 0,
            message: format!(
                "{name} is {} x {}, larger than the cutting area ({} x {}).",
                mm(b.width()).trim_end_matches(" mm"),
                mm(b.height()),
                mm(area.width()).trim_end_matches(" mm"),
                mm(area.height())
            ),
            hint: format!(
                "Scale it down to fit, or pick a bigger mat in the job settings.{and_move}"
            ),
            fixes: vec![Fix {
                label: "Scale to fit the mat".into(),
                description: format!(
                    "Scales the object down, keeping its proportions, until it fits the cutting area.{}",
                    shared_note(shared)
                ),
                command: "object.scale".into(),
                args,
            }],
        });
    }
    if crate::commands::inside(b, area) {
        return None;
    }
    // A box up to EPS wider than the area goes to its left (top) edge;
    // the upper bound never falls below the lower one.
    let x = clamp_in(b.x0, b.width(), area.x0, area.x1);
    let y = clamp_in(b.y0, b.height(), area.y0, area.y1);
    let overlaps = b.intersect(area).area() > 0.0;
    let message = if overlaps {
        format!("{name} sticks out of the cutting area.")
    } else {
        format!("{name} is outside the cutting area.")
    };
    Some(Issue {
        code: Code::OutsideArea,
        severity: Severity::Error,
        object_ids: vec![id],
        paths: 0,
        message,
        hint: "The knife only reaches the cutting area. Move it onto the mat.".into(),
        fixes: vec![Fix {
            label: "Move onto the mat".into(),
            description: "Moves the object the shortest way into the cutting area. Its size stays."
                .into(),
            command: "object.move".into(),
            args: json!({ "ids": [id], "x_mm": x, "y_mm": y }),
        }],
    })
}

/// The start of a span of length `len` at `at`, moved the shortest way
/// into `lo..hi`. A span up to the slack longer than the range is centred
/// on it, so it reaches over each end by at most half the slack. The
/// bounds never cross.
fn clamp_in(at: f64, len: f64, lo: f64, hi: f64) -> f64 {
    if len >= hi - lo {
        lo - (len - (hi - lo)) / 2.0
    } else {
        at.clamp(lo, hi - len)
    }
}

/// The anchor for a "Scale to fit": the first one about which the scaled
/// box lies inside the area, and whether one did.
fn fit_anchor(b: Rect, f: f64, area: Rect) -> (&'static str, bool) {
    let anchors: [(&str, Point); 5] = [
        ("top_left", Point::new(b.x0, b.y0)),
        ("top_right", Point::new(b.x1, b.y0)),
        ("bottom_left", Point::new(b.x0, b.y1)),
        ("bottom_right", Point::new(b.x1, b.y1)),
        ("center", b.center()),
    ];
    for (name, p) in anchors {
        let scaled = Rect::new(
            p.x + (b.x0 - p.x) * f,
            p.y + (b.y0 - p.y) * f,
            p.x + (b.x1 - p.x) * f,
            p.y + (b.y1 - p.y) * f,
        );
        if crate::commands::inside(scaled, area) {
            return (name, true);
        }
    }
    ("center", false)
}

// ---------------------------------------------------------------- nothing to cut

/// What the job build saw of each object, for the reasons a job has
/// nothing to cut.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildFacts {
    pub objects: usize,
    pub hidden: Vec<ObjectId>,
    pub skipped: Vec<ObjectId>,
    /// Cut objects without any path to cut, with how many of their
    /// filled paths have no area.
    pub nothing_placed: Vec<(ObjectId, usize)>,
    /// Cut objects that gave cut paths.
    pub placed: Vec<ObjectId>,
}

/// Every reason the job has nothing to cut, each with its fixes.
/// `too_short` is true when the planner dropped every path as shorter
/// than half a plotter step.
pub fn nothing_to_cut(input: &JobInput, facts: &BuildFacts, too_short: bool) -> Vec<Issue> {
    let limits = Limits::of(input);
    let name_of = |id: ObjectId| -> &str {
        input
            .objects
            .iter()
            .find(|o| o.id == id)
            .map_or("", |o| o.name.as_str())
    };
    let mut out = Vec::new();
    if facts.objects == 0 {
        out.push(Issue {
            code: Code::EmptyDocument,
            severity: Severity::Error,
            object_ids: Vec::new(),
            paths: 0,
            message: "The document is empty, so there is nothing to cut.".into(),
            hint: "Import an SVG or an image (Import in the ribbon, or drop a file on the window), or make test patterns.".into(),
            fixes: Vec::new(),
        });
        return out;
    }
    if !facts.hidden.is_empty() {
        let ids: Vec<u64> = facts.hidden.iter().map(|i| i.0).collect();
        let list: Vec<&str> = facts.hidden.iter().map(|i| name_of(*i)).collect();
        let (verb, them) = if ids.len() == 1 {
            ("is", "it")
        } else {
            ("are", "them")
        };
        out.push(Issue {
            code: Code::Hidden,
            severity: Severity::Error,
            object_ids: ids.clone(),
            paths: 0,
            message: format!(
                "{} {verb} hidden, and hidden objects are not cut.",
                names(&list)
            ),
            hint: format!("Show {them} again to cut {them}."),
            fixes: vec![Fix {
                label: if ids.len() == 1 {
                    "Show it".into()
                } else {
                    "Show them".into()
                },
                description: "Makes the hidden objects visible again, so they are cut.".into(),
                command: "object.set_visible".into(),
                args: json!({ "ids": ids_value(&ids), "visible": true }),
            }],
        });
    }
    if !facts.skipped.is_empty() {
        let ids: Vec<u64> = facts.skipped.iter().map(|i| i.0).collect();
        let list: Vec<&str> = facts.skipped.iter().map(|i| name_of(*i)).collect();
        let (verb, them) = if ids.len() == 1 {
            ("is", "it")
        } else {
            ("are", "them")
        };
        out.push(Issue {
            code: Code::Skipped,
            severity: Severity::Error,
            object_ids: ids.clone(),
            paths: 0,
            message: format!(
                "{} {verb} set to skip, so the knife leaves {them} out.",
                names(&list)
            ),
            hint: format!("Turn cutting back on for {them}."),
            fixes: vec![Fix {
                label: if ids.len() == 1 {
                    "Cut it".into()
                } else {
                    "Cut them".into()
                },
                description: "Turns cutting back on for the skipped objects.".into(),
                command: "object.set_cut".into(),
                args: json!({ "ids": ids_value(&ids), "cut": true }),
            }],
        });
    }
    for (id, no_area) in &facts.nothing_placed {
        let Some(o) = input.objects.iter().find(|o| o.id == *id) else {
            continue;
        };
        if *no_area > 0 {
            out.push(fill_ignored(
                input,
                o,
                *no_area,
                limits.tolerance_mm,
                Severity::Error,
            ));
        } else {
            let raster = o.geometry.is_empty();
            out.push(Issue {
                code: Code::NoPaths,
                severity: Severity::Error,
                object_ids: vec![id.0],
                paths: 0,
                message: format!("{} has no paths to cut.", quoted(&o.name)),
                hint: if raster {
                    "If it is a traced image, open Trace and try another threshold, or invert it."
                        .into()
                } else {
                    "Check the file in your drawing app: it needs visible shapes or lines.".into()
                },
                fixes: Vec::new(),
            });
        }
    }
    if too_short {
        for id in &facts.placed {
            let Some(o) = input.objects.iter().find(|o| o.id == *id) else {
                continue;
            };
            let affine = o.transform.affine(o.local_bounds);
            let fix = object_bounds(o).and_then(|b| {
                let side = b.width().max(b.height());
                (side > 0.0)
                    .then(|| {
                        scale_up_fix(
                            id.0,
                            b,
                            limits.min_feature_mm / side,
                            limits.area,
                            &members(input, o),
                        )
                    })
                    .flatten()
            });
            let n = measure(
                o,
                &o.cut,
                affine,
                limits.tolerance_mm,
                limits.min_feature_mm,
            )
            .total;
            out.push(Issue {
                code: Code::TooSmall,
                severity: Severity::Error,
                object_ids: if fix.is_some() {
                    members(input, o)
                } else {
                    vec![id.0]
                },
                paths: n,
                message: format!(
                    "{} is so tiny that the plotter would not move: every cut is shorter than half a plotter step.",
                    quoted(&o.name)
                ),
                hint: "Scale it up. Was it drawn in other units?".into(),
                fixes: fix.into_iter().collect(),
            });
        }
    }
    out
}

/// The issues of objects the planner found outside the cutting area.
pub fn out_of_area(input: &JobInput, ids: &[ObjectId]) -> Vec<Issue> {
    let limits = Limits::of(input);
    let Some(area) = limits.area else {
        return Vec::new();
    };
    let mut out: Vec<Issue> = Vec::new();
    for o in input.objects.iter().filter(|o| ids.contains(&o.id)) {
        let Some(i) = object_bounds(o).and_then(|b| area_issue(o, b, area, &members(input, o)))
        else {
            continue;
        };
        // Linked copies share their scale: one "Scale to fit" per group,
        // as `check` gives it.
        let dup = i.object_ids.len() > 1
            && out
                .iter()
                .any(|x| x.code == i.code && x.object_ids == i.object_ids);
        if !dup {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_are_rounded_for_people() {
        assert_eq!(mm(254.0), "254 mm");
        assert_eq!(mm(1.5), "1.5 mm");
        assert_eq!(mm(1.0), "1 mm");
        assert_eq!(mm(0.25), "0.25 mm");
        assert_eq!(mm(0.01), "0.01 mm");
        assert_eq!(mm(12.46), "12 mm");
    }

    #[test]
    fn names_are_listed_shortly() {
        assert_eq!(names(&["a"]), "'a'");
        assert_eq!(names(&["a", "b"]), "'a' and 'b'");
        assert_eq!(names(&["a", "b", "c"]), "'a', 'b' and 'c'");
        assert_eq!(names(&["a", "b", "c", "d", "e"]), "'a', 'b' and 3 more");
    }

    #[test]
    fn codes_are_stable_snake_case() {
        for c in Code::ALL {
            assert_eq!(json!(c), json!(c.as_str()));
        }
    }

    #[test]
    fn a_missing_font_is_reported_with_a_hint_and_no_fix() {
        let issues = svg_issues(ObjectId(7), &[ImportWarning::TextSkipped]);
        assert_eq!(issues.len(), 1);
        let i = &issues[0];
        assert_eq!(i.code, Code::MissingFont);
        assert_eq!(i.severity, Severity::Warning);
        assert_eq!(i.object_ids, vec![7]);
        assert!(i.fixes.is_empty());
        assert!(i.message.starts_with("Text uses a font"), "{}", i.message);
        assert!(i.hint.contains("Install the font"), "{}", i.hint);
    }

    #[test]
    fn every_import_warning_has_a_code() {
        let all = [
            ImportWarning::ClipOrMaskIgnored { count: 2 },
            ImportWarning::ImagesSkipped { count: 1 },
            ImportWarning::NoPaths,
            ImportWarning::DashesIgnored { count: 3 },
            ImportWarning::TextSkipped,
        ];
        let codes: Vec<Code> = svg_issues(ObjectId(1), &all)
            .iter()
            .map(|i| i.code)
            .collect();
        assert_eq!(
            codes,
            [
                Code::ClipIgnored,
                Code::ImagesSkipped,
                Code::NoPaths,
                Code::DashesIgnored,
                Code::MissingFont
            ]
        );
        for i in svg_issues(ObjectId(1), &all) {
            assert!(i.code.import_only());
            assert!(!i.message.contains('\u{2014}') && !i.hint.contains('\u{2014}'));
        }
    }
}
