//! The Prepare ribbon: the Home, Arrange and View tabs with their tools,
//! and the "Search commands" box on the right.

use egui::accesskit::Role;
use egui::{Ui, WidgetInfo};
use flatcraft_engine::registry;
use serde_json::{Value, json};

use super::canvas::{CanvasView, PX_PER_MM_100};
use super::icons::{self, Icon};
use super::prepare::{PrepareState, RibbonTab, Snapshot};
use super::settings::set_name;
use super::{Action, ICON_SIZE};

/// A button with an icon and a text.
fn tool(ui: &mut Ui, icon: Icon, text: &str, enabled: bool, hint: &str) -> bool {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ));
    ui.add_enabled(enabled, egui::Button::image_and_text(image, text))
        .on_hover_text(hint)
        .on_disabled_hover_text(hint)
        .clicked()
}

/// A toggle with an icon and a text.
pub(crate) fn toggle(ui: &mut Ui, icon: Icon, text: &str, on: &mut bool) {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ));
    if ui
        .add(egui::Button::image_and_text(image, text).selected(*on))
        .clicked()
    {
        *on = !*on;
    }
}

/// A short number field. Shows `current` while it is not being edited and
/// returns the typed number when Enter is pressed.
fn number_field(
    ui: &mut Ui,
    id_salt: &str,
    label: &str,
    name: &str,
    buf: &mut String,
    current: Option<String>,
    enabled: bool,
) -> Option<f64> {
    let id = ui.make_persistent_id(id_salt);
    if !ui.memory(|m| m.has_focus(id))
        && let Some(c) = current
    {
        *buf = c;
    }
    ui.label(label).on_hover_text(name);
    let resp = ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(buf).id(id).desired_width(52.0),
    );
    set_name(ui, &resp, name);
    let resp = resp.on_hover_text(name);
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        return buf.trim().replace(',', ".").parse::<f64>().ok();
    }
    None
}

/// What the search box does with a command.
#[derive(Debug, Clone, PartialEq)]
pub enum SearchTarget {
    /// Run it on the selection, without params.
    Run,
    /// Show this ribbon tab.
    Tab(RibbonTab),
    /// Show the Settings panel.
    Settings,
    /// Something only the window does.
    Action(Action),
}

/// Where search sends a command, or `None` for commands that only make
/// sense for agents (queries and sends).
pub fn search_target(id: &str, doc_id: u64) -> Option<SearchTarget> {
    use SearchTarget::*;
    Some(match id {
        "edit.undo" => Action(super::Action::Undo(doc_id)),
        "edit.redo" => Action(super::Action::Redo(doc_id)),
        "document.new" => Action(super::Action::NewDocument),
        "project.open" => Action(super::Action::OpenDialog),
        "project.save" => Action(super::Action::Save { doc_id }),
        "document.check_sources" => Action(super::Action::CheckSources),
        "object.import" => Action(super::Action::Import { doc_id }),
        "object.reload"
        | "object.unlink"
        | "object.delete"
        | "object.duplicate"
        | "arrange.auto"
        | "tools.test_patterns" => Run,
        "object.retrace" => Tab(RibbonTab::Home),
        "object.scale" | "object.rotate" | "object.mirror" | "arrange.align" => {
            Tab(RibbonTab::Arrange)
        }
        "object.set_cut_options"
        | "job.set_plotter"
        | "job.set_material"
        | "job.set_media"
        | "job.set_overrides" => Settings,
        _ => return None,
    })
}

/// Lowercase letters and digits only, so "Auto-arrange" finds "Auto arrange".
fn normalise(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// How well `query` matches `text`: a substring beats a scattered match,
/// an earlier one beats a later one. `None` if it does not match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q = normalise(query);
    let t = normalise(text);
    if q.is_empty() {
        return None;
    }
    if let Some(pos) = t.find(&q) {
        return Some(1000 - pos as i32 - (t.len() - q.len()) as i32);
    }
    let mut chars = t.chars();
    q.chars()
        .all(|c| chars.any(|x| x == c))
        .then(|| 100 - t.len() as i32)
}

