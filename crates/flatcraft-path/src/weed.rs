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
//!   ([`Weeds::dropped_boxes`]). The box asks [`is_weedable`] along its
//!   whole outline too, and one that would cut kept material is left out
//!   and counted ([`Weeds::blocked_boxes`]).
//! - With a box per object the lines run only inside the boxes, so the
//!   waste between them stays whole on the sheet.
//!
//! [`generate`] gives the weed geometry in a fixed order that does not
//! depend on the order of the design (export and plan share it);
//! [`sequence`] puts it into the cut order after the design: the weed
//! lines, then the box last (it is the outermost outline). With feed
//! bands (no mat) the weed lines and the box outline are cut band by band
//! after each band's design, so the roll never goes back for them; the
//! box pieces overlap by [`BOX_OVERLAP_MM`] at their joins.

use flatcraft_core::document::{ObjectId, WeedSettings};
use kurbo::{Point, Rect};

use crate::containment::{self, KeepMode, KeptMaterial};
use crate::feed::{self, BAND_MM};
use crate::plan::{KeepSide, PathRef, PipelineSettings, Toolpath, ToolpathKind};
use crate::rings;

/// Weed lines keep at least this far from every design cut, mm.
pub const WEED_CLEARANCE_MM: f64 = 1.0;

/// Weed lines are planned this much farther than [`WEED_CLEARANCE_MM`],
/// so rounding never brings them closer.
const SLACK_MM: f64 = 0.01;

/// Pieces of weed line shorter than this are left out: a piece between
/// two shapes keeps 1 mm from each, so a shorter one splits next to
/// nothing and still costs a blade drop and a lift.
pub const MIN_PIECE_MM: f64 = 5.0;

/// How far each band piece of a weed box runs on past its join into the
/// next piece (no mat), so the joins and corners separate cleanly: the
/// default overcut.
pub const BOX_OVERLAP_MM: f64 = crate::plan::DEFAULT_OVERCUT_MM;

/// The object a weed line names in its [`PathRef`]. Documents number
/// their objects from 1, so 0 is never a real object.
pub const WEED_OBJECT: ObjectId = ObjectId(0);

/// The object a weed box (or a band piece of one) names in its
/// [`PathRef`], with the box's index as path index. Documents count their
/// objects up from 1 and never reach it.
pub const WEED_BOX_OBJECT: ObjectId = ObjectId(u64::MAX);

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
    /// Boxes left out because their outline would cut material that is
    /// kept ([`is_weedable`] says no somewhere on it).
    pub blocked_boxes: usize,
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
///
/// The lines run across the design's box plus the margin. With a box per
/// object they run only inside the boxes (merged boxes count as one), so
/// the waste outside every box stays whole on the sheet.
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
    let owned: Vec<Toolpath> = design.iter().map(|t| (*t).clone()).collect();
    let kept = KeptMaterial::from_toolpaths(&owned, mode_of, area, 0.0);
    let m = s.margin_mm;
    // (design bounds, region) per group: the region is the box (clamped
    // to the cutting area) whether or not the box is cut.
    let groups: Vec<(Rect, Rect)> = if s.per_object {
        let mut ids: Vec<ObjectId> = design.iter().map(|t| t.source.object).collect();
        ids.sort();
        ids.dedup();
        let objects: Vec<Rect> = ids
            .iter()
            .filter_map(|id| {
                bbox(
                    design
                        .iter()
                        .filter(|t| t.source.object == *id)
                        .flat_map(|t| t.points.iter()),
                )
            })
            .collect();
        merge_groups(objects, m)
    } else {
        vec![all]
    }
    .into_iter()
    .map(|b| (b, b.inflate(m, m).intersect(area)))
    .collect();
    if s.weed_box {
        let c = WEED_CLEARANCE_MM - 1e-9;
        for &(b, r) in &groups {
            let fits = b.x0 - r.x0 >= c && r.x1 - b.x1 >= c && b.y0 - r.y0 >= c && r.y1 - b.y1 >= c;
            if !fits {
                out.dropped_boxes += 1;
            } else if !box_weedable(&kept, r) {
                out.blocked_boxes += 1;
            } else {
                out.boxes.push(r);
            }
        }
        out.boxes
            .sort_by(|a, b| a.y0.total_cmp(&b.y0).then(a.x0.total_cmp(&b.x0)));
    }
    if s.lines.horizontal() || s.lines.vertical() {
        let segs: Vec<(Point, Point)> = design
            .iter()
            .flat_map(|t| t.points.windows(2).map(|w| (w[0], w[1])))
            .collect();
        let regions: Vec<Rect> = if s.weed_box && s.per_object {
            groups.iter().map(|g| g.1).collect()
        } else {
            vec![all.inflate(m, m).intersect(area)]
        };
        for vertical in [false, true] {
            if (vertical && !s.lines.vertical()) || (!vertical && !s.lines.horizontal()) {
                continue;
            }
            for &region in &regions {
                out.lines
                    .extend(lines(&segs, &kept, region, s.spacing_mm, vertical));
            }
        }
    }
    out
}

