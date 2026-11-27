//! The object inspector: what the selected objects are cut with, where
//! they sit, how they are arranged, where they come from and what to
//! know about them. The right panel shows it while objects are selected
//! (the job line above it opens the Job panel over them).
//!
//! - A heading that names what is edited ("2 objects", "Group 'Badge'",
//!   "Star · in group Badge").
//! - Cut: cut mode, keep (with the picker on the mat) and strokes.
//! - Position and size: the ribbon's X, Y, W, H (with the lock), % and
//!   Angle fields, the same fields with the same rules.
//! - Arrange: align, distribute, order and groups, as small buttons.
//! - Source: the linked file, reload, reconnect and trace.
//! - Hints: the report's issues for these objects, with their fixes and
//!   Ignore, as hint cards.
//!
//! Every change runs the registry command the ribbon, the context menu or
//! the import report runs, so it is one step to undo.

use egui::{Response, Ui, vec2};
use flatcraft_core::document::{CutMode, KeepMode, StrokeMode};
use flatcraft_engine::job::JobInput;
use flatcraft_engine::report::Issue;
use serde_json::json;

use super::super::context_menu::{self, Item};
use super::super::icons::Icon;
use super::super::prepare::{FieldPlace, PrepareState, SizeField, Snapshot};
use super::super::settings::{
    CUT_MODES, keep_hint, keep_name, mode_hint, mode_name, set_name, strokes_name,
};
use super::super::theme::Tokens;
use super::super::tools::{self, Act, AlignMode, AlignTo, Setting};
use super::super::{Action, canvas, groups, keep_pick, metrics, report, ribbon};
use super::blocks::{self, HintButton};
use super::job;
use flatcraft_engine::commands::group::ReorderTo;

/// The accessible name of the Arrange section's header: the ribbon has
/// an Arrange group too.
pub const ARRANGE_NAME: &str = "Arrange objects";

/// The ids of the object inspector's sections.
pub const CUT: &str = "object.cut";
pub const POSITION: &str = "object.position";
pub const ARRANGE: &str = "object.arrange";
pub const SOURCE: &str = "object.source";
pub const HINTS: &str = "object.hints";

/// The accessible names of the sections' headers (their titles, but
/// Arrange), in the panel's order, with their ids.
pub const SECTIONS: [(&str, &str); 5] = [
    (CUT, "Cut"),
    (POSITION, "Position and size"),
    (ARRANGE, ARRANGE_NAME),
    (SOURCE, "Source"),
    (HINTS, "Hints"),
];

/// The labels of the Position and size rows, by field.
pub const FIELDS: [(SizeField, &str); 6] = [
    (SizeField::X, "X"),
    (SizeField::Y, "Y"),
    (SizeField::Width, "Width"),
    (SizeField::Height, "Height"),
    (SizeField::Percent, "Scale"),
    (SizeField::Angle, "Angle"),
];

/// Every setting and action of the object inspector, as (section id,
/// accessible name), for the reachability test: each one is in its
/// section, at most one fold away. The Source actions depend on the
/// object (a file, a picture): they are [`SOURCE_ITEMS`], always shown.
pub const SETTINGS: [(&str, &str); 26] = [
    (CUT, "Cut mode"),
    (CUT, "Keep"),
    (CUT, keep_pick::BUTTON),
    (CUT, "Strokes"),
    (POSITION, "X"),
    (POSITION, "Y"),
    (POSITION, "Width"),
    (POSITION, "Height"),
    (POSITION, "Keep aspect"),
    (POSITION, "Scale"),
    (POSITION, "Angle"),
    (ARRANGE, "Align left"),
    (ARRANGE, "Align centre"),
    (ARRANGE, "Align right"),
    (ARRANGE, "Align top"),
    (ARRANGE, "Align middle"),
    (ARRANGE, "Align bottom"),
    (ARRANGE, ALIGN_TO_AREA),
    (ARRANGE, "Distribute horizontally"),
    (ARRANGE, "Distribute vertically"),
    (ARRANGE, "Bring to front"),
    (ARRANGE, "Bring forward"),
    (ARRANGE, "Send backward"),
    (ARRANGE, "Send to back"),
    (ARRANGE, "Group"),
    (SOURCE, "File"),
];

/// The name of the switch that aligns to the cutting area.
pub const ALIGN_TO_AREA: &str = "Align to the cutting area";

/// The cards of the sections.
pub const CUT_CARD: &str = "What is cut of the selected objects, and which side of the cut you keep. Each change is one step to undo.";
pub const POSITION_CARD: &str = "Where the selection sits on the mat and how big it is: the same fields as Position and size in the ribbon. Type a number or a calculation, then press Tab or Enter or click elsewhere. Esc puts the old value back.";
pub const ARRANGE_CARD: &str = "Line the objects up, space them evenly, put them in front of or behind others, or group them so they act as one.";
pub const SOURCE_CARD: &str = "The file the objects were imported from. FlatCraft keeps its own copy in the project, so a change in the file only shows after you read it again.";
pub const HINTS_CARD: &str = "What FlatCraft noticed about these objects before you cut: things that will not cut well, and objects very close to or over each other. A button fixes it, Ignore keeps it the way it is.";

