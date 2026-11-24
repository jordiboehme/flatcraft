//! Import (file dialog and drag-and-drop) and the Trace dialog.
//!
//! The Trace dialog copies the image bytes and the trace settings of one
//! raster object when it opens, under one short lock. Previews are traced
//! on a worker thread that never touches the session: a change waits
//! [`DEBOUNCE_S`] for the next one, then goes out as a job with a new
//! generation number, and results of older generations are dropped. Apply
//! sends one `object.retrace` to the object's document.

use std::path::Path;
use std::sync::{Arc, mpsc};

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, vec2};
use flatcraft_core::document::TraceSettings;
use flatcraft_engine::commands::sources::file_kind;
use flatcraft_import::ImportError;
use flatcraft_import::trace::TraceResult;
use serde_json::json;

use super::files::{FileRequest, filter};
use super::prepare::Snapshot;
use super::settings::set_name;
use super::theme::Tokens;
use super::{Action, PROJECT_EXTENSION};

/// Seconds after the last change before a preview is traced.
pub const DEBOUNCE_S: f64 = 0.2;
/// Longest side of the preview image, in pixels.
const THUMB_MAX: usize = 512;
/// Side of the preview box in the dialog, in points.
const PREVIEW_SIZE: f32 = 280.0;

/// Traces image bytes with settings and a blade offset (mm). The app uses
/// [`flatcraft_import::trace::trace`]; tests replace it.
pub type Tracer =
    Arc<dyn Fn(&[u8], &TraceSettings, f64) -> Result<TraceResult, ImportError> + Send + Sync>;

// ---------------------------------------------------------------- import

/// The Import file dialog: everything FlatCraft imports, then each type.
pub fn import_request() -> FileRequest {
    FileRequest {
        title: "Import".into(),
        filters: vec![
            filter(
                "SVG or image",
                &["svg", "svgz", "png", "jpg", "jpeg", "webp", "gif"],
            ),
            filter("SVG", &["svg", "svgz"]),
            filter("PNG", &["png"]),
            filter("JPEG", &["jpg", "jpeg"]),
            filter("WebP", &["webp"]),
            filter("GIF", &["gif"]),
        ],
        ..Default::default()
    }
}

/// What a file dropped on the window becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropKind {
    /// A FlatCraft project: `project.open`.
    Project,
    /// An SVG or an image: `object.import`.
    Import,
    Unsupported,
}

/// Sorts a dropped file by its extension (case does not matter).
pub fn drop_kind(path: &Path) -> DropKind {
    let project = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION));
    if project {
        DropKind::Project
    } else if file_kind(path).is_ok() {
        DropKind::Import
    } else {
        DropKind::Unsupported
    }
}

/// The toast for a dropped file FlatCraft cannot use.
pub fn unsupported_text(path: &Path) -> String {
    format!(
        "Cannot import {}. Drop an SVG, PNG, JPEG, WebP or GIF file, or a .flatcraft project.",
        super::display_name(path)
    )
}

// ---------------------------------------------------------------- trace worker

/// A traced preview: the outlines in image mm (top-left 0,0, y down).
#[derive(Debug, Clone)]
pub struct Preview {
    /// The settings change this preview belongs to.
    pub generation: u64,
    /// Closed polylines, one per outline and hole.
    pub lines: Vec<Vec<kurbo::Point>>,
    pub threshold_used: u8,
    /// The threshold was picked by Otsu's method.
    pub auto_threshold: bool,
    pub dropped_small: usize,
    /// Width and height of the whole image, mm.
    pub size_mm: (f64, f64),
}

struct Job {
    generation: u64,
    settings: TraceSettings,
}

enum Reply {
    /// The image in grey, for the preview box.
    Thumbnail(egui::ColorImage),
    Traced {
        generation: u64,
        result: Result<Preview, String>,
    },
}

