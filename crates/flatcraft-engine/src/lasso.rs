//! Picking objects with a free shape: the canvas lasso, the selection box
//! (a lasso with four corners) and `selection.set_in_polygon` all go
//! through here, so the window and agents select the same objects.
//!
//! The rule: an object is picked when its outline lies fully inside the
//! shape, or with `touch` when the shape touches it. Each object is
//! tested first by its box (the box of what is cut, see
//! `commands::placed_bounds`) and only then by its outline, so a lasso
//! over thousands of objects tests only the few near it.

use flatcraft_core::document::{Document, Object, ObjectId};
use flatcraft_core::nodes::Tree;
use kurbo::{Point, Rect, Vec2};

use crate::cutgeom::Knife;

/// The tolerance the outlines are flattened with, mm. The canvas draws
/// its shapes with it too, so the lasso tests what the person sees.
pub const OUTLINE_TOLERANCE_MM: f64 = 0.02;

/// Slack for a box that ends exactly on the lasso's box, mm.
const SLACK_MM: f64 = 1e-6;

/// What a pick does with the selection there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The picked objects become the selection.
    #[default]
    Replace,
    /// The picked objects join the selection.
    Add,
    /// The picked objects leave the selection.
    Remove,
    /// Each picked object or group that is selected leaves the selection,
    /// the others join it (Shift, as in Illustrator, Figma and Inkscape).
    Toggle,
}

/// A closed free shape in mat mm. The last point joins the first.
#[derive(Debug, Clone, PartialEq)]
pub struct Lasso {
    points: Vec<Point>,
    bbox: Rect,
}

impl Lasso {
    /// The shape through `points`, closed back to the first. `None` when
    /// it has fewer than three different points, a point that is not a
    /// number, or no area (all points on one line).
    pub fn new(points: impl IntoIterator<Item = Point>) -> Option<Self> {
        let mut pts: Vec<Point> = Vec::new();
        for p in points {
            if !(p.x.is_finite() && p.y.is_finite()) {
                return None;
            }
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
        while pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() < 3 || area(&pts).abs() <= 1e-12 {
            return None;
        }
        let bbox = pts
            .iter()
            .fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p));
        Some(Self { points: pts, bbox })
    }

    /// The selection box as a lasso: its four corners. `None` for a box
    /// without area.
    pub fn rect(r: Rect) -> Option<Self> {
        let r = r.abs();
        Self::new([
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x1, r.y1),
            Point::new(r.x0, r.y1),
        ])
    }

    pub fn points(&self) -> &[Point] {
        &self.points
    }

    pub fn bounds(&self) -> Rect {
        self.bbox
    }

    /// Whether `p` lies inside the shape (even-odd rule: where the shape
    /// crosses itself, the overlap counts as outside, as a filled path).
    pub fn contains(&self, p: Point) -> bool {
        inside_ring(&self.points, p)
    }

    /// The edges of the shape whose box meets `r`: the only ones an
    /// outline inside `r` can cross.
    fn edges_near(&self, r: Rect) -> Vec<(Point, Point)> {
        let n = self.points.len();
        (0..n)
            .map(|i| (self.points[i], self.points[(i + 1) % n]))
            .filter(|(a, b)| {
                a.x.max(b.x) >= r.x0
                    && a.x.min(b.x) <= r.x1
                    && a.y.max(b.y) >= r.y0
                    && a.y.min(b.y) <= r.y1
            })
            .collect()
    }

    /// Whether the outline `lines` (moved by `offset`) lies fully inside
    /// the shape: every point inside, and no line crosses the shape's edge
    /// (a bay of the lasso can cut between two points that are inside).
    fn holds(&self, bbox: Rect, lines: &[(&[Point], bool)], offset: Vec2) -> bool {
        let edges = self.edges_near(bbox);
        lines.iter().all(|(pts, closed)| {
            pts.iter().all(|p| self.contains(*p + offset))
                && segments(pts, *closed).all(|(a, b)| {
                    !edges
                        .iter()
                        .any(|e| cross(a + offset, b + offset, e.0, e.1))
                })
        })
    }

    /// Whether the shape touches the outline `lines` (moved by `offset`):
    /// a point of it inside, or a line crossing the shape's edge. A lasso
    /// drawn inside a ring does neither, so stickers lassoed on a backing
    /// sheet do not bring the sheet along.
    fn touches(&self, bbox: Rect, lines: &[(&[Point], bool)], offset: Vec2) -> bool {
        if lines
            .iter()
            .any(|(pts, _)| pts.iter().any(|p| self.contains(*p + offset)))
        {
            return true;
        }
        let edges = self.edges_near(bbox);
        lines.iter().any(|(pts, closed)| {
            segments(pts, *closed).any(|(a, b)| {
                edges
                    .iter()
                    .any(|e| cross(a + offset, b + offset, e.0, e.1))
            })
        })
    }
}

