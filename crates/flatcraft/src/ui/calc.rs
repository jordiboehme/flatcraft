//! Number fields that take a calculation (`100/3`, `2in`, `+=10`), read by
//! [`flatcraft_core::numexpr`]: one text field for every number of the app
//! ([`number`]), the click rules ([`select_whole_value`]), the hover card
//! hint, the red outline and the message under a field, and the toasts of
//! texts that could not be read.
//!
//! How a number field works: it shows its value; a single click places
//! the text cursor, a double-click or Tab into it selects the whole value.
//! The typed text counts when the field is left (Enter, Tab, Shift-Tab, a
//! click anywhere else). Esc puts the old value back. A text it cannot read
//! changes nothing: after Enter the field keeps the text and the keys and
//! says why under it ("... Press Esc to go back."); when the field is left
//! any other way, the old value comes back and a toast says why ("...
//! Nothing changed.").

use crate::ui::metrics;
use std::hash::Hash;

use egui::emath::Numeric;
use egui::{Id, Response, Ui};
use flatcraft_core::numexpr::{Field, Kind};

use super::theme::Tokens;

/// The line a number field's hover card ends with, for its kind.
pub fn hint(kind: Kind) -> &'static str {
    match kind {
        Kind::Length => "You can type a calculation, like 100/3 or 2in.",
        Kind::Angle => "You can type a calculation, like 360/8 or +=90.",
        Kind::Percent => "You can type a calculation, like 100/3 or *=2.",
        Kind::Plain => "You can type a calculation, like 2*3 or +=1.",
    }
}

/// A hover card text with the calculation hint of `kind` after it.
pub fn with_hint(text: &str, kind: Kind) -> String {
    format!("{text} {}", hint(kind))
}

/// What a field under the keys says after Enter with a text it cannot read.
pub fn kept(message: &str) -> String {
    format!("{message} Press Esc to go back.")
}

/// What the toast says when a field with a text it cannot read was left.
pub fn dropped(message: &str) -> String {
    format!("{message} Nothing changed.")
}

// ---------------------------------------------------------------- keys

/// Enter and Escape of this pass, noted before anything consumed them.
#[derive(Clone, Copy, Default)]
struct Keys {
    pass: u64,
    enter: bool,
    escape: bool,
}

fn keys_id() -> Id {
    Id::unique("flatcraft-calc-keys")
}

/// Notes Enter and Escape of this pass. The shell calls it first, before
/// shortcut handlers consume keys, so a field still sees its Esc.
pub fn note_keys(ctx: &egui::Context) {
    let (enter, escape) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(
            keys_id(),
            Keys {
                pass,
                enter,
                escape,
            },
        )
    });
}

/// Enter and Escape pressed in this pass.
pub fn keys(ctx: &egui::Context) -> (bool, bool) {
    let pass = ctx.cumulative_pass_nr();
    let noted = ctx
        .data(|d| d.get_temp::<Keys>(keys_id()))
        .filter(|k| k.pass == pass)
        .unwrap_or_default();
    let (enter, escape) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    (enter || noted.enter, escape || noted.escape)
}

// ---------------------------------------------------------------- toasts

fn toasts_id() -> Id {
    Id::unique("flatcraft-calc-toasts")
}

/// Queues a toast; the shell shows it ([`take_toasts`]).
pub fn toast(ctx: &egui::Context, text: String) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<String>>(toasts_id())
            .push(text)
    });
}

/// The toasts number fields queued since the last call.
pub fn take_toasts(ctx: &egui::Context) -> Vec<String> {
    ctx.data_mut(|d| d.remove_temp::<Vec<String>>(toasts_id()))
        .unwrap_or_default()
}

// ---------------------------------------------------------------- messages

/// A field's message, with the pass it was last drawn in.
#[derive(Clone)]
struct Message {
    text: String,
    pass: u64,
}

fn message_id(id: Id) -> Id {
    id.with("flatcraft-calc-message")
}

/// The message a field shows, if any. The message of a field that was not
/// drawn in the last pass (its panel or window closed) is gone.
pub fn message(ctx: &egui::Context, id: Id) -> Option<String> {
    let now = ctx.cumulative_pass_nr();
    let m = ctx.data(|d| d.get_temp::<Message>(message_id(id)))?;
    (m.pass + 1 >= now).then_some(m.text)
}

fn set_message(ctx: &egui::Context, id: Id, msg: Option<String>) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| match msg {
        Some(text) => {
            d.insert_temp(message_id(id), Message { text, pass });
        }
        None => d.remove::<Message>(message_id(id)),
    });
}

