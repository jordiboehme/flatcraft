//! The Prepare ribbon: the Home, Arrange and View tabs with their groups
//! (see [`groups`] and the table in [`tools`]), and the "Search commands"
//! box on the right.

use egui::accesskit::Role;
use egui::{Ui, WidgetInfo};
use flatcraft_engine::registry;
use serde_json::json;

use super::Action;
use super::collapse::{self, Group, Mode};
use super::groups::{self, Cx, Extras};
use super::icons::Icon;
use super::prepare::{PrepareState, RibbonTab, Snapshot};
use super::settings::set_name;
use super::tools::{self, GroupId, Setting, Tool};

/// A toggle with an icon and a text, with its name as its tooltip (the
/// Cut bar's Debug panel switch).
pub(crate) fn toggle(ui: &mut Ui, mode: Mode, icon: Icon, text: &str, on: &mut bool) {
    let r = groups::wide_button(ui, mode, icon, text, true, Some(*on)).on_hover_text(text);
    if r.clicked() {
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
#[allow(clippy::too_many_arguments)]
fn number_field(
    ui: &mut Ui,
    id_salt: &str,
    label: &str,
    name: &str,
    setting: Setting,
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
    let why = (!enabled).then_some("Select something first.");
    let l = ui.label(label);
    groups::card(&l, name, setting.card(), why, false);
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
    groups::card(&resp, name, setting.card(), why, false);
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

/// One search hit: an action of the ribbon table, or a registry command
/// that no action covers.
#[derive(Debug, Clone, Copy)]
pub enum Hit {
    Tool(&'static Tool),
    Command {
        id: &'static str,
        title: &'static str,
    },
}

impl Hit {
    /// What the hit list shows: the table's title for an action.
    pub fn title(&self) -> String {
        match self {
            Hit::Tool(t) => t.full_title(),
            Hit::Command { title, .. } => title.to_string(),
        }
    }

    /// The registry command it runs, if any.
    pub fn command(&self) -> Option<&'static str> {
        match self {
            Hit::Tool(t) => t.command(),
            Hit::Command { id, .. } => Some(id),
        }
    }
}

/// Everything search can find: the actions of the table (with their
/// titles) and the commands of [`search_target`] that no action covers.
fn candidates(doc_id: u64) -> Vec<Hit> {
    let covered: Vec<&str> = tools::TOOLS.iter().filter_map(Tool::command).collect();
    let mut out: Vec<Hit> = tools::TOOLS.iter().map(Hit::Tool).collect();
    out.extend(
        registry::all()
            .iter()
            .filter(|c| search_target(c.id(), doc_id).is_some() && !covered.contains(&c.id()))
            .map(|c| Hit::Command {
                id: c.id(),
                title: c.title(),
            }),
    );
    out
}

/// What matches the query, best first.
pub fn search(query: &str, doc_id: u64) -> Vec<Hit> {
    let mut hits: Vec<(i32, String, Hit)> = candidates(doc_id)
        .into_iter()
        .filter_map(|h| {
            let title = h.title();
            let by_id = h
                .command()
                .and_then(|id| fuzzy_score(query, id))
                .map(|s| s - 1);
            let score = fuzzy_score(query, &title).max(by_id);
            score.map(|s| (s, title, h))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, _, h)| h).collect()
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

/// Shows the group's tab with its list open, where every action of the
/// group is, dimmed with its reason when it cannot run now.
pub fn reveal(st: &mut PrepareState, group: GroupId) {
    st.tab = group.def().tab;
    st.open_group = Some(group);
}

/// What picking a search hit does: an action runs as its ribbon button
/// does; one that cannot run now opens its group's list, which says why.
fn pick(hit: Hit, cx: &mut Cx) {
    match hit {
        Hit::Tool(t) => {
            if t.blocked(cx.snap).is_some() {
                reveal(cx.st, t.group);
            } else {
                t.run(cx.snap, cx.st, cx.show_debug, cx.actions);
            }
        }
        Hit::Command { id, .. } => match search_target(id, cx.snap.doc_id) {
            Some(SearchTarget::Run) => cx.actions.push(Action::run(cx.snap.doc_id, id, json!({}))),
            Some(SearchTarget::Tab(tab)) => cx.st.tab = tab,
            Some(SearchTarget::Settings) => cx.st.show_settings = true,
            Some(SearchTarget::Action(a)) => cx.actions.push(a),
            None => {}
        },
    }
}

fn search_box(ui: &mut Ui, cx: &mut Cx) {
    let id = ui.make_persistent_id("flatcraft-search");
    let st = &mut *cx.st;
    let resp = ui.add(
        egui::TextEdit::singleline(&mut st.fields.search)
            .id(id)
            .hint_text("Search commands")
            .desired_width(SEARCH_WIDTH),
    );
    set_name(ui, &resp, "Search commands");
    groups::card(
        &resp,
        "Search commands",
        Setting::Search.card(),
        None,
        false,
    );
    let hits = search(&st.fields.search, cx.snap.doc_id);
    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        cx.st.fields.search.clear();
        if let Some(hit) = hits.first() {
            pick(*hit, cx);
        }
        return;
    }
    // A press on a hit takes the focus from the field before the click
    // lands, so the list stays open while the pointer is over it.
    let open = !hits.is_empty() && (resp.has_focus() || st.search_open);
    st.search_open = false;
    if open {
        let mut chosen = None;
        let popup = egui::Popup::from_response(&resp).open(true).show(|ui| {
            ui.set_min_width(SEARCH_WIDTH);
            for hit in hits.iter().take(8) {
                let title = hit.title();
                let r = ui.selectable_label(false, title.as_str());
                set_name(ui, &r, &format!("{title} (search)"));
                if r.clicked() {
                    chosen = Some(*hit);
                }
            }
        });
        let hovered = popup.is_some_and(|p| p.response.contains_pointer());
        cx.st.search_open = resp.has_focus() || hovered;
        if let Some(hit) = chosen {
            cx.st.fields.search.clear();
            cx.st.search_open = false;
            pick(hit, cx);
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

/// The id of the ribbon's tool bar, for [`collapse::level`].
pub fn bar_id() -> egui::Id {
    egui::Id::unique("flatcraft-ribbon-tools")
}

/// The tab row with the search box at its right, and below them the groups
/// of the current tab across the whole width. The search box is drawn
/// first and never moves, so it is always reachable.
pub fn show(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    show_debug: &mut bool,
    actions: &mut Vec<Action>,
) {
    let mut cx = Cx {
        st,
        snap,
        show_debug,
        actions,
    };
    // Small corners and tight padding, so the groups use their room.
    super::theme::tight(ui.style_mut());
    let full = ui.available_rect_before_wrap();
    let search = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(egui::Layout::right_to_left(egui::Align::Min)),
            |ui| search_box(ui, &mut cx),
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
            |ui| tabs(ui, cx.st),
        )
        .response
        .rect;
    let top = tabs_rect.bottom().max(search.bottom()) + ui.spacing().item_spacing.y;
    let tools_rect = egui::Rect::from_min_max(egui::pos2(full.left(), top), full.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(tools_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            // As tall on every tab, so the ribbon does not jump.
            ui.set_min_height(groups::ROW_HEIGHT);
            let tab = cx.st.tab;
            let bar = bar_groups(ui, tab);
            let sig = signature(tab, &cx);
            collapse::show(ui, bar_id(), sig, groups::ROW_HEIGHT, &bar, &mut cx);
        },
    );
}

/// What the widths of the groups depend on, besides the window: the tab,
/// the pins, the linked-copies note and how long the size fields' texts
/// are (a field grows with its text).
fn signature(tab: RibbonTab, cx: &Cx) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tab.hash(&mut h);
    cx.snap.ribbon_pins.hash(&mut h);
    if tab == RibbonTab::Arrange {
        cx.snap.linked_reach().hash(&mut h);
        let f = &cx.st.fields;
        for text in [&f.width, &f.height, &f.percent] {
            text.chars().count().max(WIDEST_MM.len()).hash(&mut h);
        }
    }
    h.finish()
}

/// The groups of a tab, left to right, for the collapsing bar.
fn bar_groups<'a>(ui: &Ui, tab: RibbonTab) -> Vec<Group<Cx<'a>>> {
    let g = |id: GroupId, draw: fn(&mut Ui, Mode, &mut Cx)| {
        let title = id.def().title;
        let keep = id == GroupId::Size;
        Group {
            title,
            keep,
            tile: (!keep).then(|| groups::tile_width(ui, title)),
            draw,
        }
    };
    tools::groups_of(tab)
        .map(|def| match def.id {
            GroupId::Add => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Add, cx)),
            GroupId::Edit => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Edit, cx)),
            GroupId::Files => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Files, cx)),
            GroupId::Size => g(def.id, size_group),
            GroupId::Arrange => g(def.id, |ui, m, cx| {
                let extras = Extras {
                    inline: Some(each_switch),
                    in_list: Some(|ui, cx| each_switch(ui, Mode::Full, cx)),
                };
                groups::show(ui, m, GroupId::Arrange, extras, cx)
            }),
            GroupId::Zoom => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Zoom, cx)),
            GroupId::Show => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Show, cx)),
            GroupId::Panels => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Panels, cx)),
        })
        .collect()
}

