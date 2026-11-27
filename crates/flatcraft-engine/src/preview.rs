//! Preview images of the active document, drawn with tiny-skia.
//!
//! Canvas view: the media, the cutting area and what is cut of the objects
//! (their cut geometry, so a stroke shows the outline the knife cuts) in
//! their colours.
//! Path view: the media and cutting area, then the toolpaths in cut order
//! (each with its number), travel moves dashed and a dot on every start point;
//! the 0,0 where the first travel move starts is marked and labelled H (home).
//! Order numbers come from a tiny built-in 3x5 digit font, because the engine
//! has no text rendering library. Direction arrows are not drawn.
//! Simulation view: the head's path in grey, the blade tip coloured by its
//! deviation (green within `END_TO_END_LIMIT_MM`, amber within twice that,
//! red beyond), blade-down waste moves dashed in slate blue and the worst spots that miss
//! the limit ringed.
//! In firmware mode nothing is simulated, so it shows the commanded path.

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
    /// The simulated blade tip coloured by deviation (`job.simulate`).
    Simulation,
}

/// What [`draw`] puts on the mat.
enum View<'a> {
    /// The objects, by their cut geometry.
    Canvas(&'a crate::cutgeom::CutGeometry),
    Path(&'a crate::job::JobResult),
    Simulation(&'a crate::simulate::SimRun),
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
    let doc = &s.doc().document;
    match kind {
        PreviewKind::Canvas => {
            let cut = s.cut_geometry()?;
            draw(&plotter, doc, View::Canvas(&cut), max_px)
        }
        PreviewKind::Path => {
            let job = crate::job::build(s)?;
            draw(&plotter, doc, View::Path(&job), max_px)
        }
        PreviewKind::Simulation => {
            let run = crate::simulate::current_unstored(s)?;
            draw(&plotter, doc, View::Simulation(&run), max_px)
        }
    }
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
    render_document_png_seeded(plotter, doc, None, max_px)
}

/// [`render_document_png`] with cut geometry to start from (the
/// document's cached one, see `DocState::cached_cut_geometry`): only the
/// objects it does not fit are cut again. The objects are drawn as the
/// canvas preview draws them, by what is cut of them.
pub fn render_document_png_seeded(
    plotter: &flatcraft_device::PlotterProfile,
    doc: &flatcraft_core::document::Document,
    seed: Option<&std::sync::Arc<crate::cutgeom::CutGeometry>>,
    max_px: u32,
) -> Result<Vec<u8>, CommandError> {
    if !(MIN_PX..=MAX_PX).contains(&max_px) {
        return Err(CommandError::InvalidParams(format!(
            "max_px must be between {MIN_PX} and {MAX_PX}, not {max_px}"
        )));
    }
    // The same tolerance and narrowest feature as a job input
    // (`JobInput::tolerance_mm`, `JobInput::min_width_mm`).
    let cut = crate::cutgeom::objects_cut_geometry(
        &crate::job::JobInput::objects_of(doc),
        plotter.flatten_tolerance_mm,
        crate::report::min_width_mm(plotter.blade_offset_mm),
        seed,
    );
    draw(plotter, doc, View::Canvas(&cut), max_px)
}

/// The canvas, path or simulation view of `doc`.
fn draw(
    plotter: &flatcraft_device::PlotterProfile,
    doc: &flatcraft_core::document::Document,
    view: View<'_>,
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

    match view {
        View::Canvas(cut) => {
            for o in doc.objects.iter().filter(|o| o.visible) {
                // What is cut of the object, in the colour of the source
                // path each piece came from.
                let shapes = cut.pieces(o.id).iter().map(|piece| {
                    let colour = piece
                        .source
                        .and_then(|i| o.geometry.get(i))
                        .and_then(|p| p.color)
                        .or_else(|| o.geometry.iter().find_map(|p| p.color));
                    (&piece.path, colour)
                });
                for (path, colour) in shapes {
                    let c = colour.unwrap_or([0x22, 0x26, 0x2e]);
                    if let Some(p) = to_skia(path) {
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
        View::Simulation(run) => {
            let canvas = Canvas {
                pm: &mut pm,
                mat,
                px_mm: px(1.0) as f32,
                line: [lr, lg, lb],
                cut: flatcraft_core::colour::cut_colour(media.color),
            };
            draw_simulation(canvas, run);
        }
        View::Path(result) => {
            let [cr, cg, cb] = flatcraft_core::colour::cut_colour(media.color);
            let cut = paint(color(cr, cg, cb));
            // The light theme's weed blue (the image has no theme), or the
            // dark one when only that reads on the mat.
            let [wr, wg, wb] = flatcraft_core::colour::weed_colour(
                media.color,
                flatcraft_core::colour::WEED_LIGHT,
            );
            let weed = paint(color(wr, wg, wb));
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
            let weed_stroke = Stroke {
                dash: StrokeDash::new(vec![px(8.0) as f32, px(4.0) as f32], 0.0),
                ..cut_stroke.clone()
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
                    // Weed paths are dashed: they read apart from the
                    // design (solid) and the travel (short dashes, thin).
                    let (ink, stroke) = if t.kind == flatcraft_path::ToolpathKind::Weed {
                        (&weed, &weed_stroke)
                    } else {
                        (&cut, &cut_stroke)
                    };
                    pm.stroke_path(&p, ink, stroke, mat, None);
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

/// The pixmap and the scale a view draws with.
struct Canvas<'a> {
    pm: &'a mut Pixmap,
    /// Mat millimetres to pixels.
    mat: Transform,
    /// One pixel in mm.
    px_mm: f32,
    /// The mat's contrast colour.
    line: [u8; 3],
    /// The cut colour on this mat.
    cut: [u8; 3],
}

/// The tip colour of a sample with `deviation_mm`: green within the
/// end-to-end limit, amber within twice that, red beyond.
pub fn deviation_colour(deviation_mm: f32) -> [u8; 3] {
    let d = f64::from(deviation_mm);
    let limit = flatcraft_sim::END_TO_END_LIMIT_MM;
    if d.is_nan() || d <= limit {
        [0x1f, 0x9d, 0x55]
    } else if d <= 2.0 * limit {
        [0xe0, 0x9b, 0x00]
    } else {
        [0xd6, 0x28, 0x28]
    }
}

/// The colour of the rings around the worst spots.
pub const RING: [u8; 3] = [0xc0, 0x1c, 0x8c];
/// The radius of a worst spot's ring, px, at every image size.
pub const RING_RADIUS_PX: f32 = 9.0;
/// The line width of a worst spot's ring, px.
pub const RING_WIDTH_PX: f32 = 2.5;

/// The colour of blade-down waste moves (swivels, lead-ins, overshoots).
const WASTE_INK: [u8; 3] = [0x5a, 0x6b, 0x8c];

/// How a stretch of tip moves is drawn.
#[derive(Clone, Copy, PartialEq)]
enum Ink {
    Tip([u8; 3]),
    Waste([u8; 3]),
}

/// The simulation view: the head (axis) path in grey, the tip coloured by
/// deviation, waste moves dashed, travel thin and dashed, and the worst
/// spots ringed. Firmware mode draws the commanded path only.
fn draw_simulation(c: Canvas<'_>, run: &crate::simulate::SimRun) {
    let Canvas {
        pm,
        mat,
        px_mm,
        line,
        cut,
    } = c;
    let [lr, lg, lb] = line;
    let travel = paint(Color::from_rgba8(lr, lg, lb, 120));
    let travel_stroke = Stroke {
        width: px_mm,
        dash: StrokeDash::new(vec![5.0 * px_mm, 4.0 * px_mm], 0.0),
        ..Stroke::default()
    };
    let Some(result) = &run.result else {
        // Firmware mode: the commanded path, as the plotter gets it.
        let ink = paint(color(cut[0], cut[1], cut[2]));
        let stroke = Stroke {
            width: 1.5 * px_mm,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        let mut at = run.job.plan.home;
        for &i in &run.plan_index {
            for r in run.job.plan.toolpaths[i].runs() {
                let Some(&first) = r.points.first() else {
                    continue;
                };
                if let Some(p) = polyline(&[at, first], false) {
                    pm.stroke_path(&p, &travel, &travel_stroke, mat, None);
                }
                if let Some(p) = polyline(&r.points, false) {
                    pm.stroke_path(&p, &ink, &stroke, mat, None);
                }
                at = *r.points.last().unwrap_or(&first);
            }
        }
        return;
    };
    let samples = &run.display;
    // The head: every blade-down axis move, thin grey.
    let axis_paint = paint(Color::from_rgba8(0x8a, 0x8f, 0x98, 200));
    let axis_stroke = Stroke {
        width: px_mm,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    let mut axis = PathBuilder::new();
    let mut up = PathBuilder::new();
    for w in samples.windows(2) {
        let (a, b) = (w[0], w[1]);
        let pb = if b.down { &mut axis } else { &mut up };
        pb.move_to(a.axis.x as f32, a.axis.y as f32);
        pb.line_to(b.axis.x as f32, b.axis.y as f32);
    }
    if let Some(p) = up.finish() {
        pm.stroke_path(&p, &travel, &travel_stroke, mat, None);
    }
    if let Some(p) = axis.finish() {
        pm.stroke_path(&p, &axis_paint, &axis_stroke, mat, None);
    }
    // The tip: consecutive moves of one ink as one path.
    let tip_stroke = Stroke {
        width: 1.5 * px_mm,
        line_join: LineJoin::Round,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    let waste_stroke = Stroke {
        dash: StrokeDash::new(vec![3.0 * px_mm, 3.0 * px_mm], 0.0),
        ..tip_stroke.clone()
    };
    let flush = |pm: &mut Pixmap, ink: Option<Ink>, pb: PathBuilder| {
        let (Some(ink), Some(p)) = (ink, pb.finish()) else {
            return;
        };
        match ink {
            Ink::Tip([r, g, b]) => {
                pm.stroke_path(&p, &paint(color(r, g, b)), &tip_stroke, mat, None)
            }
            Ink::Waste([r, g, b]) => {
                pm.stroke_path(&p, &paint(color(r, g, b)), &waste_stroke, mat, None)
            }
        }
    };
    let mut current: Option<Ink> = None;
    let mut pb = PathBuilder::new();
    for w in samples.windows(2) {
        let (a, b) = (w[0], w[1]);
        if !b.down {
            continue;
        }
        // A waste move lies in waste on purpose, away from the line, so
        // its distance to the line says nothing: one plain colour. A waste
        // move into kept material is a worst spot and gets its ring.
        let ink = if b.waste {
            Ink::Waste(WASTE_INK)
        } else {
            Ink::Tip(deviation_colour(b.deviation_mm))
        };
        if current != Some(ink) {
            flush(pm, current, std::mem::take(&mut pb));
            current = Some(ink);
        }
        pb.move_to(a.tip.x as f32, a.tip.y as f32);
        pb.line_to(b.tip.x as f32, b.tip.y as f32);
    }
    flush(pm, current, pb);
    // The worst spots that are a problem: a ring each, the same number of
    // pixels at every image size, so it marks the place without hiding
    // it. A spot within the limit is only the least good of a good cut,
    // so it is not ringed.
    let ring = paint(color(RING[0], RING[1], RING[2]));
    let ring_stroke = Stroke {
        width: RING_WIDTH_PX * px_mm,
        ..Stroke::default()
    };
    let radius = RING_RADIUS_PX * px_mm;
    let limit = flatcraft_sim::END_TO_END_LIMIT_MM;
    for spot in result
        .metrics
        .worst_spots
        .iter()
        .filter(|x| x.kind != flatcraft_sim::SpotKind::Deviation || x.deviation_mm > limit)
    {
        if let Some(p) = PathBuilder::from_circle(spot.at.x as f32, spot.at.y as f32, radius) {
            pm.stroke_path(&p, &ring, &ring_stroke, mat, None);
        }
    }
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
