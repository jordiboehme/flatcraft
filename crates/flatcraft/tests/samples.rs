//! The samples on Home: original designs made for FlatCraft from simple
//! geometry and IBM Plex lettering (OFL-1.1, outlined here), bundled as
//! project files under MPL-2.0.
//!
//! `regenerate_samples` (ignored) writes them to `assets/samples`:
//! `cargo nextest run -p flatcraft --test samples --run-ignored only`.

mod common;

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use common::{
    FakePicker, app, app_mut, harness_sized, harness_with, session, wait_for_source_check,
    wait_until,
};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
use flatcraft::ui::home::SAMPLES;
use flatcraft_engine::session::Origin;
use serde_json::json;

// ---------------------------------------------------------------- geometry

fn poly(points: &[(f64, f64)]) -> String {
    let mut d = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        let _ = write!(d, "{}{x:.3},{y:.3} ", if i == 0 { "M" } else { "L" });
    }
    d.push('Z');
    format!(r#"<path d="{d}"/>"#)
}

fn circle(cx: f64, cy: f64, r: f64) -> String {
    format!(r#"<circle cx="{cx:.3}" cy="{cy:.3}" r="{r:.3}"/>"#)
}

/// A 300 x 300 mm page, so the shapes keep their place on the mat.
fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="300mm" viewBox="0 0 300 300">{body}</svg>"#
    )
}

fn polar(cx: f64, cy: f64, r: f64, a: f64) -> (f64, f64) {
    (cx + r * a.sin(), cy - r * a.cos())
}

/// A diamond along the direction `a`, from radius `r0` to `r1`, `w` wide.
fn diamond(cx: f64, cy: f64, a: f64, r0: f64, r1: f64, w: f64) -> String {
    let mid = (r0 + r1) / 2.0;
    let (mx, my) = polar(cx, cy, mid, a);
    let (sx, sy) = (a.cos() * w / 2.0, a.sin() * w / 2.0);
    poly(&[
        polar(cx, cy, r0, a),
        (mx + sx, my + sy),
        polar(cx, cy, r1, a),
        (mx - sx, my - sy),
    ])
}

