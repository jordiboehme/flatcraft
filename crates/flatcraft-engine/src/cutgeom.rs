//! Cut geometry of an object per cut mode (spec 7.1): centre lines, fill
//! edges and stroke outlines, in mat millimetres.
//!
//! Stroke outlines come from kurbo's stroker, run on the placed path with
//! the placed width, so the width scales with the object. kurbo's outline
//! is meant for non-zero filling: it overlaps itself at inner corners and
//! where a path crosses itself. So the outline is flattened and united with
//! itself (i_overlay, non-zero rule); what is left are the real edges of the
//! painted area, with real counters kept as holes. A filled path with an
//! outline stroke is cut along the edge of fill united with stroke. A
//! subpath whose fill is hidden under its thin stroke is cut as a line.
//! All booleans stay inside one path of one object; merging across objects is
//! milestone 2.
//!
//! Error budget per cut path (tolerance `t`, the plotter's flatten
//! tolerance, default 0.01 mm): stroker `t/4` plus flattening `t/4`, or
//! flattening `t/2` for fills, so the shape is within `t/2` before the
//! booleans. The booleans work on a fixed-point grid far below that (about
//! 1e-7 mm for 100 mm). The pipeline then flattens the straight lines
//! without error, but its cleaning merges nearly collinear points within
//! up to `t`. Worst case about `1.5 t` (0.015 mm by default), below the
//! 0.05 mm device step.
//!
//! Traced raster images are fills, so in object outlines they go through
//! the fill boolean too; for their simple outlines this only normalises
//! direction and start point.

use flatcraft_core::document::{CutMode, CutOptions, Object, StrokeMode};
use flatcraft_core::geometry::{Cap, CutPath, FillRule, Join, Paint};
use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::core::solver::Solver;
use i_overlay::float::overlay::{FloatOverlay, OverlayOptions};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape, Stroke, StrokeOpts};

/// Smallest tolerance used for stroking and flattening, mm. Guards the
/// stroker against a zero or broken profile value; the pipeline refuses
/// such a profile anyway.
const MIN_TOLERANCE_MM: f64 = 1e-4;

/// Subpath ends closer than this count as closed (the pipeline's join
/// tolerance), mm.
pub const CLOSE_TOLERANCE_MM: f64 = 0.01;

/// Stroker tolerance for bounds of the cut geometry, mm (the default
/// flatten tolerance).
pub const BOUNDS_TOLERANCE_MM: f64 = 0.01;

type Contour = Vec<[f64; 2]>;

/// The cut paths of `object` placed with `placed_affine`, in mat mm.
pub fn cut_paths(object: &Object, placed_affine: Affine, tolerance_mm: f64) -> Vec<BezPath> {
    object
        .geometry
        .iter()
        .flat_map(|p| cut_path(p, &object.cut, placed_affine, tolerance_mm))
        .collect()
}

/// Bounding box of the object's cut paths in mat mm; `None` if the object
/// has no paths. Cheap: the box of a union is the union of the boxes of
/// its parts, so no booleans run. A fill-only path with no area (cut as
/// nothing) still counts with its centre line.
pub fn cut_bounds(object: &Object) -> Option<Rect> {
    let affine = object.transform.affine(object.local_bounds);
    object
        .geometry
        .iter()
        .filter_map(|p| path_bounds(p, &object.cut, affine))
        .reduce(|a, b| a.union(b))
}

fn path_bounds(path: &CutPath, options: &CutOptions, affine: Affine) -> Option<Rect> {
    let placed = affine * &path.path;
    if placed.elements().is_empty() {
        return None;
    }
    let centre = placed.bounding_box();
    let style = match options.mode {
        CutMode::AllLines => None,
        CutMode::ObjectOutlines => outline_style(path.paint, options, affine),
    };
    let Some(style) = style.filter(|_| is_finite(&placed)) else {
        return Some(centre);
    };
    let closed = close_meeting_ends(&placed);
    let outline = kurbo::stroke(
        closed.iter(),
        &style,
        &StrokeOpts::default(),
        BOUNDS_TOLERANCE_MM,
    );
    let stroke = outline.bounding_box();
    Some(if path.paint.fill.is_some() {
        stroke.union(centre)
    } else {
        stroke
    })
}