/// Paints a red outline around a field's `rect` and `msg` just under it,
/// above everything else so the layout does not move.
pub fn show_message(ui: &Ui, id: Id, rect: egui::Rect, msg: &str) {
    let t = Tokens::of(ui.visuals());
    ui.painter().rect_stroke(
        rect.expand(1.0),
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.5, t.danger),
        egui::StrokeKind::Outside,
    );
    egui::Area::new(message_id(id))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.left_bottom() + egui::vec2(0.0, 3.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .stroke(egui::Stroke::new(1.0, t.danger))
                .show(ui, |ui| {
                    ui.set_max_width(320.0);
                    ui.label(egui::RichText::new(msg).color(t.danger));
                });
        });
}

// ---------------------------------------------------------------- selecting

/// How a number field selects: a double-click selects the whole value
/// (not only the digits under the pointer) and so does Tab or Shift-Tab
/// into it, so typing replaces it. A single click only places the text
/// cursor. Call it with the field's response and text, right after adding
/// it.
pub fn select_whole_value(ui: &Ui, resp: &Response, text: &str) {
    let by_pointer = ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
    let keyboard_focus = resp.gained_focus() && !by_pointer && !resp.clicked();
    if !(resp.double_clicked() || keyboard_focus) {
        return;
    }
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
        let len = text.chars().count();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(len),
            )));
        state.store(ui.ctx(), resp.id);
    }
}

// ---------------------------------------------------------------- held clicks

/// The number field being typed into, and the pass it was drawn in.
#[derive(Clone, Copy)]
struct Editing {
    id: Id,
    pass: u64,
}

fn editing_id() -> Id {
    Id::unique("flatcraft-calc-editing")
}

/// The number field ([`number`]) being typed into, if one was drawn in the
/// last pass.
pub fn editing(ctx: &egui::Context) -> Option<Id> {
    let now = ctx.cumulative_pass_nr();
    let e = ctx.data(|d| d.get_temp::<Editing>(editing_id()))?;
    (e.pass + 1 >= now).then_some(e.id)
}

/// A dialog button's click (OK, Apply, Save preset). While a number field
/// is being typed into, the click waits: the field is left first, so its
/// value counts (or, if it cannot be read, comes back with a toast), and
/// the click counts a frame later. Use it instead of `resp.clicked()`.
pub fn click(ui: &Ui, resp: &Response) -> bool {
    let ctx = ui.ctx();
    let held_id = resp.id.with("flatcraft-calc-held");
    let held = ctx.data(|d| d.get_temp::<bool>(held_id)).unwrap_or(false);
    let open = editing(ctx);
    if resp.clicked() || held {
        if let Some(field) = open {
            ctx.memory_mut(|m| m.surrender_focus(field));
            ctx.data_mut(|d| d.insert_temp(held_id, true));
            ctx.request_repaint();
            return false;
        }
        ctx.data_mut(|d| d.remove::<bool>(held_id));
        return true;
    }
    false
}

// ---------------------------------------------------------------- the field

/// What a number field takes and how it shows its value.
#[derive(Debug, Clone, Copy)]
pub struct Number<'a> {
    pub field: Field,
    /// Decimals shown at least and at most.
    pub decimals: (usize, usize),
    /// Shown after the field ("mm"); empty for none.
    pub unit: &'a str,
    pub width: f32,
    /// Hexadecimal digits (a USB id): no calculation, digits only.
    pub hex: Option<usize>,
    /// A value outside the range is clamped into it instead of refused.
    pub clamp: bool,
}

impl<'a> Number<'a> {
    pub const fn new(field: Field) -> Self {
        Number {
            field,
            decimals: (0, 2),
            unit: "",
            width: 48.0,
            hex: None,
            clamp: false,
        }
    }

    pub const fn decimals(mut self, min: usize, max: usize) -> Self {
        self.decimals = (min, max);
        self
    }

    pub const fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub const fn hex(digits: usize) -> Self {
        let mut n = Self::new(Field::plain().integer());
        n.hex = Some(digits);
        n
    }

    pub const fn clamp(mut self) -> Self {
        self.clamp = true;
        self
    }

