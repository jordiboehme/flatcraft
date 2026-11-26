//! Short messages at the bottom left of the canvas in Prepare and Cut
//! (the bottom-right corner of the window elsewhere), stacked with the
//! newest at the bottom. Over the canvas they never cover a control of
//! the side panels, like Send or the rating of a cut. Each card has a coloured status dot, a title and an
//! optional detail line (a full path, wrapped, never cut), and a × at the
//! top right with a thin ring that shrinks over the toast's lifetime.
//!
//! Info and success toasts stay 5 s, warnings 8 s, errors until closed
//! (no ring). Hovering a toast pauses its clock. A toast with a file
//! (a saved project, a written job) shows that file in Finder or Explorer
//! when clicked. With reduced motion the ring is not drawn; the lifetime
//! stays the same.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Color32, RichText};

use super::icons::{self, Icon};
use super::theme::Tokens;

/// How long an info or a success toast stays visible.
pub const LIFETIME: Duration = Duration::from_secs(5);
/// How long a warning stays visible.
pub const WARNING_LIFETIME: Duration = Duration::from_secs(8);

/// Width of a toast card.
pub const WIDTH: f32 = 340.0;
/// The hover card of a toast with a file.
pub const REVEAL_HINT: &str = "Click to show the file in your file manager.";
/// The accessible name of a toast's × button.
pub const CLOSE_NAME: &str = "Close message";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    fn color(self, t: &Tokens) -> Color32 {
        match self {
            ToastKind::Info => t.accent_prepare,
            ToastKind::Success => t.ok,
            ToastKind::Warning => t.warn,
            ToastKind::Error => t.danger,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    /// Stable for the toast's life: its widgets keep their ids when a
    /// toast above it goes away.
    pub id: u64,
    /// The title.
    pub text: String,
    /// The line under the title, if any (a full path, say).
    pub detail: Option<String>,
    pub kind: ToastKind,
    pub created: Instant,
    /// A file a click on the toast shows in the file manager.
    pub reveal: Option<PathBuf>,
    /// How long the pointer rested on it before, which does not count.
    pub paused: Duration,
    /// The pointer rests on it since then.
    pub hovered_since: Option<Instant>,
}

impl Toast {
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(text, ToastKind::Info)
    }

    pub fn success(text: impl Into<String>) -> Self {
        Self::new(text, ToastKind::Success)
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(text, ToastKind::Warning)
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(text, ToastKind::Error)
    }

    fn new(text: impl Into<String>, kind: ToastKind) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            text: text.into(),
            detail: None,
            kind,
            created: Instant::now(),
            reveal: None,
            paused: Duration::ZERO,
            hovered_since: None,
        }
    }

    /// Adds the line under the title.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// A click on the toast shows `file` in the file manager.
    pub fn with_reveal(mut self, file: PathBuf) -> Self {
        self.reveal = Some(file);
        self
    }

    /// How long it stays; `None` for an error, which stays until closed.
    pub fn lifetime(&self) -> Option<Duration> {
        match self.kind {
            ToastKind::Info | ToastKind::Success => Some(LIFETIME),
            ToastKind::Warning => Some(WARNING_LIFETIME),
            ToastKind::Error => None,
        }
    }

    /// How long it has been shown at `now`, without the time the pointer
    /// rested on it.
    pub fn age(&self, now: Instant) -> Duration {
        let resting = self
            .hovered_since
            .map_or(Duration::ZERO, |h| now.saturating_duration_since(h));
        now.saturating_duration_since(self.created)
            .saturating_sub(self.paused + resting)
    }

    /// Time left at `now`; `None` for a toast that stays.
    pub fn left(&self, now: Instant) -> Option<Duration> {
        self.lifetime().map(|l| l.saturating_sub(self.age(now)))
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.left(now).is_some_and(|l| l.is_zero())
    }

    /// The pointer rests on it from `now` (`true`) or left it.
    pub fn set_hovered(&mut self, hovered: bool, now: Instant) {
        match (hovered, self.hovered_since) {
            (true, None) => self.hovered_since = Some(now),
            (false, Some(since)) => {
                self.paused += now.saturating_duration_since(since);
                self.hovered_since = None;
            }
            _ => {}
        }
    }

    /// Its clock runs now: it has a lifetime and the pointer is elsewhere.
    pub fn running(&self) -> bool {
        self.lifetime().is_some() && self.hovered_since.is_none()
    }
}

/// Removes the expired toasts and returns how long until the next one
/// expires, if any has a running clock.
pub fn prune(toasts: &mut Vec<Toast>, now: Instant) -> Option<Duration> {
    toasts.retain(|t| !t.expired(now));
    toasts
        .iter()
        .filter(|t| t.running())
        .filter_map(|t| t.left(now))
        .min()
}