/// The cut paths of one source path, in mat mm. One source path may give
/// several cut paths (the edges of its painted area).
pub fn cut_path(
    path: &CutPath,
    options: &CutOptions,
    placed_affine: Affine,
    tolerance_mm: f64,
) -> Vec<BezPath> {
    let placed = placed_affine * &path.path;
    match options.mode {
        CutMode::AllLines => vec![placed],
        CutMode::ObjectOutlines => {
            let tolerance = if tolerance_mm.is_finite() {
                tolerance_mm.max(MIN_TOLERANCE_MM)
            } else {
                MIN_TOLERANCE_MM
            };
            object_outlines(placed, path.paint, options, placed_affine, tolerance)
        }
    }
}

/// Placed stroke width: the source width times `sqrt(|det|)` of the
/// placement.
fn placed_width(width_mm: f64, affine: Affine) -> f64 {
    width_mm * affine.determinant().abs().sqrt()
}

/// The kurbo stroke style if this stroke is cut as an outline.
fn outline_style(paint: Paint, options: &CutOptions, affine: Affine) -> Option<Stroke> {
    let s = paint.stroke?;
    let width = placed_width(s.width_mm, affine);
    let outline = match options.strokes {
        StrokeMode::CentreLine => false,
        StrokeMode::Outline => true,
        StrokeMode::Auto => width >= options.auto_threshold_mm,
    };
    if !outline || !width.is_finite() || width <= 0.0 {
        return None;
    }
    let style = Stroke::new(width).with_caps(match s.cap {
        Cap::Butt => kurbo::Cap::Butt,
        Cap::Round => kurbo::Cap::Round,
        Cap::Square => kurbo::Cap::Square,
    });
    Some(match s.join {
        Join::Miter { limit } => style.with_join(kurbo::Join::Miter).with_miter_limit(limit),
        Join::Round => style.with_join(kurbo::Join::Round),
        Join::Bevel => style.with_join(kurbo::Join::Bevel),
    })
}

fn object_outlines(
    placed: BezPath,
    paint: Paint,
    options: &CutOptions,
    affine: Affine,
    tolerance: f64,
) -> Vec<BezPath> {
    if !is_finite(&placed) {
        // Booleans need finite coordinates; the pipeline refuses the rest.
        return vec![placed];
    }
    let style = outline_style(paint, options, affine);
    let shapes = match (paint.fill, style) {
        // Stroke-only paths with a centre-line stroke and unpainted paths:
        // the path itself.
        (None, None) => return vec![placed],
        // A fill with no stroke or a centre-line stroke.
        (Some(rule), None) => {
            let stroke_width = paint
                .stroke
                .map(|s| placed_width(s.width_mm, affine))
                .filter(|w| w.is_finite());
            return fill_edges(&placed, fill_rule(rule), stroke_width, tolerance);
        }
        (fill, Some(style)) => {
            let closed = close_meeting_ends(&placed);
            let outline = kurbo::stroke(
                closed.iter(),
                &style,
                &StrokeOpts::default(),
                tolerance / 4.0,
            );
            let outline = contours(outline.iter(), tolerance / 4.0);
            let outline = boolean(&outline, &[], OverlayFill::NonZero, tolerance);
            let outline: Vec<Contour> = outline.into_iter().flatten().collect();
            let bands = bands(&closed, style.width / 2.0, tolerance / 2.0);
            let stroke = boolean(&outline, &bands, OverlayFill::NonZero, tolerance);
            let stroke: Vec<Contour> = stroke.into_iter().flatten().collect();
            match fill {
                // Fill first, as its own rule, then united with the stroke.
                Some(rule) => {
                    let fill = contours(placed.iter(), tolerance / 2.0);
                    let fill = boolean(&fill, &[], fill_rule(rule), tolerance);
                    let fill: Vec<Contour> = fill.into_iter().flatten().collect();
                    boolean(&fill, &stroke, OverlayFill::NonZero, tolerance)
                }
                None => stroke.into_iter().map(|c| vec![c]).collect(),
            }
        }
    };
    shapes
        .iter()
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| ring(c))
        .collect()
}

