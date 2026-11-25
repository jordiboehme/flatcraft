//! The Arrange group of the ribbon: the "Arrange" drop-down that lists
//! every arrange command, the commands pinned to the ribbon, and the
//! command panels docked on the right with their ghost preview.
//!
//! A panel works on the current selection. While it is open, the canvas
//! draws the would-be result faintly ([`ghosts`]), built with the same
//! pure functions of `flatcraft_engine::placement` that the commands use,
//! so the preview and the result cannot differ. OK runs one command (one
//! undo step); Cancel, the close button and Escape change nothing.
//!
//! The pins live in the preferences (`arrange_pins`), written by the
//! window only (read-modify-write, see [`Action::SetPin`]).

use egui::{RichText, Ui};
use flatcraft_engine::commands::object::MAX_COPIES;
use flatcraft_engine::placement;
use kurbo::{Affine, Point, Rect, Vec2};
use serde_json::json;

use super::help::{self, PrepareKey};
use super::icons::{self, Icon};
use super::prepare::{ObjView, PrepareState, Snapshot};
use super::settings::{painted_label, set_name};
use super::theme::Tokens;
use super::{Action, ICON_SIZE, ribbon};

/// One command of the Arrange drop-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrangeCmd {
    MoveCopy,
    Rotate,
    Mirror,
    Align,
    Distribute,
    RectangularPattern,
    CircularPattern,
    AutoArrange,
}

impl ArrangeCmd {
    /// Every command, in the order of the drop-down and of the pins.
    pub const ALL: [ArrangeCmd; 8] = [
        ArrangeCmd::MoveCopy,
        ArrangeCmd::Rotate,
        ArrangeCmd::Mirror,
        ArrangeCmd::Align,
        ArrangeCmd::Distribute,
        ArrangeCmd::RectangularPattern,
        ArrangeCmd::CircularPattern,
        ArrangeCmd::AutoArrange,
    ];

    /// The id stored in the preferences' `arrange_pins`.
    pub fn id(self) -> &'static str {
        match self {
            ArrangeCmd::MoveCopy => "move_copy",
            ArrangeCmd::Rotate => "rotate",
            ArrangeCmd::Mirror => "mirror",
            ArrangeCmd::Align => "align",
            ArrangeCmd::Distribute => "distribute",
            ArrangeCmd::RectangularPattern => "rectangular_pattern",
            ArrangeCmd::CircularPattern => "circular_pattern",
            ArrangeCmd::AutoArrange => "auto_arrange",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            ArrangeCmd::MoveCopy => "Move / Copy...",
            ArrangeCmd::Rotate => "Rotate...",
            ArrangeCmd::Mirror => "Mirror",
            ArrangeCmd::Align => "Align",
            ArrangeCmd::Distribute => "Distribute",
            ArrangeCmd::RectangularPattern => "Rectangular pattern...",
            ArrangeCmd::CircularPattern => "Circular pattern...",
            ArrangeCmd::AutoArrange => "Auto arrange",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            ArrangeCmd::MoveCopy => Icon::Move,
            ArrangeCmd::Rotate => Icon::Rotate,
            ArrangeCmd::Mirror => Icon::Mirror,
            ArrangeCmd::Align => Icon::Align,
            ArrangeCmd::Distribute => Icon::DistributeHorizontal,
            ArrangeCmd::RectangularPattern => Icon::RectangularPattern,
            ArrangeCmd::CircularPattern => Icon::CircularPattern,
            ArrangeCmd::AutoArrange => Icon::Arrange,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            ArrangeCmd::MoveCopy => "Move the selection by an exact offset, or leave copies behind",
            ArrangeCmd::Rotate => "Turn the selection by an exact angle",
            ArrangeCmd::Mirror => "Flip the selection left and right, or top and bottom",
            ArrangeCmd::Align => "Line up the selection, or put it on the cutting area",
            ArrangeCmd::Distribute => "Equal gaps between 3 or more objects",
            ArrangeCmd::RectangularPattern => {
                "Copies of one object in columns and rows. Sticker sheet, engage!"
            }
            ArrangeCmd::CircularPattern => "Copies around a circle, like the numbers on a clock",
            ArrangeCmd::AutoArrange => "Pack everything onto the mat, Tetris style",
        }
    }

    /// The panel the command opens, if it has one.
    pub fn panel(self) -> Option<PanelKind> {
        match self {
            ArrangeCmd::MoveCopy => Some(PanelKind::MoveCopy),
            ArrangeCmd::Rotate => Some(PanelKind::Rotate),
            ArrangeCmd::RectangularPattern => Some(PanelKind::RectangularPattern),
            ArrangeCmd::CircularPattern => Some(PanelKind::CircularPattern),
            _ => None,
        }
    }

    /// Why the command cannot run on this selection, or `None` when it
    /// can.
    fn blocked(self, snap: &Snapshot) -> Option<&'static str> {
        let n = snap.selection.len();
        match self {
            ArrangeCmd::AutoArrange => snap.objects.is_empty().then_some("Import something first."),
            ArrangeCmd::RectangularPattern => (n != 1).then_some("Select one object first."),
            ArrangeCmd::Distribute => (n < 3).then_some("Select 3 or more objects first."),
            _ => (n == 0).then_some("Select something first."),
        }
    }
}

