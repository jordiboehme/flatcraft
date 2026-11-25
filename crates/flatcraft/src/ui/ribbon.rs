//! The Prepare ribbon: the Home, Arrange and View tabs with their tools,
//! and the "Search commands" box on the right.

use egui::accesskit::Role;
use egui::{Ui, WidgetInfo};
use flatcraft_engine::registry;
use serde_json::json;

use super::arrange::{self, PanelKind};
use super::canvas::{CanvasView, PX_PER_MM_100};
use super::collapse::{self, Group, Mode};
use super::icons::{self, Icon};
use super::prepare::{PrepareState, RibbonTab, Snapshot};
use super::settings::set_name;
use super::{Action, ICON_SIZE};

fn icon_image(ui: &Ui, icon: Icon) -> egui::Image<'static> {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ))
}

/// A ribbon button: icon and text, or only the icon (`Mode::Icons`, the
/// text is its accessible name and leads its tooltip), or a row of the
/// overflow list (`Mode::List`, which closes the list when clicked).
/// `selected` makes it a toggle.
pub(crate) fn button(
    ui: &mut Ui,
    mode: Mode,
    icon: Icon,
    text: &str,
    enabled: bool,
    selected: Option<bool>,
    hint: &str,
) -> egui::Response {
    let image = icon_image(ui, icon);
    let mut b = match mode {
        Mode::Icons => egui::Button::image(image.alt_text(text)),
        Mode::Full | Mode::List => egui::Button::image_and_text(image, text),
    };
    if let Some(on) = selected {
        b = b.selected(on);
    }
    let resp = ui.add_enabled(enabled, b);
    let hint = if mode == Mode::Icons && hint != text {
        format!("{text}\n{hint}")
    } else {
        hint.to_string()
    };
    let resp = resp.on_hover_text(&hint).on_disabled_hover_text(&hint);
    if mode == Mode::List && selected.is_none() && resp.clicked() {
        ui.close();
    }
    resp
}

/// A button with an icon and a text.
fn tool(ui: &mut Ui, mode: Mode, icon: Icon, text: &str, enabled: bool, hint: &str) -> bool {
    button(ui, mode, icon, text, enabled, None, hint).clicked()
}

/// A toggle with an icon and a text.
pub(crate) fn toggle(ui: &mut Ui, mode: Mode, icon: Icon, text: &str, on: &mut bool) {
    if button(ui, mode, icon, text, true, Some(*on), text).clicked() {
        *on = !*on;
    }
}

/// The width a field needs to show `text` whole, with its frame.
pub(crate) fn field_width(ui: &Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, egui::Color32::PLACEHOLDER);
    // The frame's margin on both sides, and room for the text cursor.
    galley.size().x + 2.0 * FIELD_MARGIN.x + 4.0
}

/// The inner margin of the ribbon's number fields.
const FIELD_MARGIN: egui::Vec2 = egui::vec2(4.0, 2.0);

/// The widest value a millimetre field of the ribbon shows as wide as it
/// is: 4 digits and 2 decimals, more than any mat.
const WIDEST_MM: &str = "-0000.00";

