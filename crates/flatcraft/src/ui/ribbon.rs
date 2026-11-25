//! The Prepare ribbon: the Layout and View tabs with their groups
//! (see [`groups`] and the table in [`tools`]), and the "Search commands"
//! box on the right.

use egui::accesskit::Role;
use egui::{Ui, WidgetInfo};
use flatcraft_engine::registry;
use serde_json::json;

use super::Action;
use super::canvas;
use super::collapse::{self, Group, Mode};
use super::groups::{self, Cx, Extras};
use super::icons::Icon;
use super::prepare::{FieldEdit, Fields, PrepareState, RibbonTab, SizeField, Snapshot};
use super::rails::SidePanel;
use super::settings::set_name;
use super::theme::Tokens;
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

/// A short number field of the Position and size group.
struct NumberField<'a> {
    field: SizeField,
    label: &'a str,
    /// Its accessible name and card title.
    name: &'a str,
    setting: Setting,
    /// What it shows while it is not being edited.
    current: String,
    enabled: bool,
}

impl SizeField {
    fn id_salt(self) -> &'static str {
        match self {
            SizeField::X => "position-x",
            SizeField::Y => "position-y",
            SizeField::Width => "scale-width",
            SizeField::Height => "scale-height",
            SizeField::Percent => "scale-percent",
        }
    }

    /// Its widget id, the same wherever the group is drawn.
    fn id(self) -> egui::Id {
        egui::Id::unique(("flatcraft-size-field", self.id_salt()))
    }

    fn buf(self, f: &mut Fields) -> &mut String {
        match self {
            SizeField::X => &mut f.x,
            SizeField::Y => &mut f.y,
            SizeField::Width => &mut f.width,
            SizeField::Height => &mut f.height,
            SizeField::Percent => &mut f.percent,
        }
    }
}

/// Draws a number field. It shows `current` while it is not being edited
/// and never overwrites what the person types. The typed value counts when
/// the field loses the focus (Tab, Shift-Tab, Enter, a click elsewhere),
/// see [`finish_edit`]; Escape puts the shown value back. It is always as
/// wide as its text, and at least as wide as [`WIDEST_MM`], so no digit is
/// cut.
fn number_field(ui: &mut Ui, nf: NumberField, cx: &mut Cx) {
    let id = nf.field.id();
    let focused = ui.memory(|m| m.has_focus(id));
    let fields = &mut cx.st.fields;
    let editing = fields.editing.as_ref().is_some_and(|e| e.field == nf.field);
    if !focused && !editing {
        *nf.field.buf(fields) = nf.current.clone();
    } else if editing {
        // Nothing typed yet (the focus came by Tab, say): it follows the
        // value, which a field applied in the same frame may have changed.
        // It also follows the selection, so a Tab out of it never moves
        // what was selected before.
        let shown = fields.editing.as_ref().map(|e| e.shown.clone());
        if shown.as_deref() == Some(nf.field.buf(fields).as_str()) {
            *nf.field.buf(fields) = nf.current.clone();
            if let Some(e) = fields.editing.as_mut() {
                e.shown = nf.current.clone();
                e.doc_id = cx.snap.doc_id;
                e.ids = cx.snap.selection.iter().copied().collect();
                e.free = canvas::free_aspect(cx.snap, &e.ids);
            }
        }
    }
    let why = (!nf.enabled).then_some("Select something first.");
    let l = ui.label(nf.label);
    groups::card(&l, nf.name, nf.setting.card(), why, false);
    let buf = nf.field.buf(fields);
    let width = field_width(ui, buf).max(field_width(ui, WIDEST_MM));
    let resp = ui.add_enabled(
        nf.enabled,
        egui::TextEdit::singleline(buf)
            .id(id)
            .margin(egui::Margin::symmetric(
                FIELD_MARGIN.x as i8,
                FIELD_MARGIN.y as i8,
            ))
            .desired_width(width)
            .min_size(egui::vec2(width, 0.0)),
    );
    set_name(ui, &resp, nf.name);
    groups::card(&resp, nf.name, nf.setting.card(), why, false);
    if resp.has_focus() {
        if !editing {
            // Another field still editing lost the focus in this pass.
            finish_edit(ui.ctx(), cx.st, cx.actions);
            let ids: Vec<u64> = cx.snap.selection.iter().copied().collect();
            cx.st.fields.editing = Some(FieldEdit {
                field: nf.field,
                shown: nf.current,
                doc_id: cx.snap.doc_id,
                free: canvas::free_aspect(cx.snap, &ids),
                ids,
            });
        }
    } else if editing {
        finish_edit(ui.ctx(), cx.st, cx.actions);
    }
}

