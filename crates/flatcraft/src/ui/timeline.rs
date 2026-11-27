//! The Cut timeline: a slider over the toolpath index and a Play button.
//! Only the toolpaths up to the index are drawn. Prepare folds it away.
//!
//! In Simulate the timeline runs over the simulation instead: a slider
//! over the head's travel in mm ([`SimClock`]), played at
//! [`SIM_RATE_MM_S`] times the chosen speed.

use egui::Ui;

use super::icons::Icon;
use super::theme::Tokens;
use super::{icon_button, settings::set_name};

/// Shortest and longest playback of a whole job, in seconds.
const PLAY_MIN_S: f64 = 2.0;
const PLAY_MAX_S: f64 = 10.0;
/// Toolpaths per second while that fits the limits above.
const PLAY_RATE: f64 = 6.0;

/// Where the timeline stands.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Timeline {
    /// How many toolpaths are drawn, `0..=total`.
    pub index: usize,
    /// Toolpaths of the current plan.
    pub total: usize,
    /// While playing: the frame time and index playback started at.
    pub playing: Option<(f64, usize)>,
}

impl Timeline {
    /// A new plan: show all of it and stop playing.
    pub fn reset(&mut self, total: usize) {
        *self = Self {
            index: total,
            total,
            playing: None,
        };
    }

    /// The number of toolpaths to draw.
    pub fn visible(&self) -> usize {
        self.index.min(self.total)
    }

    /// Toolpaths per second, so a whole job plays in 2 to 10 seconds.
    fn rate(&self) -> f64 {
        let n = self.total.max(1) as f64;
        let seconds = (n / PLAY_RATE).clamp(PLAY_MIN_S, PLAY_MAX_S);
        n / seconds
    }

    /// Starts playing at `now` (seconds); from the start if at the end.
    pub fn play(&mut self, now: f64) {
        if self.index >= self.total {
            self.index = 0;
        }
        self.playing = Some((now, self.index));
    }

    /// Moves the index on while playing. Returns true while it plays.
    pub fn tick(&mut self, now: f64) -> bool {
        let Some((start, from)) = self.playing else {
            return false;
        };
        let steps = ((now - start).max(0.0) * self.rate()).floor() as usize;
        self.index = (from + steps).min(self.total);
        if self.index >= self.total {
            self.playing = None;
        }
        self.playing.is_some()
    }
}

/// The timeline row at the bottom of the Cut workspace.
pub fn show(ui: &mut Ui, tl: &mut Timeline, t: &Tokens) {
    let now = ui.input(|i| i.time);
    if tl.tick(now) {
        ui.ctx().request_repaint();
    }
    ui.horizontal(|ui| {
        let has = tl.total > 0;
        if tl.playing.is_some() {
            if icon_button(ui, Icon::Close, "Stop", true).clicked() {
                tl.playing = None;
            }
        } else if icon_button(ui, Icon::Play, "Play", has).clicked() {
            tl.play(now);
            ui.ctx().request_repaint();
        }
        let mut index = tl.visible();
        ui.spacing_mut().slider_width = (ui.available_width() - 160.0).max(120.0);
        let resp = ui.add_enabled(
            has,
            egui::Slider::new(&mut index, 0..=tl.total.max(1)).show_value(false),
        );
        set_name(ui, &resp, "Toolpath");
        if resp.changed() {
            tl.index = index.min(tl.total);
            tl.playing = None;
        }
        ui.label(
            egui::RichText::new(format!("{} of {}", tl.visible(), tl.total)).color(t.text_dim),
        );
    });
}

/// Head travel per second of playback at 1x, mm.
pub const SIM_RATE_MM_S: f64 = 50.0;

/// The playback speeds of the simulation.
pub const SIM_SPEEDS: [f64; 3] = [0.25, 1.0, 4.0];

/// The hover card of the simulation time slider.
pub const SIM_SLIDER_CARD: &str = "Drag to move the head along the job. Everything up to this point is drawn: the head's path in thin grey and the blade tip in green, amber or red. The number is how far the head has travelled, in mm.";
/// The hover card of the speed buttons.
pub const SIM_SPEED_CARD: &str = "How fast Play runs. At 1x the head moves 50 mm per second on screen, about the speed of a real cut. Slow it to 0.25x to watch a corner, speed it up to 4x for a big job.";
/// The hover card of Play in Simulate.
pub const SIM_PLAY_CARD: &str = "Plays the cut from the start, the head and the blade tip as they move. It starts over when it is at the end.";
/// Why the simulation timeline is dimmed.
pub const SIM_NOTHING: &str = "Nothing to play yet: the simulation has not finished, or firmware mode is on, where FlatCraft does not simulate the knife.";

/// Where the simulation timeline stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimClock {
    /// Head travel shown, mm, `0..=total_mm`.
    pub t_mm: f64,
    /// Head travel of the whole job, mm.
    pub total_mm: f64,
    /// The playback speed, one of [`SIM_SPEEDS`].
    pub speed: f64,
    /// While playing: the frame time and travel playback started at.
    pub playing: Option<(f64, f64)>,
}

impl Default for SimClock {
    fn default() -> Self {
        Self {
            t_mm: 0.0,
            total_mm: 0.0,
            speed: 1.0,
            playing: None,
        }
    }
}

impl SimClock {
    /// A new simulation: show all of it and stop playing. The speed stays.
    pub fn reset(&mut self, total_mm: f64) {
        *self = Self {
            t_mm: total_mm,
            total_mm,
            speed: self.speed,
            playing: None,
        };
    }

