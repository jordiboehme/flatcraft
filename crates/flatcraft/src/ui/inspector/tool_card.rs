//! The tool card: a tool with settings (Auto arrange, Rotate, Move /
//! Copy, the patterns) opens as a card on top of the inspector, with its
//! title, a short description, its fields, a note and OK and Cancel. The
//! inspector stays below it.
//!
//! A card is built from its command's parameter list alone:
//!
//! - the fields are the rows the engine's `commands::meta` table lists for
//!   the command, in that order, each with its short label and unit;
//! - the type, the range and the default of each come from the command's
//!   schema; where the command has no default, from the table's start
//!   value (which the schema's description states for agents too);
//! - the beginner's hover card comes from the UI's card table
//!   (`tools::FIELD_CARDS`), keyed by command id and parameter path;
//! - the widget follows the type: a number is a calculation field
//!   (`calc::number`) with its unit, a bool a switch, an enum a combo;
//! - OK sends the command with every field put at its path in the params
//!   (`layout.grid.columns`, `center_mm[0]`), plus the objects.
//!
//! A command listed in the table gets a working card with nothing else.
//! Only a field that needs more registers a special widget by name
//! ([`SPECIALS`]): it draws its own rows, can keep values the command does
//! not have (the rows of a grid) and fills in the command's parameters
//! from them before a preview or OK.
//!
//! The values a card ran with are remembered (`Preferences::tool_cards`);
//! a card opens with them, else with the start values.

use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};

use egui::{Align, Layout, RichText, Ui};
use flatcraft_core::numexpr::{Field, Kind};
use flatcraft_engine::commands::meta::{self, ParamInfo, ParamKind, Unit};
use serde_json::Value;

use super::{blocks, job};
use crate::ui::calc::{self, Number};
use crate::ui::help::{self, PrepareKey};
use crate::ui::icons::{self, Icon};
use crate::ui::prepare::{PrepareState, Snapshot};
use crate::ui::theme::Tokens;
use crate::ui::{Action, groups, metrics, tools};

// ---------------------------------------------------------------- values

/// The values of a card, by [`key`]: a command's parameter
/// ("object.copies layout.grid.columns") or a card's own value
/// ("pattern.rectangular rows").
pub type Values = BTreeMap<String, Value>;

/// The key of `path` of `owner` (a command id, or a card's id).
pub fn key(owner: &str, path: &str) -> String {
    format!("{owner} {path}")
}

/// What a card holds while it is open: the commands it can run, which one
/// runs now and the values so far.
#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    /// The commands of the card; a card with two switches between them
    /// with its [`Mode`] switch.
    pub commands: &'static [&'static str],
    pub mode: Option<Mode>,
    pub values: Values,
}

/// The switch between the two commands of a card ("Copy" on Move / Copy
/// runs the second one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    /// The card's id (a ribbon action id), for its value and its card text.
    pub owner: &'static str,
    pub id: &'static str,
    pub label: &'static str,
}

impl CardState {
    /// A card for `commands`: every field at its start value, then the
    /// remembered values over them.
    pub fn new(commands: &'static [&'static str], mode: Option<Mode>, memory: &Values) -> Self {
        let mut values = Values::new();
        for c in commands {
            for m in meta::fields_of(c) {
                if let Some(v) = meta::start_value(c, m.path) {
                    values.insert(key(c, m.path), v);
                }
            }
        }
        let mut state = Self {
            commands,
            mode,
            values,
        };
        for (k, v) in memory {
            if state.owns(k) {
                state.values.insert(k.clone(), v.clone());
            }
        }
        state
    }

    /// Whether `k` is one of this card's values.
    fn owns(&self, k: &str) -> bool {
        let owner = k.split(' ').next().unwrap_or_default();
        self.commands.contains(&owner)
            || self.mode.is_some_and(|m| m.owner == owner)
            || SPECIALS
                .iter()
                .any(|s| self.commands.contains(&s.command) && s.owner == owner)
    }

    /// The command OK runs now.
    pub fn command(&self) -> &'static str {
        let second = self
            .mode
            .is_some_and(|m| self.bool(m.owner, m.id).unwrap_or(false));
        let i = usize::from(second).min(self.commands.len().saturating_sub(1));
        self.commands[i]
    }

    pub fn get(&self, owner: &str, path: &str) -> Option<&Value> {
        self.values.get(&key(owner, path))
    }

    pub fn f64(&self, owner: &str, path: &str) -> Option<f64> {
        self.get(owner, path)?.as_f64()
    }

    pub fn u64(&self, owner: &str, path: &str) -> Option<u64> {
        let v = self.get(owner, path)?;
        v.as_u64()
            .or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
    }

    pub fn bool(&self, owner: &str, path: &str) -> Option<bool> {
        self.get(owner, path)?.as_bool()
    }

    pub fn str(&self, owner: &str, path: &str) -> Option<&str> {
        self.get(owner, path)?.as_str()
    }

    pub fn set(&mut self, owner: &str, path: &str, v: impl Into<Value>) {
        self.values.insert(key(owner, path), v.into());
    }

    /// The params of the command that runs now: every field at its path,
    /// and the objects (`ids`, or `id` for a command of one object).
    pub fn params(&self, ids: &[u64], unit: Option<u64>) -> Value {
        params_of(self, self.command(), ids, unit)
    }
}

