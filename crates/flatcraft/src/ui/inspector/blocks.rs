//! The building blocks of the side panels: the job line, a section that
//! folds to a one-line summary, a row with its change dot and reset arrow,
//! and a hint card. Every panel is made of these, so they look and behave
//! the same everywhere.

use std::collections::BTreeMap;

use egui::{Align, Color32, FontId, Id, Layout, Rect, Response, RichText, Sense, Ui, vec2};

use super::super::groups;
use super::super::icons::{self, Icon};
use super::super::rails::Dot;
use super::super::settings::set_name;
use super::super::theme::Tokens;
use super::super::{Action, metrics};

// ---------------------------------------------------------------- job line

/// What the job line says: plotter, material, mat and strategy, and the
/// state dot of the job.
#[derive(Debug, Clone, PartialEq)]
pub struct JobSummary {
    pub plotter: String,
    /// A shorter name of the plotter for the line, when its preset has
    /// one (the model of a USB plotter, "Cameo 2").
    pub plotter_short: Option<String>,
    pub material: String,
    pub mat: String,
    pub strategy: String,
    pub dot: Dot,
    /// Objects are selected: a click shows the Job panel over them.
    pub selection: bool,
    /// The Job panel shows over a selection: a click goes back.
    pub open: bool,
}

impl JobSummary {
    /// "Cameo 2 · Vinyl (permanent) · 12x12 · Quality".
    pub fn text(&self) -> String {
        [&self.plotter, &self.material, &self.mat, &self.strategy]
            .map(String::as_str)
            .join(" · ")
    }

    /// The accessible name of the job line.
    pub fn name(&self) -> String {
        format!("Job: {}", self.text())
    }
}

/// The card of the job line.
pub fn job_line_card(s: &JobSummary) -> String {
    let back = if s.open {
        "You see the Job panel over your selection now. Click again, or press Esc, to go back to the selection."
    } else if s.selection {
        "Click it to see only the job, over your selection. Click again, or press Esc, to go back to the selection."
    } else {
        "Nothing is selected, so the Job panel is right below. Select objects to choose what is cut of them; then a click here shows the job over them."
    };
    format!(
        "What this cut uses: {}. {}. The job is the plotter, the mat, the material and its cut values, and the order of the cut. {back}",
        s.text(),
        s.dot.label()
    )
}

/// The height of the job line: one panel row.
pub const JOB_LINE_H: f32 = metrics::ROW;

/// What the job line did this frame.
pub struct JobLine {
    /// The job part (the dot, the summary and its chevron).
    pub job: Response,
    /// The panel's collapse button at its right end.
    pub collapse: Response,
}

