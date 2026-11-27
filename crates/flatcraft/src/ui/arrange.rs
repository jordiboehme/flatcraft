//! The tool cards of the Arrange tools (Move / Copy, Rotate, the patterns
//! and Auto arrange): which commands each runs, their special widgets and
//! their ghost preview. The fields come from the commands' parameters
//! (see `inspector::tool_card`). A card opens on top of the inspector.
//!
//! A card works on the selection it was opened for; another selection
//! cancels it. While it is open, the canvas draws the would-be result
//! faintly ([`ghosts`]), built from the very params OK sends, with the same
//! pure functions of `flatcraft_engine::placement` that the commands use,
//! so the preview and the result cannot differ. OK runs one command (one
//! undo step); Cancel, the close button and Escape change nothing.

use std::collections::BTreeSet;

use egui::Ui;
use flatcraft_core::document::ObjectId;
use flatcraft_engine::commands::meta;
use flatcraft_engine::commands::object::MAX_COPIES;
use flatcraft_engine::placement;
use kurbo::{Affine, Point, Rect, Vec2};
use serde_json::Value;

use super::Action;
use super::inspector::tool_card::{
    self, CardState, FieldKind, FieldSpec, Mode, Outcome, Special, ToolCard, Values,
};
use super::prepare::{ObjView, PrepareState, Snapshot};
use super::theme::Tokens;
use super::tools::{self, Act, Tool};

// ---------------------------------------------------------------- cards

/// The tool cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    MoveCopy,
    Rotate,
    RectangularPattern,
    CircularPattern,
    AutoArrange,
}

/// Every card.
pub const KINDS: [PanelKind; 5] = [
    PanelKind::MoveCopy,
    PanelKind::Rotate,
    PanelKind::RectangularPattern,
    PanelKind::CircularPattern,
    PanelKind::AutoArrange,
];

/// The switch of Move / Copy: on, it runs `object.duplicate`.
pub const COPY: Mode = Mode {
    owner: "move_copy",
    id: "copy",
    label: "Copy",
};

impl PanelKind {
    /// The action of the ribbon table that opens the card.
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

    /// The commands the card runs; Move / Copy switches between two.
    pub fn commands(self) -> &'static [&'static str] {
        match self {
            PanelKind::MoveCopy => &["object.move", "object.duplicate"],
            PanelKind::Rotate => &["object.rotate"],
            PanelKind::RectangularPattern => &["object.copies"],
            PanelKind::CircularPattern => &["object.circular_copies"],
            PanelKind::AutoArrange => &["arrange.auto"],
        }
    }

    /// The switch between its two commands, if it has two.
    pub fn mode(self) -> Option<Mode> {
        (self == PanelKind::MoveCopy).then_some(COPY)
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

/// The open card: its document, the selection it was opened for, its
/// kind and its values.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrangePanel {
    pub doc_id: u64,
    /// The selection when it opened; another selection cancels it.
    pub selection: BTreeSet<u64>,
    pub kind: PanelKind,
    pub card: CardState,
}

impl ArrangePanel {
    /// The command OK runs and its params, the specials filled in: what
    /// the preview shows and OK sends.
    pub fn params(&self, snap: &Snapshot) -> (&'static str, Value) {
        let mut card = self.card.clone();
        card.sync(snap);
        let ids: Vec<u64> = snap.selection.iter().copied().collect();
        // One object, or one whole group.
        let unit = snap.units().first().copied();
        (card.command(), card.params(&ids, unit))
    }

    /// The gap of Auto arrange.
    pub fn spacing_mm(&self, snap: &Snapshot) -> f64 {
        self.params(snap).1["spacing_mm"].as_f64().unwrap_or(0.0)
    }
}