/// Once a frame, before the ribbon: ends the editing of a field that lost
/// the focus outside the group (a click on the canvas, another ribbon
/// tab), whose group is not on screen (`on_screen` false: another tab or
/// workspace), or whose document is no longer the active one, so its value
/// applies at once, in order and in its own document.
pub fn finish_pending(
    ctx: &egui::Context,
    st: &mut PrepareState,
    snap: &Snapshot,
    on_screen: bool,
    actions: &mut Vec<Action>,
) {
    let Some(e) = &st.fields.editing else { return };
    if !on_screen || e.doc_id != snap.doc_id || !ctx.memory(|m| m.has_focus(e.field.id())) {
        finish_edit(ctx, st, actions);
    }
}

/// Ends the editing of a field now, whether it still has the focus or
/// not: a save, close or quit counts what was typed first.
pub fn finish_now(ctx: &egui::Context, st: &mut PrepareState, actions: &mut Vec<Action>) {
    finish_edit(ctx, st, actions);
}

/// Ends the editing of the field that lost the focus: Escape puts its
/// shown text back, a changed number is applied as one command.
fn finish_edit(ctx: &egui::Context, st: &mut PrepareState, actions: &mut Vec<Action>) {
    let Some(edit) = st.fields.editing.take() else {
        return;
    };
    let buf = edit.field.buf(&mut st.fields);
    let typed = buf.trim().replace(',', ".");
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if escape || typed == edit.shown.trim() {
        *buf = edit.shown.clone();
        return;
    }
    let Ok(v) = typed.parse::<f64>() else {
        *buf = edit.shown.clone();
        return;
    };
    if edit.ids.is_empty() || !v.is_finite() {
        return;
    }
    if let Some(a) = size_action(&st.fields, &edit, v) {
        actions.push(a);
    }
}

/// The command a typed value of a Position and size field runs, in the
/// document it was typed for.
fn size_action(f: &Fields, edit: &FieldEdit, v: f64) -> Option<Action> {
    let ids = &edit.ids;
    let single = ids.len() == 1;
    let params = match edit.field {
        SizeField::X => json!({ "ids": ids, "x_mm": v }),
        SizeField::Y => json!({ "ids": ids, "y_mm": v }),
        SizeField::Percent if v > 0.0 => {
            // Each object scales about its own centre when the switch is on.
            json!({ "ids": ids, "factor": v / 100.0, "group": !single && !f.each })
        }
        SizeField::Width | SizeField::Height if v > 0.0 => {
            let key = if edit.field == SizeField::Width {
                "width_mm"
            } else {
                "height_mm"
            };
            // Several objects scale as one group, so the layout stays: the
            // common box gets the size, with its shape kept or not.
            let keep = f.keep_aspect || !edit.free;
            json!({ "ids": ids, key: v, "keep_aspect": keep, "group": !single })
        }
        _ => return None,
    };
    let command = match edit.field {
        SizeField::X | SizeField::Y => "object.move",
        _ => "object.scale",
    };
    Some(Action::RunIn {
        doc_id: edit.doc_id,
        command,
        params,
    })
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
        "object.retrace" | "object.copies" => Tab(RibbonTab::Layout),
        // The Arrange group has these; with a selection, Circular pattern
        // opens its panel (see `pick`).
        "object.scale"
        | "object.rotate"
        | "object.mirror"
        | "arrange.align"
        | "arrange.distribute"
        | "object.circular_copies" => Tab(RibbonTab::Layout),
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
            Some(SearchTarget::Settings) => cx.st.panels.set_collapsed(SidePanel::Settings, false),
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
            // The panel colour, so the field shows on the tab bar.
            .background_color(Tokens::of(ui.visuals()).panel)
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

/// Height of the tab row above the ribbon panel.
pub const TAB_ROW: f32 = 24.0;
/// Room between the ribbon panel's border and its tools.
pub const PANEL_PAD: egui::Vec2 = egui::vec2(6.0, 4.0);

/// Where the ribbon drew its parts, for the tests and the next frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub tab_row: egui::Rect,
    /// The active tab, as painted (it reaches into the panel's border).
    pub active_tab: egui::Rect,
    /// The ribbon panel under the tabs.
    pub panel: egui::Rect,
    /// The tools row inside the panel.
    pub tools: egui::Rect,
}

