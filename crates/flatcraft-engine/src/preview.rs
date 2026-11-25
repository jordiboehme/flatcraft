//! Preview images of the active document, drawn with tiny-skia.
//!
//! Canvas view: the media, the cutting area and the objects in their colours.
//! Path view: the media and cutting area, then the toolpaths in cut order
//! (each with its number), travel moves dashed and a dot on every start point;
//! the 0,0 where the first travel move starts is marked and labelled H (home).
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

/// The letter H (home), in the same 3x5 grid as the digits.
const HOME: [u8; 5] = [0b101, 0b101, 0b111, 0b101, 0b101];

/// Draws `n` with its top-left corner at pixel (x, y); `cell` px per dot.
fn draw_number(pm: &mut Pixmap, n: usize, x: f32, y: f32, cell: f32, p: &Paint) {
    draw_text(pm, &n.to_string(), x, y, cell, p);
}

/// Draws digits and `H` with the top-left corner at pixel (x, y).
fn draw_text(pm: &mut Pixmap, text: &str, x: f32, y: f32, cell: f32, p: &Paint) {
    for (k, ch) in text.chars().enumerate() {
        let glyph = match ch {
            'H' => &HOME,
            _ => &DIGITS[ch.to_digit(10).unwrap_or(0) as usize],
        };
        let ox = x + k as f32 * 4.0 * cell;
        for (row, bits) in glyph.iter().enumerate() {
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

/// The colour of the grid and the cutting area edge on a mat of colour
/// `mat`: the canvas's contrast colour.
fn line_colour(mat: [u8; 3]) -> [u8; 3] {
    flatcraft_core::colour::contrast_colour(mat)
}

/// The colour of the order numbers and the H at the origin: the mat's
/// contrast colour, so they read on dark mats too.
fn label_colour(mat: [u8; 3]) -> [u8; 3] {
    flatcraft_core::colour::contrast_colour(mat)
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
    let plotter = s
        .profiles
        .plotter(&s.doc().document.job.plotter_id)
        .map_err(crate::commands::profile_error)?;
    let job = match kind {
        PreviewKind::Canvas => None,
        PreviewKind::Path => Some(crate::job::build(s)?),
    };
    draw(&plotter, &s.doc().document, job.as_ref(), max_px)
}

/// Renders `doc` on its mat, the canvas view (objects in their colours),
/// as a PNG whose larger side is `max_px`. It needs no session, so a
/// window can draw a document it copied out of the session, or one it
/// loaded from a file, without holding the session lock.
pub fn render_document_png(
    plotter: &flatcraft_device::PlotterProfile,
    doc: &flatcraft_core::document::Document,
    max_px: u32,
) -> Result<Vec<u8>, CommandError> {
    if !(MIN_PX..=MAX_PX).contains(&max_px) {
        return Err(CommandError::InvalidParams(format!(
            "max_px must be between {MIN_PX} and {MAX_PX}, not {max_px}"
        )));
    }
    draw(plotter, doc, None, max_px)
}

/// The canvas view of `doc`, or the path view of `job` when given.
fn draw(
    plotter: &flatcraft_device::PlotterProfile,
    doc: &flatcraft_core::document::Document,
    job: Option<&crate::job::JobResult>,
    max_px: u32,
) -> Result<Vec<u8>, CommandError> {
    let (media, _) = crate::commands::effective_media(plotter, &doc.job)?;
    let area = plotter
        .cutting_area(&media.id)
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

    // The mat margin in a darker shade of the mat colour, the cutting area in
    // the mat colour itself, the printed grid in a contrast colour on top.
    let [mr, mg, mb] = flatcraft_core::colour::margin_colour(media.color);
    pm.fill(color(mr, mg, mb));
    let [r, g, b] = media.color;
    match rect_path(area) {
        Some(p) => pm.fill_path(&p, &paint(color(r, g, b)), FillRule::Winding, mat, None),
        None => pm.fill(color(r, g, b)),
    }
    let [lr, lg, lb] = line_colour(media.color);
    let mut grid_lines = PathBuilder::new();
    let (gw, gh) = (media.grid.width_mm as f32, media.grid.height_mm as f32);
    let mut at = 0.0f32;
    while at <= gw {
        grid_lines.move_to(at, 0.0);
        grid_lines.line_to(at, gh);
        at += 10.0;
    }
    at = 0.0;
    while at <= gh {
        grid_lines.move_to(0.0, at);
        grid_lines.line_to(gw, at);
        at += 10.0;
    }
    if let Some(p) = grid_lines.finish() {
        let stroke = Stroke {
            width: px(1.0) as f32,
            ..Stroke::default()
        };
        let faint = paint(Color::from_rgba8(lr, lg, lb, 60));
        pm.stroke_path(&p, &faint, &stroke, mat, None);
    }
    if let Some(p) = rect_path(area) {
        let stroke = Stroke {
            width: px(1.0) as f32,
            ..Stroke::default()
        };
        let edge = paint(Color::from_rgba8(lr, lg, lb, 160));
        pm.stroke_path(&p, &edge, &stroke, mat, None);
    }

    match job {
        None => {
            for o in doc.objects.iter().filter(|o| o.visible) {
                for cp in o.placed_paths() {
                    let c = cp.color.unwrap_or([0x22, 0x26, 0x2e]);
                    if let Some(p) = to_skia(&cp.path) {
                        // A skipped object is dimmed and dashed, as in the app.
                        let (alpha, dash) = if o.cut_enabled {
                            (255, None)
                        } else {
                            (
                                155,
                                StrokeDash::new(vec![px(5.0) as f32, px(4.0) as f32], 0.0),
                            )
                        };
                        let stroke = Stroke {
                            width: px(1.5) as f32,
                            line_join: LineJoin::Round,
                            line_cap: LineCap::Round,
                            dash,
                            ..Stroke::default()
                        };
                        let ink = paint(Color::from_rgba8(c[0], c[1], c[2], alpha));
                        pm.stroke_path(&p, &ink, &stroke, mat, None);
                    }
                }
            }
        }
        Some(result) => {
            let [cr, cg, cb] = flatcraft_core::colour::cut_colour(media.color);
            let cut = paint(color(cr, cg, cb));
            let travel = paint(Color::from_rgba8(lr, lg, lb, 150));
            let start = paint(color(0xd6, 0x28, 0x28));
            let [tr, tg, tb] = label_colour(media.color);
            let label = paint(color(tr, tg, tb));
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
            // The plan's travel starts at the device home (the media corner).
            let home = result.plan.home;
            let mut at = home;
            // Mark the home and label it H: the first dashed travel line
            // leaves from here. Below and right of the point, moved inside
            // the image when the home lies on its right or bottom edge.
            let mark = 6.0 * cell;
            let hx = ((home.x as f32 + ox) * sc).clamp(0.0, (w as f32 - mark).max(0.0));
            let hy = ((home.y as f32 + oy) * sc).clamp(0.0, (h as f32 - mark).max(0.0));
            if let Some(r) = tiny_skia::Rect::from_xywh(hx, hy, 2.0 * cell, 2.0 * cell) {
                pm.fill_rect(r, &travel, Transform::identity(), None);
            }
            draw_text(&mut pm, "H", hx + 3.0 * cell, hy + 3.0 * cell, cell, &label);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The same WCAG rule as the canvas: a mid grey mat (#808080) reads
    /// better with dark lines, which a plain luminance > 128 got wrong.
    #[test]
    fn grid_lines_use_the_wcag_contrast_colour() {
        assert_eq!(line_colour([0x80, 0x80, 0x80]), [0x1B, 0x1E, 0x22]);
        assert_eq!(line_colour([0x20, 0x20, 0x20]), [0xF4, 0xF5, 0xF7]);
        assert_eq!(line_colour([0xF4, 0xF1, 0xEA]), [0x1B, 0x1E, 0x22]);
    }

    /// Numbers and the H must read on a dark mat too.
    #[test]
    fn path_labels_read_on_a_dark_mat() {
        let dark = [0x20, 0x22, 0x26];
        assert_eq!(label_colour(dark), [0xF4, 0xF5, 0xF7]);
        assert_eq!(label_colour([0xF4, 0xF1, 0xEA]), [0x1B, 0x1E, 0x22]);
    }
}
