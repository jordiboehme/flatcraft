//! The Export dialog (the shared [`super::dialog`]): the format, which
//! objects, and whether skipped objects come along. Export asks where to
//! save with the native dialog and then runs `document.export` as the
//! person. What is exported is what will be cut: the shapes as placed on
//! the mat, without overcut, passes or blade moves.
//!
//! File > Export... (menu bar and the app bar's File menu) opens it on the
//! selection when there is one, else on all objects; the object
//! right-click menu's "Export selected..." opens it on the selection.

use egui::{RichText, Ui};
use flatcraft_engine::export::{ExportFormat, ExportScope};

use super::Action;
use super::dialog::{Answer, Dialog, Kind};
use super::groups;
use super::prepare::Snapshot;
use super::settings::set_name;
use super::theme::Tokens;

/// The dialog's title.
pub const TITLE: &str = "Export what will be cut";
/// The primary button.
pub const EXPORT: &str = "Export...";
/// The checkbox for skipped objects.
pub const INCLUDE_SKIPPED: &str = "Include objects that are not cut";
/// The scope choices.
pub const SELECTED: &str = "Selected objects";
pub const ALL: &str = "All objects";

/// What the dialog says under its title.
pub const BODY: &str = "Saves the shapes the blade will cut, placed as on the mat, in millimetres. No overcut, passes or blade moves, just the shapes.";

/// The card of the Export button.
pub const EXPORT_HINT: &str = "Pick where to save the file, then write the shapes into it.";
/// The card of Cancel.
pub const CANCEL_HINT: &str = "Close this dialog without exporting anything.";

/// The hover card of File > Export... and the menu items.
pub const MENU_HINT: &str = "Save the shapes that will be cut as SVG, DXF or PDF, placed as on the mat, to use them in other apps or to print them 1:1.";

/// The hover card of "Export selected...".
pub const SELECTED_HINT: &str =
    "Save the shapes of the selected objects as SVG, DXF or PDF, placed as on the mat.";

/// Why Export is dimmed: no document is open (Home).
pub const NO_DOCUMENT: &str =
    "Open a project or start a new one first. Export works on the document you are working on.";
/// Why: the document has no objects.
pub const NO_OBJECTS: &str = "There is nothing to export yet. Import a design first.";
/// Why: every object in scope is hidden.
pub const ALL_HIDDEN: &str =
    "Hidden objects are never exported, as they are never cut. Show them first.";
/// Why: every visible object in scope is skipped and the box is off.
pub const ALL_SKIPPED: &str = "These objects are set to Skip cutting. Tick \"Include objects that are not cut\" to export them anyway.";
/// Why the checkbox is dimmed.
pub const NOTHING_SKIPPED: &str =
    "No object here is set to Skip cutting, so there is nothing more to include.";
/// Why "Selected objects" is dimmed.
pub const NOTHING_SELECTED: &str = "Select objects on the mat first to export only those.";

/// What the hover card of each format says.
pub fn format_hint(f: ExportFormat) -> &'static str {
    match f {
        ExportFormat::Svg => {
            "A vector drawing for Inkscape, Illustrator, Affinity, the web and FlatCraft itself. Sizes are in mm, so it imports at the right size."
        }
        ExportFormat::Dxf => {
            "A CAD drawing. DXF opens in Silhouette Studio's free edition, laser and CAD tools. Each object gets its own layer."
        }
        ExportFormat::Pdf => {
            "One page the size of the mat at 1:1, to print or to look at. Print at 100% (not \"fit to page\") and a ruler agrees with the cut."
        }
    }
}

/// The hover card of the scope choices.
pub fn scope_hint(s: ExportScope) -> &'static str {
    match s {
        ExportScope::Selected => "Only the objects you selected on the mat.",
        ExportScope::All => "Every object on the mat that will be cut.",
    }
}

/// The hover card of the checkbox.
pub const INCLUDE_HINT: &str = "Also export objects you set to Skip cutting, for example a print you only cut around. Hidden objects are never exported.";

/// The open Export dialog.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportDialog {
    pub doc_id: u64,
    pub format: ExportFormat,
    pub scope: ExportScope,
    pub include_skipped: bool,
}

impl ExportDialog {
    /// The dialog for the document `doc_id`: on the selection when there
    /// is one, else on all objects.
    pub fn new(doc_id: u64, has_selection: bool) -> Self {
        Self {
            doc_id,
            format: ExportFormat::Svg,
            scope: ExportScope::default_for(has_selection),
            include_skipped: false,
        }
    }
}

/// The objects of `snap` in `scope`: (visible, cut_enabled) of each.
fn in_scope(snap: &Snapshot, scope: ExportScope) -> Vec<(bool, bool)> {
    snap.objects
        .iter()
        .filter(|o| scope == ExportScope::All || snap.selection.contains(&o.id))
        .map(|o| (o.visible, o.cut_enabled))
        .collect()
}

/// Why nothing of these objects (visible, cut_enabled) would be exported,
/// or `None` when something would.
pub fn why_nothing(objects: &[(bool, bool)], include_skipped: bool) -> Option<&'static str> {
    use flatcraft_engine::export::is_exported;
    if objects.is_empty() {
        return Some(NO_OBJECTS);
    }
    if objects
        .iter()
        .any(|(v, c)| is_exported(*v, *c, include_skipped))
    {
        return None;
    }
    if objects.iter().all(|(v, _)| !v) {
        Some(ALL_HIDDEN)
    } else {
        Some(ALL_SKIPPED)
    }
}