/// The cut paths of a fill with no stroke (`stroke_width` is `None`) or
/// with a centre-line stroke of that placed width: the edge of the
/// fill under its own rule. The thin stroke lies on that edge,
/// except along the implicit closing edge of an open subpath, which
/// must be cut to free the fill.
///
/// A subpath whose own fill is hidden under its stroke is a line (a
/// straight line, a hand-drawn line with jitter, a line with a
/// slight kink, often drawn with the default fill). It shows only
/// through its stroke, so its centre line is cut and it stays out
/// of the fill: its slivers would be cut as tiny loops or as a
/// second cut next to the first. "Hidden" compares the mean width
/// of the fill (`mean_width`) with the stroke width, at least the
/// tolerance. An open subpath has no stroke along its implicit
/// closing chord, so there the bound is half the stroke width. A
/// line is cut without its close: a close would run back along the
/// line and cut it twice. Without a stroke a fill thinner than the
/// tolerance is not visible and no knife can cut it, so it is
/// dropped.
///
/// Known gaps until the milestone 2 booleans: a stroked spur inside
/// a subpath that does enclose area (a closed shape with a tail
/// drawn in the same subpath) is lost; only the fill edge is cut.
/// A stroked subpath without area that lies inside a filled area of
/// the same path is cut into the piece. Fill-only hairline slivers
/// wider than the tolerance but below the knife's smallest feature
/// are still cut as tiny loops; milestone 2 ties a smallest feature
/// size to the blade offset.
fn fill_edges(
    placed: &BezPath,
    rule: OverlayFill,
    stroke_width: Option<f64>,
    tolerance: f64,
) -> Vec<BezPath> {
    let mut fill: Vec<Contour> = Vec::new();
    let mut lines: Vec<BezPath> = Vec::new();
    // The fill of the last subpath that is not a line: the result if it
    // is the only one.
    let mut last = Vec::new();
    for sub in subpaths(placed.iter()) {
        let contour = sub.contour(tolerance / 2.0);
        let own = match &contour {
            Some(c) => boolean(std::slice::from_ref(c), &[], rule, tolerance),
            None => Vec::new(),
        };
        let hidden_below = match stroke_width {
            Some(w) if sub.ends_meet() => w,
            Some(w) => w / 2.0,
            None => 0.0,
        };
        if mean_width(&own) < hidden_below.max(tolerance) {
            if stroke_width.is_some() && !sub.segments.is_empty() {
                lines.push(sub.open_path());
            }
        } else if let Some(c) = contour {
            fill.push(c);
            last = own;
        }
    }
    let edges = if fill.len() == 1 {
        last
    } else {
        boolean(&fill, &[], rule, tolerance)
    };
    let mut out: Vec<BezPath> = edges
        .iter()
        .filter(|shape| mean_width(std::slice::from_ref(shape)) >= tolerance)
        .flatten()
        .filter(|c| c.len() >= 3)
        .map(|c| ring(c))
        .collect();
    out.extend(lines);
    out
}

/// One rectangle per flattened segment of `path`, `half` to each side, all
/// wound the same way. kurbo's outline follows parallel curves and has no
/// joins between smooth segments, so where the stroke is wider than a
/// curve's radius (a small dot) it leaves a hole that the paint covers.
/// The bands fill exactly that: everything within `half` of a segment is
/// paint. Joins and caps still come from kurbo.
fn bands(path: &BezPath, half: f64, tolerance: f64) -> Vec<Contour> {
    let mut out = Vec::new();
    let mut band = |a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if len <= 0.0 {
            return;
        }
        let (nx, ny) = (-dy * half / len, dx * half / len);
        // Counter-clockwise in the shoelace sense for every segment.
        out.push(vec![
            [a[0] - nx, a[1] - ny],
            [b[0] - nx, b[1] - ny],
            [b[0] + nx, b[1] + ny],
            [a[0] + nx, a[1] + ny],
        ]);
    };
    for sub in subpaths(path.iter()) {
        let points = sub.points(tolerance);
        for pair in points.windows(2) {
            band(pair[0], pair[1]);
        }
        if sub.closed
            && let (Some(first), Some(last)) = (points.first(), points.last())
        {
            band(*last, *first);
        }
    }
    out
}

