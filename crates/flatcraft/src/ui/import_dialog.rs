//! Import (file dialog and drag-and-drop) and the Trace dialog.
//!
//! The Trace dialog takes the image bytes (shared, not copied) and the
//! trace settings of one raster object when it opens, under one short lock. Previews are traced
//! on a worker thread that never touches the session: a change waits
//! [`DEBOUNCE_S`] for the next one, then goes out as a job with a new
//! generation number, and results of older generations are dropped. Apply
//! sends one `object.retrace` to the object's document.

use super::tools::Setting;
use crate::ui::metrics;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use egui::{Color32, Pos2, Rect, Sense, Stroke, Ui, vec2};
use flatcraft_core::document::TraceSettings;
use flatcraft_core::geometry::CutPath;
use flatcraft_core::numexpr::{Field, Kind};
use flatcraft_engine::commands::sources::{FileKind, IMPORT_TYPES, is_importable};
use flatcraft_import::ImportError;
use flatcraft_import::trace::TraceResult;
use serde_json::json;

use super::calc::{self, Number};
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

/// File dialog filters for the import types of `kind` (all when `None`):
/// one filter with all of them, then one per type.
pub fn import_filters(all_name: &str, kind: Option<FileKind>) -> Vec<(String, Vec<String>)> {
    let types: Vec<_> = IMPORT_TYPES
        .iter()
        .filter(|t| kind.is_none_or(|k| t.kind == k))
        .collect();
    let all: Vec<&str> = types.iter().flat_map(|t| t.extensions).copied().collect();
    let mut filters = vec![filter(all_name, &all)];
    if types.len() > 1 {
        filters.extend(types.iter().map(|t| filter(t.name, t.extensions)));
    }
    filters
}

/// The Import file dialog: everything FlatCraft imports, then each type.
pub fn import_request() -> FileRequest {
    FileRequest {
        title: "Import".into(),
        filters: import_filters("SVG or image", None),
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
    } else if is_importable(path) {
        DropKind::Import
    } else {
        DropKind::Unsupported
    }
}

