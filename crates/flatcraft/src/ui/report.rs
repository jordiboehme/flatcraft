//! The import report card in Prepare and the "why nothing cuts" card in
//! Cut. Both show the engine's issues (`flatcraft_engine::report`): the
//! message, and a button per fix that runs exactly the fix's registry
//! command with its arguments, the same that agents get over MCP.

use crate::ui::metrics;
use egui::{RichText, Ui};
use flatcraft_engine::job::JobInput;
use flatcraft_engine::report::{self, Issue, Report, Severity};
use serde_json::Value;

use super::Action;
use super::groups::card;
use super::prepare::Snapshot;
use super::theme::Tokens;

/// The card of the "Show me" button.
pub const SHOW_ME_HINT: &str =
    "Selects the object and moves the view to it, so you can see what this is about.";
/// The card of the import report's Dismiss button.
pub const DISMISS_HINT: &str = "Hides this report. Nothing in the document changes.";
/// The card of the "N more below" line.
pub const MORE_HINT: &str = "More lines are below. Click to scroll down, or scroll the list.";

/// How many lines are not fully visible below the bottom of `view`.
pub fn hidden_below(rows: &[egui::Rect], view: egui::Rect) -> usize {
    rows.iter()
        .filter(|r| r.bottom() > view.bottom() + 0.5)
        .count()
}

/// The width of the import report card, in points.
pub const CARD_WIDTH: f32 = 360.0;

/// The import report of the last import, until it is dismissed or every
/// issue is gone.
#[derive(Debug, Clone)]
pub struct ImportCard {
    pub doc_id: u64,
    /// The card's heading: "Import report", "Reload report" or "Stencil
    /// on paper", what the card is about.
    pub header: String,
    /// "Imported 'logo'" or "Imported 3 files".
    pub title: String,
    /// The imported objects.
    pub ids: Vec<u64>,
    pub shapes: usize,
    /// What the import found in the files; `document.check` cannot see
    /// it again, so it stays while its objects exist.
    pub found: Vec<Issue>,
    /// The geometry checks, run again whenever the document changed.
    pub issues: Vec<Issue>,
    /// The job input the checks ran on.
    key: Option<JobInput>,
}

impl ImportCard {
    /// The card of an `object.import` result, or `None` when there is
    /// nothing to report.
    pub fn from_result(doc_id: u64, title: String, result: &Value) -> Option<Self> {
        let report: Report = serde_json::from_value(result["report"].clone()).ok()?;
        let ids: Vec<u64> = match result
            .get("imported")
            .or_else(|| result.get("reloaded"))
            .or_else(|| result.get("ids"))
        {
            Some(list) => list
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect(),
            None => result["id"].as_u64().into_iter().collect(),
        };
        let (found, issues) = report
            .issues
            .into_iter()
            .partition(|i: &Issue| i.code.import_only());
        let card = Self {
            doc_id,
            header: "Import report".into(),
            title,
            ids,
            shapes: report.shapes,
            found,
            issues,
            key: None,
        };
        (!card.is_empty()).then_some(card)
    }

    /// The card with another heading.
    pub fn with_header(mut self, header: &str) -> Self {
        self.header = header.into();
        self
    }

    /// Every issue, the most severe first (stable within a severity, the
    /// file's own issues first), so what blocks the cut is never below the
    /// fold.
    pub fn all(&self) -> Vec<&Issue> {
        by_severity(self.found.iter().chain(&self.issues))
    }

    /// Takes the report of a reload or relink: what the old files had is
    /// forgotten for the objects read again, and what their new files have
    /// is added; the geometry checks run again.
    pub fn reloaded(&mut self, ids: &[u64], result: &Value) {
        self.found
            .retain(|i| !i.object_ids.iter().all(|id| ids.contains(id)));
        if let Ok(r) = serde_json::from_value::<Report>(result["report"].clone()) {
            self.found
                .extend(r.issues.into_iter().filter(|i| i.code.import_only()));
        }
        for id in ids {
            if !self.ids.contains(id) {
                self.ids.push(*id);
            }
        }
        self.key = None;
    }

    pub fn is_empty(&self) -> bool {
        self.found.is_empty() && self.issues.is_empty()
    }