/// The job line: one bordered element a row high. On the left the job
/// part, a button of its own: the state dot, the summary on one line (cut
/// short with "...", the card holds all of it) and a chevron. On the right,
/// after a thin divider, the panel's collapse button.
pub fn job_line(
    ui: &mut Ui,
    s: &JobSummary,
    t: &Tokens,
    collapse_name: &str,
    collapse_card: &str,
) -> JobLine {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, JOB_LINE_H), Sense::hover());
    let side = metrics::ICON_BUTTON + metrics::SPACE_4;
    let job_rect = Rect::from_min_max(rect.min, egui::pos2(rect.right() - side, rect.bottom()));
    let fold_rect = Rect::from_min_max(egui::pos2(rect.right() - side, rect.top()), rect.max);
    // With nothing selected the Job panel is already below: the line only
    // shows, a click does nothing.
    let sense = if s.selection {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(job_rect, Id::unique("flatcraft-job-line"), sense);
    let name = s.name();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.as_str()));
    set_name(ui, &resp, &name);
    let fold = ui.interact(
        fold_rect,
        Id::unique("flatcraft-job-line-fold"),
        Sense::click(),
    );
    fold.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, collapse_name));
    set_name(ui, &fold, collapse_name);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = resp.hovered() && s.selection;
        let border = if s.open {
            t.accent_prepare
        } else if hovered || fold.hovered() {
            t.text_dim
        } else {
            t.border
        };
        p.rect(
            rect.shrink(0.5),
            t.radius,
            t.panel,
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        // Hover is a light fill on the part under the pointer.
        if hovered {
            p.rect_filled(
                job_rect.shrink(1.0),
                egui::CornerRadius {
                    nw: t.radius,
                    sw: t.radius,
                    ne: 0,
                    se: 0,
                },
                t.surface,
            );
        }
        if fold.hovered() {
            p.rect_filled(
                fold_rect.shrink(1.0),
                egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: t.radius,
                    se: t.radius,
                },
                t.surface,
            );
        }
        p.rect_stroke(
            rect.shrink(0.5),
            t.radius,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        p.vline(
            fold_rect.left(),
            rect.y_range().shrink(metrics::SPACE_4),
            egui::Stroke::new(1.0, t.border),
        );
        let dot = rect.left_center() + vec2(metrics::SPACE_12, 0.0);
        p.circle_filled(dot, 4.0, dot_colour(s.dot, t));
        // Down opens the Job view over the selection, left goes back. No
        // chevron with nothing selected (nothing to open).
        let size = metrics::ICON_SMALL;
        let mut right = job_rect.right() - metrics::SPACE_8;
        let icon = match (s.selection, s.open) {
            (false, _) => None,
            (true, false) => Some(Icon::ChevronDown),
            (true, true) => Some(Icon::ChevronLeft),
        };
        if let Some(icon) = icon {
            let tex = icons::texture(ui.ctx(), icon, size);
            let icon_rect = Rect::from_center_size(
                egui::pos2(right - size / 2.0, rect.center().y),
                vec2(size, size),
            );
            p.image(
                tex.id(),
                icon_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                t.text_dim,
            );
            right = icon_rect.left() - metrics::SPACE_4;
        }
        let left = dot.x + metrics::SPACE_8;
        line_parts(
            p,
            s,
            metrics::body(),
            t.text,
            egui::pos2(left, rect.center().y),
            (right - left).max(0.0),
        );
        let icon = metrics::ICON_COMPACT;
        let ink = if fold.hovered() { t.text } else { t.text_dim };
        let tex = icons::texture_colored(ui.ctx(), Icon::PanelRightClose, icon, ink);
        p.image(
            tex.id(),
            Rect::from_center_size(fold_rect.center(), vec2(icon, icon)),
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
    let resp = if s.selection {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };
    groups::card(&resp, "Job", &job_line_card(s), None, false);
    groups::card(&fold, collapse_name, collapse_card, None, false);
    JobLine {
        job: resp,
        collapse: fold,
    }
}

/// The separator between the parts of the job line.
const SEP: &str = " · ";

/// Paints the job line's parts from `at`, within `width`. The mat and the
/// strategy always show in full (they are short and they are what a
/// person checks after a change); the plotter and the material share the
/// rest, each cut short on its own with "...". Only when even the mat and
/// the strategy do not fit is the whole line cut short at its end.
fn line_parts(
    p: &egui::Painter,
    s: &JobSummary,
    font: FontId,
    color: Color32,
    at: egui::Pos2,
    width: f32,
) {
    let measure = |text: &str| {
        p.layout_no_wrap(text.to_string(), font.clone(), color)
            .size()
            .x
    };
    let plotter = s.plotter_short.as_deref().unwrap_or(&s.plotter);
    let sep = measure(SEP);
    let fixed = measure(&s.mat) + measure(&s.strategy) + 3.0 * sep;
    let room = width - fixed;
    let (pw, mw) = (measure(plotter), measure(&s.material));
    // A part that needs less than half gives the rest to the other.
    let (pw, mw) = if pw + mw <= room {
        (pw, mw)
    } else if pw <= room / 2.0 {
        (pw, room - pw)
    } else if mw <= room / 2.0 {
        (room - mw, mw)
    } else {
        (room / 2.0, room / 2.0)
    };
    // Below a few letters a part says nothing: the whole line is cut short
    // (a part that is short by itself, like "File", is fine).
    let need = |w: f32, full: f32| w < full.min(metrics::SPACE_24);
    if need(pw, measure(plotter)) || need(mw, measure(&s.material)) {
        one_line(
            p,
            &[plotter, &s.material, &s.mat, &s.strategy].join(SEP),
            font,
            color,
            at,
            width,
            Align::LEFT,
        );
        return;
    }
    let mut x = at.x;
    let parts = [
        (plotter, pw + 0.5),
        (SEP, sep + 0.5),
        (s.material.as_str(), mw + 0.5),
        (SEP, sep + 0.5),
        (s.mat.as_str(), f32::INFINITY),
        (SEP, sep + 0.5),
        (s.strategy.as_str(), f32::INFINITY),
    ];
    for (text, w) in parts {
        let r = one_line(
            p,
            text,
            font.clone(),
            color,
            egui::pos2(x, at.y),
            w,
            Align::LEFT,
        );
        x = r.right();
    }
}

/// The colour of a state dot.
pub fn dot_colour(d: Dot, t: &Tokens) -> Color32 {
    match d {
        Dot::Ready => t.ok,
        Dot::Warning => t.warn,
        Dot::Nothing => t.text_dim,
        Dot::Notes => t.accent_prepare,
    }
}

/// Paints `text` on one line, at most `width` wide (cut short with "..."),
/// vertically centred on `at` and aligned at its x as `align` says.
/// Returns the painted rect.
fn one_line(
    p: &egui::Painter,
    text: &str,
    font: FontId,
    color: Color32,
    at: egui::Pos2,
    width: f32,
    align: Align,
) -> Rect {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let galley = p.layout_job(job);
    let x = match align {
        Align::Max => at.x - galley.size().x,
        _ => at.x,
    };
    let pos = egui::pos2(x, at.y - galley.size().y / 2.0);
    let rect = Rect::from_min_size(pos, galley.size());
    p.galley(pos, galley, color);
    rect
}

// ---------------------------------------------------------------- sections

/// The open sections of the side panels, by id, as the person left them.
/// It lives in egui's memory so a section can read it while its body
/// borrows the panel state; the app seeds it from the preferences
/// ([`seed`]) and stores it back when it changed ([`take_changed`]).
#[derive(Debug, Clone, Default)]
struct Sections {
    open: BTreeMap<String, bool>,
    /// The ids opened or folded since the last [`take_changed`].
    changed: std::collections::BTreeSet<String>,
}

fn sections_id() -> Id {
    Id::unique("flatcraft-panel-sections")
}

/// Puts the stored open sections into `ctx`.
pub fn seed(ctx: &egui::Context, open: BTreeMap<String, bool>) {
    ctx.data_mut(|d| {
        d.insert_temp(
            sections_id(),
            Sections {
                open,
                changed: Default::default(),
            },
        );
    });
}

/// The sections opened or folded since the last call, with their new
/// state; `None` when there are none. Only these are stored, so another
/// window's choices for other sections are kept.
pub fn take_changed(ctx: &egui::Context) -> Option<BTreeMap<String, bool>> {
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<Sections>(sections_id());
        let changed = std::mem::take(&mut s.changed);
        (!changed.is_empty()).then(|| {
            changed
                .into_iter()
                .filter_map(|id| s.open.get(&id).map(|&o| (id, o)))
                .collect()
        })
    })
}

