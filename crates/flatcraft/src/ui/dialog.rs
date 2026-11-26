//! The one dialog every FlatCraft question uses: a round badge on the
//! left (warning, info or error), the title as a short question or
//! statement, a × at the top right, one or two plain sentences, and a
//! footer under a line with a secondary (often destructive) action on the
//! left and Cancel plus the primary action on the right.
//!
//! Keys: Enter runs the primary action (Cancel when there is none),
//! Escape and the × cancel, and Cmd+D (Ctrl+D elsewhere) runs the
//! secondary action when the dialog says so ("Don't Save"). Escape never
//! runs anything destructive. At most one answer comes back per frame: a
//! button that has the keyboard focus wins over Enter.
//!
//! Error dialogs start their title with "Great Scott!", unless the error
//! is a problem on the machine (a stopped cut, a failed write to the
//! plotter): there the title stays plain, because the person may need to
//! act fast.

use egui::{Color32, Key, Modifiers, RichText, Ui};

use super::fonts;
use super::groups::card;
use super::icons::{self, Icon};
use super::settings::set_name;
use super::theme::Tokens;

/// The prefix of an error dialog's title.
pub const GREAT_SCOTT: &str = "Great Scott!";
/// The accessible name of the × at the top right.
pub const CLOSE_NAME: &str = "Close dialog";

/// The badge, and what kind of dialog it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Warning,
    /// Something went wrong in FlatCraft or with a file.
    Error,
    /// Something went wrong on the machine (the plotter, a cut). The title
    /// stays plain.
    MachineError,
}

impl Kind {
    /// The badge's accessible name.
    pub fn badge_name(self) -> &'static str {
        match self {
            Kind::Info => "Info",
            Kind::Warning => "Warning",
            Kind::Error | Kind::MachineError => "Error",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Kind::Info => Icon::Info,
            Kind::Warning => Icon::Warning,
            Kind::Error | Kind::MachineError => Icon::Error,
        }
    }

    fn color(self, t: &Tokens) -> Color32 {
        match self {
            Kind::Info => t.accent_prepare,
            Kind::Warning => t.warn,
            Kind::Error | Kind::MachineError => t.danger,
        }
    }
}

/// The answer to a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Primary,
    Secondary,
    Cancel,
}

/// What a dialog shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Dialog {
    /// Keeps the dialog's state apart from other dialogs.
    pub id: &'static str,
    /// What this question is about (a document id), so two questions with
    /// the same title (two documents with the same name) never share
    /// their key state.
    pub subject: u64,
    pub kind: Kind,
    pub title: String,
    /// One or two plain sentences, one line each.
    pub body: Vec<String>,
    /// The action on the left of the footer, if any.
    pub secondary: Option<String>,
    /// Cmd+D (Ctrl+D) runs the secondary action ("Don't Save").
    pub secondary_key: bool,
    /// The action on the right, in the accent colour.
    pub primary: Option<String>,
    /// Why the primary action cannot run now (a Rename needs a name):
    /// the button is dimmed and its card says this. `None`: it can run.
    pub primary_blocked: Option<String>,
    pub cancel: String,
    /// What the primary button does, for its hover card.
    pub primary_hint: String,
    /// What the secondary button does, for its hover card.
    pub secondary_hint: String,
    /// What Cancel does, for its hover card.
    pub cancel_hint: String,
}

/// The card of Cancel when the dialog says nothing else.
pub const CANCEL_HINT: &str = "Close this question without doing anything.";