/// Why Strokes is dimmed.
pub const STROKES_ALL_LINES: &str = "All lines cuts every line along its middle, so there is no stroke to outline. Pick another cut mode first.";

/// The card of the Keep pick button.
const PICK_CARD: &str = keep_pick::HINT;

/// The value all selected objects share, or `None` when they differ.
fn common<T: PartialEq + Copy>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

/// What the heading says: the title and the line under it. The line
/// holds only what the panel shows nowhere else: the group the selection
/// is in and its linked copies. Size, file and kind are in the sections
/// below, so the line is often empty.
///
/// - One group: "Group 'Badge'", "2 objects · in group Sign" under it.
/// - One object: its name, "in group Badge · 1 of 3 linked copies" under
///   it.
/// - More: "2 objects" ("1 group and 2 objects" when groups are among
///   them), "in group Badge · applies to 3 linked copies" under it.
pub fn heading(snap: &Snapshot) -> (String, String) {
    let units = snap.units();
    let parent = |id: u64| {
        snap.tree
            .parent(flatcraft_core::document::ObjectId(id))
            .and_then(|g| snap.name_of(g.0))
    };
    let in_group = |ids: &[u64]| {
        let first = ids.first().map(|id| parent(*id))?;
        let all = ids.iter().all(|id| parent(*id) == first);
        all.then_some(first).flatten()
    };
    let mut sub = Vec::new();
    if let Some(p) = in_group(&units) {
        sub.push(format!("in group {p}"));
    }
    let title = match units.as_slice() {
        [] => return (String::new(), String::new()),
        [g] if snap.is_group(*g) => {
            let name = snap.name_of(*g).unwrap_or_default();
            let n = snap
                .tree
                .children(Some(flatcraft_core::document::ObjectId(*g)))
                .len();
            sub.insert(
                0,
                if n == 1 {
                    "1 object".into()
                } else {
                    format!("{n} objects")
                },
            );
            format!("Group '{name}'")
        }
        [one] => {
            if let Some(g) = snap.object(*one).and_then(|o| o.link_group) {
                sub.push(format!("1 of {} linked copies", snap.group_size(g)));
            }
            snap.name_of(*one).unwrap_or_default()
        }
        many => {
            if let Some(n) = snap.linked_reach() {
                sub.push(format!("applies to {n} linked copies"));
            }
            count_units(snap, many)
        }
    };
    (title, sub.join(" · "))
}

/// "3 objects", "2 groups", "1 group and 2 objects": what the selection
/// moves as, counted as the canvas counts it.
pub fn count_units(snap: &Snapshot, units: &[u64]) -> String {
    let groups = units.iter().filter(|u| snap.is_group(**u)).count();
    let objects = units.len() - groups;
    let word = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    match (groups, objects) {
        (0, n) => word(n, "object", "objects"),
        (g, 0) => word(g, "group", "groups"),
        (g, n) => format!(
            "{} and {}",
            word(g, "group", "groups"),
            word(n, "object", "objects")
        ),
    }
}

/// The card of the heading.
pub const HEADING_CARD: &str = "What the panel below changes: the selected objects. Click the job line above to see the job instead.";

/// The sentence on linked copies in the heading's and the sections'
/// cards, when the selection has some.
pub fn linked_note(snap: &Snapshot) -> Option<String> {
    snap.linked_reach().map(|n| {
        format!("Linked copies share the cut options, the keep mode and the scale: a change here applies to all {n} linked copies. Unlink a copy in Arrange to change it alone.")
    })
}

/// The object inspector: the heading and its sections.
pub fn panel(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    show_debug: &mut bool,
    actions: &mut Vec<Action>,
) {
    let (title, sub) = heading(snap);
    let card = match linked_note(snap) {
        Some(n) => format!("{HEADING_CARD} {n}"),
        None => HEADING_CARD.to_string(),
    };
    blocks::heading(ui, &title, &sub, &card);
    cut(ui, st, snap, actions);
    position(ui, st, snap, actions);
    arrange(ui, st, snap, show_debug, actions);
    source(ui, st, snap, actions);
    hints(ui, st, snap, t, actions);
}

/// The summary of the Cut section: cut mode and keep ("Mixed" when they
/// differ).
pub fn cut_summary(snap: &Snapshot) -> String {
    let mode = common(snap.selected().map(|o| o.cut.mode)).map_or("Mixed", mode_name);
    let keep = common(snap.selected().map(|o| o.keep)).map_or("Mixed", keep_name);
    format!("{mode} · {keep}")
}

fn cut(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    let card = match linked_note(snap) {
        Some(n) => format!("{CUT_CARD} {n}"),
        None => CUT_CARD.to_string(),
    };
    blocks::section(
        ui,
        CUT,
        "Cut",
        &card,
        &cut_summary(snap),
        true,
        |ui| cut_body(ui, st, snap, actions),
        None,
    );
}

