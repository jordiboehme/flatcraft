//! The Prepare ribbon: the Layout and View tabs with their groups
//! (see [`groups`] and the table in [`tools`]), and the "Search commands"
//! box on the right.

use egui::accesskit::Role;
use egui::{Ui, WidgetInfo};
use flatcraft_core::numexpr::{Field, Kind};
use flatcraft_engine::registry;
use serde_json::json;

use super::Action;
use super::calc;
use super::canvas;
use super::collapse::{self, Group, Mode};
use super::groups::{self, Cx, Extras};
use super::icons::Icon;
use super::prepare::{FieldEdit, FieldPlace, Fields, PrepareState, RibbonTab, SizeField, Snapshot};
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

/// The largest position or size a field takes, mm: 10 m, more than any
/// plotter cuts.
const MAX_MM: f64 = 10_000.0;

/// The largest scale the % field takes.
const MAX_PERCENT: f64 = 100_000.0;

/// The inner margin of the ribbon's number fields.
const FIELD_MARGIN: egui::Vec2 = egui::vec2(4.0, 2.0);

/// The widest value a millimetre field of the ribbon shows as wide as it
/// is: 4 digits and 2 decimals, more than any mat.
const WIDEST_MM: &str = "-0000.00";

/// The widest value the W and H fields show as wide as they are: a size
/// is never negative.
const WIDEST_SIZE: &str = "0000.00";

/// The widest value the Angle and % fields show as wide as they are: 3
/// digits and 2 decimals (a larger % grows its field).
const WIDEST_DEG: &str = "000.00";

/// A short number field of the Position and size group.
struct NumberField<'a> {
    field: SizeField,
    label: &'a str,
    /// An icon shown instead of the label text.
    icon: Option<Icon>,
    /// Shown greyed while the field is empty ("Mixed").
    hint: &'a str,
    /// The widest value it shows as wide as it is.
    widest: &'a str,
    /// Its accessible name and card title.
    name: &'a str,
    setting: Setting,
    /// What it shows while it is not being edited.
    current: String,
    /// Its value, unrounded, for a relative edit; `None` while Mixed.
    value: Option<f64>,
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
            SizeField::Angle => "rotation-angle",
        }
    }

    /// Its widget id in `place`, the same wherever the group is drawn
    /// there.
    pub fn id(self, place: FieldPlace) -> egui::Id {
        egui::Id::unique(("flatcraft-size-field", self.id_salt(), place))
    }

    /// What the field takes: its unit, its range.
    fn spec(self) -> Field {
        match self {
            SizeField::X | SizeField::Y => Field::length().range(-MAX_MM, MAX_MM),
            SizeField::Width | SizeField::Height => Field::length().positive().range(0.0, MAX_MM),
            SizeField::Percent => Field::new(Kind::Percent).positive().range(0.0, MAX_PERCENT),
            SizeField::Angle => Field::turn(),
        }
    }

    fn buf(self, f: &mut Fields) -> &mut String {
        match self {
            SizeField::X => &mut f.x,
            SizeField::Y => &mut f.y,
            SizeField::Width => &mut f.width,
            SizeField::Height => &mut f.height,
            SizeField::Percent => &mut f.percent,
            SizeField::Angle => &mut f.angle,
        }
    }
}

