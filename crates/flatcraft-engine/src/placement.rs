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

/// Slack when checking that a box fits the area.
const FIT_EPS: f64 = 1e-9;

/// The smallest width and height a box takes while packing, mm: a line
/// with no height still takes a hair, so the next one does not land on it.
const MIN_EXTENT: f64 = 1e-3;

/// One object as Auto arrange (`arrange.auto`) sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoObject<K> {
    pub key: K,
    pub visible: bool,
    /// Its bounding box on the mat, `None` while it has no cut geometry.
    pub bounds: Option<Rect>,
}

/// What Auto arrange does: the move of each packed object, the objects
/// that stay because they do not fit or have no box (`did_not_fit`), the
/// objects that stay where they are and are packed around (`fixed`), and
/// the chosen objects that stay because they are hidden (`hidden`).
#[derive(Debug, Clone, PartialEq)]
pub struct AutoPlan<K> {
    pub moves: Vec<(K, Vec2)>,
    pub did_not_fit: Vec<K>,
    pub fixed: Vec<K>,
    pub hidden: Vec<K>,
}

/// Auto arrange: which objects are packed and where they go. The command
/// and the window's preview both use it, so they cannot differ.
///
/// `chosen` are the objects to pack (the selection or the given ids);
/// empty means every visible object. Hidden objects are neither packed
/// nor packed around; chosen ones are listed in `hidden`. Every other visible object with a box is fixed: the
/// packed ones keep `spacing` mm from it and never overlap it. A chosen
/// object without a box cannot be placed and is listed in `did_not_fit`.
///
/// This is the one place that decides what is packed as one item.
pub fn auto_arrange<K: Copy + PartialEq>(
    objects: &[AutoObject<K>],
    chosen: &[K],
    area: Rect,
    spacing: f64,
) -> AutoPlan<K> {
    let mut items = Vec::new();
    let mut obstacles = Vec::new();
    let mut fixed = Vec::new();
    let mut no_box = Vec::new();
    let mut hidden = Vec::new();
    for o in objects {
        let packed = chosen.is_empty() || chosen.contains(&o.key);
        if !o.visible {
            if !chosen.is_empty() && packed {
                hidden.push(o.key);
            }
            continue;
        }
        match (packed, o.bounds) {
            (true, Some(b)) => items.push((o.key, b)),
            (true, None) => no_box.push(o.key),
            (false, Some(b)) => {
                obstacles.push(b);
                fixed.push(o.key);
            }
            // Nothing to cut, nothing to keep clear of.
            (false, None) => {}
        }
    }
    let (moves, mut did_not_fit) = pack_around(&items, &obstacles, area, spacing);
    no_box.append(&mut did_not_fit);
    AutoPlan {
        moves,
        did_not_fit: no_box,
        fixed,
        hidden,
    }
}

/// True when `a` and `b` share some area (touching is not overlapping).
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

/// True when `outer` holds all of `inner`.
fn holds(outer: Rect, inner: Rect) -> bool {
    outer.x0 <= inner.x0 && outer.y0 <= inner.y0 && outer.x1 >= inner.x1 && outer.y1 >= inner.y1
}

/// The free rects of a max-rects packer once `used` is taken: every free
/// rect `used` overlaps is replaced by the up to four largest rects
/// beside it, and rects inside another one are dropped. The order stays
/// fixed, so the result is deterministic.
fn take(free: &mut Vec<Rect>, used: Rect) {
    let mut kept = Vec::with_capacity(free.len());
    let mut new = Vec::new();
    for f in free.drain(..) {
        if !overlaps(f, used) {
            kept.push(f);
            continue;
        }
        let parts = [
            Rect::new(f.x0, f.y0, used.x0, f.y1),
            Rect::new(used.x1, f.y0, f.x1, f.y1),
            Rect::new(f.x0, f.y0, f.x1, used.y0),
            Rect::new(f.x0, used.y1, f.x1, f.y1),
        ];
        new.extend(
            parts
                .into_iter()
                .filter(|r| r.width() > 0.0 && r.height() > 0.0),
        );
    }
    // The kept rects did not hold each other before, so only the new
    // ones need checking: against the kept ones, and among themselves
    // (of two equal ones the first stays).
    let new_keep: Vec<bool> = (0..new.len())
        .map(|i| {
            !kept.iter().any(|k| holds(*k, new[i]))
                && !new
                    .iter()
                    .enumerate()
                    .any(|(j, n)| j != i && holds(*n, new[i]) && (*n != new[i] || j < i))
        })
        .collect();
    let new: Vec<Rect> = new
        .into_iter()
        .zip(new_keep)
        .filter_map(|(r, k)| k.then_some(r))
        .collect();
    kept.retain(|k| !new.iter().any(|n| holds(*n, *k)));
    kept.extend(new);
    *free = kept;
}