    /// The text of `v`.
    pub fn show(&self, v: f64) -> String {
        if let Some(digits) = self.hex {
            return format!("{:0digits$X}", v.max(0.0) as u64);
        }
        let (min, max) = if self.field.integer {
            (0, 0)
        } else {
            self.decimals
        };
        let s = format!("{v:.max$}");
        let mut s = s.as_str();
        if max > min && s.contains('.') {
            let dot = s.find('.').unwrap_or(s.len());
            while s.len() > dot + 1 + min && s.ends_with('0') {
                s = &s[..s.len() - 1];
            }
            s = s.trim_end_matches('.');
        }
        if s == "-0" { "0".into() } else { s.into() }
    }

    /// Reads `text` against the value of now: the value, or the message.
    pub fn read(&self, text: &str, current: f64) -> Result<f64, String> {
        self.read_raw(text, current).map(|(_, v)| v)
    }

    /// [`Number::read`], with the value before a clamping field clamps it.
    fn read_raw(&self, text: &str, current: f64) -> Result<(f64, f64), String> {
        if let Some(digits) = self.hex {
            return read_hex(text, digits, self.field.max).map(|v| (v, v));
        }
        if self.clamp {
            let (lo, hi) = (self.field.min, self.field.max);
            let free = self.field.range(f64::NEG_INFINITY, f64::INFINITY);
            return free
                .eval(text, Some(current))
                .map(|v| (v, v.clamp(lo, hi)))
                .map_err(|e| e.message);
        }
        self.field
            .eval(text, Some(current))
            .map(|v| (v, v))
            .map_err(|e| e.message)
    }
}

/// A hexadecimal number of at most `digits` digits, an optional `0x` in
/// front.
fn read_hex(text: &str, digits: usize, max: f64) -> Result<f64, String> {
    let t = text.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    let why = || {
        format!(
            "Can't read '{}': type up to {digits} hex digits, 0-9 and A-F.",
            text.trim()
        )
    };
    if t.is_empty() || t.chars().count() > digits || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(why());
    }
    let v = u64::from_str_radix(t, 16).map_err(|_| why())? as f64;
    if v > max {
        return Err(why());
    }
    Ok(v)
}

/// The typing going on in a field.
#[derive(Clone)]
struct Edit {
    text: String,
    /// The text it showed when the typing began.
    shown: String,
    /// The value then, for a relative edit.
    start: f64,
    pass: u64,
}

/// A number field with a stable id from `salt`: shows `value`, takes a
/// number or a calculation (see the module text) and sets `value` when the
/// typed text counts. Returns the field and whether `value` changed.
pub fn number<T: Numeric>(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug,
    enabled: bool,
    value: &mut T,
    spec: &Number<'_>,
) -> (Response, bool) {
    number_seen(ui, salt, enabled, value, spec, |_| {})
}