/// The params of `command` from the values of `state`.
pub fn params_of(state: &CardState, command: &str, ids: &[u64], unit: Option<u64>) -> Value {
    let mut out = Value::Object(Default::default());
    if meta::param_info(command, "ids").is_some() {
        out["ids"] = Value::from(ids.to_vec());
    } else if meta::param_info(command, "id").is_some() {
        out["id"] = Value::from(unit.or(ids.first().copied()));
    }
    for m in meta::fields_of(command) {
        if let Some(v) = state.get(command, m.path) {
            set_path(&mut out, m.path, v.clone());
        }
    }
    out
}

/// Puts `v` at `path` in `obj`: `a.b` nests objects, `a[1]` is an item of
/// an array (filled up with zeros before it).
pub fn set_path(obj: &mut Value, path: &str, v: Value) {
    let mut node = obj;
    let steps: Vec<&str> = path.split('.').collect();
    for (i, step) in steps.iter().enumerate() {
        let last = i + 1 == steps.len();
        let (name, index) = match step.split_once('[') {
            Some((n, rest)) => (n, rest.trim_end_matches(']').parse::<usize>().ok()),
            None => (*step, None),
        };
        if !node.is_object() {
            *node = Value::Object(Default::default());
        }
        let slot = node
            .as_object_mut()
            .expect("an object")
            .entry(name)
            .or_insert(Value::Null);
        match index {
            Some(ix) => {
                if !slot.is_array() {
                    *slot = Value::Array(Vec::new());
                }
                let arr = slot.as_array_mut().expect("an array");
                while arr.len() <= ix {
                    arr.push(Value::from(0.0));
                }
                if last {
                    arr[ix] = v;
                    return;
                }
                node = &mut arr[ix];
            }
            None if last => {
                *slot = v;
                return;
            }
            None => node = slot,
        }
    }
}

// ---------------------------------------------------------------- fields

/// The type of a field, resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    Integer {
        min: Option<f64>,
        max: Option<f64>,
    },
    Switch,
    /// Each value with its plain label.
    Choice(Vec<(&'static str, &'static str)>),
}

/// Everything a field row shows.
#[derive(Debug, Clone)]
pub struct FieldSpec {
    /// The command id, or the card's id for a card's own value.
    pub owner: &'static str,
    /// The parameter's path, or the card's own value's id.
    pub key: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub kind: FieldKind,
    /// The beginner's hover card from the card table.
    pub card: Option<&'static str>,
}

/// The schema facts of the parameters read so far, by command and path.
type InfoCache = HashMap<(String, String), Option<ParamInfo>>;

/// The schema facts of a parameter, read once.
fn info(command: &str, path: &str) -> Option<ParamInfo> {
    static CACHE: LazyLock<Mutex<InfoCache>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry((command.to_string(), path.to_string()))
        .or_insert_with(|| meta::param_info(command, path))
        .clone()
}

/// The field of `path` of `command`; `None` when the table or the schema
/// does not know it (a test checks every row resolves).
pub fn spec(command: &str, path: &str) -> Option<FieldSpec> {
    let m = meta::meta(command, path)?;
    let i = info(command, path)?;
    let kind = match i.kind {
        ParamKind::Number => FieldKind::Number {
            min: i.min,
            max: i.max,
        },
        ParamKind::Integer => FieldKind::Integer {
            min: i.min,
            max: i.max,
        },
        ParamKind::Bool => FieldKind::Switch,
        ParamKind::Enum(_) => FieldKind::Choice(m.choices.to_vec()),
    };
    Some(FieldSpec {
        owner: m.command,
        key: m.path,
        label: m.label,
        unit: m.unit,
        kind,
        card: tools::field_card(m.command, m.path),
    })
}

