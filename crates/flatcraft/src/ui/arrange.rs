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
use flatcraft_core::numexpr::{Field, Kind};
use flatcraft_engine::commands::object::MAX_COPIES;
use flatcraft_engine::placement;
use kurbo::{Affine, Point, Rect, Vec2};
use serde_json::json;

use super::calc::{self, Number};
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
    // Copies are counted per unit: a whole group is copied as one.
    let n = snap.units().len() as u64;
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
                    // One object, or one whole group.
                    "id": snap.units().first().copied().unwrap_or(ids[0]),
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
        // The selection the preview used (empty: every visible object).
        PanelKind::AutoArrange => Action::run(
            doc_id,
            "arrange.auto",
            json!({ "ids": ids, "spacing_mm": v.spacing_mm }),
        ),
    })
}

/// What Auto arrange does with the selection (or, with nothing selected,
/// with every visible object), as `arrange.auto` does it: the same
/// `placement::auto_arrange` on the same boxes.
pub fn auto_arrange_plan(snap: &Snapshot, spacing_mm: f64) -> Option<placement::AutoPlan<u64>> {
    let key = PlanKey::of(snap, spacing_mm)?;
    Some(key.plan())
}

/// Everything an Auto arrange plan depends on.
#[derive(Debug, Clone, PartialEq)]
struct PlanKey {
    objects: Vec<placement::AutoObject<u64>>,
    chosen: Vec<u64>,
    area: Rect,
    spacing_mm: f64,
}

impl PlanKey {
    fn of(snap: &Snapshot, spacing_mm: f64) -> Option<Self> {
        // A top-level group packs as one item, as `arrange.auto` has it.
        let tops = snap.tree.top_of_each();
        let top = |id: u64| {
            tops.get(&flatcraft_core::document::ObjectId(id))
                .map_or(id, |t| t.0)
        };
        let leaves: Vec<(u64, placement::AutoObject<u64>)> = snap
            .objects
            .iter()
            .map(|o| {
                let item = placement::AutoObject {
                    key: o.id,
                    visible: o.visible,
                    bounds: o.arrange_bounds(),
                };
                (top(o.id), item)
            })
            .collect();
        let mut chosen: Vec<u64> = Vec::new();
        for id in &snap.selection {
            let t = top(*id);
            if !chosen.contains(&t) {
                chosen.push(t);
            }
        }
        Some(Self {
            objects: placement::auto_units(&leaves),
            chosen,
            area: snap.area?,
            spacing_mm,
        })
    }

    fn plan(&self) -> placement::AutoPlan<u64> {
        placement::auto_arrange(&self.objects, &self.chosen, self.area, self.spacing_mm)
    }
}

/// The last Auto arrange plan and what it was made from, so the canvas
/// and the panel pack once, and again only when something changed (an
/// object, the selection, the area or the spacing), not every frame.
#[derive(Debug, Clone, Default)]
pub struct AutoPlanCache {
    key: Option<PlanKey>,
    plan: Option<placement::AutoPlan<u64>>,
}

impl AutoPlanCache {
    /// The plan of the open panel `p`, `None` when it is not Auto
    /// arrange or cannot run.
    pub fn get(&mut self, snap: &Snapshot, p: &ArrangePanel) -> Option<&placement::AutoPlan<u64>> {
        if p.kind != PanelKind::AutoArrange || not_ready(snap, p).is_some() {
            return None;
        }
        let key = PlanKey::of(snap, p.values.spacing_mm)?;
        if self.key.as_ref() != Some(&key) || self.plan.is_none() {
            self.plan = Some(key.plan());
            self.key = Some(key);
        }
        self.plan.as_ref()
    }
}

/// What the Auto arrange panel packs, for its title: "3 selected
/// objects", or "all objects" with nothing selected.
/// Hidden selected objects are not counted: they stay where they are.
pub fn auto_arrange_what(snap: &Snapshot) -> String {
    if snap.selection.is_empty() {
        return "all objects".into();
    }
    match snap.selected().filter(|o| o.visible).count() {
        1 => "1 selected object".into(),
        n => format!("{n} selected objects"),
    }
}