/// True when the whole outline of box `r` may be cut: [`is_weedable`] at
/// its corners and at most 1 mm apart along every edge. The box asks the
/// same kept question as the lines, so a keep mode that keeps the sheet
/// drops it.
fn box_weedable(kept: &KeptMaterial, r: Rect) -> bool {
    let corners = [
        Point::new(r.x0, r.y0),
        Point::new(r.x1, r.y0),
        Point::new(r.x1, r.y1),
        Point::new(r.x0, r.y1),
    ];
    (0..4).all(|i| {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let n = a.distance(b).ceil().max(1.0) as usize;
        (0..=n).all(|k| is_weedable(kept, a.lerp(b, k as f64 / n as f64)))
    })
}

/// The bounding box of `points`.
fn bbox<'a>(points: impl Iterator<Item = &'a Point>) -> Option<Rect> {
    points
        .filter(|p| p.is_finite())
        .fold(None, |acc: Option<Rect>, p| {
            Some(acc.map_or_else(|| Rect::from_points(*p, *p), |r| r.union_pt(*p)))
        })
}

/// True when the rectangles `a` and `b` share a point or lie closer than
/// [`MIN_PIECE_MM`] apart in both directions: two boxes that close would
/// leave a thin sliver of waste between two parallel cuts, so they count
/// as touching.
fn touch(a: Rect, b: Rect) -> bool {
    let g = MIN_PIECE_MM;
    a.x0 < b.x1 + g && b.x0 < a.x1 + g && a.y0 < b.y1 + g && b.y0 < a.y1 + g
}

/// Groups of objects whose boxes (`margin` around their bounds) touch
/// ([`touch`], closer than [`MIN_PIECE_MM`] counts), merged until no two
/// boxes touch: the bounds of each group. A merged box
/// is bigger and may touch more, so it repeats; each round is a sweep over
/// x with a union-find, not a scan of every pair.
pub(crate) fn merge_groups(mut groups: Vec<Rect>, margin: f64) -> Vec<Rect> {
    loop {
        let n = groups.len();
        let boxes: Vec<Rect> = groups.iter().map(|b| b.inflate(margin, margin)).collect();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| boxes[a].x0.total_cmp(&boxes[b].x0));
        let mut parent: Vec<usize> = (0..n).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut merged = false;
        let mut active: Vec<usize> = Vec::new();
        for &i in &order {
            active.retain(|&j| boxes[j].x1 + MIN_PIECE_MM > boxes[i].x0);
            for &j in &active {
                if touch(boxes[i], boxes[j]) {
                    let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                    if ri != rj {
                        parent[ri.max(rj)] = ri.min(rj);
                        merged = true;
                    }
                }
            }
            active.push(i);
        }
        if !merged {
            return groups;
        }
        let mut out: Vec<Option<Rect>> = vec![None; n];
        for (i, g) in groups.iter().enumerate() {
            let r = root(&mut parent, i);
            out[r] = Some(out[r].map_or(*g, |b| b.union(*g)));
        }
        groups = out.into_iter().flatten().collect();
    }
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