/// The field `field` as it shows now: its label, name, card and value.
/// The ribbon and the object inspector both draw it from here. `b` is the
/// selection's live box ([`canvas::live_box`]), worked out once a frame
/// by the caller.
fn number_field_of(
    field: SizeField,
    snap: &Snapshot,
    st: &PrepareState,
    b: Option<kurbo::Rect>,
) -> NumberField<'static> {
    let any = !snap.selection.is_empty();
    let nf = |label, widest, name, setting, value: Option<f64>| NumberField {
        field,
        label,
        icon: None,
        hint: "",
        widest,
        name,
        setting,
        current: value.map(mm).unwrap_or_default(),
        value,
        enabled: any,
    };
    match field {
        SizeField::X => nf(
            "X",
            WIDEST_MM,
            "X (mm)",
            Setting::PositionX,
            b.map(|b| b.x0),
        ),
        SizeField::Y => nf(
            "Y",
            WIDEST_MM,
            "Y (mm)",
            Setting::PositionY,
            b.map(|b| b.y0),
        ),
        // A size is never negative.
        SizeField::Width => nf(
            "W",
            WIDEST_SIZE,
            "Width (mm)",
            Setting::Width,
            b.map(|b| b.width()),
        ),
        SizeField::Height => nf(
            "H",
            WIDEST_SIZE,
            "Height (mm)",
            Setting::Height,
            b.map(|b| b.height()),
        ),
        // As wide as the Angle below it: 3 digits and 2 decimals.
        SizeField::Percent => nf(
            "%",
            WIDEST_DEG,
            "Scale (%)",
            Setting::ScalePercent,
            any.then(|| canvas::live_percent(st).unwrap_or(100.0)),
        ),
        SizeField::Angle => {
            let angle = canvas::live_angle(snap, st);
            // Mixed only for several objects turned different ways, never
            // for an emptied field.
            let mixed = snap.selection.len() > 1 && angle.is_none();
            NumberField {
                icon: Some(Icon::Rotate),
                // Turned different ways: no one angle to show.
                hint: if mixed { "Mixed" } else { "" },
                ..nf(
                    "Angle",
                    WIDEST_DEG,
                    "Angle (degrees)",
                    Setting::Angle,
                    angle,
                )
            }
        }
    }
}

/// The card of a field: what it is, and how to type a calculation.
fn field_card(nf: &NumberField) -> String {
    card_of(nf.field, nf.setting)
}

fn card_of(field: SizeField, setting: Setting) -> String {
    calc::with_hint(setting.card(), field.spec().kind)
}

/// Why the fields are dimmed.
const SELECT_FIRST: &str = "Select something first.";

/// Draws a number field of the ribbon: its label, then the field. It shows
/// `current` while it is not being edited and never overwrites what the
/// person types. The typed value counts when the field loses the focus
/// (Tab, Shift-Tab, Enter, a click elsewhere), see [`finish_edit`]; Escape
/// puts the shown value back. It is always as wide as its text, and at
/// least as wide as [`WIDEST_MM`], so no digit is cut.
fn number_field(ui: &mut Ui, nf: NumberField, cx: &mut Cx) -> egui::Rect {
    let why = (!nf.enabled).then_some(SELECT_FIRST);
    let l = match nf.icon {
        Some(icon) => {
            let tex = super::icons::texture(ui.ctx(), icon, LABEL_ICON);
            let mut img = egui::Image::from_texture(egui::load::SizedTexture::new(
                tex.id(),
                egui::Vec2::splat(LABEL_ICON),
            ))
            .sense(egui::Sense::hover());
            if !nf.enabled {
                img = img.tint(ui.visuals().weak_text_color());
            }
            let r = ui.add(img);
            r.widget_info(|| WidgetInfo::labeled(Role::Label, nf.enabled, nf.label));
            r
        }
        None => ui.label(nf.label),
    };
    let card = field_card(&nf);
    groups::card(&l, nf.name, &card, why, false);
    let (name, widest) = (nf.name, nf.widest);
    let resp = field_edit(
        ui,
        nf,
        FieldPlace::Ribbon,
        cx.st,
        cx.snap,
        cx.actions,
        |ui, buf| field_width(ui, buf).max(field_width(ui, widest)),
    );
    set_name(ui, &resp, name);
    groups::card(&resp, name, &card, why, false);
    resp.rect
}

/// A Position and size field of the object inspector, as wide as the row
/// gives it. The row names it and gives it its card ([`inspector_card`]).
/// The same field, value and rules as in the ribbon.
pub fn inspector_field(
    ui: &mut Ui,
    field: SizeField,
    b: Option<kurbo::Rect>,
    st: &mut PrepareState,
    snap: &Snapshot,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let nf = number_field_of(field, snap, st, b);
    field_edit(ui, nf, FieldPlace::Inspector, st, snap, actions, |ui, _| {
        ui.available_width()
    })
}