/// The pinned commands of the snapshot's preferences, in drop-down order.
pub fn pinned(snap: &Snapshot) -> Vec<ArrangeCmd> {
    ArrangeCmd::ALL
        .into_iter()
        .filter(|c| snap.arrange_pins.iter().any(|p| p == c.id()))
        .collect()
}

// ---------------------------------------------------------------- panels

/// The command panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    MoveCopy,
    Rotate,
    RectangularPattern,
    CircularPattern,
}

impl PanelKind {
    pub fn title(self) -> &'static str {
        match self {
            PanelKind::MoveCopy => "Move / Copy",
            PanelKind::Rotate => "Rotate",
            PanelKind::RectangularPattern => "Rectangular pattern",
            PanelKind::CircularPattern => "Circular pattern",
        }
    }

    fn cmd(self) -> ArrangeCmd {
        match self {
            PanelKind::MoveCopy => ArrangeCmd::MoveCopy,
            PanelKind::Rotate => ArrangeCmd::Rotate,
            PanelKind::RectangularPattern => ArrangeCmd::RectangularPattern,
            PanelKind::CircularPattern => ArrangeCmd::CircularPattern,
        }
    }
}

/// What the Move / Copy panel asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveCopyValues {
    pub dx_mm: f64,
    pub dy_mm: f64,
    /// Leave the originals and add copies.
    pub copy: bool,
    pub count: u32,
}

/// What the Rotate panel asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct RotateValues {
    pub degrees: f64,
    /// Each object about its own centre. Starts as the "Each object"
    /// preference (`Fields::each`); OK stores it back, Cancel does not.
    pub each: bool,
}

/// What the Rectangular pattern panel asks for; the original is the first
/// cell.
#[derive(Debug, Clone, PartialEq)]
pub struct RectValues {
    pub columns: u32,
    pub rows: u32,
    pub gap_x_mm: f64,
    pub gap_y_mm: f64,
    pub linked: bool,
}

/// What the Circular pattern panel asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct CircularValues {
    /// Instances with the original.
    pub count: u32,
    pub total_deg: f64,
    /// Use `center_mm` instead of the centre of the selection.
    pub own_centre: bool,
    pub center_mm: [f64; 2],
    pub rotate_copies: bool,
    pub linked: bool,
}

/// The values of every panel. The last used ones open the next panel.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelValues {
    pub move_copy: MoveCopyValues,
    pub rotate: RotateValues,
    pub rect: RectValues,
    pub circular: CircularValues,
}

impl Default for PanelValues {
    fn default() -> Self {
        Self {
            move_copy: MoveCopyValues {
                dx_mm: 10.0,
                dy_mm: 0.0,
                copy: false,
                count: 1,
            },
            rotate: RotateValues {
                degrees: 90.0,
                each: false,
            },
            rect: RectValues {
                columns: 3,
                rows: 2,
                gap_x_mm: 5.0,
                gap_y_mm: 5.0,
                linked: false,
            },
            circular: CircularValues {
                count: 6,
                total_deg: 360.0,
                own_centre: false,
                center_mm: [0.0, 0.0],
                rotate_copies: true,
                linked: false,
            },
        }
    }
}

/// The open panel: its document, its kind and the values so far.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangePanel {
    pub doc_id: u64,
    pub kind: PanelKind,
    pub values: PanelValues,
}