/// Whether the section `id` is open; `default` when the person never
/// opened or folded it.
pub fn is_open(ctx: &egui::Context, id: &str, default: bool) -> bool {
    ctx.data(|d| {
        d.get_temp::<Sections>(sections_id())
            .and_then(|s| s.open.get(id).copied())
            .unwrap_or(default)
    })
}

/// Opens or folds the section `id`.
pub fn set_open(ctx: &egui::Context, id: &str, open: bool) {
    ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_default::<Sections>(sections_id());
        s.open.insert(id.to_string(), open);
        s.changed.insert(id.to_string());
    });
}

/// The height of a section header, in points. Fixed, so a changed
/// summary never moves what is below it.
pub const HEADER_H: f32 = metrics::SECTION_HEADER;

/// The id of a section's Advanced part.
pub fn advanced_id(id: &str) -> String {
    format!("{id}.advanced")
}

/// A section: a header with the title and, while folded, a one-line
/// summary on the right, then the body while open. `card` says what the
/// section holds (its hover card). An `advanced` part shows folded at the
/// end of the body, under its own header. Returns whether it is open.
#[allow(clippy::too_many_arguments)]
pub fn section(
    ui: &mut Ui,
    id: &str,
    title: &str,
    card: &str,
    summary: &str,
    default_open: bool,
    body: impl FnOnce(&mut Ui),
    advanced: Option<&mut dyn FnMut(&mut Ui)>,
) -> bool {
    section_named(
        ui,
        id,
        title,
        title,
        card,
        summary,
        default_open,
        body,
        advanced,
    )
}

/// [`section`] whose header has the accessible name `name` instead of its
/// title, for a title another control of the window already has (the
/// ribbon's Arrange group and the object inspector's Arrange section).
#[allow(clippy::too_many_arguments)]
pub fn section_named(
    ui: &mut Ui,
    id: &str,
    title: &str,
    name: &str,
    card: &str,
    summary: &str,
    default_open: bool,
    body: impl FnOnce(&mut Ui),
    advanced: Option<&mut dyn FnMut(&mut Ui)>,
) -> bool {
    section_kind(
        ui,
        id,
        title,
        name,
        card,
        summary,
        default_open,
        Kind::Section,
        body,
        advanced,
    )
}