/// The card of a field in the object inspector: the ribbon's card.
pub fn inspector_card(field: SizeField) -> String {
    let setting = match field {
        SizeField::X => Setting::PositionX,
        SizeField::Y => Setting::PositionY,
        SizeField::Width => Setting::Width,
        SizeField::Height => Setting::Height,
        SizeField::Percent => Setting::ScalePercent,
        SizeField::Angle => Setting::Angle,
    };
    card_of(field, setting)
}

/// The text field of a number field in `place`, `width(ui, text)` wide:
/// it follows the value while it is not edited, and applies what was
/// typed when it loses the focus. A field has a twin in the other place;
/// while one is edited, the other shows what is typed.
fn field_edit(
    ui: &mut Ui,
    nf: NumberField,
    place: FieldPlace,
    st: &mut PrepareState,
    snap: &Snapshot,
    actions: &mut Vec<Action>,
    width: impl FnOnce(&Ui, &str) -> f32,
) -> egui::Response {
    let id = nf.field.id(place);
    let focused = ui.memory(|m| m.has_focus(id));
    let fields = &mut st.fields;
    // Edited here, or in its twin (the twin is drawn first in a frame, so
    // only the place being edited touches the text).
    let field = nf.field;
    let edited = |f: &Fields| f.editing.as_ref().map(|e| (e.field == field, e.place));
    let twin = edited(fields).is_some_and(|(same, p)| same && p != place);
    let editing = edited(fields).is_some_and(|(same, p)| same && p == place);
    if !focused && !twin && !editing {
        *nf.field.buf(fields) = nf.current.clone();
    } else if editing {
        // Nothing typed yet (the focus came by Tab, say): it follows the
        // value, which a field applied in the same frame may have changed.
        // It also follows the selection, so a Tab out of it never moves
        // what was selected before.
        let shown = fields.editing.as_ref().map(|e| e.shown.clone());
        let now: Vec<u64> = snap.selection.iter().copied().collect();
        if shown.as_deref() != Some(nf.field.buf(fields).as_str())
            && fields.editing.as_ref().is_some_and(|e| e.ids != now)
        {
            // Typed, and then the selection changed (an agent, a menu
            // command): a text it can read stays with the objects, the
            // document and the value it was typed for. A text it cannot
            // read, or an empty selection, ends the edit with the old
            // value.
            let typed = nf.field.buf(fields).trim().to_string();
            let start = fields.editing.as_ref().and_then(|e| e.value);
            let read = nf.field.spec().eval(&typed, start);
            match (read, now.is_empty()) {
                (Ok(_), false) => {}
                (read, _) => {
                    if let (Err(e), false) = (read, now.is_empty()) {
                        actions.push(Action::Error(calc::dropped(&e.message)));
                    }
                    *nf.field.buf(fields) = nf.current.clone();
                    fields.editing = None;
                    fields.error = None;
                    ui.memory_mut(|m| m.surrender_focus(id));
                }
            }
        } else if shown.as_deref() == Some(nf.field.buf(fields).as_str()) {
            *nf.field.buf(fields) = nf.current.clone();
            if let Some(e) = fields.editing.as_mut() {
                e.shown = nf.current.clone();
                e.value = nf.value;
                e.doc_id = snap.doc_id;
                e.ids = snap.selection.iter().copied().collect();
                e.free = canvas::free_aspect(snap, &e.ids);
            }
        }
    }
    let buf = nf.field.buf(fields);
    let w = width(ui, buf);
    let resp = ui.add_enabled(
        nf.enabled,
        egui::TextEdit::singleline(buf)
            .id(id)
            .hint_text(nf.hint)
            .margin(egui::Margin::symmetric(
                FIELD_MARGIN.x as i8,
                FIELD_MARGIN.y as i8,
            ))
            .desired_width(w)
            .min_size(egui::vec2(w, 0.0)),
    );
    calc::select_whole_value(ui, &resp, nf.field.buf(&mut st.fields));
    let here = st.fields.editing.as_ref().is_none_or(|e| e.place == place);
    if let Some((_, msg)) = st
        .fields
        .error
        .as_ref()
        .filter(|(f, _)| *f == nf.field && here)
    {
        calc::show_message(ui, id, resp.rect, msg);
    }
    let editing = edited(&st.fields).is_some_and(|(same, p)| same && p == place);
    if resp.has_focus() {
        if !editing {
            // Another field still editing lost the focus in this pass.
            finish_edit(ui.ctx(), st, actions, false);
            if st.fields.editing.is_none() {
                if twin {
                    // Its twin showed the text typed there: this one
                    // starts from the value.
                    *nf.field.buf(&mut st.fields) = nf.current.clone();
                }
                let ids: Vec<u64> = snap.selection.iter().copied().collect();
                st.fields.editing = Some(FieldEdit {
                    field: nf.field,
                    place,
                    shown: nf.current,
                    doc_id: snap.doc_id,
                    free: canvas::free_aspect(snap, &ids),
                    ids,
                    value: nf.value,
                });
            }
        }
    } else if editing {
        // Enter with a text it cannot read stays in the field.
        let enter = calc::keys(ui.ctx()).0;
        finish_edit(ui.ctx(), st, actions, enter && nf.enabled);
    }
    resp
}