/// The tabs: quiet text on the tab bar, and the active one a folder tab
/// in the ribbon panel's colour that opens into the panel. Returns the
/// active tab's rect.
fn tabs(ui: &mut Ui, st: &mut PrepareState, t: &Tokens) -> egui::Rect {
    ui.spacing_mut().item_spacing.x = 2.0;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut active = egui::Rect::NOTHING;
    for tab in RibbonTab::ALL {
        let on = st.tab == tab;
        let galley = ui
            .painter()
            .layout_no_wrap(tab.name().to_string(), font.clone(), t.text);
        let size = egui::vec2(galley.size().x + 24.0, TAB_ROW - 3.0);
        let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
        r.widget_info(|| WidgetInfo::selected(Role::Tab, true, on, tab.name()));
        let radius = egui::CornerRadius {
            nw: t.ribbon_radius,
            ne: t.ribbon_radius,
            sw: 0,
            se: 0,
        };
        let painter = ui.painter();
        if on {
            // One pixel lower: it covers the panel's top border under it.
            let tab_rect = rect.with_max_y(rect.bottom() + 1.0);
            painter.rect_filled(tab_rect, radius, t.panel);
            let stroke = egui::Stroke::new(1.0, t.border);
            let (l, r_, top, bottom) = (
                tab_rect.left() + 0.5,
                tab_rect.right() - 0.5,
                tab_rect.top() + 0.5,
                tab_rect.bottom(),
            );
            painter.line_segment([egui::pos2(l, bottom), egui::pos2(l, top)], stroke);
            painter.line_segment([egui::pos2(l, top), egui::pos2(r_, top)], stroke);
            painter.line_segment([egui::pos2(r_, top), egui::pos2(r_, bottom)], stroke);
            active = tab_rect;
        } else if r.hovered() || r.has_focus() {
            painter.rect_filled(rect, radius, t.surface.gamma_multiply(0.6));
        }
        let color = if on || r.hovered() {
            t.text
        } else {
            t.text_dim
        };
        let galley = ui
            .painter()
            .layout_no_wrap(tab.name().to_string(), font.clone(), color);
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, color);
        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
        if r.clicked() {
            st.tab = tab;
        }
    }
    active
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
    let t = Tokens::of(ui.visuals());
    let full = ui.available_rect_before_wrap();
    let tab_row = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), TAB_ROW));
    // The panel is painted first, under the active tab, once its height
    // is known.
    let panel_shape = ui.painter().add(egui::Shape::Noop);
    let search = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(tab_row.with_max_y(tab_row.bottom() - 2.0))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| search_box(ui, &mut cx),
        )
        .response
        .rect;
    let gap = ui.spacing().item_spacing.x;
    let tabs_rect = egui::Rect::from_min_max(
        egui::pos2(full.left() + 2.0, tab_row.top()),
        egui::pos2((search.left() - gap).max(full.left()), tab_row.bottom()),
    );
    let active_tab = ui
        .scope_builder(
            egui::UiBuilder::new()
                .max_rect(tabs_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Max)),
            |ui| tabs(ui, cx.st, &t),
        )
        .inner;
    let tools_rect = egui::Rect::from_min_max(
        egui::pos2(full.left() + PANEL_PAD.x, tab_row.bottom() + PANEL_PAD.y),
        egui::pos2(full.right() - PANEL_PAD.x, full.bottom()),
    );
    let tools = ui
        .scope_builder(
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
        )
        .response
        .rect;
    let tools = egui::Rect::from_min_size(
        tools.min,
        egui::vec2(tools_rect.width(), groups::ROW_HEIGHT),
    );
    let panel = egui::Rect::from_min_max(
        egui::pos2(full.left(), tab_row.bottom()),
        egui::pos2(full.right(), tools.bottom() + PANEL_PAD.y),
    );
    ui.painter().set(
        panel_shape,
        egui::Shape::rect_filled(panel, t.ribbon_radius, t.panel),
    );
    // The border, but not under the active tab, which opens into it.
    let stroke = egui::Stroke::new(1.0, t.border);
    let p = ui.painter();
    let top = panel.top() + 0.5;
    let (l, r) = (panel.left() + 0.5, panel.right() - 0.5);
    let b = panel.bottom() - 0.5;
    p.line_segment([egui::pos2(l, top), egui::pos2(l, b)], stroke);
    p.line_segment([egui::pos2(l, b), egui::pos2(r, b)], stroke);
    p.line_segment([egui::pos2(r, b), egui::pos2(r, top)], stroke);
    if active_tab.is_positive() {
        p.line_segment(
            [egui::pos2(l, top), egui::pos2(active_tab.left(), top)],
            stroke,
        );
        p.line_segment(
            [egui::pos2(active_tab.right(), top), egui::pos2(r, top)],
            stroke,
        );
    } else {
        p.line_segment([egui::pos2(l, top), egui::pos2(r, top)], stroke);
    }
    ui.advance_cursor_after_rect(panel.union(tab_row));
    cx.st.ribbon_layout = Some(Layout {
        tab_row,
        active_tab,
        panel,
        tools,
    });
}