/// The points of an arc around (cx, cy) from angle `a0` to `a1` (radians,
/// screen angles: 0 is +x, y down), ends included.
fn arc(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> Vec<(f64, f64)> {
    let n = ((a1 - a0).abs() * r / 0.5).ceil().max(4.0) as usize;
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f64 / n as f64;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// A rectangle with round corners of radius `r`, clockwise on screen.
fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> Vec<(f64, f64)> {
    let q = PI / 2.0;
    [
        arc(x + w - r, y + r, r, -q, 0.0),
        arc(x + w - r, y + h - r, r, 0.0, q),
        arc(x + r, y + h - r, r, q, 2.0 * q),
        arc(x + r, y + r, r, 2.0 * q, 3.0 * q),
    ]
    .concat()
}

/// A polygon with round corners: each corner of `corners` is replaced by
/// an arc of radius `r` that touches both of its edges.
fn round_corners(corners: &[(f64, f64)], r: f64) -> Vec<(f64, f64)> {
    let n = corners.len();
    let mut out = Vec::new();
    for i in 0..n {
        let p = corners[i];
        let a = corners[(i + n - 1) % n];
        let b = corners[(i + 1) % n];
        let unit = |(x, y): (f64, f64)| {
            let l = x.hypot(y);
            (x / l, y / l)
        };
        let u = unit((a.0 - p.0, a.1 - p.1));
        let v = unit((b.0 - p.0, b.1 - p.1));
        let half = (u.0 * v.0 + u.1 * v.1).clamp(-1.0, 1.0).acos() / 2.0;
        let along = r / half.tan();
        let bis = unit((u.0 + v.0, u.1 + v.1));
        let dist = r / half.sin();
        let c = (p.0 + bis.0 * dist, p.1 + bis.1 * dist);
        let s = (p.0 + u.0 * along, p.1 + u.1 * along);
        let e = (p.0 + v.0 * along, p.1 + v.1 * along);
        let a0 = (s.1 - c.1).atan2(s.0 - c.0);
        let mut a1 = (e.1 - c.1).atan2(e.0 - c.0);
        // The short way round: a corner arc never turns more than PI.
        while a1 - a0 > PI {
            a1 -= TAU;
        }
        while a0 - a1 > PI {
            a1 += TAU;
        }
        out.extend(arc(c.0, c.1, r, a0, a1));
    }
    out
}

/// `points` moved `d` mm inwards, each edge parallel to its original.
/// Good for the gentle shapes of this file: no edge is shorter than the
/// move and no outward curve is tighter than it.
fn inset(points: &[(f64, f64)], d: f64) -> Vec<(f64, f64)> {
    let n = points.len();
    let area: f64 = (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    let side = area.signum();
    let normal = |i: usize| {
        let (a, b) = (points[i], points[(i + 1) % n]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = dx.hypot(dy);
        (-dy / l * side, dx / l * side)
    };
    (0..n)
        .map(|i| {
            let (n1, n2) = (normal((i + n - 1) % n), normal(i));
            let k = d / (1.0 + n1.0 * n2.0 + n1.1 * n2.1);
            (
                points[i].0 + (n1.0 + n2.0) * k,
                points[i].1 + (n1.1 + n2.1) * k,
            )
        })
        .filter(|p| p.0.is_finite() && p.1.is_finite())
        .collect()
}

// ---------------------------------------------------------------- lettering

/// The IBM Plex Sans weights FlatCraft ships (OFL-1.1), the only fonts the
/// lettering of the samples is set in.
const PLEX: [&[u8]; 3] = [
    include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../../../assets/fonts/IBMPlexSans-Medium.ttf"),
    include_bytes!("../../../assets/fonts/IBMPlexSans-SemiBold.ttf"),
];

/// Lines of IBM Plex Sans SemiBold, centred on `x`, the block of lines
/// centred on `y` (by their capital letters), `size` mm, `lead` mm from one
/// baseline to the next. `spacing` is extra space between letters, in mm.
fn words(x: f64, y: f64, size: f64, lead: f64, spacing: f64, lines: &[&str]) -> String {
    let mut out = String::new();
    let first = y - (lines.len() as f64 - 1.0) * lead / 2.0 + 0.349 * size;
    for (i, line) in lines.iter().enumerate() {
        let line = line.replace('&', "&amp;");
        let _ = write!(
            out,
            r#"<text x="{:.3}" y="{:.3}" font-family="IBM Plex Sans" font-weight="600" font-size="{size}" letter-spacing="{spacing}" text-anchor="middle">{line}</text>"#,
            // Letter spacing also follows the last letter: half of it
            // to the right keeps the line centred.
            x + spacing / 2.0,
            first + i as f64 * lead
        );
    }
    LETTERING.with(|l| l.borrow_mut().push(out.clone()));
    out
}

thread_local! {
    /// Every line block [`words`] made on this thread, so a test can check
    /// all the lettering of the samples.
    static LETTERING: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `body` with every `<text>` turned into the outlines of its letters, as
/// paths in page mm. The samples keep no text, so they look the same on
/// every computer, whatever fonts it has.
fn outlined(body: &str) -> String {
    outlined_with(body, true)
}

/// [`outlined`], with the ink traps of the letters filled or not.
fn outlined_with(body: &str, fill: bool) -> String {
    let mut fonts = usvg::fontdb::Database::new();
    for font in PLEX {
        fonts.load_font_data(font.to_vec());
    }
    let options = usvg::Options {
        fontdb: std::sync::Arc::new(fonts),
        ..usvg::Options::default()
    };
    let page = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="300" viewBox="0 0 300 300">{body}</svg>"#
    );
    let tree = usvg::Tree::from_str(&page, &options).unwrap();
    let mut out = String::new();
    walk(tree.root(), false, fill, &mut out);
    out
}

/// Writes the paths of `group` in page mm. The letters (`letters`) get
/// their ink traps filled, see [`fill_ink_traps`].
fn walk(group: &usvg::Group, letters: bool, fill: bool, out: &mut String) {
    use usvg::tiny_skia_path::PathSegment;
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => walk(g, letters, fill, out),
            usvg::Node::Text(t) => walk(t.flattened(), fill, fill, out),
            usvg::Node::Path(p) => {
                let path = p.data().clone().transform(p.abs_transform()).unwrap();
                let mut d = String::new();
                if letters {
                    let mut bez = kurbo::BezPath::new();
                    let pt =
                        |a: usvg::tiny_skia_path::Point| kurbo::Point::new(a.x.into(), a.y.into());
                    for seg in path.segments() {
                        match seg {
                            PathSegment::MoveTo(a) => bez.move_to(pt(a)),
                            PathSegment::LineTo(a) => bez.line_to(pt(a)),
                            PathSegment::QuadTo(a, b) => bez.quad_to(pt(a), pt(b)),
                            PathSegment::CubicTo(a, b, c) => bez.curve_to(pt(a), pt(b), pt(c)),
                            PathSegment::Close => bez.close_path(),
                        }
                    }
                    for line in fill_ink_traps(&bez) {
                        for (i, q) in line.iter().enumerate() {
                            let _ = write!(
                                d,
                                "{}{:.3},{:.3}",
                                if i == 0 { "M" } else { "L" },
                                q.x,
                                q.y
                            );
                        }
                        d.push('Z');
                    }
                } else {
                    for seg in path.segments() {
                        let _ = match seg {
                            PathSegment::MoveTo(a) => write!(d, "M{:.3},{:.3}", a.x, a.y),
                            PathSegment::LineTo(a) => write!(d, "L{:.3},{:.3}", a.x, a.y),
                            PathSegment::QuadTo(a, b) => {
                                write!(d, "Q{:.3},{:.3} {:.3},{:.3}", a.x, a.y, b.x, b.y)
                            }
                            PathSegment::CubicTo(a, b, c) => write!(
                                d,
                                "C{:.3},{:.3} {:.3},{:.3} {:.3},{:.3}",
                                a.x, a.y, b.x, b.y, c.x, c.y
                            ),
                            PathSegment::Close => write!(d, "Z"),
                        };
                    }
                }
                let _ = write!(out, r#"<path d="{d}"/>"#);
            }
            usvg::Node::Image(_) => {}
        }
    }
}

/// True when `p` is inside the shape of `lines` (even-odd).
fn inside(lines: &[Vec<kurbo::Point>], p: kurbo::Point) -> bool {
    let mut odd = false;
    for line in lines {
        for (i, a) in line.iter().enumerate() {
            let b = line[(i + 1) % line.len()];
            if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
                odd = !odd;
            }
        }
    }
    odd
}

/// The outlines of a line of letters as closed polylines, with the ink
/// traps filled. IBM Plex cuts small slits into some joins (in G, B and
/// many lower case letters) to soak up ink spread in print. Vinyl has no
/// ink, and a slit leaves a neck under 1 mm that tears when you weed. A
/// slit is a notch of the background into a letter, under 0.5 mm wide at
/// its mouth, at least as deep as it is wide and at most 4 mm round: it is
/// closed with a straight line across its mouth. Counters, the crotches of
/// V, W and M and open counters like the e's are wider or longer, and stay.
fn fill_ink_traps(path: &kurbo::BezPath) -> Vec<Vec<kurbo::Point>> {
    const MOUTH: f64 = 0.5;
    const ROUND: f64 = 4.0;
    const STEP: f64 = 0.05;
    // Flattened, then no segment longer than STEP.
    let mut lines: Vec<Vec<kurbo::Point>> = Vec::new();
    kurbo::flatten(path.iter(), 0.005, |el| match el {
        kurbo::PathEl::MoveTo(p) => lines.push(vec![p]),
        kurbo::PathEl::LineTo(p) => {
            let line = lines.last_mut().unwrap();
            let a = *line.last().unwrap();
            let n = (a.distance(p) / STEP).ceil().max(1.0) as usize;
            line.extend((1..=n).map(|k| a.lerp(p, k as f64 / n as f64)));
        }
        kurbo::PathEl::ClosePath => {
            let line = lines.last_mut().unwrap();
            let (a, p) = (*line.last().unwrap(), line[0]);
            let n = (a.distance(p) / STEP).ceil().max(1.0) as usize;
            line.extend((1..n).map(|k| a.lerp(p, k as f64 / n as f64)));
        }
        _ => unreachable!("flatten gives lines"),
    });
    lines.retain(|l| l.len() > 2);
    let original = lines.clone();
    let window = (ROUND / STEP * 1.5) as usize;
    for line in &mut lines {
        let n = line.len();
        // Per start point, the longest notch that starts there: (from, points round).
        let mut notches: Vec<(usize, usize)> = Vec::new();
        for i in 0..n {
            let mut round = 0.0;
            let mut longest = None;
            for k in 1..window.min(n - 1) {
                round += line[(i + k - 1) % n].distance(line[(i + k) % n]);
                if round > ROUND {
                    break;
                }
                let (p, q) = (line[i], line[(i + k) % n]);
                let chord = p.distance(q);
                if k < 2 || chord >= MOUTH || round < chord + 0.1 {
                    continue;
                }
                // A slit at least as deep as it is wide: not the round end
                // of a narrow counter, nor the wedge where two curves meet.
                let depth = (1..k)
                    .map(|m| {
                        let r = line[(i + m) % n];
                        let ab = q - p;
                        let t = ((r - p).dot(ab) / ab.hypot2()).clamp(0.0, 1.0);
                        r.distance(p + ab * t)
                    })
                    .fold(0.0, f64::max);
                if depth < chord.max(0.5) {
                    continue;
                }
                if [0.25, 0.5, 0.75]
                    .iter()
                    .any(|&t| inside(&original, p.lerp(q, t)))
                {
                    continue;
                }
                longest = Some(k);
            }
            if let Some(k) = longest {
                notches.push((i, k));
            }
        }
        // The longest notches first; a notch inside a closed one is gone.
        notches.sort_by_key(|n| std::cmp::Reverse(n.1));
        let mut skip = vec![false; n];
        let mut close = vec![None; n];
        for (i, k) in notches {
            if (0..=k).any(|m| skip[(i + m) % n] || close[(i + m) % n].is_some()) {
                continue;
            }
            for m in 1..k {
                skip[(i + m) % n] = true;
            }
            close[i] = Some((i + k) % n);
        }
        let mut next = Vec::with_capacity(n);
        for i in 0..n {
            if skip[i] {
                continue;
            }
            next.push(line[i]);
            if let Some(j) = close[i] {
                let (p, q) = (line[i], line[j]);
                let steps = (p.distance(q) / STEP).ceil().max(1.0) as usize;
                next.extend((1..steps).map(|m| p.lerp(q, m as f64 / steps as f64)));
            }
        }
        *line = next;
    }
    lines.iter().map(|l| simplify(l, 0.002)).collect()
}

/// A closed polyline with the points dropped that lie within `tol` of the
/// line between their neighbours that stay (Ramer-Douglas-Peucker).
fn simplify(line: &[kurbo::Point], tol: f64) -> Vec<kurbo::Point> {
    fn rdp(pts: &[kurbo::Point], tol: f64, out: &mut Vec<kurbo::Point>) {
        let (a, b) = (pts[0], pts[pts.len() - 1]);
        let (far, dist) = pts[1..pts.len() - 1]
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ab = b - a;
                let t = if a == b {
                    0.0
                } else {
                    ((*p - a).dot(ab) / ab.hypot2()).clamp(0.0, 1.0)
                };
                let d = p.distance(a + ab * t);
                (i + 1, d)
            })
            .fold((0, -1.0), |m, x| if x.1 > m.1 { x } else { m });
        if dist > tol {
            rdp(&pts[..=far], tol, out);
            out.pop();
            rdp(&pts[far..], tol, out);
        } else {
            out.extend([a, b]);
        }
    }
    // Split at the point farthest from the first, so both halves are open.
    let far = (0..line.len())
        .max_by(|&x, &y| {
            line[0]
                .distance(line[x])
                .total_cmp(&line[0].distance(line[y]))
        })
        .unwrap();
    let mut out = Vec::new();
    rdp(&line[..=far], tol, &mut out);
    out.pop();
    let mut back: Vec<kurbo::Point> = line[far..].to_vec();
    back.push(line[0]);
    rdp(&back, tol, &mut out);
    out.pop();
    out
}