/// One subpath: its start, its drawing segments and whether it is closed.
struct Subpath {
    start: Point,
    segments: Vec<PathEl>,
    closed: bool,
}

impl Subpath {
    fn new(start: Point) -> Self {
        Self {
            start,
            segments: Vec::new(),
            closed: false,
        }
    }

    fn elements(&self) -> impl Iterator<Item = PathEl> + '_ {
        std::iter::once(PathEl::MoveTo(self.start))
            .chain(self.segments.iter().copied())
            .chain(self.closed.then_some(PathEl::ClosePath))
    }

    /// The subpath as drawn, without its close.
    fn open_path(&self) -> BezPath {
        std::iter::once(PathEl::MoveTo(self.start))
            .chain(self.segments.iter().copied())
            .collect()
    }

    /// Closed, or drawn back to its start within `CLOSE_TOLERANCE_MM`.
    fn ends_meet(&self) -> bool {
        self.closed
            || self
                .segments
                .last()
                .and_then(PathEl::end_point)
                .is_some_and(|end| end.distance(self.start) < CLOSE_TOLERANCE_MM)
    }

    /// The flattened points, from the start to the last drawn point.
    fn points(&self, tolerance: f64) -> Contour {
        let mut c: Contour = Vec::new();
        kurbo::flatten(self.elements(), tolerance, |el| match el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => c.push([p.x, p.y]),
            // `flatten` only emits moves, lines and closes.
            _ => {}
        });
        c
    }

    /// The flattened contour, closed implicitly as a fill does; `None`
    /// with fewer than three points.
    fn contour(&self, tolerance: f64) -> Option<Contour> {
        Some(self.points(tolerance)).filter(|c| c.len() >= 3)
    }
}

/// The subpaths of `path`. The one place that knows where a subpath ends:
/// a move starts a new one, a close ends one, and drawing on after a close
/// starts again at the closed subpath's start, as in SVG. A move with
/// nothing drawn after it is no subpath.
fn subpaths(path: impl IntoIterator<Item = PathEl>) -> Vec<Subpath> {
    let mut out = Vec::new();
    let mut cur: Option<Subpath> = None;
    let mut start = Point::ORIGIN;
    for el in path {
        match el {
            PathEl::MoveTo(p) => {
                out.extend(cur.take().filter(|c| !c.segments.is_empty()));
                start = p;
                cur = Some(Subpath::new(p));
            }
            PathEl::ClosePath => {
                if let Some(mut c) = cur.take() {
                    c.closed = true;
                    out.push(c);
                }
            }
            _ => cur
                .get_or_insert_with(|| Subpath::new(start))
                .segments
                .push(el),
        }
    }
    out.extend(cur.filter(|c| !c.segments.is_empty()));
    out
}

/// Mean width of boolean result shapes: twice their area over the length
/// of all their edges (holes included). A strip of length `l` and width
/// `w` has area `l w` and about `2 l` of edge, so this is `w`; a triangle
/// of height `h` over a long base gives `h / 2`. Zero without shapes.
fn mean_width(shapes: &[Vec<Contour>]) -> f64 {
    let mut area = 0.0;
    let mut edge = 0.0;
    for shape in shapes {
        let mut signed = 0.0;
        for c in shape {
            let n = c.len();
            for i in 0..n {
                let (a, b) = (c[i], c[(i + 1) % n]);
                signed += a[0] * b[1] - b[0] * a[1];
                edge += (b[0] - a[0]).hypot(b[1] - a[1]);
            }
        }
        // Holes run against their outer edge, so they subtract.
        area += (signed / 2.0).abs();
    }
    if edge > 0.0 { 2.0 * area / edge } else { 0.0 }
}

fn fill_rule(rule: FillRule) -> OverlayFill {
    match rule {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    }
}