/// Why File > Export... is dimmed: no document, or no object to export
/// at all (even with the skipped ones). The dialog says more.
pub fn menu_why(snap: Option<&Snapshot>) -> Option<&'static str> {
    let Some(snap) = snap else {
        return Some(NO_DOCUMENT);
    };
    why_nothing(&in_scope(snap, ExportScope::All), true)
}

/// Why "Export selected..." is dimmed for the objects `ids`.
pub fn selected_why(snap: &Snapshot, ids: &[u64]) -> Option<&'static str> {
    let objects: Vec<(bool, bool)> = ids
        .iter()
        .filter_map(|id| snap.object(*id))
        .map(|o| (o.visible, o.cut_enabled))
        .collect();
    why_nothing(&objects, true)
}

/// Shows the dialog in `slot`. It closes when its document is no longer
/// shown. Export pushes [`Action::Export`]; Escape, × and Cancel close it.
pub fn show(
    ctx: &egui::Context,
    slot: &mut Option<ExportDialog>,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(d) = slot else {
        return;
    };
    if d.doc_id != snap.doc_id {
        *slot = None;
        return;
    }
    let has_selection = !snap.selection.is_empty();
    if !has_selection {
        d.scope = ExportScope::All;
    }
    let dialog = Dialog::new("export", Kind::Info, TITLE)
        .about(d.doc_id)
        .body(BODY)
        .primary(EXPORT)
        .primary_hint(EXPORT_HINT)
        .cancel_hint(CANCEL_HINT);
    let answer = dialog.show_with(ctx, t, |ui| {
        ui.add_space(t.spacing);
        controls(ui, d, snap, has_selection, t).map(String::from)
    });
    match answer {
        Some(Answer::Primary) => {
            actions.push(Action::Export(d.clone()));
            *slot = None;
        }
        Some(_) => *slot = None,
        None => {}
    }
}

/// The format, scope and checkbox; returns why Export cannot run, or
/// `None` when it can.
fn controls(
    ui: &mut Ui,
    d: &mut ExportDialog,
    snap: &Snapshot,
    has_selection: bool,
    t: &Tokens,
) -> Option<&'static str> {
    ui.label(RichText::new("Format").strong());
    ui.horizontal(|ui| {
        for f in ExportFormat::ALL {
            let r = ui.radio_value(&mut d.format, f, f.label());
            set_name(ui, &r, f.label());
            groups::card(&r, f.label(), format_hint(f), None, false);
        }
    });
    ui.add_space(t.spacing / 2.0);
    ui.label(RichText::new("Objects").strong());
    ui.horizontal(|ui| {
        for (s, label) in [(ExportScope::Selected, SELECTED), (ExportScope::All, ALL)] {
            let why = (s == ExportScope::Selected && !has_selection).then_some(NOTHING_SELECTED);
            let r = ui.add_enabled(why.is_none(), egui::RadioButton::new(d.scope == s, label));
            set_name(ui, &r, label);
            if r.clicked() {
                d.scope = s;
            }
            groups::card(&r, label, scope_hint(s), why, false);
        }
    });
    ui.add_space(t.spacing / 2.0);
    let objects = in_scope(snap, d.scope);
    let any_skipped = objects.iter().any(|(v, c)| *v && !*c);
    let why = (!any_skipped).then_some(NOTHING_SKIPPED);
    let r = ui.add_enabled(
        why.is_none(),
        egui::Checkbox::new(&mut d.include_skipped, INCLUDE_SKIPPED),
    );
    set_name(ui, &r, INCLUDE_SKIPPED);
    groups::card(&r, INCLUDE_SKIPPED, INCLUDE_HINT, why, false);
    let include = d.include_skipped && any_skipped;
    let blocked = why_nothing(&objects, include);
    if let Some(why) = blocked {
        ui.add_space(t.spacing / 2.0);
        ui.add(egui::Label::new(RichText::new(why).color(t.warn)).wrap());
    }
    blocked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_says_why() {
        assert_eq!(why_nothing(&[], false), Some(NO_OBJECTS));
        assert_eq!(why_nothing(&[(false, true)], true), Some(ALL_HIDDEN));
        assert_eq!(why_nothing(&[(true, false)], false), Some(ALL_SKIPPED));
        assert_eq!(why_nothing(&[(true, false)], true), None);
        assert_eq!(why_nothing(&[(false, true), (true, true)], false), None);
    }

    #[test]
    fn texts_have_no_em_dash() {
        let mut all = vec![
            TITLE,
            EXPORT,
            INCLUDE_SKIPPED,
            SELECTED,
            ALL,
            BODY,
            MENU_HINT,
            SELECTED_HINT,
            EXPORT_HINT,
            CANCEL_HINT,
            NO_DOCUMENT,
            NO_OBJECTS,
            ALL_HIDDEN,
            ALL_SKIPPED,
            NOTHING_SKIPPED,
            NOTHING_SELECTED,
            INCLUDE_HINT,
        ];
        all.extend(ExportFormat::ALL.map(format_hint));
        all.extend([ExportScope::Selected, ExportScope::All].map(scope_hint));
        for s in all {
            assert!(!s.contains('\u{2014}'), "{s}");
        }
    }
}