/// Block letters without closed counters, so each falls out as one piece.
fn letter(c: char, x: f64, y: f64) -> String {
    let p = |pts: &[(f64, f64)]| poly(&pts.iter().map(|(a, b)| (x + a, y + b)).collect::<Vec<_>>());
    match c {
        'C' => p(&[
            (0.0, 0.0),
            (50.0, 0.0),
            (50.0, 14.0),
            (14.0, 14.0),
            (14.0, 50.0),
            (50.0, 50.0),
            (50.0, 64.0),
            (0.0, 64.0),
        ]),
        'U' => p(&[
            (0.0, 0.0),
            (14.0, 0.0),
            (14.0, 50.0),
            (36.0, 50.0),
            (36.0, 0.0),
            (50.0, 0.0),
            (50.0, 64.0),
            (0.0, 64.0),
        ]),
        'T' => p(&[
            (0.0, 0.0),
            (50.0, 0.0),
            (50.0, 14.0),
            (32.0, 14.0),
            (32.0, 64.0),
            (18.0, 64.0),
            (18.0, 14.0),
            (0.0, 14.0),
        ]),
        _ => unreachable!("only C, U and T"),
    }
}

// ---------------------------------------------------------------- designs

type Session = flatcraft_mcp::SharedSession;

fn import(s: &Session, name: &str, body: &str) -> u64 {
    let r = s
        .lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": svg(body), "name": name }),
            Origin::User,
        )
        .unwrap();
    r["id"].as_u64().unwrap()
}

fn run(s: &Session, command: &str, params: serde_json::Value) -> serde_json::Value {
    s.lock()
        .unwrap()
        .execute(command, params, Origin::User)
        .unwrap_or_else(|e| panic!("{command}: {e}"))
}

/// A pantry label with scooped corners, `w` x `h` mm, its top left corner
/// at (x, y), moved `d` mm inwards.
fn label(x: f64, y: f64, w: f64, h: f64, d: f64) -> Vec<(f64, f64)> {
    let r = 7.0 + d;
    let (x0, y0, x1, y1) = (x + d, y + d, x + w - d, y + h - d);
    let (cx0, cy0, cx1, cy1) = (x, y, x + w, y + h);
    let q = PI / 2.0;
    [
        arc(cx1, cy0, r, 2.0 * q, q),
        arc(cx1, cy1, r, -q, -2.0 * q),
        arc(cx0, cy1, r, 0.0, -q),
        arc(cx0, cy0, r, q, 0.0),
    ]
    .concat()
    .into_iter()
    .map(|(px, py)| (px.clamp(x0, x1), py.clamp(y0, y1)))
    .collect()
}

/// 12 pantry labels: one, then 11 linked copies in 3 columns. Each has a
/// rim, a groove and a blank panel to write on.
fn pantry_labels(s: &Session) {
    let (x, y, w, h) = (15.0, 25.0, 84.0, 52.0);
    let body: String = [0.0, 2.5, 4.0]
        .into_iter()
        .map(|d| poly(&label(x, y, w, h, d)))
        .collect();
    let id = import(s, "Pantry label", &body);
    run(
        s,
        "object.copies",
        json!({ "id": id, "count": 11, "layout": { "grid": { "columns": 3 } }, "gap_x_mm": 9.0, "gap_y_mm": 14.0, "linked": true }),
    );
    run(s, "job.set_material", json!({ "id": "vinyl" }));
    s.lock().unwrap().doc_mut().document.job.strategy =
        flatcraft_core::document::CutStrategy::OutlinesLast;
}

/// The name of the shirt motif, which says why it is mirrored.
/// The reason comes first, so the object list shows it even when the name
/// is cut short.
const SHIRT_MOTIF: &str = "Mirrored for iron-on: shirt motif";

