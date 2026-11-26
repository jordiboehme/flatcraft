//! Number fields that take a calculation (`100/3`, `2in`, `+=10`), read by
//! [`flatcraft_core::numexpr`]: the hover card hint, the red outline and
//! the message under a field that could not read its text, and
//! [`drag`], a `DragValue` that reads calculations.

use std::cell::RefCell;

use egui::emath::Numeric;
use egui::{Response, Ui};
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

/// Where a field's last message is kept.
fn message_id(id: egui::Id) -> egui::Id {
    id.with("flatcraft-calc-message")
}

/// The message a field shows, if any.
pub fn message(ctx: &egui::Context, id: egui::Id) -> Option<String> {
    ctx.data(|d| d.get_temp::<String>(message_id(id)))
}

fn set_message(ctx: &egui::Context, id: egui::Id, msg: Option<String>) {
    ctx.data_mut(|d| match msg {
        Some(m) => {
            d.insert_temp(message_id(id), m);
        }
        None => d.remove::<String>(message_id(id)),
    });
}

/// Paints a red outline around a field's `rect` and `msg` just under it,
/// above everything else so the layout does not move.
pub fn show_message(ui: &Ui, id: egui::Id, rect: egui::Rect, msg: &str) {
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

/// A `DragValue` that reads what is typed with [`Field::eval`]: a
/// calculation, units of `field`'s kind, a relative edit from the value of
/// now. Whole-number types round and keep their limits. A text it cannot
/// read (or a value outside `field`'s range) changes nothing, and the
/// field shows why under it until it is edited again or Esc is pressed.
/// The typed value counts when the field loses the focus; Esc puts the old
/// one back (as egui does). `style` sets speed, decimals and suffix.
pub fn drag<T: Numeric>(
    ui: &mut Ui,
    enabled: bool,
    value: &mut T,
    field: Field,
    style: impl for<'a> FnOnce(egui::DragValue<'a>) -> egui::DragValue<'a>,
) -> Response {
    drag_seen(ui, enabled, value, field, style, |_| {})
}

/// [`drag`], with `seen` told every value that was read (before the
/// `DragValue`'s own range clamps it).
pub fn drag_seen<T: Numeric>(
    ui: &mut Ui,
    enabled: bool,
    value: &mut T,
    field: Field,
    style: impl for<'a> FnOnce(egui::DragValue<'a>) -> egui::DragValue<'a>,
    seen: impl Fn(f64),
) -> Response {
    let reader = Reader::new::<T>(field, value.to_f64(), &seen);
    let resp = {
        let d = style(egui::DragValue::new(value))
            .update_while_editing(false)
            .custom_parser(|t| reader.read(t));
        ui.add_enabled(enabled, d)
    };
    reader.finish(ui, &resp);
    resp
}

/// A `Slider` whose value box reads calculations like [`drag`]; a value
/// outside the slider's range is refused, not clamped.
pub fn slider<T: Numeric>(
    ui: &mut Ui,
    enabled: bool,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    field: Field,
    style: impl for<'a> FnOnce(egui::Slider<'a>) -> egui::Slider<'a>,
) -> Response {
    let field = field.range(range.start().to_f64(), range.end().to_f64());
    let reader = Reader::new::<T>(field, value.to_f64(), &|_| {});
    let resp = {
        let s = style(egui::Slider::new(value, range))
            .update_while_editing(false)
            .custom_parser(|t| reader.read(t));
        ui.add_enabled(enabled, s)
    };
    reader.finish(ui, &resp);
    resp
}

/// Reads the typed text of one field in one frame and remembers how it
/// went.
struct Reader<'s> {
    field: Field,
    current: f64,
    seen: &'s dyn Fn(f64),
    outcome: RefCell<Option<Result<(), String>>>,
}

impl<'s> Reader<'s> {
    /// A whole-number type rounds and keeps its limits.
    fn new<T: Numeric>(field: Field, current: f64, seen: &'s dyn Fn(f64)) -> Self {
        let mut field = field;
        if T::INTEGRAL {
            field.integer = true;
            field.min = field.min.max(T::MIN.to_f64());
            field.max = field.max.min(T::MAX.to_f64());
        }
        Reader {
            field,
            current,
            seen,
            outcome: RefCell::new(None),
        }
    }

    fn read(&self, text: &str) -> Option<f64> {
        match self.field.eval(text, Some(self.current)) {
            Ok(v) => {
                (self.seen)(v);
                *self.outcome.borrow_mut() = Some(Ok(()));
                Some(v)
            }
            Err(e) => {
                *self.outcome.borrow_mut() = Some(Err(e.message));
                None
            }
        }
    }

    fn finish(self, ui: &Ui, resp: &Response) {
        after_edit(ui, resp, self.outcome.into_inner());
    }
}

/// Keeps or clears a field's message after its frame: a failed read sets
/// it; a good one, the focus (editing again) or Esc clear it.
fn after_edit(ui: &Ui, resp: &Response, outcome: Option<Result<(), String>>) {
    let ctx = ui.ctx();
    match outcome {
        Some(Err(msg)) => set_message(ctx, resp.id, Some(msg)),
        Some(Ok(())) => set_message(ctx, resp.id, None),
        None => {
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if resp.has_focus() || escape {
                set_message(ctx, resp.id, None);
            }
        }
    }
    select_on_double_click(ui, resp);
    if let Some(msg) = message(ctx, resp.id) {
        show_message(ui, resp.id, resp.rect, &msg);
    }
}

/// A double-click while a `DragValue` is being typed into selects its
/// whole value (egui selects it when the field gets the focus, by a
/// click or Tab, already).
fn select_on_double_click(ui: &Ui, resp: &Response) {
    if !(resp.double_clicked() && resp.has_focus()) {
        return;
    }
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
        // The end is clamped to the text's length.
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(usize::MAX / 2),
            )));
        state.store(ui.ctx(), resp.id);
    }
}