/// A group with only its actions.
fn plain(ui: &mut Ui, mode: Mode, group: GroupId, cx: &mut Cx) {
    groups::show(ui, mode, group, Extras::default(), cx);
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

/// The `arrange.align` of an Align action.
pub(crate) fn align_action(doc_id: u64, ids: &[u64], mode: &str, to_area: bool) -> Action {
    let to = if to_area { "cutting_area" } else { "selection" };
    Action::run(
        doc_id,
        "arrange.align",
        json!({ "ids": ids, "mode": mode, "relative_to": to }),
    )
}

/// The `arrange.distribute` of a Distribute action.
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

/// A switch that is not an action ("Keep aspect", "Each object"): a
/// checkbox with its text, or an icon toggle when the ribbon is short.
/// Either way it is a checkbox named `text` with its state, and its card
/// says what it does (and why it is dimmed).
fn switch(
    ui: &mut Ui,
    mode: Mode,
    on: &mut bool,
    icon: Icon,
    text: &str,
    setting: Setting,
    why: Option<&str>,
) -> egui::Response {
    let enabled = why.is_none();
    let resp = match mode {
        Mode::Icons | Mode::Collapsed => {
            let mut r = groups::wide_button(ui, Mode::Icons, icon, text, enabled, Some(*on));
            if r.clicked() {
                // Like a checkbox: the caller sees the change.
                *on = !*on;
                r.mark_changed();
            }
            r
        }
        Mode::Full | Mode::List => ui.add_enabled(enabled, egui::Checkbox::new(on, text)),
    };
    resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, *on, text));
    groups::card(&resp, text, setting.card(), why, false);
    resp
}