/// The signed area of a ring (shoelace).
fn area(pts: &[Point]) -> f64 {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        / 2.0
}

/// Even-odd point in ring.
fn inside_ring(ring: &[Point], p: Point) -> bool {
    let n = ring.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The segments of a polyline, with the closing one for a ring.
fn segments(pts: &[Point], closed: bool) -> impl Iterator<Item = (Point, Point)> + '_ {
    let closing = (closed && pts.len() > 2).then(|| (pts[pts.len() - 1], pts[0]));
    pts.windows(2).map(|w| (w[0], w[1])).chain(closing)
}

/// Whether the segments `a`-`b` and `c`-`d` cross or touch.
fn cross(a: Point, b: Point, c: Point, d: Point) -> bool {
    let side = |p: Point, q: Point, r: Point| (q - p).cross(r - p);
    let (d1, d2) = (side(c, d, a), side(c, d, b));
    let (d3, d4) = (side(a, b, c), side(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    let on = |p: Point, q: Point, r: Point, s: f64| {
        s == 0.0
            && r.x >= p.x.min(q.x)
            && r.x <= p.x.max(q.x)
            && r.y >= p.y.min(q.y)
            && r.y <= p.y.max(q.y)
    };
    on(c, d, a, d1) || on(c, d, b, d2) || on(a, b, c, d3) || on(a, b, d, d4)
}

/// An outline: its lines, each with its points and whether it closes.
pub type Lines<'a> = [(&'a [Point], bool)];

/// What [`Outlines::with_lines`] calls with an outline and its offset.
pub type LinesVisitor<'v> = dyn FnMut(&Lines<'_>, Vec2) -> bool + 'v;

/// What the pick reads of an object: hidden or not, its box and its
/// outline. The window reads its cached shapes, the command the document.
pub trait Outlines {
    /// Whether object `id` is shown. Hidden objects are never picked.
    fn visible(&self, id: ObjectId) -> bool;
    /// The box of what is cut of `id`, in mat mm; `None` when there is
    /// nothing (it is never picked).
    fn bounds(&self, id: ObjectId) -> Option<Rect>;
    /// Calls `f` with the outline of `id`: its lines (points and whether
    /// they close) and the offset to add to every point to get mat mm.
    fn with_lines(&self, id: ObjectId, f: &mut LinesVisitor<'_>) -> bool;
}

/// The outcome of [`pick`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picked {
    /// The objects picked, in document order: whole groups bring every
    /// member, hidden ones too (a group stays one piece).
    pub ids: Vec<ObjectId>,
    /// The same objects, one list per picked object or group, so a
    /// toggle flips each as one piece.
    pub units: Vec<Vec<ObjectId>>,
    /// How many objects passed the box test and had their outline tested.
    pub outline_tests: usize,
}

/// Whether one shown object passes: its box first, then its outline. An
/// object without lines (a picture not traced) is tested by its box.
fn passes(
    lasso: &Lasso,
    touch: bool,
    shapes: &dyn Outlines,
    id: ObjectId,
    tests: &mut usize,
) -> bool {
    let Some(b) = shapes.bounds(id) else {
        return false;
    };
    let l = lasso.bbox;
    let near = if touch {
        b.x0 <= l.x1 && b.x1 >= l.x0 && b.y0 <= l.y1 && b.y1 >= l.y0
    } else {
        b.x0 >= l.x0 - SLACK_MM
            && b.y0 >= l.y0 - SLACK_MM
            && b.x1 <= l.x1 + SLACK_MM
            && b.y1 <= l.y1 + SLACK_MM
    };
    if !near {
        return false;
    }
    *tests += 1;
    let corners = [
        Point::new(b.x0, b.y0),
        Point::new(b.x1, b.y0),
        Point::new(b.x1, b.y1),
        Point::new(b.x0, b.y1),
    ];
    let run = |lines: &[(&[Point], bool)], offset: Vec2| {
        if touch {
            lasso.touches(b, lines, offset)
        } else {
            lasso.holds(b, lines, offset)
        }
    };
    let mut test = |lines: &[(&[Point], bool)], offset: Vec2| {
        if lines.is_empty() {
            run(&[(&corners[..], true)], Vec2::ZERO)
        } else {
            run(lines, offset)
        }
    };
    shapes.with_lines(id, &mut test)
}

