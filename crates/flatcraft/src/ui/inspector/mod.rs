//! The inspector on the right of Prepare: the job line on top, always
//! visible, and below it what the selection is cut with, or the Job panel.
//!
//! - Nothing selected: the Job panel.
//! - Objects selected: their cut options (the interim Selection section,
//!   until the object inspector takes over), then the Job panel.
//! - A click on the job line shows only the Job panel, also over a
//!   selection ([`InspectorView::Job`]); a second click, Escape or another
//!   selection goes back.

pub mod blocks;
pub mod job;

use egui::{RichText, Ui};

use super::prepare::{PrepareState, Snapshot};
use super::rails::{Dot, Side, SidePanel};
use super::theme::Tokens;
use super::{Action, fonts};

/// What the inspector shows. UI state only: no undo step, never saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorView {
    /// What the selection asks for: its cut options and the job, or only
    /// the job when nothing is selected.
    #[default]
    Selection,
    /// Only the Job panel, also over a selection (the job line asked).
    Job,
}

/// The ids of every tip a person can hide (`Preferences.hidden_tips`).
pub const KNOWN_TIPS: &[&str] = &[super::materials::TEST_CUT_TIP];

/// The id of the interim Selection section.
pub const SELECTION: &str = "object.cut";

/// Whether the Job panel shows over a selection (the job line is "open").
pub fn job_over_selection(st: &PrepareState, snap: &Snapshot) -> bool {
    st.inspector == InspectorView::Job && !snap.selection.is_empty()
}

/// Goes back to the selection's inspector when the document or the
/// selection changed since the last frame. Call once per frame, before
/// drawing.
pub fn follow_selection(st: &mut PrepareState, snap: &Snapshot) {
    let key = (snap.doc_id, snap.selection.clone());
    if st.inspector_key.as_ref() != Some(&key) {
        st.inspector = InspectorView::Selection;
        st.inspector_key = Some(key);
    }
}

/// The subtitles under "Job": nothing selected, over a selection (the
/// Job view), and under the selection's cut options.
pub const SUBTITLES: [&str; 3] = [
    "Nothing selected: what this cut uses. Select objects to choose what is cut of them.",
    "What this cut uses. Click the job line or press Esc to go back.",
    "What this cut uses, for the whole document.",
];

/// The subtitle under "Job".
pub fn job_subtitle(st: &PrepareState, snap: &Snapshot) -> &'static str {
    if snap.selection.is_empty() {
        SUBTITLES[0]
    } else if st.inspector == InspectorView::Job {
        SUBTITLES[1]
    } else {
        SUBTITLES[2]
    }
}

/// The inspector: the job line with the panel's fold button, then the
/// sections in a scroll area.
pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    dot: Dot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let open = job_over_selection(st, snap);
    let summary = job::summary(snap, dot, open);
    let mut clicked = false;
    let mut collapse = false;
    let w = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(w, blocks::JOB_LINE_H),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            collapse = super::rails::collapse_button(ui, SidePanel::Settings, Side::Right);
            clicked = blocks::job_line(ui, &summary, t).clicked();
        },
    );
    if collapse {
        st.panels.set_collapsed(SidePanel::Settings, true);
    }
    // With nothing selected the Job panel is below anyway: the line does
    // not toggle (it does not take clicks then).
    if clicked && !snap.selection.is_empty() {
        st.inspector = match st.inspector {
            InspectorView::Selection => InspectorView::Job,
            InspectorView::Job => InspectorView::Selection,
        };
    }
    ui.add_space(t.spacing);
    egui::ScrollArea::vertical()
        .id_salt("flatcraft-inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !snap.selection.is_empty() && st.inspector == InspectorView::Selection {
                super::settings::selection(ui, st, snap, t, actions);
                ui.add_space(t.spacing);
            }
            ui.label(RichText::new("Job").font(fonts::semibold(16.0)));
            ui.label(
                RichText::new(job_subtitle(st, snap))
                    .size(12.0)
                    .color(t.text_dim),
            );
            ui.add_space(t.spacing / 2.0);
            job::panel(ui, st, snap, t, actions);
        });
}