/// Once a frame, before the ribbon: ends the editing of a field that lost
/// the focus outside the group (a click on the canvas, another ribbon
/// tab), whose place is not on screen (`on_screen` of its place false:
/// another tab or workspace, a folded section), or whose document is no
/// longer the active one, so its value applies at once, in order and in
/// its own document.
pub fn finish_pending(
    ctx: &egui::Context,
    st: &mut PrepareState,
    snap: &Snapshot,
    on_screen: impl Fn(FieldPlace) -> bool,
    actions: &mut Vec<Action>,
) {
    let Some(e) = &st.fields.editing else { return };
    let here = on_screen(e.place) && e.doc_id == snap.doc_id;
    if !here || !ctx.memory(|m| m.has_focus(e.field.id(e.place))) {
        finish_edit(ctx, st, actions, false);
    }
}

/// Ends the editing of a field now, whether it still has the focus or
/// not: a save, close or quit counts what was typed first.
pub fn finish_now(ctx: &egui::Context, st: &mut PrepareState, actions: &mut Vec<Action>) {
    finish_edit(ctx, st, actions, false);
}

/// Ends the editing of the field that lost the focus: Escape puts its
/// shown text back, a changed value (a number or a calculation, see
/// [`flatcraft_core::numexpr`]) is applied as one command. A text it
/// cannot read applies nothing: with `keep` (Enter) the field keeps the
/// text and the focus and says why under it; otherwise (Tab, a click
/// elsewhere, another tab or document, a save) the old value comes back
/// and a toast says why. The focus is never taken back.
fn finish_edit(ctx: &egui::Context, st: &mut PrepareState, actions: &mut Vec<Action>, keep: bool) {
    let Some(edit) = st.fields.editing.take() else {
        return;
    };
    let buf = edit.field.buf(&mut st.fields);
    let typed = buf.trim().to_string();
    let escape = calc::keys(ctx).1;
    if escape || typed == edit.shown.trim() || edit.ids.is_empty() {
        *buf = edit.shown.clone();
        st.fields.error = None;
        return;
    }
    match edit.field.spec().eval(&typed, edit.value) {
        Ok(v) => {
            st.fields.error = None;
            if let Some(a) = size_action(&st.fields, &edit, v) {
                actions.push(a);
            }
        }
        Err(e) if keep => {
            ctx.memory_mut(|m| m.request_focus(edit.field.id(edit.place)));
            st.fields.error = Some((edit.field, calc::kept(&e.message)));
            st.fields.editing = Some(edit);
        }
        Err(e) => {
            *buf = edit.shown.clone();
            st.fields.error = None;
            actions.push(Action::Error(calc::dropped(&e.message)));
        }
    }
}