/// A striped 80s sunset over "BORN TO MAKE", mirrored for heat transfer
/// vinyl, which is cut from the back.
fn iron_on_shirt(s: &Session) {
    let (cx, horizon, r) = (150.0, 146.0, 84.0);
    let half = |y: f64| (r * r - (y - horizon).powi(2)).max(0.0).sqrt();
    // Bands of the sun, the gaps between them growing towards the horizon.
    let mut body = String::new();
    let mut top = horizon - r;
    for (gap_at, gap) in [(97.0, 2.5), (114.0, 3.5), (129.0, 4.5), (horizon, 0.0)] {
        let steps = ((gap_at - top) / 0.5).ceil() as usize;
        let ys: Vec<f64> = (0..=steps)
            .map(|i| top + (gap_at - top) * i as f64 / steps as f64)
            .collect();
        let mut pts: Vec<(f64, f64)> = ys.iter().map(|&y| (cx + half(y), y)).collect();
        pts.extend(ys.iter().rev().map(|&y| (cx - half(y), y)));
        pts.dedup_by(|a, b| (a.0 - b.0).hypot(a.1 - b.1) < 1e-6);
        if (pts[0].0 - pts[pts.len() - 1].0).abs() < 1e-6 {
            pts.pop();
        }
        body.push_str(&poly(&pts));
        top = gap_at + gap;
    }
    body.push_str(&words(cx, 172.0, 24.0, 0.0, 4.0, &["BORN TO"]));
    body.push_str(&words(cx, 212.0, 62.0, 0.0, 3.0, &["MAKE"]));
    let id = import(s, SHIRT_MOTIF, &outlined(&body));
    run(
        s,
        "object.mirror",
        json!({ "ids": [id], "axis": "horizontal" }),
    );
    run(
        s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
}

/// The extra sticker of the maker quote sheet, a motto with no author.
const MOTTO: &str = "Eat. Sleep. Make. Repeat.";

/// A wavy oval seal centred on (cx, cy), moved `d` mm inwards.
fn seal(cx: f64, cy: f64, rx: f64, ry: f64, d: f64) -> Vec<(f64, f64)> {
    (0..720)
        .map(|i| {
            let a = TAU * i as f64 / 720.0;
            let wave = 1.2 * (20.0 * a).cos() - d;
            (cx + (rx + wave) * a.cos(), cy + (ry + wave) * a.sin())
        })
        .collect()
}

/// The two lines of a sticker frame: the outline (`shape(0)`) and a line
/// 3 mm inside it. The 3 mm band is kept, and the background inside it
/// weeds as one piece around the letters.
fn frame(shape: &dyn Fn(f64) -> Vec<(f64, f64)>) -> String {
    [poly(&shape(0.0)), poly(&shape(3.0))].concat()
}

/// The maker quotes of the About dialog as vinyl stickers: each quote in
/// solid capital letters inside a framed shape, its author's name a
/// separate object below it, and the motto in a pill at the bottom.
/// Capitals, because the joins of Plex lower case letters stay under 1 mm
/// at any size that fits a sticker.
fn maker_quotes(s: &Session) {
    use flatcraft::ui::about::QUOTES;
    let quote = |author: &str| {
        QUOTES
            .iter()
            .find(|(_, a)| *a == author)
            .unwrap_or_else(|| panic!("no quote by {author}"))
            .0
    };
    let (left, right) = (78.0, 222.0);
    let w = 132.0;
    // Row centres: a banner and a circle, a hex badge and a seal, two
    // rectangles, then the motto alone.
    let rows = [48.6, 130.85, 200.1, 264.35];
    // The author lines sit this far under their row centre.
    let names = [48.6 + 43.9, 130.85 + 35.9, 200.1 + 30.9];
    let hex = |cx: f64, cy: f64, h: f64, d: f64| {
        let (x0, x1, y0, y1) = (cx - w / 2.0, cx + w / 2.0, cy - h / 2.0, cy + h / 2.0);
        let corners = [
            (x0, cy),
            (x0 + 12.0, y0),
            (x1 - 12.0, y0),
            (x1, cy),
            (x1 - 12.0, y1),
            (x0 + 12.0, y1),
        ];
        round_corners(&inset(&corners, d), 7.0 - d)
    };
    let banner = |cx: f64, cy: f64, d: f64| {
        let (x0, x1, y0, y1) = (cx - w / 2.0, cx + w / 2.0, cy - 28.0, cy + 28.0);
        inset(
            &[
                (x0, y0),
                (x1, y0),
                (x1 - 12.0, cy),
                (x1, y1),
                (x0, y1),
                (x0 + 12.0, cy),
            ],
            d,
        )
    };
    let rect = |cx: f64, cy: f64, h: f64, r: f64, d: f64| {
        rounded_rect(
            cx - w / 2.0 + d,
            cy - h / 2.0 + d,
            w - 2.0 * d,
            h - 2.0 * d,
            r - d,
        )
    };
    // (centre, shape, quote lines, letter size, author line y, author),
    // in the order of QUOTES.
    type Sticker<'a> = ((f64, f64), String, &'a [&'a str], f64, f64, &'a str);
    let stickers: [Sticker; 6] = [
        (
            (right, rows[1]),
            frame(&|d| seal(right, rows[1], w / 2.0, 27.0, d)),
            &["Every defect", "gets respect."],
            12.5,
            names[1],
            "Laura Kampf",
        ),
        (
            (left, rows[0]),
            frame(&|d| banner(left, rows[0], d)),
            &["Failure is", "always an", "option."],
            12.5,
            names[0],
            "Adam Savage",
        ),
        (
            (left, rows[2]),
            frame(&|d| rect(left, rows[2], 44.0, 10.0, d)),
            &["Super massive", "stability stuff."],
            12.5,
            names[2],
            "Felix Schelhasse",
        ),
        (
            (right, rows[0]),
            frame(&|d| arc(right, rows[0], 35.0 - d, 0.0, TAU)[1..].to_vec()),
            &["Words", "are", "hard."],
            12.0,
            names[0],
            "Evan & Katelyn",
        ),
        (
            (left, rows[1]),
            frame(&|d| hex(left, rows[1], 56.0, d)),
            &["Don't glue", "yourself to", "the table."],
            12.5,
            names[1],
            "Joel Telling",
        ),
        (
            (right, rows[2]),
            frame(&|d| rect(right, rows[2], 44.0, 4.0, d)),
            &["Always Be", "Knolling."],
            12.5,
            names[2],
            "Tom Sachs",
        ),
    ];
    let caps = |lines: &[&str]| lines.iter().map(|l| l.to_uppercase()).collect::<Vec<_>>();
    for ((cx, cy), shape, lines, size, name_y, author) in stickers {
        let full = lines.join(" ");
        assert_eq!(full, quote(author), "word for word");
        let upper = caps(lines);
        let upper: Vec<&str> = upper.iter().map(String::as_str).collect();
        let body = format!("{shape}{}", words(cx, cy, size, 14.0, 0.6, &upper));
        import(s, &full, &outlined(&body));
        let name = author.to_uppercase();
        import(
            s,
            author,
            &outlined(&words(cx, name_y, 12.5, 0.0, 0.6, &[&name])),
        );
    }
    let pill = |d: f64| rect(150.0, rows[3], 44.0, 22.0, d);
    let motto: Vec<&str> = vec!["Eat. Sleep.", "Make. Repeat."];
    assert_eq!(motto.join(" "), MOTTO);
    let upper = caps(&motto);
    let upper: Vec<&str> = upper.iter().map(String::as_str).collect();
    let body = format!(
        "{}{}",
        frame(&pill),
        words(150.0, rows[3], 12.5, 14.0, 0.6, &upper)
    );
    import(s, MOTTO, &outlined(&body));
    run(s, "job.set_material", json!({ "id": "vinyl" }));
}

/// "CUT" cut out of the sheet on the mat, keep mode Stencil: the sheet is
/// the stencil and the letters fall out. No frame: with Stencil the whole
/// sheet is kept, and a frame ring would turn the stencil itself into a
/// shape that falls out.
fn stencil(s: &Session) {
    let mut body = String::new();
    for (i, c) in ['C', 'U', 'T'].into_iter().enumerate() {
        body.push_str(&letter(c, 55.0 + i as f64 * 70.0, 120.0));
    }
    let id = import(s, "Stencil", &body);
    run(
        s,
        "object.set_keep",
        json!({ "ids": [id], "keep": "stencil" }),
    );
}

/// A tag with clipped top corners and a hole, then 5 linked copies
/// around the middle of the mat.
fn gift_tags(s: &Session) {
    let (x0, x1, top, bottom) = (132.0, 168.0, 30.0, 92.0);
    let tag = poly(&[
        (x0 + 11.0, top),
        (x1 - 11.0, top),
        (x1, top + 11.0),
        (x1, bottom),
        (x0, bottom),
        (x0, top + 11.0),
    ]);
    let hole = circle(150.0, top + 10.0, 3.5);
    let id = import(s, "Gift tag", &format!("{tag}{hole}"));
    run(
        s,
        "object.circular_copies",
        json!({ "ids": [id], "count": 6, "center_mm": [150.0, 150.0], "linked": true }),
    );
}

/// An eight-point star with three rings of inner shapes.
fn window_star(s: &Session) {
    let (cx, cy) = (150.0, 150.0);
    let outer: Vec<(f64, f64)> = (0..16)
        .map(|i| {
            let a = PI / 8.0 * i as f64;
            polar(cx, cy, if i % 2 == 0 { 125.0 } else { 85.0 }, a)
        })
        .collect();
    let mut body = poly(&outer);
    for k in 0..8 {
        let a = PI / 4.0 * k as f64;
        body.push_str(&diamond(cx, cy, a, 52.0, 106.0, 16.0));
        body.push_str(&diamond(cx, cy, a + PI / 8.0, 48.0, 74.0, 11.0));
        let (x, y) = polar(cx, cy, 33.0, a + PI / 8.0);
        body.push_str(&circle(x, y, 5.0));
    }
    body.push_str(&circle(cx, cy, 14.0));
    import(s, "Window star", &body);
}

fn build(name: &str) -> (tempfile::TempDir, Session) {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    match name {
        "pantry-labels" => pantry_labels(&s),
        "iron-on-shirt" => iron_on_shirt(&s),
        "maker-quotes" => maker_quotes(&s),
        "stencil" => stencil(&s),
        "gift-tags" => gift_tags(&s),
        "window-star" => window_star(&s),
        other => panic!("no sample {other}"),
    }
    (dir, s)
}

#[test]
#[ignore = "writes assets/samples; run it after changing a design"]
fn regenerate_samples() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples");
    std::fs::create_dir_all(&out).unwrap();
    for sample in &SAMPLES {
        let (_dir, s) = build(sample.id);
        let doc = s.lock().unwrap().doc().document.clone();
        let profiles = s.lock().unwrap().profiles.clone();
        let thumbnail = flatcraft_engine::commands::project::thumbnail_png(&profiles, &doc);
        assert!(thumbnail.is_some(), "{} has a picture", sample.id);
        let bytes = flatcraft_core::project::to_archive(&doc, thumbnail.as_deref())
            .unwrap()
            .to_bytes()
            .unwrap();
        std::fs::write(out.join(format!("{}.flatcraft", sample.id)), bytes).unwrap();
    }
}