/// The toast for a dropped file FlatCraft cannot use.
pub fn unsupported_text(path: &Path) -> String {
    let names: Vec<&str> = IMPORT_TYPES.iter().map(|t| t.name).collect();
    let types = match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => names.join(""),
    };
    format!(
        "Cannot import {}. Drop an {types} file, or a .flatcraft project.",
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
    /// Specks removed by despeckle.
    pub despeckled: usize,
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
        despeckled: t.despeckled,
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

/// What the worker needs besides its channels.
struct WorkerInput {
    bytes: Arc<[u8]>,
    blade_offset_mm: f64,
    tracer: Tracer,
    /// Set when the dialog closes: nothing more is decoded or traced.
    cancel: Arc<AtomicBool>,
    ctx: egui::Context,
}

/// Runs preview jobs until the dialog closes. Only the newest waiting job
/// is traced; the others are already stale. The thumbnail is decoded after
/// the first trace, so the first outline is not kept waiting.
fn worker(input: WorkerInput, jobs: mpsc::Receiver<Job>, replies: mpsc::Sender<Reply>) {
    let WorkerInput {
        bytes,
        blade_offset_mm,
        tracer,
        cancel,
        ctx,
    } = input;
    let cancelled = || cancel.load(Ordering::Relaxed);
    let mut thumbnail_sent = false;
    while let Ok(mut job) = jobs.recv() {
        while let Ok(newer) = jobs.try_recv() {
            job = newer;
        }
        if cancelled() {
            return;
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
        // The outline shows now, before the thumbnail decode below ends.
        ctx.request_repaint();
        if !thumbnail_sent && !cancelled() {
            thumbnail_sent = true;
            if let Some(image) = thumbnail(&bytes) {
                if replies.send(Reply::Thumbnail(image)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
    }
}

/// What identifies the image and settings a dialog was opened for.
///
/// The engine has no revision counter, so this is checked every frame
/// (under the snapshot's lock) as cheaply as possible:
/// - settings and byte length are compared every frame (a few words);
/// - the object's geometry `Arc` is held, so its address cannot be reused.
///   Every retrace, reload and relink replaces it, and an undo restores
///   another one. While it is the same `Arc`, the bytes are the same;
/// - only when it changed and the length matches are the bytes hashed
///   again (once per change, about 30 ms for a 50 MB image). Equal bytes
///   (an undo of something else) keep the dialog open.
#[derive(Debug, Clone)]
pub struct SourceRev {
    geometry: Arc<Vec<CutPath>>,
    len: usize,
    hash: u64,
    settings: TraceSettings,
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

impl SourceRev {
    pub fn of(bytes: &[u8], settings: &TraceSettings, geometry: &Arc<Vec<CutPath>>) -> Self {
        Self {
            geometry: geometry.clone(),
            len: bytes.len(),
            hash: hash_bytes(bytes),
            settings: settings.clone(),
        }
    }

    /// True if these are not the bytes and settings of the revision.
    pub fn changed(
        &mut self,
        bytes: &[u8],
        settings: &TraceSettings,
        geometry: &Arc<Vec<CutPath>>,
    ) -> bool {
        if *settings != self.settings || bytes.len() != self.len {
            return true;
        }
        if !Arc::ptr_eq(geometry, &self.geometry) {
            if hash_bytes(bytes) != self.hash {
                return true;
            }
            self.geometry = geometry.clone();
        }
        false
    }
}

// ---------------------------------------------------------------- dialog

/// What the person opened the dialog with.
pub struct TraceSource {
    pub doc_id: u64,
    pub id: u64,
    pub name: String,
    pub bytes: Arc<[u8]>,
    pub settings: TraceSettings,
    pub blade_offset_mm: f64,
    /// Taken from the object's own bytes, not the copy.
    pub rev: SourceRev,
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
    /// The object's source when the dialog opened.
    pub rev: SourceRev,
    jobs: mpsc::Sender<Job>,
    replies: mpsc::Receiver<Reply>,
    cancel: Arc<AtomicBool>,
}

impl Drop for TraceDialog {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
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
        let cancel = Arc::new(AtomicBool::new(false));
        let input = WorkerInput {
            bytes: src.bytes,
            blade_offset_mm: src.blade_offset_mm,
            tracer,
            cancel: cancel.clone(),
            ctx: ctx.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("trace-preview".into())
            .spawn(move || worker(input, job_rx, reply_tx));
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
            rev: src.rev,
            jobs,
            replies,
            cancel,
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
        let specks = match p.despeckled {
            1 => "1 speck removed".to_string(),
            n => format!("{n} specks removed"),
        };
        let dropped = match p.dropped_small {
            1 => "1 small shape dropped".to_string(),
            n => format!("{n} small shapes dropped"),
        };
        let updating = if self.busy() { " (updating...)" } else { "" };
        format!(
            "Threshold {}{how}, {specks}, {dropped}{updating}",
            p.threshold_used
        )
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
                            // The greyed slider shows what Otsu picked, and
                            // unchecking Auto starts from it.
                            if self.settings.threshold.is_none() {
                                self.manual_threshold = p.threshold_used;
                            }
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
    if snap.trace_source_changed {
        *dialog = None;
        return Some("The image changed, open Trace again.");
    }
    let now = ctx.input(|i| i.time);
    d.poll(ctx, now);
    let mut outcome = Outcome::Open;
    // Esc in a number field only puts its value back.
    if !ctx.egui_wants_keyboard_input()
        && calc::editing(ctx).is_none()
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
                ui.add_space(metrics::SPACE_16);
                preview_box(ui, d, t);
            });
            ui.add_space(metrics::SPACE_8);
            let status = d.status();
            let colour = if d.error.is_some() {
                t.danger
            } else {
                t.text_dim
            };
            ui.label(egui::RichText::new(status).color(colour));
            // Linked copies share the trace settings: say that Apply
            // retraces them all, though the preview shows one.
            if let Some(n) = snap
                .object(d.id)
                .and_then(|o| o.link_group)
                .map(|g| snap.group_size(g))
            {
                ui.label(
                    egui::RichText::new(format!("Retraces all {n} linked copies"))
                        .color(t.text_dim),
                );
            }
            ui.add_space(metrics::SPACE_8);
            ui.horizontal(|ui| {
                let apply = ui
                    .button("Apply")
                    .on_hover_text("Trace the image with these settings");
                if calc::click(ui, &apply) {
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

/// A slider, a number field for its value (a calculation works, a value
/// outside `range` is refused) and its label, with hover cards. The slider
/// is named `label`, the field "`label` value". True when the value
/// changed.
fn slider_row<T: egui::emath::Numeric>(
    ui: &mut Ui,
    enabled: bool,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    spec: Number,
    label: &str,
    setting: Setting,
) -> bool {
    let kind = spec.field.kind;
    let (track, field, changed) =
        calc::slider(ui, ("flatcraft-trace", label), enabled, value, range, &spec);
    set_name(ui, &track, label);
    set_name(ui, &field, &format!("{label} value"));
    let card = calc::with_hint(setting.card(), kind);
    super::groups::card(&track, label, setting.card(), None, false);
    super::groups::card(&field, label, &card, None, false);
    let l = ui.label(label);
    super::groups::card(&l, label, setting.card(), None, false);
    changed
}

/// The settings. Returns true when one changed.
fn controls(ui: &mut Ui, d: &mut TraceDialog) -> bool {
    let mut changed = false;
    let s = &mut d.settings;

    ui.horizontal(|ui| {
        let mut auto = s.threshold.is_none();
        let threshold = slider_row(
            ui,
            !auto,
            &mut d.manual_threshold,
            0..=255,
            Number::new(Field::plain()),
            "Threshold",
            Setting::Threshold,
        );
        if threshold {
            s.threshold = Some(d.manual_threshold);
            changed = true;
        }
        let r = ui.checkbox(&mut auto, "Auto");
        super::groups::card(&r, "Auto", Setting::AutoThreshold.card(), None, false);
        set_name(ui, &r, "Automatic threshold");
        if r.changed() {
            s.threshold = (!auto).then_some(d.manual_threshold);
            changed = true;
        }
    });
    let r = ui.checkbox(&mut s.invert, "Invert");
    super::groups::card(&r, "Invert", Setting::Invert.card(), None, false);
    changed |= r.changed();
    ui.horizontal(|ui| {
        let (r, typed) = calc::number(
            ui,
            "flatcraft-trace-despeckle",
            true,
            &mut s.despeckle_mm2,
            &Number::new(Field::plain().range(0.0, 100.0)).unit("mm²"),
        );
        set_name(ui, &r, "Despeckle (mm²)");
        let card = calc::with_hint(Setting::Despeckle.card(), Kind::Plain);
        super::groups::card(&r, "Despeckle", &card, None, false);
        changed |= typed;
        let l = ui.label("Despeckle");
        super::groups::card(&l, "Despeckle", Setting::Despeckle.card(), None, false);
    });
    changed |= ui
        .horizontal(|ui| {
            slider_row(
                ui,
                true,
                &mut s.corner_threshold_deg,
                0.0..=180.0,
                Number::new(Field::new(Kind::Angle)).unit("deg"),
                "Corner threshold",
                Setting::CornerThreshold,
            )
        })
        .inner;
    changed |= ui
        .horizontal(|ui| {
            slider_row(
                ui,
                true,
                &mut s.smoothing,
                0..=10,
                Number::new(Field::plain()),
                "Smoothing",
                Setting::Smoothing,
            )
        })
        .inner;
    ui.horizontal(|ui| {
        let mut auto = s.min_feature_mm.is_none();
        let (r, typed) = calc::number(
            ui,
            "flatcraft-trace-min-feature",
            !auto,
            &mut d.manual_min_feature,
            &Number::new(Field::length().range(0.0, 50.0)).unit("mm"),
        );
        set_name(ui, &r, "Minimum feature (mm)");
        if typed {
            s.min_feature_mm = Some(d.manual_min_feature);
            changed = true;
        }
        let card = calc::with_hint(Setting::MinFeature.card(), Kind::Length);
        super::groups::card(&r, "Minimum feature", &card, None, false);
        let l = ui.label("Minimum feature");
        super::groups::card(
            &l,
            "Minimum feature",
            Setting::MinFeature.card(),
            None,
            false,
        );
        let r = ui.checkbox(&mut auto, "Auto");
        super::groups::card(&r, "Auto", Setting::AutoMinFeature.card(), None, false);
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
            bytes: Arc::from(Vec::new()),
            settings: TraceSettings::default(),
            blade_offset_mm: 0.25,
            rev: SourceRev::of(&[], &TraceSettings::default(), &Arc::default()),
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

    #[test]
    fn a_closed_dialog_traces_nothing_more() {
        let (started_tx, started_rx) = mpsc::channel::<bool>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let started_tx = std::sync::Mutex::new(started_tx);
        let release_rx = std::sync::Mutex::new(release_rx);
        let tracer: Tracer = Arc::new(move |_, s, _| {
            let _ = started_tx.lock().unwrap().send(s.invert);
            let _ = release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
            Err(ImportError::Parse("not traced in this test".into()))
        });
        let ctx = egui::Context::default();
        let src = TraceSource {
            doc_id: 1,
            id: 1,
            name: "x".into(),
            bytes: Arc::from(Vec::new()),
            settings: TraceSettings::default(),
            blade_offset_mm: 0.25,
            rev: SourceRev::of(&[], &TraceSettings::default(), &Arc::default()),
        };
        let mut d = TraceDialog::open(&ctx, src, tracer);
        assert_eq!(started_rx.recv_timeout(Duration::from_secs(5)), Ok(false));
        // A change is queued, then the dialog closes before it is traced.
        d.settings.invert = true;
        d.changed(0.0);
        d.poll(&ctx, 1.0);
        drop(d);
        release_tx.send(()).unwrap();
        assert!(started_rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    fn geometry() -> Arc<Vec<flatcraft_core::geometry::CutPath>> {
        Arc::new(Vec::new())
    }

    #[test]
    fn changed_bytes_of_the_same_length_close_the_dialog() {
        let s = TraceSettings::default();
        let g = geometry();
        let mut rev = SourceRev::of(&[1, 2, 3], &s, &g);
        assert!(!rev.changed(&[1, 2, 3], &s, &g));
        // A reload with other bytes of the same length: new geometry too.
        assert!(rev.changed(&[1, 2, 4], &s, &geometry()));
        let other = TraceSettings {
            invert: true,
            ..TraceSettings::default()
        };
        assert!(rev.changed(&[1, 2, 3], &other, &g));
    }

    #[test]
    fn the_same_image_in_a_new_place_keeps_the_dialog() {
        let s = TraceSettings::default();
        let mut rev = SourceRev::of(&[1, 2, 3], &s, &geometry());
        // An undo restores an equal copy: new buffers, same content.
        let copy = vec![1u8, 2, 3];
        let g = geometry();
        assert!(!rev.changed(&copy, &s, &g));
        assert!(!rev.changed(&copy, &s, &g));
    }
}