/// The command a typed value of a Position and size field runs, in the
/// document it was typed for.
fn size_action(f: &Fields, edit: &FieldEdit, v: f64) -> Option<Action> {
    let ids = &edit.ids;
    let single = ids.len() == 1;
    let params = match edit.field {
        SizeField::X => json!({ "ids": ids, "x_mm": v }),
        // Every object gets the angle, each about its own centre.
        SizeField::Angle => json!({ "ids": ids, "angle_deg": v }),
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
        SizeField::Angle => "object.rotate",
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
        "document.export" => Action(super::Action::ExportDialog { doc_id }),
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
        // The Position and size, Transform, Arrange and Edit groups have
        // these; with a selection, Circular pattern opens its panel (see
        // `pick`).
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
pub const TAB_ROW: f32 = 22.0;
/// Room between the ribbon panel's border and its tools.
pub const PANEL_PAD: egui::Vec2 = egui::vec2(4.0, 3.0);

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
        // The first tab's outline starts on the panel's left edge, so the
        // tab and the panel read as one folder.
        egui::pos2(full.left(), tab_row.top()),
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
                ui.spacing_mut().item_spacing.x = groups::BUTTON_GAP;
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
            fold: Some(id.def().fold),
        }
    };
    tools::groups_of(tab)
        .map(|def| match def.id {
            GroupId::Add => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Add, cx)),
            GroupId::Edit => g(def.id, |ui, m, cx| {
                let extras = Extras {
                    inline: Some(select_toggle),
                    in_list: Some(|ui, cx| select_toggle(ui, Mode::List, cx)),
                };
                groups::show(ui, m, GroupId::Edit, extras, cx)
            }),
            GroupId::Files => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Files, cx)),
            GroupId::Size => g(def.id, size_group),
            GroupId::Transform => g(def.id, |ui, m, cx| {
                let extras = Extras {
                    inline: Some(each_switch),
                    in_list: Some(|ui, cx| each_switch(ui, Mode::List, cx)),
                };
                groups::show(ui, m, GroupId::Transform, extras, cx)
            }),
            GroupId::Arrange => g(def.id, |ui, m, cx| plain(ui, m, GroupId::Arrange, cx)),
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

/// A switch that is not an action ("Keep aspect", "Each object"): an
/// icon toggle whose icon shows its state (`icons.0` on, `icons.1` off),
/// with its text only in a list. It is a checkbox named `text` with its
/// state, and its card says what it does (and why it is dimmed).
fn switch(
    ui: &mut Ui,
    mode: Mode,
    on: &mut bool,
    icons: (Icon, Icon),
    text: &str,
    setting: Setting,
    why: Option<&str>,
) -> egui::Response {
    let enabled = why.is_none();
    let label = mode == Mode::List;
    let icon = if *on { icons.0 } else { icons.1 };
    let mut resp = groups::compact_switch(ui, icon, text, label, enabled, *on);
    if resp.clicked() {
        // Like a checkbox: the caller sees the change.
        *on = !*on;
        resp.mark_changed();
    }
    groups::card(&resp, text, setting.card(), why, false);
    resp
}

/// The Position and size group: X, Y, W, H, %, Angle and Keep aspect, with its
/// title under them. It has no actions and never collapses.
fn size_group(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    let def = GroupId::Size.def();
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = groups::GROUP_GAP;
        // Three columns of two rows: X above Y, W above H, and % above
        // the Angle; each is a grid, so the labels line up. The Keep
        // aspect lock sits left of W and H, on a bracket that joins them.
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
                    // The Keep aspect lock and its bracket sit left of W
                    // and H. Their room now, the toggle after % and Angle:
                    // Tab goes X, Y, W, H, %, Angle, then the lock.
                    let gap = ui.spacing().item_spacing.x;
                    ui.spacing_mut().item_spacing.x = SIZE_GRID_GAP.x;
                    // The grid ends with its own gap.
                    ui.add_space(-SIZE_GRID_GAP.x);
                    let (column, _) = ui.allocate_exact_size(
                        egui::vec2(LOCK_COLUMN, groups::BIG_HEIGHT),
                        egui::Sense::hover(),
                    );
                    ui.spacing_mut().item_spacing.x = gap;
                    let wh = egui::Grid::new("flatcraft-size-wh")
                        .num_columns(2)
                        .min_col_width(0.0)
                        .spacing(SIZE_GRID_GAP)
                        .show(ui, |ui| size_wh(ui, cx))
                        .inner;
                    egui::Grid::new("flatcraft-size-scale")
                        .num_columns(2)
                        .min_col_width(0.0)
                        .spacing(SIZE_GRID_GAP)
                        .show(ui, |ui| size_scale(ui, cx));
                    keep_aspect(ui, FieldPlace::Ribbon, cx.st, cx.snap, column, wh, false);
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
    let b = canvas::live_box(cx.snap, cx.st);
    for field in [SizeField::X, SizeField::Y] {
        let nf = number_field_of(field, cx.snap, cx.st, b);
        number_field(ui, nf, cx);
        ui.end_row();
    }
}