/// [`section`] whose header says `live` while it is open: a state that
/// comes and goes ("checking...") sits in the header's one line, so the
/// body never changes height for it. Folded, the header shows `summary`
/// as usual. The live text is also a label for screen readers.
#[allow(clippy::too_many_arguments)]
pub fn section_live(
    ui: &mut Ui,
    id: &str,
    title: &str,
    card: &str,
    summary: &str,
    live: &str,
    default_open: bool,
    body: impl FnOnce(&mut Ui),
) -> bool {
    section_kind(
        ui,
        id,
        title,
        title,
        card,
        summary,
        default_open,
        Kind::Live(live),
        body,
        None,
    )
}

/// What a header is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind<'a> {
    /// A section's header: the summary shows while folded.
    Section,
    /// A section's header that shows this text while open.
    Live(&'a str),
    /// The header of a section's Advanced part.
    Sub,
}

#[allow(clippy::too_many_arguments)]
fn section_kind(
    ui: &mut Ui,
    id: &str,
    title: &str,
    name: &str,
    card: &str,
    summary: &str,
    default_open: bool,
    kind: Kind<'_>,
    body: impl FnOnce(&mut Ui),
    advanced: Option<&mut dyn FnMut(&mut Ui)>,
) -> bool {
    let open = header(ui, id, title, name, card, summary, default_open, kind);
    if open {
        clear_labels(ui);
        body(ui);
        if let Some(advanced) = advanced {
            let id = advanced_id(id);
            let card = format!(
                "Settings of {title} you rarely need. They are folded so the common ones stay in view."
            );
            if header(ui, &id, "Advanced", "Advanced", &card, "", false, Kind::Sub) {
                advanced(ui);
            }
        }
        clear_labels(ui);
    }
    open
}

/// The header of a section (or of its Advanced part, `sub`). Returns
/// whether the section is open after this frame's click.
#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut Ui,
    id: &str,
    title: &str,
    name: &str,
    card: &str,
    summary: &str,
    default_open: bool,
    kind: Kind<'_>,
) -> bool {
    let open = is_open(ui.ctx(), id, default_open);
    let open = header_row(ui, title, name, card, summary, open, kind);
    if open != is_open(ui.ctx(), id, default_open) {
        set_open(ui.ctx(), id, open);
    }
    open
}

/// Draws a header with the state `open` and returns it after a click.
fn header_row(
    ui: &mut Ui,
    title: &str,
    name: &str,
    card: &str,
    summary: &str,
    mut open: bool,
    kind: Kind<'_>,
) -> bool {
    let sub = kind == Kind::Sub;
    let w = ui.available_width();
    let h = if sub { metrics::SUB_HEADER } else { HEADER_H };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if resp.clicked() {
        open = !open;
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    set_name(ui, &resp, name);
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.set_expanded(open);
    });
    if ui.is_rect_visible(rect) {
        let tk = tokens(ui);
        let (ink, dim, rule) = (tk.text, tk.text_dim, tk.border);
        let p = ui.painter();
        if !sub {
            p.hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, rule));
        }
        // The caret: right while folded, down while open.
        let c = egui::pos2(rect.left() + 5.0, rect.center().y);
        let s = 3.5;
        let pts = if open {
            vec![
                c + vec2(-s, -s * 0.6),
                c + vec2(s, -s * 0.6),
                c + vec2(0.0, s * 0.8),
            ]
        } else {
            vec![
                c + vec2(-s * 0.6, -s),
                c + vec2(-s * 0.6, s),
                c + vec2(s * 0.8, 0.0),
            ]
        };
        let caret = if resp.hovered() { ink } else { dim };
        p.add(egui::Shape::convex_polygon(pts, caret, egui::Stroke::NONE));
        let font = if sub {
            metrics::subsection_title()
        } else {
            metrics::section_title()
        };
        let color = if sub { dim } else { ink };
        let left = rect.left() + metrics::CARET_SLOT;
        let t = one_line(
            p,
            title,
            font,
            color,
            egui::pos2(left, rect.center().y),
            (rect.right() - left).max(0.0),
            Align::LEFT,
        );
        let (shown, live) = match (open, kind) {
            (false, _) => (summary, false),
            (true, Kind::Live(l)) => (l, true),
            (true, _) => ("", false),
        };
        if !shown.is_empty() {
            let room = rect.right() - t.right() - metrics::SPACE_16;
            if room > metrics::SPACE_12 {
                let r = one_line(
                    p,
                    shown,
                    metrics::secondary(),
                    dim,
                    egui::pos2(rect.right(), rect.center().y),
                    room,
                    Align::Max,
                );
                if live {
                    let node = ui.interact(r, resp.id.with("summary"), Sense::hover());
                    node.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, shown));
                    set_name(ui, &node, shown);
                }
            }
        }
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let what = if open {
        "Click to fold it."
    } else {
        "Click to open it."
    };
    let text = if summary.is_empty() {
        format!("{card} {what}")
    } else {
        format!("{card}\n\nNow: {summary}. {what}")
    };
    groups::card(&resp, title, &text, None, false);
    open
}