// ---------------------------------------------------------------- checks

/// `a` and `b` agree, numbers within 1e-6 (a JSON round trip may change
/// the last digit). Returns the path of the first difference.
fn differs(a: &serde_json::Value, b: &serde_json::Value, at: &str) -> Option<String> {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            ((x - y).abs() > 1e-6).then(|| format!("{at}: {x} != {y}"))
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .enumerate()
            .find_map(|(i, (p, q))| differs(p, q, &format!("{at}[{i}]"))),
        (Value::Object(x), Value::Object(y)) if x.len() == y.len() => {
            x.iter().find_map(|(k, p)| match y.get(k) {
                Some(q) => differs(p, q, &format!("{at}.{k}")),
                None => Some(format!("{at}.{k} is missing")),
            })
        }
        _ => (a != b).then(|| format!("{at} differs")),
    }
}

/// The project.json of a bundled sample.
fn project_json(sample: &flatcraft::ui::home::Sample) -> serde_json::Value {
    flatcraft_core::project::Archive::from_bytes(sample.bytes)
        .unwrap()
        .project
}

#[test]
fn the_bundled_samples_are_the_designs_of_this_file() {
    for sample in &SAMPLES {
        let (_dir, s) = build(sample.id);
        let doc = s.lock().unwrap().doc().document.clone();
        let value = flatcraft_core::project::to_value(&doc).unwrap();
        let bundled = project_json(sample);
        if let Some(d) = differs(&value, &bundled, sample.id) {
            panic!("{d}: run regenerate_samples");
        }
    }
}

#[test]
fn every_sample_loads_plans_without_warnings_and_fits_the_default_mat() {
    for sample in &SAMPLES {
        let text = project_json(sample).to_string();
        assert!(!text.contains("/Users"), "{}", sample.id);
        assert!(!text.contains("\"link\""), "{} has a file link", sample.id);
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let mut st = s.lock().unwrap();
        let r =
            flatcraft_engine::commands::project::open_bytes(&mut st, sample.bytes, sample.title)
                .unwrap();
        assert_eq!(r["warnings"], json!([]), "{}", sample.id);
        assert!(st.doc().path.is_none());
        let state = st
            .execute("document.state", json!({}), Origin::User)
            .unwrap();
        let area = &state["cutting_area_mm"];
        let (ax0, ay0, ax1, ay1) = (
            area["x0"].as_f64().unwrap(),
            area["y0"].as_f64().unwrap(),
            area["x1"].as_f64().unwrap(),
            area["y1"].as_f64().unwrap(),
        );
        for o in state["objects"].as_array().unwrap() {
            let b = &o["bounds_mm"];
            assert!(
                b["x0"].as_f64().unwrap() >= ax0
                    && b["y0"].as_f64().unwrap() >= ay0
                    && b["x1"].as_f64().unwrap() <= ax1
                    && b["y1"].as_f64().unwrap() <= ay1,
                "{}: {} at {b} is outside {area}",
                sample.id,
                o["name"]
            );
        }
        let job = flatcraft_engine::job::build(&st).unwrap();
        assert!(job.warnings.is_empty(), "{}: {:?}", sample.id, job.warnings);
        assert!(!job.plan.toolpaths.is_empty());
    }
}

