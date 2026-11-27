//! Ring helpers: direction, corners, entry candidates and the exit state
//! after an overcut (spec 8.3.4).
//!
//! Rings are one lap in mat coordinates (x across, y down) without a repeated
//! closing point. Counter-clockwise as seen on the canvas is a negative
//! shoelace area; clockwise is positive.

use kurbo::{Point, Vec2};

use flatcraft_core::geometry::RingDirection;

use crate::containment::signed_area;
use crate::plan::{CORNER_DEG, KeepSide};

/// A ring as the functions here read it: `Toolpath::lap()` repeats the first
/// point at the end, and that copy is dropped. Only the last point goes, so
/// an index into the result is the same index into the lap (T5 and T9 use
/// candidate indices on the lap directly). Every function in this module
/// calls it first, so passing a lap or its stripped ring gives the same
/// answer. `orient` and `rotate_start` return stripped rings.
pub fn ring_of(lap: &[Point]) -> &[Point] {
    match lap {
        [first, .., last] if first == last && lap.len() > 2 => &lap[..lap.len() - 1],
        _ => lap,
    }
}

/// The most entry candidates a ring offers.
pub const MAX_CANDIDATES: usize = 16;

/// One place a ring may be entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// Vertex index in the ring.
    pub index: usize,
    /// The corner's forward extension lies in waste (see
    /// [`extension_in_waste`]). Always false for vertices that are no corner.
    pub waste_corner: bool,
}

/// The turn in degrees at `b` when walking `a`, `b`, `c`: 0 straight on,
/// 180 a hairpin. A zero-length leg counts as no turn.
pub fn turn_deg(a: Point, b: Point, c: Point) -> f64 {
    let (u, v) = (b - a, c - b);
    if u.hypot2() == 0.0 || v.hypot2() == 0.0 {
        return 0.0;
    }
    u.cross(v).abs().atan2(u.dot(v)).to_degrees()
}

/// The ring in direction `dir`, keeping its first vertex first. A ring of
/// zero area has no direction and is returned unchanged.
pub fn orient(ring: &[Point], dir: RingDirection) -> Vec<Point> {
    let ring = ring_of(ring);
    let area = signed_area(ring);
    let is_ccw = area < 0.0;
    let wanted_ccw = dir == RingDirection::Ccw;
    let mut out = ring.to_vec();
    if area != 0.0 && is_ccw != wanted_ccw && out.len() > 1 {
        out[1..].reverse();
    }
    out
}

/// The ring starting at vertex `index` (taken modulo the length).
pub fn rotate_start(ring: &[Point], index: usize) -> Vec<Point> {
    let ring = ring_of(ring);
    let mut out = ring.to_vec();
    if !out.is_empty() {
        out.rotate_left(index % ring.len());
    }
    out
}

/// The nearest points before and after vertex `i` that differ from it, so a
/// repeated point never hides a corner. `None` for a ring with no length.
fn neighbours(ring: &[Point], i: usize) -> Option<(Point, Point)> {
    let n = ring.len();
    let here = ring[i];
    let prev = (1..n).map(|k| ring[(i + n - k) % n]).find(|&q| q != here)?;
    let next = (1..n).map(|k| ring[(i + k) % n]).find(|&q| q != here)?;
    Some((prev, next))
}

/// Cross product of the incoming and outgoing tangent at vertex `i`.
fn corner_cross(ring: &[Point], i: usize) -> f64 {
    neighbours(ring, i).map_or(0.0, |(prev, next)| (ring[i] - prev).cross(next - ring[i]))
}

/// True if the turn at vertex `i` has the sign of `signed_area(ring)`: the
/// ring bends away from its own interior there. Straight vertices are not
/// convex.
pub fn is_convex(ring: &[Point], i: usize) -> bool {
    let ring = ring_of(ring);
    if ring.len() < 3 {
        return false;
    }
    let i = i % ring.len();
    let (cross, area) = (corner_cross(ring, i), signed_area(ring));
    cross != 0.0 && area != 0.0 && (cross > 0.0) == (area > 0.0)
}

/// Whether the forward extension of the incoming edge past vertex `i` lies in
/// waste. It lies outside the ring at a convex vertex and inside it at a
/// concave one; `keep` says which side is kept. `Both` keeps everything, so
/// nothing is waste. Straight vertices have no side and give false.
///
/// This is a local pre-filter only: Task 9 confirms every waste move against
/// `KeptMaterial`. `keep` is Task 2's `KeepSide` (from `keep_side`).
pub fn extension_in_waste(ring: &[Point], i: usize, keep: KeepSide) -> bool {
    let ring = ring_of(ring);
    if ring.len() < 3 {
        return false;
    }
    let i = i % ring.len();
    waste_extension(corner_cross(ring, i), signed_area(ring), keep)
}