/// The second column: W above H. Returns the rects of the two fields.
fn size_wh(ui: &mut Ui, cx: &mut Cx) -> [egui::Rect; 2] {
    let mut rects = [egui::Rect::NOTHING; 2];
    let b = canvas::live_box(cx.snap, cx.st);
    for (i, field) in [SizeField::Width, SizeField::Height]
        .into_iter()
        .enumerate()
    {
        let nf = number_field_of(field, cx.snap, cx.st, b);
        rects[i] = number_field(ui, nf, cx);
        ui.end_row();
    }
    rects
}

/// The Angle under the %, with the rotate icon as its label.
fn size_angle(ui: &mut Ui, cx: &mut Cx) {
    // The Angle needs no box.
    let nf = number_field_of(SizeField::Angle, cx.snap, cx.st, None);
    number_field(ui, nf, cx);
    ui.end_row();
}

/// The size of an icon that labels a field.
const LABEL_ICON: f32 = 12.0;

/// Room between the cells of the group's columns.
const SIZE_GRID_GAP: egui::Vec2 = egui::vec2(4.0, 4.0);

/// The third column: % above the Angle.
fn size_scale(ui: &mut Ui, cx: &mut Cx) {
    // The % needs no box.
    let nf = number_field_of(SizeField::Percent, cx.snap, cx.st, None);
    number_field(ui, nf, cx);
    ui.end_row();
    size_angle(ui, cx);
}

/// The width of the Keep aspect column: the lock and the bracket's arms.
const LOCK_COLUMN: f32 = 14.0;

/// Where the lock's centre sits in its column, from the column's left:
/// room for an open shackle on the left.
const LOCK_X: f32 = 7.5;

/// How far the bracket's line stays away from the lock's centre, up and
/// down.
const LOCK_GAP: f32 = 8.5;

/// The lock, drawn with the painter: a solid body and a shackle line of
/// the bracket's width. Open, the shackle is lifted and swung out to the
/// left of the body, its right leg free.
#[derive(Debug, Clone, PartialEq)]
pub struct LockShape {
    pub body: egui::Rect,
    /// The shackle as one line: a leg, the arc, the other leg.
    pub shackle: Vec<egui::Pos2>,
}

/// The lock around `c`, closed or open, snapped to the pixels of `ppp`
/// (a 1 pt line sits on pixel centres at 1x and on pixel edges at 2x).
pub fn lock_shape(c: egui::Pos2, open: bool, ppp: f32) -> LockShape {
    let line = |v: f32| snap_line(v, ppp);
    let px = |v: f32| (v * ppp).round() / ppp;
    // Body: 7 x 6 pt, below the centre.
    let body = egui::Rect::from_min_max(
        egui::pos2(px(c.x - 3.5), px(c.y - 0.5)),
        egui::pos2(px(c.x + 3.5), px(c.y + 5.5)),
    );
    let r = 2.0;
    // Closed: legs at the body's middle +-r, the arc on top. Open: lifted
    // 1.5 pt and swung left about the left leg, so the arc's other end
    // (the right leg) hangs free beside the body.
    let (left_leg, lift) = if open {
        (c.x - r - 2.0 * r, 1.5)
    } else {
        (c.x - r, 0.0)
    };
    let right_leg = left_leg + 2.0 * r;
    let arc_y = c.y - 3.0 - lift;
    let (lx, rx) = (line(left_leg), line(right_leg));
    let mut shackle = Vec::new();
    // The leg that goes into the body: the right one when closed (both
    // do), the right one of the swung shackle when open.
    // Legs end on the body's edge (only their x needs the pixel grid).
    let (left_end, right_end) = if open {
        (arc_y + 2.0, body.top())
    } else {
        (body.top(), body.top())
    };
    shackle.push(egui::pos2(lx, left_end));
    shackle.push(egui::pos2(lx, arc_y));
    let steps = 10;
    for k in 1..steps {
        let a = std::f32::consts::PI * (1.0 + k as f32 / steps as f32);
        let mid = (lx + rx) / 2.0;
        let rr = (rx - lx) / 2.0;
        shackle.push(egui::pos2(mid + rr * a.cos(), arc_y + rr * a.sin()));
    }
    shackle.push(egui::pos2(rx, arc_y));
    shackle.push(egui::pos2(rx, right_end));
    LockShape { body, shackle }
}