    /// Starts playing at `now` (seconds); from the start if at the end.
    pub fn play(&mut self, now: f64) {
        if self.t_mm >= self.total_mm {
            self.t_mm = 0.0;
        }
        self.playing = Some((now, self.t_mm));
    }

    /// A new speed; playback goes on from where it is.
    pub fn set_speed(&mut self, speed: f64, now: f64) {
        self.speed = speed;
        if self.playing.is_some() {
            self.playing = Some((now, self.t_mm));
        }
    }

    /// Moves the time on while playing. Returns true while it plays.
    pub fn tick(&mut self, now: f64) -> bool {
        let Some((start, from)) = self.playing else {
            return false;
        };
        let t = from + (now - start).max(0.0) * SIM_RATE_MM_S * self.speed;
        self.t_mm = t.min(self.total_mm);
        if self.t_mm >= self.total_mm {
            self.playing = None;
        }
        self.playing.is_some()
    }
}

/// An icon-only button named `name` without a tooltip of its own: its
/// hover card comes from `groups::card`, and two would stack.
fn card_icon_button(ui: &mut Ui, icon: Icon, name: &str, enabled: bool) -> egui::Response {
    let tex = super::icons::texture(ui.ctx(), icon, super::ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(super::ICON_SIZE),
    ))
    .alt_text(name);
    ui.add_enabled(enabled, egui::Button::image(image))
}

/// The speed's label: "0.25x".
pub fn speed_label(speed: f64) -> String {
    format!("{speed}x")
}

/// The timeline row of the Simulate stage. `has` is false while there is
/// nothing to play.
pub fn show_sim(ui: &mut Ui, clock: &mut SimClock, has: bool, t: &Tokens) {
    let now = ui.input(|i| i.time);
    if clock.tick(now) {
        ui.ctx().request_repaint();
    }
    let why = (!has).then_some(SIM_NOTHING);
    ui.horizontal(|ui| {
        if clock.playing.is_some() {
            let r = card_icon_button(ui, Icon::Close, "Stop", true);
            super::groups::card(&r, "Stop", "Stops the playback where it is.", None, false);
            if r.clicked() {
                clock.playing = None;
            }
        } else {
            let r = card_icon_button(ui, Icon::Play, "Play", has);
            super::groups::card(&r, "Play", SIM_PLAY_CARD, why, false);
            if r.clicked() {
                clock.play(now);
                ui.ctx().request_repaint();
            }
        }
        for speed in SIM_SPEEDS {
            let label = speed_label(speed);
            let r = ui.add_enabled(
                has,
                egui::Button::new(label.as_str()).selected(clock.speed == speed),
            );
            super::groups::card(&r, &format!("Speed {label}"), SIM_SPEED_CARD, why, false);
            set_name(ui, &r, &format!("Speed {label}"));
            if r.clicked() {
                clock.set_speed(speed, now);
            }
        }
        let mut at = clock.t_mm.min(clock.total_mm);
        ui.spacing_mut().slider_width = (ui.available_width() - 200.0).max(120.0);
        let resp = ui.add_enabled(
            has,
            egui::Slider::new(&mut at, 0.0..=clock.total_mm.max(1.0)).show_value(false),
        );
        set_name(ui, &resp, "Time");
        super::groups::card(&resp, "Time", SIM_SLIDER_CARD, why, false);
        if resp.changed() {
            clock.t_mm = at.min(clock.total_mm);
            clock.playing = None;
        }
        let text = if has {
            format!(
                "{:.0} of {:.0} mm",
                clock.t_mm.min(clock.total_mm),
                clock.total_mm
            )
        } else {
            "Nothing to play".to_string()
        };
        ui.label(egui::RichText::new(text).color(t.text_dim));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sim_clock_plays_at_50_mm_per_second_times_the_speed() {
        let mut c = SimClock::default();
        c.reset(200.0);
        assert_eq!(c.t_mm, 200.0);
        c.play(10.0);
        assert_eq!(c.t_mm, 0.0);
        assert!(c.tick(11.0));
        assert!((c.t_mm - 50.0).abs() < 1e-9);
        c.set_speed(4.0, 11.0);
        assert!(c.tick(11.5));
        assert!((c.t_mm - 150.0).abs() < 1e-9);
        c.set_speed(0.25, 11.5);
        assert!(c.tick(12.5));
        assert!((c.t_mm - 162.5).abs() < 1e-9);
        assert!(!c.tick(100.0));
        assert_eq!(c.t_mm, 200.0);
        // A new simulation keeps the speed.
        c.reset(10.0);
        assert_eq!((c.t_mm, c.speed), (10.0, 0.25));
    }

    #[test]
    fn play_runs_from_the_start_to_the_end() {
        let mut tl = Timeline::default();
        tl.reset(12);
        assert_eq!(tl.visible(), 12);
        tl.play(100.0);
        assert_eq!(tl.index, 0);
        assert!(tl.tick(100.5));
        assert_eq!(tl.index, 3);
        assert!(!tl.tick(103.0));
        assert_eq!(tl.index, 12);
        assert!(tl.playing.is_none());
    }

    #[test]
    fn a_new_plan_clamps_the_index() {
        let mut tl = Timeline::default();
        tl.reset(5);
        tl.index = 4;
        tl.total = 2;
        assert_eq!(tl.visible(), 2);
    }
}