/// The rule behind [`extension_in_waste`], shared with the knife stage so
/// the two cannot drift: at a vertex whose incoming and outgoing tangents
/// have cross product `cross`, on a ring of signed area `area` kept on
/// side `keep`, the forward extension of the incoming edge lies in waste.
/// It lies outside the ring at a convex vertex (the turn has the sign of
/// the area) and inside it at a concave one. A straight vertex, a ring
/// with no area and `KeepSide::Both` give false.
pub fn waste_extension(cross: f64, area: f64, keep: KeepSide) -> bool {
    if cross == 0.0 || area == 0.0 {
        return false;
    }
    let convex = (cross > 0.0) == (area > 0.0);
    match keep {
        KeepSide::Inside => convex,
        KeepSide::Outside => !convex,
        KeepSide::Both => false,
    }
}

/// Arc length from vertex 0 to each vertex, and the perimeter.
fn arc_positions(ring: &[Point]) -> (Vec<f64>, f64) {
    let n = ring.len();
    let mut pos = Vec::with_capacity(n);
    let mut s = 0.0;
    for i in 0..n {
        pos.push(s);
        s += ring[i].distance(ring[(i + 1) % n]);
    }
    (pos, s)
}

/// Picks `k` of `items` (ring vertex indices) spread evenly by arc length.
/// Target `t` sits at `(t + 0.5) / k` of the perimeter; each takes the
/// nearest unused item, ties to the lower index. The result is sorted.
fn spread(items: &[usize], k: usize, pos: &[f64], perimeter: f64) -> Vec<usize> {
    if k >= items.len() {
        return items.to_vec();
    }
    let mut used = vec![false; items.len()];
    for t in 0..k {
        let target = (t as f64 + 0.5) / k as f64 * perimeter;
        let mut best: Option<(f64, usize)> = None;
        for (j, &v) in items.iter().enumerate() {
            if used[j] {
                continue;
            }
            let d = (pos[v] - target).abs();
            let d = d.min((perimeter - d).max(0.0));
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, j));
            }
        }
        if let Some((_, j)) = best {
            used[j] = true;
        }
    }
    let mut out: Vec<usize> = (0..items.len())
        .filter(|&j| used[j])
        .map(|j| items[j])
        .collect();
    out.sort_unstable();
    out
}

/// Where a ring may be entered, best first: at most [`MAX_CANDIDATES`].
///
/// The corners (turn of [`CORNER_DEG`] or more) come first, waste-side
/// corners before the rest. If there are more waste corners than the cap,
/// they are spread by arc length; otherwise the remaining slots go to the
/// other corners, spread by arc length. A ring without corners offers
/// vertices spread by arc length. Every strategy uses the same candidates.
pub fn entry_candidates(ring: &[Point], keep: KeepSide) -> Vec<Candidate> {
    let ring = ring_of(ring);
    let n = ring.len();
    if n == 0 {
        return Vec::new();
    }
    let (pos, perimeter) = arc_positions(ring);
    let (mut waste, mut other) = (Vec::new(), Vec::new());
    for i in 0..n {
        // The second of two equal points is no vertex of its own.
        if n > 1 && ring[i] == ring[(i + n - 1) % n] {
            continue;
        }
        let turn = neighbours(ring, i).map_or(0.0, |(a, c)| turn_deg(a, ring[i], c));
        if turn >= CORNER_DEG {
            if extension_in_waste(ring, i, keep) {
                waste.push(i);
            } else {
                other.push(i);
            }
        }
    }
    let make = |idx: Vec<usize>, waste_corner: bool| {
        idx.into_iter().map(move |index| Candidate {
            index,
            waste_corner,
        })
    };
    if waste.is_empty() && other.is_empty() {
        let all: Vec<usize> = (0..n)
            .filter(|&i| n == 1 || ring[i] != ring[(i + n - 1) % n])
            .collect();
        return make(spread(&all, MAX_CANDIDATES, &pos, perimeter), false).collect();
    }
    let w = spread(&waste, MAX_CANDIDATES, &pos, perimeter);
    let room = MAX_CANDIDATES - w.len();
    let o = spread(&other, room, &pos, perimeter);
    make(w, true).chain(make(o, false)).collect()
}