/// [`number`], with `seen` told each value that was read, before a
/// clamping field clamps it.
pub fn number_seen<T: Numeric>(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug,
    enabled: bool,
    value: &mut T,
    spec: &Number<'_>,
    seen: impl Fn(f64),
) -> (Response, bool) {
    let mut spec = *spec;
    if T::INTEGRAL {
        spec.field.integer = true;
        spec.field.min = spec.field.min.max(T::MIN.to_f64());
        spec.field.max = spec.field.max.min(T::MAX.to_f64());
    }
    let id = ui.make_persistent_id(("flatcraft-number", salt));
    let ctx = ui.ctx().clone();
    let pass = ctx.cumulative_pass_nr();
    let current = value.to_f64();
    let shown = spec.show(current);
    let edit_key = id.with("edit");
    // An edit of a field that was not drawn last pass is over.
    let mut edit = ctx
        .data(|d| d.get_temp::<Edit>(edit_key))
        .filter(|e| e.pass + 1 >= pass);
    // Nothing typed yet: the field follows the value.
    if let Some(e) = edit.as_mut()
        && e.text == e.shown
        && e.shown != shown
    {
        e.text = shown.clone();
        e.shown = shown.clone();
        e.start = current;
    }
    let mut text = edit
        .as_ref()
        .map_or_else(|| shown.clone(), |e| e.text.clone());
    // As wide as its text, so no digit is cut.
    let width = spec.width.max(super::ribbon::field_width(ui, &text));
    let resp = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = metrics::SPACE_4;
            let r = ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(&mut text)
                    .id(id)
                    .margin(egui::Margin::symmetric(
                        metrics::FIELD_MARGIN.x as i8,
                        metrics::FIELD_MARGIN.y as i8,
                    ))
                    .desired_width(width)
                    .min_size(egui::vec2(width, 0.0)),
            );
            if !spec.unit.is_empty() {
                ui.label(egui::RichText::new(spec.unit).weak());
            }
            r
        })
        .inner;
    select_whole_value(ui, &resp, &text);
    if resp.has_focus() && edit.is_none() {
        edit = Some(Edit {
            text: text.clone(),
            shown: shown.clone(),
            start: current,
            pass,
        });
    }
    let mut changed = false;
    if let Some(e) = edit.as_mut() {
        e.text = text;
        e.pass = pass;
    }
    if let Some(e) = edit.clone()
        && !resp.has_focus()
    {
        edit = None;
        let (enter, escape) = keys(&ctx);
        if escape || e.text.trim() == e.shown.trim() {
            set_message(&ctx, id, None);
        } else {
            match spec.read_raw(&e.text, e.start) {
                Ok((raw, v)) => {
                    seen(raw);
                    *value = T::from_f64(v);
                    changed = value.to_f64() != current;
                    set_message(&ctx, id, None);
                }
                Err(msg) if enter && enabled => {
                    // Enter: still in the field, to fix the text.
                    ctx.memory_mut(|m| m.request_focus(id));
                    set_message(&ctx, id, Some(kept(&msg)));
                    edit = Some(e);
                }
                Err(msg) => {
                    set_message(&ctx, id, None);
                    toast(&ctx, dropped(&msg));
                }
            }
        }
    }
    match &edit {
        Some(e) => {
            ctx.data_mut(|d| {
                d.insert_temp(edit_key, e.clone());
                d.insert_temp(editing_id(), Editing { id, pass });
            });
        }
        None => {
            ctx.data_mut(|d| {
                d.remove::<Edit>(edit_key);
                if d.get_temp::<Editing>(editing_id())
                    .is_some_and(|x| x.id == id)
                {
                    d.remove::<Editing>(editing_id());
                }
            });
        }
    }
    match message(&ctx, id) {
        Some(msg) => {
            // Drawn in this pass: it stays for the next one.
            set_message(&ctx, id, Some(msg.clone()));
            show_message(ui, id, resp.rect, &msg);
        }
        None => set_message(&ctx, id, None),
    }
    (resp, changed)
}

/// A slider with a [`number`] field beside it for its value (the slider's
/// own value box is off). A value outside `range` is refused.
#[allow(clippy::too_many_arguments)]
pub fn slider<T: Numeric>(
    ui: &mut Ui,
    salt: impl Hash + std::fmt::Debug,
    enabled: bool,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    spec: &Number<'_>,
) -> (Response, Response, bool) {
    let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
    let before = value.to_f64();
    let track = ui.add_enabled(enabled, egui::Slider::new(value, range).show_value(false));
    let mut spec = *spec;
    spec.field = spec.field.range(lo, hi);
    let (field, typed) = number(ui, salt, enabled, value, &spec);
    let changed = typed || value.to_f64() != before;
    (track, field, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_show_their_decimals() {
        let n = Number::new(Field::length());
        assert_eq!(n.show(5.0), "5");
        assert_eq!(n.show(5.25), "5.25");
        assert_eq!(n.show(5.256), "5.26");
        assert_eq!(n.decimals(2, 3).show(5.0), "5.00");
        assert_eq!(n.decimals(1, 1).show(5.0), "5.0");
        assert_eq!(Number::new(Field::plain().integer()).show(3.0), "3");
        assert_eq!(n.show(-0.0), "0");
        assert_eq!(Number::hex(4).show(2989.0), "0BAD");
    }

    #[test]
    fn hex_fields_read_hex_digits_only() {
        let n = Number::hex(4);
        let n = Number {
            field: n.field.range(0.0, 65535.0),
            ..n
        };
        assert_eq!(n.read("0bad", 0.0), Ok(2989.0));
        assert_eq!(n.read("0x1A", 0.0), Ok(26.0));
        assert!(n.read("1+1", 0.0).is_err());
        assert!(n.read("12345", 0.0).is_err());
        assert!(n.read("", 0.0).is_err());
        let e = n.read("zz", 0.0).unwrap_err();
        assert!(e.contains("hex digits"), "{e}");
    }

    #[test]
    fn a_clamping_field_clamps() {
        let n = Number::new(Field::plain().integer().range(1.0, 10.0)).clamp();
        assert_eq!(n.read("11", 3.0), Ok(10.0));
        assert_eq!(n.read("+=2", 3.0), Ok(5.0));
        assert!(n.read("2+", 3.0).is_err());
    }
}