fn cut_body(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    let doc_id = snap.doc_id;
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let h = metrics::CONTROL;
    blocks::labels(ui, &["Cut mode", "Keep", "Strokes"]);

    let mode = common(snap.selected().map(|o| o.cut.mode));
    let modes = CUT_MODES.map(|m| (m, mode_name(m).to_string()));
    let mut picked = None;
    blocks::row(ui, "Cut mode", Setting::CutMode.card(), None, |ui| {
        let (r, p) = job::combo(
            ui,
            "object-cut-mode",
            mode.map_or("Mixed", mode_name),
            &modes,
            &CUT_MODES.map(mode_hint),
            mode.as_ref(),
            mode.as_ref(),
            true,
        );
        picked = p;
        r
    });
    if let Some(m) = picked {
        actions.push(Action::run(
            doc_id,
            "object.set_cut_options",
            json!({ "mode": m.key() }),
        ));
    }

    let keep = common(snap.selected().map(|o| o.keep));
    let kinds = [KeepMode::Pieces, KeepMode::Stencil, KeepMode::Both];
    let choices = kinds.map(|k| (k, keep_name(k).to_string()));
    let mut picked = None;
    let mut pick = false;
    blocks::row(ui, "Keep", Setting::Keep.card(), None, |ui| {
        let w = (ui.available_width() - button_size().x - metrics::SPACE_4).max(24.0);
        let (r, p) = ui
            .allocate_ui(vec2(w, h), |ui| {
                job::combo(
                    ui,
                    "object-keep",
                    keep.map_or("Mixed", keep_name),
                    &choices,
                    &kinds.map(keep_hint),
                    keep.as_ref(),
                    keep.as_ref(),
                    true,
                )
            })
            .inner;
        picked = p;
        let why = keep_pick::why_not(snap, &ids);
        pick = icon_button(
            ui,
            Icon::Pipette,
            keep_pick::BUTTON,
            keep_pick::BUTTON,
            PICK_CARD,
            why,
        )
        .clicked();
        r
    });
    if let Some(k) = picked {
        actions.push(Action::run(
            doc_id,
            "object.set_keep",
            json!({ "keep": k.key() }),
        ));
    }
    if pick {
        st.keep_pick = keep_pick::KeepPick::start(snap, &ids);
    }

    let strokes = common(snap.selected().map(|o| o.cut.strokes));
    let kinds = [
        StrokeMode::Auto,
        StrokeMode::CentreLine,
        StrokeMode::Outline,
    ];
    let choices = kinds.map(|m| (m, strokes_name(m).to_string()));
    // All lines cuts strokes along their middle: nothing to choose.
    let why = snap
        .selected()
        .all(|o| o.cut.mode == CutMode::AllLines)
        .then_some(STROKES_ALL_LINES);
    let mut picked = None;
    blocks::row_dimmed(ui, "Strokes", Setting::Strokes.card(), why, None, |ui| {
        let (r, p) = job::combo(
            ui,
            "object-strokes",
            strokes.map_or("Mixed", strokes_name),
            &choices,
            &[],
            strokes.as_ref(),
            strokes.as_ref(),
            why.is_none(),
        );
        picked = p;
        r
    });
    if let Some(m) = picked {
        let v = match m {
            StrokeMode::Auto => "auto",
            StrokeMode::CentreLine => "centre_line",
            StrokeMode::Outline => "outline",
        };
        actions.push(Action::run(
            doc_id,
            "object.set_cut_options",
            json!({ "strokes": v }),
        ));
    }
}

/// "20", "20.5": a millimetre value for a summary.
fn short(v: f64) -> String {
    tools::trim_number((v * 10.0).round() / 10.0)
}

/// The summary of Position and size: "X 20 · Y 20 · 225 x 85".
pub fn position_summary(snap: &Snapshot, st: &PrepareState) -> String {
    match canvas::live_box(snap, st) {
        Some(b) => format!(
            "X {} · Y {} · {} x {}",
            short(b.x0),
            short(b.y0),
            short(b.width()),
            short(b.height())
        ),
        None => String::new(),
    }
}

fn position(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    let summary = position_summary(snap, st);
    blocks::section(
        ui,
        POSITION,
        "Position and size",
        &match linked_note(snap) {
            Some(n) => format!("{POSITION_CARD} {n}"),
            None => POSITION_CARD.to_string(),
        },
        &summary,
        false,
        |ui| {
            // Its fields are on screen: an edit in them stays open.
            st.inspector_fields = true;
            blocks::labels(ui, &FIELDS.map(|(_, l)| l));
            let mut wh = [egui::Rect::NOTHING; 2];
            let b = canvas::live_box(snap, st);
            for (field, label) in FIELDS {
                let card = ribbon::inspector_card(field);
                let r = blocks::row(ui, label, &card, None, |ui| {
                    ribbon::inspector_field(ui, field, b, st, snap, actions)
                });
                match field {
                    SizeField::Width => wh[0] = r.widget.rect,
                    SizeField::Height => wh[1] = r.widget.rect,
                    _ => {}
                }
            }
            // Keep aspect joins W and H on the right, in the rows' reset
            // slot (these values have no source to reset to).
            // This column is wider than the ribbon's (`LOCK_COLUMN`) on
            // purpose: it is the rows' reset slot, one button wide, so the
            // lock lines up with the reset arrows of the rows above it.
            let h = metrics::CONTROL;
            let left = wh[0].right() + metrics::SPACE_4;
            let column = egui::Rect::from_x_y_ranges(left..=left + h, wh[0].top()..=wh[1].bottom());
            ribbon::keep_aspect(ui, FieldPlace::Inspector, st, snap, column, wh, true);
        },
        None,
    );
}