/// `subj` (and `clip`, if any) united, each under `rule`. The result's
/// contours are oriented (outer edges one way, holes the other), so a
/// second boolean may read them with the non-zero rule.
fn boolean(
    subj: &[Contour],
    clip: &[Contour],
    rule: OverlayFill,
    tolerance: f64,
) -> Vec<Vec<Contour>> {
    if subj.is_empty() && clip.is_empty() {
        return Vec::new();
    }
    let mut options = OverlayOptions::<f64>::default();
    // Drops slivers that no knife can cut.
    options.min_output_area = tolerance * tolerance;
    let overlay_rule = if clip.is_empty() {
        OverlayRule::Subject
    } else {
        OverlayRule::Union
    };
    let subj = subj.to_vec();
    let clip = clip.to_vec();
    FloatOverlay::with_subj_and_clip_custom(&subj, &clip, options, Solver::default())
        .overlay(overlay_rule, rule)
}

fn is_finite(path: &BezPath) -> bool {
    path.elements().iter().all(|el| match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => p.is_finite(),
        PathEl::QuadTo(a, b) => a.is_finite() && b.is_finite(),
        PathEl::CurveTo(a, b, c) => a.is_finite() && b.is_finite() && c.is_finite(),
        PathEl::ClosePath => true,
    })
}

/// `path` with every subpath whose ends meet within `CLOSE_TOLERANCE_MM`
/// closed, its last point moved onto its start, so the stroker joins the
/// ends instead of capping them.
fn close_meeting_ends(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    for mut sub in subpaths(path.iter()) {
        let start = sub.start;
        if !sub.closed
            && let Some(last) = sub.segments.last_mut()
            && let Some(end) = last.end_point()
            && end.distance(start) < CLOSE_TOLERANCE_MM
        {
            *last = match *last {
                PathEl::LineTo(_) => PathEl::LineTo(start),
                PathEl::QuadTo(a, _) => PathEl::QuadTo(a, start),
                PathEl::CurveTo(a, b, _) => PathEl::CurveTo(a, b, start),
                other => other,
            };
            sub.closed = true;
        }
        out.extend(sub.elements());
    }
    out
}

/// Flattened closed contours of a path; open subpaths are closed
/// implicitly, as a fill does.
fn contours(path: impl IntoIterator<Item = PathEl>, tolerance: f64) -> Vec<Contour> {
    subpaths(path)
        .iter()
        .filter_map(|sub| sub.contour(tolerance))
        .collect()
}

fn ring(c: &[[f64; 2]]) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(Point::new(c[0][0], c[0][1]));
    for q in &c[1..] {
        p.line_to(Point::new(q[0], q[1]));
    }
    p.close_path();
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_ends_are_closed() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        p.line_to((0.005, 0.0));
        let c = close_meeting_ends(&p);
        assert_eq!(c.elements().last(), Some(&PathEl::ClosePath));
        assert_eq!(c.elements()[3], PathEl::LineTo(Point::new(0.0, 0.0)));
        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((10.0, 0.0));
        assert_eq!(close_meeting_ends(&open), open);
    }

    #[test]
    fn drawing_after_a_close_starts_at_the_closed_start() {
        let mut p = BezPath::new();
        p.move_to((1.0, 2.0));
        p.line_to((10.0, 2.0));
        p.close_path();
        p.line_to((1.0, 9.0));
        p.move_to((5.0, 5.0));
        let subs = subpaths(p.iter());
        assert_eq!(subs.len(), 2, "a move alone is no subpath");
        assert!(subs[0].closed && !subs[1].closed);
        assert_eq!(subs[1].start, Point::new(1.0, 2.0));
        assert_eq!(subs[1].segments, vec![PathEl::LineTo(Point::new(1.0, 9.0))]);
    }

    #[test]
    fn mean_width_of_a_strip_is_its_width() {
        let strip = vec![vec![[0.0, 0.0], [50.0, 0.0], [50.0, 0.1], [0.0, 0.1]]];
        assert!((mean_width(&[strip]) - 0.1).abs() < 1e-3);
        assert_eq!(mean_width(&[]), 0.0);
    }

    #[test]
    fn open_subpaths_close_for_fills() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        assert_eq!(contours(p.iter(), 0.01).len(), 1);
    }
}