/// Opens the panel `kind` with the values used last, and the "Each
/// object" switch as it is now.
pub fn open(st: &mut PrepareState, snap: &Snapshot, kind: PanelKind) {
    let mut values = st.panel_last.clone();
    values.rotate.each = st.fields.each;
    st.panel = Some(ArrangePanel {
        doc_id: snap.doc_id,
        kind,
        values,
    });
}

fn selected(snap: &Snapshot) -> Vec<&ObjView> {
    snap.objects
        .iter()
        .filter(|o| snap.selection.contains(&o.id))
        .collect()
}

/// An object's box, or its centre as a point while it has none (as the
/// engine's `placed_bounds_or_center`).
fn bounds(o: &ObjView) -> Rect {
    o.bounds()
        .unwrap_or_else(|| Rect::from_points(o.transform.center_mm, o.transform.center_mm))
}

/// The centre a circular pattern of the selection turns about.
pub fn circular_centre(snap: &Snapshot, v: &CircularValues) -> Option<Point> {
    if v.own_centre {
        return Some(Point::new(v.center_mm[0], v.center_mm[1]));
    }
    let boxes: Vec<Rect> = selected(snap).into_iter().map(bounds).collect();
    placement::pattern_centre(&boxes)
}

fn copies_count(v: &RectValues) -> u32 {
    (v.columns * v.rows).saturating_sub(1)
}

/// Why OK cannot run, or `None` when it can.
fn not_ready(snap: &Snapshot, p: &ArrangePanel) -> Option<String> {
    if let Some(why) = p.kind.cmd().blocked(snap) {
        return Some(why.into());
    }
    let n = snap.selection.len() as u64;
    let too_many = |copies: u64| {
        (copies * n > u64::from(MAX_COPIES))
            .then(|| format!("That is more than {MAX_COPIES} copies. Even Kamino has limits."))
    };
    let v = &p.values;
    match p.kind {
        PanelKind::MoveCopy if v.move_copy.copy => too_many(u64::from(v.move_copy.count)),
        PanelKind::RectangularPattern => match copies_count(&v.rect) {
            0 => Some("One cell is just the original. Add a column or a row.".into()),
            c => too_many(u64::from(c)),
        },
        PanelKind::CircularPattern if v.circular.total_deg == 0.0 => {
            Some("A total angle of 0 stacks every copy on the original. Pick another angle.".into())
        }
        PanelKind::CircularPattern => too_many(u64::from(v.circular.count.saturating_sub(1))),
        _ => None,
    }
}

/// The command OK runs, for the selection. `None` when it cannot run.
pub fn ok_action(snap: &Snapshot, p: &ArrangePanel) -> Option<Action> {
    if not_ready(snap, p).is_some() {
        return None;
    }
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let doc_id = snap.doc_id;
    let v = &p.values;
    Some(match p.kind {
        PanelKind::MoveCopy => {
            let m = &v.move_copy;
            if m.copy {
                Action::run(
                    doc_id,
                    "object.duplicate",
                    json!({ "ids": ids, "offset_mm": [m.dx_mm, m.dy_mm], "count": m.count }),
                )
            } else {
                Action::run(
                    doc_id,
                    "object.move",
                    json!({ "ids": ids, "dx_mm": m.dx_mm, "dy_mm": m.dy_mm }),
                )
            }
        }
        PanelKind::Rotate => ribbon::rotate_action(doc_id, &ids, v.rotate.degrees, v.rotate.each),
        PanelKind::RectangularPattern => {
            let r = &v.rect;
            Action::run(
                doc_id,
                "object.copies",
                json!({
                    "id": ids[0],
                    "count": copies_count(r),
                    "layout": { "grid": { "columns": r.columns } },
                    "gap_x_mm": r.gap_x_mm,
                    "gap_y_mm": r.gap_y_mm,
                    "linked": r.linked,
                }),
            )
        }
        PanelKind::CircularPattern => {
            let c = &v.circular;
            // The centre the preview used, so both agree.
            let centre = circular_centre(snap, c)?;
            Action::run(
                doc_id,
                "object.circular_copies",
                json!({
                    "ids": ids,
                    "count": c.count,
                    "total_deg": c.total_deg,
                    "center_mm": [centre.x, centre.y],
                    "rotate_copies": c.rotate_copies,
                    "linked": c.linked,
                }),
            )
        }
    })
}