/// A 1 pt line's coordinate on the pixel grid of `ppp`: on a pixel's
/// centre when the line is an odd number of pixels wide, else on an edge.
fn snap_line(v: f32, ppp: f32) -> f32 {
    let width_px = ppp.round().max(1.0) as i32;
    if width_px % 2 == 1 {
        ((v * ppp - 0.5).round() + 0.5) / ppp
    } else {
        (v * ppp).round() / ppp
    }
}

/// Keep aspect: a small lock (closed: on, open: off) in `column`, left of
/// the W and H fields (`wh`), on a thin bracket that joins W and H. Lock
/// and bracket are one toggle: one hit area (the bracket's height in the
/// column, never over the fields), one name, one card, and they light up
/// together on hover. Monochrome, in the bracket's colour.
///
/// The object inspector draws the same toggle right of its W and H rows
/// (`fields_left`), with its own id in its `place`.
pub fn keep_aspect(
    ui: &mut Ui,
    place: FieldPlace,
    st: &mut PrepareState,
    snap: &Snapshot,
    column: egui::Rect,
    wh: [egui::Rect; 2],
    fields_left: bool,
) -> egui::Response {
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let any = !ids.is_empty();
    let forced = canvas::aspect_lock(snap, &ids).filter(|_| any);
    // With linked copies selected the card says how they follow.
    let card = if snap.selected().any(|o| o.link_group.is_some()) {
        Setting::KeepAspectLinked
    } else {
        Setting::KeepAspect
    };
    let why = forced.or((!any).then_some(SELECT_FIRST));
    let enabled = why.is_none();
    let on = forced.is_some() || st.fields.keep_aspect;

    let (top, bottom) = (wh[0].center().y, wh[1].center().y);
    let hit = egui::Rect::from_x_y_ranges(column.x_range(), (top - 6.0)..=(bottom + 6.0));
    let id = egui::Id::unique(("flatcraft-keep-aspect", place));
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let mut resp = ui.interact(hit, id, sense);
    resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, on, "Keep aspect"));
    if enabled && resp.clicked() {
        st.fields.keep_aspect = !st.fields.keep_aspect;
        resp.mark_changed();
    }
    let on = forced.is_some() || st.fields.keep_aspect;

    let v = ui.visuals();
    let colour = if !enabled {
        v.weak_text_color().gamma_multiply(0.5)
    } else if resp.hovered() || resp.has_focus() {
        v.strong_text_color()
    } else {
        v.text_color()
    };
    let painter = ui.painter();
    if enabled && (resp.hovered() || resp.has_focus()) {
        painter.rect_filled(hit, 3.0, v.widgets.hovered.weak_bg_fill);
    }
    // The bracket: arms from W and H to a line through the lock, broken
    // around it with room to spare.
    let ppp = ui.ctx().pixels_per_point();
    // The lock's line, and where the arms meet the fields.
    let lock_x = if fields_left {
        hit.right() - LOCK_X
    } else {
        hit.left() + LOCK_X
    };
    let x = snap_line(lock_x, ppp);
    let arms = if fields_left {
        hit.left() + 1.0
    } else {
        hit.right() - 1.0
    };
    let (top, bottom) = (snap_line(top, ppp), snap_line(bottom, ppp));
    let middle = (top + bottom) / 2.0;
    let stroke = egui::Stroke::new(1.0, colour);
    painter.line(
        vec![
            egui::pos2(arms, top),
            egui::pos2(x, top),
            egui::pos2(x, snap_line(middle - LOCK_GAP, ppp)),
        ],
        stroke,
    );
    painter.line(
        vec![
            egui::pos2(arms, bottom),
            egui::pos2(x, bottom),
            egui::pos2(x, snap_line(middle + LOCK_GAP, ppp)),
        ],
        stroke,
    );
    let lock = lock_shape(egui::pos2(lock_x, middle), !on, ppp);
    painter.rect_filled(lock.body, 1.0, colour);
    painter.line(lock.shackle, stroke);
    groups::card(&resp, "Keep aspect", card.card(), why, false);
    resp
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
    // Several objects turn and flip as one piece, or each on its own. A
    // whole group counts as one object.
    let several = cx.snap.units().len() > 1;
    let why = (!several)
        .then_some("Select 2 or more objects first. One object, or one group, always turns about its own centre.");
    let f = &mut cx.st.fields;
    if switch(
        ui,
        mode,
        &mut f.each,
        (Icon::EachObject, Icon::AsGroup),
        "Each object",
        Setting::EachObject,
        why,
    )
    .changed()
    {
        cx.actions.push(Action::RememberEach(f.each));
    }
}

