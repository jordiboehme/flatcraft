//! Where moves, rotations and patterns put objects: pure functions that
//! the commands use and the window's ghost preview uses too, so the
//! preview and the result cannot differ.
//!
//! Units mm, origin the 0,0 of the grid printed on the mat, x right, y
//! down. Positive angles turn clockwise on screen (y points down), as
//! `object.rotate` does.

use kurbo::{Point, Rect, Vec2};

/// Sine and cosine of `degrees`, exact at multiples of 90 degrees, so a
/// quarter turn lands centres on round numbers.
pub fn sin_cos_deg(degrees: f64) -> (f64, f64) {
    let quarter = degrees / 90.0;
    if quarter == quarter.round() {
        match (quarter.round() as i64).rem_euclid(4) {
            0 => (0.0, 1.0),
            1 => (1.0, 0.0),
            2 => (0.0, -1.0),
            _ => (-1.0, 0.0),
        }
    } else {
        degrees.to_radians().sin_cos()
    }
}

/// `p` turned about `pivot` by `degrees`, clockwise on screen.
pub fn rotate_about(p: Point, pivot: Point, degrees: f64) -> Point {
    let (sin, cos) = sin_cos_deg(degrees);
    let d = p - pivot;
    pivot + Vec2::new(d.x * cos - d.y * sin, d.x * sin + d.y * cos)
}

/// The mean of `centres`: the point a group rotate or mirror turns about.
/// `None` for no centres.
pub fn mean_centre(centres: &[Point]) -> Option<Point> {
    let n = centres.len() as f64;
    (n > 0.0).then(|| {
        let sum = centres.iter().fold(Vec2::ZERO, |acc, c| acc + c.to_vec2());
        (sum / n).to_point()
    })
}

/// The centres after `object.rotate` by `degrees`. With `each` (or one
/// object) every centre stays; else they turn about their mean.
pub fn rotated_centres(centres: &[Point], degrees: f64, each: bool) -> Vec<Point> {
    let pivot = if each || centres.len() < 2 {
        None
    } else {
        mean_centre(centres)
    };
    centres
        .iter()
        .map(|c| pivot.map_or(*c, |p| rotate_about(*c, p, degrees)))
        .collect()
}

/// The offsets of `count` copies, each one `step` further than the one
/// before: copy k (1-based) moves by k times `step`. `object.duplicate`
/// with a count uses it.
pub fn step_offsets(step: Vec2, count: u32) -> Vec<Vec2> {
    (1..=count).map(|k| step * f64::from(k)).collect()
}

/// Grid cell (column, row) of copy `k` (1-based) in rows of `columns`
/// cells; the original is cell 0. `None` columns is one row.
pub fn grid_cell(columns: Option<u32>, k: u32) -> (u32, u32) {
    match columns {
        None => (k, 0),
        Some(c) => {
            let c = c.max(1);
            (k % c, k / c)
        }
    }
}

/// The offsets of `count` copies in a row (`columns` `None`) or in rows of
/// `columns` cells, laid out by the original's bounding box `bounds` plus
/// the gaps. `object.copies` uses it.
pub fn grid_offsets(
    bounds: Rect,
    columns: Option<u32>,
    count: u32,
    gap_x: f64,
    gap_y: f64,
) -> Vec<Vec2> {
    let (step_x, step_y) = (bounds.width() + gap_x, bounds.height() + gap_y);
    (1..=count)
        .map(|k| {
            let (col, row) = grid_cell(columns, k);
            // From the original each time, so rounding does not add up.
            Vec2::new(f64::from(col) * step_x, f64::from(row) * step_y)
        })
        .collect()
}

/// True when `total_deg` is a full turn: then the instances share it out
/// evenly, else the first and the last instance sit at the ends of the arc.
fn full_turn(total_deg: f64) -> bool {
    (total_deg.abs() - 360.0).abs() < 1e-9
}

/// The angles of the copies of a circular pattern of `instances` (the
/// original included, so `instances - 1` copies). A full turn (360 or
/// -360) divides by `instances`, so the last copy does not land on the
/// original; a smaller arc divides by `instances - 1`, so the last copy
/// sits at its end. 4 over 360 gives 90, 180, 270; 3 over 180 gives 90,
/// 180.
pub fn circular_angles(instances: u32, total_deg: f64) -> Vec<f64> {
    if instances < 2 {
        return Vec::new();
    }
    let parts = if full_turn(total_deg) {
        instances
    } else {
        instances - 1
    };
    let step = total_deg / f64::from(parts);
    (1..instances).map(|k| step * f64::from(k)).collect()
}