/// The would-be objects of the open panel: for each, the object it shows
/// and the affine from its centre-relative shape to mat mm. Built with the
/// same `placement` functions as the commands.
pub fn ghosts(snap: &Snapshot, p: &ArrangePanel) -> Vec<(u64, Affine)> {
    if not_ready(snap, p).is_some() {
        return Vec::new();
    }
    let objs = selected(snap);
    let at = |c: Point| Affine::translate(c.to_vec2());
    let turned = |c: Point, deg: f64| at(c) * Affine::rotate(deg.to_radians());
    let v = &p.values;
    let mut out = Vec::new();
    match p.kind {
        PanelKind::MoveCopy => {
            let m = &v.move_copy;
            let d = Vec2::new(m.dx_mm, m.dy_mm);
            let offsets = if m.copy {
                placement::step_offsets(d, m.count)
            } else {
                vec![d]
            };
            for off in offsets {
                for o in &objs {
                    out.push((o.id, at(o.transform.center_mm + off)));
                }
            }
        }
        PanelKind::Rotate => {
            let centres: Vec<Point> = objs.iter().map(|o| o.transform.center_mm).collect();
            let deg = v.rotate.degrees;
            for (o, c) in objs
                .iter()
                .zip(placement::rotated_centres(&centres, deg, v.rotate.each))
            {
                out.push((o.id, turned(c, deg)));
            }
        }
        PanelKind::RectangularPattern => {
            let r = &v.rect;
            let o = objs[0];
            // No box yet (a big shape still building): no preview rather
            // than a wrong one.
            let Some(b) = o.bounds() else {
                return out;
            };
            for off in
                placement::grid_offsets(b, Some(r.columns), copies_count(r), r.gap_x_mm, r.gap_y_mm)
            {
                out.push((o.id, at(o.transform.center_mm + off)));
            }
        }
        PanelKind::CircularPattern => {
            let c = &v.circular;
            let Some(pivot) = circular_centre(snap, c) else {
                return out;
            };
            for o in &objs {
                for t in placement::circular_copies(
                    o.transform.center_mm,
                    pivot,
                    c.count,
                    c.total_deg,
                    c.rotate_copies,
                ) {
                    out.push((o.id, turned(t.center, t.rotate_deg)));
                }
            }
        }
    }
    out
}

/// A number field in the panel's grid, with its accessible name.
fn number(
    ui: &mut Ui,
    name: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    enabled: bool,
) {
    painted_label(ui, name);
    let r = ui.add_enabled(
        enabled,
        egui::DragValue::new(value)
            .range(range)
            .speed(0.1)
            .max_decimals(3)
            .suffix(suffix),
    );
    set_name(ui, &r, name);
    ui.end_row();
}

/// A whole-number field in the panel's grid.
fn count(
    ui: &mut Ui,
    name: &str,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    enabled: bool,
) {
    painted_label(ui, name);
    let r = ui.add_enabled(enabled, egui::DragValue::new(value).range(range).speed(0.1));
    set_name(ui, &r, name);
    ui.end_row();
}