/// A weed line or box as a toolpath: `Weed` kind, [`WEED_OBJECT`] (the
/// caller sets [`WEED_BOX_OBJECT`] for a box), its index among the lines
/// (renumbered in cut order by [`sequence`]) or the boxes as path index.
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

/// The feed band of a weed piece: by its top, so a piece that ends on a
/// band edge belongs to the band it lies in.
fn piece_band(points: &[Point]) -> usize {
    let y = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    if y.is_finite() && y > 0.0 {
        (y / BAND_MM).floor() as usize
    } else {
        0
    }
}

/// `line` cut exactly at the feed band edges it crosses, so every piece
/// lies in one band. Pieces shorter than `min_mm` (a hair at a band edge)
/// are left out.
fn split_at_bands(line: [Point; 2], min_mm: f64) -> Vec<[Point; 2]> {
    let [a, b] = line;
    let (top, bottom) = if a.y <= b.y { (a, b) } else { (b, a) };
    let mut cuts = vec![top];
    let mut k = (top.y / BAND_MM).floor() + 1.0;
    while k * BAND_MM < bottom.y {
        let t = (k * BAND_MM - top.y) / (bottom.y - top.y);
        cuts.push(top.lerp(bottom, t));
        k += 1.0;
    }
    cuts.push(bottom);
    cuts.windows(2)
        .map(|w| [w[0], w[1]])
        .filter(|p| p[0].distance(p[1]) >= min_mm.max(1e-9))
        .collect()
}

/// A weed piece to order: its object ([`WEED_OBJECT`] for a line,
/// [`WEED_BOX_OBJECT`] for a box), its index, its points and whether it
/// is a closed ring.
struct Piece {
    object: ObjectId,
    index: usize,
    points: Vec<Point>,
    closed: bool,
}

/// Orders `pieces` greedily from `head`: always the nearest end next, cut
/// from that end (a closed ring from its start). Returns the toolpaths and
/// the head after them.
fn greedy(mut pieces: Vec<Piece>, mut head: Point) -> (Vec<Toolpath>, Point) {
    let mut out = Vec::with_capacity(pieces.len());
    while !pieces.is_empty() {
        let mut best = (f64::INFINITY, 0, false);
        for (i, p) in pieces.iter().enumerate() {
            let (Some(a), Some(b)) = (p.points.first(), p.points.last()) else {
                continue;
            };
            for (rev, q) in [(false, a), (true, b)] {
                if rev && p.closed {
                    continue;
                }
                let d = head.distance(*q);
                if d < best.0 {
                    best = (d, i, rev);
                }
            }
        }
        let mut p = pieces.swap_remove(best.1);
        if best.2 {
            p.points.reverse();
        }
        head = p.points.last().copied().unwrap_or(head);
        let mut t = weed_toolpath(p.points, p.closed, p.index);
        t.source.object = p.object;
        out.push(t);
    }
    (out, head)
}

