//! Stage 5: nearest-neighbour ordering, inside first.

use kurbo::{ParamCurveNearest as _, Point, Rect};

use crate::plan::Toolpath;

/// Even-odd test of `p` against the polygon `ring`. `None` when `p` lies
/// on (or within `EPS` of) the boundary, where the answer is ambiguous.
fn inside_ring(p: Point, ring: &[Point]) -> Option<bool> {
    #[cfg(test)]
    tests::INSIDE_CALLS.with(|c| c.set(c.get() + 1));
    const EPS: f64 = 1e-9;
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        if kurbo::Line::new(a, b).nearest(p, 0.0).distance_sq <= EPS * EPS {
            return None;
        }
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                inside = !inside;
            }
        }
    }
    Some(inside)
}

/// Absolute shoelace area of a ring.
fn area(points: &[Point]) -> f64 {
    let n = points.len();
    let twice: f64 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    (twice / 2.0).abs()
}

fn bbox(points: &[Point]) -> Option<Rect> {
    let (first, rest) = points.split_first()?;
    Some(
        rest.iter()
            .fold(Rect::from_points(*first, *first), |r, p| r.union_pt(*p)),
    )
}

/// How many vertices of the inner ring are tried before giving up.
const PROBES: usize = 3;

/// True if closed ring `inner` lies in closed ring `outer`: its
/// box lies in the outer box, it is smaller, and one of its vertices is
/// inside by even-odd. The first of up to [`PROBES`] vertices that is not
/// on the outer boundary decides.
fn nested(inner: &[Point], ib: Rect, ia: f64, outer: &[Point], ob: Rect, oa: f64) -> bool {
    if ib.x0 < ob.x0 || ib.y0 < ob.y0 || ib.x1 > ob.x1 || ib.y1 > ob.y1 || ia >= oa {
        return false;
    }
    inner
        .iter()
        .take(PROBES)
        .find_map(|p| inside_ring(*p, outer))
        .unwrap_or(false)
}

/// For each path, how many closed paths lie inside it, and for each path
/// the paths it lies inside. Open paths neither block nor are blocked.
/// One even-odd test per box-nested pair keeps this at O(n^2 * v).
fn containment(paths: &[Toolpath]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let rings: Vec<Option<(Rect, f64)>> = paths
        .iter()
        .map(|t| {
            if t.closed {
                bbox(&t.points).map(|b| (b, area(&t.points)))
            } else {
                None
            }
        })
        .collect();
    let mut inside_count = vec![0; paths.len()];
    let mut parents = vec![Vec::new(); paths.len()];
    for (i, outer) in paths.iter().enumerate() {
        let Some((ob, oa)) = rings[i] else { continue };
        for (j, inner) in paths.iter().enumerate() {
            let Some((ib, ia)) = rings[j] else { continue };
            if i != j && nested(&inner.points, ib, ia, &outer.points, ob, oa) {
                inside_count[i] += 1;
                parents[j].push(i);
            }
        }
    }
    (inside_count, parents)
}