/// The tokens of the theme `ui` draws in.
fn tokens(ui: &Ui) -> Tokens {
    if ui.visuals().dark_mode {
        Tokens::dark()
    } else {
        Tokens::light()
    }
}

/// A line that is always there, one row high, with `text` (maybe empty)
/// on it: a state that comes and goes ("Checking...") never moves what
/// is below it. The text is cut short on its one line.
pub fn status_line(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let h = metrics::ROW;
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    if !text.is_empty() {
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
        set_name(ui, &resp, text);
        one_line(
            ui.painter(),
            text,
            metrics::secondary(),
            color,
            egui::pos2(rect.left(), rect.center().y),
            w,
            Align::LEFT,
        );
    }
    resp
}

/// A short dimmed line inside a section (an explanation, a "why").
pub fn note(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(metrics::TEXT_SECONDARY)
                .color(color),
        )
        .wrap(),
    )
}

// ---------------------------------------------------------------- rows

/// Where a value comes from, for the change dot: the source's name (for
/// example "the material Vinyl (permanent)"), its value, whether the
/// shown value differs from it, and the action that puts it back.
#[derive(Debug, Clone)]
pub struct SourceValue {
    pub label: String,
    pub value: String,
    pub changed: bool,
    pub reset: Action,
}

/// What a row did this frame.
pub struct RowResponse {
    /// The value widget.
    pub widget: Response,
    /// The reset arrow was clicked: run this.
    pub reset: Option<Action>,
}

/// The width of the change dot's slot.
const DOT_W: f32 = metrics::DOT_SLOT;

fn label_column_id(ui: &Ui) -> Id {
    ui.scope_id().with("flatcraft-label-column")
}