/// The objects the lasso picks. Top-level objects and groups are tested
/// as one each: a group is picked when all its shown members are inside
/// (with `touch`, when the lasso touches one of them). Inside the group
/// `within` (the group the person went into), its children are tested
/// instead and nothing outside it is picked.
pub fn pick(
    tree: &Tree,
    shapes: &dyn Outlines,
    lasso: &Lasso,
    touch: bool,
    within: Option<ObjectId>,
) -> Picked {
    let mut out = Picked::default();
    for unit in tree.children(within) {
        let leaves = tree.leaves_of(unit);
        let shown: Vec<ObjectId> = leaves
            .iter()
            .copied()
            .filter(|l| shapes.visible(*l))
            .collect();
        if shown.is_empty() {
            continue;
        }
        let mut test = |l: &ObjectId| passes(lasso, touch, shapes, *l, &mut out.outline_tests);
        let hit = if touch {
            shown.iter().any(&mut test)
        } else {
            shown.iter().all(&mut test)
        };
        if hit {
            out.ids.extend(&leaves);
            out.units.push(leaves);
        }
    }
    let order: std::collections::BTreeSet<ObjectId> = out.ids.iter().copied().collect();
    out.ids = tree.expand(&order.into_iter().collect::<Vec<_>>());
    out
}

/// The selection after a pick: the picked `units` (see [`Picked::units`])
/// replace `current`, join it, leave it or toggle in it.
pub fn combine(current: &[ObjectId], units: &[Vec<ObjectId>], mode: Mode) -> Vec<ObjectId> {
    let picked: Vec<ObjectId> = units.iter().flatten().copied().collect();
    match mode {
        Mode::Replace => picked,
        Mode::Toggle => {
            let mut out = current.to_vec();
            for u in units {
                if u.iter().all(|i| current.contains(i)) {
                    out.retain(|i| !u.contains(i));
                } else {
                    out.extend(
                        u.iter()
                            .filter(|i| !out.contains(i))
                            .copied()
                            .collect::<Vec<_>>(),
                    );
                }
            }
            out
        }
        Mode::Add => {
            let mut out = current.to_vec();
            out.extend(picked.iter().filter(|p| !current.contains(p)));
            out
        }
        Mode::Remove => current
            .iter()
            .copied()
            .filter(|c| !picked.contains(c))
            .collect(),
    }
}

/// The objects of a document as the pick reads them: the placed paths
/// flattened with [`OUTLINE_TOLERANCE_MM`] (what the canvas draws) and
/// the box of what is cut.
pub struct DocumentOutlines<'a> {
    doc: &'a Document,
    knife: Knife,
}

impl<'a> DocumentOutlines<'a> {
    pub fn new(doc: &'a Document, knife: Knife) -> Self {
        Self { doc, knife }
    }

    fn object(&self, id: ObjectId) -> Option<&'a Object> {
        self.doc.get(id)
    }
}

/// The lines of an object's placed paths, flattened with
/// [`OUTLINE_TOLERANCE_MM`], in mat mm.
pub fn outline_lines(o: &Object) -> Vec<(Vec<Point>, bool)> {
    let mut lines: Vec<(Vec<Point>, bool)> = Vec::new();
    for p in o.placed_paths() {
        let mut current: Vec<Point> = Vec::new();
        let mut flush = |current: &mut Vec<Point>, closed: bool| {
            if closed {
                let start = current.first().copied();
                while current.len() > 1 && current.last().copied() == start {
                    current.pop();
                }
            }
            if current.len() > 1 {
                lines.push((std::mem::take(current), closed));
            } else {
                current.clear();
            }
        };
        let mut start = None;
        kurbo::flatten(
            p.path.elements().iter().copied(),
            OUTLINE_TOLERANCE_MM,
            |el| match el {
                kurbo::PathEl::MoveTo(pt) => {
                    flush(&mut current, false);
                    current.push(pt);
                    start = Some(pt);
                }
                kurbo::PathEl::LineTo(pt) => {
                    if current.last() != Some(&pt) {
                        current.push(pt);
                    }
                }
                kurbo::PathEl::ClosePath => {
                    flush(&mut current, true);
                    if let Some(s) = start {
                        current.push(s);
                    }
                }
                _ => {}
            },
        );
        flush(&mut current, false);
    }
    lines
}