/// The size of a small icon button.
fn button_size() -> egui::Vec2 {
    vec2(metrics::ICON_BUTTON, metrics::ICON_BUTTON)
}

/// The icon of a small button, fitting [`button_size`].
/// A dimmed one is drawn faint, so it reads as not available.
fn icon_image(ui: &Ui, icon: Icon, name: &str, dimmed: bool) -> egui::Image<'static> {
    let size = metrics::ICON_COMPACT;
    let image = groups::image(ui, icon, size).alt_text(name.to_string());
    if dimmed {
        image.tint(egui::Color32::WHITE.gamma_multiply(0.35))
    } else {
        image
    }
}

/// A small icon button named `name`, with its card (and why it is
/// dimmed). Returns its response.
fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    name: &str,
    title: &str,
    card: &str,
    why: Option<&str>,
) -> Response {
    let r = ui
        .add_enabled_ui(why.is_none(), |ui| {
            ui.spacing_mut().button_padding = vec2(metrics::SPACE_2, metrics::SPACE_2);
            ui.add_sized(
                button_size(),
                egui::Button::image(icon_image(ui, icon, name, why.is_some())),
            )
        })
        .inner;
    set_name(ui, &r, name);
    groups::card(&r, title, card, why, false);
    r
}

/// The ribbon action `act` as a small icon button named by its title.
/// A click runs it as the ribbon runs it.
fn tool_button(
    ui: &mut Ui,
    act: Act,
    st: &mut PrepareState,
    snap: &Snapshot,
    show_debug: &mut bool,
    actions: &mut Vec<Action>,
) {
    let tool = tools::by_act(act);
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let why = tool.blocked(snap);
    let r = ui
        .add_enabled_ui(why.is_none(), |ui| {
            ui.spacing_mut().button_padding = vec2(metrics::SPACE_2, metrics::SPACE_2);
            ui.add_sized(
                button_size(),
                egui::Button::image(icon_image(ui, tool.icon, tool.title, why.is_some())),
            )
        })
        .inner;
    set_name(ui, &r, tool.title);
    // Titled as it is named: the Align option is the switch below.
    groups::card(
        &r,
        tool.title,
        tool.live_description(snap, &ids),
        why,
        false,
    );
    if r.clicked() {
        tool.run(snap, st, show_debug, actions);
    }
}

/// The summary of Arrange.
pub const ARRANGE_SUMMARY: &str = "align, order, group";

fn arrange(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    show_debug: &mut bool,
    actions: &mut Vec<Action>,
) {
    blocks::section_named(
        ui,
        ARRANGE,
        "Arrange",
        ARRANGE_NAME,
        ARRANGE_CARD,
        ARRANGE_SUMMARY,
        false,
        |ui| arrange_body(ui, st, snap, show_debug, actions),
        None,
    );
}

fn arrange_body(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    show_debug: &mut bool,
    actions: &mut Vec<Action>,
) {
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    blocks::labels(ui, &["Align", "Distribute", "Order", "Groups"]);
    let to = AlignTo::of(st.fields.align_to_area);
    blocks::actions_row(ui, "Align", Item::Align.description(), |ui| {
        for mode in [
            AlignMode::Left,
            AlignMode::Centre,
            AlignMode::Right,
            AlignMode::Top,
            AlignMode::Middle,
            AlignMode::Bottom,
        ] {
            tool_button(ui, Act::Align(mode, to), st, snap, show_debug, actions);
        }
    });
    blocks::actions_row(ui, "", "What Align lines the objects up with.", |ui| {
        let mut on = st.fields.align_to_area;
        let r = ui.checkbox(&mut on, AlignTo::Area.label());
        set_name(ui, &r, ALIGN_TO_AREA);
        groups::card(
            &r,
            ALIGN_TO_AREA,
            "On: Align lines the objects up with the cutting area, the part of the mat the blade reaches. Off: with each other. The ribbon's Align list has the same choice.",
            None,
            false,
        );
        st.fields.align_to_area = on;
    });
    blocks::actions_row(ui, "Distribute", Item::Distribute.description(), |ui| {
        for act in [Act::DistributeHorizontal, Act::DistributeVertical] {
            tool_button(ui, act, st, snap, show_debug, actions);
        }
    });
    blocks::actions_row(
        ui,
        "Order",
        "What lies in front of what. Objects lower in the Objects list lie in front; Visible outline cuts only what can be seen.",
        |ui| {
            for to in [
                ReorderTo::Front,
                ReorderTo::Forward,
                ReorderTo::Backward,
                ReorderTo::Back,
            ] {
                tool_button(ui, Act::Order(to), st, snap, show_debug, actions);
            }
        },
    );
    // Groups and linked copies: always the same four buttons, dimmed
    // with their reason when they do not apply, so the section keeps its
    // height whatever is selected.
    blocks::actions_row(
        ui,
        "Groups",
        "Group objects so they move, scale and turn as one. Linked copies share their source, cut options and scale.",
        |ui| {
            for (item, icon) in GROUP_ITEMS {
                item_button(ui, item, icon, st, snap, &ids, actions);
            }
        },
    );
}

/// The buttons of Arrange's Groups row, with their icons.
pub const GROUP_ITEMS: [(Item, Icon); 4] = [
    (Item::Group, Icon::Group),
    (Item::Ungroup, Icon::Ungroup),
    (Item::LinkCopies, Icon::Linked),
    (Item::UnlinkCopies, Icon::BrokenLink),
];