impl Dialog {
    /// A dialog with a title and its body, a Cancel button and nothing
    /// else yet.
    pub fn new(id: &'static str, kind: Kind, title: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            id,
            subject: 0,
            kind,
            title: Self::titled(kind, &title),
            body: Vec::new(),
            secondary: None,
            secondary_key: false,
            primary: None,
            primary_blocked: None,
            cancel: "Cancel".into(),
            primary_hint: String::new(),
            secondary_hint: String::new(),
            cancel_hint: CANCEL_HINT.into(),
        }
    }

    /// The title a dialog of `kind` shows for `title`: an error in
    /// FlatCraft or with a file starts with "Great Scott!", a problem on
    /// the machine never does.
    pub fn titled(kind: Kind, title: &str) -> String {
        match kind {
            Kind::Error if !title.starts_with(GREAT_SCOTT) => format!("{GREAT_SCOTT} {title}"),
            _ => title.to_string(),
        }
    }

    /// What the question is about (see [`Dialog::subject`]).
    pub fn about(mut self, subject: u64) -> Self {
        self.subject = subject;
        self
    }

    pub fn body(mut self, line: impl Into<String>) -> Self {
        self.body.push(line.into());
        self
    }

    pub fn primary(mut self, label: impl Into<String>) -> Self {
        self.primary = Some(label.into());
        self
    }

    /// Dims the primary action with this reason (`None` lets it run).
    pub fn primary_blocked(mut self, why: Option<impl Into<String>>) -> Self {
        self.primary_blocked = why.map(Into::into);
        self
    }

    /// What the primary button does, for its hover card.
    pub fn primary_hint(mut self, hint: impl Into<String>) -> Self {
        self.primary_hint = hint.into();
        self
    }

    /// What the secondary button does, for its hover card.
    pub fn secondary_hint(mut self, hint: impl Into<String>) -> Self {
        self.secondary_hint = hint.into();
        self
    }

    /// What Cancel does, for its hover card.
    pub fn cancel_hint(mut self, hint: impl Into<String>) -> Self {
        self.cancel_hint = hint.into();
        self
    }

    /// The left action. With `key`, Cmd+D (Ctrl+D) runs it.
    pub fn secondary(mut self, label: impl Into<String>, key: bool) -> Self {
        self.secondary = Some(label.into());
        self.secondary_key = key;
        self
    }

    pub fn cancel(mut self, label: impl Into<String>) -> Self {
        self.cancel = label.into();
        self
    }

    /// Shows the dialog with `content` under the body (a name field, say)
    /// and returns the answer of this frame, if any.
    pub fn show(
        &self,
        ctx: &egui::Context,
        t: &Tokens,
        content: impl FnOnce(&mut Ui),
    ) -> Option<Answer> {
        self.show_with(ctx, t, |ui| {
            content(ui);
            self.primary_blocked.clone()
        })
    }

    /// Like [`Self::show`], with `content` returning why the primary
    /// action cannot run (`None` when it can), read after it drew (a name
    /// field typed this frame). The dimmed button's card gives the reason.
    pub fn show_with(
        &self,
        ctx: &egui::Context,
        t: &Tokens,
        content: impl FnOnce(&mut Ui) -> Option<String>,
    ) -> Option<Answer> {
        // Keys count only once every key that was down when this question
        // appeared is up again: a key still held (or repeating) from the
        // answer before never answers this one.
        let armed_id =
            egui::Id::unique(("flatcraft-dialog-armed", self.id, self.subject, &self.title));
        let armed = ctx.data(|d| d.get_temp::<bool>(armed_id)).unwrap_or(false);
        let keys_up = ctx.input(|i| i.keys_down.is_empty());
        if !armed && keys_up {
            ctx.data_mut(|d| d.insert_temp(armed_id, true));
        }
        // The previous question's arming is gone once another title shows.
        let last_id = egui::Id::unique(("flatcraft-dialog-last", self.id));
        let last: Option<egui::Id> = ctx.data(|d| d.get_temp(last_id));
        if last != Some(armed_id) {
            if let Some(old) = last {
                ctx.data_mut(|d| d.remove::<bool>(old));
            }
            ctx.data_mut(|d| d.insert_temp(last_id, armed_id));
        }
        let mut primary_blocked = self.primary_blocked.clone();
        let mut clicked: Option<Answer> = None;
        let frame = egui::Frame::new()
            .fill(t.panel)
            .stroke(egui::Stroke::new(1.0, t.border))
            .corner_radius(t.ribbon_radius * 3)
            .inner_margin(0);
        let modal = egui::Modal::new(egui::Id::unique(("flatcraft-dialog", self.id)))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(WIDTH);
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(PAD as i8))
                    .show(ui, |ui| {
                        ui.horizontal_top(|ui| {
                            badge(ui, self.kind, t);
                            ui.add_space(t.spacing * 1.5);
                            ui.vertical(|ui| {
                                ui.set_width(WIDTH - 2.0 * PAD - BADGE - t.spacing * 1.5);
                                // The title leaves room at the right for the ×.
                                ui.scope(|ui| {
                                    ui.set_max_width(ui.available_width() - 28.0);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&self.title)
                                                .font(fonts::semibold(15.0))
                                                .color(t.text),
                                        )
                                        .wrap(),
                                    );
                                });
                                ui.add_space(t.spacing / 2.0);
                                for line in &self.body {
                                    ui.add(
                                        egui::Label::new(RichText::new(line).color(t.text_dim))
                                            .wrap(),
                                    );
                                }
                                primary_blocked = content(ui);
                            });
                        });
                    });
                // The × at the top right, over the content.
                let max = ui.min_rect();
                let x_rect = egui::Rect::from_center_size(
                    egui::pos2(max.right() - PAD - 6.0, max.top() + PAD + 6.0),
                    egui::Vec2::splat(22.0),
                );
                let x = ui.place(
                    x_rect,
                    egui::Button::image(
                        icon_image(ui, Icon::Close, 14.0, t.text_dim).alt_text(CLOSE_NAME),
                    )
                    .frame_when_inactive(false),
                );
                set_name(ui, &x, CLOSE_NAME);
                if x.clicked() {
                    clicked = Some(Answer::Cancel);
                }
                // The footer under a line.
                let line_y = ui.cursor().top();
                let footer_bg = ui.painter().add(egui::Shape::Noop);
                let footer = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(PAD as i8, (PAD / 1.5) as i8))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 2.0 * PAD);
                        let (left, right) = egui::Sides::new().show(
                            ui,
                            |ui| {
                                let label = self.secondary.as_ref()?;
                                let color = if self.kind == Kind::Info {
                                    t.text
                                } else {
                                    t.danger
                                };
                                let b = egui::Button::new(RichText::new(label).color(color));
                                let r = ui.add(b);
                                card(&r, label, &self.secondary_hint, None, false);
                                r.clicked().then_some(Answer::Secondary)
                            },
                            |ui| {
                                let mut out = None;
                                // The accent fill with a text that keeps
                                // its contrast in both themes.
                                let (fill, ink) = if t.is_dark() {
                                    (t.accent_prepare, t.bg)
                                } else {
                                    (t.icon_accent, Color32::WHITE)
                                };
                                // Right to left: the primary action is last.
                                let primary = self.primary.as_ref();
                                if let Some(label) = primary {
                                    let b =
                                        egui::Button::new(RichText::new(label).color(ink).strong())
                                            .fill(fill);
                                    let r = ui.add_enabled(primary_blocked.is_none(), b);
                                    card(
                                        &r,
                                        label,
                                        &self.primary_hint,
                                        primary_blocked.as_deref(),
                                        false,
                                    );
                                    if r.clicked() {
                                        out = Some(Answer::Primary);
                                    }
                                }
                                let cancel = if primary.is_none() {
                                    // Cancel is the default answer then.
                                    egui::Button::new(
                                        RichText::new(&self.cancel).color(ink).strong(),
                                    )
                                    .fill(fill)
                                } else {
                                    egui::Button::new(&self.cancel)
                                };
                                let r = ui.add(cancel);
                                card(&r, &self.cancel, &self.cancel_hint, None, false);
                                if r.clicked() {
                                    out = Some(Answer::Cancel);
                                }
                                out
                            },
                        );
                        if let Some(a) = left.or(right) {
                            clicked = Some(a);
                        }
                    });
                // The footer's band across the whole dialog, under a line;
                // the bottom corners follow the dialog's.
                let band = egui::Rect::from_x_y_ranges(
                    max.x_range(),
                    line_y..=footer.response.rect.bottom(),
                );
                let r = t.ribbon_radius * 3;
                ui.painter().set(
                    footer_bg,
                    egui::Shape::rect_filled(
                        band,
                        egui::CornerRadius {
                            nw: 0,
                            ne: 0,
                            sw: r,
                            se: r,
                        },
                        t.rail,
                    ),
                );
                ui.painter()
                    .hline(max.x_range(), line_y, egui::Stroke::new(1.0, t.border));
            });
        if clicked.is_some() {
            return clicked;
        }
        // A click outside is Cancel, like the ×.
        if modal.backdrop_response.clicked() {
            return Some(Answer::Cancel);
        }
        // Keys only for the dialog on top, and only once armed.
        if !armed || !modal.is_top_modal {
            return None;
        }
        // Cmd+D (Ctrl+D): only a fresh press, never a repeat of a held key.
        let key_d = self.secondary.is_some()
            && self.secondary_key
            && ctx.input_mut(|i| take_fresh_press(i, Modifiers::COMMAND, Key::D));
        if key_d {
            return Some(Answer::Secondary);
        }
        // Escape: Cancel. Never the destructive action.
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            return Some(Answer::Cancel);
        }
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        if enter {
            return match &self.primary {
                Some(_) if primary_blocked.is_none() => Some(Answer::Primary),
                Some(_) => None,
                None => Some(Answer::Cancel),
            };
        }
        None
    }
}

