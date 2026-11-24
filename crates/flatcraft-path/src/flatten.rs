//! Stage 1 and 4: flatten curves to lines and drop collinear points.

use kurbo::{BezPath, PathEl, Point};

/// Flatten all curves to lines. The output has only MoveTo, LineTo and ClosePath.
pub fn flatten(p: &BezPath, tol: f64) -> BezPath {
    let mut out = BezPath::new();
    kurbo::flatten(p.iter(), tol, |el| match el {
        PathEl::MoveTo(pt) => out.move_to(pt),
        PathEl::LineTo(pt) => out.line_to(pt),
        PathEl::ClosePath => out.close_path(),
        // kurbo::flatten only emits the three variants above
        PathEl::QuadTo(..) | PathEl::CurveTo(..) => unreachable!("flatten emitted a curve"),
    });
    out
}

/// Douglas-Peucker simplification: drop points that lie within `tol` of the
/// segment between the points kept around them. The first and last point are
/// always kept, and every dropped point stays within `tol` of the result, so
/// there is no drift on gentle curves. A spike is never dropped.
pub fn merge_collinear(points: &[Point], tol: f64) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((lo, hi)) = stack.pop() {
        let far = (lo + 1..hi)
            .map(|i| (i, segment_distance(points[lo], points[hi], points[i])))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, d)) = far
            && d >= tol
        {
            keep[i] = true;
            stack.push((lo, i));
            stack.push((i, hi));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// Distance from `p` to the segment `a`-`b`.
fn segment_distance(a: Point, b: Point, p: Point) -> f64 {
    let ab = b - a;
    let len_sq = ab.hypot2();
    if len_sq == 0.0 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Circle, ParamCurveNearest, PathEl, Shape};

    #[test]
    fn flatten_circle_within_tolerance() {
        let tol = 0.01;
        let circle = Circle::new((50.0, 50.0), 10.0).to_path(0.001);
        let flat = flatten(&circle, tol);
        let mut pts = vec![];
        for el in flat.elements() {
            match el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => pts.push(*p),
                PathEl::ClosePath => {}
                other => panic!("curve left in flattened path: {other:?}"),
            }
        }
        assert!(pts.len() > 8);
        let c = Point::new(50.0, 50.0);
        for w in pts.windows(2) {
            let r = c.distance(w[0]);
            assert!((r - 10.0).abs() <= tol + 1e-9, "vertex radius {r}");
            let r = c.distance(w[0].midpoint(w[1]));
            assert!((r - 10.0).abs() <= tol + 1e-9, "midpoint radius {r}");
        }
    }

    #[test]
    fn merge_collinear_reduces_line() {
        let pts: Vec<Point> = (0..100).map(|i| Point::new(i as f64, 0.0)).collect();
        let m = merge_collinear(&pts, 0.001);
        assert_eq!(m, vec![Point::new(0.0, 0.0), Point::new(99.0, 0.0)]);
    }

    #[test]
    fn merge_collinear_keeps_corners_and_ring_ends() {
        let ring = vec![
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
            Point::new(0.0, 0.0),
        ];
        let m = merge_collinear(&ring, 0.001);
        assert_eq!(m.len(), 5);
        assert_eq!(m.first(), m.last());
    }

    #[test]
    fn merge_collinear_does_not_drift_on_curves() {
        let tol = 0.01;
        let pts: Vec<Point> = (0..=200)
            .map(|i| {
                let a = i as f64 * 0.005;
                Point::new(10.0 * a.cos(), 10.0 * a.sin())
            })
            .collect();
        let m = merge_collinear(&pts, tol);
        assert!(m.len() < pts.len());
        // every dropped point stays near the simplified polyline
        for p in &pts {
            let d = m
                .windows(2)
                .map(|w| {
                    kurbo::Line::new(w[0], w[1])
                        .nearest(*p, 1e-9)
                        .distance_sq
                        .sqrt()
                })
                .fold(f64::MAX, f64::min);
            assert!(d <= tol + 1e-9, "drifted {d}");
        }
    }

    #[test]
    fn merge_collinear_keeps_spikes() {
        let pts = vec![
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(5.0, 0.0),
        ];
        assert_eq!(merge_collinear(&pts, 0.01), pts);
    }
}
