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
use super::super::{Action, canvas, groups, keep_pick, report, ribbon};
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
/// object (a file, a picture) and are listed by [`source_items`].
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

/// What the heading says: the title and the line under it.
///
/// - One group: "Group 'Badge'", its objects under it.
/// - One object: its name, with "· in group Badge" when it is in a group;
///   its size, its file, "Picture" and its linked copies under it.
/// - More: "2 objects", their names under it, with "· in group Badge"
///   when they share a group.
pub fn heading(snap: &Snapshot) -> (String, String) {
    let units = snap.units();
    let parent = |id: u64| {
        snap.tree
            .parent(flatcraft_core::document::ObjectId(id))
            .and_then(|g| snap.name_of(g.0))
    };
    let names = |ids: &[u64]| {
        ids.iter()
            .filter_map(|id| snap.name_of(*id))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let in_group = |ids: &[u64]| {
        let first = ids.first().map(|id| parent(*id))?;
        let all = ids.iter().all(|id| parent(*id) == first);
        all.then_some(first).flatten()
    };
    match units.as_slice() {
        [] => (String::new(), String::new()),
        [g] if snap.is_group(*g) => {
            let name = snap.name_of(*g).unwrap_or_default();
            let members: Vec<u64> = snap
                .tree
                .children(Some(flatcraft_core::document::ObjectId(*g)))
                .into_iter()
                .map(|c| c.0)
                .collect();
            let mut sub = names(&members);
            if let Some(p) = parent(*g) {
                sub = format!("{sub} · in group {p}");
            }
            (format!("Group '{name}'"), sub)
        }
        [one] => {
            let o = snap.object(*one);
            let name = snap.name_of(*one).unwrap_or_default();
            let title = match parent(*one) {
                Some(p) => format!("{name} · in group {p}"),
                None => name,
            };
            let mut sub = Vec::new();
            if let Some(b) = snap.box_of(&[*one]) {
                sub.push(format!("{} x {} mm", short(b.width()), short(b.height())));
            }
            if let Some(file) = o
                .and_then(|o| o.source_link.as_ref())
                .and_then(|l| l.path.file_name())
            {
                sub.push(file.to_string_lossy().into_owned());
            }
            if o.is_some_and(|o| o.raster) {
                sub.push("Picture".into());
            }
            if let Some(g) = o.and_then(|o| o.link_group) {
                sub.push(format!("1 of {} linked copies", snap.group_size(g)));
            }
            (title, sub.join(" · "))
        }
        many => {
            let mut sub = names(many);
            if let Some(p) = in_group(many) {
                sub = format!("{sub} · in group {p}");
            }
            (format!("{} objects", many.len()), sub)
        }
    }
}

/// The card of the heading.
pub const HEADING_CARD: &str = "What the panel below changes: the selected objects. Click the job line above to see the job instead.";

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
    blocks::heading(ui, &title, &sub, HEADING_CARD);
    ui.add_space(t.spacing / 2.0);
    cut(ui, st, snap, t, actions);
    position(ui, st, snap, t, actions);
    arrange(ui, st, snap, show_debug, actions);
    source(ui, st, snap, t, actions);
    hints(ui, st, snap, t, actions);
}

/// The summary of the Cut section: cut mode and keep ("Mixed" when they
/// differ).
pub fn cut_summary(snap: &Snapshot) -> String {
    let mode = common(snap.selected().map(|o| o.cut.mode)).map_or("Mixed", mode_name);
    let keep = common(snap.selected().map(|o| o.keep)).map_or("Mixed", keep_name);
    format!("{mode} · {keep}")
}

fn cut(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    blocks::section(
        ui,
        CUT,
        "Cut",
        CUT_CARD,
        &cut_summary(snap),
        true,
        |ui| cut_body(ui, st, snap, t, actions),
        None,
    );
}