/// Sets the label column of the rows that follow in this section: as
/// wide as the longest of `labels` (within `metrics::LABEL_MIN` and
/// `LABEL_MAX`). Every section calls it first with all its labels, so
/// its rows line up and the column never changes from frame to frame.
pub fn labels(ui: &Ui, labels: &[&str]) {
    let font = metrics::body();
    let widest = labels
        .iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap((*l).to_string(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let w = metrics::label_column(widest);
    let id = label_column_id(ui);
    ui.ctx().data_mut(|d| d.insert_temp(id, w));
}

/// Forgets the label column (a section's end).
fn clear_labels(ui: &Ui) {
    let id = label_column_id(ui);
    ui.ctx().data_mut(|d| d.remove::<f32>(id));
}

/// The label column of a row labelled `label`: the section's, or one that
/// fits this label alone.
pub fn label_width(ui: &Ui, label: &str) -> f32 {
    let id = label_column_id(ui);
    ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or_else(|| {
        let w = ui
            .painter()
            .layout_no_wrap(label.to_string(), metrics::body(), Color32::PLACEHOLDER)
            .size()
            .x;
        metrics::label_column(w)
    })
}

/// Sets the panel spacing on `ui`: controls [`metrics::CONTROL`] tall and
/// no automatic room between widgets (rows and headers carry their own).
pub fn panel_spacing(ui: &mut Ui) {
    let s = ui.spacing_mut();
    s.interact_size.y = metrics::CONTROL;
    s.button_padding.y = metrics::SPACE_2;
    s.item_spacing.y = 0.0;
}

/// A row: the dimmed label, the change dot, the value widget and the
/// reset arrow. The widget gets the row's name (with "(modified)" while
/// the value differs from its source) and the hover card `card`, plus a
/// sentence on the source.
pub fn row(
    ui: &mut Ui,
    label: &str,
    card: &str,
    source: Option<SourceValue>,
    widget: impl FnOnce(&mut Ui) -> Response,
) -> RowResponse {
    row_dimmed(ui, label, card, None, source, widget)
}

/// [`row`] for a widget that may be dimmed: `why` says why and what to do
/// (the widget itself draws dimmed).
pub fn row_dimmed(
    ui: &mut Ui,
    label: &str,
    card: &str,
    why: Option<&str>,
    source: Option<SourceValue>,
    widget: impl FnOnce(&mut Ui) -> Response,
) -> RowResponse {
    let h = metrics::CONTROL;
    let w = ui.available_width();
    let changed = source.as_ref().is_some_and(|s| s.changed);
    let reset_w = h + metrics::SPACE_4;
    let label_w = label_width(ui, label);
    let mut reset = None;
    let mut out = None;
    let card = match &source {
        Some(s) if s.changed => format!(
            "{card} Changed: {} says {}. The orange dot marks a changed value, and the arrow puts {} back.",
            s.label, s.value, s.value
        ),
        Some(s) => format!("{card} This is the value of {}.", s.label),
        None => card.to_string(),
    };
    let (row_rect, _) = ui.allocate_exact_size(vec2(w, metrics::ROW), Sense::hover());
    let builder = egui::UiBuilder::new()
        .max_rect(row_rect)
        .layout(Layout::left_to_right(Align::Center));
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (lr, lresp) = ui.allocate_exact_size(vec2(label_w, h), Sense::hover());
        let dim = tokens(ui).text_dim;
        one_line(
            ui.painter(),
            label,
            metrics::body(),
            dim,
            egui::pos2(lr.left(), lr.center().y),
            lr.width() - metrics::SPACE_4,
            Align::LEFT,
        );
        groups::card(&lresp, label, &card, why, false);
        let (dr, dresp) = ui.allocate_exact_size(vec2(DOT_W, h), Sense::hover());
        if changed {
            let color = tokens(ui).changed;
            ui.painter().circle_filled(dr.center(), 3.5, color);
            if let Some(s) = &source {
                groups::card(
                    &dresp,
                    "Changed",
                    &format!("{label} differs from {}, which says {}.", s.label, s.value),
                    None,
                    false,
                );
            }
        }
        let ww = (ui.available_width() - reset_w).max(metrics::CONTROL);
        let r = ui
            .allocate_ui_with_layout(vec2(ww, h), Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = metrics::SPACE_4;
                widget(ui)
            })
            .inner;
        let name = if changed {
            format!("{label} (modified)")
        } else {
            label.to_string()
        };
        set_name(ui, &r, &name);
        groups::card(&r, label, &card, why, false);
        ui.add_space(metrics::SPACE_4);
        match &source {
            Some(s) if s.changed => {
                let name = format!("Reset {}", label.to_lowercase());
                let b = reset_button(ui, &name);
                groups::card(
                    &b,
                    &name,
                    &format!("Puts back {} from {}. One step to undo.", s.value, s.label),
                    None,
                    false,
                );
                if b.clicked() {
                    reset = Some(s.reset.clone());
                }
            }
            _ => {
                ui.allocate_exact_size(vec2(h, h), Sense::hover());
            }
        }
        out = Some(r);
    });
    RowResponse {
        widget: out.expect("the row draws its widget"),
        reset,
    }
}

/// A row of buttons (or links) after the dimmed label, for actions that
/// belong together ("Align": six small buttons). The buttons keep their
/// own names and cards; `card` is the label's. They wrap to more lines
/// when the panel is narrow. An empty label leaves the label column empty.
pub fn actions_row(ui: &mut Ui, label: &str, card: &str, add: impl FnOnce(&mut Ui)) {
    // Each line of buttons is a row: the buttons sit in its middle.
    let pad = (metrics::ROW - metrics::ICON_BUTTON) / 2.0;
    let label_w = label_width(ui, label);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (lr, lresp) =
            ui.allocate_exact_size(vec2(label_w + DOT_W, metrics::ROW), Sense::hover());
        if !label.is_empty() {
            let dim = tokens(ui).text_dim;
            one_line(
                ui.painter(),
                label,
                metrics::body(),
                dim,
                egui::pos2(lr.left(), lr.center().y),
                label_w - metrics::SPACE_4,
                Align::LEFT,
            );
            groups::card(&lresp, label, card, None, false);
        }
        ui.vertical(|ui| {
            ui.add_space(pad);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(metrics::SPACE_4, 2.0 * pad);
                ui.spacing_mut().interact_size.y = metrics::ICON_BUTTON;
                add(ui);
            });
            ui.add_space(pad);
        });
    });
}

