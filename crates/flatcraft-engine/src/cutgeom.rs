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

/// A closed line whose fill is on average narrower than this is cut once,
/// along its spine, mm. Two device steps of 0.05 mm: two cuts that close
/// together tear the material. Wider ones are real slots and keep both
/// sides.
pub const HAIRLINE_MM: f64 = 0.1;

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
    #[cfg(test)]
    tests::CUT_BOUNDS_CALLS.with(|n| n.set(n.get() + 1));
    let affine = object.transform.affine(object.local_bounds);
    object
        .geometry
        .iter()
        .filter_map(|p| path_bounds(p, &object.cut, affine))
        .reduce(|a, b| a.union(b))
}

fn path_bounds(path: &CutPath, options: &CutOptions, affine: Affine) -> Option<Rect> {
    let placed = affine * &path.path;
    if !flatcraft_core::geometry::has_segments(&placed) {
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
/// closing chord, so there the bound is half the stroke width; it is
/// cut as drawn. A closed line is a thin slot: with a mean width of
/// `HAIRLINE_MM` or more it is cut as drawn, a ring with both sides.
/// Below that it is a hairline that runs back along itself, and it is
/// cut once, along its `spine`. Without a stroke a fill thinner than
/// the tolerance is not visible and no knife can cut it, so it is
/// dropped.
///
/// Known gaps until the milestone 2 booleans: a stroked spur inside
/// a subpath that does enclose area (a closed shape with a tail
/// drawn in the same subpath) is lost; only the fill edge is cut.
/// A stroked subpath without area that lies inside a filled area of
/// the same path is cut into the piece. Fill-only hairline slivers
/// wider than the tolerance but below the knife's smallest feature
/// are still cut as tiny loops; milestone 2 ties a smallest feature
/// size to the blade offset. The ends of a closed hairline are looked
/// for over a reach of 0.2 mm, which can pick the apex of a narrow "V"
/// instead of an end; such a ring is then cut as drawn, with both sides.
/// Milestone 2 improves the end detection together with the medial axis.
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
        let width = mean_width(&own);
        if width < hidden_below.max(tolerance) {
            if stroke_width.is_some() && !sub.segments.is_empty() {
                let hairline = sub.ends_meet() && width < HAIRLINE_MM.max(tolerance);
                lines.push(
                    match hairline.then(|| spine(&sub.points(tolerance / 2.0))) {
                        Some(Some(spine)) => spine,
                        _ => sub.elements().collect(),
                    },
                );
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

/// The single cut of a closed hairline with the flattened vertices `ring`.
///
/// A hairline runs out and back along itself, so it has two ends where it
/// turns around (`hairline_ends`). They split the ring into two sides. The
/// cut is the spine through the middle: the midpoints of the two sides at
/// the same share of each side's length. Every choice is made from the
/// coordinates and from each vertex's own surroundings, and the midpoints
/// treat both sides alike, so the cut does not depend on where the ring
/// starts. A blunt end (a thin rectangle) starts at one of its corners,
/// half a width off the middle.
///
/// The spine is used only if it is proven to lie on the hairline: the two
/// sides are never more than two hairline widths apart at the same share,
/// and the spine is about half the ring long. Otherwise the two sides do
/// not run alike, and the longer side is cut instead: it is part of the
/// ring, so it is never more than the hairline's width off and never a
/// second cut.
///
/// `None` if the ring has no two clear ends (a loop without tips), or if
/// neither the spine nor one side covers the whole ring; it is then cut
/// as drawn, which leaves nothing uncut.
fn spine(ring: &[[f64; 2]]) -> Option<BezPath> {
    let mut ring = ring.to_vec();
    ring.dedup();
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    let (i, j) = hairline_ends(&ring)?;
    let (i, j) = (i.min(j), i.max(j));
    // Both sides, from the lower end to the upper end.
    let lo = if point_order(&ring[i], &ring[j]).is_le() {
        ring[i]
    } else {
        ring[j]
    };
    let mut sides = [
        ring[i..=j].to_vec(),
        ring[j..].iter().chain(&ring[..=i]).copied().collect(),
    ];
    for side in &mut sides {
        if side[0] != lo {
            side.reverse();
        }
    }
    let [a, b] = sides.map(Side::new);
    // Every vertex of either side gives one spine point.
    let mut shares: Vec<f64> = a.shares.iter().chain(&b.shares).copied().collect();
    shares.sort_by(f64::total_cmp);
    shares.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
    let mut together = true;
    let mut points: Vec<Point> = shares
        .iter()
        .map(|&u| {
            let (p, q) = (a.at(u), b.at(u));
            // The midpoint is at most half this far from the ring.
            together &= p.distance(q) <= 2.0 * HAIRLINE_MM;
            p.midpoint(q)
        })
        .collect();
    points.dedup();
    let length: f64 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let half_ring = (a.length + b.length) / 2.0;
    let slack = (4.0 * HAIRLINE_MM).max(0.02 * half_ring);
    if !together || (length - half_ring).abs() > slack {
        // The longer side; equal sides go by their coordinates.
        let pick_a = a
            .length
            .total_cmp(&b.length)
            .then_with(|| {
                a.points
                    .iter()
                    .zip(&b.points)
                    .map(|(p, q)| point_order(p, q))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .is_ge();
        let side = if pick_a { a } else { b };
        // One side stands for the whole ring only if the rest of the ring
        // runs along it. If not (the ends were picked at the apex of a
        // narrow "V", so each side is one arm), cutting it would leave
        // the other part uncut: the ring is cut as drawn.
        if !all_near(ring_samples(&ring), &side.points, 2.0 * HAIRLINE_MM) {
            return None;
        }
        points = side.points.iter().map(|p| Point::new(p[0], p[1])).collect();
    }
    let mut path = BezPath::new();
    path.move_to(*points.first()?);
    for p in &points[1..] {
        path.line_to(*p);
    }
    Some(path)
}

/// The vertices of the closed `ring` and points along each of its edges,
/// the closing edge too, at most `HAIRLINE_MM` apart. A long edge between
/// two vertices must be covered as well as the vertices. The points are
/// made one at a time as they are used, so a long ring needs no buffer.
fn ring_samples(ring: &[[f64; 2]]) -> impl Iterator<Item = [f64; 2]> + '_ {
    let n = ring.len();
    (0..n).flat_map(move |k| {
        let (a, b) = (ring[k], ring[(k + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let parts = (dx.hypot(dy) / HAIRLINE_MM).ceil().max(1.0) as usize;
        (0..parts).map(move |m| {
            let t = m as f64 / parts as f64;
            [a[0] + dx * t, a[1] + dy * t]
        })
    })
}

/// Whether every one of `points` lies within `reach` of the polyline
/// `line`. The line's segments are sorted into a grid of cells two
/// `reach` wide, so each point is compared only with the segments near it.
fn all_near(points: impl IntoIterator<Item = [f64; 2]>, line: &[[f64; 2]], reach: f64) -> bool {
    let cell = 2.0 * reach;
    let key = |x: f64, y: f64| ((x / cell).floor() as i64, (y / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (k, w) in line.windows(2).enumerate() {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        // A mark every half cell: the nearest mark to any point of the
        // segment is a quarter cell away, so a point within `reach` of
        // the segment finds it in its own cell or a neighbour.
        let marks = (dx.hypot(dy) / (cell / 2.0)).ceil().max(1.0) as usize;
        for m in 0..=marks {
            let t = m as f64 / marks as f64;
            let cells = grid
                .entry(key(w[0][0] + dx * t, w[0][1] + dy * t))
                .or_default();
            if cells.last() != Some(&k) {
                cells.push(k);
            }
        }
    }
    let distance = |p: &[f64; 2], k: usize| {
        let (a, b) = (line[k], line[k + 1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dy * t)
    };
    points.into_iter().all(|p| {
        let (cx, cy) = key(p[0], p[1]);
        (-1..=1).any(|ox| {
            (-1..=1).any(|oy| {
                grid.get(&(cx + ox, cy + oy))
                    .is_some_and(|near| near.iter().any(|&k| distance(&p, k) <= reach))
            })
        })
    })
}

/// Indices of the two ends of a closed hairline: the two vertices where
/// the ring turns around most sharply. The farthest two vertices would be
/// wrong for a hairline bent by more than half a turn (a "C").
///
/// The turn at a vertex is measured between the two points a short way
/// (two hairline widths, at most a quarter of the ring) along the ring to
/// either side, so a blunt or rounded end counts like a pointed one and
/// jitter does not. The first end is the sharpest turn; the second is the
/// sharpest one at least a quarter of the ring away from it. Equal turns
/// go by coordinates. Each turn is measured by walking from its own
/// vertex, so the answer does not depend on where the ring starts.
///
/// `None` if either end turns by less than 120 degrees: the ring has no
/// two clear ends.
fn hairline_ends(ring: &[[f64; 2]]) -> Option<(usize, usize)> {
    /// Cosine of the angle between the two arms at an end that turns by
    /// 120 degrees.
    const CLEAR_END: f64 = 0.5;
    let n = ring.len();
    if n < 2 {
        return None;
    }
    let step = |k: usize| {
        let (p, q) = (ring[k], ring[(k + 1) % n]);
        (q[0] - p[0]).hypot(q[1] - p[1])
    };
    // Length along the ring from vertex 0 to every vertex.
    let mut along = Vec::with_capacity(n);
    let mut total = 0.0;
    for k in 0..n {
        along.push(total);
        total += step(k);
    }
    let reach = (2.0 * HAIRLINE_MM).min(total / 4.0);
    if reach <= 0.0 {
        return None;
    }
    // The point `reach` along the ring from vertex `k`, forwards or back,
    // relative to the vertex. The walk stops after `MAX_ARM_STEPS`
    // vertices: in a tiny ring (reach a quarter of the ring) or a long run
    // of repeated vertices, every vertex would otherwise walk most of the
    // ring, O(n^2) in all. A stopped arm ends at the vertex it reached; on
    // a run of copies that is no direction, so no end there.
    const MAX_ARM_STEPS: usize = 64;
    let arm = |k: usize, forwards: bool| {
        let mut left = reach;
        let mut at = k;
        for _ in 0..MAX_ARM_STEPS {
            let next = if forwards {
                (at + 1) % n
            } else {
                (at + n - 1) % n
            };
            let len = step(if forwards { at } else { next });
            if len >= left && len > 0.0 {
                let t = left / len;
                let (p, q) = (ring[at], ring[next]);
                return [
                    p[0] + (q[0] - p[0]) * t - ring[k][0],
                    p[1] + (q[1] - p[1]) * t - ring[k][1],
                ];
            }
            left -= len;
            at = next;
        }
        [ring[at][0] - ring[k][0], ring[at][1] - ring[k][1]]
    };
    // How sharply the ring turns around at every vertex: the cosine of
    // the angle between its two arms, 1 where it folds back onto itself.
    let sharp: Vec<f64> = (0..n)
        .map(|k| {
            let (f, b) = (arm(k, true), arm(k, false));
            let norm = f[0].hypot(f[1]) * b[0].hypot(b[1]);
            if norm > 0.0 {
                (f[0] * b[0] + f[1] * b[1]) / norm
            } else {
                // Both arms end on the vertex: no direction, no end.
                -1.0
            }
        })
        .collect();
    let sharpest = |allowed: &dyn Fn(usize) -> bool| {
        (0..n).filter(|&k| allowed(k)).max_by(|&a, &b| {
            sharp[a]
                .total_cmp(&sharp[b])
                .then(point_order(&ring[a], &ring[b]))
        })
    };
    let first = sharpest(&|_| true)?;
    let second = sharpest(&|k| {
        let d = (along[k] - along[first]).abs();
        d.min(total - d) >= total / 4.0
    })?;
    (sharp[first] >= CLEAR_END && sharp[second] >= CLEAR_END).then_some((first, second))
}

/// One side of a hairline with, for every vertex, its share of the side's
/// length (0 at the first vertex, 1 at the last).
struct Side {
    points: Contour,
    shares: Vec<f64>,
    length: f64,
}

impl Side {
    fn new(points: Contour) -> Self {
        let mut shares = Vec::with_capacity(points.len());
        let mut length = 0.0;
        shares.push(0.0);
        for w in points.windows(2) {
            length += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            shares.push(length);
        }
        for s in &mut shares {
            // A side without length is one point; every share is its start.
            *s = if length > 0.0 { *s / length } else { 0.0 };
        }
        if let Some(last) = shares.last_mut().filter(|_| length > 0.0) {
            *last = 1.0;
        }
        Self {
            points,
            shares,
            length,
        }
    }

    /// The point at share `u` of the side's length.
    fn at(&self, u: f64) -> Point {
        let point = |k: usize| Point::new(self.points[k][0], self.points[k][1]);
        // The first vertex with a share of `u` or more.
        let k = self.shares.partition_point(|s| *s < u);
        if k == 0 {
            return point(0);
        }
        if k >= self.points.len() {
            return point(self.points.len() - 1);
        }
        let (s0, s1) = (self.shares[k - 1], self.shares[k]);
        let t = if s1 > s0 { (u - s0) / (s1 - s0) } else { 1.0 };
        point(k - 1).lerp(point(k), t)
    }
}

fn point_order(a: &[f64; 2], b: &[f64; 2]) -> std::cmp::Ordering {
    a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1]))
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
pub(crate) mod tests {
    thread_local! {
        /// How often `cut_bounds` ran on this thread.
        pub(crate) static CUT_BOUNDS_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    use super::*;

    #[test]
    fn a_path_without_segments_has_no_cut_bounds() {
        for stray in ["M50 50", "M50 50 Z"] {
            let path = CutPath {
                path: BezPath::from_svg(stray).unwrap(),
                color: None,
                label: None,
                paint: Default::default(),
            };
            for mode in [CutMode::AllLines, CutMode::ObjectOutlines] {
                let options = CutOptions {
                    mode,
                    ..CutOptions::default()
                };
                assert_eq!(
                    path_bounds(&path, &options, Affine::IDENTITY),
                    None,
                    "{stray}"
                );
            }
        }
    }

    /// A tiny ring (its reach is a quarter of the ring) with a run of
    /// 60000 copies of one vertex: every vertex in the run used to walk
    /// the whole run, about 7e9 steps.
    #[test]
    fn hairline_ends_of_a_tiny_ring_with_many_duplicates_are_quick() {
        let mut ring = vec![[0.0, 0.0]];
        ring.extend(std::iter::repeat_n([0.3, 0.0], 60_000));
        ring.extend([[0.6, 0.0], [0.6, 0.02], [0.0, 0.02]]);
        let start = std::time::Instant::now();
        let _ = hairline_ends(&ring);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
    }

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
    fn spine_of_a_there_and_back_line_is_the_line() {
        let s = spine(&[[0.0, 0.0], [50.0, 0.0], [0.0, 0.0]]).unwrap();
        let line = [
            PathEl::MoveTo(Point::new(0.0, 0.0)),
            PathEl::LineTo(Point::new(50.0, 0.0)),
        ];
        assert_eq!(s.elements(), line.as_slice());
        assert!(spine(&[[1.0, 1.0], [1.0, 1.0]]).is_none());
    }

    #[test]
    fn spine_of_a_thin_rectangle_runs_through_its_middle() {
        let s = spine(&[[0.0, 0.0], [50.0, 0.0], [50.0, 0.08], [0.0, 0.08]]).unwrap();
        let pts: Vec<Point> = s.elements().iter().filter_map(|e| e.end_point()).collect();
        assert_eq!(pts.len(), 4, "{pts:?}");
        // From a corner to a corner, with the long run on the middle.
        assert!(pts[1].distance(Point::new(0.04, 0.04)) < 1e-3, "{pts:?}");
        assert!(pts[2].distance(Point::new(49.96, 0.04)) < 1e-3, "{pts:?}");
    }

    #[test]
    fn all_near_finds_far_points_along_long_segments() {
        let line = [[0.0, 0.0], [30.0, 40.0], [30.0, 40.1]];
        // On the long diagonal, off it by 0.15 and by 0.25.
        assert!(all_near([[15.0, 20.0], [15.12, 19.91]], &line, 0.2));
        assert!(!all_near([[15.0, 20.0], [15.2, 19.85]], &line, 0.2));
        assert!(!all_near([[30.0, 40.4]], &line, 0.2));
        assert!(all_near(std::iter::empty(), &line, 0.2));
    }

    #[test]
    fn a_side_that_misses_a_long_ring_edge_is_not_cut_alone() {
        // Two ends, one side over an apex 5 mm up, the other side one
        // straight 50 mm edge. The longer side passes through all three
        // vertices, but the straight edge is up to 5 mm away from it.
        let ring = [[0.0, 0.0], [25.0, 5.0], [50.0, 0.0]];
        assert_eq!(hairline_ends(&ring), Some((0, 2)));
        assert!(spine(&ring).is_none(), "{:?}", spine(&ring));
    }

    /// The samples are streamed, never collected: a 1000 km edge costs
    /// nothing until its samples are asked for.
    #[test]
    fn ring_samples_are_streamed() {
        let ring = [[0.0, 0.0], [1e9, 0.0], [1e9, 0.05]];
        let first: Vec<[f64; 2]> = ring_samples(&ring).take(3).collect();
        assert_eq!(first[0], [0.0, 0.0]);
        assert!((first[1][0] - HAIRLINE_MM).abs() < 1e-6, "{first:?}");
    }

    #[test]
    fn ring_samples_are_at_most_a_hairline_apart() {
        let ring = [[0.0, 0.0], [50.0, 0.0], [50.0, 0.05], [10.0, 0.25]];
        let samples: Vec<[f64; 2]> = ring_samples(&ring).collect();
        assert!(ring.iter().all(|v| samples.contains(v)));
        let n = samples.len();
        for k in 0..n {
            let (a, b) = (samples[k], samples[(k + 1) % n]);
            let d = (b[0] - a[0]).hypot(b[1] - a[1]);
            assert!(d <= HAIRLINE_MM + 1e-12, "{a:?} {b:?}");
        }
        // About one sample per 0.1 mm of ring, not more.
        assert!(n < 1100, "{n}");
    }

    #[test]
    fn a_loop_without_tips_has_no_ends() {
        let circle: Vec<[f64; 2]> = (0..48)
            .map(|k| {
                let a = f64::from(k) * std::f64::consts::TAU / 48.0;
                [0.05 * a.cos(), 0.05 * a.sin()]
            })
            .collect();
        assert_eq!(hairline_ends(&circle), None);
        assert!(spine(&circle).is_none());
    }

    #[test]
    fn sides_that_do_not_run_alike_give_one_side() {
        // Out along a straight line, back with a small zigzag near one
        // end. The zigzag makes that side longer, so points at the same
        // share of the two sides drift more than 1 mm apart and the
        // middle is not trusted. The zigzag side still runs within
        // 0.2 mm of the whole ring, so it is cut alone.
        let mut ring = vec![[0.0, 0.0], [50.0, 0.0]];
        for k in 0..=20 {
            let y = if k % 2 == 0 { 0.02 } else { 0.17 };
            ring.push([10.0 - 0.1 * f64::from(k), y]);
        }
        let s = spine(&ring).unwrap();
        let pts: Vec<Point> = s.elements().iter().filter_map(|e| e.end_point()).collect();
        let mut side: Vec<Point> = ring[1..].iter().map(|p| Point::new(p[0], p[1])).collect();
        side.push(Point::new(0.0, 0.0));
        side.reverse();
        assert_eq!(pts, side);
    }

    #[test]
    fn a_side_with_a_detour_does_not_stand_for_the_ring() {
        // Back with a 3 mm detour: the longer side leaves the straight
        // side uncut under the detour, so neither is cut alone.
        let ring = [
            [0.0, 0.0],
            [50.0, 0.0],
            [30.0, 0.02],
            [25.0, 3.0],
            [20.0, 0.02],
        ];
        assert!(spine(&ring).is_none(), "{:?}", spine(&ring));
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
