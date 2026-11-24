//! Cut geometry of an object per cut mode (spec 7.1): centre lines, fills
//! and stroke outlines, in mat millimetres.
//!
//! Stroke outlines come from kurbo's stroker, run on the placed path with
//! the placed width, so the width scales with the object. kurbo's outline
//! is meant for non-zero filling: at every inner corner it runs through the
//! corner of the centre line and leaves a small loop. A knife would cut that
//! loop out of the strip, so the outline is flattened and short loops are
//! removed. Without polygon booleans (milestone 2), a path with a fill and
//! an outline stroke is cut along one ring of the stroke outline per closed
//! subpath: the outer ring, or the inner ring when the subpath is the edge
//! of a hole.

use flatcraft_core::document::{CutMode, CutOptions, Object, StrokeMode};
use flatcraft_core::geometry::{Cap, CutPath, FillRule, Join, Paint};
use kurbo::{Affine, BezPath, PathEl, Point, Shape, Stroke, StrokeOpts};

/// Smallest tolerance used for stroking and flattening, mm. Guards the
/// stroker against a zero or broken profile value; the pipeline refuses
/// such a profile anyway.
const MIN_TOLERANCE_MM: f64 = 1e-4;

/// Loops of a stroke outline up to this many stroke widths long are
/// removed. Inner corner loops are about one width long plus twice the
/// overshoot of the inner edge, which grows for very sharp corners.
const MAX_LOOP_WIDTHS: f64 = 12.0;

/// The cut paths of `object` placed with `placed_affine`, in mat mm.
pub fn cut_paths(object: &Object, placed_affine: Affine, tolerance_mm: f64) -> Vec<BezPath> {
    object
        .geometry
        .iter()
        .flat_map(|p| cut_path(p, &object.cut, placed_affine, tolerance_mm))
        .collect()
}

/// The cut paths of one source path, in mat mm. One source path may give
/// several cut paths (the rings of a stroke outline).
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
            object_outlines(&placed, path.paint, options, placed_affine, tolerance)
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
    placed: &BezPath,
    paint: Paint,
    options: &CutOptions,
    affine: Affine,
    tolerance: f64,
) -> Vec<BezPath> {
    let Some(style) = outline_style(paint, options, affine) else {
        // Fill only, centre-line stroke (with or without fill, cut once),
        // or nothing painted: the path itself.
        return vec![placed.clone()];
    };
    let mut out = Vec::new();
    for (sub, closed) in subpaths(placed) {
        let rings = stroke_rings(&sub, &style, tolerance);
        match paint.fill {
            Some(rule) if closed && !rings.is_empty() => {
                out.push(fill_edge_ring(rings, placed, rule));
            }
            _ => out.extend(rings),
        }
    }
    out.into_iter().map(|r| polyline(&r)).collect()
}

/// The subpaths of `path`, each starting with a move, and whether each is
/// closed.
fn subpaths(path: &BezPath) -> Vec<(BezPath, bool)> {
    let mut out: Vec<(BezPath, bool)> = Vec::new();
    let mut cur = BezPath::new();
    let mut start = Point::ORIGIN;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                if !cur.elements().is_empty() {
                    out.push((std::mem::take(&mut cur), false));
                }
                start = *p;
                cur.push(*el);
            }
            PathEl::ClosePath => {
                if !cur.elements().is_empty() {
                    cur.push(*el);
                    out.push((std::mem::take(&mut cur), true));
                }
            }
            _ => {
                if cur.elements().is_empty() {
                    // Drawing on after a close starts at the closed
                    // subpath's start.
                    cur.move_to(start);
                }
                cur.push(*el);
            }
        }
    }
    if !cur.elements().is_empty() {
        out.push((cur, false));
    }
    out.retain(|(p, _)| p.elements().len() > 1);
    out
}

/// The stroke outline of one subpath as clean closed rings of points:
/// open subpaths give one ring, closed ones two.
fn stroke_rings(sub: &BezPath, style: &Stroke, tolerance: f64) -> Vec<Vec<Point>> {
    let outline = kurbo::stroke(sub.iter(), style, &StrokeOpts::default(), tolerance / 2.0);
    let max_loop = MAX_LOOP_WIDTHS * style.width;
    let mut rings = Vec::new();
    let mut ring: Vec<Point> = Vec::new();
    let mut finish = |ring: &mut Vec<Point>| {
        let mut pts = std::mem::take(ring);
        dedup(&mut pts);
        let pts = remove_loops(pts, max_loop);
        if pts.len() >= 3 && signed_area(&pts).abs() > 0.0 {
            rings.push(pts);
        }
    };
    kurbo::flatten(outline.iter(), tolerance / 2.0, |el| match el {
        PathEl::MoveTo(p) => {
            finish(&mut ring);
            ring.push(p);
        }
        PathEl::LineTo(p) => ring.push(p),
        PathEl::ClosePath => finish(&mut ring),
        // `flatten` only emits moves, lines and closes.
        _ => {}
    });
    finish(&mut ring);
    rings
}