/// The polylines of a trace result.
fn preview_of(generation: u64, auto_threshold: bool, t: TraceResult) -> Preview {
    let lines = t
        .paths
        .iter()
        .map(|p| {
            p.path
                .elements()
                .iter()
                .filter_map(|el| match el {
                    kurbo::PathEl::MoveTo(pt) | kurbo::PathEl::LineTo(pt) => Some(*pt),
                    _ => None,
                })
                .collect()
        })
        .collect();
    Preview {
        generation,
        lines,
        threshold_used: t.threshold_used,
        auto_threshold,
        dropped_small: t.dropped_small,
        size_mm: t.size_mm,
    }
}

/// The image in grey, at most [`THUMB_MAX`] pixels on its longest side.
fn thumbnail(bytes: &[u8]) -> Option<egui::ColorImage> {
    let grey = flatcraft_import::raster::decode(bytes).ok()?;
    let (w, h) = grey.dimensions();
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let step = w.max(h).div_ceil(THUMB_MAX).max(1);
    let (tw, th) = (w.div_ceil(step), h.div_ceil(step));
    let raw = grey.as_raw();
    let pixels = (0..th).flat_map(|y| (0..tw).map(move |x| raw[(y * step) * w + x * step]));
    Some(egui::ColorImage::from_gray_iter([tw, th], pixels))
}