/// The panel's fields.
fn fields(ui: &mut Ui, p: &mut ArrangePanel, snap: &Snapshot, each_now: bool) {
    let spacing = ui.spacing().item_spacing;
    let v = &mut p.values;
    match p.kind {
        PanelKind::MoveCopy => {
            let m = &mut v.move_copy;
            egui::Grid::new("flatcraft-move-grid")
                .num_columns(2)
                .spacing(spacing)
                .show(ui, |ui| {
                    number(
                        ui,
                        "X offset",
                        &mut m.dx_mm,
                        -10_000.0..=10_000.0,
                        " mm",
                        true,
                    );
                    number(
                        ui,
                        "Y offset",
                        &mut m.dy_mm,
                        -10_000.0..=10_000.0,
                        " mm",
                        true,
                    );
                });
            ui.checkbox(&mut m.copy, "Copy").on_hover_text(
                "Leave the originals where they are and add copies, each one offset further",
            );
            egui::Grid::new("flatcraft-move-count")
                .num_columns(2)
                .spacing(spacing)
                .show(ui, |ui| {
                    count(ui, "Number of copies", &mut m.count, 1..=MAX_COPIES, m.copy);
                });
        }
        PanelKind::Rotate => {
            egui::Grid::new("flatcraft-rotate-grid")
                .num_columns(2)
                .spacing(spacing)
                .show(ui, |ui| {
                    number(
                        ui,
                        "Angle",
                        &mut v.rotate.degrees,
                        -360.0..=360.0,
                        " deg",
                        true,
                    );
                });
            ui.label(RichText::new("Positive turns clockwise.").weak());
            let several = snap.selection.len() > 1;
            let each = &mut v.rotate.each;
            ui.add_enabled_ui(several, |ui| {
                ui.radio_value(each, false, "As one group")
                    .on_hover_text("The selection turns about its common centre, like Voltron");
                ui.radio_value(each, true, "Each about its own centre");
            });
            if *each != each_now {
                ui.label(
                    RichText::new("OK also sets the Each object switch on the ribbon.").weak(),
                );
            }
        }
        PanelKind::RectangularPattern => {
            let r = &mut v.rect;
            egui::Grid::new("flatcraft-rect-grid")
                .num_columns(2)
                .spacing(spacing)
                .show(ui, |ui| {
                    count(ui, "Columns", &mut r.columns, 1..=MAX_COPIES + 1, true);
                    count(ui, "Rows", &mut r.rows, 1..=MAX_COPIES + 1, true);
                    number(ui, "Spacing X", &mut r.gap_x_mm, 0.0..=1000.0, " mm", true);
                    number(ui, "Spacing Y", &mut r.gap_y_mm, 0.0..=1000.0, " mm", true);
                });
            ui.label(
                RichText::new(
                    "Spacing is the gap between neighbours. The original is the top-left cell.",
                )
                .weak(),
            );
            ui.checkbox(&mut r.linked, "Linked copies").on_hover_text(
                "One design, many bodies, like the Clone Army: the copies share the source, the cut options and the scale. Each keeps its own place, rotation and mirror.",
            );
        }
        PanelKind::CircularPattern => {
            let centre = circular_centre(
                snap,
                &CircularValues {
                    own_centre: false,
                    ..v.circular.clone()
                },
            );
            let c = &mut v.circular;
            egui::Grid::new("flatcraft-circular-grid")
                .num_columns(2)
                .spacing(spacing)
                .show(ui, |ui| {
                    count(ui, "Instances", &mut c.count, 2..=MAX_COPIES + 1, true);
                    number(
                        ui,
                        "Total angle",
                        &mut c.total_deg,
                        -360.0..=360.0,
                        " deg",
                        true,
                    );
                });
            ui.label(
                RichText::new(
                    "Instances count the original. Only a full turn (360 deg) shares the copies out evenly; a smaller angle puts the last copy at the end of the arc.",
                )
                .weak(),
            );
            ui.radio_value(&mut c.own_centre, false, "Selection centre");
            ui.radio_value(&mut c.own_centre, true, "Centre at X and Y");
            // The fields follow the selection until they are the centre,
            // so switching starts from where the preview was.
            if !c.own_centre
                && let Some(p) = centre
            {
                c.center_mm = [p.x, p.y];
            }
            let own = c.own_centre;
            egui::Grid::new("flatcraft-circular-centre")
                .num_columns(2)
                .spacing(spacing)
                .show(ui, |ui| {
                    let [x, y] = &mut c.center_mm;
                    number(ui, "Centre X", x, -10_000.0..=10_000.0, " mm", own);
                    number(ui, "Centre Y", y, -10_000.0..=10_000.0, " mm", own);
                });
            ui.checkbox(&mut c.rotate_copies, "Rotate copies")
                .on_hover_text("Each copy turns with the circle, like spokes. Off: they all keep the original's angle.");
            ui.checkbox(&mut c.linked, "Linked copies").on_hover_text(
                "The copies share the source, the cut options and the scale with their original. Each keeps its own place, rotation and mirror.",
            );
        }
    }
}