/// What the widths of the groups depend on, besides the window: the tab,
/// the pins, the linked-copies note and how long the size fields' texts
/// are (a field grows with its text).
fn signature(tab: RibbonTab, cx: &Cx) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tab.hash(&mut h);
    cx.snap.ribbon_pins.hash(&mut h);
    if tab == RibbonTab::Layout {
        cx.snap.linked_reach().hash(&mut h);
        let f = &cx.st.fields;
        for text in [&f.x, &f.y, &f.width, &f.height, &f.percent] {
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
/// compact button with its icon, and its text while the ribbon has room.
/// It is a checkbox named `text` with its state, and its card says what
/// it does (and why it is dimmed).
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
    let label = matches!(mode, Mode::Full | Mode::List);
    let mut resp = groups::compact_switch(ui, icon, text, label, enabled, *on);
    if resp.clicked() {
        // Like a checkbox: the caller sees the change.
        *on = !*on;
        resp.mark_changed();
    }
    groups::card(&resp, text, setting.card(), why, false);
    resp
}

/// The Position and size group: X, Y, W, H, % and Keep aspect, with its
/// title under them. It has no actions and never collapses.
fn size_group(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let def = GroupId::Size.def();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = groups::GROUP_GAP;
        // Three columns of two rows: X above Y, W above H, and % above
        // Keep aspect. Each is a grid, so the labels line up.
        let top = groups::pins_row(ui, |ui| {
            // Two rows fill the pins row's height, so the columns start at
            // its top (a grid does not centre itself in a row).
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), groups::BIG_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    egui::Grid::new("flatcraft-size-xy")
                        .num_columns(2)
                        .min_col_width(0.0)
                        .spacing(SIZE_GRID_GAP)
                        .show(ui, |ui| size_xy(ui, cx));
                    egui::Grid::new("flatcraft-size-wh")
                        .num_columns(2)
                        .min_col_width(0.0)
                        .spacing(SIZE_GRID_GAP)
                        .show(ui, |ui| size_wh(ui, cx));
                    egui::Grid::new("flatcraft-size-scale")
                        .num_columns(2)
                        .min_col_width(0.0)
                        .spacing(SIZE_GRID_GAP)
                        .show(ui, |ui| size_scale(ui, mode, cx));
                },
            );
            linked_note(ui, mode, cx);
        });
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

