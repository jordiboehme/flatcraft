//! Stage 3: split flattened paths into polylines and join near-touching ends.

use kurbo::{BezPath, PathEl, Point};

#[derive(Debug, Clone, PartialEq)]
pub struct Polyline {
    pub points: Vec<Point>,
    pub closed: bool,
}

/// Segments shorter than this are zero-length.
const ZERO_LEN: f64 = 1e-9;

/// Split a flattened path into polylines. Zero-length segments are removed,
/// open subpaths whose ends meet within `join_tol` are joined (greedily, in
/// source order), and rings are closed (last point equals first) when
/// explicitly closed or when their ends meet within `join_tol`.
pub fn clean(p: &BezPath, join_tol: f64) -> Vec<Polyline> {
    let mut lines = split_subpaths(p);
    join_open(&mut lines, join_tol);
    for line in lines.iter_mut().filter(|l| !l.closed) {
        close_if_ring(line, join_tol);
    }
    lines
}

fn split_subpaths(p: &BezPath) -> Vec<Polyline> {
    let mut lines = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    let mut start = Point::ZERO;
    let finish = |cur: &mut Vec<Point>, explicit: bool, lines: &mut Vec<Polyline>| {
        let mut points = std::mem::take(cur);
        if explicit && points.len() >= 2 && points[0].distance(points[points.len() - 1]) < ZERO_LEN
        {
            points.pop();
        }
        if explicit && points.len() >= 3 {
            points.push(points[0]);
            lines.push(Polyline {
                points,
                closed: true,
            });
        } else if points.len() >= 2 {
            lines.push(Polyline {
                points,
                closed: false,
            });
        }
    };
    let push = |cur: &mut Vec<Point>, pt: Point| {
        if cur.last().is_none_or(|l| l.distance(pt) >= ZERO_LEN) {
            cur.push(pt);
        }
    };
    for el in p.elements() {
        match *el {
            PathEl::MoveTo(pt) => {
                finish(&mut cur, false, &mut lines);
                start = pt;
                cur.push(pt);
            }
            PathEl::LineTo(pt) | PathEl::QuadTo(_, pt) | PathEl::CurveTo(_, _, pt) => {
                if cur.is_empty() {
                    cur.push(start);
                }
                push(&mut cur, pt);
            }
            PathEl::ClosePath => finish(&mut cur, true, &mut lines),
        }
    }
    finish(&mut cur, false, &mut lines);
    lines
}

/// Join open polylines whose ends meet within `tol`, reversing as needed,
/// until no join applies. The merged line keeps the earlier source position.
fn join_open(lines: &mut Vec<Polyline>, tol: f64) {
    'again: loop {
        for i in 0..lines.len() {
            if lines[i].closed {
                continue;
            }
            for j in (i + 1)..lines.len() {
                if lines[j].closed {
                    continue;
                }
                if let Some(merged) = try_join(&lines[i].points, &lines[j].points, tol) {
                    lines[i].points = merged;
                    lines.remove(j);
                    continue 'again;
                }
            }
        }
        return;
    }
}

fn try_join(a: &[Point], b: &[Point], tol: f64) -> Option<Vec<Point>> {
    let (a_first, a_last) = (a[0], a[a.len() - 1]);
    let (b_first, b_last) = (b[0], b[b.len() - 1]);
    let (mut head, tail): (Vec<Point>, Vec<Point>) = if a_last.distance(b_first) <= tol {
        (a.to_vec(), b.to_vec())
    } else if a_last.distance(b_last) <= tol {
        (a.to_vec(), b.iter().rev().copied().collect())
    } else if a_first.distance(b_first) <= tol {
        (a.iter().rev().copied().collect(), b.to_vec())
    } else if a_first.distance(b_last) <= tol {
        (b.to_vec(), a.to_vec())
    } else {
        return None;
    };
    // the shared point is kept once, from the head
    head.extend_from_slice(&tail[1..]);
    Some(head)
}

fn close_if_ring(line: &mut Polyline, tol: f64) {
    let n = line.points.len();
    // at least a triangle: three distinct vertices plus the repeated first point
    if n >= 4 && line.points[0].distance(line.points[n - 1]) <= tol {
        line.points[n - 1] = line.points[0];
        line.closed = true;
    }
}

/// Length of the polyline through `points`.
pub fn length(points: &[Point]) -> f64 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(pts: &[(f64, f64)], close: bool) -> BezPath {
        let mut p = BezPath::new();
        p.move_to(pts[0]);
        for &q in &pts[1..] {
            p.line_to(q);
        }
        if close {
            p.close_path();
        }
        p
    }

    #[test]
    fn clean_marks_near_closed() {
        let near = poly(
            &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.005)],
            false,
        );
        let r = clean(&near, 0.01);
        assert_eq!(r.len(), 1);
        assert!(r[0].closed);
        assert_eq!(r[0].points.first(), r[0].points.last());
        let far = poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 0.02)], false);
        let r = clean(&far, 0.01);
        assert_eq!(r.len(), 1);
        assert!(!r[0].closed);
        assert_eq!(r[0].points.len(), 4);
    }

    #[test]
    fn clean_closes_explicit_close_path() {
        let r = clean(&poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true), 0.01);
        assert!(r[0].closed);
        assert_eq!(r[0].points.len(), 4);
        assert_eq!(r[0].points.first(), r[0].points.last());
    }

    #[test]
    fn clean_drops_zero_length_segments() {
        let p = poly(
            &[(0.0, 0.0), (0.0, 0.0), (5.0, 0.0), (5.0, 0.0), (5.0, 5.0)],
            false,
        );
        let r = clean(&p, 0.01);
        assert_eq!(r.len(), 1);
        assert_eq!(
            r[0].points,
            vec![
                Point::new(0.0, 0.0),
                Point::new(5.0, 0.0),
                Point::new(5.0, 5.0)
            ]
        );
    }

    #[test]
    fn clean_joins_touching_subpaths() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        // second half is drawn backwards and misses by 0.005
        p.move_to((0.0, 0.005));
        p.line_to((0.0, 10.0));
        p.line_to((10.0, 10.0));
        let r = clean(&p, 0.01);
        assert_eq!(r.len(), 1, "{r:?}");
        assert!(r[0].closed);
        assert_eq!(r[0].points.first(), r[0].points.last());
        assert_eq!(r[0].points.len(), 5);
    }

    #[test]
    fn clean_keeps_far_subpaths_apart() {
        let mut p = poly(&[(0.0, 0.0), (1.0, 0.0)], false);
        p.move_to((5.0, 5.0));
        p.line_to((6.0, 5.0));
        assert_eq!(clean(&p, 0.01).len(), 2);
    }

    #[test]
    fn clean_drops_single_points() {
        let mut p = BezPath::new();
        p.move_to((1.0, 1.0));
        p.move_to((2.0, 2.0));
        p.line_to((3.0, 2.0));
        let r = clean(&p, 0.01);
        assert_eq!(r.len(), 1);
    }
}
