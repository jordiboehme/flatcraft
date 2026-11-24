//! Stage 5: nearest-neighbour ordering.

use kurbo::Point;

use crate::plan::Toolpath;

/// Greedy nearest-neighbour order, beginning at `start`. An open path is
/// reversed when its end is nearer than its start. A closed path keeps its
/// source start and direction. Ties go to the earlier path.
pub fn nearest_neighbour(paths: Vec<Toolpath>, start: Point) -> Vec<Toolpath> {
    let mut rest = paths;
    let mut out = Vec::with_capacity(rest.len());
    let mut here = start;
    while !rest.is_empty() {
        let mut best = (f64::INFINITY, 0usize, false);
        for (i, t) in rest.iter().enumerate() {
            let (Some(&first), Some(&last)) = (t.points.first(), t.points.last()) else {
                continue;
            };
            let d_start = here.distance(first);
            if d_start < best.0 {
                best = (d_start, i, false);
            }
            if !t.closed {
                let d_end = here.distance(last);
                if d_end < best.0 {
                    best = (d_end, i, true);
                }
            }
        }
        let mut t = rest.remove(best.1);
        if best.2 {
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
}