/// Box `r` as band pieces for feed bands: its outline is cut at every band
/// edge it crosses, and each piece runs from one cut round the corners to
/// the next (the top edge in the first band, the bottom edge in the last,
/// the sides band by band). Each piece goes on `overlap` mm along the
/// outline into the next piece, so the joins separate cleanly. A box
/// inside one band is one closed ring. Each piece comes with its band
/// (by the piece without its overlap).
fn box_pieces(r: Rect, overlap: f64) -> Vec<(Vec<Point>, bool, usize)> {
    let edges: Vec<f64> = {
        let mut v = Vec::new();
        let mut k = (r.y0 / BAND_MM).floor() + 1.0;
        while k * BAND_MM < r.y1 {
            v.push(k * BAND_MM);
            k += 1.0;
        }
        v
    };
    let (tl, tr) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0));
    let (br, bl) = (Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
    if edges.is_empty() {
        let band = piece_band(&[tl, bl]);
        return vec![(vec![tl, tr, br, bl, tl], true, band)];
    }
    // The outline clockwise on screen from the first cut on the right
    // side, with the cut points marked.
    let mut ring: Vec<(Point, bool)> = Vec::new();
    for &y in &edges {
        ring.push((Point::new(r.x1, y), true));
    }
    ring.push((br, false));
    ring.push((bl, false));
    for &y in edges.iter().rev() {
        ring.push((Point::new(r.x0, y), true));
    }
    ring.push((tl, false));
    ring.push((tr, false));
    let n = ring.len();
    // Pieces: from each cut to the next cut along the ring.
    let cuts: Vec<usize> = (0..n).filter(|&i| ring[i].1).collect();
    let mut pieces: Vec<Vec<Point>> = Vec::new();
    for (k, &c) in cuts.iter().enumerate() {
        let next = cuts[(k + 1) % cuts.len()];
        let mut pts = vec![ring[c].0];
        let mut i = c;
        loop {
            i = (i + 1) % n;
            pts.push(ring[i].0);
            if i == next {
                break;
            }
        }
        pieces.push(pts);
    }
    // Each piece runs on `overlap` mm into the next one.
    let count = pieces.len();
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let mut pts = pieces[k].clone();
        let mut left = overlap;
        for w in pieces[(k + 1) % count].windows(2) {
            let len = w[0].distance(w[1]);
            if len >= left {
                pts.push(w[0].lerp(w[1], left / len));
                break;
            }
            pts.push(w[1]);
            left -= len;
        }
        // The band of the piece itself, not of its overlap.
        out.push((pts, false, piece_band(&pieces[k])));
    }
    out
}

/// The cut order with the weeds: the ordered `design` (before passes),
/// then the weed lines (greedy from the end of the design), then the
/// boxes, each a closed ring cut in `s.direction` from its corner nearest
/// the head.
///
/// With feed bands (`s.monotonic_feed`, no mat) the roll never goes back
/// for a weed path: the weed lines are cut at the band edges and the box
/// outline into band pieces ([`box_pieces`]), and each band's pieces
/// follow that band's design (by the bands the order uses,
/// [`crate::feed::bands`]), weed lines first, then the box pieces.
pub fn sequence(design: Vec<Toolpath>, weeds: &Weeds, s: &PipelineSettings) -> Vec<Toolpath> {
    if weeds.is_empty() {
        return design;
    }
    // The bands the order cut the design in (a child goes with its
    // parent). Today a child's box lies in its parent's box, so this
    // equals `band_of`; reading the order's own bands keeps the weeds in
    // step if that rule changes.
    let design_bands = if s.monotonic_feed {
        feed::bands(&design, &containment::build(&design))
    } else {
        Vec::new()
    };
    sequence_in_bands(design, &design_bands, weeds, s)
}