/// The Size group: W, H, Keep aspect and %, with its title under them. It
/// has no actions and never collapses.
fn size_group(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let def = GroupId::Size.def();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = groups::GROUP_GAP;
        let top = ui
            .horizontal(|ui| {
                ui.set_min_height(groups::BIG_HEIGHT);
                size_wh(ui, cx);
                size_rest(ui, mode, cx);
            })
            .response
            .rect;
        // Its title, like a group button, but nothing to open.
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(top.width(), groups::GROUP_BUTTON_HEIGHT),
            egui::Sense::hover(),
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            def.title,
            egui::TextStyle::Small.resolve(ui.style()),
            ui.visuals().weak_text_color(),
        );
        groups::card(&resp, def.title, def.description, None, false);
    });
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
        Setting::Width,
        &mut f.width,
        Some(b.map(|b| mm(b.width())).unwrap_or_default()),
        any,
    );
    let h = number_field(
        ui,
        "scale-height",
        "H",
        "Height (mm)",
        Setting::Height,
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
        switch(
            ui,
            mode,
            &mut on,
            Icon::KeepAspect,
            "Keep aspect",
            Setting::KeepAspect,
            Some("Several objects always scale together and keep their shape."),
        );
    } else {
        switch(
            ui,
            mode,
            &mut f.keep_aspect,
            Icon::KeepAspect,
            "Keep aspect",
            Setting::KeepAspect,
            (!any).then_some("Select something first."),
        );
    }
    if let Some(p) = number_field(
        ui,
        "scale-percent",
        "%",
        "Scale (%)",
        Setting::ScalePercent,
        &mut f.percent,
        Some(if any { "100.00".into() } else { String::new() }),
        any,
    ) && p > 0.0
    {
        // Each object scales about its own centre when the switch is on.
        cx.actions.push(Action::run(
            doc_id,
            "object.scale",
            json!({ "ids": ids, "factor": p / 100.0, "group": !single && !f.each }),
        ));
    }
    // Linked copies share the scale.
    if let Some(n) = snap.linked_reach() {
        let text = format!("Scales all {n} linked copies");
        let hint = "Linked copies share the scale. Each keeps its own place.";
        if mode == Mode::Full {
            let r = ui.label(egui::RichText::new(&text).weak());
            groups::card(&r, &text, hint, None, false);
        } else {
            let tex = super::icons::texture(ui.ctx(), Icon::Linked, super::ICON_SIZE);
            let r = ui.add(
                egui::Image::from_texture(egui::load::SizedTexture::new(
                    tex.id(),
                    egui::Vec2::splat(super::ICON_SIZE),
                ))
                .tint(ui.visuals().weak_text_color())
                .sense(egui::Sense::hover()),
            );
            r.widget_info(|| WidgetInfo::labeled(Role::Label, true, &text));
            groups::card(&r, &text, hint, None, false);
        }
    }
}