/// The field of the card's mode switch.
pub fn mode_spec(m: Mode) -> FieldSpec {
    FieldSpec {
        owner: m.owner,
        key: m.id,
        label: m.label,
        unit: Unit::None,
        kind: FieldKind::Switch,
        card: tools::field_card(m.owner, m.id),
    }
}

/// The calculation kind of a unit.
fn calc_kind(unit: Unit) -> Kind {
    match unit {
        Unit::Mm => Kind::Length,
        Unit::Deg => Kind::Angle,
        Unit::Count | Unit::None => Kind::Plain,
    }
}

/// The range sentence of a number field ("From -360 to 360 deg.").
pub fn range_text(kind: &FieldKind, unit: Unit) -> String {
    let (min, max, whole) = match kind {
        FieldKind::Number { min, max } => (*min, *max, false),
        FieldKind::Integer { min, max } => (*min, *max, true),
        _ => return String::new(),
    };
    let u = match unit.symbol() {
        "" => String::new(),
        s => format!(" {s}"),
    };
    match (min, max) {
        (Some(lo), Some(hi)) if whole => format!("A whole number from {lo} to {hi}."),
        (Some(lo), Some(hi)) => format!("From {lo}{u} to {hi}{u}."),
        (Some(lo), None) => format!("{lo}{u} or more."),
        (None, Some(hi)) => format!("At most {hi}{u}."),
        (None, None) if whole => "A whole number.".into(),
        (None, None) => String::new(),
    }
}

/// The whole hover card of a field: the card table's text, the range
/// and, for a number, the calculation hint.
pub fn card_text(s: &FieldSpec) -> String {
    let mut out = s.card.unwrap_or_default().to_string();
    let range = range_text(&s.kind, s.unit);
    let hint = match s.kind {
        FieldKind::Number { .. } | FieldKind::Integer { .. } => calc::hint(calc_kind(s.unit)),
        _ => "",
    };
    for part in [range.as_str(), hint] {
        if !part.is_empty() {
            out.push(' ');
            out.push_str(part);
        }
    }
    out
}

/// The width of `unit` after a field, in whole points.
fn unit_width(ui: &Ui, unit: &str) -> f32 {
    if unit.is_empty() {
        return 0.0;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(unit.into(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x
        .ceil()
        + ui.spacing().item_spacing.x
}

/// A field row of `s`, its value in `state` under `s.owner`/`s.key`.
/// `why` dims it. The widget follows the field's kind.
pub fn field_row(ui: &mut Ui, s: &FieldSpec, state: &mut CardState, why: Option<&str>) {
    let enabled = why.is_none();
    let card = card_text(s);
    blocks::row_dimmed(ui, s.label, &card, why, None, |ui| match &s.kind {
        FieldKind::Number { min, max } | FieldKind::Integer { min, max } => {
            let whole = matches!(s.kind, FieldKind::Integer { .. });
            let unit = s.unit.symbol();
            let w = ui.available_width() - unit_width(ui, unit);
            let field = Field::new(calc_kind(s.unit)).range(
                min.unwrap_or(f64::NEG_INFINITY),
                max.unwrap_or(f64::INFINITY),
            );
            let decimals = if whole { (0, 0) } else { (2, 3) };
            let spec = Number::new(field)
                .decimals(decimals.0, decimals.1)
                .unit(unit)
                .width(w);
            let salt = ("flatcraft-tool-card", s.owner, s.key);
            if whole {
                let mut v = state.u64(s.owner, s.key).unwrap_or(0);
                let (r, changed) = calc::number(ui, salt, enabled, &mut v, &spec);
                if changed {
                    state.set(s.owner, s.key, v);
                }
                r
            } else {
                let mut v = state.f64(s.owner, s.key).unwrap_or(0.0);
                let (r, changed) = calc::number(ui, salt, enabled, &mut v, &spec);
                if changed {
                    state.set(s.owner, s.key, v);
                }
                r
            }
        }
        FieldKind::Switch => {
            let mut v = state.bool(s.owner, s.key).unwrap_or(false);
            let r = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut v));
            if r.changed() {
                state.set(s.owner, s.key, v);
            }
            r
        }
        FieldKind::Choice(choices) => {
            let now = state.str(s.owner, s.key).map(str::to_string);
            let items: Vec<(String, String)> = choices
                .iter()
                .map(|(v, l)| (v.to_string(), l.to_string()))
                .collect();
            let text = choices
                .iter()
                .find(|(v, _)| Some(*v) == now.as_deref())
                .map_or("", |c| c.1);
            let salt = format!("flatcraft-tool-card-{}-{}", s.owner, s.key);
            let (r, picked) = job::combo(
                ui,
                &salt,
                text,
                &items,
                &[],
                now.as_ref(),
                now.as_ref(),
                enabled,
            );
            if let Some(p) = picked {
                state.set(s.owner, s.key, p);
            }
            r
        }
    });
}

// ---------------------------------------------------------------- specials

/// A field that needs more than its type gives: it draws the rows of the
/// parameters in `paths` itself, may keep values of its own under
/// `owner`, and fills in the parameters from them before a preview or OK.
#[derive(Clone, Copy)]
pub struct Special {
    pub name: &'static str,
    pub command: &'static str,
    /// The parameters it draws; it is drawn where the first one is.
    pub paths: &'static [&'static str],
    /// The labels of the rows it draws, for the label column.
    pub labels: &'static [&'static str],
    /// The id its own values are kept under (a card id).
    pub owner: &'static str,
    pub draw: fn(&mut Ui, &mut CardState, &Snapshot),
    /// Fills in the command's parameters from the values.
    pub sync: Option<fn(&mut CardState, &Snapshot)>,
    /// Opens with a stored preference instead of the remembered value.
    pub load: Option<fn(&mut CardState, &PrepareState)>,
    /// OK stores the preference.
    pub store: Option<fn(&CardState, &mut PrepareState, &mut Vec<Action>)>,
}

