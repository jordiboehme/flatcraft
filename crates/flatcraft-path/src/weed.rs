//! Weed box and weed lines (Task 51).
//!
//! Weeding means peeling the waste off the backing after the cut. A weed
//! box is a rectangle around the design, so the waste around it comes off
//! as one piece and the rest of the sheet stays clean. Weed lines are
//! straight cuts through the waste that split it into strips, so a big
//! area of waste does not have to be peeled in one go.
//!
//! Both are cut only through waste:
//!
//! - A weed line keeps more than [`WEED_CLEARANCE_MM`] from every design
//!   cut, open or closed: it is split at every design shape. Only the
//!   background is cut: a piece of line inside any ring (a piece, or the
//!   counter of a letter) is left out, so no small bits fall out of a
//!   counter. Which side is kept is asked in one place,
//!   [`is_weedable`], so per-object keep modes (Task 29) plug in there.
//! - The weed box keeps the margin from the design. Boxes per object
//!   that would overlap each other or another object are merged into one.
//!   A box that does not fit inside the cutting area with at least the
//!   clearance to the design is left out and counted
//!   ([`Weeds::dropped_boxes`]).
//!
//! [`generate`] gives the weed geometry in a fixed order that does not
//! depend on the order of the design (export and plan share it);
//! [`sequence`] puts it into the cut order after the design: the weed
//! lines, then the box last (it is the outermost outline). With feed
//! bands (no mat) each band's weed lines follow that band's design, so
//! the head still never goes back more than a band; the box stays last.

use flatcraft_core::document::{ObjectId, WeedSettings};
use kurbo::{Point, Rect};

use crate::containment::{KeepMode, KeptMaterial};
use crate::feed::{BAND_MM, band_of};
use crate::plan::{KeepSide, PathRef, PipelineSettings, Toolpath, ToolpathKind};
use crate::rings;

/// Weed lines keep at least this far from every design cut, mm.
pub const WEED_CLEARANCE_MM: f64 = 1.0;

/// Weed lines are planned this much farther than [`WEED_CLEARANCE_MM`],
/// so rounding never brings them closer.
const SLACK_MM: f64 = 0.01;

/// Pieces of weed line shorter than this are left out: they split nothing
/// worth splitting.
pub const MIN_PIECE_MM: f64 = 2.0;

/// The object a weed toolpath names in its [`PathRef`]. Documents number
/// their objects from 1, so 0 is never a real object.
pub const WEED_OBJECT: ObjectId = ObjectId(0);

/// The weed geometry of a job.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Weeds {
    /// Straight weed lines, each from its start to its end: horizontal
    /// lines first (by y, then x), then vertical lines (by x, then y).
    pub lines: Vec<[Point; 2]>,
    /// Weed boxes, by top edge, then left edge.
    pub boxes: Vec<Rect>,
    /// Boxes left out because they do not fit inside the cutting area
    /// with the clearance to the design.
    pub dropped_boxes: usize,
}

impl Weeds {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.boxes.is_empty()
    }
}

/// True when a weed line may cut at `p`: `p` is waste and lies in the
/// background, inside no ring. The one place that decides which side is
/// kept for weeding; the clearance to the cuts is checked separately.
pub fn is_weedable(kept: &KeptMaterial, p: Point) -> bool {
    !kept.is_kept(p) && !kept.enclosed(p)
}

