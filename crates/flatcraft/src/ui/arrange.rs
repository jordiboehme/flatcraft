//! The Arrange command panels docked on the right (Move / Copy, Rotate,
//! the patterns and Auto arrange) with their ghost preview, and the big
//! button the ribbon pins them with.
//!
//! A panel works on the current selection. While it is open, the canvas
//! draws the would-be result faintly ([`ghosts`]), built with the same
//! pure functions of `flatcraft_engine::placement` that the commands use,
//! so the preview and the result cannot differ. OK runs one command (one
//! undo step); Cancel, the close button and Escape change nothing.

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
use super::tools::{self, Act, Setting, Tool};
use super::{Action, ICON_SIZE, ribbon};

// ---------------------------------------------------------------- panels

/// The command panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    MoveCopy,
    Rotate,
    RectangularPattern,
    CircularPattern,
    AutoArrange,
}

impl PanelKind {
    /// The action of the ribbon table that opens the panel.
    pub fn tool(self) -> &'static Tool {
        tools::by_act(match self {
            PanelKind::MoveCopy => Act::MoveCopy,
            PanelKind::Rotate => Act::Rotate,
            PanelKind::RectangularPattern => Act::RectangularPattern,
            PanelKind::CircularPattern => Act::CircularPattern,
            PanelKind::AutoArrange => Act::AutoArrangePanel,
        })
    }

    pub fn title(self) -> &'static str {
        self.tool().title.trim_end_matches("...")
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
    /// The gap of Auto arrange, in mm. Opens as the preference
    /// (`auto_arrange_spacing_mm`); OK stores it back.
    pub spacing_mm: f64,
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
            spacing_mm: flatcraft_engine::prefs::DEFAULT_AUTO_SPACING_MM,
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
    values.spacing_mm = snap.auto_spacing_mm;
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
    if let Some(why) = p.kind.tool().blocked(snap) {
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
        PanelKind::AutoArrange if snap.area.is_none() => Some(
            "There is no cutting area to fill. Pick a plotter and a mat in the Settings panel."
                .into(),
        ),
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
        PanelKind::AutoArrange => Action::run(
            doc_id,
            "arrange.auto",
            json!({ "spacing_mm": v.spacing_mm }),
        ),
    })
}

/// Where Auto arrange puts the visible objects that have a box, as
/// `arrange.auto` does it (the same `placement::shelf_pack`), with the
/// move of each.
pub fn auto_arrange_moves(snap: &Snapshot, spacing_mm: f64) -> Vec<(u64, Vec2)> {
    let Some(area) = snap.area else {
        return Vec::new();
    };
    let boxes: Vec<(u64, Rect)> = snap
        .objects
        .iter()
        .filter(|o| o.visible)
        .filter_map(|o| o.bounds().map(|b| (o.id, b)))
        .collect();
    placement::shelf_pack(&boxes, area, spacing_mm).0
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
        PanelKind::AutoArrange => {
            for (id, d) in auto_arrange_moves(snap, v.spacing_mm) {
                if let Some(o) = snap.object(id) {
                    out.push((id, at(o.transform.center_mm + d)));
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
            .min_decimals(2)
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
        PanelKind::AutoArrange => {
            egui::Grid::new("flatcraft-auto-grid")
                .num_columns(2)
                .spacing(spacing)
                .show(ui, |ui| {
                    number(ui, "Spacing", &mut v.spacing_mm, 0.0..=100.0, " mm", true);
                });
            ui.label(RichText::new(Setting::AutoArrangeSpacing.card()).weak());
            ui.label(
                RichText::new(
                    "Hidden objects stay where they are. Objects too big for the cutting area stay too.",
                )
                .weak(),
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
        let tex = icons::texture(ui.ctx(), p.kind.tool().icon, ICON_SIZE);
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
        if p.kind == PanelKind::AutoArrange && p.values.spacing_mm != snap.auto_spacing_mm {
            actions.push(Action::RememberSpacing(p.values.spacing_mm));
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