/// The special widgets, by name.
pub const SPECIALS: &[Special] = crate::ui::arrange::SPECIALS;

/// The special widget `name`.
pub fn special(name: &str) -> Option<&'static Special> {
    SPECIALS.iter().find(|s| s.name == name)
}

/// The special that draws `path` of `command`, if one does.
pub fn special_of(command: &str, path: &str) -> Option<&'static Special> {
    SPECIALS
        .iter()
        .find(|s| s.command == command && s.paths.contains(&path))
}

impl CardState {
    /// Runs the specials' fill-ins; call before a preview or OK.
    pub fn sync(&mut self, snap: &Snapshot) {
        for s in SPECIALS
            .iter()
            .filter(|s| self.commands.contains(&s.command))
        {
            if let Some(f) = s.sync {
                f(self, snap);
            }
        }
    }

    /// Opens the fields with a stored preference with it.
    pub fn load_prefs(&mut self, st: &PrepareState) {
        for s in SPECIALS
            .iter()
            .filter(|s| self.commands.contains(&s.command))
        {
            if let Some(f) = s.load {
                f(self, st);
            }
        }
    }

    /// OK stores the fields with a preference that changed.
    pub fn store_prefs(&self, st: &mut PrepareState, actions: &mut Vec<Action>) {
        for s in SPECIALS
            .iter()
            .filter(|s| self.commands.contains(&s.command))
        {
            if let Some(f) = s.store {
                f(self, st, actions);
            }
        }
    }

    /// The labels of the rows the card shows now, for the label column.
    pub fn labels(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self.mode.iter().map(|m| m.label).collect();
        let command = self.command();
        let mut drawn: Vec<&str> = Vec::new();
        for m in meta::fields_of(command) {
            match special_of(command, m.path) {
                Some(sp) if !drawn.contains(&sp.name) => {
                    drawn.push(sp.name);
                    out.extend(sp.labels);
                }
                Some(_) => {}
                None => out.push(m.label),
            }
        }
        out
    }
}

/// The rows of the card: the mode switch, then the fields of the command
/// that runs now, in the table's order.
fn fields(ui: &mut Ui, state: &mut CardState, snap: &Snapshot) {
    if let Some(m) = state.mode {
        field_row(ui, &mode_spec(m), state, None);
    }
    let command = state.command();
    let mut drawn: Vec<&str> = Vec::new();
    for m in meta::fields_of(command) {
        if let Some(sp) = special_of(command, m.path) {
            if !drawn.contains(&sp.name) {
                drawn.push(sp.name);
                (sp.draw)(ui, state, snap);
            }
            continue;
        }
        if let Some(s) = spec(command, m.path) {
            field_row(ui, &s, state, None);
        }
    }
}