/// What the person did with the toasts this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Show this file in the file manager.
    Reveal(PathBuf),
    /// Close the toast with this id.
    Close(u64),
}

/// Where the notices go: the canvas, and what already sits on it that
/// they must leave free: the import report card (`report`, top right),
/// the status pills (`pills`, bottom left) and the keep pick bar (`bar`,
/// bottom right).
#[derive(Debug, Clone, Copy)]
pub struct Place {
    pub canvas: egui::Rect,
    pub report: Option<egui::Rect>,
    pub pills: Option<egui::Rect>,
    pub bar: Option<egui::Rect>,
}

/// The smallest height the stack needs for one notice. With less room
/// over the canvas the notices fold into one line ([`folded_label`]).
pub const MIN_STACK_HEIGHT: f32 = 48.0;
/// The narrowest a card gets on a narrow canvas.
pub const MIN_WIDTH: f32 = 160.0;

/// The one-line button the notices fold into on a canvas too short for a
/// card: "1 notice", "3 notices". A click opens them.
pub fn folded_label(n: usize) -> String {
    if n == 1 {
        "1 notice".to_string()
    } else {
        format!("{n} notices")
    }
}

/// The card of the folded line.
pub const FOLDED_HINT: &str = "The canvas is too short to show the notices here. Click to open them over the canvas; click Hide to fold them again.";
/// The button that folds opened notices again.
pub const FOLD_LABEL: &str = "Hide notices";
/// The card of [`FOLD_LABEL`].
pub const FOLD_HINT: &str = "Folds the notices back into one line, so the canvas is free again.";

fn rect_id() -> egui::Id {
    egui::Id::unique("flatcraft-toasts-rect")
}

fn open_id() -> egui::Id {
    egui::Id::unique("flatcraft-toasts-open")
}

/// Where the notice stack stood last frame over the canvas, if it shows
/// there: the Cut canvas moves its zoom hint above it.
pub fn stack_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp(rect_id()))
}

/// Draws the toasts, oldest at the top, and tracks which one the pointer
/// rests on: at the bottom left of the canvas of `place` (above the
/// status pills) when there is one, else in the window's bottom-right
/// corner. Designs mostly sit at the top left of the mat, so the bottom
/// left covers the least. Over a canvas the stack never reaches past it,
/// onto the keep pick bar or onto the import report card: a taller stack
/// scrolls, with the newest notice in view. `reduce_motion` leaves out
/// the rings. Returns what was clicked.
pub fn show(
    ctx: &egui::Context,
    toasts: &mut [Toast],
    t: &Tokens,
    now: Instant,
    reduce_motion: bool,
    place: Option<Place>,
) -> Vec<Event> {
    let mut events = Vec::new();
    if toasts.is_empty() {
        ctx.data_mut(|d| {
            d.remove::<egui::Rect>(rect_id());
            d.remove::<bool>(open_id());
        });
        return events;
    }
    let gap = t.spacing * 2.0;
    let mut width = WIDTH;
    // Folded into one line: the canvas is too short for a card.
    let mut folded = false;
    // Less room than one card needs between what sits on the canvas.
    let mut cramped = false;
    let opened: bool = ctx.data(|d| d.get_temp(open_id())).unwrap_or(false);
    let area = egui::Area::new(egui::Id::unique("flatcraft-toasts"));
    let (area, max_height) = match place.filter(|p| p.canvas.is_positive()) {
        Some(p) => {
            let r = p.canvas;
            let left = r.left() + super::canvas::RULER + gap;
            // Never wider than the canvas: the side panels stay free.
            width = WIDTH.min(r.right() - gap - left).max(MIN_WIDTH);
            let right = left + width;
            let across = |o: &egui::Rect| o.right() > left && o.left() < right;
            let mut bottom = r.bottom() - gap;
            for o in [p.pills.filter(|o| o.height() >= 1.0), p.bar]
                .into_iter()
                .flatten()
                .filter(across)
            {
                bottom = bottom.min(o.top() - t.spacing);
            }
            let top = p
                .report
                .filter(across)
                .map_or(r.top() + super::canvas::RULER + gap, |o| {
                    o.bottom() + t.spacing
                });
            let room = bottom - top;
            cramped = room < MIN_STACK_HEIGHT;
            folded = cramped && !opened;
            if cramped && opened {
                // Opened over the canvas on purpose: from its bottom up to
                // its top (over the ruler if need be), the Hide button
                // included, never past the top.
                let bottom = r.bottom() - gap;
                let button = ctx.global_style().spacing.interact_size.y + t.spacing;
                let full = bottom - (r.top() + t.spacing) - button;
                (
                    area.pivot(egui::Align2::LEFT_BOTTOM)
                        .fixed_pos(egui::pos2(left, bottom)),
                    full.max(1.0),
                )
            } else {
                (
                    area.pivot(egui::Align2::LEFT_BOTTOM)
                        .fixed_pos(egui::pos2(left, bottom)),
                    room.max(1.0),
                )
            }
        }
        None => (
            area.anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-gap, -gap)),
            f32::INFINITY,
        ),
    };
    let area = area.order(egui::Order::Foreground).show(ctx, |ui| {
        if folded {
            let label = folded_label(toasts.len());
            let b = ui.button(&label);
            super::groups::card(&b, &label, FOLDED_HINT, None, false);
            if b.clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(open_id(), true));
            }
            return;
        }
        if opened && cramped {
            let b = ui.button(FOLD_LABEL);
            super::groups::card(&b, FOLD_LABEL, FOLD_HINT, None, false);
            if b.clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(open_id(), false));
            }
        }
        egui::ScrollArea::vertical()
            .max_height(max_height)
            // A very short canvas still keeps the stack inside it (egui
            // keeps 64 pt by default).
            .min_scrolled_height(max_height.min(64.0))
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = t.spacing;
                for toast in toasts.iter_mut() {
                    let (event, hovered) = card(ui, toast, t, now, reduce_motion, width);
                    toast.set_hovered(hovered, now);
                    events.extend(event);
                }
            });
    });
    ctx.data_mut(|d| {
        if place.is_some_and(|p| p.canvas.is_positive()) {
            d.insert_temp(rect_id(), area.response.rect);
        } else {
            d.remove::<egui::Rect>(rect_id());
        }
    });
    // egui places an area by last frame's size, and the stack is pinned at
    // its bottom (left over the canvas, right in the window corner): when
    // the size changed (a toast came or went), the cards stood in the
    // wrong place this pass. Draw again at once, so no click lands on a
    // card that only looked like it was there.
    let size_id = egui::Id::unique("flatcraft-toasts-size");
    let size = area.response.rect.size();
    let last: Option<egui::Vec2> = ctx.data(|d| d.get_temp(size_id));
    ctx.data_mut(|d| d.insert_temp(size_id, size));
    if last.is_some_and(|l| l != size) {
        ctx.request_discard("the toast stack changed size");
    }
    events
}