    /// Runs the geometry checks again when the snapshot's job input
    /// changed (a fix ran, an object moved, undo), so a line that is fixed
    /// goes away. Objects that are gone leave the report.
    pub fn refresh(&mut self, snap: &Snapshot) {
        if snap.doc_id != self.doc_id {
            return;
        }
        let Some(Ok(input)) = &snap.job_input else {
            return;
        };
        if self.key.as_ref() == Some(input) {
            return;
        }
        let alive = |id: &u64| snap.objects.iter().any(|o| o.id == *id);
        self.ids.retain(alive);
        self.found.retain(|i| i.object_ids.iter().any(alive));
        let ids: Vec<flatcraft_core::document::ObjectId> = self
            .ids
            .iter()
            .map(|i| flatcraft_core::document::ObjectId(*i))
            .collect();
        let report = report::check(input, Some(&ids));
        self.shapes = report.shapes;
        self.issues = report.issues;
        self.key = Some(input.clone());
    }
}

/// What a fix's button runs: exactly the fix's registry command with its
/// arguments, the same an agent runs. `None` when the command is unknown.
pub fn fix_action(doc_id: u64, fix: &report::Fix) -> Option<Action> {
    // The registry's own id: a fix names a real command.
    let command = flatcraft_engine::registry::get(&fix.command)?;
    Some(Action::run(doc_id, command.id(), fix.args.clone()))
}

/// Issues, the most severe first, keeping their order within a severity.
pub fn by_severity<'a>(issues: impl Iterator<Item = &'a Issue>) -> Vec<&'a Issue> {
    let mut v: Vec<&Issue> = issues.collect();
    v.sort_by_key(|i| std::cmp::Reverse(i.severity));
    v
}

/// The colour of a severity.
pub fn severity_colour(s: Severity, t: &Tokens) -> egui::Color32 {
    match s {
        Severity::Error => t.danger,
        Severity::Warning => t.warn,
        Severity::Info => t.text_dim,
    }
}

/// The issues one under the other: a dot in the severity's colour, the
/// message (its card says what to do), then a button per fix and "Show
/// me". An issue without a fix shows its hint instead. Returns true when
/// a button was clicked.
pub fn issue_list(
    ui: &mut Ui,
    issues: &[&Issue],
    doc_id: u64,
    t: &Tokens,
    show_me: bool,
    actions: &mut Vec<Action>,
) -> bool {
    issue_rows(ui, issues, doc_id, t, show_me, actions).0
}

/// [`issue_list`], and the screen rect of each line.
fn issue_rows(
    ui: &mut Ui,
    issues: &[&Issue],
    doc_id: u64,
    t: &Tokens,
    show_me: bool,
    actions: &mut Vec<Action>,
) -> (bool, Vec<egui::Rect>) {
    let mut clicked = false;
    let mut rows = Vec::with_capacity(issues.len());
    for (n, issue) in issues.iter().enumerate() {
        if n > 0 {
            ui.add_space(metrics::SPACE_4);
        }
        let row = ui.horizontal_top(|ui| {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 3.5, severity_colour(issue.severity, t));
            ui.vertical(|ui| {
                let r = ui.add(egui::Label::new(issue.message.as_str()).wrap());
                card(&r, issue.code.title(), &issue.hint, None, false);
                if issue.fixes.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(issue.hint.as_str()).color(t.text_dim))
                            .wrap(),
                    );
                }
                let wants_show = show_me && !issue.object_ids.is_empty();
                if issue.fixes.is_empty() && !wants_show {
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    for fix in &issue.fixes {
                        let Some(action) = fix_action(doc_id, fix) else {
                            continue;
                        };
                        let r = ui.button(fix.label.as_str());
                        card(&r, &fix.label, &fix.description, None, false);
                        if r.clicked() {
                            actions.push(action);
                            clicked = true;
                        }
                    }
                    if wants_show {
                        let r = ui.button("Show me");
                        card(&r, "Show me", SHOW_ME_HINT, None, false);
                        if r.clicked() {
                            actions.push(Action::ShowMe {
                                doc_id,
                                ids: issue.object_ids.clone(),
                            });
                            clicked = true;
                        }
                    }
                });
            });
        });
        rows.push(row.response.rect);
    }
    (clicked, rows)
}