/// The weed geometry for `design`: the job's `Cut` toolpaths as one lap
/// each (pass 1, before passes and overcut), in any order. `mode_of`
/// gives each object's keep mode, `area` is the cutting area. Nothing
/// when weeding is off or there is no design.
pub fn generate(
    design: &[Toolpath],
    mode_of: impl Fn(ObjectId) -> KeepMode,
    area: Rect,
    s: &WeedSettings,
) -> Weeds {
    let mut out = Weeds::default();
    let design: Vec<&Toolpath> = design
        .iter()
        .filter(|t| t.kind == ToolpathKind::Cut && t.pass == 1 && !t.points.is_empty())
        .collect();
    if !s.any() || design.is_empty() {
        return out;
    }
    let Some(all) = bbox(design.iter().flat_map(|t| t.points.iter())) else {
        return out;
    };
    if s.weed_box {
        (out.boxes, out.dropped_boxes) = boxes(&design, all, area, s);
    }
    if s.lines.horizontal() || s.lines.vertical() {
        let owned: Vec<Toolpath> = design.iter().map(|t| (*t).clone()).collect();
        let kept = KeptMaterial::from_toolpaths(&owned, mode_of, area, 0.0);
        let segs: Vec<(Point, Point)> = design
            .iter()
            .flat_map(|t| t.points.windows(2).map(|w| (w[0], w[1])))
            .collect();
        let region = all.inflate(s.margin_mm, s.margin_mm).intersect(area);
        if s.lines.horizontal() {
            out.lines
                .extend(lines(&segs, &kept, region, s.spacing_mm, false));
        }
        if s.lines.vertical() {
            out.lines
                .extend(lines(&segs, &kept, region, s.spacing_mm, true));
        }
    }
    out
}

/// The bounding box of `points`.
fn bbox<'a>(points: impl Iterator<Item = &'a Point>) -> Option<Rect> {
    points
        .filter(|p| p.is_finite())
        .fold(None, |acc: Option<Rect>, p| {
            Some(acc.map_or_else(|| Rect::from_points(*p, *p), |r| r.union_pt(*p)))
        })
}

/// True when the closed rectangles `a` and `b` share a point.
fn touch(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// The weed boxes and how many were left out.
fn boxes(design: &[&Toolpath], all: Rect, area: Rect, s: &WeedSettings) -> (Vec<Rect>, usize) {
    let m = s.margin_mm;
    // (design bounds, box) per group.
    let mut groups: Vec<(Rect, Rect)> = if s.per_object {
        let mut ids: Vec<ObjectId> = design.iter().map(|t| t.source.object).collect();
        ids.sort();
        ids.dedup();
        ids.iter()
            .filter_map(|id| {
                bbox(
                    design
                        .iter()
                        .filter(|t| t.source.object == *id)
                        .flat_map(|t| t.points.iter()),
                )
            })
            .map(|b| (b, b.inflate(m, m)))
            .collect()
    } else {
        vec![(all, all.inflate(m, m))]
    };
    // Merge until no box touches another group's box or design: then a
    // box crosses no other object's cuts and no other box.
    'merge: loop {
        for i in 0..groups.len() {
            for j in 0..groups.len() {
                if i == j {
                    continue;
                }
                let (bi, ri) = groups[i];
                let (bj, rj) = groups[j];
                if touch(ri, rj) || touch(ri, bj) {
                    let b = bi.union(bj);
                    groups[i.min(j)] = (b, b.inflate(m, m));
                    groups.remove(i.max(j));
                    continue 'merge;
                }
            }
        }
        break;
    }
    let mut out = Vec::new();
    let mut dropped = 0;
    let c = WEED_CLEARANCE_MM - 1e-9;
    for (b, r) in groups {
        let r = r.intersect(area);
        let fits = b.x0 - r.x0 >= c && r.x1 - b.x1 >= c && b.y0 - r.y0 >= c && r.y1 - b.y1 >= c;
        if fits {
            out.push(r);
        } else {
            dropped += 1;
        }
    }
    out.sort_by(|a, b| a.y0.total_cmp(&b.y0).then(a.x0.total_cmp(&b.x0)));
    (out, dropped)
}

