//! The kept and waste overlay: kept material drawn as solid paper, waste
//! with a light hatch. It reads the plan's own [`KeptMaterial`] (the one
//! the knife stage judged every waste move against), so the canvas shows
//! exactly what the planner keeps. Always on in Cut; a View toggle in
//! Prepare.

use std::sync::Arc;

use egui::{Color32, ColorImage, Pos2, Rect, Stroke, TextureHandle, TextureOptions, pos2};
use flatcraft_engine::job::JobResult;
use flatcraft_path::KeptMaterial;

use super::canvas::{self, CanvasView};
use super::theme::Tokens;

/// The most texels along the long side of the mask.
pub const MAX_TEXELS: usize = 1600;

/// The smallest texel, in mm: finer shows nothing a cut line does not
/// cover anyway.
pub const MIN_TEXEL_MM: f64 = 0.1;

/// The hatch spacing on screen, in points.
pub const HATCH_PX: f32 = 7.0;

/// What is kept, sampled over the cutting area: one coverage value per
/// texel (0 waste, 255 kept), row by row from the top. Each row is the
/// exact answer of [`KeptMaterial::kept_spans`] along its middle, so a
/// texel's value is the share of that line that is kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    /// The cutting area in mat mm.
    pub area: kurbo::Rect,
    /// Texels across and down.
    pub size: [usize; 2],
    pub coverage: Vec<u8>,
}

impl Mask {
    /// The mask of `kept` over `area`, at most `max` texels on the long
    /// side. `None` for an empty or broken area.
    pub fn of(kept: &KeptMaterial, area: kurbo::Rect, max: usize) -> Option<Self> {
        let (w, h) = (area.width(), area.height());
        if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) || max == 0 {
            return None;
        }
        let texel = (w.max(h) / max as f64).max(MIN_TEXEL_MM);
        let size = [
            ((w / texel).ceil() as usize).max(1),
            ((h / texel).ceil() as usize).max(1),
        ];
        let (tx, ty) = (w / size[0] as f64, h / size[1] as f64);
        let mut coverage = vec![0u8; size[0] * size[1]];
        for row in 0..size[1] {
            let y = area.y0 + (row as f64 + 0.5) * ty;
            let line = &mut coverage[row * size[0]..(row + 1) * size[0]];
            for (a, b) in kept.kept_spans(y, area.x0, area.x1) {
                // Texel columns from a to b, the end ones in part.
                let (fa, fb) = ((a - area.x0) / tx, (b - area.x0) / tx);
                let first = (fa.floor().max(0.0) as usize).min(size[0] - 1);
                let last = ((fb.ceil() as usize).max(1) - 1).min(size[0] - 1);
                for (col, c) in line.iter_mut().enumerate().take(last + 1).skip(first) {
                    let lo = fa.max(col as f64);
                    let hi = fb.min(col as f64 + 1.0);
                    let add = ((hi - lo).max(0.0) * 255.0).round() as u16;
                    *c = (u16::from(*c) + add).min(255) as u8;
                }
            }
        }
        Some(Self {
            area,
            size,
            coverage,
        })
    }

    /// The coverage of the texel holding `p` (mat mm), `None` outside.
    pub fn at(&self, p: kurbo::Point) -> Option<u8> {
        let a = self.area;
        if !a.contains(p) {
            return None;
        }
        let col = (((p.x - a.x0) / a.width() * self.size[0] as f64) as usize).min(self.size[0] - 1);
        let row =
            (((p.y - a.y0) / a.height() * self.size[1] as f64) as usize).min(self.size[1] - 1);
        Some(self.coverage[row * self.size[0] + col])
    }

    /// The middle of texel (`col`, `row`) in mat mm.
    pub fn centre(&self, col: usize, row: usize) -> kurbo::Point {
        let a = self.area;
        kurbo::Point::new(
            a.x0 + (col as f64 + 0.5) * a.width() / self.size[0] as f64,
            a.y0 + (row as f64 + 0.5) * a.height() / self.size[1] as f64,
        )
    }

    /// White, with the coverage as alpha (premultiplied), to be tinted.
    fn image(&self) -> ColorImage {
        let rgba: Vec<u8> = self.coverage.iter().flat_map(|&c| [c, c, c, c]).collect();
        ColorImage::from_rgba_premultiplied(self.size, &rgba)
    }
}

