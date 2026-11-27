//! A uniform grid of 1 mm cells over line segments, for the nearest-segment
//! and crossing queries of the metrics.

use std::collections::HashMap;

use kurbo::{Point, Rect};

/// The edge of a grid cell, in millimetres.
const CELL_MM: f64 = 1.0;

/// Segments are indexed in pieces no longer than this, so a long diagonal
/// fills a band of cells instead of its whole box.
const PIECE_MM: f64 = CELL_MM / 2.0;

/// Cells are grown by this, so a point on a cell line finds the segment
/// in its own cell.
const EPS: f64 = 1e-9;

/// A query point more than this many cells outside the indexed cells
/// checks every segment once instead of walking rings of empty cells.
const FAR_CELLS: i64 = 64;

#[cfg(test)]
thread_local! {
    /// Grid cells visited by `nearest`, for the cost tests.
    pub(crate) static CELL_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Segment distances computed by `nearest`, for the cost tests.
    pub(crate) static SEGMENT_CHECKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Calls of `nearest`, for the cost tests.
    pub(crate) static QUERIES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn cell(v: f64) -> i64 {
    (v / CELL_MM).floor() as i64
}

/// The distance from `p` to the segment `a`-`b` (a point when `a == b`).
pub(crate) fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len_sq = ab.hypot2();
    if len_sq == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + t * ab)
}

/// Line segments with a tag each, indexed in 1 mm cells.
#[derive(Debug, Default)]
pub(crate) struct SegmentGrid {
    segs: Vec<(Point, Point, usize)>,
    cells: HashMap<(i64, i64), Vec<u32>>,
    /// The indexed cells: (col0, row0, col1, row1).
    extent: Option<(i64, i64, i64, i64)>,
}

impl SegmentGrid {
    /// Index the segment `a`-`b` under `tag`. Non-finite segments are skipped.
    pub(crate) fn insert(&mut self, a: Point, b: Point, tag: usize) {
        if !(a.is_finite() && b.is_finite()) {
            return;
        }
        let id = self.segs.len() as u32;
        self.segs.push((a, b, tag));
        let pieces = (a.distance(b) / PIECE_MM).ceil().max(1.0) as usize;
        for k in 0..pieces {
            let p0 = a.lerp(b, k as f64 / pieces as f64);
            let p1 = a.lerp(b, (k + 1) as f64 / pieces as f64);
            let r = Rect::from_points(p0, p1).inflate(EPS, EPS);
            for row in cell(r.y0)..=cell(r.y1) {
                for col in cell(r.x0)..=cell(r.x1) {
                    let ids = self.cells.entry((col, row)).or_default();
                    // One segment is indexed before the next, so a repeat
                    // can only be the last entry.
                    if ids.last() != Some(&id) {
                        ids.push(id);
                    }
                    self.extent = Some(match self.extent {
                        None => (col, row, col, row),
                        Some((c0, r0, c1, r1)) => {
                            (c0.min(col), r0.min(row), c1.max(col), r1.max(row))
                        }
                    });
                }
            }
        }
    }

    /// The segment `id`: start, end and tag.
    pub(crate) fn segment(&self, id: u32) -> (Point, Point, usize) {
        self.segs[id as usize]
    }

    /// The distance from `p` to the nearest segment whose tag `take`
    /// accepts, `f64::INFINITY` when there is none. The search may stop
    /// early with any value at or below `limit` once one is found, so
    /// `limit = 0` gives the exact distance.
    pub(crate) fn nearest(&self, p: Point, take: impl Fn(usize) -> bool, limit: f64) -> f64 {
        #[cfg(test)]
        QUERIES.with(|c| c.set(c.get() + 1));
        let Some((c0, r0, c1, r1)) = self.extent else {
            return f64::INFINITY;
        };
        if !p.is_finite() {
            return f64::INFINITY;
        }
        let mut best = f64::INFINITY;
        let check = |id: u32, best: &mut f64| {
            #[cfg(test)]
            SEGMENT_CHECKS.with(|c| c.set(c.get() + 1));
            let (a, b, tag) = self.segs[id as usize];
            if take(tag) {
                *best = best.min(segment_distance(p, a, b));
            }
        };
        let (pc, pr) = (cell(p.x), cell(p.y));
        // The ring of cells around p's cell that first touches the extent,
        // and the one that covers all of it.
        let k_min = (c0 - pc).max(pc - c1).max(r0 - pr).max(pr - r1).max(0);
        if k_min > FAR_CELLS {
            for id in 0..self.segs.len() as u32 {
                check(id, &mut best);
            }
            return best;
        }
        let k_max = (pc - c0).max(c1 - pc).max(pr - r0).max(r1 - pr);
        for k in k_min..=k_max {
            // A cell k rings away lies at least (k - 1) cells from p.
            if best <= limit || (k >= 1 && best <= (k - 1) as f64 * CELL_MM) {
                break;
            }
            for row in (pr - k).max(r0)..=(pr + k).min(r1) {
                let mut visit = |col: i64| {
                    #[cfg(test)]
                    CELL_VISITS.with(|c| c.set(c.get() + 1));
                    for &id in self.cells.get(&(col, row)).into_iter().flatten() {
                        check(id, &mut best);
                    }
                };
                if row == pr - k || row == pr + k {
                    ((pc - k).max(c0)..=(pc + k).min(c1)).for_each(&mut visit);
                } else {
                    [pc - k, pc + k]
                        .into_iter()
                        .filter(|c| (c0..=c1).contains(c))
                        .for_each(&mut visit);
                }
            }
        }
        best
    }