/// One copy of a circular pattern: where its centre goes and how much its
/// rotation grows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turned {
    pub center: Point,
    pub rotate_deg: f64,
}

/// The copies of an object with centre `center` in a circular pattern
/// about `pivot`: one per angle of [`circular_angles`]. With
/// `rotate_copies` each copy also turns by its angle, else it keeps the
/// original's rotation.
pub fn circular_copies(
    center: Point,
    pivot: Point,
    instances: u32,
    total_deg: f64,
    rotate_copies: bool,
) -> Vec<Turned> {
    circular_angles(instances, total_deg)
        .into_iter()
        .map(|a| Turned {
            center: rotate_about(center, pivot, a),
            rotate_deg: if rotate_copies { a } else { 0.0 },
        })
        .collect()
}

/// The default centre of a circular pattern: the centre of the common
/// bounding box of the objects (what the window shows as the selection
/// box). `None` for no boxes.
pub fn pattern_centre(boxes: &[Rect]) -> Option<Point> {
    boxes
        .iter()
        .copied()
        .reduce(|a, b| a.union(b))
        .map(|r| r.center())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a - b).hypot() < 1e-9
    }

    #[test]
    fn four_over_a_full_turn_are_quarter_turns() {
        assert_eq!(circular_angles(4, 360.0), vec![90.0, 180.0, 270.0]);
        let c = circular_copies(Point::new(10.0, 0.0), Point::new(0.0, 0.0), 4, 360.0, true);
        let want = [
            (Point::new(0.0, 10.0), 90.0),
            (Point::new(-10.0, 0.0), 180.0),
            (Point::new(0.0, -10.0), 270.0),
        ];
        assert_eq!(c.len(), 3);
        for (t, (p, a)) in c.iter().zip(want) {
            assert!(close(t.center, p), "{t:?} vs {p:?}");
            assert_eq!(t.rotate_deg, a);
        }
    }

    #[test]
    fn three_over_a_half_turn_end_at_the_arc_end() {
        assert_eq!(circular_angles(3, 180.0), vec![90.0, 180.0]);
        let c = circular_copies(
            Point::new(60.0, 50.0),
            Point::new(50.0, 50.0),
            3,
            180.0,
            false,
        );
        assert!(close(c[0].center, Point::new(50.0, 60.0)), "{c:?}");
        assert!(close(c[1].center, Point::new(40.0, 50.0)), "{c:?}");
        assert!(c.iter().all(|t| t.rotate_deg == 0.0), "rotate copies off");
    }

    #[test]
    fn odd_angles_and_a_full_turn_backwards() {
        let a = circular_angles(6, 360.0);
        assert_eq!(a.len(), 5);
        assert!((a[0] - 60.0).abs() < 1e-12);
        assert_eq!(circular_angles(4, -360.0), vec![-90.0, -180.0, -270.0]);
        assert!(circular_angles(1, 360.0).is_empty());
    }

    #[test]
    fn rotated_centres_turn_about_the_mean_or_stay() {
        let c = [Point::new(0.0, 0.0), Point::new(10.0, 0.0)];
        let g = rotated_centres(&c, 90.0, false);
        assert!(close(g[0], Point::new(5.0, -5.0)), "{g:?}");
        assert!(close(g[1], Point::new(5.0, 5.0)), "{g:?}");
        assert_eq!(rotated_centres(&c, 90.0, true), c.to_vec());
        assert_eq!(rotated_centres(&c[..1], 90.0, false), vec![c[0]]);
    }

    #[test]
    fn grid_and_step_offsets() {
        let b = Rect::new(0.0, 0.0, 10.0, 20.0);
        let o = grid_offsets(b, Some(2), 3, 1.0, 2.0);
        assert_eq!(
            o,
            vec![
                Vec2::new(11.0, 0.0),
                Vec2::new(0.0, 22.0),
                Vec2::new(11.0, 22.0)
            ]
        );
        assert_eq!(grid_offsets(b, None, 2, 5.0, 0.0)[1], Vec2::new(30.0, 0.0));
        assert_eq!(
            step_offsets(Vec2::new(1.5, -2.0), 2),
            vec![Vec2::new(1.5, -2.0), Vec2::new(3.0, -4.0)]
        );
    }

    #[test]
    fn pattern_centre_is_the_box_centre() {
        let boxes = [
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(20.0, 0.0, 40.0, 30.0),
        ];
        assert_eq!(pattern_centre(&boxes), Some(Point::new(20.0, 15.0)));
        assert_eq!(pattern_centre(&[]), None);
    }
}