fn cut_body(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let doc_id = snap.doc_id;
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let h = ui.spacing().interact_size.y;

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
        let w = (ui.available_width() - button_size(ui).x - 4.0).max(24.0);
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
    if let Some(n) = snap.linked_reach() {
        let r = blocks::note(ui, &format!("Applies to {n} linked copies"), t.text_dim);
        groups::card(
            &r,
            "Linked copies",
            "Linked copies share the cut options, the keep mode and the scale. Unlink a copy in Arrange or the Objects panel to change it alone.",
            None,
            false,
        );
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

fn position(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let summary = position_summary(snap, st);
    blocks::section(
        ui,
        POSITION,
        "Position and size",
        POSITION_CARD,
        &summary,
        false,
        |ui| {
            // Its fields are on screen: an edit in them stays open.
            st.inspector_fields = true;
            let mut wh = [egui::Rect::NOTHING; 2];
            for (field, label) in FIELDS {
                let card = ribbon::inspector_card(field, snap, st);
                let r = blocks::row(ui, label, &card, None, |ui| {
                    ribbon::inspector_field(ui, field, st, snap, actions)
                });
                match field {
                    SizeField::Width => wh[0] = r.widget.rect,
                    SizeField::Height => wh[1] = r.widget.rect,
                    _ => {}
                }
            }
            // Keep aspect joins W and H on the right, in the rows' reset
            // slot (these values have no source to reset to).
            let h = ui.spacing().interact_size.y;
            let left = wh[0].right() + 4.0;
            let column = egui::Rect::from_x_y_ranges(left..=left + h, wh[0].top()..=wh[1].bottom());
            ribbon::keep_aspect(ui, FieldPlace::Inspector, st, snap, column, wh, true);
            if let Some(n) = snap.linked_reach() {
                let text = format!("Scales all {n} linked copies");
                let r = blocks::note(ui, &text, t.text_dim);
                groups::card(
                    &r,
                    &text,
                    "Linked copies share the scale. Each keeps its own place.",
                    None,
                    false,
                );
            }
        },
        None,
    );
}

/// The size of a small icon button: a square as high as a row.
fn button_size(ui: &Ui) -> egui::Vec2 {
    let h = ui.spacing().interact_size.y;
    vec2(h + 4.0, h)
}

/// The icon of a small button, fitting [`button_size`].
fn icon_image(ui: &Ui, icon: Icon, name: &str) -> egui::Image<'static> {
    let size = (ui.spacing().interact_size.y - 4.0).clamp(12.0, 16.0);
    groups::image(ui, icon, size).alt_text(name.to_string())
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
            ui.spacing_mut().button_padding = vec2(2.0, 2.0);
            ui.add_sized(
                button_size(ui),
                egui::Button::image(icon_image(ui, icon, name)),
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
            ui.spacing_mut().button_padding = vec2(2.0, 2.0);
            ui.add_sized(
                button_size(ui),
                egui::Button::image(icon_image(ui, tool.icon, tool.title)),
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
    // Groups and linked copies: text buttons, they say what they do
    // ("Remove 2 objects from group").
    let wanted = [
        Item::Group,
        Item::Ungroup,
        Item::LinkCopies,
        Item::UnlinkCopies,
    ];
    let offered: Vec<Item> = context_menu::items(snap, st, &ids)
        .into_iter()
        .flatten()
        .filter(|i| wanted.contains(i))
        .collect();
    blocks::actions_row(
        ui,
        "Groups",
        "Group objects so they move, scale and turn as one. Linked copies share their source, cut options and scale.",
        |ui| {
            for item in offered {
                item_button(ui, item, st, snap, &ids, actions);
            }
        },
    );
}

/// A context menu item as a small text button, with its card and why it
/// is dimmed. A click runs it as the menu runs it.
fn item_button(
    ui: &mut Ui,
    item: Item,
    st: &mut PrepareState,
    snap: &Snapshot,
    ids: &[u64],
    actions: &mut Vec<Action>,
) {
    let text = item.text(snap, ids);
    let why = item.why(snap, st, ids);
    let r = ui.add_enabled(why.is_none(), egui::Button::new(&text));
    set_name(ui, &r, &text);
    groups::card(&r, &text, item.card_text(snap, ids), why, false);
    if r.clicked() {
        context_menu::run_item(item, snap, st, ids, actions);
    }
}

/// The Source actions offered for the selection, in the panel's order:
/// those the context menu offers for it.
pub fn source_items(snap: &Snapshot, st: &PrepareState) -> Vec<Item> {
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let offered: Vec<Item> = context_menu::items(snap, st, &ids)
        .into_iter()
        .flatten()
        .collect();
    [
        Item::RevealSource,
        Item::CopySourcePath,
        Item::Reload,
        Item::Reconnect,
        Item::TraceAgain,
    ]
    .into_iter()
    .filter(|i| offered.contains(i))
    .collect()
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
        [] => (
            "Not linked to a file".into(),
            "These objects were not imported from a file (test shapes, for example), so there is no file to read again.".into(),
        ),
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

fn source(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
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
            blocks::value_row(ui, "File", &card, &file);
            let ids: Vec<u64> = snap.selection.iter().copied().collect();
            let items = source_items(snap, st);
            if items.is_empty() {
                blocks::note(
                    ui,
                    "Nothing to read again: there is no file behind these objects.",
                    t.text_dim,
                );
                return;
            }
            blocks::actions_row(ui, "", SOURCE_CARD, |ui| {
                for item in items {
                    item_button(ui, item, st, snap, &ids, actions);
                }
            });
        },
        None,
    );
}

// ---------------------------------------------------------------- hints

/// The report's issues for the selected objects, checked again when the
/// document, the profiles or the selection changed (`FlatCraftApp`
/// checks them before the frame is drawn, never during a drag).
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// What they were checked for: document, its revision, the profiles'
    /// revision and the selection.
    key: Option<(u64, u64, u64, Vec<u64>)>,
    issues: Vec<Issue>,
}

impl Hints {
    fn key_of(snap: &Snapshot) -> (u64, u64, u64, Vec<u64>) {
        (
            snap.doc_id,
            snap.doc_rev,
            snap.profiles_rev,
            snap.selection.iter().copied().collect(),
        )
    }

    /// The document, the profiles or the selection changed since the
    /// last check.
    pub fn stale(&self, snap: &Snapshot) -> bool {
        self.key.as_ref() != Some(&Self::key_of(snap))
    }

    /// Stores the issues found for `snap` (at the document's revision
    /// `rev`, which may be newer than the snapshot's).
    pub fn store(&mut self, snap: &Snapshot, rev: u64, issues: Vec<Issue>) {
        let mut key = Self::key_of(snap);
        key.1 = rev;
        self.key = Some(key);
        self.issues = issues;
    }

    /// The issues of the snapshot's document and selection, `None` when
    /// they were checked for another document or selection (a tab switch
    /// never shows the other document's hints). During a drag the last
    /// check stays.
    pub fn of(&self, snap: &Snapshot) -> Option<&[Issue]> {
        let (doc, _, _, ids) = self.key.as_ref()?;
        let now: Vec<u64> = snap.selection.iter().copied().collect();
        (*doc == snap.doc_id && *ids == now).then_some(self.issues.as_slice())
    }
}

/// The hints for the selection, the most severe first, and how many of
/// them the import report on the canvas shows now (those are left out
/// here, so no hint shows twice).
pub fn hint_issues<'a>(st: &'a PrepareState, snap: &Snapshot) -> (Vec<&'a Issue>, usize) {
    let checked = st.object_hints.of(snap).unwrap_or_default();
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

fn hints(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let (issues, on_card) = hint_issues(st, snap);
    let issues: Vec<Issue> = issues.into_iter().cloned().collect();
    blocks::section(
        ui,
        HINTS,
        "Hints",
        HINTS_CARD,
        &hints_summary(issues.len() + on_card),
        true,
        |ui| {
            if on_card > 0 {
                blocks::note(ui, &on_the_card(on_card), t.text_dim);
            }
            if issues.is_empty() {
                if on_card == 0 {
                    blocks::note(ui, NO_HINTS, t.text_dim);
                }
                return;
            }
            for (n, issue) in issues.iter().enumerate() {
                if n > 0 {
                    ui.add_space(t.spacing / 2.0);
                }
                let fixes: Vec<_> = issue
                    .fixes
                    .iter()
                    .filter_map(|f| Some((f, report::fix_action(snap.doc_id, f)?)))
                    .collect();
                let buttons: Vec<HintButton> = fixes
                    .iter()
                    .map(|(f, _)| HintButton {
                        label: &f.label,
                        card: &f.description,
                        why: None,
                    })
                    .collect();
                // Without a fix the hint says what to do.
                let detail = buttons.is_empty().then_some(issue.hint.as_str());
                let title = issue.code.title();
                if let Some(i) = blocks::hint_card_with(
                    ui,
                    t,
                    None,
                    Some((title, &issue.hint)),
                    &issue.message,
                    detail,
                    &buttons,
                ) {
                    actions.push(fixes[i].1.clone());
                }
            }
        },
        None,
    );
}
