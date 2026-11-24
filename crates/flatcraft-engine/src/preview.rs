//! Preview images of the active document, drawn with tiny-skia.
//!
//! Canvas view: the media, the cutting area and the objects in their colours.
//! Path view: the media and cutting area, then the toolpaths in cut order
//! (each with its number), travel moves dashed and a dot on every start point.
//! Order numbers come from a tiny built-in 3x5 digit font, because the engine
//! has no text rendering library. Direction arrows are not drawn.

use kurbo::{BezPath, PathEl, Point, Rect};
use serde::Deserialize;
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform,
};

use crate::session::{CommandError, Session};

/// Smallest and largest image side, px.
pub const MIN_PX: u32 = 64;
pub const MAX_PX: u32 = 4096;

/// What a preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PreviewKind {
    /// The physical mat, cutting area and the objects in their colours.
    Canvas,
    /// The toolpaths in cut order with travel moves and start points.
    Path,
}

fn color(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

/// A kurbo path as a tiny-skia path. `None` if it has no usable geometry.
fn to_skia(path: &BezPath) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let f = |p: Point| (p.x as f32, p.y as f32);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                let (x, y) = f(p);
                pb.move_to(x, y);
            }
            PathEl::LineTo(p) => {
                let (x, y) = f(p);
                pb.line_to(x, y);
            }
            PathEl::QuadTo(a, b) => {
                let (ax, ay) = f(a);
                let (bx, by) = f(b);
                pb.quad_to(ax, ay, bx, by);
            }
            PathEl::CurveTo(a, b, c) => {
                let (ax, ay) = f(a);
                let (bx, by) = f(b);
                let (cx, cy) = f(c);
                pb.cubic_to(ax, ay, bx, by, cx, cy);
            }
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

fn polyline(points: &[Point], closed: bool) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = points.split_first()?;
    pb.move_to(first.x as f32, first.y as f32);
    for p in rest {
        pb.line_to(p.x as f32, p.y as f32);
    }
    if closed {
        pb.close();
    }
    pb.finish()
}

fn rect_path(r: Rect) -> Option<tiny_skia::Path> {
    tiny_skia::Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32)
        .map(PathBuilder::from_rect)
}

/// 3x5 digit shapes, one row per `u8` (bit 2 is the left column).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Draws `n` with its top-left corner at pixel (x, y); `cell` px per dot.
fn draw_number(pm: &mut Pixmap, n: usize, x: f32, y: f32, cell: f32, p: &Paint) {
    let text = n.to_string();
    for (k, ch) in text.chars().enumerate() {
        let digit = ch.to_digit(10).unwrap_or(0) as usize;
        let ox = x + k as f32 * 4.0 * cell;
        for (row, bits) in DIGITS[digit].iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0
                    && let Some(r) = tiny_skia::Rect::from_xywh(
                        ox + col as f32 * cell,
                        y + row as f32 * cell,
                        cell,
                        cell,
                    )
                {
                    pm.fill_rect(r, p, Transform::identity(), None);
                }
            }
        }
    }
}

fn dot(pm: &mut Pixmap, x: f32, y: f32, r: f32, p: &Paint) {
    if let Some(path) = PathBuilder::from_circle(x, y, r) {
        pm.fill_path(&path, p, FillRule::Winding, Transform::identity(), None);
    }
}

fn failed(what: &str) -> CommandError {
    CommandError::Failed(format!("cannot draw the preview: {what}"))
}