/// A short number field. Shows `current` while it is not being edited and
/// returns the typed number when Enter is pressed. It is always as wide as
/// its text, and at least as wide as [`WIDEST_MM`], so no digit is cut.
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
    let width = field_width(ui, buf).max(field_width(ui, WIDEST_MM));
    let resp = ui.add_enabled(
        enabled,
        egui::TextEdit::singleline(buf)
            .id(id)
            .margin(egui::Margin::symmetric(
                FIELD_MARGIN.x as i8,
                FIELD_MARGIN.y as i8,
            ))
            .desired_width(width)
            .min_size(egui::vec2(width, 0.0)),
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
        // With one object selected, picking these opens the Trace dialog or
        // the Rectangular pattern panel (see `pick`).
        "object.retrace" | "object.copies" => Tab(RibbonTab::Home),
        // The Arrange tab has these; with a selection, Circular pattern
        // opens its panel (see `pick`).
        "object.scale"
        | "object.rotate"
        | "object.mirror"
        | "arrange.align"
        | "arrange.distribute"
        | "object.circular_copies" => Tab(RibbonTab::Arrange),
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

/// The linked objects among `ids` whose file is not broken, for Reload.
/// `None` when there are none.
pub fn reload_ids_of(
    snap: &Snapshot,
    sources: &super::prepare::SourceStatus,
    ids: &[u64],
) -> Option<Vec<u64>> {
    let ids: Vec<u64> = ids
        .iter()
        .filter_map(|id| snap.object(*id))
        .filter(|o| o.link.is_some() && !sources.broken(o.id))
        .map(|o| o.id)
        .collect();
    (!ids.is_empty()).then_some(ids)
}

/// The id of the selected object when it is the only one and an image.
pub fn single_raster(snap: &Snapshot) -> Option<u64> {
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    single_raster_of(snap, &ids)
}

/// The id in `ids` when it is the only one and an image.
pub fn single_raster_of(snap: &Snapshot, ids: &[u64]) -> Option<u64> {
    match ids {
        [id] => snap.object(*id).filter(|o| o.raster).map(|o| o.id),
        _ => None,
    }
}

/// The panel a search hit opens on this selection, if any: Copies opens
/// the Rectangular pattern panel for one object, Rotate and Circular
/// pattern open theirs for any selection.
fn search_panel(id: &str, snap: &Snapshot) -> Option<PanelKind> {
    match id {
        "object.copies" if snap.selection.len() == 1 => Some(PanelKind::RectangularPattern),
        "object.circular_copies" if !snap.selection.is_empty() => Some(PanelKind::CircularPattern),
        "object.rotate" if !snap.selection.is_empty() => Some(PanelKind::Rotate),
        _ => None,
    }
}

/// Search hits for arrange commands that the tab may not show (they may
/// not be pinned): they open the Arrange drop-down, which lists Mirror,
/// Align and Distribute with each direction, and Rotate and Circular
/// pattern with what they still need when nothing is selected.
fn search_reveals(id: &str) -> bool {
    matches!(
        id,
        "object.mirror"
            | "arrange.align"
            | "arrange.distribute"
            | "object.rotate"
            | "object.circular_copies"
    )
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
    if let Some(kind) = search_panel(id, snap) {
        // Like the ribbon buttons; else the tab shows where it is.
        arrange::open(st, snap, kind);
        return;
    }
    if search_reveals(id) {
        arrange::reveal(st);
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

/// Width of the search box.
pub const SEARCH_WIDTH: f32 = 200.0;

/// What the ribbon's groups draw with.
pub(crate) struct Cx<'a> {
    pub st: &'a mut PrepareState,
    pub snap: &'a Snapshot,
    pub show_debug: &'a mut bool,
    pub actions: &'a mut Vec<Action>,
}

/// The id of the ribbon's tool bar, for [`collapse::level`].
pub fn bar_id() -> egui::Id {
    egui::Id::unique("flatcraft-ribbon-tools")
}

/// The tab row with the search box at its right, and below them the tools
/// of the current tab across the whole width. The search box is drawn
/// first and never moves, so it is always reachable.
pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    show_debug: &mut bool,
    actions: &mut Vec<Action>,
) {
    let full = ui.available_rect_before_wrap();
    let search = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(egui::Layout::right_to_left(egui::Align::Min)),
            |ui| search_box(ui, snap, st, actions),
        )
        .response
        .rect;
    let gap = ui.spacing().item_spacing.x;
    let tab_row = egui::Rect::from_min_max(
        full.min,
        egui::pos2((search.left() - gap).max(full.left()), full.bottom()),
    );
    let tabs_rect = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(tab_row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| tabs(ui, st),
        )
        .response
        .rect;
    let top = tabs_rect.bottom().max(search.bottom()) + ui.spacing().item_spacing.y;
    let tools = egui::Rect::from_min_max(egui::pos2(full.left(), top), full.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(tools)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            // As tall as the Arrange tab's big buttons on every tab, so
            // the ribbon does not jump.
            ui.set_min_height(arrange::BIG_HEIGHT);
            let groups = groups(st.tab, snap);
            let sig = signature(st.tab, snap);
            let mut cx = Cx {
                st,
                snap,
                show_debug,
                actions,
            };
            collapse::show(ui, bar_id(), sig, arrange::BIG_HEIGHT, &groups, &mut cx);
        },
    );
}