/// Commands that match the query, best first, with their titles.
pub fn search(query: &str, doc_id: u64) -> Vec<(&'static str, &'static str)> {
    let mut hits: Vec<(i32, &'static str, &'static str)> = registry::all()
        .iter()
        .filter(|c| search_target(c.id(), doc_id).is_some())
        .filter_map(|c| {
            let score =
                fuzzy_score(query, c.title()).max(fuzzy_score(query, c.id()).map(|s| s - 1));
            score.map(|s| (s, c.id(), c.title()))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(b.2)));
    hits.into_iter().map(|(_, id, title)| (id, title)).collect()
}

/// The ids Reload sends: the selection's linked objects, or every linked
/// object (an empty list) when nothing is selected. `None` when there is
/// nothing to reload.
pub fn reload_ids(snap: &Snapshot) -> Option<Vec<u64>> {
    if snap.selection.is_empty() {
        return snap.objects.iter().any(|o| o.link.is_some()).then(Vec::new);
    }
    let ids: Vec<u64> = snap
        .selected()
        .filter(|o| o.link.is_some())
        .map(|o| o.id)
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// The id of the selected object when it is the only one and an image.
pub fn single_raster(snap: &Snapshot) -> Option<u64> {
    match snap.selected().collect::<Vec<_>>().as_slice() {
        [o] if o.raster => Some(o.id),
        _ => None,
    }
}

/// What picking a search hit does.
fn pick(id: &'static str, snap: &Snapshot, st: &mut PrepareState, actions: &mut Vec<Action>) {
    if id == "object.reload" {
        // Like the ribbon button.
        if let Some(ids) = reload_ids(snap) {
            actions.push(Action::run(snap.doc_id, id, json!({ "ids": ids })));
        }
        return;
    }
    if id == "object.retrace"
        && let Some(o) = single_raster(snap)
    {
        // Like the ribbon button; else the Home tab shows where Trace is.
        actions.push(Action::TraceDialog {
            doc_id: snap.doc_id,
            id: o,
        });
        return;
    }
    match search_target(id, snap.doc_id) {
        Some(SearchTarget::Run) => actions.push(Action::run(snap.doc_id, id, json!({}))),
        Some(SearchTarget::Tab(tab)) => st.tab = tab,
        Some(SearchTarget::Settings) => st.show_settings = true,
        Some(SearchTarget::Action(a)) => actions.push(a),
        None => {}
    }
}

fn search_box(ui: &mut Ui, snap: &Snapshot, st: &mut PrepareState, actions: &mut Vec<Action>) {
    let id = ui.make_persistent_id("flatcraft-search");
    let resp = ui.add(
        egui::TextEdit::singleline(&mut st.fields.search)
            .id(id)
            .hint_text("Search commands")
            .desired_width(200.0),
    );
    set_name(ui, &resp, "Search commands");
    let hits = search(&st.fields.search, snap.doc_id);
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        if let Some((id, _)) = hits.first() {
            pick(id, snap, st, actions);
        }
        st.fields.search.clear();
        return;
    }
    // A press on a hit takes the focus from the field before the click
    // lands, so the list stays open while the pointer is over it.
    let open = !hits.is_empty() && (resp.has_focus() || st.search_open);
    st.search_open = false;
    if open {
        let mut chosen = None;
        let popup = egui::Popup::from_response(&resp).open(true).show(|ui| {
            ui.set_min_width(200.0);
            for (id, title) in hits.iter().take(8) {
                let r = ui.selectable_label(false, *title);
                set_name(ui, &r, &format!("{title} (search)"));
                if r.clicked() {
                    chosen = Some(*id);
                }
            }
        });
        let hovered = popup.is_some_and(|p| p.response.contains_pointer());
        st.search_open = resp.has_focus() || hovered;
        if let Some(id) = chosen {
            pick(id, snap, st, actions);
            st.fields.search.clear();
            st.search_open = false;
        }
    }
}

fn tabs(ui: &mut Ui, st: &mut PrepareState) {
    for tab in RibbonTab::ALL {
        let on = st.tab == tab;
        let r = ui.add(egui::Button::selectable(on, tab.name()));
        r.widget_info(|| WidgetInfo::selected(Role::Tab, true, on, tab.name()));
        if r.clicked() {
            st.tab = tab;
        }
    }
}

fn mm(v: f64) -> String {
    format!("{v:.2}")
}