/// Runs preview jobs until the dialog drops its sender. Only the newest
/// waiting job is traced; the others are already stale.
fn worker(
    bytes: Arc<Vec<u8>>,
    blade_offset_mm: f64,
    tracer: Tracer,
    jobs: mpsc::Receiver<Job>,
    replies: mpsc::Sender<Reply>,
    ctx: egui::Context,
) {
    if let Some(image) = thumbnail(&bytes) {
        if replies.send(Reply::Thumbnail(image)).is_err() {
            return;
        }
        ctx.request_repaint();
    }
    while let Ok(mut job) = jobs.recv() {
        while let Ok(newer) = jobs.try_recv() {
            job = newer;
        }
        let auto = job.settings.threshold.is_none();
        let result = tracer(&bytes, &job.settings, blade_offset_mm)
            .map(|t| preview_of(job.generation, auto, t))
            .map_err(|e| e.to_string());
        let reply = Reply::Traced {
            generation: job.generation,
            result,
        };
        if replies.send(reply).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------- dialog

/// What the person opened the dialog with.
pub struct TraceSource {
    pub doc_id: u64,
    pub id: u64,
    pub name: String,
    pub bytes: Vec<u8>,
    pub settings: TraceSettings,
    pub blade_offset_mm: f64,
}

/// The open Trace dialog for one raster object.
pub struct TraceDialog {
    pub doc_id: u64,
    pub id: u64,
    pub name: String,
    /// The settings as edited; Apply sends them.
    pub settings: TraceSettings,
    /// Shown by the threshold slider while Auto is on.
    pub manual_threshold: u8,
    /// Shown by the minimum feature field while Auto is on.
    pub manual_min_feature: f64,
    pub blade_offset_mm: f64,
    /// The newest preview of the current settings, or of older ones until
    /// the new one arrives.
    pub preview: Option<Preview>,
    /// Why the last preview failed.
    pub error: Option<String>,
    generation: u64,
    /// The generation the last reply belonged to.
    done: u64,
    /// When (egui time) the settings last changed, until that change is sent.
    changed_at: Option<f64>,
    thumbnail: Option<egui::TextureHandle>,
    jobs: mpsc::Sender<Job>,
    replies: mpsc::Receiver<Reply>,
}

impl std::fmt::Debug for TraceDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceDialog")
            .field("doc_id", &self.doc_id)
            .field("id", &self.id)
            .field("settings", &self.settings)
            .field("generation", &self.generation)
            .field("done", &self.done)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl TraceDialog {
    /// Opens the dialog and starts tracing the first preview at once.
    pub fn open(ctx: &egui::Context, src: TraceSource, tracer: Tracer) -> Self {
        let (jobs, job_rx) = mpsc::channel();
        let (reply_tx, replies) = mpsc::channel();
        let bytes = Arc::new(src.bytes);
        let worker_ctx = ctx.clone();
        let blade = src.blade_offset_mm;
        let spawned = std::thread::Builder::new()
            .name("trace-preview".into())
            .spawn(move || worker(bytes, blade, tracer, job_rx, reply_tx, worker_ctx));
        let error = spawned
            .err()
            .map(|e| format!("Cannot start the preview: {e}"));
        let manual_threshold = src.settings.threshold.unwrap_or(128);
        let manual_min_feature = src
            .settings
            .min_feature_mm
            .unwrap_or(6.0 * src.blade_offset_mm);
        let mut d = Self {
            doc_id: src.doc_id,
            id: src.id,
            name: src.name,
            settings: src.settings,
            manual_threshold,
            manual_min_feature,
            blade_offset_mm: src.blade_offset_mm,
            preview: None,
            error,
            generation: 0,
            done: 0,
            changed_at: None,
            thumbnail: None,
            jobs,
            replies,
        };
        d.generation = 1;
        d.send();
        d
    }

    /// The generation of the current settings.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// A preview of the current settings is still to come.
    pub fn busy(&self) -> bool {
        self.done < self.generation
    }

    /// The line under the preview: the threshold used (Otsu when Auto) and
    /// the small shapes dropped.
    pub fn status(&self) -> String {
        if let Some(e) = &self.error {
            return format!("Trace failed: {e}");
        }
        let Some(p) = &self.preview else {
            return "Tracing... reticulating splines.".into();
        };
        let how = if p.auto_threshold { " (Otsu)" } else { "" };
        let dropped = match p.dropped_small {
            1 => "1 small shape dropped".to_string(),
            n => format!("{n} small shapes dropped"),
        };
        format!("Threshold {}{how}, {dropped}", p.threshold_used)
    }

    /// The params of the `object.retrace` that Apply sends. Fields the
    /// dialog does not show (dpi) keep their stored values.
    pub fn retrace_params(&self) -> serde_json::Value {
        json!({ "id": self.id, "settings": self.settings })
    }

    fn send(&mut self) {
        self.changed_at = None;
        let job = Job {
            generation: self.generation,
            settings: self.settings.clone(),
        };
        if self.jobs.send(job).is_err() && self.error.is_none() {
            self.error = Some("The preview stopped.".into());
        }
    }

    fn changed(&mut self, now: f64) {
        self.generation += 1;
        self.changed_at = Some(now);
    }

    /// Takes the worker's replies and sends a waiting change once it is
    /// [`DEBOUNCE_S`] old.
    fn poll(&mut self, ctx: &egui::Context, now: f64) {
        while let Ok(reply) = self.replies.try_recv() {
            match reply {
                Reply::Thumbnail(image) => {
                    self.thumbnail = Some(ctx.load_texture(
                        "flatcraft-trace-preview",
                        image,
                        egui::TextureOptions::NEAREST,
                    ));
                }
                Reply::Traced { generation, result } => {
                    if generation != self.generation {
                        continue; // stale
                    }
                    self.done = generation;
                    match result {
                        Ok(p) => {
                            self.preview = Some(p);
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
        if let Some(at) = self.changed_at {
            let wait = at + DEBOUNCE_S - now;
            if wait <= 0.0 {
                self.send();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            }
        }
    }
}

/// What the dialog's buttons asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Open,
    Apply,
    Close,
}

/// Draws the Trace dialog, if open, and closes it when its document is no
/// longer active or its object is gone. Escape closes it without changes.
pub fn show(
    ctx: &egui::Context,
    dialog: &mut Option<TraceDialog>,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) -> Option<&'static str> {
    let d = dialog.as_mut()?;
    if d.doc_id != snap.doc_id {
        *dialog = None;
        return Some("The Trace dialog closed because another document became active.");
    }
    if !snap.object(d.id).is_some_and(|o| o.raster) {
        *dialog = None;
        return Some("The Trace dialog closed because its image is gone.");
    }
    let now = ctx.input(|i| i.time);
    d.poll(ctx, now);
    let mut outcome = Outcome::Open;
    if !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        outcome = Outcome::Close;
    }
    let mut open = true;
    egui::Window::new(format!("Trace {}", d.name))
        .id(egui::Id::unique("flatcraft-trace-dialog"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - vec2(320.0, 220.0))
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(260.0);
                    if controls(ui, d) {
                        d.changed(now);
                    }
                });
                ui.add_space(t.spacing * 2.0);
                preview_box(ui, d, t);
            });
            ui.add_space(t.spacing);
            let status = d.status();
            let colour = if d.error.is_some() {
                t.danger
            } else {
                t.text_dim
            };
            ui.label(egui::RichText::new(status).color(colour));
            ui.add_space(t.spacing);
            ui.horizontal(|ui| {
                if ui
                    .button("Apply")
                    .on_hover_text("Trace the image with these settings")
                    .clicked()
                {
                    outcome = Outcome::Apply;
                }
                if ui.button("Cancel").clicked() {
                    outcome = Outcome::Close;
                }
            });
        });
    if !open {
        outcome = Outcome::Close;
    }
    match outcome {
        Outcome::Open => {}
        Outcome::Apply => {
            actions.push(Action::run(d.doc_id, "object.retrace", d.retrace_params()));
            *dialog = None;
        }
        Outcome::Close => *dialog = None,
    }
    None
}

