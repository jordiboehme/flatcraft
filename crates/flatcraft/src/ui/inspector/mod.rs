//! The inspector on the right of Prepare: the job line on top, always
//! visible, and below it what the selection is cut with, or the Job panel.
//!
//! - Nothing selected: the Job panel.
//! - Objects selected: the object inspector ([`object`]).
//! - A click on the job line shows the Job panel over a selection
//!   ([`InspectorView::Job`]); a second click, Escape or another selection
//!   goes back.
//!
//! The job line and the heading under it stay where they are when the
//! selection changes, so the panel never jumps.

pub mod blocks;
pub mod job;
pub mod object;

use egui::Ui;

use super::Action;
use super::prepare::{PrepareState, Snapshot};
use super::rails::{Dot, Side, SidePanel};
use super::theme::Tokens;

/// What the inspector shows. UI state only: no undo step, never saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorView {
    /// What the selection asks for: the object inspector, or the job when
    /// nothing is selected.
    #[default]
    Selection,
    /// Only the Job panel, also over a selection (the job line asked).
    Job,
}

/// The ids of every tip a person can hide (`Preferences.hidden_tips`).
pub const KNOWN_TIPS: &[&str] = &[super::materials::TEST_CUT_TIP];

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

/// The line under "Job": nothing selected, and over a selection (the
/// Job view). The heading's card says the rest ([`SUBTITLES`]).
pub const SUBTITLE_LINES: [&str; 2] = [
    "Nothing selected: what this cut uses",
    "What this cut uses. Esc goes back",
];

/// The cards of the heading "Job", in the order of [`SUBTITLE_LINES`].
pub const SUBTITLES: [&str; 2] = [
    "Nothing selected: what this cut uses. Select objects to choose what is cut of them.",
    "What this cut uses, over your selection. Click the job line or press Esc to go back to the selection.",
];

/// The line under "Job" and its card.
pub fn job_subtitle(st: &PrepareState, snap: &Snapshot) -> (&'static str, &'static str) {
    let i = usize::from(!snap.selection.is_empty() && st.inspector == InspectorView::Job);
    (SUBTITLE_LINES[i], SUBTITLES[i])
}

/// Whether the object inspector shows (objects are selected and the job
/// line did not ask for the job).
pub fn shows_objects(st: &PrepareState, snap: &Snapshot) -> bool {
    !snap.selection.is_empty() && st.inspector == InspectorView::Selection
}

/// The inspector: the job line with the panel's fold button, then the
/// heading and the sections in a scroll area: the object inspector with a
/// selection, else the Job panel. Each keeps its own scroll position.
pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    dot: Dot,
    t: &Tokens,
    show_debug: &mut bool,
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
    let objects = shows_objects(st, snap);
    egui::ScrollArea::vertical()
        .id_salt(if objects {
            "flatcraft-inspector-objects"
        } else {
            "flatcraft-inspector"
        })
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Room for the scroll bar on the right, so it never lies over
            // a reset arrow or the Keep aspect lock.
            let bar = ui.spacing().scroll;
            let room = bar.bar_width + bar.bar_inner_margin + bar.bar_outer_margin;
            ui.set_max_width((ui.available_width() - room).max(0.0));
            if objects {
                object::panel(ui, st, snap, t, show_debug, actions);
            } else {
                let (line, card) = job_subtitle(st, snap);
                blocks::heading(ui, "Job", line, card);
                ui.add_space(t.spacing / 2.0);
                job::panel(ui, st, snap, t, actions);
            }
        });
}
