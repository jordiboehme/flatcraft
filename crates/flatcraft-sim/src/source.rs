//! Distances to the source geometry: how far the cut lies from the shapes
//! as drawn, before flattening. The simulator's own metrics compare the
//! trace with the intended (flattened) path; this measures the end-to-end
//! error (R7, [`crate::END_TO_END_LIMIT_MM`]) and the upstream error of the
//! intended path itself, both two-sided.

use std::collections::HashMap;

use flatcraft_path::{Toolpath, ToolpathKind, WASTE_SEG};
use kurbo::{BezPath, PathEl, Point};

use crate::SimTrace;

/// The tolerance the source shapes are flattened with before measuring:
/// far below every limit, so the flattening adds nothing that matters.
pub const SOURCE_FLATTEN_MM: f64 = 1e-4;

/// The spacing of the samples along the source shapes.
pub const SAMPLE_STEP_MM: f64 = 0.01;

/// The cell size of [`SegmentIndex`]: a query looks one cell round its
/// own, so any segment within this distance is found.
pub const CELL_MM: f64 = 0.1;

/// The distance of `p` from the segment `a`-`b`.
pub fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + t * ab)
}

/// Segments in square cells of [`CELL_MM`], to find the segment near a
/// point.
pub struct SegmentIndex {
    cells: HashMap<(i64, i64), Vec<usize>>,
    segments: Vec<(Point, Point)>,
}

fn cell(v: f64) -> i64 {
    (v / CELL_MM).floor() as i64
}

impl SegmentIndex {
    pub fn new(segments: Vec<(Point, Point)>) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (k, &(a, b)) in segments.iter().enumerate() {
            for x in cell(a.x.min(b.x))..=cell(a.x.max(b.x)) {
                for y in cell(a.y.min(b.y))..=cell(a.y.max(b.y)) {
                    cells.entry((x, y)).or_default().push(k);
                }
            }
        }
        Self { cells, segments }
    }

    /// The distance from `p` to the nearest segment, or infinity when no
    /// segment lies within [`CELL_MM`].
    pub fn distance(&self, p: Point) -> f64 {
        let (cx, cy) = (cell(p.x), cell(p.y));
        let mut best = f64::INFINITY;
        for x in cx - 1..=cx + 1 {
            for y in cy - 1..=cy + 1 {
                for &k in self.cells.get(&(x, y)).into_iter().flatten() {
                    let (a, b) = self.segments[k];
                    best = best.min(segment_distance(p, a, b));
                }
            }
        }
        if best <= CELL_MM { best } else { f64::INFINITY }
    }

    /// The distance from `p` to the nearest segment, searched over all of
    /// them when none lies within [`CELL_MM`].
    pub fn distance_any(&self, p: Point) -> f64 {
        let near = self.distance(p);
        if near.is_finite() {
            return near;
        }
        self.segments
            .iter()
            .map(|&(a, b)| segment_distance(p, a, b))
            .fold(f64::INFINITY, f64::min)
    }
}

/// The segments of `paths`, flattened at [`SOURCE_FLATTEN_MM`].
pub fn source_segments(paths: &[BezPath]) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    for p in paths {
        let (mut start, mut here) = (None::<Point>, None::<Point>);
        kurbo::flatten(p.iter(), SOURCE_FLATTEN_MM, |el| match el {
            PathEl::MoveTo(q) => {
                start = Some(q);
                here = Some(q);
            }
            PathEl::LineTo(q) => {
                if let Some(h) = here {
                    out.push((h, q));
                }
                here = Some(q);
            }
            PathEl::ClosePath => {
                if let (Some(h), Some(s)) = (here, start) {
                    out.push((h, s));
                }
                here = start;
            }
            PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
        });
    }
    out
}

/// The segments of a polyline.
pub fn polyline_segments(points: &[Point]) -> impl Iterator<Item = (Point, Point)> + '_ {
    points.windows(2).map(|w| (w[0], w[1]))
}