/// The tools of the current tab, then the search box at the right.
pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    show_debug: &mut bool,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let run = |actions: &mut Vec<Action>, cmd: &'static str, params: Value| {
        actions.push(Action::run(doc_id, cmd, params));
    };
    let any = !snap.selection.is_empty();
    let selected: Vec<u64> = snap.selection.iter().copied().collect();
    {
        {
            ui.vertical(|ui| {
                ui.horizontal(|ui| tabs(ui, st));
                ui.horizontal(|ui| match st.tab {
                    RibbonTab::Home => {
                        if tool(
                            ui,
                            Icon::Import,
                            "Import",
                            true,
                            "Import an SVG or an image",
                        ) {
                            actions.push(Action::Import { doc_id });
                        }
                        let raster = single_raster(snap);
                        if tool(
                            ui,
                            Icon::Trace,
                            "Trace",
                            raster.is_some(),
                            "Trace the selected image with your own settings",
                        ) && let Some(id) = raster
                        {
                            actions.push(Action::TraceDialog { doc_id, id });
                        }
                        if tool(
                            ui,
                            Icon::TestPatterns,
                            "Test patterns",
                            true,
                            "Add the test sheet. Cut it first on a new machine.",
                        ) {
                            run(actions, "tools.test_patterns", json!({}));
                        }
                        if tool(ui, Icon::Duplicate, "Duplicate", any, "Copy the selection") {
                            run(actions, "object.duplicate", json!({ "ids": selected }));
                        }
                        if tool(ui, Icon::Delete, "Delete", any, "Delete the selection") {
                            run(actions, "object.delete", json!({ "ids": selected }));
                        }
                        let ids = reload_ids(snap);
                        if tool(
                            ui,
                            Icon::Reload,
                            "Reload",
                            ids.is_some(),
                            "Read the linked files again (the selection, or all)",
                        ) && let Some(ids) = ids
                        {
                            run(actions, "object.reload", json!({ "ids": ids }));
                        }
                    }
                    RibbonTab::Arrange => arrange(ui, st, snap, actions),
                    RibbonTab::View => {
                        if tool(ui, Icon::ZoomFit, "Zoom to fit", true, "Show the whole mat") {
                            st.fit_pending = true;
                        }
                        if tool(
                            ui,
                            Icon::ZoomActual,
                            "Zoom 100%",
                            true,
                            "About real size on a typical screen",
                        ) {
                            let at = st.canvas_rect.center();
                            let f = PX_PER_MM_100 / st.view.px_per_mm;
                            let mut v: CanvasView = st.view;
                            v.zoom_about(at, f);
                            st.view = v;
                        }
                        toggle(ui, Icon::Grid, "Grid", &mut st.show_grid);
                        toggle(ui, Icon::Ruler, "Rulers", &mut st.show_rulers);
                        toggle(ui, Icon::PanelLeft, "Objects panel", &mut st.show_objects);
                        toggle(
                            ui,
                            Icon::PanelRight,
                            "Settings panel",
                            &mut st.show_settings,
                        );
                        toggle(ui, Icon::Debug, "Debug panel", show_debug);
                    }
                });
            });
        }
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        search_box(ui, snap, st, actions);
    });
}

