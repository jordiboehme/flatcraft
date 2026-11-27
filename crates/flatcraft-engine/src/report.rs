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

use flatcraft_core::document::{CutMode, CutOptions, KeepMode, ObjectId};
use flatcraft_import::ImportWarning;
use kurbo::{BezPath, PathEl, Point, Rect, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use std::sync::Arc;

use crate::cutgeom::{self, CLOSE_TOLERANCE_MM, CutPiece, PieceFacts};
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
    /// Filled shapes thinner than the knife's narrowest feature (half the
    /// blade offset): the cut drops them.
    ThinSlivers,
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
    /// A Stencil object on a material without a backing sheet (paper,
    /// cardstock) has closed inner parts (the middle of an O) that fall
    /// out. Information only: it never blocks the cut.
    StencilIsland,
    /// Strokes the stroke mode Auto cuts once, along their middle, though
    /// they are wide enough to show as a band. A hint (info).
    StrokesCutOnce,
    /// Two objects closer than the minimum feature size, not overlapping:
    /// the strip between them is too thin to cut cleanly or to weed. A
    /// hint only (info): close objects can be the design.
    ObjectsClose,
    /// Two objects that overlap: the knife cuts through both. A hint only
    /// (info): overlapping objects can be the design.
    ObjectsOverlap,
    /// Pairs of objects in one top-level group that are close or overlap
    /// (letters as separate objects, tight kerning): one hint (info) for
    /// the whole group instead of one per pair.
    GroupObjectsClose,
}

impl Code {
    /// Every code, in the order the tool description lists them.
    pub const ALL: [Code; 25] = [
        Code::OpenPaths,
        Code::Hairline,
        Code::BelowMinFeature,
        Code::FillIgnored,
        Code::ThinSlivers,
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
        Code::StencilIsland,
        Code::StrokesCutOnce,
        Code::ObjectsClose,
        Code::ObjectsOverlap,
        Code::GroupObjectsClose,
    ];

    /// The code as agents see it, like `open_paths`.
    pub fn as_str(self) -> &'static str {
        match self {
            Code::OpenPaths => "open_paths",
            Code::Hairline => "hairline",
            Code::BelowMinFeature => "below_min_feature",
            Code::FillIgnored => "fill_ignored",
            Code::ThinSlivers => "thin_slivers",
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
            Code::StencilIsland => "stencil_island",
            Code::StrokesCutOnce => "strokes_cut_once",
            Code::ObjectsClose => "objects_close",
            Code::ObjectsOverlap => "objects_overlap",
            Code::GroupObjectsClose => "group_objects_close",
        }
    }

    /// A short name for people, for the title of a hover card.
    pub fn title(self) -> &'static str {
        match self {
            Code::OpenPaths => "Open shapes",
            Code::Hairline => "Thin double cuts",
            Code::BelowMinFeature => "Tiny shapes",
            Code::FillIgnored => "Fills that are not cut",
            Code::ThinSlivers => "Slivers too thin to cut",
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
            Code::StencilIsland => "Stencil middles fall out",
            Code::StrokesCutOnce => "Strokes cut once",
            Code::ObjectsClose => "Objects nearly touching",
            Code::ObjectsOverlap => "Objects overlap",
            Code::GroupObjectsClose => "Close objects in a group",
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
    flatcraft_import::trace::MIN_FEATURE_BLADE_OFFSETS * blade_offset(blade_offset_mm)
}

/// The narrowest filled shape the knife cuts, mm: half the blade offset.
/// Both edges of a thinner strip lie inside the blade's own swing, so the
/// cut cannot follow them; the cut geometry drops such slivers. Every
/// width limit follows from the blade offset like this one and
/// [`min_feature_mm`].
pub fn min_width_mm(blade_offset_mm: f64) -> f64 {
    MIN_WIDTH_BLADE_OFFSETS * blade_offset(blade_offset_mm)
}

/// [`min_width_mm`] in blade offsets.
pub const MIN_WIDTH_BLADE_OFFSETS: f64 = 0.5;

/// The blade offset, or the default 0.25 mm for a broken value.
fn blade_offset(blade_offset_mm: f64) -> f64 {
    if blade_offset_mm.is_finite() && blade_offset_mm > 0.0 {
        blade_offset_mm
    } else {
        0.25
    }
}

/// An open outline whose ends are closer than this (and farther apart than
/// the join tolerance) has a gap: the piece hangs on a thin bridge.
pub fn gap_limit_mm(min_feature: f64) -> f64 {
    min_feature
}

/// A stroke at least this wide shows as a band of colour, mm: the stroke
/// mode Auto outlined strokes from here until its default threshold rose
/// to the minimum feature size. One cut once along its middle loses its
/// width, so the report says so (`strokes_cut_once`).
pub const WIDE_STROKE_MM: f64 = 0.5;

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
    /// Filled shapes dropped as slivers.
    slivers: usize,
    /// Strokes at least [`WIDE_STROKE_MM`] wide that Auto cuts once.
    cut_once: usize,
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

/// Counts the open shapes that should be closed: open cut lines of a
/// filled path with area (`filled`: all lines cuts it as drawn, object
/// outlines would cut the edge of its fill), and open lines with a small
/// gap between their ends. Returns (filled, gaps).
fn open_count<'a>(
    cut: impl IntoIterator<Item = &'a BezPath>,
    filled: bool,
    gap_limit: f64,
) -> (usize, usize) {
    let mut out = (0, 0);
    for s in cut.into_iter().flat_map(subs).filter(|s| !s.closed) {
        if filled {
            out.0 += 1;
        } else if s.gap < gap_limit && s.bounds.width().max(s.bounds.height()) > gap_limit {
            out.1 += 1;
        }
    }
    out
}

/// The cut pieces of `o` cut with `options`, and what that cut left out:
/// the input's shared cut geometry for the object's own options, a fresh
/// cut for any other (what a fix would change it to).
fn pieces_as(
    input: &JobInput,
    o: &JobObject,
    options: &CutOptions,
) -> (Arc<Vec<CutPiece>>, PieceFacts) {
    if *options == o.cut {
        let g = input.cut_geometry();
        if let Some(p) = g.per_object.get(&o.id) {
            return (p.clone(), g.facts(o.id));
        }
    }
    let affine = o.transform.affine(o.local_bounds);
    let (pieces, facts, _) = cutgeom::object_pieces(
        &o.geometry,
        options,
        affine,
        input.tolerance_mm(),
        input.min_width_mm(),
    );
    (Arc::new(pieces), facts)
}