/// Points along `segments`, at most [`SAMPLE_STEP_MM`] apart, both ends
/// of each segment included.
fn samples(segments: &[(Point, Point)]) -> impl Iterator<Item = Point> + '_ {
    segments.iter().flat_map(|&(a, b)| {
        let n = (a.distance(b) / SAMPLE_STEP_MM).ceil().max(1.0) as usize;
        (0..=n).map(move |k| a.lerp(b, k as f64 / n as f64))
    })
}

/// The two-sided (Hausdorff) distance between the polyline segments
/// `line` and the source shapes `source` (as [`source_segments`] gives
/// them): (the largest distance of a point of `line` from the source, the
/// largest distance of a point of the source from `line`). Both sides are
/// sampled every [`SAMPLE_STEP_MM`].
pub fn two_sided(source: &[(Point, Point)], line: &[(Point, Point)]) -> (f64, f64) {
    let to_source = SegmentIndex::new(source.to_vec());
    let to_line = SegmentIndex::new(line.to_vec());
    let line_to_source = samples(line)
        .map(|q| to_source.distance_any(q))
        .fold(0.0, f64::max);
    let source_to_line = samples(source)
        .map(|q| to_line.distance_any(q))
        .fold(0.0, f64::max);
    (line_to_source, source_to_line)
}

/// The cut moves of `trace`: each move into a blade-down sample that cuts
/// a `Cut` toolpath (not waste), as tip segments.
pub fn cut_moves(toolpaths: &[Toolpath], trace: &SimTrace) -> Vec<(Point, Point)> {
    cut_moves_where(toolpaths, trace, |_| true)
}

/// [`cut_moves`] of the toolpaths whose index `of` accepts.
pub fn cut_moves_where(
    toolpaths: &[Toolpath],
    trace: &SimTrace,
    of: impl Fn(usize) -> bool,
) -> Vec<(Point, Point)> {
    trace
        .samples
        .windows(2)
        .filter(|w| {
            let x = &w[1];
            x.down
                && w[0].down
                && x.seg != WASTE_SEG
                && toolpaths[x.toolpath].kind == ToolpathKind::Cut
                && of(x.toolpath)
        })
        .map(|w| (w[0].tip, w[1].tip))
        .collect()
}

/// The end-to-end deviation of a simulated job from its source shapes
/// `source` (as [`source_segments`] gives them), both ways: (the largest
/// distance of a cut tip from the source, an over-cut; the largest
/// distance of a point of the source from the cut, an under-cut such as a
/// corner the tip never reaches). Only the cut moves of `Cut` toolpaths
/// count ([`cut_moves`]).
pub fn end_to_end(
    source: &[(Point, Point)],
    toolpaths: &[Toolpath],
    trace: &SimTrace,
) -> (f64, f64) {
    two_sided(source, &cut_moves(toolpaths, trace))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Circle, Shape};

    #[test]
    fn a_line_against_itself_is_zero() {
        let a = vec![(Point::new(0.0, 0.0), Point::new(10.0, 0.0))];
        let (x, y) = two_sided(&a, &a);
        assert!(x < 1e-12 && y < 1e-12, "{x} {y}");
    }

    #[test]
    fn both_sides_are_measured() {
        let source = vec![(Point::new(0.0, 0.0), Point::new(10.0, 0.0))];
        // A shorter line, 0.02 off: 0.02 from the source, but the source
        // reaches 2 mm past its end.
        let line = vec![(Point::new(0.0, 0.02), Point::new(8.0, 0.02))];
        let (over, under) = two_sided(&source, &line);
        assert!((over - 0.02).abs() < 1e-12, "{over}");
        assert!((under - (2.0f64.hypot(0.02))).abs() < 1e-9, "{under}");
    }

    #[test]
    fn source_circles_are_flattened_finely() {
        let c = Circle::new((50.0, 50.0), 5.0).to_path(1e-6);
        let segs = source_segments(&[c]);
        for &(a, b) in &segs {
            let mid = a.midpoint(b);
            let r = mid.distance(Point::new(50.0, 50.0));
            assert!((r - 5.0).abs() <= SOURCE_FLATTEN_MM + 1e-6, "{r}");
        }
        // Closed: the last segment ends where the first starts.
        assert!(segs.last().unwrap().1.distance(segs[0].0) < 1e-9);
    }
}