/// The x interval of the horizontal line at height `c` that lies within
/// `r` of the segment `a`-`b` (a capsule, so the interval is one piece).
fn capsule_span(a: Point, b: Point, c: f64, r: f64) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut add = |span: Option<(f64, f64)>| {
        if let Some((u, v)) = span
            && u <= v
        {
            lo = lo.min(u);
            hi = hi.max(v);
        }
    };
    let disk = |p: Point| {
        let dy = c - p.y;
        (dy.abs() <= r).then(|| {
            let w = (r * r - dy * dy).max(0.0).sqrt();
            (p.x - w, p.x + w)
        })
    };
    add(disk(a));
    add(disk(b));
    let d = b - a;
    let len2 = d.hypot2();
    if len2 > 0.0 {
        let len = len2.sqrt();
        // The projection t(x) = ((x - a.x) d.x + (c - a.y) d.y) / len2 in
        // [0, 1], and the distance to the line |d.x (c - a.y) - d.y (x -
        // a.x)| / len at most r. Both are linear in x.
        let t = linear_span(d.x / len2, ((c - a.y) * d.y - a.x * d.x) / len2, 0.0, 1.0);
        let f = linear_span(-d.y / len, (d.x * (c - a.y) + d.y * a.x) / len, -r, r);
        if let (Some((t0, t1)), Some((f0, f1))) = (t, f) {
            add(Some((t0.max(f0), t1.min(f1))));
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// The x with `lo <= k x + q <= hi`: an interval, everything (an
/// unbounded interval) or nothing.
fn linear_span(k: f64, q: f64, lo: f64, hi: f64) -> Option<(f64, f64)> {
    if k == 0.0 {
        return (lo <= q && q <= hi).then_some((f64::NEG_INFINITY, f64::INFINITY));
    }
    let (u, v) = ((lo - q) / k, (hi - q) / k);
    Some((u.min(v), u.max(v)))
}

/// The weed lines across `region`, every `spacing` mm, split at every
/// design cut. `vertical` lines run along y; they are worked out as
/// horizontal lines with x and y swapped.
fn lines(
    segs: &[(Point, Point)],
    kept: &KeptMaterial,
    region: Rect,
    spacing: f64,
    vertical: bool,
) -> Vec<[Point; 2]> {
    let swap = |p: Point| if vertical { Point::new(p.y, p.x) } else { p };
    let segs: Vec<(Point, Point)> = segs.iter().map(|&(a, b)| (swap(a), swap(b))).collect();
    let (x0, x1, y0, y1) = if vertical {
        (region.y0, region.y1, region.x0, region.x1)
    } else {
        (region.x0, region.x1, region.y0, region.y1)
    };
    let r = WEED_CLEARANCE_MM + SLACK_MM;
    let mut out = Vec::new();
    let mut k = 1;
    loop {
        let c = y0 + k as f64 * spacing;
        if c >= y1 - 1e-9 || !c.is_finite() {
            break;
        }
        k += 1;
        let mut blocked: Vec<(f64, f64)> = segs
            .iter()
            .filter(|(a, b)| a.y.min(b.y) - r <= c && c <= a.y.max(b.y) + r)
            .filter_map(|&(a, b)| capsule_span(a, b, c, r))
            .collect();
        blocked.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut free = Vec::new();
        let mut at = x0;
        for (u, v) in blocked {
            if u > at {
                free.push((at, u.min(x1)));
            }
            at = at.max(v);
            if at >= x1 {
                break;
            }
        }
        if at < x1 {
            free.push((at, x1));
        }
        for (u, v) in free {
            if v - u < MIN_PIECE_MM {
                continue;
            }
            // No cut crosses a free piece, so one point tells its side.
            let mid = swap(Point::new((u + v) / 2.0, c));
            if is_weedable(kept, mid) {
                out.push([swap(Point::new(u, c)), swap(Point::new(v, c))]);
            }
        }
    }
    out
}

/// A weed line or box as a toolpath: `Weed` kind, [`WEED_OBJECT`], its
/// index among the lines (renumbered in cut order by [`sequence`]) or the
/// boxes as path index.
fn weed_toolpath(points: Vec<Point>, closed: bool, index: usize) -> Toolpath {
    let mut t = Toolpath::new(
        points,
        closed,
        PathRef {
            object: WEED_OBJECT,
            path_index: index,
        },
    );
    t.kind = ToolpathKind::Weed;
    // The box holds the design inside; the outside is waste.
    t.keep = if closed {
        KeepSide::Inside
    } else {
        KeepSide::Both
    };
    t
}

/// The band of a weed line piece: by its top end, so a piece that ends on
/// a band edge belongs to the band it lies in.
fn piece_band(a: Point, b: Point) -> usize {
    let y = a.y.min(b.y);
    if y.is_finite() && y > 0.0 {
        (y / BAND_MM).floor() as usize
    } else {
        0
    }
}

/// `line` cut at the feed band edges it crosses. A piece shorter than
/// [`MIN_PIECE_MM`] stays with its neighbour.
fn split_at_bands(line: [Point; 2]) -> Vec<[Point; 2]> {
    let [a, b] = line;
    let (top, bottom) = if a.y <= b.y { (a, b) } else { (b, a) };
    let mut cuts = vec![top];
    let mut k = (top.y / BAND_MM).floor() + 1.0;
    while k * BAND_MM < bottom.y {
        let y = k * BAND_MM;
        let t = (y - top.y) / (bottom.y - top.y);
        let p = top.lerp(bottom, t);
        if p.distance(*cuts.last().unwrap_or(&top)) >= MIN_PIECE_MM
            && p.distance(bottom) >= MIN_PIECE_MM
        {
            cuts.push(p);
        }
        k += 1.0;
    }
    cuts.push(bottom);
    cuts.windows(2).map(|w| [w[0], w[1]]).collect()
}

/// Orders `pieces` greedily from `head`: always the nearest end next,
/// cut from that end. Returns the toolpaths and the head after them.
fn greedy(mut pieces: Vec<(usize, [Point; 2])>, mut head: Point) -> (Vec<Toolpath>, Point) {
    let mut out = Vec::with_capacity(pieces.len());
    while !pieces.is_empty() {
        let mut best = (f64::INFINITY, 0, false);
        for (i, (_, [a, b])) in pieces.iter().enumerate() {
            for (rev, p) in [(false, a), (true, b)] {
                let d = head.distance(*p);
                if d < best.0 {
                    best = (d, i, rev);
                }
            }
        }
        let (index, [a, b]) = pieces.swap_remove(best.1);
        let (a, b) = if best.2 { (b, a) } else { (a, b) };
        out.push(weed_toolpath(vec![a, b], false, index));
        head = b;
    }
    (out, head)
}

/// The cut order with the weeds: the ordered `design` (before passes),
/// then the weed lines (greedy from the end of the design), then the
/// boxes. With feed bands (`s.monotonic_feed`) the weed lines are split
/// at the band edges and each band's pieces follow that band's design.
/// Each box is cut in `s.direction` from its corner nearest the head.
pub fn sequence(design: Vec<Toolpath>, weeds: &Weeds, s: &PipelineSettings) -> Vec<Toolpath> {
    if weeds.is_empty() {
        return design;
    }
    let end_of = |t: &Toolpath| t.points.last().copied();
    let mut out: Vec<Toolpath> = Vec::with_capacity(design.len() + weeds.lines.len());
    let indexed: Vec<(usize, [Point; 2])> = weeds.lines.iter().copied().enumerate().collect();
    if s.monotonic_feed {
        let mut pieces: Vec<(usize, usize, [Point; 2])> = indexed
            .iter()
            .flat_map(|&(i, l)| split_at_bands(l).into_iter().map(move |p| (i, p)))
            .map(|(i, p)| (piece_band(p[0], p[1]), i, p))
            .collect();
        pieces.sort_by_key(|p| p.0);
        let mut running = 0;
        let mut next = 0;
        let mut head = s.home;
        let flush = |band: usize, next: &mut usize, head: &mut Point, out: &mut Vec<Toolpath>| {
            let start = *next;
            while *next < pieces.len() && pieces[*next].0 <= band {
                *next += 1;
            }
            let chunk: Vec<(usize, [Point; 2])> =
                pieces[start..*next].iter().map(|p| (p.1, p.2)).collect();
            let (paths, h) = greedy(chunk, *head);
            *head = h;
            out.extend(paths);
        };
        for t in design {
            running = running.max(band_of(&t));
            // Weed pieces of the bands before this design path's band go
            // first: their band is done.
            if running > 0 {
                flush(running - 1, &mut next, &mut head, &mut out);
            }
            head = end_of(&t).unwrap_or(head);
            out.push(t);
        }
        flush(usize::MAX, &mut next, &mut head, &mut out);
    } else {
        let head = design.last().and_then(end_of).unwrap_or(s.home);
        out.extend(design);
        out.extend(greedy(indexed, head).0);
    }
    // Number the weed lines in cut order, so the listing names each piece
    // once (band pieces of one line get their own numbers).
    for (n, t) in out
        .iter_mut()
        .filter(|t| t.kind == ToolpathKind::Weed)
        .enumerate()
    {
        t.source.path_index = n;
    }
    for (i, r) in weeds.boxes.iter().enumerate() {
        let head = out.last().and_then(end_of).unwrap_or(s.home);
        let corners = [
            Point::new(r.x0, r.y0),
            Point::new(r.x1, r.y0),
            Point::new(r.x1, r.y1),
            Point::new(r.x0, r.y1),
        ];
        let ring = rings::orient(&corners, s.direction);
        let start = (0..ring.len())
            .min_by(|&a, &b| head.distance(ring[a]).total_cmp(&head.distance(ring[b])))
            .unwrap_or(0);
        let mut points = rings::rotate_start(&ring, start);
        points.push(points[0]);
        out.push(weed_toolpath(points, true, i));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatcraft_core::document::WeedLines;

    #[test]
    fn capsule_span_of_a_horizontal_segment() {
        let span = capsule_span(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 0.6, 1.0).unwrap();
        let w = (1.0f64 - 0.36).sqrt();
        assert!((span.0 + w).abs() < 1e-12 && (span.1 - 10.0 - w).abs() < 1e-12);
        assert!(capsule_span(Point::new(0.0, 0.0), Point::new(10.0, 0.0), 1.5, 1.0).is_none());
    }

    #[test]
    fn capsule_span_of_a_diagonal_matches_brute_force() {
        let (a, b) = (Point::new(2.0, -3.0), Point::new(7.0, 4.0));
        let r = 1.0;
        for c in [-3.5, -1.0, 0.0, 2.5, 4.7] {
            let span = capsule_span(a, b, c, r);
            let line = kurbo::Line::new(a, b);
            use kurbo::ParamCurveNearest as _;
            let inside: Vec<f64> = (-2000..2000)
                .map(|i| i as f64 * 0.01)
                .filter(|&x| line.nearest(Point::new(x, c), 0.0).distance_sq.sqrt() <= r)
                .collect();
            match span {
                None => assert!(inside.is_empty(), "{c}"),
                Some((lo, hi)) => {
                    assert!(
                        (inside[0] - lo).abs() <= 0.011,
                        "{c}: {lo} vs {}",
                        inside[0]
                    );
                    let last = inside[inside.len() - 1];
                    assert!((last - hi).abs() <= 0.011, "{c}: {hi} vs {last}");
                }
            }
        }
    }

    #[test]
    fn band_split_keeps_short_ends_with_their_neighbour() {
        let pieces = split_at_bands([Point::new(5.0, 10.0), Point::new(5.0, 101.0)]);
        let ys: Vec<(f64, f64)> = pieces.iter().map(|p| (p[0].y, p[1].y)).collect();
        // The cut at 100 would leave 1 mm: it stays with the piece above.
        assert_eq!(ys, vec![(10.0, 50.0), (50.0, 101.0)]);
    }

    #[test]
    fn nothing_when_off() {
        // structural
        let t = Toolpath::new(
            vec![
                Point::new(10.0, 10.0),
                Point::new(20.0, 10.0),
                Point::new(20.0, 20.0),
                Point::new(10.0, 10.0),
            ],
            true,
            PathRef {
                object: ObjectId(1),
                path_index: 0,
            },
        );
        let s = WeedSettings {
            lines: WeedLines::Off,
            weed_box: false,
            ..WeedSettings::default()
        };
        let w = generate(
            &[t],
            |_| KeepMode::Pieces,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            &s,
        );
        assert!(w.is_empty());
    }
}