fn arrange(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    let doc_id = snap.doc_id;
    let run = |actions: &mut Vec<Action>, cmd: &'static str, params: Value| {
        actions.push(Action::run(doc_id, cmd, params));
    };
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let any = !ids.is_empty();
    let single = ids.len() == 1;
    let b = super::canvas::selection_box(snap);
    let f = &mut st.fields;

    let w = number_field(
        ui,
        "scale-width",
        "W",
        "Width (mm)",
        &mut f.width,
        Some(b.map(|b| mm(b.width())).unwrap_or_default()),
        any,
    );
    let h = number_field(
        ui,
        "scale-height",
        "H",
        "Height (mm)",
        &mut f.height,
        Some(b.map(|b| mm(b.height())).unwrap_or_default()),
        any,
    );
    for (target, key, have) in [
        (w, "width_mm", b.map(|b| b.width())),
        (h, "height_mm", b.map(|b| b.height())),
    ] {
        let Some(target) = target.filter(|v| *v > 0.0) else {
            continue;
        };
        if single {
            run(
                actions,
                "object.scale",
                json!({ "ids": ids, key: target, "keep_aspect": f.keep_aspect }),
            );
        } else if let Some(have) = have.filter(|h| *h > 1e-9) {
            // Several objects: one factor from the common box.
            run(
                actions,
                "object.scale",
                json!({ "ids": ids, "factor": target / have }),
            );
        }
    }
    if ids.len() > 1 {
        let mut on = true;
        ui.add_enabled(false, egui::Checkbox::new(&mut on, "Keep aspect"))
            .on_disabled_hover_text("Several objects scale together, keeping their shape.");
    } else {
        ui.add_enabled(any, egui::Checkbox::new(&mut f.keep_aspect, "Keep aspect"));
    }
    if let Some(p) = number_field(
        ui,
        "scale-percent",
        "%",
        "Scale (%)",
        &mut f.percent,
        Some(if any { "100".into() } else { String::new() }),
        any,
    ) && p > 0.0
    {
        run(
            actions,
            "object.scale",
            json!({ "ids": ids, "factor": p / 100.0 }),
        );
    }
    ui.separator();
    if let Some(d) = number_field(
        ui,
        "rotate-by",
        "Rotate",
        "Rotate by (deg)",
        &mut f.rotate,
        None,
        any,
    ) {
        run(
            actions,
            "object.rotate",
            json!({ "ids": ids, "degrees": d }),
        );
    }
    if super::icon_button(ui, Icon::RotateLeft, "Rotate -90", any).clicked() {
        run(
            actions,
            "object.rotate",
            json!({ "ids": ids, "degrees": -90.0 }),
        );
    }
    if super::icon_button(ui, Icon::Rotate, "Rotate +90", any).clicked() {
        run(
            actions,
            "object.rotate",
            json!({ "ids": ids, "degrees": 90.0 }),
        );
    }
    ui.separator();
    if super::icon_button(ui, Icon::Mirror, "Mirror horizontally", any).clicked() {
        run(
            actions,
            "object.mirror",
            json!({ "ids": ids, "axis": "horizontal" }),
        );
    }
    if super::icon_button(ui, Icon::MirrorVertical, "Mirror vertically", any).clicked() {
        run(
            actions,
            "object.mirror",
            json!({ "ids": ids, "axis": "vertical" }),
        );
    }
    ui.separator();
    let align_to_area = &mut st.fields.align_to_area;
    ui.add_enabled_ui(any, |ui| {
        ui.menu_button("Align", |ui| {
            ui.radio_value(align_to_area, false, "To the selection");
            ui.radio_value(align_to_area, true, "To the cutting area");
            ui.separator();
            for (mode, text) in [
                ("left", "Left"),
                ("hcenter", "Centre"),
                ("right", "Right"),
                ("top", "Top"),
                ("vcenter", "Middle"),
                ("bottom", "Bottom"),
            ] {
                if ui.button(text).clicked() {
                    let to = if *align_to_area {
                        "cutting_area"
                    } else {
                        "selection"
                    };
                    run(
                        actions,
                        "arrange.align",
                        json!({ "ids": ids, "mode": mode, "relative_to": to }),
                    );
                    ui.close();
                }
            }
        });
    });
    if tool(
        ui,
        Icon::Arrange,
        "Auto-arrange",
        !snap.objects.is_empty(),
        "Pack everything onto the mat, Tetris style",
    ) {
        run(actions, "arrange.auto", json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyphens_and_case_do_not_matter() {
        let hits = search("Auto-arrange", 1);
        assert_eq!(hits.first().map(|h| h.0), Some("arrange.auto"));
        assert!(fuzzy_score("dupl", "Duplicate").is_some());
        assert!(fuzzy_score("xyz", "Duplicate").is_none());
    }

    #[test]
    fn runnable_commands_need_no_params() {
        for c in registry::all() {
            if search_target(c.id(), 1) == Some(SearchTarget::Run) {
                let schema = serde_json::to_value(c.params_schema()).unwrap();
                let required = schema["required"].as_array().map_or(0, Vec::len);
                assert_eq!(required, 0, "{} needs params", c.id());
            }
        }
    }

    #[test]
    fn searching_never_sends_a_job() {
        assert!(search("send", 1).iter().all(|h| h.0 != "job.send"));
        assert!(search_target("job.send", 1).is_none());
    }
}