/// Takes a press of `key` with `modifiers` that is not a key repeat out of
/// this frame's events. Repeats of it are taken out too, and ignored.
fn take_fresh_press(i: &mut egui::InputState, modifiers: Modifiers, key: Key) -> bool {
    let mut fresh = false;
    i.events.retain(|e| match e {
        egui::Event::Key {
            key: k,
            pressed: true,
            repeat,
            modifiers: m,
            ..
        } if *k == key && m.matches_logically(modifiers) => {
            fresh |= !repeat;
            false
        }
        _ => true,
    });
    fresh
}

/// Width of every dialog.
const WIDTH: f32 = 440.0;
/// Room inside the dialog's edge.
const PAD: f32 = 18.0;
/// Size of the round badge.
const BADGE: f32 = 36.0;

fn icon_image(ui: &Ui, icon: Icon, size: f32, tint: Color32) -> egui::Image<'static> {
    let tex = icons::texture_colored(ui.ctx(), icon, size, tint);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(size),
    ))
}

/// The round badge: a tinted circle with the kind's icon in its colour.
fn badge(ui: &mut Ui, kind: Kind, t: &Tokens) {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(BADGE), egui::Sense::hover());
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Image, true, kind.badge_name())
    });
    let color = kind.color(t);
    ui.painter()
        .circle_filled(rect.center(), BADGE / 2.0, color.gamma_multiply(0.18));
    let icon = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0));
    icon_image(ui, kind.icon(), 20.0, color).paint_at(ui, icon);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_great_scott_but_machine_errors_never_do() {
        let d = Dialog::new("t", Kind::Error, "The file could not be opened.");
        assert_eq!(d.title, "Great Scott! The file could not be opened.");
        // Never twice.
        assert_eq!(
            Dialog::titled(Kind::Error, "Great Scott! It broke."),
            "Great Scott! It broke."
        );
        for title in ["The cut stopped", "The plotter did not take the job"] {
            let d = Dialog::new("t", Kind::MachineError, title);
            assert_eq!(d.title, title);
            assert!(!d.title.contains(GREAT_SCOTT));
        }
        for kind in [Kind::Info, Kind::Warning] {
            assert_eq!(Dialog::titled(kind, "Save changes?"), "Save changes?");
        }
    }
}