/// The Select/Lasso toggle: what a drag on the canvas draws. Two
/// switches, one on; on the bar they stand one above the other.
fn select_toggle(ui: &mut Ui, mode: Mode, cx: &mut Cx) {
    use super::prepare::SelectTool;
    let rows = [
        (
            SelectTool::Select,
            Icon::Pointer,
            "Select",
            Setting::SelectTool,
        ),
        (SelectTool::Lasso, Icon::Lasso, "Lasso", Setting::LassoTool),
    ];
    let label = matches!(mode, Mode::Full | Mode::List);
    let mut draw = |ui: &mut Ui| {
        for (tool, icon, text, setting) in rows {
            let on = cx.st.select_tool == tool;
            let resp = groups::compact_switch(ui, icon, text, label, true, on);
            groups::card(&resp, text, setting.card(), None, false);
            if resp.clicked() {
                cx.st.select_tool = tool;
            }
        }
    };
    if mode == Mode::List {
        draw(ui);
    } else {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            draw(ui);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_lock_swings_its_shackle_out_to_the_left() {
        let c = egui::pos2(100.0, 50.0);
        for ppp in [1.0, 2.0] {
            let closed = lock_shape(c, false, ppp);
            let open = lock_shape(c, true, ppp);
            assert_eq!(closed.body, open.body, "the body stays");
            let min_x = |l: &LockShape| l.shackle.iter().map(|p| p.x).fold(f32::MAX, f32::min);
            let max_x = |l: &LockShape| l.shackle.iter().map(|p| p.x).fold(f32::MIN, f32::max);
            let top = |l: &LockShape| l.shackle.iter().map(|p| p.y).fold(f32::MAX, f32::min);
            // Closed: the shackle stands on the body, both legs in it.
            assert!(min_x(&closed) > closed.body.left() && max_x(&closed) < closed.body.right());
            assert_eq!(closed.shackle.first().unwrap().y, closed.body.top());
            assert_eq!(closed.shackle.last().unwrap().y, closed.body.top());
            // Open: lifted, out past the body's left side, one leg free.
            assert!(min_x(&open) < open.body.left() - 1.0, "{ppp}");
            assert!(top(&open) < top(&closed), "{ppp}");
            assert!(
                open.shackle.first().unwrap().y < open.body.top() - 1.0,
                "a free leg"
            );
            assert_eq!(open.shackle.last().unwrap().y, open.body.top());
            // Narrow and inside the column with the bracket.
            assert!(closed.body.width() <= 7.0);
            assert!(min_x(&open) >= c.x - LOCK_X, "{ppp}");
        }
    }

    #[test]
    fn a_one_point_line_sits_on_the_pixel_grid() {
        // 1x: on pixel centres; 2x: on pixel edges.
        assert_eq!(snap_line(10.2, 1.0), 10.5);
        assert_eq!(snap_line(10.2, 2.0), 10.0);
        assert_eq!(snap_line(10.3, 2.0), 10.5);
    }

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