/// The buttons of the Source section, with their icons.
pub const SOURCE_ITEMS: [(Item, Icon); 6] = [
    (Item::RevealSource, Icon::SvgFile),
    (Item::CopySourcePath, Icon::Duplicate),
    (Item::Reload, Icon::Reload),
    (Item::Reconnect, Icon::Plug),
    (Item::TraceAgain, Icon::Trace),
    (Item::UnlinkFile, Icon::FileX),
];

/// Why Link copies is dimmed for these objects.
pub const LINKED_ALREADY: &str = "These are linked copies already.";
pub const OTHER_DESIGNS: &str =
    "Only copies of one design can be linked. These come from different designs.";
/// Why Unlink is dimmed.
pub const NOT_A_LINKED_COPY: &str = "None of these is a linked copy.";
/// Why the file actions are dimmed for objects without a file.
pub const NOT_LINKED: &str = "Not linked to a file: these objects were not imported from one, so there is no file to show or read again.";
/// Why Reload is dimmed while the file is gone.
pub const FILE_GONE: &str = "The file is not where it was. Use Reconnect source first.";
/// Why Reconnect is dimmed while the file is where it was.
pub const FILE_THERE: &str = "The file is where it was, so there is nothing to reconnect.";
/// Why Trace again is dimmed for drawings.
pub const NOT_A_PICTURE: &str = "Only pictures can be traced again. These are drawings.";

/// Why a Groups or Source button cannot run on `ids` now: the context
/// menu's reason, or why the menu would not list it.
pub fn item_why(
    item: Item,
    snap: &Snapshot,
    st: &PrepareState,
    ids: &[u64],
) -> Option<&'static str> {
    let objs: Vec<_> = ids.iter().filter_map(|id| snap.object(*id)).collect();
    let linked = objs.iter().any(|o| o.link.is_some());
    let broken = objs
        .iter()
        .any(|o| o.link.is_some() && st.sources.broken(o.id));
    let whole = objs
        .iter()
        .any(|o| o.link.is_some() && !st.sources.broken(o.id));
    let menu = || item.why(snap, st, ids);
    match item {
        Item::LinkCopies => {
            let one = objs.first().and_then(|o| o.link_group).is_some()
                && objs.iter().all(|o| o.link_group == objs[0].link_group);
            if one {
                Some(LINKED_ALREADY)
            } else if objs.len() > 1 && !objs.iter().all(|o| o.same_source(objs[0])) {
                Some(OTHER_DESIGNS)
            } else {
                menu()
            }
        }
        Item::UnlinkCopies => (!objs.iter().any(|o| o.link_group.is_some()))
            .then_some(NOT_A_LINKED_COPY)
            .or_else(menu),
        Item::RevealSource | Item::CopySourcePath => {
            if !objs.iter().any(|o| o.source_link.is_some()) {
                Some(NOT_LINKED)
            } else {
                menu()
            }
        }
        Item::Reload if !linked => Some(NOT_LINKED),
        Item::Reload if !whole => Some(FILE_GONE),
        Item::Reconnect if !linked => Some(NOT_LINKED),
        Item::Reconnect if !broken => Some(FILE_THERE),
        Item::TraceAgain if !objs.iter().any(|o| o.raster) => Some(NOT_A_PICTURE),
        Item::UnlinkFile if !linked => Some(context_menu::NO_FILE_TO_UNLINK),
        _ => menu(),
    }
}

/// A context menu item as a small icon button, named by its text, with
/// its card and why it is dimmed. A click runs it as the menu runs it.
fn item_button(
    ui: &mut Ui,
    item: Item,
    icon: Icon,
    st: &mut PrepareState,
    snap: &Snapshot,
    ids: &[u64],
    actions: &mut Vec<Action>,
) {
    let text = item.text(snap, ids);
    let why = item_why(item, snap, st, ids);
    let r = icon_button(ui, icon, &text, &text, item.card_text(snap, ids), why);
    if r.clicked() && why.is_none() {
        context_menu::run_item(item, snap, st, ids, actions);
    }
}

/// What the File row says: the file's name, "2 files", or that there is
/// none; and its card (the whole path).
pub fn source_file(snap: &Snapshot) -> (String, String) {
    let mut paths: Vec<&std::path::Path> = snap
        .selected()
        .filter_map(|o| Some(o.source_link.as_ref()?.path.as_path()))
        .collect();
    paths.sort();
    paths.dedup();
    match paths.as_slice() {
        [] => ("Not linked to a file".into(), NOT_LINKED.into()),
        [one] => (
            one.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            format!(
                "The file these objects were imported from: {}. FlatCraft keeps its own copy in the project; Reload reads the file again after you changed it.",
                one.display()
            ),
        ),
        many => (
            format!("{} files", many.len()),
            "The selected objects come from different files. Select objects from one file to reveal it.".into(),
        ),
    }
}