/// Packs `items` into `area` around the fixed `obstacles`: a max-rects
/// packer on bounding boxes, tallest first (equal heights keep their
/// order), each one at the free spot nearest the top of the area, then
/// nearest its left. Packed boxes keep `spacing` mm from each other and
/// from every obstacle; rotation and size never change. Returns the move
/// of each box that is placed, and the keys of those that do not fit, in
/// that order.
///
/// The spacing is added to the right and bottom of every box (packed or
/// obstacle) and of the area, so two boxes that touch after that are
/// exactly `spacing` apart, and a box may still touch the area's edge.
pub fn pack_around<K: Copy>(
    items: &[(K, Rect)],
    obstacles: &[Rect],
    area: Rect,
    spacing: f64,
) -> (Vec<(K, Vec2)>, Vec<K>) {
    let grow = |x0: f64, y0: f64, w: f64, h: f64| {
        Rect::new(
            x0,
            y0,
            x0 + (w + spacing).max(MIN_EXTENT),
            y0 + (h + spacing).max(MIN_EXTENT),
        )
    };
    let bin = Rect::new(area.x0, area.y0, area.x1 + spacing, area.y1 + spacing);
    let mut free = vec![bin];
    for o in obstacles {
        let g = grow(o.x0, o.y0, o.width(), o.height());
        if overlaps(g, bin) {
            take(&mut free, g);
        }
    }
    let mut items: Vec<(K, Rect)> = items.to_vec();
    // Stable: equal heights keep their order.
    items.sort_by(|a, b| b.1.height().total_cmp(&a.1.height()));
    let mut placed = Vec::new();
    let mut did_not_fit = Vec::new();
    for (key, b) in items {
        let size = grow(0.0, 0.0, b.width(), b.height());
        let (w, h) = (size.width(), size.height());
        let spot = free
            .iter()
            .filter(|f| f.width() + FIT_EPS >= w && f.height() + FIT_EPS >= h)
            .map(|f| Point::new(f.x0, f.y0))
            .min_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        match spot {
            Some(p) => {
                take(&mut free, Rect::new(p.x, p.y, p.x + w, p.y + h));
                placed.push((key, p - Point::new(b.x0, b.y0)));
            }
            None => did_not_fit.push(key),
        }
    }
    (placed, did_not_fit)
}

/// How much closer than the gap [`free_spot`] still counts as the gap, mm
/// (float noise in placed bounds).
pub const GAP_EPS: f64 = 1e-6;

/// The most cells [`free_spot`] looks at; a larger area gets coarser steps.
const FREE_SPOT_CELLS: f64 = 1_000_000.0;