/// The panel's title: its tool's name, and for Auto arrange what it packs
/// ("Auto arrange 3 selected objects").
pub fn panel_title(snap: &Snapshot, p: &ArrangePanel) -> String {
    match p.kind {
        PanelKind::AutoArrange => format!("{} {}", p.kind.title(), auto_arrange_what(snap)),
        _ => p.kind.title().to_string(),
    }
}

/// What the Auto arrange panel says when some objects do not fit.
pub fn did_not_fit_note(n: usize) -> String {
    if n == 1 {
        "1 object does not fit. It stays where it is, marked in red.".into()
    } else {
        format!("{n} objects do not fit. They stay where they are, marked in red.")
    }
}

/// The hover card of the Auto arrange panel's title.
pub const AUTO_ARRANGE_CARD: &str = "Select the objects you want packed. Everything else stays where you put it, and the packed objects fit around it.";

/// The objects the open panel's preview draws as fixed: for Auto arrange,
/// the visible objects it packs around. Empty for the other panels.
pub fn fixed(snap: &Snapshot, p: &ArrangePanel) -> Vec<u64> {
    AutoPlanCache::default()
        .get(snap, p)
        .map_or_else(Vec::new, |plan| plan.fixed.clone())
}

/// The would-be objects of the open panel: for each, the object it shows
/// and the affine from its centre-relative shape to mat mm. Built with the
/// same `placement` functions as the commands.
pub fn ghosts(snap: &Snapshot, p: &ArrangePanel) -> Vec<(u64, Affine)> {
    ghosts_with(snap, p, AutoPlanCache::default().get(snap, p))
}

/// [`ghosts`] with the Auto arrange plan already made (see
/// [`AutoPlanCache`]); the other panels ignore it.
pub fn ghosts_with(
    snap: &Snapshot,
    p: &ArrangePanel,
    auto: Option<&placement::AutoPlan<u64>>,
) -> Vec<(u64, Affine)> {
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
            // A whole group turns as one piece, as `object.rotate` has it.
            let units = snap.units();
            let members: Vec<Vec<&ObjView>> = units
                .iter()
                .map(|u| {
                    snap.leaves_of(*u)
                        .iter()
                        .filter_map(|id| snap.object(*id))
                        .collect()
                })
                .collect();
            let turn_units: Vec<placement::TurnUnit> = units
                .iter()
                .zip(&members)
                .filter_map(|(u, m)| {
                    Some(placement::TurnUnit {
                        centre: snap.unit_centre(*u)?,
                        members: m.iter().map(|o| o.transform.center_mm).collect(),
                    })
                })
                .collect();
            if turn_units.len() != units.len() {
                return out;
            }
            let deg = v.rotate.degrees;
            let moved = placement::rotated_units(&turn_units, deg, v.rotate.each);
            for (m, centres) in members.iter().zip(moved) {
                for (o, c) in m.iter().zip(centres) {
                    out.push((o.id, turned(c, deg)));
                }
            }
        }
        PanelKind::RectangularPattern => {
            let r = &v.rect;
            // One object, or a whole group laid out by its common box.
            let ids: Vec<u64> = objs.iter().map(|o| o.id).collect();
            // No box yet (a big shape still building): no preview rather
            // than a wrong one.
            if objs.iter().any(|o| o.bounds().is_none()) {
                return out;
            }
            let Some(b) = snap.box_of(&ids) else {
                return out;
            };
            for off in
                placement::grid_offsets(b, Some(r.columns), copies_count(r), r.gap_x_mm, r.gap_y_mm)
            {
                for o in &objs {
                    out.push((o.id, at(o.transform.center_mm + off)));
                }
            }
        }
        PanelKind::CircularPattern => {
            let c = &v.circular;
            let Some(pivot) = circular_centre(snap, c) else {
                return out;
            };
            for u in snap.units() {
                let Some(centre) = snap.unit_centre(u) else {
                    continue;
                };
                let members: Vec<&ObjView> = snap
                    .leaves_of(u)
                    .iter()
                    .filter_map(|id| snap.object(*id))
                    .collect();
                for t in
                    placement::circular_copies(centre, pivot, c.count, c.total_deg, c.rotate_copies)
                {
                    for o in &members {
                        let to = placement::carried(o.transform.center_mm, centre, t);
                        out.push((o.id, turned(to, t.rotate_deg)));
                    }
                }
            }
        }
        PanelKind::AutoArrange => {
            for (id, d) in auto.map_or(&[][..], |p| &p.moves[..]).iter().copied() {
                // A packed group moves all its members.
                for leaf in snap.leaves_of(id) {
                    if let Some(o) = snap.object(leaf) {
                        out.push((leaf, at(o.transform.center_mm + d)));
                    }
                }
            }
        }
    }
    out
}