#[test]
fn the_window_star_cuts_its_inner_shapes_first() {
    let sample = SAMPLES.iter().find(|x| x.id == "window-star").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut st = s.lock().unwrap();
    flatcraft_engine::commands::project::open_bytes(&mut st, sample.bytes, "star").unwrap();
    let job = flatcraft_engine::job::build(&st).unwrap();
    let paths = &job.plan.toolpaths;
    assert_eq!(paths.len(), 8 * 3 + 2, "25 inner shapes and the star");
    // The star is the one path that reaches 125 mm from the middle.
    let reach = |i: usize| {
        paths[i]
            .points
            .iter()
            .map(|p| ((p.x - 150.0).powi(2) + (p.y - 150.0).powi(2)).sqrt())
            .fold(0.0, f64::max)
    };
    let star = (0..paths.len())
        .max_by(|&a, &b| reach(a).total_cmp(&reach(b)))
        .unwrap();
    assert_eq!(star, paths.len() - 1, "the outline is cut last");
}

#[test]
fn the_patterns_are_linked_copies() {
    for (id, n) in [("pantry-labels", 12), ("gift-tags", 6)] {
        let sample = SAMPLES.iter().find(|x| x.id == id).unwrap();
        let (doc, _) = flatcraft_core::project::load_report_bytes(sample.bytes).unwrap();
        assert_eq!(doc.objects.len(), n, "{id}");
        let group = doc.objects[0].link_group;
        assert!(group.is_some(), "{id}");
        assert!(doc.objects.iter().all(|o| o.link_group == group), "{id}");
    }
}

/// A bundled sample, opened as the app opens it.
fn open(id: &str) -> (tempfile::TempDir, Session) {
    let sample = SAMPLES.iter().find(|x| x.id == id).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    flatcraft_engine::commands::project::open_bytes(&mut s.lock().unwrap(), sample.bytes, id)
        .unwrap();
    (dir, s)
}

/// The cut lines of the objects that are cut, on the mat, as points about
/// every 0.1 mm: (contour, distance along it, point). Per contour: its
/// length and the name of its object.
struct CutLines {
    points: Vec<(usize, f64, kurbo::Point)>,
    contours: Vec<(f64, String)>,
}

fn cut_lines(doc: &flatcraft_core::document::Document) -> CutLines {
    let mut points = Vec::new();
    let mut contours = Vec::new();
    for o in doc.objects.iter().filter(|o| o.cut_enabled) {
        let affine = o.transform.affine(o.local_bounds);
        for cut in o.geometry.iter() {
            let mut lines: Vec<Vec<kurbo::Point>> = Vec::new();
            kurbo::flatten(affine * cut.path.clone(), 0.005, |el| match el {
                kurbo::PathEl::MoveTo(p) => lines.push(vec![p]),
                kurbo::PathEl::LineTo(p) => lines.last_mut().unwrap().push(p),
                kurbo::PathEl::ClosePath => {
                    let line = lines.last_mut().unwrap();
                    let first = line[0];
                    line.push(first);
                }
                _ => unreachable!("flatten gives lines"),
            });
            for line in lines {
                let contour = contours.len();
                let mut along = 0.0;
                let mut next = 0.0;
                for w in line.windows(2) {
                    let len = w[0].distance(w[1]);
                    while next <= along + len {
                        let t = if len > 0.0 { (next - along) / len } else { 0.0 };
                        points.push((contour, next, w[0].lerp(w[1], t)));
                        next += 0.1;
                    }
                    along += len;
                }
                contours.push((along, o.name.clone()));
            }
        }
    }
    CutLines { points, contours }
}

/// The places where cut lines come closer than 1 mm: two different lines,
/// or one line that comes back to itself after more than 2 mm along it
/// (a stroke or a gap under 1 mm wide, from about 0.6 mm long). A pointed
/// tip or notch is fine: there the line between the two places and the
/// way round enclose about a triangle, half the box of the gap times the
/// depth, where a stroke or a neck fills all of it or more. Returns, per
/// object name, the narrowest place in mm and where it is.
fn narrow_places(lines: &CutLines) -> std::collections::BTreeMap<String, (f64, kurbo::Point)> {
    // Where each contour's points start in `lines.points`.
    let mut starts = vec![0; lines.contours.len() + 1];
    for (i, (c, _, _)) in lines.points.iter().enumerate().rev() {
        starts[*c] = i;
    }
    starts[lines.contours.len()] = lines.points.len();
    // True when the shorter way round from point i to point j of one
    // contour is a pointed tip or notch.
    let tip = |i: usize, j: usize, c: usize| {
        let (lo, hi) = (i.min(j), i.max(j));
        let (start, end) = (starts[c], starts[c + 1]);
        let way: Vec<kurbo::Point> = if hi - lo <= (end - start) / 2 {
            (lo..=hi).map(|k| lines.points[k].2).collect()
        } else {
            (hi..end)
                .chain(start..=lo)
                .map(|k| lines.points[k].2)
                .collect()
        };
        let (p, q) = (way[0], way[way.len() - 1]);
        let pq = q - p;
        let depth = way
            .iter()
            .map(|r| {
                let t = ((*r - p).dot(pq) / pq.hypot2()).clamp(0.0, 1.0);
                r.distance(p + pq * t)
            })
            .fold(0.0, f64::max);
        let area = way
            .iter()
            .zip(way.iter().cycle().skip(1))
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum::<f64>()
            .abs()
            / 2.0;
        area < 0.85 * p.distance(q) * depth
    };
    let cell = |p: kurbo::Point| (p.x.floor() as i64, p.y.floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
    for (i, (_, _, p)) in lines.points.iter().enumerate() {
        grid.entry(cell(*p)).or_default().push(i);
    }
    let mut narrow = std::collections::BTreeMap::<String, (f64, kurbo::Point)>::new();
    for (i, &(c, at, p)) in lines.points.iter().enumerate() {
        let (gx, gy) = cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &j in grid.get(&(gx + dx, gy + dy)).into_iter().flatten() {
                    let (d, bt, q) = lines.points[j];
                    let gap = p.distance(q);
                    if j <= i || gap >= 1.0 {
                        continue;
                    }
                    let length = lines.contours[c].0;
                    let apart = (at - bt).abs().min(length - (at - bt).abs());
                    if d != c || (apart > 2.0 && !tip(i, j, c)) {
                        let name = &lines.contours[c].1;
                        let e = narrow.entry(name.clone()).or_insert((gap, p));
                        if gap < e.0 {
                            *e = (gap, p);
                        }
                    }
                }
            }
        }
    }
    narrow
}