/// The top-left corner of the first spot, reading the area row by row from
/// its top-left on a 1 mm grid (coarser for a very large area), where a box
/// of `size` lies inside `area` and keeps at least `gap` mm from every box
/// in `taken`. `None` when there is no such spot.
///
/// The forbidden corners of each taken box are marked in a 2D difference
/// array, so the search costs one pass over the grid plus one step per box,
/// however many objects lie on the mat.
pub fn free_spot(area: Rect, taken: &[Rect], size: Vec2, gap: f64) -> Option<Point> {
    let (room_x, room_y) = (area.width() - size.x, area.height() - size.y);
    if !(room_x >= -FIT_EPS && room_y >= -FIT_EPS) {
        return None;
    }
    let room_x = room_x.max(0.0);
    let room_y = room_y.max(0.0);
    let mut step = 1.0_f64;
    let cells = |step: f64| ((room_x / step).floor() + 1.0) * ((room_y / step).floor() + 1.0);
    if cells(step) > FREE_SPOT_CELLS {
        step = (room_x * room_y / FREE_SPOT_CELLS).sqrt().ceil().max(1.0);
    }
    let nx = (room_x / step + FIT_EPS).floor() as usize + 1;
    let ny = (room_y / step + FIT_EPS).floor() as usize + 1;
    // diff[(nx + 1) * j + i]
    let w = nx + 1;
    let mut diff = vec![0i32; w * (ny + 1)];
    // Cells i with lo < x0 + i * step < hi: a box `gap` away is fine,
    // also when float noise makes it a hair closer ([`GAP_EPS`]).
    let span = |lo: f64, hi: f64, origin: f64, n: usize| -> Option<(usize, usize)> {
        let (lo, hi) = (lo + GAP_EPS, hi - GAP_EPS);
        let a = ((lo - origin) / step).floor() + 1.0;
        let b = ((hi - origin) / step).ceil() - 1.0;
        let a = a.max(0.0);
        let b = b.min(n as f64 - 1.0);
        (a <= b).then_some((a as usize, b as usize))
    };
    for r in taken {
        let xs = span(r.x0 - gap - size.x, r.x1 + gap, area.x0, nx);
        let ys = span(r.y0 - gap - size.y, r.y1 + gap, area.y0, ny);
        if let (Some((i0, i1)), Some((j0, j1))) = (xs, ys) {
            diff[w * j0 + i0] += 1;
            diff[w * j0 + i1 + 1] -= 1;
            diff[w * (j1 + 1) + i0] -= 1;
            diff[w * (j1 + 1) + i1 + 1] += 1;
        }
    }
    // Prefix sums, row by row: a cell is free when its sum is 0.
    let mut above = vec![0i32; nx];
    for j in 0..ny {
        let mut run = 0;
        for (i, col) in above.iter_mut().enumerate() {
            run += diff[w * j + i];
            *col += run;
            if *col == 0 {
                return Some(Point::new(
                    area.x0 + i as f64 * step,
                    area.y0 + j as f64 * step,
                ));
            }
        }
    }
    None
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

    fn clear_of(spot: Point, size: Vec2, taken: &[Rect], gap: f64) -> bool {
        let r = Rect::from_origin_size(spot, size.to_size());
        taken.iter().all(|t| {
            r.x0 >= t.x1 + gap - 1e-9
                || r.x1 <= t.x0 - gap + 1e-9
                || r.y0 >= t.y1 + gap - 1e-9
                || r.y1 <= t.y0 - gap + 1e-9
        })
    }

    #[test]
    fn an_empty_area_gives_its_top_left() {
        let area = Rect::new(5.0, 5.0, 300.0, 300.0);
        let size = Vec2::new(30.0, 30.0);
        assert_eq!(free_spot(area, &[], size, 5.0), Some(Point::new(5.0, 5.0)));
    }

    #[test]
    fn the_spot_keeps_the_gap_and_stays_inside() {
        let area = Rect::new(5.0, 5.0, 299.8, 299.8);
        let size = Vec2::new(30.0, 30.0);
        let taken = [
            Rect::new(0.0, 0.0, 100.0, 50.0),
            Rect::new(120.5, 3.0, 180.0, 20.0),
            Rect::new(200.0, 0.0, 299.8, 299.8),
        ];
        let spot = free_spot(area, &taken, size, 5.0).unwrap();
        assert!(clear_of(spot, size, &taken, 5.0), "{spot:?}");
        let r = Rect::from_origin_size(spot, size.to_size());
        assert!(r.x0 >= area.x0 && r.y0 >= area.y0 && r.x1 <= area.x1 && r.y1 <= area.y1);
        // Row by row from the top: between the first two boxes there is
        // room only from x 105 (gap after 100), and 105 + 30 + 5 > 120.5,
        // so the first row that fits is below the second box.
        assert_eq!(spot, Point::new(105.0, 25.0));
    }

    #[test]
    fn a_full_area_has_no_spot() {
        let area = Rect::new(5.0, 5.0, 299.8, 299.8);
        let size = Vec2::new(30.0, 30.0);
        assert_eq!(
            free_spot(area, &[Rect::new(20.0, 20.0, 280.0, 280.0)], size, 5.0),
            None
        );
        assert_eq!(
            free_spot(Rect::new(0.0, 0.0, 20.0, 20.0), &[], size, 5.0),
            None
        );
    }

    #[test]
    fn a_long_roll_is_searched_in_coarser_steps() {
        let area = Rect::new(5.0, 5.0, 299.8, 2995.0);
        let size = Vec2::new(30.0, 30.0);
        let taken = [Rect::new(0.0, 0.0, 300.0, 2000.0)];
        let spot = free_spot(area, &taken, size, 5.0).unwrap();
        assert!(clear_of(spot, size, &taken, 5.0), "{spot:?}");
        assert!(spot.y >= 2005.0 && spot.y < 2010.0, "{spot:?}");
    }

    #[test]
    fn a_gap_a_hair_short_still_counts() {
        let area = Rect::new(0.0, 0.0, 300.0, 300.0);
        let size = Vec2::new(30.0, 30.0);
        // Float noise: the box ends 1e-9 mm past 95, so x 100 is 5 mm
        // away less a hair.
        let taken = [Rect::new(0.0, 0.0, 95.000_000_001, 300.0)];
        assert_eq!(
            free_spot(area, &taken, size, 5.0),
            Some(Point::new(100.0, 0.0))
        );
        // A real overlap of 0.01 mm does not.
        let taken = [Rect::new(0.0, 0.0, 95.01, 300.0)];
        assert_eq!(
            free_spot(area, &taken, size, 5.0),
            Some(Point::new(101.0, 0.0))
        );
    }
}