/// The first column: X above Y, the top-left corner of the selection box.
fn size_xy(ui: &mut Ui, cx: &mut Cx) {
    let any = !cx.snap.selection.is_empty();
    let b = canvas::live_box(cx.snap, cx.st);
    for (field, label, name, setting, value) in [
        (
            SizeField::X,
            "X",
            "X (mm)",
            Setting::PositionX,
            b.map(|b| b.x0),
        ),
        (
            SizeField::Y,
            "Y",
            "Y (mm)",
            Setting::PositionY,
            b.map(|b| b.y0),
        ),
    ] {
        let nf = NumberField {
            field,
            label,
            name,
            setting,
            current: value.map(mm).unwrap_or_default(),
            enabled: any,
        };
        number_field(ui, nf, cx);
        ui.end_row();
    }
}

/// The second column: W above H.
fn size_wh(ui: &mut Ui, cx: &mut Cx) {
    let any = !cx.snap.selection.is_empty();
    let b = canvas::live_box(cx.snap, cx.st);
    for (field, label, name, setting, value) in [
        (
            SizeField::Width,
            "W",
            "Width (mm)",
            Setting::Width,
            b.map(|b| b.width()),
        ),
        (
            SizeField::Height,
            "H",
            "Height (mm)",
            Setting::Height,
            b.map(|b| b.height()),
        ),
    ] {
        let nf = NumberField {
            field,
            label,
            name,
            setting,
            current: value.map(mm).unwrap_or_default(),
            enabled: any,
        };
        number_field(ui, nf, cx);
        ui.end_row();
    }
}

/// Room between the cells of the group's columns.
const SIZE_GRID_GAP: egui::Vec2 = egui::vec2(4.0, 4.0);

/// The third column: % above Keep aspect, which sits under the field.
fn size_scale(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let ids: Vec<u64> = cx.snap.selection.iter().copied().collect();
    let any = !ids.is_empty();
    let current = if any {
        mm(canvas::live_percent(cx.st).unwrap_or(100.0))
    } else {
        String::new()
    };
    let nf = NumberField {
        field: SizeField::Percent,
        label: "%",
        name: "Scale (%)",
        setting: Setting::ScalePercent,
        current,
        enabled: any,
    };
    number_field(ui, nf, cx);
    ui.end_row();
    // No label of its own: the switch sits under the % field.
    ui.allocate_exact_size(egui::Vec2::ZERO, egui::Sense::hover());
    let f = &mut cx.st.fields;
    let forced = canvas::aspect_lock(cx.snap, &ids).filter(|_| any);
    // With linked copies selected the card says how they follow.
    let card = if cx.snap.selected().any(|o| o.link_group.is_some()) {
        Setting::KeepAspectLinked
    } else {
        Setting::KeepAspect
    };
    if let Some(why) = forced {
        let mut on = true;
        switch(
            ui,
            mode,
            &mut on,
            Icon::KeepAspect,
            "Keep aspect",
            card,
            Some(why),
        );
    } else {
        switch(
            ui,
            mode,
            &mut f.keep_aspect,
            Icon::KeepAspect,
            "Keep aspect",
            card,
            (!any).then_some("Select something first."),
        );
    }
    ui.end_row();
}

/// "Scales all N linked copies" after the columns, when it applies.
fn linked_note(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    // Linked copies share the scale.
    if let Some(n) = cx.snap.linked_reach() {
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