/// The overlay of the newest plan: its mask, made once per plan, and the
/// texture made from it once per window.
#[derive(Default)]
pub struct KeptCache {
    plan: Option<Arc<JobResult>>,
    mask: Option<Arc<Mask>>,
    texture: Option<TextureHandle>,
    /// How many masks were made, for tests.
    pub made: usize,
}

impl std::fmt::Debug for KeptCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeptCache")
            .field("made", &self.made)
            .finish_non_exhaustive()
    }
}

impl Clone for KeptCache {
    /// A copy starts empty: the mask is made again when it is needed.
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl KeptCache {
    /// The mask of `job` over `area`: made once per plan.
    pub fn mask(&mut self, job: &Arc<JobResult>, area: kurbo::Rect) -> Option<Arc<Mask>> {
        let same = self.plan.as_ref().is_some_and(|p| Arc::ptr_eq(p, job))
            && self.mask.as_ref().is_some_and(|m| m.area == area);
        if !same {
            self.plan = Some(job.clone());
            self.mask = Mask::of(&job.plan.kept, area, MAX_TEXELS).map(Arc::new);
            self.texture = None;
            self.made += 1;
        }
        self.mask.clone()
    }

    /// The texture of the newest mask, once it was drawn.
    pub fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture.as_ref().map(TextureHandle::id)
    }

    /// Draws the overlay of `job` on the cutting area: the hatch over all
    /// of it, then the kept paper on top, so only waste shows the hatch.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        view: &CanvasView,
        job: &Arc<JobResult>,
        area: kurbo::Rect,
        mat: [u8; 3],
        t: &Tokens,
    ) {
        let Some(mask) = self.mask(job, area) else {
            return;
        };
        let texture = self.texture.get_or_insert_with(|| {
            ctx.load_texture("flatcraft-kept", mask.image(), TextureOptions::LINEAR)
        });
        let r = view.rect_to_screen(area);
        hatch(painter, r, hatch_colour(mat, t));
        painter.image(
            texture.id(),
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            t.kept_paper,
        );
    }
}

/// The hatch colour on a mat: the theme's on a light mat, the mat's light
/// contrast colour with the same alpha on a dark one.
pub fn hatch_colour(mat: [u8; 3], t: &Tokens) -> Color32 {
    let contrast = canvas::contrast_colour(mat);
    if contrast[0] > 0x80 {
        let a = t.waste_hatch.a();
        Color32::from_rgba_unmultiplied(contrast[0], contrast[1], contrast[2], a)
    } else {
        t.waste_hatch
    }
}