    /// The ids of the segments indexed in the cells along `a`-`b`, sorted,
    /// each once. Every segment that meets `a`-`b` is among them.
    pub(crate) fn along(&self, a: Point, b: Point) -> Vec<u32> {
        let mut ids = Vec::new();
        if self.extent.is_none() || !(a.is_finite() && b.is_finite()) {
            return ids;
        }
        let pieces = (a.distance(b) / PIECE_MM).ceil().max(1.0) as usize;
        for k in 0..pieces {
            let p0 = a.lerp(b, k as f64 / pieces as f64);
            let p1 = a.lerp(b, (k + 1) as f64 / pieces as f64);
            let r = Rect::from_points(p0, p1).inflate(EPS, EPS);
            for row in cell(r.y0)..=cell(r.y1) {
                for col in cell(r.x0)..=cell(r.x1) {
                    ids.extend(self.cells.get(&(col, row)).into_iter().flatten());
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_matches_brute_force() {
        let mut g = SegmentGrid::default();
        let segs = [
            (Point::new(0.0, 0.0), Point::new(10.0, 3.0)),
            (Point::new(4.0, -6.0), Point::new(4.5, 8.0)),
            (Point::new(-3.0, 2.0), Point::new(-3.0, 2.0)),
            (Point::new(20.0, 20.0), Point::new(12.0, 25.0)),
        ];
        for (i, &(a, b)) in segs.iter().enumerate() {
            g.insert(a, b, i);
        }
        for ix in -40..=60 {
            for iy in -40..=60 {
                let p = Point::new(ix as f64 * 0.73 - 8.0, iy as f64 * 0.61 - 9.0);
                let brute = segs
                    .iter()
                    .map(|&(a, b)| segment_distance(p, a, b))
                    .fold(f64::INFINITY, f64::min);
                let got = g.nearest(p, |_| true, 0.0);
                assert!((got - brute).abs() < 1e-12, "{p:?}: {got} vs {brute}");
                let odd = segs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| i % 2 == 1)
                    .map(|(_, &(a, b))| segment_distance(p, a, b))
                    .fold(f64::INFINITY, f64::min);
                let got = g.nearest(p, |t| t % 2 == 1, 0.0);
                assert!((got - odd).abs() < 1e-12, "{p:?}: {got} vs {odd}");
            }
        }
        // far away, and an empty grid
        let far = Point::new(500.0, -300.0);
        let brute = segs
            .iter()
            .map(|&(a, b)| segment_distance(far, a, b))
            .fold(f64::INFINITY, f64::min);
        assert!((g.nearest(far, |_| true, 0.0) - brute).abs() < 1e-9);
        assert_eq!(
            SegmentGrid::default().nearest(Point::ZERO, |_| true, 0.0),
            f64::INFINITY
        );
        assert_eq!(g.nearest(Point::ZERO, |_| false, 0.0), f64::INFINITY);
    }

    #[test]
    fn along_finds_every_crossing_segment() {
        let mut g = SegmentGrid::default();
        g.insert(Point::new(0.0, -5.0), Point::new(0.0, 5.0), 0);
        g.insert(Point::new(30.0, -5.0), Point::new(30.0, 5.0), 1);
        let ids = g.along(Point::new(-3.0, 0.3), Point::new(3.0, 0.9));
        assert_eq!(ids, vec![0]);
        assert_eq!(g.segment(0).2, 0);
    }
}