/// The settings. Returns true when one changed.
fn controls(ui: &mut Ui, d: &mut TraceDialog) -> bool {
    let mut changed = false;
    let s = &mut d.settings;

    ui.horizontal(|ui| {
        let mut auto = s.threshold.is_none();
        let r = ui
            .add_enabled(
                !auto,
                egui::Slider::new(&mut d.manual_threshold, 0..=255).text("Threshold"),
            )
            .on_hover_text("Pixels darker than this are cut");
        set_name(ui, &r, "Threshold");
        if r.changed() {
            s.threshold = Some(d.manual_threshold);
            changed = true;
        }
        let r = ui
            .checkbox(&mut auto, "Auto")
            .on_hover_text("Let Otsu's method pick the threshold");
        set_name(ui, &r, "Automatic threshold");
        if r.changed() {
            s.threshold = (!auto).then_some(d.manual_threshold);
            changed = true;
        }
    });
    changed |= ui
        .checkbox(&mut s.invert, "Invert")
        .on_hover_text("Cut the light parts instead of the dark ones")
        .changed();
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::DragValue::new(&mut s.despeckle_mm2)
                .range(0.0..=100.0)
                .speed(0.05)
                .max_decimals(2)
                .suffix(" mm²"),
        );
        set_name(ui, &r, "Despeckle (mm²)");
        changed |= r.changed();
        ui.label("Despeckle")
            .on_hover_text("Specks smaller than this area vanish");
    });
    let r = ui
        .add(egui::Slider::new(&mut s.corner_threshold_deg, 0.0..=180.0).text("Corner threshold"))
        .on_hover_text("Angles sharper than this stay corners (degrees)");
    set_name(ui, &r, "Corner threshold (deg)");
    changed |= r.changed();
    let r = ui
        .add(egui::Slider::new(&mut s.smoothing, 0..=10).text("Smoothing"))
        .on_hover_text("0 follows the pixels, 10 gives the smoothest curves");
    set_name(ui, &r, "Smoothing");
    changed |= r.changed();
    ui.horizontal(|ui| {
        let mut auto = s.min_feature_mm.is_none();
        let r = ui.add_enabled(
            !auto,
            egui::DragValue::new(&mut d.manual_min_feature)
                .range(0.0..=50.0)
                .speed(0.05)
                .max_decimals(2)
                .suffix(" mm"),
        );
        set_name(ui, &r, "Minimum feature (mm)");
        if r.changed() {
            s.min_feature_mm = Some(d.manual_min_feature);
            changed = true;
        }
        ui.label("Minimum feature").on_hover_text(
            "Shapes thinner than this are dropped. A blade cannot cut what it cannot turn in.",
        );
        let r = ui
            .checkbox(&mut auto, "Auto")
            .on_hover_text("6 times the blade offset of the plotter");
        set_name(ui, &r, "Automatic minimum feature");
        if r.changed() {
            s.min_feature_mm = (!auto).then_some(d.manual_min_feature);
            changed = true;
        }
    });
    changed
}