// ---------------------------------------------------------------- the card

/// What the person did with the card this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Cancel,
}

/// A tool card to draw.
pub struct ToolCard<'a> {
    pub icon: Icon,
    pub title: &'a str,
    /// The title's hover card.
    pub title_card: &'a str,
    /// One or two dimmed lines under the title.
    pub description: &'a str,
    /// A dimmed note over the buttons (what does not fit, why OK is off).
    pub note: Option<&'a str>,
    /// Why OK cannot run, or `None` when it can.
    pub ok_why: Option<&'a str>,
}

/// The name of the card's close button.
pub const CLOSE: &str = "Close card";
/// The cards of the close button, OK and Cancel.
pub const CLOSE_CARD: &str =
    "Close this card and change nothing. The preview on the mat goes away. Esc does the same.";
pub const OK_CARD: &str =
    "Do it now, with these values, on the objects the preview shows. One step to undo.";
pub const CANCEL_CARD: &str =
    "Close this card and change nothing. The preview on the mat goes away.";

/// Draws the card; returns what was clicked and the card's rect.
pub fn show(
    ui: &mut Ui,
    card: &ToolCard<'_>,
    state: &mut CardState,
    snap: &Snapshot,
    t: &Tokens,
) -> (Option<Outcome>, egui::Rect) {
    let mut out = None;
    let frame = egui::Frame::new()
        .fill(t.panel)
        .stroke(egui::Stroke::new(metrics::HAIRLINE, t.border))
        .corner_radius(t.radius)
        .inner_margin(egui::Margin::same(metrics::CARD_PAD as i8));
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        blocks::panel_spacing(ui);
        // Header: icon, title and the close button.
        ui.horizontal_top(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                let r = crate::ui::icon_button(ui, Icon::Close, CLOSE, true);
                groups::card(&r, CLOSE, CLOSE_CARD, None, false);
                if r.clicked() {
                    out = Some(Outcome::Cancel);
                }
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = metrics::SPACE_8;
                    let tex = icons::texture(ui.ctx(), card.icon, metrics::ICON_COMPACT);
                    ui.image(egui::load::SizedTexture::new(
                        tex.id(),
                        egui::Vec2::splat(metrics::ICON_COMPACT),
                    ));
                    let r = ui.add(
                        egui::Label::new(RichText::new(card.title).font(metrics::heading())).wrap(),
                    );
                    groups::card(&r, card.title, card.title_card, None, false);
                });
            });
        });
        if !card.description.is_empty() {
            ui.add_space(metrics::SPACE_4);
            ui.add(
                egui::Label::new(
                    RichText::new(card.description)
                        .font(metrics::secondary())
                        .color(t.text_dim),
                )
                .wrap(),
            );
        }
        ui.add_space(metrics::SPACE_4);
        // The fields scroll on their own when the window is short: the
        // room left in the panel, less the note and the buttons, and never
        // more than a share of the window, so the inspector keeps some
        // room below the card.
        let left = ui.available_height() - metrics::ROW * metrics::TOOL_CARD_FOOT_ROWS;
        let room = left
            .min(ui.ctx().content_rect().height() * metrics::TOOL_CARD_FIELDS_SHARE)
            .max(metrics::ROW * 2.0);
        egui::ScrollArea::vertical()
            .id_salt("flatcraft-tool-card")
            .max_height(room)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                blocks::panel_spacing(ui);
                blocks::labels(ui, &state.labels());
                fields(ui, state, snap);
            });
        if let Some(note) = card.note {
            ui.add_space(metrics::SPACE_4);
            ui.add(egui::Label::new(RichText::new(note).color(t.text_dim)).wrap());
        }
        ui.add_space(metrics::SPACE_8);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = metrics::SPACE_8;
            let r = ui.add_enabled(card.ok_why.is_none(), egui::Button::new("OK"));
            groups::card(&r, "OK", OK_CARD, card.ok_why, false);
            if calc::click(ui, &r) {
                out = Some(Outcome::Ok);
            }
            let cancel =
                egui::Button::new("Cancel").shortcut_text(help::hint(PrepareKey::ClearSelection));
            let r = ui.add(cancel);
            groups::card(&r, "Cancel", CANCEL_CARD, None, false);
            if r.clicked() {
                out = Some(Outcome::Cancel);
            }
        });
    });
    (out, shown.response.rect)
}