fn source(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    let (file, card) = source_file(snap);
    let mut summary = file.clone();
    if snap.selected().any(|o| st.sources.broken(o.id)) {
        summary = format!("{summary} · not found");
    }
    blocks::section(
        ui,
        SOURCE,
        "Source",
        SOURCE_CARD,
        &summary,
        false,
        |ui| {
            blocks::labels(ui, &["File"]);
            blocks::value_row(ui, "File", &card, &file);
            let ids: Vec<u64> = snap.selection.iter().copied().collect();
            // The same buttons in every case, dimmed with their reason.
            blocks::actions_row(ui, "", SOURCE_CARD, |ui| {
                for (item, icon) in SOURCE_ITEMS {
                    item_button(ui, item, icon, st, snap, &ids, actions);
                }
            });
        },
        None,
    );
}

// ---------------------------------------------------------------- hints

/// What the hints are checked for: the document, its revision, the
/// profiles' revision and the selection.
type HintKey = (u64, u64, u64, Vec<u64>);

/// Checks objects of a job input: `report::check_selection_until`, which
/// gives up (`None`) when its third argument says stop; tests replace it
/// (a slow one, one that notes its thread).
pub type HintChecker = std::sync::Arc<
    dyn Fn(
            &JobInput,
            &[flatcraft_core::document::ObjectId],
            &dyn Fn() -> bool,
        ) -> Option<(flatcraft_engine::report::Report, bool)>
        + Send
        + Sync,
>;

/// What a check found.
#[derive(Debug, Clone)]
enum Outcome {
    Issues(Vec<Issue>),
    /// There was no plotter or material to check with.
    NoProfiles,
    /// The worker could not be started.
    Failed,
}

/// A finished check.
type Checked = (HintKey, Outcome);

/// What the worker sends back: the check, and the input when its cut
/// geometry is the whole document's (it goes back into the cache).
type Landed = (Checked, Option<JobInput>);

/// A check on the worker: its key, where its result comes, the flag that
/// tells it to stop, and the flag it sets when it ended.
type Running = (
    HintKey,
    std::sync::mpsc::Receiver<Landed>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
);

/// How many stopped checks may still be winding down (a cut geometry
/// build in progress ends before it can stop) before a new one waits.
const MAX_STOPPING: usize = 2;

/// The report's issues for the selected objects, for the object
/// inspector's Hints. The check runs on a worker thread, keyed by
/// document, revision, profiles and selection; a frame never waits for
/// it. A newer ask stops a running check of something else (it gives up
/// between objects and pairs) and starts at once. Meanwhile the last
/// result for the same document and selection shows, faded.
pub struct Hints {
    done: Option<Checked>,
    running: Option<Running>,
    /// The newest check asked for while one runs.
    queued: Option<(HintKey, Option<JobInput>)>,
    /// The ended-flags of stopped checks that may still run.
    stopping: Vec<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    checker: HintChecker,
    /// Starting a worker failed once: it was said once, not every frame.
    told_failure: bool,
    /// Tests: every start of a worker fails.
    fail_spawns: bool,
    /// How many checks were started (tests).
    pub started: u64,
    /// How many starts were tried, failed ones included (tests).
    pub tried: u64,
}

impl std::fmt::Debug for Hints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hints")
            .field("done", &self.done.as_ref().map(|(k, _)| k))
            .field("running", &self.running.as_ref().map(|r| &r.0))
            .field("queued", &self.queued.as_ref().map(|(k, _)| k))
            .field("stopping", &self.stopping.len())
            .finish()
    }
}

/// A copy keeps the finished check but not the worker's.
impl Clone for Hints {
    fn clone(&self) -> Self {
        Self {
            done: self.done.clone(),
            running: None,
            queued: None,
            stopping: Vec::new(),
            checker: self.checker.clone(),
            told_failure: self.told_failure,
            fail_spawns: self.fail_spawns,
            started: self.started,
            tried: self.tried,
        }
    }
}

impl Default for Hints {
    fn default() -> Self {
        Self {
            done: None,
            running: None,
            queued: None,
            stopping: Vec::new(),
            checker: std::sync::Arc::new(|input, ids, stop| {
                flatcraft_engine::report::check_selection_until(input, ids, stop)
            }),
            told_failure: false,
            fail_spawns: false,
            started: 0,
            tried: 0,
        }
    }
}

/// What the Hints section shows for the snapshot's document and
/// selection.
#[derive(Debug, Clone, Copy)]
pub struct HintsView<'a> {
    /// The last finished check of these objects; `None` before the first.
    pub issues: Option<&'a [Issue]>,
    /// The document or the profiles changed since: it is shown faded.
    pub stale: bool,
    /// A check of the current state runs or waits.
    pub checking: bool,
    /// There is no plotter or material to check with.
    pub no_profiles: bool,
    /// The check could not be started.
    pub failed: bool,
}

impl Hints {
    fn key_of(snap: &Snapshot) -> HintKey {
        (
            snap.doc_id,
            snap.doc_rev,
            snap.profiles_rev,
            snap.selection.iter().copied().collect(),
        )
    }

    /// Checks with `f` instead of the engine (tests).
    pub fn set_checker(&mut self, f: HintChecker) {
        self.checker = f;
    }

    /// Makes every start of a worker fail (tests).
    pub fn fail_spawns(&mut self, yes: bool) {
        self.fail_spawns = yes;
    }

    /// Forgets a check that could not be started, so the next frame
    /// tries again (the person selected the objects again).
    pub fn forget_failure(&mut self) {
        if matches!(self.done, Some((_, Outcome::Failed))) {
            self.done = None;
        }
    }