/// Opens the card `kind`: its fields at their start values, then the
/// values the cards ran with last, and the fields with a preference as
/// stored now.
pub fn open(st: &mut PrepareState, snap: &Snapshot, kind: PanelKind) {
    let mut card = CardState::new(kind.commands(), kind.mode(), &st.panel_last);
    card.load_prefs(st);
    card.sync(snap);
    st.panel = Some(ArrangePanel {
        doc_id: snap.doc_id,
        selection: snap.selection.clone(),
        kind,
        card,
    });
}

/// The gap the one-click Auto arrange leaves: the Auto arrange card's
/// last one, else the command's default.
pub fn remembered_spacing(memory: &Values) -> f64 {
    memory
        .get(&tool_card::key("arrange.auto", "spacing_mm"))
        .and_then(Value::as_f64)
        .or_else(|| meta::start_value("arrange.auto", "spacing_mm")?.as_f64())
        .unwrap_or_default()
}

/// The stored card values (`Preferences::tool_cards`, JSON text) as values.
pub fn memory_from_prefs(stored: &std::collections::BTreeMap<String, String>) -> Values {
    stored
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), serde_json::from_str(v).ok()?)))
        .collect()
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

/// The centre of the selection a circular pattern turns about by default.
pub fn selection_centre(snap: &Snapshot) -> Option<Point> {
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

fn num(p: &Value, path: &str) -> f64 {
    p[path].as_f64().unwrap_or(0.0)
}

fn count(p: &Value, path: &str) -> u64 {
    p[path].as_u64().unwrap_or(0)
}

fn pair(p: &Value, path: &str) -> Option<[f64; 2]> {
    let a = p[path].as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
}

/// Why OK cannot run, or `None` when it can.
fn not_ready(snap: &Snapshot, p: &ArrangePanel) -> Option<String> {
    if let Some(why) = p.kind.tool().blocked(snap) {
        return Some(why.into());
    }
    let (command, params) = p.params(snap);
    // Copies are counted per unit: a whole group is copied as one.
    let n = snap.units().len() as u64;
    let too_many = |copies: u64| {
        (copies.saturating_mul(n) > u64::from(MAX_COPIES))
            .then(|| format!("That is more than {MAX_COPIES} copies. Even Kamino has limits."))
    };
    match command {
        "object.duplicate" => too_many(count(&params, "count")),
        "object.copies" => match count(&params, "count") {
            0 => Some("One cell is just the original. Add a column or a row.".into()),
            c => too_many(c),
        },
        "object.circular_copies" if num(&params, "total_deg") == 0.0 => {
            Some("A total angle of 0 stacks every copy on the original. Pick another angle.".into())
        }
        "object.circular_copies" => too_many(count(&params, "count").saturating_sub(1)),
        _ => None,
    }
}

/// The command OK runs, for the selection. `None` when it cannot run.
pub fn ok_action(snap: &Snapshot, p: &ArrangePanel) -> Option<Action> {
    if not_ready(snap, p).is_some() {
        return None;
    }
    let (command, params) = p.params(snap);
    Some(Action::run(snap.doc_id, command, params))
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
/// and the card pack once, and again only when something changed (an
/// object, the selection, the area or the spacing), not every frame.
#[derive(Debug, Clone, Default)]
pub struct AutoPlanCache {
    key: Option<PlanKey>,
    plan: Option<placement::AutoPlan<u64>>,
}

impl AutoPlanCache {
    /// The plan of the open card `p`, `None` when it is not Auto
    /// arrange or cannot run.
    pub fn get(&mut self, snap: &Snapshot, p: &ArrangePanel) -> Option<&placement::AutoPlan<u64>> {
        if p.kind != PanelKind::AutoArrange || not_ready(snap, p).is_some() {
            return None;
        }
        let key = PlanKey::of(snap, p.spacing_mm(snap))?;
        if self.key.as_ref() != Some(&key) || self.plan.is_none() {
            self.plan = Some(key.plan());
            self.key = Some(key);
        }
        self.plan.as_ref()
    }
}

/// What the Auto arrange card packs, for its title: "3 selected
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

/// The card's title: its tool's name, and for Auto arrange what it packs
/// ("Auto arrange 3 selected objects").
pub fn panel_title(snap: &Snapshot, p: &ArrangePanel) -> String {
    match p.kind {
        PanelKind::AutoArrange => format!("{} {}", p.kind.title(), auto_arrange_what(snap)),
        _ => p.kind.title().to_string(),
    }
}

/// What the Auto arrange card says when some objects do not fit.
pub fn did_not_fit_note(n: usize) -> String {
    if n == 1 {
        "1 object does not fit. It stays where it is, marked in red.".into()
    } else {
        format!("{n} objects do not fit. They stay where they are, marked in red.")
    }
}

/// The hover card of the Auto arrange card's title.
pub const AUTO_ARRANGE_CARD: &str = "Select the objects you want packed. Everything else stays where you put it, and the packed objects fit around it.";

/// The objects the open card's preview draws as fixed: for Auto arrange,
/// the visible objects it packs around. Empty for the other cards.
pub fn fixed(snap: &Snapshot, p: &ArrangePanel) -> Vec<u64> {
    AutoPlanCache::default()
        .get(snap, p)
        .map_or_else(Vec::new, |plan| plan.fixed.clone())
}

/// The would-be objects of the open card: for each, the object it shows
/// and the affine from its centre-relative shape to mat mm. Built from the
/// params OK sends, with the same `placement` functions as the commands.
pub fn ghosts(snap: &Snapshot, p: &ArrangePanel) -> Vec<(u64, Affine)> {
    ghosts_with(snap, p, AutoPlanCache::default().get(snap, p))
}

/// [`ghosts`] with the Auto arrange plan already made (see
/// [`AutoPlanCache`]); the other cards ignore it.
pub fn ghosts_with(
    snap: &Snapshot,
    p: &ArrangePanel,
    auto: Option<&placement::AutoPlan<u64>>,
) -> Vec<(u64, Affine)> {
    if not_ready(snap, p).is_some() {
        return Vec::new();
    }
    let (command, params) = p.params(snap);
    let objs = selected(snap);
    let at = |c: Point| Affine::translate(c.to_vec2());
    let turned = |c: Point, deg: f64| at(c) * Affine::rotate(deg.to_radians());
    let mut out = Vec::new();
    let shifted = |offsets: Vec<Vec2>, out: &mut Vec<(u64, Affine)>| {
        for off in offsets {
            for o in &objs {
                out.push((o.id, at(o.transform.center_mm + off)));
            }
        }
    };
    match command {
        "object.move" => {
            let d = Vec2::new(num(&params, "dx_mm"), num(&params, "dy_mm"));
            shifted(vec![d], &mut out);
        }
        "object.duplicate" => {
            let [dx, dy] = pair(&params, "offset_mm").unwrap_or_default();
            let n = u32::try_from(count(&params, "count")).unwrap_or(u32::MAX);
            shifted(placement::step_offsets(Vec2::new(dx, dy), n), &mut out);
        }
        "object.rotate" => {
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
            let deg = num(&params, "degrees");
            let each = params["about"].as_str() == Some("each");
            let moved = placement::rotated_units(&turn_units, deg, each);
            for (m, centres) in members.iter().zip(moved) {
                for (o, c) in m.iter().zip(centres) {
                    out.push((o.id, turned(c, deg)));
                }
            }
        }
        "object.copies" => {
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
            let columns = params["layout"]["grid"]["columns"]
                .as_u64()
                .map(|c| u32::try_from(c).unwrap_or(u32::MAX));
            let n = u32::try_from(count(&params, "count")).unwrap_or(u32::MAX);
            let offsets = placement::grid_offsets(
                b,
                columns,
                n,
                num(&params, "gap_x_mm"),
                num(&params, "gap_y_mm"),
            );
            shifted(offsets, &mut out);
        }
        "object.circular_copies" => {
            let Some([x, y]) = pair(&params, "center_mm") else {
                return out;
            };
            let pivot = Point::new(x, y);
            let n = u32::try_from(count(&params, "count")).unwrap_or(u32::MAX);
            let total = num(&params, "total_deg");
            let spokes = params["rotate_copies"].as_bool().unwrap_or(true);
            for u in snap.units() {
                let Some(centre) = snap.unit_centre(u) else {
                    continue;
                };
                let members: Vec<&ObjView> = snap
                    .leaves_of(u)
                    .iter()
                    .filter_map(|id| snap.object(*id))
                    .collect();
                for t in placement::circular_copies(centre, pivot, n, total, spokes) {
                    for o in &members {
                        let to = placement::carried(o.transform.center_mm, centre, t);
                        out.push((o.id, turned(to, t.rotate_deg)));
                    }
                }
            }
        }
        "arrange.auto" => {
            for (id, d) in auto.map_or(&[][..], |p| &p.moves[..]).iter().copied() {
                // A packed group moves all its members.
                for leaf in snap.leaves_of(id) {
                    if let Some(o) = snap.object(leaf) {
                        out.push((leaf, at(o.transform.center_mm + d)));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

// ---------------------------------------------------------------- specials

/// The rows a rectangular pattern starts with.
const START_ROWS: u64 = 2;
/// The most rows (and columns) a pattern takes: one more than the copies.
pub const MAX_ROWS: u64 = MAX_COPIES as u64 + 1;

/// The special widgets of these cards (see `tool_card::SPECIALS`).
pub const SPECIALS: &[Special] = &[
    Special {
        name: "group_or_each",
        command: "object.rotate",
        paths: &["about"],
        labels: &["Turn about"],
        owner: "object.rotate",
        draw: draw_about,
        sync: None,
        load: Some(load_each),
        store: Some(store_each),
    },
    Special {
        name: "grid_rows",
        command: "object.copies",
        paths: &["count"],
        labels: &["Rows"],
        owner: "pattern.rectangular",
        draw: draw_rows,
        sync: Some(sync_rows),
        load: None,
        store: None,
    },
    Special {
        name: "pattern_centre",
        command: "object.circular_copies",
        paths: &["center_mm[0]", "center_mm[1]"],
        labels: &["Centre", "Centre X", "Centre Y"],
        owner: "pattern.circular",
        draw: draw_centre,
        sync: Some(sync_centre),
        load: None,
        store: None,
    },
];

/// "Turn about": the enum's combo, dimmed below two objects.
fn draw_about(ui: &mut Ui, card: &mut CardState, snap: &Snapshot) {
    let Some(s) = tool_card::spec("object.rotate", "about") else {
        return;
    };
    let why = (snap.units().len() < 2)
        .then_some("Select 2 or more objects first. One object always turns about its own centre.");
    tool_card::field_row(ui, &s, card, why);
}

/// It opens with the ribbon's Each object switch (`rotate_each`), which
/// edits the same value.
fn load_each(card: &mut CardState, st: &PrepareState) {
    let about = if st.fields.each { "each" } else { "group" };
    card.set("object.rotate", "about", about);
}

/// OK stores the choice in the Each object switch.
fn store_each(card: &CardState, st: &mut PrepareState, actions: &mut Vec<Action>) {
    let each = card.str("object.rotate", "about") == Some("each");
    if each != st.fields.each {
        st.fields.each = each;
        actions.push(Action::RememberEach(each));
    }
}

/// The card's Rows: a value of its own; the copies are columns times rows
/// less the original.
pub const ROWS: FieldSpec = FieldSpec {
    owner: "pattern.rectangular",
    key: "rows",
    label: "Rows",
    unit: meta::Unit::Count,
    kind: FieldKind::Integer {
        min: Some(1.0),
        max: Some(MAX_ROWS as f64),
    },
    card: None,
};

fn rows_spec() -> FieldSpec {
    FieldSpec {
        card: tools::field_card(ROWS.owner, ROWS.key),
        ..ROWS
    }
}

fn draw_rows(ui: &mut Ui, card: &mut CardState, _: &Snapshot) {
    if card.get(ROWS.owner, ROWS.key).is_none() {
        card.set(ROWS.owner, ROWS.key, START_ROWS);
    }
    tool_card::field_row(ui, &rows_spec(), card, None);
}

/// The copies of the grid: columns times rows less the original, without
/// overflow (a count over the most copies is refused before OK).
fn sync_rows(card: &mut CardState, _: &Snapshot) {
    let columns = card
        .u64("object.copies", "layout.grid.columns")
        .unwrap_or(1);
    let rows = card.u64(ROWS.owner, ROWS.key).unwrap_or(START_ROWS);
    let copies = columns.saturating_mul(rows).saturating_sub(1);
    card.set("object.copies", "count", copies);
}

/// The choice of the circle's centre: the selection's, or a point.
pub const CENTRE: FieldSpec = FieldSpec {
    owner: "pattern.circular",
    key: "own_centre",
    label: "Centre",
    unit: meta::Unit::None,
    kind: FieldKind::Switch,
    card: None,
};

/// Its two choices.
pub const CENTRE_CHOICES: [&str; 2] = ["Selection centre", "Centre at X and Y"];

fn draw_centre(ui: &mut Ui, card: &mut CardState, _: &Snapshot) {
    let own = card.bool(CENTRE.owner, CENTRE.key).unwrap_or(false);
    let choice = FieldSpec {
        kind: FieldKind::Choice(vec![
            ("selection", CENTRE_CHOICES[0]),
            ("point", CENTRE_CHOICES[1]),
        ]),
        card: tools::field_card(CENTRE.owner, CENTRE.key),
        ..CENTRE
    };
    let mut tmp = card.clone();
    tmp.set(
        CENTRE.owner,
        CENTRE.key,
        if own { "point" } else { "selection" },
    );
    tool_card::field_row(ui, &choice, &mut tmp, None);
    let own = tmp.str(CENTRE.owner, CENTRE.key) == Some("point");
    card.set(CENTRE.owner, CENTRE.key, own);
    let why = (!own).then_some(
        "Choose Centre at X and Y first. Until then the circle turns about the centre of the selection.",
    );
    for path in ["center_mm[0]", "center_mm[1]"] {
        if let Some(s) = tool_card::spec("object.circular_copies", path) {
            tool_card::field_row(ui, &s, card, why);
        }
    }
}

/// While the centre is the selection's, Centre X and Y follow it, so
/// switching starts from where the preview was.
fn sync_centre(card: &mut CardState, snap: &Snapshot) {
    if card.bool(CENTRE.owner, CENTRE.key).unwrap_or(false) {
        return;
    }
    if let Some(p) = selection_centre(snap) {
        card.set("object.circular_copies", "center_mm[0]", p.x);
        card.set("object.circular_copies", "center_mm[1]", p.y);
    }
}

// ---------------------------------------------------------------- the card

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
/// fields, Cancel and OK. OK runs one command (one undo step), remembers
/// the values and closes it; Cancel and the close button change nothing.
/// Returns the card's rect, `None` when no card is open.
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
    let mut card = p.card.clone();
    let tc = ToolCard {
        icon: p.kind.tool().icon,
        title: &title,
        title_card: p.kind.title_card(),
        description: p.kind.description(),
        note: note.as_deref(),
        ok_why: why.as_deref(),
    };
    let (outcome, rect) = tool_card::show(ui, &tc, &mut card, snap, t);
    card.sync(snap);
    let Some(open) = st.panel.as_mut() else {
        return Some(rect);
    };
    open.card = card;
    match outcome {
        Some(Outcome::Ok) => {
            let p = open.clone();
            if let Some(a) = ok_action(snap, &p) {
                actions.push(a);
                p.card.store_prefs(st, actions);
                st.panel_last.extend(p.card.values.clone());
                actions.push(Action::RememberCards(p.card.values.clone()));
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