/// [`sequence`] with the design's feed bands given (`design_bands[i]` is
/// the band the order put `design[i]` in; only read with feed bands).
fn sequence_in_bands(
    design: Vec<Toolpath>,
    design_bands: &[usize],
    weeds: &Weeds,
    s: &PipelineSettings,
) -> Vec<Toolpath> {
    let end_of = |t: &Toolpath| t.points.last().copied();
    let mut out: Vec<Toolpath> = Vec::with_capacity(design.len() + weeds.lines.len());
    let line = |i: usize, l: [Point; 2]| Piece {
        object: WEED_OBJECT,
        index: i,
        points: l.to_vec(),
        closed: false,
    };
    if s.monotonic_feed {
        // (band, 0 for a line or 1 for a box piece, piece)
        let mut pieces: Vec<(usize, u8, Piece)> = Vec::new();
        for (i, l) in weeds.lines.iter().enumerate() {
            for p in split_at_bands(*l, s.min_cut_mm) {
                pieces.push((piece_band(&p), 0, line(i, p)));
            }
        }
        for (i, r) in weeds.boxes.iter().enumerate() {
            for (points, closed, band) in box_pieces(*r, BOX_OVERLAP_MM) {
                pieces.push((
                    band,
                    1,
                    Piece {
                        object: WEED_BOX_OBJECT,
                        index: i,
                        points,
                        closed,
                    },
                ));
            }
        }
        pieces.sort_by_key(|p| (p.0, p.1));
        let mut pieces = std::collections::VecDeque::from(pieces);
        let mut running = 0;
        let mut head = s.home;
        let mut flush = |band: usize, head: &mut Point, out: &mut Vec<Toolpath>| {
            while pieces.front().is_some_and(|p| p.0 <= band) {
                // One band at a time: its lines, then its box pieces.
                let b = pieces[0].0;
                for class in [0, 1] {
                    let mut chunk = Vec::new();
                    while pieces.front().is_some_and(|p| p.0 == b && p.1 == class) {
                        chunk.extend(pieces.pop_front().map(|p| p.2));
                    }
                    let (paths, h) = greedy(chunk, *head);
                    *head = h;
                    out.extend(paths);
                }
            }
        };
        for (i, t) in design.into_iter().enumerate() {
            let band = design_bands
                .get(i)
                .copied()
                .unwrap_or_else(|| feed::band_of(&t));
            running = running.max(band);
            // Weed pieces of the bands before this design path's band go
            // first: their band is done.
            if running > 0 {
                flush(running - 1, &mut head, &mut out);
            }
            head = end_of(&t).unwrap_or(head);
            out.push(t);
        }
        flush(usize::MAX, &mut head, &mut out);
    } else {
        let head = design.last().and_then(end_of).unwrap_or(s.home);
        out.extend(design);
        let lines = weeds.lines.iter().enumerate().map(|(i, l)| line(i, *l));
        out.extend(greedy(lines.collect(), head).0);
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
            let mut t = weed_toolpath(points, true, i);
            t.source.object = WEED_BOX_OBJECT;
            out.push(t);
        }
    }
    // Number the weed lines in cut order, so the listing names each piece
    // once (band pieces of one line get their own numbers).
    for (n, t) in out
        .iter_mut()
        .filter(|t| t.kind == ToolpathKind::Weed && t.source.object == WEED_OBJECT)
        .enumerate()
    {
        t.source.path_index = n;
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
    fn band_split_cuts_exactly_at_band_edges() {
        let pieces = split_at_bands([Point::new(5.0, 49.0), Point::new(5.0, 101.0)], 0.025);
        let ys: Vec<(f64, f64)> = pieces.iter().map(|p| (p[0].y, p[1].y)).collect();
        // Every piece in one band, even the short ones at the ends.
        assert_eq!(ys, vec![(49.0, 50.0), (50.0, 100.0), (100.0, 101.0)]);
        // A hair below the shortest cut is left out.
        let pieces = split_at_bands([Point::new(5.0, 10.0), Point::new(5.0, 50.01)], 0.025);
        assert_eq!(pieces.len(), 1);
    }

    /// The old merge: scan every pair, merge, start again. The sweep must
    /// give the same groups.
    fn merge_naive(mut groups: Vec<Rect>, m: f64) -> Vec<Rect> {
        'merge: loop {
            for i in 0..groups.len() {
                for j in 0..groups.len() {
                    if i != j && touch(groups[i].inflate(m, m), groups[j].inflate(m, m)) {
                        let b = groups[i].union(groups[j]);
                        groups[i.min(j)] = b;
                        groups.remove(i.max(j));
                        continue 'merge;
                    }
                }
            }
            return groups;
        }
    }

    #[test]
    fn the_sweep_merge_matches_the_pairwise_merge() {
        // A small fixed pseudo random generator: the same layouts on
        // every machine.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |lo: f64, hi: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            lo + (seed % 10_000) as f64 / 10_000.0 * (hi - lo)
        };
        let key = |r: &Rect| (r.x0, r.y0, r.x1, r.y1);
        for case in 0..200 {
            let n = 1 + case % 40;
            let rects: Vec<Rect> = (0..n)
                .map(|_| {
                    let (x, y) = (next(0.0, 280.0), next(0.0, 280.0));
                    Rect::new(x, y, x + next(1.0, 30.0), y + next(1.0, 30.0))
                })
                .collect();
            let m = next(1.0, 10.0);
            let mut a = merge_groups(rects.clone(), m);
            let mut b = merge_naive(rects, m);
            a.sort_by(|p, q| key(p).partial_cmp(&key(q)).unwrap());
            b.sort_by(|p, q| key(p).partial_cmp(&key(q)).unwrap());
            assert_eq!(a, b, "case {case}");
        }
    }

    #[test]
    fn a_box_across_bands_is_band_pieces_that_overlap() {
        let r = Rect::new(10.0, 30.0, 60.0, 130.0);
        let pieces = box_pieces(r, 0.5);
        // Cuts at 50 and 100 on both sides: four pieces, the top one round
        // the top corners, the bottom one round the bottom corners.
        assert_eq!(pieces.len(), 4);
        assert!(pieces.iter().all(|(_, closed, _)| !closed));
        let total: f64 = pieces
            .iter()
            .map(|(p, _, _)| p.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>())
            .sum();
        let perimeter = 2.0 * (r.width() + r.height());
        assert!((total - perimeter - 4.0 * 0.5).abs() < 1e-9, "{total}");
        // One band: one closed ring.
        let one = box_pieces(Rect::new(10.0, 5.0, 60.0, 45.0), 0.5);
        assert_eq!(one.len(), 1);
        assert!(one[0].1);
    }

    #[test]
    fn weed_pieces_follow_the_bands_of_the_order_not_band_of() {
        // Design path A pokes into band 1 (max y 55) but the order put it
        // in band 0 (as if a child that goes with its parent); B is in
        // band 1. A weed line in band 0 belongs after A: the order still
        // cuts band 0 there. Reading `band_of` would put it before A.
        let path = |object: u64, y1: f64| {
            Toolpath::new(
                vec![Point::new(10.0, 10.0), Point::new(10.0, y1)],
                false,
                PathRef {
                    object: ObjectId(object),
                    path_index: 0,
                },
            )
        };
        let design = vec![path(1, 55.0), path(2, 60.0)];
        let weeds = Weeds {
            lines: vec![[Point::new(30.0, 20.0), Point::new(60.0, 20.0)]],
            ..Weeds::default()
        };
        let mut s = PipelineSettings::with_defaults(
            Rect::new(0.0, 0.0, 300.0, 300.0),
            Point::new(0.0, 0.0),
        );
        s.monotonic_feed = true;
        let kinds = |out: &[Toolpath]| -> Vec<u64> {
            out.iter()
                .map(|t| {
                    if t.kind == ToolpathKind::Weed {
                        0
                    } else {
                        t.source.object.0
                    }
                })
                .collect()
        };
        let out = sequence_in_bands(design.clone(), &[0, 1], &weeds, &s);
        assert_eq!(kinds(&out), vec![1, 0, 2]);
        // With the bands `band_of` reads (1 and 1) the weed line goes
        // first: the test tells the two apart.
        let out = sequence_in_bands(design, &[1, 1], &weeds, &s);
        assert_eq!(kinds(&out), vec![0, 1, 2]);
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