/// Every cut stroke and every gap is at least 1 mm wide, so the pieces
/// weed on adhesive vinyl and iron-on.
#[test]
fn every_cut_stroke_and_gap_is_at_least_1_mm() {
    for sample in &SAMPLES {
        let (_dir, s) = open(sample.id);
        let narrow = narrow_places(&cut_lines(&s.lock().unwrap().doc().document));
        assert!(narrow.is_empty(), "{}: {narrow:#?}", sample.id);
    }
}

#[test]
fn the_narrow_place_check_finds_a_thin_stroke_and_a_tight_gap() {
    // The narrowest place of `body`, or None when all is 1 mm or more.
    let narrowest = |body: &str| {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        import(&s, "Shapes", body);
        let doc = s.lock().unwrap().doc().document.clone();
        narrow_places(&cut_lines(&doc)).get("Shapes").map(|n| n.0)
    };
    let wide_bar = r#"<rect x="80" y="20" width="1.2" height="30"/>"#;
    let squares = r#"<rect x="40" y="20" width="10" height="10"/><rect x="50.9" y="20" width="10" height="10"/>"#;
    let thin_bar = r#"<rect x="20" y="20" width="0.8" height="30"/>"#;
    assert_eq!(narrowest(wide_bar), None);
    let gap = narrowest(&format!("{wide_bar}{squares}")).unwrap();
    assert!((gap - 0.9).abs() < 0.05, "{gap}");
    let stroke = narrowest(&format!("{wide_bar}{thin_bar}")).unwrap();
    assert!((stroke - 0.8).abs() < 0.05, "{stroke}");
    // Inside one line: a short 0.8 x 1.5 mm spur on a square, and two
    // squares joined by a 0.8 x 3 mm neck.
    let spur = poly(&[
        (20.0, 30.0),
        (20.0, 20.0),
        (24.6, 20.0),
        (24.6, 18.5),
        (25.4, 18.5),
        (25.4, 20.0),
        (30.0, 20.0),
        (30.0, 30.0),
    ]);
    let spur = narrowest(&spur).unwrap();
    assert!((spur - 0.8).abs() < 0.05, "{spur}");
    let neck = poly(&[
        (40.0, 20.0),
        (50.0, 20.0),
        (50.0, 24.6),
        (53.0, 24.6),
        (53.0, 20.0),
        (63.0, 20.0),
        (63.0, 30.0),
        (53.0, 30.0),
        (53.0, 25.4),
        (50.0, 25.4),
        (50.0, 30.0),
        (40.0, 30.0),
    ]);
    let neck = narrowest(&neck).unwrap();
    assert!((neck - 0.8).abs() < 0.05, "{neck}");
    // A sharp point of 30 degrees and a notch of 30 degrees are fine.
    let point = poly(&[(80.0, 40.0), (74.64, 20.0), (85.36, 20.0)]);
    assert_eq!(narrowest(&point), None);
    let notch = poly(&[
        (100.0, 20.0),
        (104.64, 20.0),
        (110.0, 40.0),
        (115.36, 20.0),
        (120.0, 20.0),
        (120.0, 50.0),
        (100.0, 50.0),
    ]);
    assert_eq!(narrowest(&notch), None);
}

#[test]
fn the_labels_cut_every_inner_line_before_the_outlines_on_vinyl() {
    let (_dir, s) = open("pantry-labels");
    let st = s.lock().unwrap();
    let job = &st.doc().document.job;
    assert_eq!(
        job.strategy,
        flatcraft_core::document::CutStrategy::OutlinesLast
    );
    assert_eq!(job.material_id, "vinyl");
    // A rim, a groove and a panel: three lines per label.
    let plan = flatcraft_engine::job::build(&st).unwrap().plan;
    assert_eq!(plan.toolpaths.len(), 12 * 3);
    // The 12 outlines, the largest lines, are cut last.
    let area = |t: &flatcraft_path::Toolpath| {
        t.points
            .iter()
            .fold(
                kurbo::Rect::from_points(t.points[0], t.points[0]),
                |r, p| r.union_pt(*p),
            )
            .area()
    };
    let mut sizes: Vec<f64> = plan.toolpaths.iter().map(area).collect();
    sizes.sort_by(f64::total_cmp);
    let smallest_outline = sizes[sizes.len() - 12];
    for (i, t) in plan.toolpaths.iter().enumerate() {
        assert_eq!(area(t) >= smallest_outline, i >= 24, "toolpath {i}");
    }
}

#[test]
fn the_shirt_motif_is_mirrored_for_heat_transfer_vinyl() {
    let (_dir, s) = open("iron-on-shirt");
    let st = s.lock().unwrap();
    let doc = &st.doc().document;
    assert_eq!(doc.job.material_id, "heat-transfer-vinyl");
    assert_eq!(doc.objects.len(), 1);
    let motif = &doc.objects[0];
    assert!(motif.transform.mirror, "stored mirrored");
    assert_eq!(motif.name, SHIRT_MOTIF);
    assert!(motif.name.starts_with("Mirrored for iron-on"));
}

#[test]
fn the_maker_quotes_are_word_for_word_with_their_authors_apart() {
    use flatcraft::ui::about::QUOTES;
    let (_dir, s) = open("maker-quotes");
    let st = s.lock().unwrap();
    let doc = &st.doc().document;
    assert_eq!(doc.job.material_id, "vinyl");
    let names: Vec<&str> = doc.objects.iter().map(|o| o.name.as_str()).collect();
    // The short quotes; the two long ones would not weed as vinyl.
    let short: Vec<(&str, &str)> = QUOTES
        .iter()
        .copied()
        .filter(|(q, _)| q.len() < 40)
        .collect();
    assert_eq!(short.len(), 6);
    let mut want = Vec::new();
    for (quote, author) in short {
        want.extend([quote, author]);
    }
    want.push(MOTTO);
    assert_eq!(names, want);
    // Every letter is its own cut line: none is merged into its shape.
    let contours: usize = doc
        .objects
        .iter()
        .flat_map(|o| o.geometry.iter())
        .map(|c| {
            c.path
                .elements()
                .iter()
                .filter(|e| matches!(e, kurbo::PathEl::MoveTo(_)))
                .count()
        })
        .sum();
    let plan = flatcraft_engine::job::build(&st).unwrap().plan;
    assert_eq!(plan.toolpaths.len(), contours);
    // Each sticker has a frame band (two lines) and its quote's letters.
    for o in doc.objects.iter().filter(|o| o.name.ends_with('.')) {
        let lines = o.geometry.iter().map(|c| {
            c.path
                .elements()
                .iter()
                .filter(|e| matches!(e, kurbo::PathEl::MoveTo(_)))
                .count()
        });
        let letters = o.name.chars().filter(|c| c.is_alphanumeric()).count();
        assert!(lines.sum::<usize>() > 2 + letters, "{}", o.name);
    }
}