/// The docked panel: title, close button, fields, Cancel and OK. Closes
/// itself when its document is no longer shown.
pub fn panel_ui(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(p) = st.panel.as_mut() else {
        return;
    };
    let mut close = false;
    let mut ok = None;
    ui.horizontal(|ui| {
        let tex = icons::texture(ui.ctx(), p.kind.cmd().icon(), ICON_SIZE);
        ui.image(egui::load::SizedTexture::new(
            tex.id(),
            egui::Vec2::splat(ICON_SIZE),
        ));
        ui.label(RichText::new(p.kind.title()).font(super::fonts::semibold(15.0)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if super::icon_button(ui, Icon::Close, "Close panel", true).clicked() {
                close = true;
            }
        });
    });
    ui.separator();
    fields(ui, p, snap, st.fields.each);
    ui.add_space(t.spacing);
    let why = not_ready(snap, p);
    if let Some(why) = &why {
        ui.label(RichText::new(why.as_str()).color(t.text_dim));
    } else if matches!(
        p.kind,
        PanelKind::RectangularPattern | PanelKind::CircularPattern
    ) {
        ui.label(
            RichText::new(
                "Copies that land off the mat still arrive. The red pill tells you how many.",
            )
            .color(t.text_dim),
        );
    }
    ui.add_space(t.spacing / 2.0);
    ui.horizontal(|ui| {
        let r = ui.add_enabled(why.is_none(), egui::Button::new("OK"));
        if r.clicked() {
            ok = ok_action(snap, p);
        }
        let cancel =
            egui::Button::new("Cancel").shortcut_text(help::hint(PrepareKey::ClearSelection));
        if ui.add(cancel).clicked() {
            close = true;
        }
    });
    if let Some(a) = ok {
        actions.push(a);
        if p.kind == PanelKind::Rotate && p.values.rotate.each != st.fields.each {
            st.fields.each = p.values.rotate.each;
            actions.push(Action::RememberEach(st.fields.each));
        }
        st.panel_last = p.values.clone();
        close = true;
    }
    if close {
        st.panel = None;
    }
}

/// Closes the panel when its document is no longer the shown one.
pub fn keep_or_close(st: &mut PrepareState, snap: &Snapshot) {
    if st.panel.as_ref().is_some_and(|p| p.doc_id != snap.doc_id) {
        st.panel = None;
    }
}

// ---------------------------------------------------------------- ribbon

/// The items of the Mirror menu.
pub(crate) fn mirror_items(
    ui: &mut Ui,
    doc_id: u64,
    ids: &[u64],
    each: bool,
    actions: &mut Vec<Action>,
) {
    for (text, axis) in [
        ("Mirror horizontally", "horizontal"),
        ("Mirror vertically", "vertical"),
    ] {
        if ui.button(text).clicked() {
            actions.push(ribbon::mirror_action(doc_id, ids, axis, each));
            ui.close();
        }
    }
}

/// The items of the Distribute menu.
pub(crate) fn distribute_items(ui: &mut Ui, doc_id: u64, ids: &[u64], actions: &mut Vec<Action>) {
    let three = ids.len() >= 3;
    let need = if three {
        ""
    } else {
        " Select 3 or more objects first."
    };
    for ((name, axis), hint) in ribbon::DISTRIBUTE.into_iter().zip([
        "Equal gaps from left to right. The outer objects stay put.",
        "Equal gaps from top to bottom. The outer objects stay put.",
    ]) {
        let text = format!("{hint}{need}");
        if ui
            .add_enabled(three, egui::Button::new(name))
            .on_hover_text(&text)
            .on_disabled_hover_text(&text)
            .clicked()
        {
            actions.push(ribbon::distribute_action(doc_id, ids, axis));
            ui.close();
        }
    }
}

fn small_image(ui: &Ui, icon: Icon) -> egui::Image<'static> {
    let size = ICON_SIZE * 0.7;
    let tex = icons::texture(ui.ctx(), icon, size);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(size),
    ))
}

fn image(ui: &Ui, icon: Icon) -> egui::Image<'static> {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ))
}

/// The contents of the submenu of Mirror, Align or Distribute.
fn submenu(
    ui: &mut Ui,
    cmd: ArrangeCmd,
    snap: &Snapshot,
    st: &mut PrepareState,
    actions: &mut Vec<Action>,
) {
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    let doc_id = snap.doc_id;
    match cmd {
        ArrangeCmd::Mirror => mirror_items(ui, doc_id, &ids, st.fields.each, actions),
        ArrangeCmd::Align => {
            ribbon::align_items(ui, &mut st.fields.align_to_area, doc_id, &ids, actions);
        }
        ArrangeCmd::Distribute => distribute_items(ui, doc_id, &ids, actions),
        _ => {}
    }
}