/// The point and unit tangent after walking `overcut_mm` of arc length from
/// vertex `index` along `ring` in the order given. This is where the head and
/// the blade heading are after the cut.
///
/// The ring is taken as oriented: this pure function does not orient it
/// (Ruling F28). The test ring (0,0), (10,0), (10,10), (0,10) has a positive
/// shoelace area, which is clockwise by the sign rule, and that is intended.
/// `index` is taken modulo the ring length. Exactly on a vertex the tangent
/// is the edge just travelled; at zero overcut it is the closing edge into
/// the start, the blade's real heading when the ring is finished. The
/// overcut is capped at the perimeter, as `overcut::apply` does: the result
/// is then the start point with the last edge's direction. An empty ring
/// gives the origin and a zero tangent; a ring with no length gives its
/// vertex and a zero tangent.
pub fn exit_state(ring: &[Point], index: usize, overcut_mm: f64) -> (Point, Vec2) {
    let ring = ring_of(ring);
    let n = ring.len();
    if n == 0 {
        return (Point::ORIGIN, Vec2::ZERO);
    }
    let index = index % n;
    let start = ring[index];
    let (_, perimeter) = arc_positions(ring);
    if n < 2 || perimeter <= 0.0 {
        return (start, Vec2::ZERO);
    }
    let mut remaining = overcut_mm.max(0.0).min(perimeter);
    if remaining == 0.0 {
        // The cut ends where it started, coming in along the closing edge.
        let closing =
            neighbours(ring, index).map_or(Vec2::ZERO, |(prev, _)| (start - prev).normalize());
        return (start, closing);
    }
    let mut last_dir = Vec2::ZERO;
    for step in 0..n {
        let a = ring[(index + step) % n];
        let b = ring[(index + step + 1) % n];
        let len = a.distance(b);
        if len == 0.0 {
            continue;
        }
        let dir = (b - a) / len;
        last_dir = dir;
        if remaining <= len {
            let pt = if remaining == len {
                b
            } else {
                a + dir * remaining
            };
            return (pt, dir);
        }
        remaining -= len;
    }
    (start, last_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{BezPath, Circle, Rect, Shape};

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn square(s: f64) -> Vec<Point> {
        vec![p(0.0, 0.0), p(s, 0.0), p(s, s), p(0.0, s)]
    }

    fn l_shape() -> Vec<Point> {
        vec![
            p(0.0, 0.0),
            p(20.0, 0.0),
            p(20.0, 10.0),
            p(10.0, 10.0),
            p(10.0, 20.0),
            p(0.0, 20.0),
        ]
    }

    fn star(points: usize) -> Vec<Point> {
        (0..2 * points)
            .map(|k| {
                let r = if k % 2 == 0 { 20.0 } else { 6.0 };
                let a = std::f64::consts::PI * k as f64 / points as f64;
                p(r * a.cos(), r * a.sin())
            })
            .collect()
    }

    #[test]
    fn ccw_means_negative_shoelace_in_mat_coordinates() {
        // (0,0) -> (10,0) -> (10,10) goes right then down: clockwise on a
        // y-down canvas, positive area.
        let cw = square(10.0);
        assert!(signed_area(&cw) > 0.0);
        let ccw = orient(&cw, RingDirection::Ccw);
        assert!(signed_area(&ccw) < 0.0);
        assert_eq!(ccw[0], cw[0]);
        let back = orient(&ccw, RingDirection::Cw);
        assert_eq!(back, cw);
        assert_eq!(orient(&cw, RingDirection::Cw), cw);
    }

    #[test]
    fn rotate_start_moves_the_first_vertex() {
        let r = rotate_start(&square(10.0), 2);
        assert_eq!(r[0], p(10.0, 10.0));
        assert_eq!(r.len(), 4);
    }

    #[test]
    fn turn_of_right_angle_is_90() {
        assert!((turn_deg(p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)) - 90.0).abs() < 1e-9);
        assert!(turn_deg(p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)).abs() < 1e-9);
    }

    #[test]
    fn hairpin_is_180() {
        assert!((turn_deg(p(0.0, 0.0), p(1.0, 0.0), p(0.0, 0.0)) - 180.0).abs() < 1e-9);
    }

    #[test]
    fn square_outline_corners_are_waste_side() {
        let sq = square(10.0);
        for ccw in [false, true] {
            let ring = if ccw {
                orient(&sq, RingDirection::Ccw)
            } else {
                sq.clone()
            };
            for i in 0..4 {
                assert!(is_convex(&ring, i));
                assert!(extension_in_waste(&ring, i, KeepSide::Inside));
                assert!(!extension_in_waste(&ring, i, KeepSide::Both));
            }
            assert!(
                entry_candidates(&ring, KeepSide::Inside)
                    .iter()
                    .all(|c| c.waste_corner)
            );
        }
    }

    #[test]
    fn hole_convex_corners_are_not_waste_side() {
        for ring in [l_shape(), orient(&l_shape(), RingDirection::Ccw)] {
            let waste: Vec<usize> = (0..ring.len())
                .filter(|&i| extension_in_waste(&ring, i, KeepSide::Outside))
                .collect();
            assert_eq!(waste.len(), 1);
            assert!(!is_convex(&ring, waste[0]));
            assert_eq!(ring[waste[0]], p(10.0, 10.0));
            let cands = entry_candidates(&ring, KeepSide::Outside);
            assert_eq!(cands.len(), 6);
            assert_eq!(
                cands[0],
                Candidate {
                    index: waste[0],
                    waste_corner: true
                }
            );
            assert_eq!(cands.iter().filter(|c| c.waste_corner).count(), 1);
        }
    }

    #[test]
    fn candidates_are_capped_with_waste_first() {
        // 20 tips (waste for an outline) and 20 notches: 40 corners.
        let s = star(20);
        let cands = entry_candidates(&s, KeepSide::Inside);
        assert_eq!(cands.len(), MAX_CANDIDATES);
        assert!(cands.iter().all(|c| c.waste_corner));
        assert!(cands.iter().all(|c| c.index % 2 == 0));
        // 10 tips and 10 notches: all 10 waste first, then 6 notches.
        let s = star(10);
        let cands = entry_candidates(&s, KeepSide::Inside);
        assert_eq!(cands.len(), MAX_CANDIDATES);
        assert!(cands[..10].iter().all(|c| c.waste_corner));
        assert!(cands[10..].iter().all(|c| !c.waste_corner));
        assert_eq!(cands, entry_candidates(&s, KeepSide::Inside));
    }

    #[test]
    fn circle_has_spread_candidates() {
        let n = 200;
        let ring: Vec<Point> = (0..n)
            .map(|k| {
                let a = std::f64::consts::TAU * k as f64 / n as f64;
                p(5.0 * a.cos(), 5.0 * a.sin())
            })
            .collect();
        let cands = entry_candidates(&ring, KeepSide::Inside);
        assert_eq!(cands.len(), MAX_CANDIDATES);
        assert!(cands.iter().all(|c| !c.waste_corner));
        let mut idx: Vec<usize> = cands.iter().map(|c| c.index).collect();
        idx.dedup();
        assert_eq!(idx.len(), MAX_CANDIDATES);
        // Even spread: neighbours are about n / 16 apart.
        for w in idx.windows(2) {
            assert!((w[1] - w[0]).abs_diff(n / MAX_CANDIDATES) <= 1);
        }
    }

    #[test]
    fn exit_state_after_overcut() {
        let sq = square(10.0);
        let (pt, t) = exit_state(&sq, 0, 0.5);
        assert!((pt - p(0.5, 0.0)).hypot() < 1e-12);
        assert_eq!(t, Vec2::new(1.0, 0.0));
        let (pt, t) = exit_state(&sq, 0, 12.0);
        assert!((pt - p(10.0, 2.0)).hypot() < 1e-12);
        assert_eq!(t, Vec2::new(0.0, 1.0));
        // Exactly on a vertex: the edge just travelled.
        let (pt, t) = exit_state(&sq, 0, 10.0);
        assert_eq!(pt, p(10.0, 0.0));
        assert_eq!(t, Vec2::new(1.0, 0.0));
        // Zero overcut: at the start, heading along the closing edge.
        for oc in [0.0, -1.0] {
            assert_eq!(exit_state(&sq, 0, oc), (p(0.0, 0.0), Vec2::new(0.0, -1.0)));
        }
        assert_eq!(exit_state(&sq, 1, 0.0), (p(10.0, 0.0), Vec2::new(1.0, 0.0)));
        // Capped at the perimeter: back at the start, heading of the last edge.
        for oc in [40.0, 40.5] {
            let (pt, t) = exit_state(&sq, 0, oc);
            assert_eq!(pt, p(0.0, 0.0));
            assert_eq!(t, Vec2::new(0.0, -1.0));
        }
    }

    #[test]
    fn exit_state_survives_degenerate_input() {
        assert_eq!(exit_state(&[], 3, 1.0), (Point::ORIGIN, Vec2::ZERO));
        let sq = square(10.0);
        assert_eq!(
            exit_state(&sq, 6, 0.5).0,
            p(10.0, 10.0 - 0.0).lerp(p(0.0, 10.0), 0.05)
        );
        // A 0.4 mm ring with 0.5 mm overcut.
        let tiny = square(0.1);
        let (pt, t) = exit_state(&tiny, 0, 0.5);
        assert_eq!(pt, p(0.0, 0.0));
        assert_eq!(t, Vec2::new(0.0, -1.0));
        let one = [p(1.0, 1.0)];
        assert_eq!(exit_state(&one, 0, 1.0), (p(1.0, 1.0), Vec2::ZERO));
    }

    #[test]
    fn a_repeated_point_does_not_hide_a_corner() {
        let ring = vec![
            p(0.0, 0.0),
            p(10.0, 0.0),
            p(10.0, 0.0),
            p(10.0, 10.0),
            p(0.0, 10.0),
        ];
        assert!(is_convex(&ring, 1));
        assert!(is_convex(&ring, 2));
        let cands = entry_candidates(&ring, KeepSide::Inside);
        assert_eq!(
            cands.iter().map(|c| c.index).collect::<Vec<_>>(),
            vec![0, 1, 3, 4]
        );
        assert!(cands.iter().all(|c| c.waste_corner));
    }

    #[test]
    fn a_cornerless_ring_with_repeated_points_offers_each_place_once() {
        let n = 100;
        let mut ring = Vec::new();
        for k in 0..n {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            let q = p(5.0 * a.cos(), 5.0 * a.sin());
            ring.push(q);
            ring.push(q);
        }
        let cands = entry_candidates(&ring, KeepSide::Inside);
        assert_eq!(cands.len(), MAX_CANDIDATES);
        let mut at: Vec<(u64, u64)> = cands
            .iter()
            .map(|c| (ring[c.index].x.to_bits(), ring[c.index].y.to_bits()))
            .collect();
        at.sort_unstable();
        at.dedup();
        assert_eq!(at.len(), MAX_CANDIDATES);
    }

    fn lap_of(path: BezPath) -> Vec<Point> {
        use crate::plan::{PathRef, PipelineSettings, PlacedPath, plan};
        use flatcraft_core::document::ObjectId;
        let s = PipelineSettings {
            overcut_mm: 0.0,
            ..PipelineSettings::with_defaults(Rect::new(0.0, 0.0, 300.0, 300.0), Point::ZERO)
        };
        let input = [PlacedPath {
            path,
            source: PathRef {
                object: ObjectId(1),
                path_index: 0,
            },
        }];
        let out = plan(&input, &s).unwrap();
        let tp = &out.toolpaths[0];
        assert!(tp.closed);
        tp.lap().to_vec()
    }

    fn poly(pts: &[(f64, f64)]) -> BezPath {
        let mut b = BezPath::new();
        b.move_to(pts[0]);
        for &q in &pts[1..] {
            b.line_to(q);
        }
        b.close_path();
        b
    }

    #[test]
    fn lap_input_gives_the_same_answers_as_the_stripped_ring() {
        let circle = Circle::new((50.0, 50.0), 5.0).to_path(0.01);
        let cases = [
            (
                poly(&[(10.0, 10.0), (20.0, 10.0), (20.0, 20.0), (10.0, 20.0)]),
                KeepSide::Inside,
                4,
            ),
            (
                poly(&[
                    (10.0, 10.0),
                    (30.0, 10.0),
                    (30.0, 20.0),
                    (20.0, 20.0),
                    (20.0, 30.0),
                    (10.0, 30.0),
                ]),
                KeepSide::Outside,
                1,
            ),
            (circle, KeepSide::Inside, 0),
        ];
        for (path, keep, waste) in cases {
            let lap = lap_of(path);
            assert_eq!(lap.first(), lap.last());
            let ring = ring_of(&lap);
            assert_eq!(ring.len(), lap.len() - 1);
            assert_eq!(entry_candidates(&lap, keep), entry_candidates(ring, keep));
            let cands = entry_candidates(&lap, keep);
            assert_eq!(cands.iter().filter(|c| c.waste_corner).count(), waste);
            assert!(cands.iter().all(|c| c.index < ring.len()));
            // No phantom corner at the repeated closing point.
            for i in 0..lap.len() {
                assert_eq!(is_convex(&lap, i), is_convex(ring, i));
                assert_eq!(
                    extension_in_waste(&lap, i, keep),
                    extension_in_waste(ring, i, keep)
                );
            }
            assert_eq!(orient(&lap, RingDirection::Ccw).len(), ring.len());
            assert_eq!(rotate_start(&lap, 1).len(), ring.len());
            assert_eq!(exit_state(&lap, 0, 0.5), exit_state(ring, 0, 0.5));
        }
    }
}