/// What the widths of the tools depend on, besides the window.
fn signature(tab: RibbonTab, snap: &Snapshot) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tab.hash(&mut h);
    if tab == RibbonTab::Arrange {
        snap.arrange_pins.hash(&mut h);
        snap.linked_reach().hash(&mut h);
    }
    h.finish()
}

/// The groups of a tab, left to right.
fn groups(tab: RibbonTab, snap: &Snapshot) -> Vec<Group<Cx<'_>>> {
    let g = |title, keep, draw: fn(&mut Ui, Mode, &mut Cx)| Group { title, keep, draw };
    match tab {
        RibbonTab::Home => vec![
            g("Add", false, home_add),
            g("Edit", false, home_edit),
            g("Linked files", false, home_files),
        ],
        RibbonTab::Arrange => {
            let pins = arrange::pinned(snap);
            let mut v = vec![
                g("Size", false, arrange_size),
                // Search opens its drop-down, so it stays on the ribbon.
                g("Arrange", true, |ui, mode, cx| {
                    arrange::dropdown_button(ui, mode, cx.st, cx.snap, cx.actions)
                }),
            ];
            if pins.iter().any(|c| c.big()) {
                v.push(g("Pinned", false, |ui, mode, cx| {
                    arrange::big_pins(ui, mode, cx.st, cx.snap, cx.actions)
                }));
            }
            if pins.iter().any(|c| !c.big()) {
                v.push(g("Pinned directions", false, |ui, mode, cx| {
                    arrange::small_pins(ui, mode, cx.st, cx.snap, cx.actions)
                }));
            }
            v.push(g("Rotate and mirror", false, arrange_each));
            v
        }
        RibbonTab::View => vec![
            g("Zoom", false, view_zoom),
            g("Show", false, view_show),
            g("Panels", false, view_panels),
        ],
    }
}

fn home_add(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let doc_id = cx.snap.doc_id;
    if tool(
        ui,
        mode,
        Icon::Import,
        "Import",
        true,
        "Import an SVG or an image",
    ) {
        cx.actions.push(Action::Import { doc_id });
    }
    let raster = single_raster(cx.snap);
    if tool(
        ui,
        mode,
        Icon::Trace,
        "Trace",
        raster.is_some(),
        "Trace the selected image with your own settings",
    ) && let Some(id) = raster
    {
        cx.actions.push(Action::TraceDialog { doc_id, id });
    }
    if tool(
        ui,
        mode,
        Icon::TestPatterns,
        "Test patterns",
        true,
        "Add the test sheet. Cut it first on a new machine.",
    ) {
        cx.actions
            .push(Action::run(doc_id, "tools.test_patterns", json!({})));
    }
}

fn home_edit(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let doc_id = cx.snap.doc_id;
    let selected: Vec<u64> = cx.snap.selection.iter().copied().collect();
    let any = !selected.is_empty();
    if tool(
        ui,
        mode,
        Icon::Duplicate,
        "Duplicate",
        any,
        "Copy the selection",
    ) {
        cx.actions.push(duplicate_action(doc_id, &selected));
    }
    let one = selected.len() == 1;
    if tool(
        ui,
        mode,
        Icon::Copies,
        "Copies...",
        one,
        if one {
            "Many copies of one object in columns and rows. Sticker sheet, engage!"
        } else {
            "Many copies of one object in columns and rows. Select one object first."
        },
    ) {
        arrange::open(cx.st, cx.snap, PanelKind::RectangularPattern);
    }
    if tool(
        ui,
        mode,
        Icon::Delete,
        "Delete",
        any,
        "Delete the selection",
    ) {
        cx.actions.push(delete_action(doc_id, &selected));
    }
}

fn home_files(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let ids = reload_ids(cx.snap);
    if tool(
        ui,
        mode,
        Icon::Reload,
        "Reload",
        ids.is_some(),
        "Read the linked files again (the selection, or all)",
    ) && let Some(ids) = ids
    {
        cx.actions.push(reload_action(cx.snap.doc_id, ids));
    }
}