/// The height of the heading under the job line: one panel row.
pub const HEADING_H: f32 = metrics::HEADING;

/// The heading under the job line, on one line: what the panel shows
/// ("2 objects", "Group 'Badge'"), then dimmed what the panel shows
/// nowhere else ("· in group Badge", "· 1 of 3 linked copies"), cut short
/// with "..." when it does not fit. Always one row tall, so nothing below
/// it moves when the selection changes; its card holds the whole text.
pub fn heading(ui: &mut Ui, title: &str, subtitle: &str, card: &str) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, HEADING_H), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, title));
    set_name(ui, &resp, title);
    if ui.is_rect_visible(rect) {
        let tk = tokens(ui);
        let p = ui.painter();
        let y = rect.center().y;
        let t = one_line(
            p,
            title,
            metrics::heading(),
            tk.text,
            egui::pos2(rect.left(), y),
            w,
            Align::LEFT,
        );
        if !subtitle.is_empty() {
            let sep = one_line(
                p,
                SEP,
                metrics::secondary(),
                tk.text_dim,
                egui::pos2(t.right(), y),
                (rect.right() - t.right()).max(0.0),
                Align::LEFT,
            );
            let room = rect.right() - sep.right();
            if room > metrics::SPACE_24 {
                one_line(
                    p,
                    subtitle,
                    metrics::secondary(),
                    tk.text_dim,
                    egui::pos2(sep.right(), y),
                    room,
                    Align::LEFT,
                );
            }
        }
    }
    let text = if subtitle.is_empty() {
        card.to_string()
    } else {
        format!("{title} · {subtitle}.\n\n{card}")
    };
    groups::card(&resp, title, &text, None, false);
    resp
}

/// The small reset arrow of a changed row.
fn reset_button(ui: &mut Ui, name: &str) -> Response {
    let size = metrics::ICON_COMPACT;
    let tex = icons::texture(ui.ctx(), Icon::RotateLeft, size);
    let image =
        egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), vec2(size, size)))
            .alt_text(name);
    let h = metrics::CONTROL;
    let r = ui.add_sized(vec2(h, h), egui::Button::image(image).frame(false));
    set_name(ui, &r, name);
    r
}

/// A row that only shows a value (one that is changed elsewhere, for
/// example in the presets). The value is painted on one line; for screen
/// readers it is a read-only field named after the row.
pub fn value_row(ui: &mut Ui, label: &str, card: &str, value: &str) -> Response {
    row(ui, label, card, None, |ui| {
        let size = vec2(ui.available_width(), metrics::CONTROL);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
        let ink = tokens(ui).text;
        one_line(
            ui.painter(),
            value,
            metrics::body(),
            ink,
            egui::pos2(rect.left() + metrics::SPACE_4, rect.center().y),
            rect.width() - metrics::SPACE_4,
            Align::LEFT,
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
        ui.ctx().accesskit_node_builder(resp.id, |node| {
            node.set_read_only();
            node.set_value(value);
        });
        resp
    })
    .widget
}

// ---------------------------------------------------------------- hint cards

/// A button of a hint card: its text, its hover card and why it is
/// dimmed, if it is.
#[derive(Debug, Clone, Copy)]
pub struct HintButton<'a> {
    pub label: &'a str,
    pub card: &'a str,
    pub why: Option<&'a str>,
}

/// The tips the person hid with the X of their hint card, by id. Like
/// the open sections it lives in egui's memory: the app seeds it from
/// `Preferences.hidden_tips` ([`seed_tips`]) and stores what was hidden
/// ([`take_hidden_tips`]). The Settings window shows them again.
#[derive(Debug, Clone, Default)]
struct Tips {
    hidden: std::collections::BTreeSet<String>,
    /// Hidden since the last [`take_hidden_tips`].
    new: std::collections::BTreeSet<String>,
}

fn tips_id() -> Id {
    Id::unique("flatcraft-hidden-tips")
}

/// Puts the stored hidden tips into `ctx`.
pub fn seed_tips(ctx: &egui::Context, hidden: std::collections::BTreeSet<String>) {
    ctx.data_mut(|d| {
        d.insert_temp(
            tips_id(),
            Tips {
                hidden,
                new: Default::default(),
            },
        );
    });
}