/// One toast card. Returns its event and whether the pointer is on it.
fn card(
    ui: &mut egui::Ui,
    toast: &Toast,
    t: &Tokens,
    now: Instant,
    reduce_motion: bool,
    width: f32,
) -> (Option<Event>, bool) {
    const X: f32 = 20.0;
    let pad = 10.0;
    let frame = egui::Frame::new()
        .fill(t.panel)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(t.ribbon_radius)
        .inner_margin(egui::Margin::same(pad as i8))
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(40),
        });
    // The whole card is the click target of a toast with a file; it is
    // laid out first, so the × on top of it wins its own clicks.
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = frame.show(ui, |ui| {
        ui.set_width(width - 2.0 * pad);
        ui.horizontal_top(|ui| {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 4.0, toast.kind.color(t));
            ui.vertical(|ui| {
                ui.set_max_width(width - 2.0 * pad - 10.0 - X - 2.0 * ui.spacing().item_spacing.x);
                ui.add(
                    egui::Label::new(
                        RichText::new(&toast.text)
                            .font(super::fonts::semibold(14.0))
                            .color(t.text),
                    )
                    .wrap(),
                );
                if let Some(detail) = &toast.detail {
                    // Wrapped anywhere, so a long path is shown whole.
                    let mut job = egui::text::LayoutJob::simple(
                        detail.clone(),
                        egui::TextStyle::Small.resolve(ui.style()),
                        t.text_dim,
                        ui.available_width(),
                    );
                    job.wrap.break_anywhere = true;
                    ui.add(egui::Label::new(job));
                }
            });
        });
    });
    let rect = inner.response.rect;
    let id = egui::Id::unique(("flatcraft-toast", toast.id));
    let body = ui.interact(rect, id, egui::Sense::click());
    let mut event = None;
    if let Some(file) = &toast.reveal {
        let label = super::files::reveal_label();
        body.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, label));
        let body = body.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
        super::groups::card(&body, label, REVEAL_HINT, None, false);
        if body.clicked() {
            event = Some(Event::Reveal(file.clone()));
        }
    }
    if body.hovered() {
        ui.painter().set(
            bg,
            egui::Shape::rect_filled(rect, t.ribbon_radius, t.surface.gamma_multiply(0.4)),
        );
    }
    // The × with its countdown ring.
    let x_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.right() - pad - X / 2.0 + 2.0,
            rect.top() + pad + X / 2.0 - 1.0,
        ),
        egui::Vec2::splat(X),
    );
    let x = ui.interact(x_rect, id.with("close"), egui::Sense::click());
    x.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, CLOSE_NAME));
    let x = x.on_hover_text(CLOSE_NAME);
    if x.hovered() {
        ui.painter()
            .circle_filled(x_rect.center(), X / 2.0, t.surface);
    }
    let tex = icons::texture_colored(ui.ctx(), Icon::Close, 12.0, t.text_dim);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(12.0),
    ))
    .paint_at(
        ui,
        egui::Rect::from_center_size(x_rect.center(), egui::Vec2::splat(12.0)),
    );
    if let (Some(life), Some(left)) = (toast.lifetime(), toast.left(now))
        && !reduce_motion
    {
        let fraction = left.as_secs_f32() / life.as_secs_f32();
        ring(
            ui,
            x_rect.center(),
            X / 2.0 - 1.0,
            fraction,
            toast.kind.color(t),
        );
    }
    if x.clicked() {
        event = Some(Event::Close(toast.id));
    }
    let hovered = body.hovered() || x.hovered() || ui.rect_contains_pointer(rect);
    (event, hovered)
}