fn measure(input: &JobInput, o: &JobObject, options: &CutOptions, min_feature: f64) -> Shapes {
    let affine = o.transform.affine(o.local_bounds);
    // A visible outline leaves its covered parts open on purpose: what is
    // open, closed or small is measured on its outlines as drawn, so the
    // open-paths check never undoes the person's choice.
    let drawn = CutOptions {
        mode: CutMode::ObjectOutlines,
        ..*options
    };
    let options = if options.mode == CutMode::VisibleOutline {
        &drawn
    } else {
        options
    };
    let (pieces, facts) = pieces_as(input, o, options);
    let mut m = Shapes {
        hairline_widths: (f64::INFINITY, 0.0),
        no_area: facts.no_area,
        slivers: facts.slivers,
        ..Shapes::default()
    };
    let gap_limit = gap_limit_mm(min_feature);
    // In all lines, the source paths whose fill object outlines would cut.
    let with_area: std::collections::BTreeSet<usize> = if options.mode == CutMode::AllLines {
        let outlines = CutOptions {
            mode: CutMode::ObjectOutlines,
            ..*options
        };
        pieces_as(input, o, &outlines)
            .0
            .iter()
            .filter_map(|p| p.source)
            .collect()
    } else {
        Default::default()
    };
    // The pieces of each source path, sorted once.
    let mut by_source: Vec<Vec<&BezPath>> = vec![Vec::new(); o.geometry.len()];
    for c in pieces.iter() {
        if let Some(list) = c.source.and_then(|i| by_source.get_mut(i)) {
            list.push(&c.path);
        }
    }
    for (i, p) in o.geometry.iter().enumerate() {
        let cut = by_source[i].iter().copied();
        // A filled path cut as drawn should be closed only if its fill
        // has area: a filled straight line is a line.
        let filled = p.paint.fill.is_some() && with_area.contains(&i);
        let (open, gaps) = open_count(cut.clone(), filled, gap_limit);
        m.open += open + gaps;
        m.gaps += gaps;
        for s in cut.flat_map(subs) {
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
        if cutgeom::auto_centre_line_width(p.paint, options, affine)
            .is_some_and(|w| w >= WIDE_STROKE_MM)
        {
            m.cut_once += 1;
        }
        if let Some(w) = cutgeom::outline_width(p.paint, options, affine)
            && w < min_feature
        {
            m.hairline += 1;
            m.hairline_widths.0 = m.hairline_widths.0.min(w);
            m.hairline_widths.1 = m.hairline_widths.1.max(w);
        }
    }
    // Pieces of several source paths (welded outlines) count too.
    for s in pieces
        .iter()
        .filter(|c| c.source.is_none())
        .flat_map(|c| subs(&c.path))
    {
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
    m
}

/// The box of what is cut of an object in mat mm, from the input's cut
/// geometry ([`cutgeom::CutGeometry::bounds`]): the one box every check of
/// the report measures, and the rule `placed_bounds` follows too.
pub fn object_bounds(input: &JobInput, o: &JobObject) -> Option<Rect> {
    let cut = input.cut_geometry();
    if cut.per_object.contains_key(&o.id) {
        return cut.bounds(o.id);
    }
    let affine = o.transform.affine(o.local_bounds);
    cutgeom::cut_bounds_of(&o.geometry, &o.cut, affine, input.knife())
}

/// What the report checks with: the cutting area, the flatten tolerance
/// and the minimum feature size of the job's plotter and mat.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub area: Option<Rect>,
    pub tolerance_mm: f64,
    pub min_feature_mm: f64,
    /// The narrowest filled shape the knife cuts ([`min_width_mm`]).
    pub min_width_mm: f64,
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
            min_width_mm: input.min_width_mm(),
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
    report.issues.extend(pair_issues(input, ids, &limits));
    report
}

/// [`check`] of the objects `ids` that builds the cut geometry only of the
/// objects the result can depend on, not of the whole document:
///
/// - the objects themselves and their linked copies;
/// - every object a merged or visible outline among them is cut together
///   with (a weld's box is the box of all of it), as far as that reaches;
/// - every object whose box comes near one of those (a pair);
/// - the groups those move with, and every object within the largest
///   move a Move apart fix may try (it must not bring them near another);
/// - and again what a merged or visible outline among those reaches.
///
/// The boxes are [`cutgeom::loose_bounds`]: never smaller than what is
/// cut, and found without cutting. Gives the same report as
/// `check(input, Some(ids))`. The flag is true when that took every object
/// of the input: its cut geometry is then the document's and may go back
/// into the document's cache.
pub fn check_selection(input: &JobInput, ids: &[ObjectId]) -> (Report, bool) {
    let limits = Limits::of(input);
    let limit = gap_limit_mm(limits.min_feature_mm);
    let tol = limits.tolerance_mm;
    let objects = &input.objects;
    let n = objects.len();
    // Boxes that hold all an object cuts, without cutting it: a box too
    // large only adds an object, never leaves one out.
    let boxes: Vec<Option<Rect>> = objects.iter().map(cutgeom::loose_bounds).collect();
    let near = |a: Option<Rect>, b: Option<Rect>, by: f64| matches!((a, b), (Some(a), Some(b)) if a.inflate(by, by).overlaps(b));
    // Merged and visible outlines are cut with the objects over them,
    // and those with theirs: a walk from every needed object.
    let cross_closure = |need: &mut Vec<bool>| {
        let mut todo: Vec<usize> = (0..n).filter(|&i| need[i]).collect();
        while let Some(i) = todo.pop() {
            for j in 0..n {
                if need[j] {
                    continue;
                }
                let cross =
                    objects[i].cut.mode.is_cross_object() || objects[j].cut.mode.is_cross_object();
                if cross && near(boxes[i], boxes[j], tol) {
                    need[j] = true;
                    todo.push(j);
                }
            }
        }
    };
    let mut need: Vec<bool> = objects.iter().map(|o| ids.contains(&o.id)).collect();
    let links: std::collections::BTreeSet<_> = objects
        .iter()
        .zip(&need)
        .filter(|(_, n)| **n)
        .filter_map(|(o, _)| o.link_group)
        .collect();
    for (i, o) in objects.iter().enumerate() {
        if o.link_group.is_some_and(|g| links.contains(&g)) {
            need[i] = true;
        }
    }
    cross_closure(&mut need);
    // Their pairs.
    let core: Vec<usize> = (0..n).filter(|&i| need[i]).collect();
    for j in 0..n {
        if core.iter().any(|&i| near(boxes[i], boxes[j], limit + tol)) {
            need[j] = true;
        }
    }
    // What a pair's object moves with, and where a move may take it.
    let units: std::collections::BTreeSet<ObjectId> = (0..n)
        .filter(|&i| need[i])
        .map(|i| objects[i].unit)
        .collect();
    for (i, o) in objects.iter().enumerate() {
        if units.contains(&o.unit) {
            need[i] = true;
        }
    }
    let movers: Vec<usize> = (0..n).filter(|&i| need[i]).collect();
    let reach = movers
        .iter()
        .filter_map(|&i| boxes[i])
        .map(|b| b.width().max(b.height()))
        .fold(0.0_f64, f64::max)
        * 2.0
        + 2.0 * limit
        + tol;
    for j in 0..n {
        if movers.iter().any(|&i| near(boxes[i], boxes[j], reach)) {
            need[j] = true;
        }
    }
    cross_closure(&mut need);
    if need.iter().all(|n| *n) {
        return (check(input, Some(ids)), true);
    }
    let mut narrow = input.clone();
    narrow.objects = objects
        .iter()
        .zip(&need)
        .filter(|(_, n)| **n)
        .map(|(o, _)| o.clone())
        .collect();
    narrow.cut = crate::job::CutSeed::new(input.cut.seed());
    (check(&narrow, Some(ids)), false)
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
    let measure_as = |options: &CutOptions| measure(input, o, options, limits.min_feature_mm);
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
    let bounds = object_bounds(input, o);
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
    if m.slivers > 0 {
        issues.push(thin_slivers(o, m.slivers, limits, Severity::Warning));
    }
    if m.cut_once > 0 {
        let threshold = mm(o.cut.auto_threshold_mm);
        let (what, them) = if m.cut_once == 1 {
            (
                "1 stroke is thinner than".to_string(),
                ("it", "its", "Outline it"),
            )
        } else {
            (
                format!("{} strokes are thinner than", m.cut_once),
                ("they", "their", "Outline them"),
            )
        };
        issues.push(Issue {
            code: Code::StrokesCutOnce,
            severity: Severity::Info,
            object_ids: shared.clone(),
            paths: m.cut_once,
            message: format!(
                "{name}: {what} {threshold}, so {} {} cut once along {} middle. {} if you want the stroke width as vinyl.",
                them.0,
                if m.cut_once == 1 { "is" } else { "are" },
                them.1,
                them.2
            ),
            hint: format!(
                "To cut around the stroke instead, set Strokes to Outline in the Cut panel. A band thinner than {} may tear; the report then says so.",
                mm(limits.min_feature_mm)
            ),
            // No fix: Outline makes a band under the minimum feature
            // size, whose own fix (cut as single lines) would undo it.
            fixes: Vec::new(),
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
    if o.keep == KeepMode::Stencil && !input.material.backing {
        let n = islands(input.cut_geometry().pieces(o.id), limits.tolerance_mm);
        if n > 0 {
            issues.push(Issue {
                code: Code::StencilIsland,
                severity: Severity::Info,
                // Linked copies share the keep mode: one line for all.
                object_ids: shared.clone(),
                paths: n,
                message: format!(
                    "{name}: {} inside the letters of this paper stencil {} out.",
                    count(n, "closed part", "closed parts"),
                    if n == 1 { "falls" } else { "fall" }
                ),
                hint: STENCIL_ISLAND_HINT.into(),
                fixes: Vec::new(),
            });
        }
    }
    Report {
        shapes: m.total,
        issues,
    }
}

/// The hint of [`Code::StencilIsland`].
pub const STENCIL_ISLAND_HINT: &str = "The middle of letters like O, A and B falls out of a paper stencil. Use a stencil font with bridges if you need them.";

/// How many closed cut rings of `o` lie inside an odd number of its other
/// rings: the counter of an O (inside the letter's outline), whose middle
/// is an island in a stencil. Each ring is flattened and tested by one of
/// its points against the rings whose box holds that point.
fn islands(pieces: &[CutPiece], tolerance: f64) -> usize {
    let mut rings: Vec<(Vec<Point>, Rect)> = Vec::new();
    {
        for cut in pieces.iter().map(|p| &p.path) {
            let mut ring: Vec<Point> = Vec::new();
            let mut finish = |ring: &mut Vec<Point>, closed: bool| {
                if closed && ring.len() > 2 {
                    let b = ring
                        .iter()
                        .skip(1)
                        .fold(Rect::from_points(ring[0], ring[0]), |r, q| r.union_pt(*q));
                    rings.push((std::mem::take(ring), b));
                } else {
                    ring.clear();
                }
            };
            kurbo::flatten(cut, tolerance.max(0.01), |el| match el {
                PathEl::MoveTo(q) => {
                    finish(&mut ring, false);
                    ring.push(q);
                }
                PathEl::LineTo(q) => ring.push(q),
                PathEl::ClosePath => finish(&mut ring, true),
                _ => {}
            });
            finish(&mut ring, false);
        }
    }
    (0..rings.len())
        .filter(|&i| {
            let q = rings[i].0[0];
            let depth = (0..rings.len())
                .filter(|&j| j != i && rings[j].1.contains(q))
                .filter(|&j| {
                    flatcraft_path::containment::point_in_ring(q, &rings[j].0) == Some(true)
                })
                .count();
            depth % 2 == 1
        })
        .count()
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

/// The hint of [`Code::ThinSlivers`].
pub const THIN_SLIVERS_HINT: &str = "Both edges of such a thin strip lie inside the swing of the blade tip, so the knife cannot follow them. Make the shapes wider in your drawing app, or give them a stroke: a stroked thin shape is cut once, along its middle.";

/// The `thin_slivers` issue: `n` filled slivers of `o` are dropped.
fn thin_slivers(o: &JobObject, n: usize, limits: &Limits, severity: Severity) -> Issue {
    let (what, verb) = if n == 1 {
        ("1 filled sliver is".to_string(), "is")
    } else {
        (format!("{n} filled slivers are"), "are")
    };
    Issue {
        code: Code::ThinSlivers,
        severity,
        object_ids: vec![o.id.0],
        paths: n,
        message: format!(
            "{}: {what} thinner than {}, half the blade offset, so {verb} not cut.",
            quoted(&o.name),
            mm(limits.min_width_mm)
        ),
        hint: THIN_SLIVERS_HINT.into(),
        fixes: Vec::new(),
    }
}

/// The `thin_slivers` issue of an object the job does cut.
pub(crate) fn thin_slivers_issue(input: &JobInput, o: &JobObject, n: usize) -> Option<Issue> {
    (n > 0).then(|| thin_slivers(o, n, &Limits::of(input), Severity::Warning))
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
    let as_lines = measure(
        input,
        o,
        &with_mode(o, CutMode::AllLines),
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

// ---------------------------------------------------------------- pairs

/// The hint about objects that nearly touch, with the minimum gap.
fn objects_close_hint(limit: f64) -> String {
    format!(
        "Close shaves can be part of the design. If they are not, move the objects at least {} apart, the narrowest strip this blade cuts cleanly. If they are, ignore this hint for the pair.",
        mm(limit)
    )
}

/// The hint about objects that overlap.
pub const OBJECTS_OVERLAP_HINT: &str = "Overlapping objects can be part of the design, like a sticker on a sticker. If they are not, move one of them away. If they are, ignore this hint for the pair.";

/// The cut lines of one object for the pair checks: flattened polylines,
/// each with whether it is closed, and their box.
struct Lines {
    lines: Vec<(Vec<Point>, bool)>,
    bounds: Rect,
}

impl Lines {
    fn of(pieces: &[CutPiece], tolerance: f64) -> Option<Self> {
        let mut lines: Vec<(Vec<Point>, bool)> = Vec::new();
        for piece in pieces {
            let mut cur: Vec<Point> = Vec::new();
            let finish =
                |cur: &mut Vec<Point>, closed: bool, lines: &mut Vec<(Vec<Point>, bool)>| {
                    if closed && cur.len() > 1 {
                        cur.push(cur[0]);
                    }
                    if cur.len() > 1 {
                        lines.push((std::mem::take(cur), closed));
                    } else {
                        cur.clear();
                    }
                };
            kurbo::flatten(piece.path.iter(), tolerance.max(0.01), |el| match el {
                PathEl::MoveTo(q) => {
                    finish(&mut cur, false, &mut lines);
                    cur.push(q);
                }
                PathEl::LineTo(q) => cur.push(q),
                PathEl::ClosePath => finish(&mut cur, true, &mut lines),
                _ => {}
            });
            finish(&mut cur, false, &mut lines);
        }
        let bounds = lines
            .iter()
            .flat_map(|(l, _)| l.iter())
            .fold(None, |b: Option<Rect>, p| {
                Some(b.map_or(Rect::from_points(*p, *p), |b| b.union_pt(*p)))
            })?;
        Some(Self { lines, bounds })
    }

    fn translated(&self, d: kurbo::Vec2) -> Self {
        Self {
            lines: self
                .lines
                .iter()
                .map(|(l, c)| (l.iter().map(|p| *p + d).collect(), *c))
                .collect(),
            bounds: self.bounds + d,
        }
    }

    /// True if `q` lies in this object's material: inside an odd number
    /// of its closed lines (the middle of an O is not).
    fn holds(&self, q: Point) -> bool {
        self.lines
            .iter()
            .filter(|(l, closed)| *closed && l.len() > 3)
            .filter(|(l, _)| {
                flatcraft_path::containment::point_in_ring(q, &l[..l.len() - 1]) == Some(true)
            })
            .count()
            % 2
            == 1
    }

    fn first_point(&self) -> Option<Point> {
        self.lines.first().map(|(l, _)| l[0])
    }
}

/// The closest points of the segments `a0 a1` and `b0 b1`, and their
/// distance (0 where they cross).
fn segment_gap(a0: Point, a1: Point, b0: Point, b1: Point) -> (f64, Point, Point) {
    let cross = |o: Point, p: Point, q: Point| (p - o).cross(q - o);
    let (d1, d2) = (cross(a0, a1, b0), cross(a0, a1, b1));
    let (d3, d4) = (cross(b0, b1, a0), cross(b0, b1, a1));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        let t = d3 / (d3 - d4);
        let p = a0.lerp(a1, t);
        return (0.0, p, p);
    }
    let nearest = |p: Point, s0: Point, s1: Point| {
        let d = s1 - s0;
        let len2 = d.hypot2();
        let t = if len2 > 0.0 {
            ((p - s0).dot(d) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        s0 + d * t
    };
    let options = [
        (a0, nearest(a0, b0, b1)),
        (a1, nearest(a1, b0, b1)),
        (nearest(b0, a0, a1), b0),
        (nearest(b1, a0, a1), b1),
    ];
    options
        .into_iter()
        .map(|(p, q)| (p.distance(q), p, q))
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .expect("four options")
}

/// The closest approach of the lines `a` and `b` if they come within
/// `reach`: the gap and the closest point on each. Segments are sorted
/// into a grid of cells two `reach` wide, by marks every half cell, so
/// only segments near each other are compared (as in `cutgeom::all_near`).
fn approach(a: &Lines, b: &Lines, reach: f64) -> Option<(f64, Point, Point)> {
    use std::collections::{HashMap, HashSet};
    let cell = 2.0 * reach.max(1e-3);
    let key = |p: Point| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let segments = |l: &Lines| -> Vec<(Point, Point)> {
        l.lines
            .iter()
            .flat_map(|(pts, _)| pts.windows(2).map(|w| (w[0], w[1])))
            .collect()
    };
    let marks = |(p, q): (Point, Point)| {
        let n = (p.distance(q) / (cell / 2.0)).ceil().max(1.0) as usize;
        (0..=n).map(move |m| p.lerp(q, m as f64 / n as f64))
    };
    let sa = segments(a);
    let sb = segments(b);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in sa.iter().enumerate() {
        for m in marks(*s) {
            let cells = grid.entry(key(m)).or_default();
            if cells.last() != Some(&i) {
                cells.push(i);
            }
        }
    }
    let mut best: Option<(f64, Point, Point)> = None;
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    for (j, s) in sb.iter().enumerate() {
        for m in marks(*s) {
            let (cx, cy) = key(m);
            for ox in -1..=1 {
                for oy in -1..=1 {
                    let Some(near) = grid.get(&(cx + ox, cy + oy)) else {
                        continue;
                    };
                    for &i in near {
                        if !seen.insert((i, j)) {
                            continue;
                        }
                        let (a0, a1) = sa[i];
                        let found = segment_gap(a0, a1, s.0, s.1);
                        if found.0 <= reach && best.is_none_or(|b| found.0 < b.0) {
                            best = Some(found);
                        }
                    }
                }
            }
        }
    }
    best
}

/// What the pair check found about two objects.
enum Pair {
    Overlap,
    Close { gap: f64 },
}

/// How two objects lie: overlapping (their cut lines cross or touch, or
/// one lies in the other's material), closer than `limit`, or neither.
fn pair_of(a: &Lines, b: &Lines, limit: f64, tolerance: f64) -> Option<Pair> {
    let near = a.bounds.inflate(limit, limit);
    if near.x0 > b.bounds.x1
        || b.bounds.x0 > near.x1
        || near.y0 > b.bounds.y1
        || b.bounds.y0 > near.y1
    {
        return None;
    }
    let inside =
        b.first_point().is_some_and(|q| a.holds(q)) || a.first_point().is_some_and(|q| b.holds(q));
    let near = approach(a, b, limit);
    match near {
        Some((gap, _, _)) if gap <= tolerance => Some(Pair::Overlap),
        _ if inside => Some(Pair::Overlap),
        Some((gap, _, _)) if gap < limit => Some(Pair::Close { gap }),
        _ => None,
    }
}

/// The pairs of boxes in `boxes` (each already grown by half the reach on
/// every side) that touch, as index pairs `(i, j)` with `i < j`, in order.
/// The boxes are sorted into a grid of cells about one box wide, and only
/// boxes that share a cell are compared, so a sheet of many small objects
/// costs about as many comparisons as it has objects, not their square.
fn touching_boxes(boxes: &[Rect]) -> Vec<(usize, usize)> {
    use std::collections::{BTreeSet, HashMap};
    if boxes.len() < 2 {
        return Vec::new();
    }
    let mean = boxes.iter().map(|b| b.width().max(b.height())).sum::<f64>() / boxes.len() as f64;
    let cell = mean.max(1e-3);
    // A huge object would cover too many cells: limit the cells per box,
    // so it is compared with every object near its box instead.
    const MAX_CELLS: i64 = 4096;
    let range = |lo: f64, hi: f64| ((lo / cell).floor() as i64, (hi / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut huge: Vec<usize> = Vec::new();
    for (i, b) in boxes.iter().enumerate() {
        let (x0, x1) = range(b.x0, b.x1);
        let (y0, y1) = range(b.y0, b.y1);
        // Checked: a huge coordinate saturates the cell numbers, and
        // their difference would overflow.
        let cells = x1
            .checked_sub(x0)
            .and_then(|w| w.checked_add(1))
            .zip(y1.checked_sub(y0).and_then(|h| h.checked_add(1)))
            .and_then(|(w, h)| w.checked_mul(h));
        if cells.is_none_or(|c| c > MAX_CELLS) {
            huge.push(i);
            continue;
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                grid.entry((x, y)).or_default().push(i);
            }
        }
    }
    let mut out: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut test = |i: usize, j: usize, out: &mut BTreeSet<(usize, usize)>| {
        let (i, j) = (i.min(j), i.max(j));
        if i == j || !seen.insert((i, j)) {
            return;
        }
        #[cfg(test)]
        tests::PAIR_BOX_TESTS.with(|n| n.set(n.get() + 1));
        let (a, b) = (boxes[i], boxes[j]);
        if a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1 {
            out.insert((i, j));
        }
    };
    for members in grid.values() {
        for (k, &i) in members.iter().enumerate() {
            for &j in &members[k + 1..] {
                test(i, j, &mut out);
            }
        }
    }
    for &h in &huge {
        for j in 0..boxes.len() {
            test(h, j, &mut out);
        }
    }
    out.into_iter().collect()
}

/// The hints about pairs of objects that nearly touch or overlap, for
/// the pairs with at least one object of `ids` (every pair when `None`).
/// Only objects that are cut count: a hidden or skipped object is not on
/// the sheet. Pairs inside one top-level group (letters set as separate
/// objects) come as one `group_objects_close` hint per group, so a tight
/// word does not flood the report; pairs across groups or with a loose
/// object stay one hint each. Pairs and groups the person turned off
/// (`Document::ignored_pairs`, `Document::ignored_groups`) are left out.
fn pair_issues(input: &JobInput, ids: Option<&[ObjectId]>, limits: &Limits) -> Vec<Issue> {
    let cut = input.cut_geometry();
    let tolerance = limits.tolerance_mm.max(0.01);
    let limit = gap_limit_mm(limits.min_feature_mm);
    let cuttable: Vec<(&JobObject, Rect)> = input
        .objects
        .iter()
        .filter(|o| o.visible && o.cut_enabled)
        .filter(|o| !cut.pieces(o.id).is_empty())
        .filter_map(|o| Some((o, cut.bounds(o.id)?)))
        .collect();
    let grown: Vec<Rect> = cuttable
        .iter()
        .map(|(_, b)| b.inflate(limit / 2.0, limit / 2.0))
        .collect();
    let wanted = |o: &JobObject| ids.is_none_or(|ids| ids.contains(&o.id));
    let ignored = |code: Code, a: &JobObject, b: &JobObject| {
        input
            .ignored_pairs
            .contains(&flatcraft_core::document::IgnoredPair::new(
                code.as_str(),
                a.id.0,
                b.id.0,
            ))
    };
    let lines: Vec<std::cell::OnceCell<Option<Lines>>> = cuttable
        .iter()
        .map(|_| std::cell::OnceCell::new())
        .collect();
    let lines_of = |i: usize| {
        lines[i]
            .get_or_init(|| Lines::of(cut.pieces(cuttable[i].0.id), tolerance))
            .as_ref()
    };
    let near = touching_boxes(&grown);
    let ctx = PairContext {
        input,
        limits,
        limit,
        tolerance,
        cuttable: &cuttable,
        grown: &grown,
        near: &near,
    };
    let mut out = Vec::new();
    // Pairs inside one top-level group, collected into one hint per group:
    // the group and the objects of its pairs.
    let mut in_groups: Vec<(ObjectId, usize, Vec<ObjectId>)> = Vec::new();
    for &(i, j) in &near {
        let (a, b) = (cuttable[i].0, cuttable[j].0);
        if !wanted(a) && !wanted(b) {
            continue;
        }
        let (Some(la), Some(lb)) = (lines_of(i), lines_of(j)) else {
            continue;
        };
        let mut pair = pair_of(la, lb, limit, tolerance);
        // A visible outline behind an object with painted area is clipped
        // by it: its cut ends on the front object's edge and never runs
        // through it, so the knife does not cut through both. What it
        // leaves to see can be a strip too thin to cut cleanly, though:
        // (width, back, front).
        let mut thin: Option<(f64, &JobObject, &JobObject)> = None;
        if matches!(pair, Some(Pair::Overlap)) {
            let at = |o: &JobObject| input.objects.iter().position(|x| x.id == o.id);
            let (back, front) = if at(a) < at(b) { (a, b) } else { (b, a) };
            if back.cut.mode == CutMode::VisibleOutline && covers(input, front) {
                pair = None;
                thin = thin_seen_strip(&cut, back, front, limit).map(|w| (w, back, front));
            }
        }
        let group = (a.unit == b.unit && a.unit != a.id && b.unit != b.id).then_some(a.unit);
        let in_group = match (&pair, thin) {
            (Some(Pair::Overlap), _) => Some(Code::ObjectsOverlap),
            (Some(Pair::Close { .. }), _) | (None, Some(_)) => Some(Code::ObjectsClose),
            (None, None) => None,
        };
        if let (Some(g), Some(code)) = (group, in_group) {
            if input.ignored_groups.contains(&g) || ignored(code, a, b) {
                continue;
            }
            let at = match in_groups.iter().position(|(x, _, _)| *x == g) {
                Some(k) => k,
                None => {
                    in_groups.push((g, 0, Vec::new()));
                    in_groups.len() - 1
                }
            };
            let entry = &mut in_groups[at];
            entry.1 += 1;
            for id in [a.id, b.id] {
                if !entry.2.contains(&id) {
                    entry.2.push(id);
                }
            }
            continue;
        }
        match pair {
            Some(Pair::Overlap) if !ignored(Code::ObjectsOverlap, a, b) => {
                out.push(Issue {
                    code: Code::ObjectsOverlap,
                    severity: Severity::Info,
                    object_ids: vec![a.id.0, b.id.0],
                    paths: 0,
                    message: format!(
                        "{} and {} overlap, so the knife cuts through both.",
                        capitalized(&seen_name(&cut, a)),
                        seen_name(&cut, b)
                    ),
                    hint: OBJECTS_OVERLAP_HINT.into(),
                    fixes: merge_fix(input, a, b)
                        .into_iter()
                        .chain([ignore_fix(Code::ObjectsOverlap, a, b)])
                        .collect(),
                });
            }
            Some(Pair::Close { gap, .. }) if !ignored(Code::ObjectsClose, a, b) => {
                let mut fixes: Vec<Fix> =
                    move_apart_fix(&ctx, &lines_of, i, j).into_iter().collect();
                fixes.push(ignore_fix(Code::ObjectsClose, a, b));
                out.push(Issue {
                    code: Code::ObjectsClose,
                    severity: Severity::Info,
                    object_ids: vec![a.id.0, b.id.0],
                    paths: 0,
                    message: format!(
                        "{} and {} are only {} apart. The strip between them is too thin to cut cleanly or to weed (peel the waste away), and the blade may tear it.",
                        capitalized(&seen_name(&cut, a)),
                        seen_name(&cut, b),
                        mm(gap)
                    ),
                    hint: objects_close_hint(limit),
                    fixes,
                });
            }
            _ => {}
        }
        if let Some((width, back, front)) = thin
            && !ignored(Code::ObjectsClose, a, b)
        {
            out.push(Issue {
                code: Code::ObjectsClose,
                severity: Severity::Info,
                object_ids: vec![a.id.0, b.id.0],
                paths: 0,
                message: format!(
                    "{} leaves only a {} strip of {} to see. The strip is too thin to cut cleanly or to weed (peel the waste away), and the blade may tear it.",
                    capitalized(&seen_name(&cut, front)),
                    mm(width),
                    seen_name(&cut, back)
                ),
                hint: format!(
                    "Move the front object so at least {} of the one behind shows, or so it covers that edge completely. If the thin strip is on purpose, ignore this hint for the pair.",
                    mm(limit)
                ),
                fixes: vec![ignore_fix(Code::ObjectsClose, a, b)],
            });
        }
    }
    for (group, pairs, mut ids) in in_groups {
        ids.sort_by_key(|id| input.objects.iter().position(|o| o.id == *id));
        let name = input
            .group_names
            .get(&group)
            .map_or_else(|| format!("group {}", group.0), |n| quoted(n));
        let what = if pairs == 1 {
            "1 pair of objects".to_string()
        } else {
            format!("{pairs} pairs of objects")
        };
        let verb = if pairs == 1 { "is" } else { "are" };
        out.push(Issue {
            code: Code::GroupObjectsClose,
            severity: Severity::Info,
            object_ids: ids.iter().map(|i| i.0).collect(),
            paths: pairs,
            message: format!("{what} in {name} {verb} very close or overlap."),
            hint: format!(
                "Parts of one design, like letters set as separate objects, often sit this close on purpose. If they do not, move them at least {} apart, the narrowest strip this blade cuts cleanly. If they do, ignore this hint for the group.",
                mm(limit)
            ),
            fixes: vec![Fix {
                label: "Ignore for this group".into(),
                description: "Hides this hint for the whole group, for good: it is kept in the project. Nothing in the design changes. Undo brings it back.".into(),
                command: "document.ignore_group_hint".into(),
                args: json!({ "group_id": group.0 }),
            }],
        });
    }
    out
}

/// What a "Move apart" looks at besides the pair: every cut object, its
/// grown box and which boxes touch.
struct PairContext<'a> {
    input: &'a JobInput,
    limits: &'a Limits,
    limit: f64,
    tolerance: f64,
    cuttable: &'a [(&'a JobObject, Rect)],
    grown: &'a [Rect],
    near: &'a [(usize, usize)],
}

/// True when `o` has painted area (a fill, or a stroke cut as an
/// outline): only such an object covers what lies behind it in visible
/// outline. A centre-line stroke or an all-lines line covers nothing.
fn covers(input: &JobInput, o: &JobObject) -> bool {
    // The same rule as the cut: a sliver that is not cut covers nothing.
    let affine = o.transform.affine(o.local_bounds);
    o.geometry.iter().any(|p| {
        !cutgeom::painted(
            p,
            &o.cut,
            affine,
            input.tolerance_mm(),
            input.min_width_mm(),
        )
        .area
        .is_empty()
    })
}

/// The width of the narrowest strip that `front` leaves to see of the
/// visible outline `back`, when it is thinner than `limit`. Only strips
/// near `front` count.
fn thin_seen_strip(
    cut: &cutgeom::CutGeometry,
    back: &JobObject,
    front: &JobObject,
    limit: f64,
) -> Option<f64> {
    let near = cut.bounds(front.id)?.inflate(limit, limit);
    cut.seen_shapes
        .get(&back.id)?
        .iter()
        .filter(|(w, b)| *w < limit && b.is_some_and(|b| b.overlaps(near)))
        .map(|(w, _)| *w)
        .reduce(f64::min)
}

/// How the person knows `o` on the canvas: its name, or "the merged
/// outline of 'X'" when `o` holds a weld of several objects.
fn seen_name(cut: &cutgeom::CutGeometry, o: &JobObject) -> String {
    if cut.merged_into.values().any(|b| *b == o.id) {
        format!("the merged outline of {}", quoted(&o.name))
    } else {
        quoted(&o.name)
    }
}

/// `text` with its first letter upper case, to start a sentence.
fn capitalized(text: &str) -> String {
    let mut c = text.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// "Merge into one outline": sets `a` and `b` to merged outline, so they
/// are cut as one welded shape and the knife no longer cuts through both.
/// Offered only when they would weld (shown, cut, the same keep mode,
/// painted areas that overlap): then the hint is gone after it. Not
/// offered for objects that are only close: merging does not bridge a gap,
/// so the hint would stay. Not offered when either object is in visible
/// outline, which the person chose for overlaps.
fn merge_fix(input: &JobInput, a: &JobObject, b: &JobObject) -> Option<Fix> {
    // A visible outline is a choice about overlaps already: merging would
    // undo it.
    let visible = |o: &JobObject| o.cut.mode == CutMode::VisibleOutline;
    if visible(a)
        || visible(b)
        || !cutgeom::would_weld(a, b, input.tolerance_mm(), input.min_width_mm())
    {
        return None;
    }
    let linked = members(input, a).len() > 1 || members(input, b).len() > 1;
    let note = if linked {
        " Their linked copies change together with them."
    } else {
        ""
    };
    Some(Fix {
        label: "Merge into one outline".into(),
        description: format!(
            "Sets both objects to Merged outline, so they are cut as one piece: one cut around both and none where they overlap. One undo step.{note}"
        ),
        command: "object.set_cut_options".into(),
        args: json!({ "ids": [a.id.0, b.id.0], "mode": "merged_outline" }),
    })
}

/// "Ignore for this object pair": turns the hint `code` off for `a` and
/// `b`, stored in the project.
fn ignore_fix(code: Code, a: &JobObject, b: &JobObject) -> Fix {
    Fix {
        label: "Ignore for this object pair".into(),
        description: "Hides this hint for these two objects, for good: it is kept in the project. Nothing in the design changes. Undo brings it back.".into(),
        command: "document.ignore_issue".into(),
        args: json!({ "code": code.as_str(), "object_ids": [a.id.0, b.id.0] }),
    }
}

/// Room a "Move apart" leaves over the minimum gap, so rounding never
/// leaves it just short.
const MOVE_APART_SLACK: f64 = 1.02;

/// "Move apart": moves the second object (`cuttable[j]`, with its group
/// when the two are not in the same group) along one of the four axis
/// directions until its gap to the first (`cuttable[i]`) reaches the
/// minimum. Of the directions that keep it in the cutting area and bring
/// none of the moved objects close to (or over) an object it was not close
/// to before, the one with the smallest move wins. As an absolute
/// position, so running it again does no harm. `None` when no direction
/// works.
fn move_apart_fix<'a>(
    ctx: &PairContext<'a>,
    lines_of: &dyn Fn(usize) -> Option<&'a Lines>,
    i: usize,
    j: usize,
) -> Option<Fix> {
    let (a, b) = (ctx.cuttable[i].0, ctx.cuttable[j].0);
    let la = lines_of(i)?;
    let limit = ctx.limit;
    // The whole group of `b` moves, unless `a` is in it too.
    let unit = if a.unit != b.unit { b.unit } else { b.id };
    let moves = |o: &JobObject| {
        if unit == b.id {
            o.id == b.id
        } else {
            o.unit == unit
        }
    };
    let movers: Vec<&JobObject> = ctx.input.objects.iter().filter(|o| moves(o)).collect();
    let any_shown = movers.iter().any(|o| o.visible);
    let start = movers
        .iter()
        .filter(|o| o.visible || !any_shown)
        .map(|o| {
            object_bounds(ctx.input, o)
                .unwrap_or_else(|| Rect::from_points(o.transform.center_mm, o.transform.center_mm))
        })
        .reduce(|x, y| x.union(y))?;
    // The moved objects that are cut, by their index in `cuttable`.
    let moved: Vec<usize> = (0..ctx.cuttable.len())
        .filter(|&k| moves(ctx.cuttable[k].0))
        .collect();
    let lb = lines_of(j)?;
    let ab = ctx.cuttable[i].1;
    let bb = ctx.cuttable[j].1;
    let directions = [
        (
            "to the right",
            kurbo::Vec2::new(1.0, 0.0),
            ab.x1 + limit - bb.x0,
        ),
        (
            "to the left",
            kurbo::Vec2::new(-1.0, 0.0),
            bb.x1 - (ab.x0 - limit),
        ),
        ("down", kurbo::Vec2::new(0.0, 1.0), ab.y1 + limit - bb.y0),
        ("up", kurbo::Vec2::new(0.0, -1.0), bb.y1 - (ab.y0 - limit)),
    ];
    let gap_after = |d: kurbo::Vec2| match pair_of(la, &lb.translated(d), limit, ctx.tolerance) {
        Some(Pair::Overlap) => 0.0,
        Some(Pair::Close { gap, .. }) => gap,
        None => limit,
    };
    let mut best: Option<(f64, &str, kurbo::Vec2)> = None;
    for (word, dir, clear) in directions {
        // The smallest shift that clears `a`: step up from the gap, and at
        // most until the boxes are a minimum gap apart, which always does.
        let clear = clear.max(0.0) * MOVE_APART_SLACK + 0.01;
        let mut shift = 0.01_f64.max(limit * 0.1);
        let mut found = None;
        for _ in 0..16 {
            if shift >= clear {
                found = Some(clear);
                break;
            }
            let now = gap_after(dir * shift);
            if now >= limit {
                found = Some(shift);
                break;
            }
            shift += (limit - now) * MOVE_APART_SLACK + 0.01;
        }
        let shift = found.unwrap_or(clear);
        if best.is_some_and(|(s, _, _)| s <= shift) {
            continue;
        }
        let d = dir * shift;
        if let Some(area) = ctx.limits.area
            && !crate::commands::inside(start + d, area)
        {
            continue;
        }
        if creates_a_pair(ctx, lines_of, &moved, d) {
            continue;
        }
        best = Some((shift, word, d));
    }
    let (shift, word, d) = best?;
    let moved_box = start + d;
    let what = if unit == b.id {
        quoted(&b.name)
    } else {
        format!("{} and the rest of its group", quoted(&b.name))
    };
    Some(Fix {
        label: "Move apart".into(),
        description: format!(
            "Moves {what} {} {word}, away from {}, so the gap is at least {}. One undo step.",
            mm(shift),
            quoted(&a.name),
            mm(limit)
        ),
        command: "object.move".into(),
        args: json!({ "ids": [unit.0], "x_mm": moved_box.x0, "y_mm": moved_box.y0 }),
    })
}

/// True if moving the objects `moved` (indexes into `cuttable`) by `d`
/// brings one of them close to, or over, an object that stays and that it
/// was not close to before.
fn creates_a_pair<'a>(
    ctx: &PairContext<'a>,
    lines_of: &dyn Fn(usize) -> Option<&'a Lines>,
    moved: &[usize],
    d: kurbo::Vec2,
) -> bool {
    // How the two lay before the move: an overlap after a close pair is
    // new too.
    let before = |m: usize, k: usize| {
        let (x, y) = (m.min(k), m.max(k));
        if ctx.near.binary_search(&(x, y)).is_err() {
            return None;
        }
        match (lines_of(m), lines_of(k)) {
            (Some(lm), Some(lk)) => pair_of(lm, lk, ctx.limit, ctx.tolerance),
            _ => None,
        }
    };
    let worse = |after: &Pair, before: Option<Pair>| {
        matches!(
            (after, before),
            (_, None) | (Pair::Overlap, Some(Pair::Close { .. }))
        )
    };
    moved.iter().any(|&m| {
        let Some(lm) = lines_of(m) else {
            return false;
        };
        let shifted = lm.translated(d);
        let grown = ctx.grown[m] + d;
        (0..ctx.cuttable.len())
            .filter(|k| !moved.contains(k))
            .filter(|&k| {
                let g = ctx.grown[k];
                g.x0 <= grown.x1 && grown.x0 <= g.x1 && g.y0 <= grown.y1 && grown.y0 <= g.y1
            })
            .any(|k| {
                lines_of(k)
                    .and_then(|lk| pair_of(lk, &shifted, ctx.limit, ctx.tolerance))
                    .is_some_and(|after| worse(&after, before(m, k)))
            })
    })
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
        let slivers = input.cut_geometry().facts(*id).slivers;
        if slivers > 0 {
            out.push(thin_slivers(o, slivers, &limits, Severity::Error));
        }
        if slivers > 0 && *no_area == 0 {
            // The slivers are the reason; there is no other path.
        } else if *no_area > 0 {
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
            let fix = object_bounds(input, o).and_then(|b| {
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
            let n = measure(input, o, &o.cut, limits.min_feature_mm).total;
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
        let Some(i) =
            object_bounds(input, o).and_then(|b| area_issue(o, b, area, &members(input, o)))
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
pub(crate) mod tests {
    thread_local! {
        /// How many pairs of boxes the pair check compared on this thread.
        pub(crate) static PAIR_BOX_TESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    use super::*;

    /// A huge coordinate saturates the cell numbers: the box is compared
    /// with every other one instead of overflowing or walking all cells.
    #[test]
    fn touching_boxes_takes_huge_coordinates() {
        let boxes = [
            Rect::new(0.0, 0.0, 1.0, 1.0),
            Rect::new(-1e30, -1e30, 1e30, 1e30),
            Rect::new(1e25, 1e25, 1e25 + 1.0, 1e25 + 1.0),
        ];
        assert_eq!(touching_boxes(&boxes), vec![(0, 1), (1, 2)]);
    }

    /// `n` objects of four shapes (squares, circles, triangles, thin
    /// bars) on a grid at 9.2 mm, 25 to a row, so neighbouring squares are
    /// 1.2 mm apart: the same density whatever `n`.
    fn sheet(n: usize) -> JobInput {
        use flatcraft_core::document::{Document, JobSettings, Source};
        use flatcraft_core::geometry::CutPath;
        let mut doc = Document::new(JobSettings {
            plotter_id: "file-output-gpgl-cameo2".into(),
            material_id: "copy-paper".into(),
            media_id: "mat-12x12".into(),
            speed: None,
            force: None,
            passes: None,
            strategy: Default::default(),
            weed: Default::default(),
        });
        for k in 0..n {
            let (x, y) = (5.0 + (k % 25) as f64 * 9.2, 5.0 + (k / 25) as f64 * 9.2);
            let path = match k % 4 {
                0 => kurbo::Rect::new(x, y, x + 8.0, y + 8.0).to_path(0.01),
                1 => kurbo::Circle::new((x + 4.0, y + 4.0), 4.0).to_path(0.01),
                2 => {
                    let mut p = BezPath::new();
                    p.move_to((x, y + 8.0));
                    p.line_to((x + 4.0, y));
                    p.line_to((x + 8.0, y + 8.0));
                    p.close_path();
                    p
                }
                _ => kurbo::Rect::new(x, y + 3.5, x + 8.0, y + 4.5).to_path(0.01),
            };
            let id = doc.add(
                format!("shape {k}"),
                Source::Svg {
                    name: String::new(),
                    bytes: std::sync::Arc::from(Vec::<u8>::new()),
                },
                vec![CutPath {
                    path,
                    color: None,
                    label: None,
                    paint: Default::default(),
                }],
            );
            let o = doc.get_mut(id).unwrap();
            o.transform.center_mm = o.local_bounds.center();
        }
        let dir = tempfile::tempdir().unwrap();
        let store = flatcraft_device::ProfileStore::load(dir.path()).unwrap();
        let plotter = store.plotter("file-output-gpgl-cameo2").unwrap();
        let material = store.material("copy-paper").unwrap();
        JobInput::from_parts(&doc, plotter, &material)
    }

    /// The pair check compares boxes only within grid cells: twice the
    /// objects at the same density is about twice the work, never four
    /// times (a quadratic blow-up).
    #[test]
    fn the_pair_check_grows_with_the_objects_not_their_square() {
        let count = |n: usize| {
            let input = sheet(n);
            let _ = input.cut_geometry();
            PAIR_BOX_TESTS.with(|c| c.set(0));
            let r = check(&input, None);
            let close = r
                .issues
                .iter()
                .filter(|i| i.code == Code::ObjectsClose)
                .count();
            (PAIR_BOX_TESTS.with(|c| c.get()), close)
        };
        let (small, close_small) = count(250);
        let (large, close_large) = count(500);
        assert!(close_small > 0, "the sheet has close pairs");
        assert!(close_large > close_small);
        assert!(
            large <= small * 5 / 2,
            "250 objects: {small} box tests, 500: {large}"
        );
        // Far below the 124750 pairs of 500 objects.
        assert!(large < 20_000, "{large}");
    }

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