/// Whether the tip `id` is hidden.
pub fn tip_hidden(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| {
        d.get_temp::<Tips>(tips_id())
            .is_some_and(|t| t.hidden.contains(id))
    })
}

/// Hides the tip `id`.
pub fn hide_tip(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| {
        let t = d.get_temp_mut_or_default::<Tips>(tips_id());
        t.hidden.insert(id.to_string());
        t.new.insert(id.to_string());
    });
}

/// The tips hidden since the last call, to store; `None` when none.
pub fn take_hidden_tips(ctx: &egui::Context) -> Option<std::collections::BTreeSet<String>> {
    ctx.data_mut(|d| {
        let t = d.get_temp_mut_or_default::<Tips>(tips_id());
        let new = std::mem::take(&mut t.new);
        (!new.is_empty()).then_some(new)
    })
}

/// The size of the X's hit target, in points.
pub const HIDE_TIP_TARGET: f32 = 20.0;

/// The name and the card of a hint card's X.
pub const HIDE_TIP: &str = "Hide this tip";
pub const HIDE_TIP_CARD: &str = "Hide this tip. You can show it again in Settings.";

/// A hint: an issue in a soft amber box, with a title (and its card), the
/// text and buttons that fix it. Returns the index of the clicked button.
/// A tip with a `dismiss` id has an X in its top right corner that hides
/// it for good (for every material and document) until the Settings
/// window shows hidden tips again; a hidden tip draws nothing.
pub fn hint_card(
    ui: &mut Ui,
    t: &Tokens,
    dismiss: Option<&str>,
    title: Option<(&str, &str)>,
    issue: &str,
    actions: &[HintButton<'_>],
) -> Option<usize> {
    hint_card_with(ui, t, dismiss, title, issue, None, false, actions)
}

/// [`hint_card`] with a dimmed line under the text (`detail`: what to do
/// when no button fixes it); a `blocking` one (it stops the cut) is red.
#[allow(clippy::too_many_arguments)]
pub fn hint_card_with(
    ui: &mut Ui,
    t: &Tokens,
    dismiss: Option<&str>,
    title: Option<(&str, &str)>,
    issue: &str,
    detail: Option<&str>,
    blocking: bool,
    actions: &[HintButton<'_>],
) -> Option<usize> {
    let (accent, title_ink) = if blocking {
        (t.danger, t.danger)
    } else {
        (t.warn, t.warn_text)
    };
    if dismiss.is_some_and(|id| tip_hidden(ui.ctx(), id)) {
        return None;
    }
    let mut clicked = None;
    let mut hide = false;
    egui::Frame::new()
        .fill(accent.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.6)))
        .corner_radius(t.radius)
        .inner_margin(egui::Margin::same(metrics::CARD_PAD as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = metrics::SPACE_4;
            if title.is_some() || dismiss.is_some() {
                ui.horizontal(|ui| {
                    if let Some((title, card)) = title {
                        let r = ui.label(
                            RichText::new(title)
                                .font(metrics::section_title())
                                .color(title_ink),
                        );
                        groups::card(&r, title, card, None, false);
                    }
                    if dismiss.is_some() {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let size = metrics::ICON_SMALL;
                            let tex = icons::texture(ui.ctx(), Icon::Close, size);
                            let image = egui::Image::from_texture(egui::load::SizedTexture::new(
                                tex.id(),
                                vec2(size, size),
                            ))
                            .tint(t.text_dim)
                            .alt_text(HIDE_TIP);
                            // A small icon, a target big enough to hit.
                            let r = ui.add_sized(
                                vec2(HIDE_TIP_TARGET, HIDE_TIP_TARGET),
                                egui::Button::image(image).frame(false),
                            );
                            set_name(ui, &r, HIDE_TIP);
                            groups::card(&r, HIDE_TIP, HIDE_TIP_CARD, None, false);
                            hide = r.clicked();
                        });
                    }
                });
            }
            ui.add(egui::Label::new(RichText::new(issue).color(t.text)).wrap());
            if let Some(detail) = detail {
                ui.add(egui::Label::new(RichText::new(detail).color(t.text_dim)).wrap());
            }
            if !actions.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for (i, b) in actions.iter().enumerate() {
                        let r = ui.add_enabled(b.why.is_none(), egui::Button::new(b.label));
                        set_name(ui, &r, b.label);
                        groups::card(&r, b.label, b.card, b.why, false);
                        if r.clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            }
        });
    if hide && let Some(id) = dismiss {
        hide_tip(ui.ctx(), id);
        return None;
    }
    clicked
}