    /// How many stopped checks may still run (tests).
    pub fn stopping(&mut self) -> usize {
        use std::sync::atomic::Ordering;
        self.stopping.retain(|ended| !ended.load(Ordering::SeqCst));
        self.stopping.len()
    }

    /// A check runs or waits.
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.queued.is_some()
    }

    /// The current state was checked, or its check runs or waits.
    pub fn asked(&self, snap: &Snapshot) -> bool {
        let key = Self::key_of(snap);
        self.done.as_ref().is_some_and(|(k, _)| *k == key)
            || self.running.as_ref().is_some_and(|r| r.0 == key)
            || self.queued.as_ref().is_some_and(|(k, _)| *k == key)
    }

    /// Asks for a check of `snap`'s state with `input` (`None`: no
    /// plotter or material). A running check of something else is
    /// stopped, and this one starts at once.
    pub fn ask(&mut self, ctx: &egui::Context, snap: &Snapshot, input: Option<JobInput>) {
        self.queued = Some((Self::key_of(snap), input));
        self.pump(ctx);
    }

    /// Takes a finished check (and starts the waiting one). Returns the
    /// input of a check whose cut geometry is the whole document's, with
    /// its document and revision, for the document's cache. Never waits.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<(u64, u64, JobInput)> {
        let mut offer = None;
        if let Some(r) = &self.running {
            match r.1.try_recv() {
                Ok((checked, input)) => {
                    let (doc, rev) = (checked.0.0, checked.0.1);
                    offer = input.map(|i| (doc, rev, i));
                    self.done = Some(checked);
                    self.running = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                // The worker died (a panic): the objects cannot be checked
                // now; no frame tries again until they or the document
                // change, or they are selected again.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(r) = self.running.take() {
                        self.done = Some((r.0, Outcome::Failed));
                    }
                }
            }
        }
        self.pump(ctx);
        offer
    }

    /// Starts the waiting check: at once when nothing runs, else after
    /// stopping the running check (of an older state), unless too many
    /// stopped ones still wind down.
    fn pump(&mut self, ctx: &egui::Context) {
        use std::sync::atomic::Ordering;
        self.stopping.retain(|ended| !ended.load(Ordering::SeqCst));
        if self.queued.is_none() {
            return;
        }
        if let Some(r) = &self.running {
            if self.stopping.len() >= MAX_STOPPING {
                return;
            }
            r.2.store(true, Ordering::SeqCst);
            if let Some(r) = self.running.take() {
                self.stopping.push(r.3);
            }
        }
        let Some((key, input)) = self.queued.take() else {
            return;
        };
        let Some(input) = input else {
            self.done = Some((key, Outcome::NoProfiles));
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let checker = self.checker.clone();
        let repaint = ctx.clone();
        let k = key.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop2, ended2) = (stop.clone(), ended.clone());
        self.tried += 1;
        let spawned = if self.fail_spawns {
            Err(std::io::Error::other("no threads in this test"))
        } else {
            std::thread::Builder::new()
                .name("flatcraft-hints".into())
                .spawn(move || {
                    // Says it ended, then asks for a frame, also when the
                    // check panics: its slot among the stopping checks
                    // frees and an idle window starts a waiting check.
                    struct Ended(std::sync::Arc<std::sync::atomic::AtomicBool>, egui::Context);
                    impl Drop for Ended {
                        fn drop(&mut self) {
                            self.0.store(true, Ordering::SeqCst);
                            self.1.request_repaint();
                        }
                    }
                    let _ended = Ended(ended2, repaint);
                    let ids: Vec<flatcraft_core::document::ObjectId> =
                        k.3.iter()
                            .map(|i| flatcraft_core::document::ObjectId(*i))
                            .collect();
                    let stopped = || stop2.load(Ordering::SeqCst);
                    if let Some((report, whole)) = checker(&input, &ids, &stopped) {
                        let input =
                            (whole && input.built_cut_geometry().is_some()).then_some(input);
                        let _ = tx.send(((k, Outcome::Issues(report.issues)), input));
                    }
                })
                .map(|_| ())
        };
        if let Err(e) = spawned {
            // Said once; the state is marked as checked, so no frame
            // tries again until the objects or the document change.
            if !self.told_failure {
                eprintln!("flatcraft: cannot start the hints worker: {e}");
                self.told_failure = true;
            }
            self.done = Some((key, Outcome::Failed));
            return;
        }
        self.started += 1;
        self.running = Some((key, rx, stop, ended));
    }

    /// What to show for `snap`'s document and selection. A check of
    /// another document or selection is never shown (a tab switch never
    /// shows the other document's hints).
    pub fn view(&self, snap: &Snapshot) -> HintsView<'_> {
        let key = Self::key_of(snap);
        let same = |k: &HintKey| k.0 == key.0 && k.3 == key.3;
        let done = self.done.as_ref().filter(|(k, _)| same(k));
        let checking = self.running.as_ref().is_some_and(|r| r.0 == key)
            || self.queued.as_ref().is_some_and(|(k, _)| *k == key);
        let now = |o: fn(&Outcome) -> bool| done.is_some_and(|(k, d)| *k == key && o(d));
        HintsView {
            issues: done.and_then(|(_, d)| match d {
                Outcome::Issues(i) => Some(i.as_slice()),
                _ => None,
            }),
            stale: done.is_some_and(|(k, _)| *k != key),
            checking,
            no_profiles: now(|d| matches!(d, Outcome::NoProfiles)),
            failed: now(|d| matches!(d, Outcome::Failed)),
        }
    }
}

