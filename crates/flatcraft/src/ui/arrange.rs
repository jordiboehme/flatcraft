//! The tool cards of the Arrange tools (Move / Copy, Rotate, the patterns
//! and Auto arrange): their fields, built from the commands' parameters
//! (see `inspector::tool_card`), and their ghost preview. A card opens on
//! top of the inspector.
//!
//! A card works on the selection it was opened for; another selection
//! cancels it. While it is open, the canvas
//! draws the would-be result faintly ([`ghosts`]), built with the same
//! pure functions of `flatcraft_engine::placement` that the commands use,
//! so the preview and the result cannot differ. OK runs one command (one
//! undo step); Cancel, the close button and Escape change nothing.

use std::collections::BTreeSet;

use egui::Ui;
use flatcraft_core::document::ObjectId;
use flatcraft_engine::commands::meta::Unit;
use flatcraft_engine::commands::object::MAX_COPIES;
use flatcraft_engine::placement;
use kurbo::{Affine, Point, Rect, Vec2};
use serde_json::json;

use super::inspector::tool_card::{
    self, CardField, Outcome, Slot, ToolCard, WindowField, WindowKind,
};
use super::prepare::{ObjView, PrepareState, Snapshot};
use super::theme::Tokens;
use super::tools::{self, Act, Tool};
use super::{Action, ribbon};

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

/// The open card: its document, the selection it was opened for, its
/// kind and the values so far.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangePanel {
    pub doc_id: u64,
    /// The selection when it opened; another selection cancels it.
    pub selection: BTreeSet<u64>,
    pub kind: PanelKind,
    pub values: PanelValues,
}

/// Opens the card `kind` with the values used last, and the fields with
/// a Preferences key as they are stored now.
pub fn open(st: &mut PrepareState, snap: &Snapshot, kind: PanelKind) {
    let mut values = st.panel_last.clone();
    load_prefs(&mut values, st, snap);
    st.panel = Some(ArrangePanel {
        doc_id: snap.doc_id,
        selection: snap.selection.clone(),
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
    // As the engine: hidden objects do not count unless all are hidden.
    let objs = selected(snap);
    let any_shown = objs.iter().any(|o| o.visible);
    let boxes: Vec<Rect> = objs
        .into_iter()
        .filter(|o| o.visible || !any_shown)
        .map(bounds)
        .reduce(|a, b| a.union(b))
        .into_iter()
        .collect();
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
        // What packs as one item, as `arrange.auto` has it.
        let order: Vec<ObjectId> = snap.objects.iter().map(|o| ObjectId(o.id)).collect();
        let picked: Vec<ObjectId> = snap.selection.iter().map(|i| ObjectId(*i)).collect();
        let (keys, units) = placement::auto_keys(&snap.tree, &order, &picked);
        let leaves: Vec<(u64, placement::AutoObject<u64>)> = snap
            .objects
            .iter()
            .zip(keys)
            .map(|(o, key)| {
                let item = placement::AutoObject {
                    key: o.id,
                    visible: o.visible,
                    bounds: o.arrange_bounds(),
                };
                (key.0, item)
            })
            .collect();
        let chosen: Vec<u64> = units.into_iter().map(|u| u.0).collect();
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
    // A whole group counts as one object, as it packs as one.
    let shown = snap
        .units()
        .into_iter()
        .filter(|u| {
            snap.leaves_of(*u)
                .iter()
                .any(|l| snap.object(*l).is_some_and(|o| o.visible))
        })
        .count();
    match shown {
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

// ---------------------------------------------------------------- cards

/// A [`Slot`] binding to one place in the values.
macro_rules! bind {
    ($kind:ident, $($path:tt)+) => {{
        fn bind(v: &mut PanelValues) -> Slot<'_> {
            Slot::$kind(&mut v.$($path)+)
        }
        bind
    }};
}

/// Why the number of copies does not apply.
fn needs_copy(v: &PanelValues, _: &Snapshot) -> Option<&'static str> {
    (!v.move_copy.copy).then_some("Turn on Copy first. Without it the selection only moves.")
}

/// Why the choice of group or each does not apply.
fn needs_two(_: &PanelValues, snap: &Snapshot) -> Option<&'static str> {
    (snap.units().len() < 2)
        .then_some("Select 2 or more objects first. One object always turns about its own centre.")
}

/// Why the centre fields do not apply.
fn needs_own_centre(v: &PanelValues, _: &Snapshot) -> Option<&'static str> {
    (!v.circular.own_centre).then_some(
        "Choose Centre at X and Y first. Until then the circle turns about the centre of the selection.",
    )
}