/// Greedy nearest-neighbour order, beginning at `start`. An open path is
/// reversed when its end is nearer than its start. A closed path keeps its
/// source start and direction. Ties go to the earlier path.
///
/// Inside first: a closed path waits while an uncut closed path lies inside
/// it, so holes and inner parts are cut before the outline that frees them
/// (once an outline is cut, the part inside it can shift on the mat). If
/// every remaining path waits (which proper nesting never gives), the
/// nearest one goes. This is a minimal rule; a containment tree can replace
/// it later. Containment is found with at most `PROBES` even-odd tests
/// per pair of closed paths whose boxes nest, so deep nesting stays fast.
pub fn nearest_neighbour(paths: Vec<Toolpath>, start: Point) -> Vec<Toolpath> {
    let (mut waiting_on, parents) = containment(&paths);
    let mut slots: Vec<Option<Toolpath>> = paths.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(slots.len());
    let mut here = start;
    loop {
        // (distance, index, reversed) of the best free and the best of all.
        let mut best: Option<(f64, usize, bool)> = None;
        let mut any: Option<(f64, usize, bool)> = None;
        for (i, slot) in slots.iter().enumerate() {
            let Some(t) = slot else { continue };
            let (Some(&first), Some(&last)) = (t.points.first(), t.points.last()) else {
                any.get_or_insert((f64::INFINITY, i, false));
                continue;
            };
            let free = waiting_on[i] == 0;
            let mut consider = |d: f64, rev: bool| {
                if any.is_none_or(|b| d < b.0) {
                    any = Some((d, i, rev));
                }
                if free && best.is_none_or(|b| d < b.0) {
                    best = Some((d, i, rev));
                }
            };
            consider(here.distance(first), false);
            if !t.closed {
                consider(here.distance(last), true);
            }
        }
        let Some((_, i, reversed)) = best.or(any) else {
            break;
        };
        let mut t = slots[i].take().expect("picked a remaining path");
        for p in &parents[i] {
            waiting_on[*p] = waiting_on[*p].saturating_sub(1);
        }
        if reversed {
            t.points.reverse();
        }
        if let Some(&last) = t.points.last() {
            here = last;
        }
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Even-odd tests run on this thread (`inside_ring`).
        pub(super) static INSIDE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    use crate::plan::PathRef;
    use flatcraft_core::document::ObjectId;

    fn tp(id: u64, pts: &[(f64, f64)], closed: bool) -> Toolpath {
        Toolpath {
            points: pts.iter().map(|&p| p.into()).collect(),
            closed,
            source: PathRef {
                object: ObjectId(id),
                path_index: 0,
            },
            pass: 1,
        }
    }

    #[test]
    fn open_path_starts_at_nearest_end() {
        let out = nearest_neighbour(
            vec![tp(1, &[(100.0, 0.0), (10.0, 0.0)], false)],
            Point::ZERO,
        );
        assert_eq!(out[0].points[0], Point::new(10.0, 0.0));
        assert_eq!(out[0].points[1], Point::new(100.0, 0.0));
    }

    #[test]
    fn closed_path_keeps_start_and_direction() {
        let sq = [(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 10.0)];
        let out = nearest_neighbour(vec![tp(1, &sq, true)], Point::ZERO);
        assert_eq!(out[0].points, tp(1, &sq, true).points);
    }

    #[test]
    fn next_path_is_chosen_from_previous_end() {
        let a = tp(1, &[(0.0, 1.0), (50.0, 1.0)], false);
        let far = tp(2, &[(500.0, 0.0), (400.0, 0.0)], false);
        let near_end = tp(3, &[(60.0, 0.0), (51.0, 0.0)], false);
        let out = nearest_neighbour(vec![far, a, near_end], Point::ZERO);
        let ids: Vec<u64> = out.iter().map(|t| t.source.object.0).collect();
        assert_eq!(ids, vec![1, 3, 2]);
        assert_eq!(out[1].points[0], Point::new(51.0, 0.0));
        assert_eq!(out[2].points[0], Point::new(400.0, 0.0));
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(nearest_neighbour(vec![], Point::ZERO).is_empty());
    }

    fn ring(id: u64, x0: f64, y0: f64, x1: f64, y1: f64) -> Toolpath {
        tp(
            id,
            &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)],
            true,
        )
    }

    fn ids(out: &[Toolpath]) -> Vec<u64> {
        out.iter().map(|t| t.source.object.0).collect()
    }

    #[test]
    fn hole_is_cut_before_outline() {
        // The outline's start is nearer to the origin than the hole's.
        let outer = ring(1, 10.0, 10.0, 60.0, 60.0);
        let hole = ring(2, 30.0, 30.0, 40.0, 40.0);
        let out = nearest_neighbour(vec![outer, hole], Point::ZERO);
        assert_eq!(ids(&out), vec![2, 1]);
    }

    #[test]
    fn three_deep_nesting_cuts_inside_out() {
        let a = ring(1, 0.0, 0.0, 100.0, 100.0);
        let b = ring(2, 10.0, 10.0, 90.0, 90.0);
        let c = ring(3, 40.0, 40.0, 60.0, 60.0);
        let out = nearest_neighbour(vec![a, b, c], Point::ZERO);
        assert_eq!(ids(&out), vec![3, 2, 1]);
    }

    #[test]
    fn two_siblings_come_before_their_parent() {
        let parent = ring(1, 0.0, 0.0, 100.0, 50.0);
        let left = ring(2, 10.0, 10.0, 30.0, 30.0);
        let right = ring(3, 60.0, 10.0, 80.0, 30.0);
        let out = nearest_neighbour(vec![parent, right, left], Point::ZERO);
        assert_eq!(ids(&out), vec![2, 3, 1]);
    }

    #[test]
    fn open_paths_are_not_held_back() {
        let outer = ring(1, 10.0, 10.0, 60.0, 60.0);
        // An open line inside the ring does not block it.
        let line = tp(2, &[(30.0, 30.0), (40.0, 40.0)], false);
        let out = nearest_neighbour(vec![outer, line], Point::ZERO);
        assert_eq!(ids(&out), vec![1, 2]);
    }

    #[test]
    fn identical_rings_fall_back_to_the_nearest() {
        let a = ring(1, 10.0, 10.0, 60.0, 60.0);
        let b = ring(2, 10.0, 10.0, 60.0, 60.0);
        let out = nearest_neighbour(vec![a, b], Point::ZERO);
        assert_eq!(out.len(), 2);
        assert_eq!(ids(&out), vec![1, 2]);
    }

    /// A circle of `n` vertices, closed by repeating the first.
    fn circle(id: u64, c: (f64, f64), r: f64, n: usize) -> Toolpath {
        let mut pts: Vec<(f64, f64)> = (0..n)
            .map(|k| {
                let a = k as f64 / n as f64 * std::f64::consts::TAU;
                (c.0 + r * a.cos(), c.1 + r * a.sin())
            })
            .collect();
        pts.push(pts[0]);
        tp(id, &pts, true)
    }

    #[test]
    fn deep_nesting_orders_fast() {
        // 200 concentric rings of 400 vertices. The guard is the number of
        // even-odd tests, so it holds on any machine: at most PROBES per
        // pair whose boxes nest. No wall-clock limit: the count is the check.
        let rings: Vec<Toolpath> = (1..=200)
            .map(|k| circle(k, (300.0, 300.0), k as f64, 400))
            .collect();
        let boxes: Vec<Rect> = rings.iter().map(|t| bbox(&t.points).unwrap()).collect();
        let box_nested = boxes
            .iter()
            .enumerate()
            .flat_map(|(i, o)| {
                boxes.iter().enumerate().filter(move |(j, b)| {
                    i != *j && b.x0 >= o.x0 && b.y0 >= o.y0 && b.x1 <= o.x1 && b.y1 <= o.y1
                })
            })
            .count();
        INSIDE_CALLS.with(|c| c.set(0));
        let out = nearest_neighbour(rings, Point::ZERO);
        let calls = INSIDE_CALLS.with(|c| c.get());
        assert_eq!(ids(&out), (1..=200).collect::<Vec<u64>>());
        assert_eq!(box_nested, 200 * 199 / 2);
        assert!(
            calls <= PROBES * box_nested,
            "{calls} even-odd tests for {box_nested} nested pairs"
        );
    }

    #[test]
    fn a_ring_touching_the_outline_still_counts_as_inside() {
        // The inner ring shares its first vertex's x with the outline's
        // left edge, so a vertex is tried that is clearly inside.
        let outer = ring(1, 10.0, 10.0, 60.0, 60.0);
        let inner = tp(
            2,
            &[(10.0, 30.0), (30.0, 25.0), (30.0, 35.0), (10.0, 30.0)],
            true,
        );
        let out = nearest_neighbour(vec![outer, inner], Point::ZERO);
        assert_eq!(ids(&out), vec![2, 1]);
    }

    #[test]
    fn a_ring_beside_another_in_its_box_does_not_block() {
        // An L-shaped outline whose box holds a square that lies outside
        // the L itself.
        let l = tp(
            1,
            &[
                (0.0, 0.0),
                (100.0, 0.0),
                (100.0, 20.0),
                (20.0, 20.0),
                (20.0, 100.0),
                (0.0, 100.0),
                (0.0, 0.0),
            ],
            true,
        );
        let square = ring(2, 50.0, 50.0, 70.0, 70.0);
        let out = nearest_neighbour(vec![l, square], Point::ZERO);
        assert_eq!(ids(&out), vec![1, 2]);
    }
}