fn view_zoom(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let st = &mut *cx.st;
    if tool(
        ui,
        mode,
        Icon::ZoomFit,
        "Zoom to fit",
        true,
        "Show the whole mat",
    ) {
        st.fit_pending = true;
    }
    if tool(
        ui,
        mode,
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
}

fn view_show(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    toggle(ui, mode, Icon::Grid, "Grid", &mut cx.st.show_grid);
    toggle(ui, mode, Icon::Ruler, "Rulers", &mut cx.st.show_rulers);
}

fn view_panels(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let st = &mut *cx.st;
    toggle(
        ui,
        mode,
        Icon::PanelLeft,
        "Objects panel",
        &mut st.show_objects,
    );
    toggle(
        ui,
        mode,
        Icon::PanelRight,
        "Settings panel",
        &mut st.show_settings,
    );
    toggle(ui, mode, Icon::Debug, "Debug panel", cx.show_debug);
}

/// The `object.rotate` of a rotate button, about each object or the group.
pub(crate) fn rotate_action(doc_id: u64, ids: &[u64], degrees: f64, each: bool) -> Action {
    Action::run(
        doc_id,
        "object.rotate",
        json!({ "ids": ids, "degrees": degrees, "about": about(each) }),
    )
}

/// The `object.mirror` of a mirror button; `axis` is "horizontal" or
/// "vertical".
pub(crate) fn mirror_action(doc_id: u64, ids: &[u64], axis: &str, each: bool) -> Action {
    Action::run(
        doc_id,
        "object.mirror",
        json!({ "ids": ids, "axis": axis, "about": about(each) }),
    )
}

fn about(each: bool) -> &'static str {
    if each { "each" } else { "group" }
}

/// `(mode, label)` of the Align items.
pub(crate) const ALIGN_MODES: [(&str, &str); 6] = [
    ("left", "Left"),
    ("hcenter", "Centre"),
    ("right", "Right"),
    ("top", "Top"),
    ("vcenter", "Middle"),
    ("bottom", "Bottom"),
];

/// The `arrange.align` of an Align item.
pub(crate) fn align_action(doc_id: u64, ids: &[u64], mode: &str, to_area: bool) -> Action {
    let to = if to_area { "cutting_area" } else { "selection" };
    Action::run(
        doc_id,
        "arrange.align",
        json!({ "ids": ids, "mode": mode, "relative_to": to }),
    )
}

/// The items inside an Align menu: what to align to, then the six modes.
/// Used by the ribbon and the context menu.
pub(crate) fn align_items(
    ui: &mut Ui,
    to_area: &mut bool,
    doc_id: u64,
    ids: &[u64],
    actions: &mut Vec<Action>,
) {
    ui.radio_value(to_area, false, "To the selection");
    ui.radio_value(to_area, true, "To the cutting area");
    ui.separator();
    for (mode, text) in ALIGN_MODES {
        if ui.button(text).clicked() {
            actions.push(align_action(doc_id, ids, mode, *to_area));
            ui.close();
        }
    }
}

/// `(label, axis)` of the Distribute commands.
pub(crate) const DISTRIBUTE: [(&str, &str); 2] = [
    ("Distribute horizontally", "horizontal"),
    ("Distribute vertically", "vertical"),
];

/// The `arrange.distribute` of a Distribute button.
pub(crate) fn distribute_action(doc_id: u64, ids: &[u64], axis: &str) -> Action {
    Action::run(
        doc_id,
        "arrange.distribute",
        json!({ "ids": ids, "axis": axis }),
    )
}

/// The `object.duplicate` of the Duplicate button.
pub(crate) fn duplicate_action(doc_id: u64, ids: &[u64]) -> Action {
    Action::run(doc_id, "object.duplicate", json!({ "ids": ids }))
}

/// The `object.delete` of the Delete button.
pub(crate) fn delete_action(doc_id: u64, ids: &[u64]) -> Action {
    Action::run(doc_id, "object.delete", json!({ "ids": ids }))
}

/// The `object.reload` of the Reload button for `ids`.
pub(crate) fn reload_action(doc_id: u64, ids: Vec<u64>) -> Action {
    Action::run(doc_id, "object.reload", json!({ "ids": ids }))
}