/// A number field in the panel's grid, with its accessible name and a
/// hover card. It takes a calculation (see [`calc`]); the unit follows
/// `suffix`.
fn number(
    ui: &mut Ui,
    name: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    enabled: bool,
) {
    painted_label(ui, name);
    let kind = match suffix.trim() {
        "mm" => Kind::Length,
        "deg" => Kind::Angle,
        _ => Kind::Plain,
    };
    let (lo, hi) = (*range.start(), *range.end());
    let spec = Number::new(Field::new(kind).range(lo, hi))
        .decimals(2, 3)
        .unit(suffix.trim_start());
    let (r, _) = calc::number(ui, ("flatcraft-arrange", name), enabled, value, &spec);
    set_name(ui, &r, name);
    let text = format!("From {lo}{suffix} to {hi}{suffix}. {}", calc::hint(kind));
    // Only the centre fields are ever dimmed.
    let why = (!enabled).then_some("Choose Centre at X and Y first.");
    super::groups::card(&r, name, &text, why, false);
    ui.end_row();
}

/// A whole-number field in the panel's grid, with its hover card.
fn count(
    ui: &mut Ui,
    name: &str,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
    enabled: bool,
) {
    painted_label(ui, name);
    let (lo, hi) = (*range.start(), *range.end());
    let spec = Number::new(Field::plain().range(f64::from(lo), f64::from(hi)));
    let (r, _) = calc::number(ui, ("flatcraft-arrange", name), enabled, value, &spec);
    set_name(ui, &r, name);
    let text = format!(
        "A whole number from {lo} to {hi}. {}",
        calc::hint(Kind::Plain)
    );
    let why = (!enabled).then_some("Turn on Copy first.");
    super::groups::card(&r, name, &text, why, false);
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
                    number(
                        ui,
                        "Spacing",
                        &mut v.spacing_mm,
                        flatcraft_engine::prefs::AUTO_SPACING_RANGE_MM,
                        " mm",
                        true,
                    );
                });
            ui.label(RichText::new(Setting::AutoArrangeSpacing.card()).weak());
            ui.label(RichText::new(AUTO_ARRANGE_CARD).weak());
            ui.label(
                RichText::new(
                    "Hidden objects stay where they are. Objects that do not fit stay too.",
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
    ui.horizontal_top(|ui| {
        // The close button first, from the right, so a long title
        // ("Auto arrange 12 selected objects") wraps before it.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if super::icon_button(ui, Icon::Close, "Close panel", true).clicked() {
                close = true;
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                let tex = icons::texture(ui.ctx(), p.kind.tool().icon, ICON_SIZE);
                ui.image(egui::load::SizedTexture::new(
                    tex.id(),
                    egui::Vec2::splat(ICON_SIZE),
                ));
                let title = panel_title(snap, p);
                let r = ui.add(
                    egui::Label::new(RichText::new(&title).font(super::fonts::semibold(15.0)))
                        .wrap(),
                );
                if p.kind == PanelKind::AutoArrange {
                    super::groups::card(&r, &title, AUTO_ARRANGE_CARD, None, false);
                }
            });
        });
    });
    ui.separator();
    fields(ui, p, snap, st.fields.each);
    ui.add_space(t.spacing);
    let why = not_ready(snap, p);
    if let Some(why) = &why {
        ui.label(RichText::new(why.as_str()).color(t.text_dim));
    } else if let Some(n) = st
        .auto_plan
        .get(snap, p)
        .map(|plan| plan.did_not_fit.len())
        .filter(|n| *n > 0)
    {
        ui.label(RichText::new(did_not_fit_note(n)).color(t.text_dim));
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
        if calc::click(ui, &r) {
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