/// The part of a ring that is left: from the top, clockwise.
fn ring(ui: &egui::Ui, center: egui::Pos2, radius: f32, fraction: f32, color: Color32) {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return;
    }
    let steps = (48.0 * fraction).ceil().max(2.0) as usize;
    let sweep = std::f32::consts::TAU * fraction;
    let points: Vec<egui::Pos2> = (0..=steps)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + sweep * i as f32 / steps as f32;
            center + radius * egui::vec2(a.cos(), a.sin())
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
}

/// Whether the operating system asks for reduced motion (macOS: the
/// Accessibility setting "Reduce motion"). Elsewhere it is off.
///
/// The answer is cached and read again on a worker thread at most every
/// [`MOTION_REFRESH`], so the window never waits for the system (the
/// first question can take a moment while AppKit loads).
pub fn system_reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, Ordering};
        static REDUCE: AtomicBool = AtomicBool::new(false);
        static ASKED: Mutex<Option<Instant>> = Mutex::new(None);
        let mut asked = ASKED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if asked.is_none_or(|at| at.elapsed() >= MOTION_REFRESH) {
            *asked = Some(Instant::now());
            let spawned = std::thread::Builder::new()
                .name("reduce-motion".into())
                .spawn(|| {
                    let on = objc2_app_kit::NSWorkspace::sharedWorkspace()
                        .accessibilityDisplayShouldReduceMotion();
                    REDUCE.store(on, Ordering::Relaxed);
                });
            // Without a thread the last answer stays.
            drop(spawned);
        }
        REDUCE.load(Ordering::Relaxed)
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// How often [`system_reduce_motion`] asks the system again.
pub const MOTION_REFRESH: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifetimes_per_kind() {
        let t0 = Instant::now();
        let mut toasts = vec![
            Toast::info("info"),
            Toast::success("Saved"),
            Toast::warning("careful"),
            Toast::error("broken"),
        ];
        for t in &mut toasts {
            t.created = t0;
        }
        assert_eq!(toasts[0].lifetime(), Some(Duration::from_secs(5)));
        assert_eq!(toasts[1].lifetime(), Some(Duration::from_secs(5)));
        assert_eq!(toasts[2].lifetime(), Some(Duration::from_secs(8)));
        assert_eq!(toasts[3].lifetime(), None);
        assert_eq!(
            prune(&mut toasts, t0 + Duration::from_millis(4900)),
            Some(Duration::from_millis(100))
        );
        assert_eq!(toasts.len(), 4);
        assert_eq!(
            prune(&mut toasts, t0 + LIFETIME),
            Some(Duration::from_secs(3))
        );
        assert_eq!(toasts.len(), 2, "info and success are gone");
        assert_eq!(prune(&mut toasts, t0 + WARNING_LIFETIME), None);
        assert_eq!(toasts.len(), 1);
        // An error stays.
        assert_eq!(prune(&mut toasts, t0 + Duration::from_secs(3600)), None);
        assert_eq!(toasts[0].text, "broken");
    }

    #[test]
    fn hovering_pauses_the_clock() {
        let t0 = Instant::now();
        let mut t = Toast::info("saved");
        t.created = t0;
        let s = Duration::from_secs;
        t.set_hovered(true, t0 + s(2));
        // Ten seconds of resting: the clock stands at 2 s.
        assert_eq!(t.age(t0 + s(12)), s(2));
        assert!(!t.expired(t0 + s(12)));
        let mut v = vec![t.clone()];
        assert_eq!(prune(&mut v, t0 + s(12)), None, "no timer while resting");
        t.set_hovered(false, t0 + s(12));
        assert_eq!(t.left(t0 + s(12)), Some(s(3)));
        assert!(t.expired(t0 + s(15)));
    }
}