/// The import report card at the top right of the Prepare canvas. Returns
/// false when it was dismissed, and the rect it took (notices go below).
pub fn import_card(
    ui: &mut Ui,
    canvas: egui::Rect,
    c: &ImportCard,
    t: &Tokens,
    actions: &mut Vec<Action>,
) -> (bool, egui::Rect) {
    let mut keep = true;
    let worst = c
        .all()
        .iter()
        .map(|i| i.severity)
        .max()
        .unwrap_or(Severity::Info);
    let width = CARD_WIDTH
        .min(canvas.width() - 2.0 * metrics::SPACE_8)
        .max(120.0);
    let max = egui::Rect::from_min_size(
        egui::pos2(
            canvas.right() - width - metrics::SPACE_8,
            canvas.top() + super::canvas::RULER + metrics::SPACE_8,
        ),
        egui::vec2(width, canvas.height() * 0.7),
    );
    let shown = ui.scope_builder(egui::UiBuilder::new().max_rect(max), |ui| {
        egui::Frame::new()
            .fill(t.surface)
            .stroke(egui::Stroke::new(1.0, severity_colour(worst, t)))
            .corner_radius(t.radius)
            .inner_margin(metrics::SPACE_8)
            .show(ui, |ui| {
                ui.set_width(width - 2.0 * metrics::SPACE_8);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&c.header).font(metrics::heading()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let r = ui.button("Dismiss");
                        card(&r, "Dismiss", DISMISS_HINT, None, false);
                        if r.clicked() {
                            keep = false;
                        }
                    });
                });
                let shapes = if c.shapes == 1 {
                    "1 shape".to_string()
                } else {
                    format!("{} shapes", c.shapes)
                };
                let lead = if worst == Severity::Error {
                    "Houston, we have a problem."
                } else {
                    "A few things to know before you cut."
                };
                ui.label(RichText::new(format!("{}: {shapes}. {lead}", c.title)).color(t.text_dim));
                ui.add_space(metrics::SPACE_4);
                // A solid bar that shows whenever more lines are below,
                // not a floating one that shows only on hover.
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                // Room for the "N more below" line under the list.
                let more_h = ui.text_style_height(&egui::TextStyle::Body) + metrics::SPACE_4;
                let jump_id = egui::Id::unique("flatcraft-report-scroll-down");
                let jump: Option<f32> = ui.ctx().data_mut(|d| d.remove_temp(jump_id));
                let mut area = egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - more_h).max(40.0))
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    );
                if let Some(offset) = jump {
                    area = area.vertical_scroll_offset(offset);
                }
                let out = area.show(ui, |ui| {
                    issue_rows(ui, &c.all(), c.doc_id, t, true, actions).1
                });
                let hidden = hidden_below(&out.inner, out.inner_rect);
                if hidden > 0 {
                    let text = if hidden == 1 {
                        "1 more below".to_string()
                    } else {
                        format!("{hidden} more below")
                    };
                    let r = ui.add(
                        egui::Label::new(RichText::new(text).color(t.text_dim))
                            .sense(egui::Sense::click()),
                    );
                    card(&r, "More below", MORE_HINT, None, false);
                    if r.clicked() {
                        let down = out.state.clamped_offset().y + out.inner_rect.height();
                        ui.ctx().data_mut(|d| d.insert_temp(jump_id, down));
                        ui.ctx().request_repaint();
                    }
                }
            });
    });
    (keep, shown.response.rect)
}

/// The title of the Cut workspace's card for these issues.
pub fn nothing_title(nothing_to_cut: bool) -> &'static str {
    if nothing_to_cut {
        "Nothing to cut"
    } else {
        "Houston, we have a problem"
    }
}

/// Why the job cannot be cut, in the Cut workspace's stage panel: every
/// reason with its fixes.
pub fn reasons_box(
    ui: &mut Ui,
    issues: &[Issue],
    nothing_to_cut: bool,
    doc_id: u64,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    egui::Frame::new()
        .fill(t.danger.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, t.danger))
        .corner_radius(t.radius)
        .inner_margin(metrics::SPACE_8)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(nothing_title(nothing_to_cut)).font(metrics::section_title()));
            let lead = if nothing_to_cut {
                "The knife is ready, but nothing is in its way. Here is why:"
            } else {
                "This is what stops the cut:"
            };
            ui.label(RichText::new(lead).color(t.text_dim));
            ui.add_space(metrics::SPACE_4);
            issue_list(ui, &by_severity(issues.iter()), doc_id, t, true, actions);
        });
}