const COPY: WindowField = WindowField {
    owner: "move_copy",
    id: "copy",
    label: "Copy",
    unit: Unit::None,
    kind: WindowKind::Switch,
};

const ROWS: WindowField = WindowField {
    owner: "pattern.rectangular",
    id: "rows",
    label: "Rows",
    unit: Unit::Count,
    kind: WindowKind::Count {
        min: 1,
        max: MAX_COPIES + 1,
    },
};

const CENTRE: WindowField = WindowField {
    owner: "pattern.circular",
    id: "centre",
    label: "Centre",
    unit: Unit::None,
    kind: WindowKind::Choice(&["Selection centre", "Centre at X and Y"]),
};

const MOVE_COPY_FIELDS: &[CardField] = &[
    CardField::param("object.move", "dx_mm", bind!(Number, move_copy.dx_mm)),
    CardField::param("object.move", "dy_mm", bind!(Number, move_copy.dy_mm)),
    CardField::window(COPY, bind!(Switch, move_copy.copy)),
    CardField::param("object.duplicate", "count", bind!(Count, move_copy.count)).dimmed(needs_copy),
];

const ROTATE_FIELDS: &[CardField] = &[
    CardField::param("object.rotate", "degrees", bind!(Number, rotate.degrees)),
    CardField::param("object.rotate", "about", bind!(Pair, rotate.each))
        .dimmed(needs_two)
        .pref(PREF_EACH),
];

const RECT_FIELDS: &[CardField] = &[
    CardField::param(
        "object.copies",
        "layout.grid.columns",
        bind!(Count, rect.columns),
    ),
    CardField::window(ROWS, bind!(Count, rect.rows)),
    CardField::param("object.copies", "gap_x_mm", bind!(Number, rect.gap_x_mm)),
    CardField::param("object.copies", "gap_y_mm", bind!(Number, rect.gap_y_mm)),
    CardField::param("object.copies", "linked", bind!(Switch, rect.linked)),
];

const CIRCULAR_FIELDS: &[CardField] = &[
    CardField::param(
        "object.circular_copies",
        "count",
        bind!(Count, circular.count),
    ),
    CardField::param(
        "object.circular_copies",
        "total_deg",
        bind!(Number, circular.total_deg),
    ),
    CardField::window(CENTRE, bind!(Pair, circular.own_centre)).special("pattern_centre"),
    CardField::param(
        "object.circular_copies",
        "center_mm[0]",
        bind!(Number, circular.center_mm[0]),
    )
    .dimmed(needs_own_centre),
    CardField::param(
        "object.circular_copies",
        "center_mm[1]",
        bind!(Number, circular.center_mm[1]),
    )
    .dimmed(needs_own_centre),
    CardField::param(
        "object.circular_copies",
        "rotate_copies",
        bind!(Switch, circular.rotate_copies),
    ),
    CardField::param(
        "object.circular_copies",
        "linked",
        bind!(Switch, circular.linked),
    ),
];

const AUTO_FIELDS: &[CardField] =
    &[
        CardField::param("arrange.auto", "spacing_mm", bind!(Number, spacing_mm))
            .pref(PREF_SPACING),
    ];

/// The Preferences key of the "Each object" switch (`rotate_each`).
pub const PREF_EACH: &str = "rotate_each";
/// The Preferences key of the Auto arrange spacing.
pub const PREF_SPACING: &str = "auto_arrange_spacing_mm";

/// Every card, for the tests.
pub const KINDS: [PanelKind; 5] = [
    PanelKind::MoveCopy,
    PanelKind::Rotate,
    PanelKind::RectangularPattern,
    PanelKind::CircularPattern,
    PanelKind::AutoArrange,
];

impl PanelKind {
    /// The fields of its card.
    pub fn fields(self) -> &'static [CardField] {
        match self {
            PanelKind::MoveCopy => MOVE_COPY_FIELDS,
            PanelKind::Rotate => ROTATE_FIELDS,
            PanelKind::RectangularPattern => RECT_FIELDS,
            PanelKind::CircularPattern => CIRCULAR_FIELDS,
            PanelKind::AutoArrange => AUTO_FIELDS,
        }
    }

    /// The dimmed lines under the card's title.
    pub fn description(self) -> &'static str {
        match self {
            PanelKind::MoveCopy => "Moves the selection by exact amounts, or adds copies in a row.",
            PanelKind::Rotate => "Turns the selection by an exact angle. Positive turns clockwise.",
            PanelKind::RectangularPattern => {
                "Spacing is the gap between neighbours. The original is the top-left cell."
            }
            PanelKind::CircularPattern => {
                "Instances count the original. Only a full turn (360 deg) shares the copies out evenly."
            }
            PanelKind::AutoArrange => {
                "Hidden objects stay where they are. Objects that do not fit stay too."
            }
        }
    }

    /// The title's hover card.
    pub fn title_card(self) -> &'static str {
        match self {
            PanelKind::AutoArrange => AUTO_ARRANGE_CARD,
            _ => self.tool().description,
        }
    }
}