impl Outlines for DocumentOutlines<'_> {
    fn visible(&self, id: ObjectId) -> bool {
        self.object(id).is_some_and(|o| o.visible)
    }

    fn bounds(&self, id: ObjectId) -> Option<Rect> {
        crate::commands::placed_bounds(self.object(id)?, self.knife)
    }

    fn with_lines(&self, id: ObjectId, f: &mut LinesVisitor<'_>) -> bool {
        let Some(o) = self.object(id) else {
            return false;
        };
        let lines = outline_lines(o);
        let refs: Vec<(&[Point], bool)> = lines.iter().map(|(p, c)| (p.as_slice(), *c)).collect();
        f(&refs, Vec2::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Vec<Point> {
        vec![
            Point::new(x, y),
            Point::new(x + s, y),
            Point::new(x + s, y + s),
            Point::new(x, y + s),
        ]
    }

    fn lasso(pts: &[(f64, f64)]) -> Lasso {
        Lasso::new(pts.iter().map(|(x, y)| Point::new(*x, *y))).unwrap()
    }

    #[test]
    fn a_shape_needs_three_points_and_an_area() {
        assert!(Lasso::new([Point::ORIGIN, Point::new(1.0, 0.0)]).is_none());
        assert!(Lasso::new([Point::ORIGIN, Point::new(1.0, 0.0), Point::new(2.0, 0.0)]).is_none());
        assert!(
            Lasso::new([
                Point::ORIGIN,
                Point::new(f64::NAN, 0.0),
                Point::new(0.0, 1.0)
            ])
            .is_none()
        );
        // A last point on the first one closes it: it counts once.
        let l = Lasso::new(square(0.0, 0.0, 1.0).into_iter().chain([Point::ORIGIN])).unwrap();
        assert_eq!(l.points().len(), 4);
        assert!(Lasso::rect(Rect::new(0.0, 0.0, 0.0, 5.0)).is_none());
    }

    #[test]
    fn fully_inside_needs_every_line_inside_and_touch_any_contact() {
        let big = lasso(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let inside = square(2.0, 2.0, 3.0);
        let over = square(8.0, 2.0, 4.0);
        let apart = square(20.0, 2.0, 3.0);
        let b = |s: &[Point]| Rect::from_points(s[0], s[2]);
        fn l(s: &[Point]) -> Vec<(&[Point], bool)> {
            vec![(s, true)]
        }
        assert!(big.holds(b(&inside), &l(&inside), Vec2::ZERO));
        assert!(!big.holds(b(&over), &l(&over), Vec2::ZERO));
        assert!(big.touches(b(&over), &l(&over), Vec2::ZERO));
        assert!(!big.touches(b(&apart), &l(&apart), Vec2::ZERO));
        // The offset moves the outline: the inside square moved 20 mm is
        // apart.
        assert!(!big.touches(b(&inside), &l(&inside), Vec2::new(20.0, 0.0)));
    }

    #[test]
    fn a_bay_of_the_lasso_between_two_inside_points_is_not_inside() {
        // A U shape: both ends of the line are inside, its middle crosses
        // the bay.
        let u = lasso(&[
            (0.0, 0.0),
            (3.0, 0.0),
            (3.0, 8.0),
            (7.0, 8.0),
            (7.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ]);
        let line = [Point::new(1.0, 4.0), Point::new(9.0, 4.0)];
        let lines = [(&line[..], false)];
        let b = Rect::new(1.0, 4.0, 9.0, 4.0);
        assert!(u.contains(line[0]) && u.contains(line[1]));
        assert!(!u.holds(b, &lines, Vec2::ZERO));
        assert!(u.touches(b, &lines, Vec2::ZERO));
    }

    #[test]
    fn a_lasso_inside_a_ring_does_not_touch_it() {
        let ring = square(0.0, 0.0, 10.0);
        let small = lasso(&[(4.0, 4.0), (6.0, 4.0), (5.0, 6.0)]);
        let b = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(!small.touches(b, &[(&ring[..], true)], Vec2::ZERO));
        assert!(!small.holds(b, &[(&ring[..], true)], Vec2::ZERO));
    }

    #[test]
    fn combine_replaces_adds_and_removes() {
        let id = |v: &[u64]| v.iter().map(|i| ObjectId(*i)).collect::<Vec<_>>();
        let cur = id(&[1, 2]);
        let got = vec![id(&[2]), id(&[3])];
        assert_eq!(combine(&cur, &got, Mode::Replace), id(&[2, 3]));
        assert_eq!(combine(&cur, &got, Mode::Add), id(&[1, 2, 3]));
        assert_eq!(combine(&cur, &got, Mode::Remove), id(&[1]));
        // Toggle: 2 was selected and leaves, 3 joins.
        assert_eq!(combine(&cur, &got, Mode::Toggle), id(&[1, 3]));
        // A group toggles as one: partly selected, it joins whole.
        let group = vec![id(&[2, 3])];
        assert_eq!(combine(&cur, &group, Mode::Toggle), id(&[1, 2, 3]));
        assert_eq!(combine(&id(&[1, 2, 3]), &group, Mode::Toggle), id(&[1]));
    }
}