/// The ring of a closed subpath's stroke outline that is the edge between
/// paint and empty: the ring with the largest area, unless that ring lies
/// inside the fill (the subpath is the edge of a hole), then the smallest.
fn fill_edge_ring(mut rings: Vec<Vec<Point>>, placed: &BezPath, rule: FillRule) -> Vec<Point> {
    rings.sort_by(|a, b| signed_area(a).abs().total_cmp(&signed_area(b).abs()));
    let largest = rings.pop().expect("at least one ring");
    if rings.is_empty() || !inside_fill(placed, rule, sample_point(&largest)) {
        return largest;
    }
    rings.swap_remove(0)
}

fn inside_fill(path: &BezPath, rule: FillRule, p: Point) -> bool {
    let w = path.winding(p);
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// The middle of the ring's longest edge: on the ring, away from corners.
fn sample_point(ring: &[Point]) -> Point {
    let n = ring.len();
    (0..n)
        .map(|i| (ring[i], ring[(i + 1) % n]))
        .max_by(|a, b| a.0.distance(a.1).total_cmp(&b.0.distance(b.1)))
        .map(|(a, b)| a.midpoint(b))
        .unwrap_or(ring[0])
}

fn polyline(ring: &[Point]) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(ring[0]);
    for q in &ring[1..] {
        p.line_to(*q);
    }
    p.close_path();
    p
}

/// Removes repeated points and a last point equal to the first.
fn dedup(pts: &mut Vec<Point>) {
    const EPS: f64 = 1e-9;
    pts.dedup_by(|a, b| a.distance(*b) < EPS);
    while pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) < EPS {
        pts.pop();
    }
}

/// Shoelace area; the sign gives the orientation.
fn signed_area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n)
        .map(|i| pts[i].to_vec2().cross(pts[(i + 1) % n].to_vec2()))
        .sum::<f64>()
        / 2.0
}

/// The point where segments `a0-a1` and `b0-b1` cross, if they cross
/// properly (not just touch or overlap).
fn crossing(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<Point> {
    let da = a1 - a0;
    let db = b1 - b0;
    let d1 = da.cross(b0 - a0);
    let d2 = da.cross(b1 - a0);
    let d3 = db.cross(a0 - b0);
    let d4 = db.cross(a1 - b0);
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        Some(b0 + db * (d1 / (d1 - d2)))
    } else {
        None
    }
}

/// Cuts self-crossing loops out of a closed ring: where two edges cross,
/// the shorter part between them is dropped, if it is at most `max_len`
/// long and shorter than the rest of the ring.
fn remove_loops(mut pts: Vec<Point>, max_len: f64) -> Vec<Point> {
    'outer: loop {
        let n = pts.len();
        if n < 4 {
            return pts;
        }
        let perimeter: f64 = (0..n).map(|i| pts[i].distance(pts[(i + 1) % n])).sum();
        let limit = max_len.min(perimeter / 2.0);
        for i in 0..n {
            let (a0, a1) = (pts[i], pts[(i + 1) % n]);
            let mut len = 0.0;
            // Edge i + k, for k from 2 up to the edge before edge i.
            for k in 2..n - 1 {
                let j = (i + k) % n;
                len += pts[(j + n - 1) % n].distance(pts[j]);
                if len > limit {
                    break;
                }
                if let Some(x) = crossing(a0, a1, pts[j], pts[(j + 1) % n]) {
                    pts.rotate_left(i);
                    let mut kept = vec![pts[0], x];
                    kept.extend_from_slice(&pts[k + 1..]);
                    pts = kept;
                    continue 'outer;
                }
            }
        }
        return pts;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_finds_the_point() {
        let x = crossing(
            Point::new(0.0, 0.0),
            Point::new(2.0, 2.0),
            Point::new(0.0, 2.0),
            Point::new(2.0, 0.0),
        );
        assert_eq!(x, Some(Point::new(1.0, 1.0)));
        assert_eq!(
            crossing(
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(0.0, 1.0),
                Point::new(1.0, 1.0)
            ),
            None
        );
    }

    #[test]
    fn bow_tie_loop_is_removed() {
        // A square whose top edge overshoots and comes back through a small
        // loop at the top-right corner.
        let pts = vec![
            Point::new(0.0, 0.0),
            Point::new(11.0, 0.0),
            Point::new(11.0, -1.0),
            Point::new(10.0, -1.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        let clean = remove_loops(pts, 5.0);
        assert_eq!(
            clean,
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 10.0),
                Point::new(0.0, 10.0),
            ]
        );
    }

    #[test]
    fn long_loops_stay() {
        let pts = vec![
            Point::new(0.0, 0.0),
            Point::new(11.0, 0.0),
            Point::new(11.0, -1.0),
            Point::new(10.0, -1.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ];
        assert_eq!(remove_loops(pts.clone(), 1.5).len(), pts.len());
    }
}