/// Renders the active document as a PNG whose larger side is `max_px`.
pub fn render_png(s: &Session, kind: PreviewKind, max_px: u32) -> Result<Vec<u8>, CommandError> {
    if !(MIN_PX..=MAX_PX).contains(&max_px) {
        return Err(CommandError::InvalidParams(format!(
            "max_px must be between {MIN_PX} and {MAX_PX}, not {max_px}"
        )));
    }
    let job = &s.doc().document.job;
    let plotter = s
        .profiles
        .plotter(&job.plotter_id)
        .map_err(crate::commands::profile_error)?;
    let media = plotter
        .media
        .iter()
        .find(|m| m.id == job.media_id)
        .ok_or_else(|| {
            CommandError::NotFound(format!(
                "media '{}' is not defined for plotter '{}'",
                job.media_id, job.plotter_id
            ))
        })?;
    let area = plotter
        .cutting_area(&job.media_id)
        .ok_or_else(|| failed("no cutting area"))?;

    // The larger side is exactly max_px.
    let scale = f64::from(max_px) / media.width_mm.max(media.height_mm);
    let side = |mm: f64| ((mm * scale).round() as u32).clamp(1, max_px);
    let (w, h) = if media.width_mm >= media.height_mm {
        (max_px, side(media.height_mm))
    } else {
        (side(media.width_mm), max_px)
    };
    let mut pm = Pixmap::new(w, h).ok_or_else(|| failed("the image is too large"))?;
    let sc = scale as f32;
    // App coordinates start at the grid's 0,0, which lies at the grid origin on the mat.
    let (ox, oy) = (media.grid.origin_x_mm as f32, media.grid.origin_y_mm as f32);
    let mat = Transform::from_scale(sc, sc).pre_translate(ox, oy);
    let px = |n: f32| f64::from(n / sc); // pixel length in mm

    pm.fill(color(0xdd, 0xdf, 0xe3));
    if let Some(p) = rect_path(area) {
        pm.fill_path(
            &p,
            &paint(color(255, 255, 255)),
            FillRule::Winding,
            mat,
            None,
        );
        let stroke = Stroke {
            width: px(1.0) as f32,
            ..Stroke::default()
        };
        pm.stroke_path(&p, &paint(color(0x9a, 0xa0, 0xaa)), &stroke, mat, None);
    }

    match kind {
        PreviewKind::Canvas => {
            for o in s.doc().document.objects.iter().filter(|o| o.visible) {
                for cp in o.placed_paths() {
                    let c = cp.color.unwrap_or([0x22, 0x26, 0x2e]);
                    if let Some(p) = to_skia(&cp.path) {
                        let stroke = Stroke {
                            width: px(1.5) as f32,
                            line_join: LineJoin::Round,
                            line_cap: LineCap::Round,
                            ..Stroke::default()
                        };
                        pm.stroke_path(&p, &paint(color(c[0], c[1], c[2])), &stroke, mat, None);
                    }
                }
            }
        }
        PreviewKind::Path => {
            let result = crate::job::build(s)?;
            let cut = paint(color(0x1f, 0x4f, 0xb4));
            let travel = paint(color(0x88, 0x8e, 0x99));
            let start = paint(color(0xd6, 0x28, 0x28));
            let label = paint(color(0x11, 0x11, 0x11));
            let cut_stroke = Stroke {
                width: px(1.5) as f32,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            let travel_stroke = Stroke {
                width: px(1.0) as f32,
                dash: StrokeDash::new(vec![px(5.0) as f32, px(4.0) as f32], 0.0),
                ..Stroke::default()
            };
            let cell = (max_px as f32 / 512.0).round().max(1.0);
            let mut at = Point::ZERO; // the plan's travel starts at the origin
            for (i, t) in result.plan.toolpaths.iter().enumerate() {
                let Some(first) = t.points.first() else {
                    continue;
                };
                if let Some(p) = polyline(&[at, *first], false) {
                    pm.stroke_path(&p, &travel, &travel_stroke, mat, None);
                }
                if let Some(p) = polyline(&t.points, t.closed) {
                    pm.stroke_path(&p, &cut, &cut_stroke, mat, None);
                }
                at = *t.points.last().unwrap_or(first);
                let (sx, sy) = ((first.x as f32 + ox) * sc, (first.y as f32 + oy) * sc);
                dot(&mut pm, sx, sy, 2.5 * cell, &start);
                draw_number(
                    &mut pm,
                    i + 1,
                    sx + 3.5 * cell,
                    sy - 2.5 * cell,
                    cell,
                    &label,
                );
            }
        }
    }
    pm.encode_png().map_err(|e| failed(&e.to_string()))
}