#[test]
fn a_sample_opens_as_an_untitled_copy() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let before: Vec<Vec<u8>> = SAMPLES.iter().map(|x| x.bytes.to_vec()).collect();
    let picker = FakePicker::default();
    let requests = picker.requests.clone();
    // Tall enough for the second row of sample cards.
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1400.0], 4, |a| {
        a.with_file_picker(picker)
    });
    h.get_by_role_and_label(Role::Button, "Gift tags").click();
    wait_for_source_check(&mut h);
    {
        let st = s.lock().unwrap();
        assert_eq!(st.docs.len(), 1, "the empty start document is used");
        assert!(st.doc().path.is_none(), "untitled");
        assert_eq!(st.doc().document.objects.len(), 6);
    }
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    h.get_by_role_and_label(Role::Button, "Untitled 1");
    // Save asks where: the bundled file is never the target.
    let doc = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Save { doc_id: doc }]);
    assert_eq!(requests.lock().unwrap()[0].title, "Save As");
    // A second sample opens next to it.
    app_mut(&mut h).ui.workspace = Workspace::Home;
    h.run();
    h.get_by_role_and_label(Role::Button, "Stencil").click();
    wait_for_source_check(&mut h);
    assert_eq!(s.lock().unwrap().docs.len(), 2);
    let after: Vec<Vec<u8>> = SAMPLES.iter().map(|x| x.bytes.to_vec()).collect();
    assert_eq!(before, after);
    // Nothing was written next to the samples or in the project folder.
    let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples");
    for sample in &SAMPLES {
        let on_disk = std::fs::read(samples.join(format!("{}.flatcraft", sample.id))).unwrap();
        assert_eq!(on_disk, sample.bytes);
    }
}

#[test]
fn sample_pictures_come_from_the_sample_files() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_inline_pictures(false));
    wait_until(&mut h, |a| a.ui.home.busy());
    for sample in &SAMPLES {
        let id = flatcraft::ui::home::sample_id(sample);
        let (version, thumb) = app(&h).ui.home.thumbs.get(&id).unwrap();
        assert_eq!(
            *version,
            flatcraft_engine::thumbnails::bytes_version(sample.bytes)
        );
        let flatcraft::ui::home::Thumb::Ready(tex) = thumb else {
            panic!("{} has no picture: {thumb:?}", sample.id);
        };
        // The picture in the sample file: the mat's larger side is 512 px.
        let [w, h] = tex.size();
        assert_eq!(w.max(h), flatcraft_core::project::THUMBNAIL_PX as usize);
        assert!(
            flatcraft_core::project::thumbnail_from_bytes(sample.bytes)
                .unwrap()
                .is_some(),
            "{}",
            sample.id
        );
    }
    // Nothing is written to the config dir.
    assert!(!dir.path().join("thumbnails").exists());
}

#[test]
fn the_gift_tags_cut_each_hole_before_its_tag() {
    let sample = SAMPLES.iter().find(|x| x.id == "gift-tags").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut st = s.lock().unwrap();
    flatcraft_engine::commands::project::open_bytes(&mut st, sample.bytes, "tags").unwrap();
    let job = flatcraft_engine::job::build(&st).unwrap();
    let boxes: Vec<kurbo::Rect> = job
        .plan
        .toolpaths
        .iter()
        .map(|t| {
            t.points.iter().fold(
                kurbo::Rect::from_points(t.points[0], t.points[0]),
                |r, p| r.union_pt(*p),
            )
        })
        .collect();
    let holes: Vec<usize> = (0..boxes.len())
        .filter(|&i| boxes[i].width() < 10.0)
        .collect();
    assert_eq!(holes.len(), 6, "one hole per tag");
    assert_eq!(boxes.len(), 12);
    for h in holes {
        let tag = (0..boxes.len())
            .find(|&i| i != h && boxes[i].width() >= 10.0 && boxes[i].contains(boxes[h].center()))
            .unwrap_or_else(|| panic!("hole {h} is in no tag"));
        assert!(h < tag, "hole {h} is cut after its tag {tag}");
    }
}

/// Every line of lettering in the samples: after the ink-trap fill no
/// place under 1 mm is left and every counter is still there.
#[test]
fn the_ink_trap_fill_closes_the_slits_and_keeps_the_counters() {
    LETTERING.with(|l| l.borrow_mut().clear());
    for sample in &SAMPLES {
        drop(build(sample.id));
    }
    let lettering = LETTERING.with(|l| l.borrow().clone());
    // 2 blocks on the shirt; 6 quotes, 6 names and the motto.
    assert_eq!(lettering.len(), 2 + 13);
    // (narrow places, number of cut lines) of `body`.
    let check = |body: &str| {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        import(&s, "Letters", body);
        let doc = s.lock().unwrap().doc().document.clone();
        let lines = cut_lines(&doc);
        (
            narrow_places(&lines).get("Letters").copied(),
            lines.contours.len(),
        )
    };
    let mut slits = 0;
    for text in &lettering {
        let (before, contours) = check(&outlined_with(text, false));
        let (after, filled) = check(&outlined(text));
        slits += usize::from(before.is_some());
        assert_eq!(after, None, "{text}");
        assert_eq!(filled, contours, "every counter stays: {text}");
    }
    assert!(slits > 0, "Plex has slits under 1 mm the fill closes");
}

#[test]
fn the_stencil_keeps_the_sheet_and_its_letters_fall_out() {
    let (_dir, s) = open("stencil");
    let st = s.lock().unwrap();
    let doc = &st.doc().document;
    assert_eq!(doc.objects.len(), 1);
    assert_eq!(
        doc.objects[0].keep,
        flatcraft_core::document::KeepMode::Stencil
    );
    let job = flatcraft_engine::job::build(&st).unwrap();
    let kept = &job.plan.kept;
    // Inside a letter is waste: a point on the middle line of each letter
    // that the letter covers. Between and around the letters is the sheet.
    let rings: Vec<Vec<kurbo::Point>> = job
        .plan
        .toolpaths
        .iter()
        .filter(|t| t.closed && t.kind == flatcraft_path::ToolpathKind::Cut)
        .map(|t| t.lap().to_vec())
        .collect();
    let inside: Vec<kurbo::Point> = (0..3)
        .filter_map(|i| {
            let x0 = 55.0 + i as f64 * 70.0;
            (0..60 * 80)
                .map(|k| kurbo::Point::new(x0 + (k % 60) as f64, 120.0 + (k / 60) as f64))
                .find(|&p| inside(&rings, p))
        })
        .collect();
    assert_eq!(inside.len(), 3, "a point in each letter");
    for p in inside {
        assert!(!kept.is_kept(p), "{p:?} is in a letter: waste");
    }
    for p in [
        (40.0, 150.0),
        (150.0, 110.0),
        (150.0, 200.0),
        (280.0, 150.0),
    ] {
        let p = kurbo::Point::new(p.0, p.1);
        assert!(kept.is_kept(p), "{p:?} is the sheet: kept");
    }
}