/// The "Each object" switch for Rotate, Mirror and the % field.
fn each_switch(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    // Several objects turn and flip as one group, or each on its own.
    let several = cx.snap.selection.len() > 1;
    let why = (!several)
        .then_some("Select 2 or more objects first. One object always turns about its own centre.");
    let f = &mut cx.st.fields;
    if switch(
        ui,
        mode,
        &mut f.each,
        Icon::EachObject,
        "Each object",
        Setting::EachObject,
        why,
    )
    .changed()
    {
        cx.actions.push(Action::RememberEach(f.each));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyphens_and_case_do_not_matter() {
        let hits = search("Auto-arrange", 1);
        assert_eq!(hits.first().and_then(Hit::command), Some("arrange.auto"));
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
    fn search_shows_the_titles_of_the_table() {
        // Every action is found by its own title, as the table names it.
        for t in tools::TOOLS {
            let hits = search(&t.full_title(), 1);
            assert!(
                hits.iter()
                    .any(|h| matches!(h, Hit::Tool(x) if x.id == t.id)),
                "{}",
                t.id
            );
        }
        // A command an action covers is listed as the action, never twice.
        let mirror = search("mirror", 1);
        assert!(mirror.iter().all(|h| matches!(h, Hit::Tool(_))));
        let titles: Vec<String> = mirror.iter().map(Hit::title).collect();
        assert!(
            titles.contains(&"Mirror horizontally".to_string()),
            "{titles:?}"
        );
    }

    #[test]
    fn searching_never_sends_a_job() {
        assert!(
            search("send", 1)
                .iter()
                .all(|h| h.command() != Some("job.send"))
        );
        assert!(search_target("job.send", 1).is_none());
    }
}