/// Shows the Arrange tab with the drop-down open and, for Mirror, Align
/// and Distribute, their items under their row. Search uses it.
pub fn reveal(st: &mut PrepareState, cmd: ArrangeCmd) {
    st.tab = super::prepare::RibbonTab::Arrange;
    st.arrange_open = true;
    st.arrange_reveal = has_submenu(cmd).then_some(cmd);
}

/// Runs a command that has no submenu: opens its panel or runs it.
fn pick(cmd: ArrangeCmd, snap: &Snapshot, st: &mut PrepareState, actions: &mut Vec<Action>) {
    if let Some(kind) = cmd.panel() {
        open(st, snap, kind);
    } else if cmd == ArrangeCmd::AutoArrange {
        actions.push(Action::run(snap.doc_id, "arrange.auto", json!({})));
    }
}

fn has_submenu(cmd: ArrangeCmd) -> bool {
    matches!(
        cmd,
        ArrangeCmd::Mirror | ArrangeCmd::Align | ArrangeCmd::Distribute
    )
}

/// The "Arrange" drop-down, then the pinned commands as buttons.
pub fn group(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    let config = egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let button = egui::Button::image_and_text(image(ui, Icon::Arrange), "Arrange")
        .right_text(small_image(ui, Icon::ChevronDown));
    let resp = ui
        .add(button)
        .on_hover_text("Every arrange command. Pin your favourites to the ribbon.");
    // Like egui's MenuButton, but search can open it too.
    let open = if std::mem::take(&mut st.arrange_open) {
        Some(egui::SetOpenCommand::Bool(true))
    } else {
        resp.clicked().then_some(egui::SetOpenCommand::Toggle)
    };
    let shown = egui::Popup::menu(&resp)
        .open_memory(open)
        .close_behavior(config.close_behavior)
        .info(
            egui::UiStackInfo::new(egui::UiKind::Menu)
                .with_tag_value(egui::containers::menu::MenuConfig::MENU_CONFIG_TAG, config),
        )
        .show(|ui| dropdown(ui, st, snap, actions));
    if shown.is_none() {
        st.arrange_reveal = None;
    }
    for cmd in pinned(snap) {
        let blocked = cmd.blocked(snap);
        let hint = blocked.map_or_else(
            || cmd.hint().to_string(),
            |why| format!("{} {why}", cmd.hint()),
        );
        let label = cmd.label().trim_end_matches("...");
        if has_submenu(cmd) {
            // Distribute's items say themselves what is missing.
            let resp = big_button(ui, cmd.icon(), label, !snap.selection.is_empty(), &hint);
            egui::Popup::menu(&resp)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| submenu(ui, cmd, snap, st, actions));
        } else if big_button(ui, cmd.icon(), label, blocked.is_none(), &hint).clicked() {
            pick(cmd, snap, st, actions);
        }
    }
}

/// Height of the ribbon's big buttons, and so of its tools row on every
/// tab.
pub const BIG_HEIGHT: f32 = 48.0;

/// A big ribbon button: a 24 pt icon above a small label. Its accessible
/// name is the label.
fn big_button(ui: &mut Ui, icon: Icon, label: &str, enabled: bool, hint: &str) -> egui::Response {
    const ICON: f32 = 24.0;
    let resp = ui
        .add_enabled_ui(enabled, |ui| {
            let font = egui::TextStyle::Small.resolve(ui.style());
            let galley =
                ui.painter()
                    .layout_no_wrap(label.to_string(), font, egui::Color32::PLACEHOLDER);
            let width = (galley.size().x + 12.0).max(BIG_HEIGHT);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(width, BIG_HEIGHT), egui::Sense::click());
            let on = ui.is_enabled();
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::accesskit::Role::Button, on, label)
            });
            if ui.is_rect_visible(rect) {
                let visuals = ui.style().interact(&resp);
                if resp.hovered() || resp.has_focus() {
                    ui.painter().rect(
                        rect,
                        visuals.corner_radius,
                        visuals.weak_bg_fill,
                        visuals.bg_stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                let tex = icons::texture(ui.ctx(), icon, ICON);
                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.center().x, rect.top() + 4.0 + ICON / 2.0),
                    egui::Vec2::splat(ICON),
                );
                // A disabled Ui fades what its painter draws.
                egui::Image::from_texture(egui::load::SizedTexture::new(
                    tex.id(),
                    egui::Vec2::splat(ICON),
                ))
                .paint_at(ui, icon_rect);
                let at = egui::pos2(
                    rect.center().x - galley.size().x / 2.0,
                    rect.bottom() - 4.0 - galley.size().y,
                );
                ui.painter().galley(at, galley, visuals.text_color());
            }
            resp
        })
        .inner;
    resp.on_hover_text(hint).on_disabled_hover_text(hint)
}

