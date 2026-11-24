//! Cut geometry of an object per cut mode (spec 7.1): centre lines, fill
//! edges and stroke outlines, in mat millimetres.
//!
//! Stroke outlines come from kurbo's stroker, run on the placed path with
//! the placed width, so the width scales with the object. kurbo's outline
//! is meant for non-zero filling: it overlaps itself at inner corners and
//! where a path crosses itself. So the outline is flattened and united with
//! itself (i_overlay, non-zero rule); what is left are the real edges of the
//! painted area, with real counters kept as holes. A filled path with an
//! outline stroke is cut along the edge of fill united with stroke. All
//! booleans stay inside one path of one object; merging across objects is
//! milestone 2.
//!
//! Error budget per cut path (tolerance `t`, the plotter's flatten
//! tolerance): stroker `t/4` plus flattening `t/4`, or flattening `t/2` for
//! fills, so the result is within `t/2` of the exact shape. The booleans
//! work on a fixed-point grid far below that (about 1e-7 mm for 100 mm).
//! The pipeline then flattens straight lines without further error.

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
        // Centre-line strokes (with or without fill) and unpainted paths:
        // the path itself, once.
        (None, None) => return vec![placed],
        (Some(_), None) if paint.stroke.is_some() => return vec![placed],
        (Some(rule), None) => {
            let fill = contours(placed.iter(), tolerance / 2.0);
            boolean(&fill, &[], fill_rule(rule), tolerance)
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

/// One rectangle per flattened segment of `path`, `half` to each side, all
/// wound the same way. kurbo's outline follows parallel curves and has no
/// joins between smooth segments, so where the stroke is wider than a
/// curve's radius (a small dot) it leaves a hole that the paint covers.
/// The bands fill exactly that: everything within `half` of a segment is
/// paint. Joins and caps still come from kurbo.
fn bands(path: &BezPath, half: f64, tolerance: f64) -> Vec<Contour> {
    let mut out = Vec::new();
    let mut last: Option<Point> = None;
    let mut start: Option<Point> = None;
    let mut band = |a: Point, b: Point| {
        let d = b - a;
        let len = d.hypot();
        if len <= 0.0 {
            return;
        }
        let n = kurbo::Vec2::new(-d.y, d.x) * (half / len);
        // Counter-clockwise in the shoelace sense for every segment.
        out.push(vec![
            [(a - n).x, (a - n).y],
            [(b - n).x, (b - n).y],
            [(b + n).x, (b + n).y],
            [(a + n).x, (a + n).y],
        ]);
    };
    kurbo::flatten(path.iter(), tolerance, |el| match el {
        PathEl::MoveTo(p) => {
            last = Some(p);
            start = Some(p);
        }
        PathEl::LineTo(p) => {
            if let Some(a) = last {
                band(a, p);
            }
            last = Some(p);
        }
        PathEl::ClosePath => {
            if let (Some(a), Some(s)) = (last, start) {
                band(a, s);
            }
            last = start;
        }
        _ => {}
    });
    out
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
    let els = path.elements();
    let mut out = BezPath::new();
    let mut start: Option<Point> = None;
    let mut drawn = false;
    let finish = |out: &mut BezPath, start: Option<Point>, drawn: bool| {
        let Some(s) = start else { return };
        if !drawn {
            return;
        }
        let last = out.elements().len() - 1;
        let end = out.elements()[last].end_point();
        if let Some(e) = end
            && e.distance(s) < CLOSE_TOLERANCE_MM
        {
            let el = match out.elements()[last] {
                PathEl::LineTo(_) => PathEl::LineTo(s),
                PathEl::QuadTo(a, _) => PathEl::QuadTo(a, s),
                PathEl::CurveTo(a, b, _) => PathEl::CurveTo(a, b, s),
                other => other,
            };
            let mut els = out.elements().to_vec();
            els[last] = el;
            *out = BezPath::from_vec(els);
            out.close_path();
        }
    };
    for el in els {
        match el {
            PathEl::MoveTo(p) => {
                finish(&mut out, start, drawn);
                start = Some(*p);
                drawn = false;
                out.push(*el);
            }
            PathEl::ClosePath => {
                out.push(*el);
                start = None;
                drawn = false;
            }
            _ => {
                if start.is_none() {
                    // Drawing on after a close: SVG starts again at the
                    // closed subpath's start.
                    let s = out
                        .elements()
                        .iter()
                        .rev()
                        .find_map(|e| match e {
                            PathEl::MoveTo(p) => Some(*p),
                            _ => None,
                        })
                        .unwrap_or(Point::ORIGIN);
                    out.move_to(s);
                    start = Some(s);
                }
                out.push(*el);
                drawn = true;
            }
        }
    }
    finish(&mut out, start, drawn);
    out
}

/// Flattened closed contours of a path; open subpaths are closed
/// implicitly, as a fill does.
fn contours(path: impl IntoIterator<Item = PathEl>, tolerance: f64) -> Vec<Contour> {
    let mut out = Vec::new();
    let mut cur: Contour = Vec::new();
    let push = |out: &mut Vec<Contour>, cur: &mut Contour| {
        let c = std::mem::take(cur);
        if c.len() >= 3 {
            out.push(c);
        }
    };
    kurbo::flatten(path, tolerance, |el| match el {
        PathEl::MoveTo(p) => {
            push(&mut out, &mut cur);
            cur.push([p.x, p.y]);
        }
        PathEl::LineTo(p) => cur.push([p.x, p.y]),
        PathEl::ClosePath => push(&mut out, &mut cur),
        // `flatten` only emits moves, lines and closes.
        _ => {}
    });
    push(&mut out, &mut cur);
    out
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
    fn open_subpaths_close_for_fills() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        assert_eq!(contours(p.iter(), 0.01).len(), 1);
    }
}