/// The image with the traced outline on top.
fn preview_box(ui: &mut Ui, d: &TraceDialog, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(PREVIEW_SIZE, PREVIEW_SIZE), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius, t.surface);
    let aspect = d
        .preview
        .as_ref()
        .map(|p| p.size_mm)
        .filter(|(w, h)| *w > 0.0 && *h > 0.0)
        .map(|(w, h)| (w / h) as f32)
        .or_else(|| {
            d.thumbnail.as_ref().map(|tex| {
                let s = tex.size_vec2();
                s.x / s.y.max(1.0)
            })
        })
        .unwrap_or(1.0);
    let inner = rect.shrink(8.0);
    let size = if aspect >= inner.width() / inner.height() {
        vec2(inner.width(), inner.width() / aspect)
    } else {
        vec2(inner.height() * aspect, inner.height())
    };
    let image = Rect::from_center_size(inner.center(), size);
    if let Some(tex) = &d.thumbnail {
        // Faded so the outline reads on any image.
        painter.image(
            tex.id(),
            image,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::from_white_alpha(140),
        );
    }
    if let Some(p) = &d.preview {
        let (w, h) = p.size_mm;
        if w > 0.0 && h > 0.0 {
            let to_screen = |pt: &kurbo::Point| {
                Pos2::new(
                    image.min.x + (pt.x / w) as f32 * image.width(),
                    image.min.y + (pt.y / h) as f32 * image.height(),
                )
            };
            let stroke = Stroke::new(1.5, t.accent_prepare);
            for line in &p.lines {
                let pts: Vec<Pos2> = line.iter().map(to_screen).collect();
                painter.add(egui::Shape::closed_line(pts, stroke));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn dropped_files_are_sorted_by_extension() {
        let k = |p: &str| drop_kind(&PathBuf::from(p));
        assert_eq!(k("/a/b.FlatCraft"), DropKind::Project);
        assert_eq!(k("/a/b.SVGZ"), DropKind::Import);
        assert_eq!(k("/a/b.jpeg"), DropKind::Import);
        assert_eq!(k("/a/b.pdf"), DropKind::Unsupported);
        assert_eq!(k("/a/noext"), DropKind::Unsupported);
        let text = unsupported_text(&PathBuf::from("/a/b.pdf"));
        assert!(
            text.contains("b.pdf") && !text.contains('\u{2014}'),
            "{text}"
        );
    }

    #[test]
    fn a_change_is_traced_after_the_debounce() {
        let (tx, rx) = mpsc::channel::<bool>();
        let tx = std::sync::Mutex::new(tx);
        let tracer: Tracer = Arc::new(move |_, s, _| {
            let _ = tx.lock().unwrap().send(s.invert);
            Err(ImportError::Parse("not traced in this test".into()))
        });
        let ctx = egui::Context::default();
        let src = TraceSource {
            doc_id: 1,
            id: 1,
            name: "x".into(),
            bytes: Vec::new(),
            settings: TraceSettings::default(),
            blade_offset_mm: 0.25,
        };
        let mut d = TraceDialog::open(&ctx, src, tracer);
        // The first preview goes out at once.
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(false));
        d.settings.invert = true;
        d.changed(10.0);
        d.poll(&ctx, 10.15);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        d.poll(&ctx, 10.2);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(true));
        assert!(d.busy());
    }
}