/// The rows of the drop-down. Hovering a row shows its pin toggle.
fn dropdown(ui: &mut Ui, st: &mut PrepareState, snap: &Snapshot, actions: &mut Vec<Action>) {
    ui.set_min_width(250.0);
    let pins = pinned(snap);
    for cmd in ArrangeCmd::ALL {
        let row_id = ui.make_persistent_id(("flatcraft-arrange-row", cmd.id()));
        let last: Option<egui::Rect> = ui.data(|d| d.get_temp(row_id));
        let hovered = last.is_some_and(|r| ui.rect_contains_pointer(r));
        let is_pinned = pins.contains(&cmd);
        let blocked = cmd.blocked(snap);
        let hint = blocked.map_or_else(
            || cmd.hint().to_string(),
            |why| format!("{} {why}", cmd.hint()),
        );
        let name = format!("{} (Arrange)", cmd.label());
        let row = ui.horizontal(|ui| {
            let main_width = ui.available_width() - ICON_SIZE - 3.0 * ui.spacing().item_spacing.x;
            let resp = if has_submenu(cmd) {
                let enabled = !snap.selection.is_empty();
                ui.add_enabled_ui(enabled, |ui| {
                    ui.set_width(main_width);
                    ui.menu_image_text_button(image(ui, cmd.icon()), cmd.label(), |ui| {
                        submenu(ui, cmd, snap, st, actions);
                    })
                    .response
                })
                .inner
            } else {
                let r = ui.add_enabled(
                    blocked.is_none(),
                    egui::Button::image_and_text(image(ui, cmd.icon()), cmd.label())
                        .min_size(egui::vec2(main_width, 0.0)),
                );
                if r.clicked() {
                    pick(cmd, snap, st, actions);
                    ui.close();
                }
                r
            };
            set_name(ui, &resp, &name);
            resp.on_hover_text(&hint).on_disabled_hover_text(&hint);
            if hovered {
                let (icon, what) = if is_pinned {
                    (Icon::PinOff, "Unpin")
                } else {
                    (Icon::Pin, "Pin")
                };
                let label = format!("{what} {}", cmd.label().trim_end_matches("..."));
                let tip = if is_pinned {
                    "Take it off the ribbon"
                } else {
                    "Show it on the ribbon"
                };
                if super::icon_button_hint(ui, icon, &label, true, tip).clicked() {
                    actions.push(Action::SetPin {
                        id: cmd.id(),
                        pinned: !is_pinned,
                    });
                }
            } else if is_pinned {
                let tex = icons::texture(ui.ctx(), Icon::Pin, ICON_SIZE);
                ui.add(
                    egui::Image::from_texture(egui::load::SizedTexture::new(
                        tex.id(),
                        egui::Vec2::splat(ICON_SIZE),
                    ))
                    .tint(egui::Color32::from_white_alpha(110)),
                );
            }
        });
        ui.data_mut(|d| d.insert_temp(row_id, row.response.rect));
        if st.arrange_reveal == Some(cmd) {
            // Found by search: its items right here, no hover needed.
            ui.add_enabled_ui(!snap.selection.is_empty(), |ui| {
                ui.indent(("flatcraft-arrange-reveal", cmd.id()), |ui| {
                    submenu(ui, cmd, snap, st, actions);
                });
            });
        }
    }
    ui.separator();
    ui.label(
        RichText::new("Pinned commands show on the ribbon. Hover a row to pin or unpin it.")
            .weak()
            .small(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_defaults_are_known() {
        for c in ArrangeCmd::ALL {
            assert_eq!(ArrangeCmd::from_id(c.id()), Some(c));
            assert!(!c.label().contains('\u{2014}') && !c.hint().contains('\u{2014}'));
        }
        for id in flatcraft_engine::prefs::default_arrange_pins() {
            assert!(ArrangeCmd::from_id(&id).is_some(), "{id}");
        }
        assert_eq!(ArrangeCmd::from_id("nope"), None);
    }
}
