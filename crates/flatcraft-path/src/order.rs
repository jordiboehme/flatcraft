//! Stage 5: nearest-neighbour ordering, inside first.

use kurbo::{Point, Rect};

use crate::plan::Toolpath;

/// Even-odd test of `p` against the polygon `ring`.
fn inside_ring(p: Point, ring: &[Point]) -> bool {
    let mut inside = false;
    let n = ring.len();
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                inside = !inside;
            }
        }
    }
    inside
}

fn bbox(points: &[Point]) -> Option<Rect> {
    let (first, rest) = points.split_first()?;
    Some(
        rest.iter()
            .fold(Rect::from_points(*first, *first), |r, p| r.union_pt(*p)),
    )
}

/// For each path, the closed paths that lie fully inside it (every vertex
/// inside by even-odd, after a bounding-box check). Open paths neither
/// block nor are blocked.
fn blockers(paths: &[Toolpath]) -> Vec<Vec<usize>> {
    let boxes: Vec<Option<Rect>> = paths
        .iter()
        .map(|t| if t.closed { bbox(&t.points) } else { None })
        .collect();
    let mut out = vec![Vec::new(); paths.len()];
    for (i, outer) in paths.iter().enumerate() {
        let Some(ob) = boxes[i] else { continue };
        for (j, inner) in paths.iter().enumerate() {
            let Some(ib) = boxes[j] else { continue };
            if i == j || ib.x0 < ob.x0 || ib.y0 < ob.y0 || ib.x1 > ob.x1 || ib.y1 > ob.y1 {
                continue;
            }
            if inner.points.iter().all(|p| inside_ring(*p, &outer.points)) {
                out[i].push(j);
            }
        }
    }
    out
}

/// Greedy nearest-neighbour order, beginning at `start`. An open path is
/// reversed when its end is nearer than its start. A closed path keeps its
/// source start and direction. Ties go to the earlier path.
///
/// Inside first (spec 8.3, ruling R32): a closed path waits while an uncut
/// closed path lies fully inside it, so holes and inner parts are cut
/// before the outline that frees them. If every remaining path waits
/// (rings inside each other, which proper nesting never gives), the
/// nearest one goes. This is the minimal rule for M1; the M2 plan (Tasks 2
/// and 5) replaces it with a containment tree.
pub fn nearest_neighbour(paths: Vec<Toolpath>, start: Point) -> Vec<Toolpath> {
    let blocked_by = blockers(&paths);
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
            let free = blocked_by[i].iter().all(|j| slots[*j].is_none());
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
}