/// A checkbox; icon-only it drops its text, which stays its accessible
/// name and leads its tooltip.
fn checkbox(
    ui: &mut Ui,
    mode: Mode,
    on: &mut bool,
    text: &str,
    enabled: bool,
    hint: &str,
) -> egui::Response {
    let resp = if mode == Mode::Icons {
        let r = ui.add_enabled(enabled, egui::Checkbox::without_text(on));
        set_name(ui, &r, text);
        r
    } else {
        ui.add_enabled(enabled, egui::Checkbox::new(on, text))
    };
    let hint = match (mode, hint.is_empty()) {
        (Mode::Icons, true) => text.to_string(),
        (Mode::Icons, false) => format!("{text}\n{hint}"),
        _ => hint.to_string(),
    };
    if hint.is_empty() {
        resp
    } else {
        resp.on_hover_text(&hint).on_disabled_hover_text(&hint)
    }
}

/// The size fields: W, H, Keep aspect and %. In the overflow list they
/// stack in rows.
fn arrange_size(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    if mode == Mode::List {
        ui.horizontal(|ui| size_wh(ui, cx));
        ui.horizontal(|ui| size_rest(ui, mode, cx));
    } else {
        size_wh(ui, cx);
        size_rest(ui, mode, cx);
    }
}

fn size_wh(ui: &mut Ui, cx: &mut Cx) {
    let snap = cx.snap;
    let doc_id = snap.doc_id;
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let any = !ids.is_empty();
    let single = ids.len() == 1;
    let b = super::canvas::selection_box(snap);
    let f = &mut cx.st.fields;
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
            cx.actions.push(Action::run(
                doc_id,
                "object.scale",
                json!({ "ids": ids, key: target, "keep_aspect": f.keep_aspect }),
            ));
        } else if let Some(have) = have.filter(|h| *h > 1e-9) {
            // Several objects: one factor from the common box, which
            // scales as one group so its layout stays.
            cx.actions.push(Action::run(
                doc_id,
                "object.scale",
                json!({ "ids": ids, "factor": target / have, "group": true }),
            ));
        }
    }
}

fn size_rest(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let snap = cx.snap;
    let doc_id = snap.doc_id;
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let any = !ids.is_empty();
    let single = ids.len() == 1;
    let f = &mut cx.st.fields;
    if ids.len() > 1 {
        let mut on = true;
        checkbox(
            ui,
            mode,
            &mut on,
            "Keep aspect",
            false,
            "Several objects scale together, keeping their shape.",
        );
    } else {
        checkbox(ui, mode, &mut f.keep_aspect, "Keep aspect", any, "");
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
        cx.actions.push(Action::run(
            doc_id,
            "object.scale",
            json!({ "ids": ids, "factor": p / 100.0, "group": !single }),
        ));
    }
    // Linked copies share the scale.
    if let Some(n) = snap.linked_reach() {
        let text = format!("Scales all {n} linked copies");
        let hint = "Linked copies share the scale. Each keeps its own place.";
        if mode == Mode::Icons {
            let r = ui
                .add(
                    icon_image(ui, Icon::Linked)
                        .tint(ui.visuals().weak_text_color())
                        .sense(egui::Sense::hover()),
                )
                .on_hover_text(format!("{text}. {hint}"));
            r.widget_info(|| WidgetInfo::labeled(Role::Label, true, &text));
        } else {
            ui.label(egui::RichText::new(text).weak())
                .on_hover_text(hint);
        }
    }
}

/// The "Each object" switch for Rotate and Mirror.
fn arrange_each(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    // Several objects turn and flip as one group, or each on its own.
    let several = cx.snap.selection.len() > 1;
    let hint = if several {
        "Rotate and mirror each object about its own centre. Off: the selection turns as one, like Voltron."
    } else {
        "Rotate and mirror each object about its own centre. Select several objects first."
    };
    let f = &mut cx.st.fields;
    if checkbox(ui, mode, &mut f.each, "Each object", several, hint).changed() {
        cx.actions.push(Action::RememberEach(f.each));
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
