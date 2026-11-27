//! The tool card: a tool with settings (Auto arrange, Rotate, Move /
//! Copy, the patterns) opens as a card on top of the inspector, with its
//! title, a short description, its fields, a warning line and OK and
//! Cancel. The inspector stays below it.
//!
//! The fields are built from the tool's parameter list ([`CardField`]):
//!
//! - a command's parameter takes its type, default and range from the
//!   command's schema and its short label and unit from the engine's
//!   `commands::meta` table, so the window and the agents see one
//!   description of it;
//! - a window-only field (one the command does not have, like "Rows" of a
//!   rectangular pattern) is declared next to its card ([`WindowField`]);
//! - the beginner's hover card of either comes from the UI's card table
//!   (`tools::FIELD_CARDS`), keyed by the command id (or the card's id)
//!   and the parameter's name.
//!
//! A number is a calculation field (`calc::number`) with its unit, a
//! choice of two is a combo, a bool is a switch. A field that needs more
//! names a special widget ([`SPECIALS`]). A field with a `pref` opens with
//! the value stored under that Preferences key, and OK stores it back.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use egui::{Align, Layout, Response, RichText, Ui};
use flatcraft_core::numexpr::{Field, Kind};
use flatcraft_engine::commands::meta::{self, ParamInfo, ParamKind, Unit};

use super::{blocks, job};
use crate::ui::arrange::PanelValues;
use crate::ui::calc::{self, Number};
use crate::ui::help::{self, PrepareKey};
use crate::ui::icons::{self, Icon};
use crate::ui::prepare::Snapshot;
use crate::ui::theme::Tokens;
use crate::ui::{groups, metrics, tools};