/// The hints of `view` to show, the most severe first, and how many of
/// them the import report on the canvas shows now (those are left out
/// here, so no hint shows twice).
pub fn hint_issues<'a>(
    st: &PrepareState,
    snap: &Snapshot,
    view: &HintsView<'a>,
) -> (Vec<&'a Issue>, usize) {
    let checked = view.issues.unwrap_or_default();
    let card: Vec<&Issue> = st
        .report
        .as_ref()
        .filter(|c| c.doc_id == snap.doc_id)
        .map(|c| c.all())
        .unwrap_or_default();
    let on_card = |i: &Issue| {
        card.iter()
            .any(|c| c.code == i.code && c.object_ids == i.object_ids)
    };
    let (shown, there): (Vec<&Issue>, Vec<&Issue>) = checked.iter().partition(|i| !on_card(i));
    (report::by_severity(shown.into_iter()), there.len())
}

/// "1 hint", "3 hints", "none".
pub fn hints_summary(n: usize) -> String {
    match n {
        0 => "none".into(),
        1 => "1 hint".into(),
        n => format!("{n} hints"),
    }
}

/// What the Hints section says about the hints the import report on the
/// canvas shows.
pub fn on_the_card(n: usize) -> String {
    match n {
        1 => "1 hint is in the report on the canvas.".into(),
        n => format!("{n} hints are in the report on the canvas."),
    }
}

/// What the Hints section says when there is nothing to tell.
pub const NO_HINTS: &str = "Nothing to tell. These objects are good to go.";
/// While the objects are checked.
pub const CHECKING: &str = "checking...";
/// When there is nothing to check with.
pub const NO_PROFILES: &str = "Pick a plotter and a material to check these objects.";
/// When the check could not be started.
pub const CANNOT_CHECK: &str =
    "These objects cannot be checked right now. Select them again to try once more.";
/// Why the buttons of an older check are dimmed.
pub const STALE_FIX: &str = "Wait a moment: these objects changed and are checked again.";

/// At most this many hint cards; the rest is counted.
pub const MAX_CARDS: usize = 8;

/// "and 3 more hints. Select fewer objects to see them."
pub fn more_hints(n: usize) -> String {
    if n == 1 {
        "And 1 more hint. Select fewer objects to see it.".into()
    } else {
        format!("And {n} more hints. Select fewer objects to see them.")
    }
}

fn hints(ui: &mut Ui, st: &PrepareState, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    let view = st.object_hints.view(snap);
    let (issues, on_card) = hint_issues(st, snap, &view);
    // A running check says so in the header's one line, so the cards
    // below (the last check, faded) never move while it runs. Open, the
    // header says nothing else: the body shows the hints.
    let summary = if view.checking {
        CHECKING.to_string()
    } else {
        hints_summary(issues.len() + on_card)
    };
    let live = if view.checking { CHECKING } else { "" };
    blocks::section_live(ui, HINTS, "Hints", HINTS_CARD, &summary, live, true, |ui| {
        // What the last check found when there is no card to show.
        let status = if view.no_profiles {
            NO_PROFILES
        } else if view.failed {
            CANNOT_CHECK
        } else if on_card > 0 {
            ""
        } else if view.issues.is_some_and(|_| issues.is_empty()) {
            NO_HINTS
        } else {
            ""
        };
        // Everything the last check found fades while it is checked again.
        ui.scope(|ui| {
            if view.stale {
                ui.multiply_opacity(0.5);
            }
            if !status.is_empty() {
                blocks::status_line(ui, status, t.text_dim);
            }
            if on_card > 0 {
                blocks::note(ui, &on_the_card(on_card), t.text_dim);
            }
            for (n, issue) in issues.iter().take(MAX_CARDS).enumerate() {
                if n > 0 {
                    ui.add_space(metrics::SPACE_4);
                }
                let fixes: Vec<_> = issue
                    .fixes
                    .iter()
                    .filter_map(|f| Some((f, report::fix_action(snap.doc_id, f)?)))
                    .collect();
                let why = view.stale.then_some(STALE_FIX);
                let buttons: Vec<HintButton> = fixes
                    .iter()
                    .map(|(f, _)| HintButton {
                        label: &f.label,
                        card: &f.description,
                        why,
                    })
                    .collect();
                // Without a fix the hint says what to do.
                let detail = buttons.is_empty().then_some(issue.hint.as_str());
                if let Some(i) = blocks::hint_card_with(
                    ui,
                    t,
                    None,
                    Some((issue.code.title(), &issue.hint)),
                    &issue.message,
                    detail,
                    // An error stops the cut: red, like the import card.
                    issue.severity == flatcraft_engine::report::Severity::Error,
                    &buttons,
                ) {
                    actions.push(fixes[i].1.clone());
                }
            }
            if issues.len() > MAX_CARDS {
                blocks::note(ui, &more_hints(issues.len() - MAX_CARDS), t.text_dim);
            }
        });
    });
}