/// Diagonal lines [`HATCH_PX`] apart over `r` (screen), anchored to its
/// corner so they move with the mat, drawn only where the painter clips.
pub fn hatch(painter: &egui::Painter, r: Rect, colour: Color32) {
    let clip = painter.clip_rect().intersect(r);
    if !clip.is_positive() {
        return;
    }
    let p = painter.with_clip_rect(clip);
    let stroke = Stroke::new(1.0, colour);
    // Lines x + y = k with k - (r.left + r.top) a multiple of the spacing.
    let base = r.left() + r.top();
    let k0 = clip.left() + clip.top();
    let k1 = clip.right() + clip.bottom();
    let mut k = base + ((k0 - base) / HATCH_PX).floor() * HATCH_PX;
    while k <= k1 {
        // From the bottom edge (or the left) to the top edge (or the right).
        let a = Pos2::new(k - clip.bottom(), clip.bottom());
        let b = Pos2::new(k - clip.top(), clip.top());
        p.line_segment([a, b], stroke);
        k += HATCH_PX;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flatcraft_core::document::{KeepMode, ObjectId};
    use flatcraft_path::{PathRef, Toolpath};
    use kurbo::Point;

    fn ring(object: u64, pts: &[(f64, f64)]) -> Toolpath {
        let mut points: Vec<Point> = pts.iter().map(|&p| p.into()).collect();
        points.push(points[0]);
        Toolpath::new(
            points,
            true,
            PathRef {
                object: ObjectId(object),
                path_index: 0,
            },
        )
    }

    fn square(object: u64, x0: f64, y0: f64, x1: f64, y1: f64) -> Toolpath {
        ring(object, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
    }

    /// A square with a hole (1), a triangle (2) and a square that
    /// overlaps the first (3).
    fn job() -> Vec<Toolpath> {
        vec![
            square(1, 10.0, 10.0, 50.0, 50.0),
            square(1, 20.0, 20.0, 40.0, 40.0),
            ring(2, &[(60.0, 70.0), (90.0, 70.0), (75.0, 95.0)]),
            square(3, 35.0, 35.0, 70.0, 60.0),
        ]
    }

    /// Every texel whose row is wholly kept or wholly waste agrees with
    /// `is_kept` at its middle. Returns (kept, waste) texels checked.
    fn agrees(kept: &KeptMaterial, mask: &Mask) -> (usize, usize) {
        let (mut k, mut w) = (0, 0);
        for row in 0..mask.size[1] {
            for col in 0..mask.size[0] {
                let c = mask.coverage[row * mask.size[0] + col];
                let p = mask.centre(col, row);
                match c {
                    255 => {
                        assert!(kept.is_kept(p), "{p:?} is drawn kept");
                        k += 1;
                    }
                    0 => {
                        assert!(!kept.is_kept(p), "{p:?} is drawn as waste");
                        w += 1;
                    }
                    // A texel a boundary runs through.
                    _ => {}
                }
                assert_eq!(mask.at(p), Some(c));
            }
        }
        (k, w)
    }

    #[test]
    fn the_overlay_matches_the_kept_material_for_every_mode() {
        let area = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0);
        let paths = job();
        for mode in [KeepMode::Pieces, KeepMode::Stencil, KeepMode::Both] {
            let kept = KeptMaterial::from_toolpaths(&paths, |_| mode, area, 0.6);
            let mask = Mask::of(&kept, area, 400).unwrap();
            assert_eq!(mask.size, [400, 400]);
            let (k, w) = agrees(&kept, &mask);
            assert!(k > 1000 && w > 1000, "{mode:?}: {k} kept, {w} waste");
        }
    }

    #[test]
    fn the_overlay_shows_that_kept_wins_where_objects_overlap() {
        let area = kurbo::Rect::new(0.0, 0.0, 100.0, 100.0);
        let paths = job();
        // The overlapping square is a stencil, the others pieces: inside
        // the first square the piece wins; inside its hole the stencil's
        // sheet wins; inside the stencil's own square, away from the
        // piece, it falls out.
        let mode = |o: ObjectId| {
            if o.0 == 3 {
                KeepMode::Stencil
            } else {
                KeepMode::Pieces
            }
        };
        let kept = KeptMaterial::from_toolpaths(&paths, mode, area, 0.6);
        let mask = Mask::of(&kept, area, 500).unwrap();
        agrees(&kept, &mask);
        let at = |x: f64, y: f64| mask.at(Point::new(x, y)).unwrap();
        assert_eq!(at(45.1, 45.1), 255, "the piece overlaps the stencil");
        assert_eq!(at(25.1, 25.1), 255, "the hole is on the stencil's sheet");
        assert_eq!(at(60.1, 50.1), 0, "the stencil's shape falls out");
        assert_eq!(at(5.1, 95.1), 255, "the sheet");
        assert_eq!(at(75.1, 85.1), 255, "the triangle piece");
    }

    #[test]
    fn a_mask_has_bounded_size_and_refuses_a_broken_area() {
        let kept = KeptMaterial::from_toolpaths(
            &job(),
            |_| KeepMode::Pieces,
            kurbo::Rect::new(0.0, 0.0, 300.0, 300.0),
            0.0,
        );
        let big = Mask::of(&kept, kurbo::Rect::new(0.0, 0.0, 300.0, 150.0), MAX_TEXELS).unwrap();
        assert_eq!(big.size, [MAX_TEXELS, MAX_TEXELS / 2]);
        let small = Mask::of(&kept, kurbo::Rect::new(0.0, 0.0, 10.0, 5.0), MAX_TEXELS).unwrap();
        assert_eq!(small.size, [100, 50], "no texel under {MIN_TEXEL_MM} mm");
        for bad in [
            kurbo::Rect::new(0.0, 0.0, 0.0, 10.0),
            kurbo::Rect::new(0.0, 0.0, f64::NAN, 10.0),
        ] {
            assert!(Mask::of(&kept, bad, MAX_TEXELS).is_none());
        }
    }
}