/// Where a field's value lives in the card's values.
pub enum Slot<'a> {
    /// A number with decimals.
    Number(&'a mut f64),
    /// A whole number.
    Count(&'a mut u32),
    /// A switch.
    Switch(&'a mut bool),
    /// A choice of two: the second when true.
    Pair(&'a mut bool),
}

/// A field that only the window has, declared next to its card.
#[derive(Debug, Clone, Copy)]
pub struct WindowField {
    /// The card's id (a ribbon action id), for the card table.
    pub owner: &'static str,
    pub id: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub kind: WindowKind,
}

/// The type of a window-only field.
#[derive(Debug, Clone, Copy)]
pub enum WindowKind {
    Count {
        min: u32,
        max: u32,
    },
    Switch,
    /// The labels of a choice of two.
    Choice(&'static [&'static str]),
}

/// What a field shows.
#[derive(Debug, Clone, Copy)]
pub enum Source {
    /// A parameter of a registry command, by its path in the params.
    Param {
        command: &'static str,
        path: &'static str,
    },
    Window(WindowField),
}

/// Why a field does not apply now, or `None` when it does.
pub type Why = fn(&PanelValues, &Snapshot) -> Option<&'static str>;

/// A special widget: draws the value of the field into the row's value
/// column and returns its response. `enabled` is false while it is
/// dimmed.
pub type Special = fn(&mut Ui, &FieldSpec, &mut PanelValues, &Snapshot, bool) -> Response;

/// One field of a card.
#[derive(Clone, Copy)]
pub struct CardField {
    pub source: Source,
    pub bind: fn(&mut PanelValues) -> Slot<'_>,
    pub why: Option<Why>,
    /// The name of a special widget in [`SPECIALS`].
    pub special: Option<&'static str>,
    /// The Preferences key the value opens with and OK stores to.
    pub pref: Option<&'static str>,
}

impl CardField {
    pub const fn param(
        command: &'static str,
        path: &'static str,
        bind: fn(&mut PanelValues) -> Slot<'_>,
    ) -> Self {
        Self {
            source: Source::Param { command, path },
            bind,
            why: None,
            special: None,
            pref: None,
        }
    }

    pub const fn window(field: WindowField, bind: fn(&mut PanelValues) -> Slot<'_>) -> Self {
        Self {
            source: Source::Window(field),
            bind,
            why: None,
            special: None,
            pref: None,
        }
    }

    pub const fn dimmed(mut self, why: Why) -> Self {
        self.why = Some(why);
        self
    }

    pub const fn special(mut self, name: &'static str) -> Self {
        self.special = Some(name);
        self
    }

    pub const fn pref(mut self, key: &'static str) -> Self {
        self.pref = Some(key);
        self
    }
}

/// The special widgets, by name.
pub const SPECIALS: &[(&str, Special)] = &[("pattern_centre", crate::ui::arrange::centre_choice)];

/// The special widget `name`.
pub fn special(name: &str) -> Option<Special> {
    SPECIALS.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

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
    /// The labels of a choice of two.
    Choice(Vec<&'static str>),
}

/// Everything a field row shows.
#[derive(Debug, Clone)]
pub struct FieldSpec {
    /// The command id, or the card's id for a window-only field.
    pub owner: &'static str,
    /// The parameter's path, or the window-only field's id.
    pub key: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub kind: FieldKind,
    /// The beginner's hover card from the card table.
    pub card: Option<&'static str>,
}

/// The schema facts of the parameters read so far, by command and path.
type InfoCache = HashMap<(&'static str, &'static str), Option<ParamInfo>>;

/// The schema facts of a parameter, read once.
fn info(command: &'static str, path: &'static str) -> Option<ParamInfo> {
    static CACHE: LazyLock<Mutex<InfoCache>> = LazyLock::new(Default::default);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache
        .entry((command, path))
        .or_insert_with(|| meta::param_info(command, path))
        .clone()
}

/// What the field `f` shows; `None` when its parameter is not known (a
/// test checks every card's fields resolve).
pub fn spec(f: &CardField) -> Option<FieldSpec> {
    match f.source {
        Source::Param { command, path } => {
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
                ParamKind::Enum(_) => FieldKind::Choice(m.choices.iter().map(|c| c.1).collect()),
            };
            Some(FieldSpec {
                owner: command,
                key: path,
                label: m.label,
                unit: m.unit,
                kind,
                card: tools::field_card(command, path),
            })
        }
        Source::Window(w) => Some(FieldSpec {
            owner: w.owner,
            key: w.id,
            label: w.label,
            unit: w.unit,
            kind: match w.kind {
                WindowKind::Count { min, max } => FieldKind::Integer {
                    min: Some(f64::from(min)),
                    max: Some(f64::from(max)),
                },
                WindowKind::Switch => FieldKind::Switch,
                WindowKind::Choice(labels) => FieldKind::Choice(labels.to_vec()),
            },
            card: tools::field_card(w.owner, w.id),
        }),
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

/// Bounds this far out are no limit a person meets (a kilometre and
/// more): the card does not mention them.
const FAR: f64 = 1e5;

/// The range sentence of a number field ("From -360 to 360 deg.").
pub fn range_text(kind: &FieldKind, unit: Unit) -> String {
    let (min, max, whole) = match kind {
        FieldKind::Number { min, max } => (*min, *max, false),
        FieldKind::Integer { min, max } => (*min, *max, true),
        _ => return String::new(),
    };
    let near = |v: Option<f64>| v.filter(|v| v.abs() < FAR);
    let u = match unit.symbol() {
        "" => String::new(),
        s => format!(" {s}"),
    };
    match (near(min), near(max)) {
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
    for part in [range.as_str(), calc_hint(s)] {
        if !part.is_empty() {
            out.push(' ');
            out.push_str(part);
        }
    }
    out
}

fn calc_hint(s: &FieldSpec) -> &'static str {
    match s.kind {
        FieldKind::Number { .. } | FieldKind::Integer { .. } => calc::hint(calc_kind(s.unit)),
        _ => "",
    }
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

/// A number field that fills the value column, less its unit.
fn number_widget<T: egui::emath::Numeric>(
    ui: &mut Ui,
    s: &FieldSpec,
    value: &mut T,
    enabled: bool,
) -> Response {
    let (min, max, decimals) = match s.kind {
        FieldKind::Number { min, max } => (min, max, (2, 3)),
        FieldKind::Integer { min, max } => (min, max, (0, 0)),
        _ => (None, None, (0, 0)),
    };
    let unit = s.unit.symbol();
    let w = ui.available_width() - unit_width(ui, unit);
    let field = Field::new(calc_kind(s.unit)).range(
        min.unwrap_or(f64::NEG_INFINITY),
        max.unwrap_or(f64::INFINITY),
    );
    let spec = Number::new(field)
        .decimals(decimals.0, decimals.1)
        .unit(unit)
        .width(w);
    calc::number(
        ui,
        ("flatcraft-tool-card", s.owner, s.key),
        enabled,
        value,
        &spec,
    )
    .0
}

/// One field row.
fn field_row(ui: &mut Ui, f: &CardField, values: &mut PanelValues, snap: &Snapshot) {
    let Some(s) = spec(f) else {
        return;
    };
    let why = f.why.and_then(|w| w(values, snap));
    let enabled = why.is_none();
    let card = card_text(&s);
    if let Some(draw) = f.special.and_then(special) {
        blocks::row_dimmed(ui, s.label, &card, why, None, |ui| {
            draw(ui, &s, values, snap, enabled)
        });
        return;
    }
    blocks::row_dimmed(ui, s.label, &card, why, None, |ui| match (f.bind)(values) {
        Slot::Number(v) => number_widget(ui, &s, v, enabled),
        Slot::Count(v) => number_widget(ui, &s, v, enabled),
        Slot::Switch(v) => ui.add_enabled(enabled, egui::Checkbox::without_text(v)),
        Slot::Pair(v) => pair_widget(ui, &s, v, enabled),
    });
}

/// A choice of two as a combo.
pub fn pair_widget(ui: &mut Ui, s: &FieldSpec, v: &mut bool, enabled: bool) -> Response {
    let labels = match &s.kind {
        FieldKind::Choice(l) => l.clone(),
        _ => Vec::new(),
    };
    let label = |i: usize| labels.get(i).copied().unwrap_or_default().to_string();
    let choices = [(false, label(0)), (true, label(1))];
    let text = label(usize::from(*v));
    let salt = format!("flatcraft-tool-card-{}-{}", s.owner, s.key);
    let (r, picked) = job::combo(ui, &salt, &text, &choices, &[], Some(v), Some(v), enabled);
    if let Some(p) = picked {
        *v = p;
    }
    r
}

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
    pub fields: &'a [CardField],
    /// A dimmed note over the buttons (what does not fit, why OK is off).
    pub note: Option<&'a str>,
    /// Why OK cannot run, or `None` when it can.
    pub ok_why: Option<&'a str>,
}

/// The name of the card's close button.
pub const CLOSE: &str = "Close panel";
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
    values: &mut PanelValues,
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
        // The fields scroll on their own when the window is short, so the
        // title and the buttons stay in sight.
        // The room left in the panel, less the note and the buttons, and
        // never more than a share of the window, so the inspector keeps
        // some room below the card.
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
                let labels: Vec<&str> = card
                    .fields
                    .iter()
                    .filter_map(spec)
                    .map(|s| s.label)
                    .collect();
                blocks::labels(ui, &labels);
                for f in card.fields {
                    field_row(ui, f, values, snap);
                }
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
