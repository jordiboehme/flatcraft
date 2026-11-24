//! Stage 6: overcut for closed rings.

use kurbo::Point;

/// `points` is a closed ring (last equals first). Appends the points along the
/// ring from its start for `overcut_mm`, ending on an interpolated point.
/// The overcut never exceeds one full lap.
pub fn apply(points: &[Point], overcut_mm: f64) -> Vec<Point> {
    let mut out = points.to_vec();
    if overcut_mm <= 0.0 || points.len() < 2 {
        return out;
    }
    let perimeter = crate::clean::length(points);
    let mut remaining = overcut_mm.min(perimeter);
    for w in points.windows(2) {
        let d = w[0].distance(w[1]);
        if d == 0.0 {
            continue;
        }
        if remaining <= d {
            out.push(w[0].lerp(w[1], remaining / d));
            break;
        }
        out.push(w[1]);
        remaining -= d;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::length;
    use proptest::prelude::*;

    fn square() -> Vec<Point> {
        vec![
            Point::new(5.0, 5.0),
            Point::new(15.0, 5.0),
            Point::new(15.0, 15.0),
            Point::new(5.0, 15.0),
            Point::new(5.0, 5.0),
        ]
    }

    #[test]
    fn overcut_extends_closed_square() {
        let out = apply(&square(), 0.5);
        assert!((length(&out) - 40.5).abs() < 1e-9);
        assert_eq!(*out.last().unwrap(), Point::new(5.5, 5.0));
        assert_eq!(out[..5], square()[..]);
    }

    #[test]
    fn overcut_spans_corners() {
        let out = apply(&square(), 12.0);
        assert!((length(&out) - 52.0).abs() < 1e-9);
        assert_eq!(*out.last().unwrap(), Point::new(15.0, 7.0));
    }

    #[test]
    fn overcut_zero_is_identity() {
        assert_eq!(apply(&square(), 0.0), square());
    }

    #[test]
    fn overcut_is_capped_at_one_lap() {
        let out = apply(&square(), 1000.0);
        assert!((length(&out) - 80.0).abs() < 1e-9);
    }

    proptest! {
        #[test]
        fn overcut_adds_exact_length(n in 3usize..12, seed in 0u64..1000, mm in 0.0f64..5.0) {
            let mut pts: Vec<Point> = (0..n)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / n as f64;
                    let r = 10.0 + ((seed + i as u64) % 7) as f64;
                    Point::new(r * a.cos(), r * a.sin())
                })
                .collect();
            pts.push(pts[0]);
            let out = apply(&pts, mm);
            prop_assert!((length(&out) - (length(&pts) + mm)).abs() < 1e-6);
        }
    }
}