/// The values the fields with a Preferences key open with.
fn load_prefs(values: &mut PanelValues, st: &PrepareState, snap: &Snapshot) {
    for f in KINDS.iter().flat_map(|k| k.fields()) {
        match f.pref {
            Some(PREF_EACH) => values.rotate.each = st.fields.each,
            Some(PREF_SPACING) => values.spacing_mm = snap.auto_spacing_mm,
            _ => {}
        }
    }
}

/// OK stores the fields with a Preferences key that changed.
fn store_prefs(
    p: &ArrangePanel,
    st: &mut PrepareState,
    snap: &Snapshot,
    actions: &mut Vec<Action>,
) {
    for f in p.kind.fields() {
        match f.pref {
            Some(PREF_EACH) if p.values.rotate.each != st.fields.each => {
                st.fields.each = p.values.rotate.each;
                actions.push(Action::RememberEach(st.fields.each));
            }
            Some(PREF_SPACING) if p.values.spacing_mm != snap.auto_spacing_mm => {
                actions.push(Action::RememberSpacing(p.values.spacing_mm));
            }
            _ => {}
        }
    }
}

/// The special widget "pattern_centre": the choice of the circle's centre.
/// While it is the selection's centre, the Centre X and Y fields follow
/// the selection, so switching starts from where the preview was.
pub fn centre_choice(
    ui: &mut Ui,
    spec: &tool_card::FieldSpec,
    v: &mut PanelValues,
    snap: &Snapshot,
    enabled: bool,
) -> egui::Response {
    let c = &mut v.circular;
    if !c.own_centre
        && let Some(p) = circular_centre(
            snap,
            &CircularValues {
                own_centre: false,
                ..c.clone()
            },
        )
    {
        c.center_mm = [p.x, p.y];
    }
    tool_card::pair_widget(ui, spec, &mut c.own_centre, enabled)
}

/// The note over the buttons: why OK cannot run, what does not fit, or
/// where the copies of a pattern land.
fn note(st: &mut PrepareState, snap: &Snapshot, p: &ArrangePanel) -> Option<String> {
    if let Some(why) = not_ready(snap, p) {
        return Some(why);
    }
    if let Some(n) = st
        .auto_plan
        .get(snap, p)
        .map(|plan| plan.did_not_fit.len())
        .filter(|n| *n > 0)
    {
        return Some(did_not_fit_note(n));
    }
    matches!(
        p.kind,
        PanelKind::RectangularPattern | PanelKind::CircularPattern
    )
    .then(|| "Copies that land off the mat still arrive. The red pill tells you how many.".into())
}

/// The open tool's card on top of the inspector: title, close button,
/// fields, Cancel and OK. OK runs one command (one undo step) and closes
/// it; Cancel and the close button change nothing. Returns the card's
/// rect, `None` when no card is open.
pub fn card_ui(
    ui: &mut Ui,
    st: &mut PrepareState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) -> Option<egui::Rect> {
    let p = st.panel.clone()?;
    let title = panel_title(snap, &p);
    let note = note(st, snap, &p);
    let why = not_ready(snap, &p);
    let mut values = p.values.clone();
    let card = ToolCard {
        icon: p.kind.tool().icon,
        title: &title,
        title_card: p.kind.title_card(),
        description: p.kind.description(),
        fields: p.kind.fields(),
        note: note.as_deref(),
        ok_why: why.as_deref(),
    };
    let (outcome, rect) = tool_card::show(ui, &card, &mut values, snap, t);
    let Some(open) = st.panel.as_mut() else {
        return Some(rect);
    };
    open.values = values;
    match outcome {
        Some(Outcome::Ok) => {
            let p = open.clone();
            if let Some(a) = ok_action(snap, &p) {
                actions.push(a);
                store_prefs(&p, st, snap, actions);
                st.panel_last = p.values.clone();
                st.panel = None;
            }
        }
        Some(Outcome::Cancel) => st.panel = None,
        None => {}
    }
    Some(rect)
}

/// Cancels the card when its document is no longer the shown one, or the
/// selection changed since it opened: a card never runs on objects it was
/// not opened for.
pub fn keep_or_close(st: &mut PrepareState, snap: &Snapshot) {
    if st
        .panel
        .as_ref()
        .is_some_and(|p| p.doc_id != snap.doc_id || p.selection != snap.selection)
    {
        st.panel = None;
    }
}
