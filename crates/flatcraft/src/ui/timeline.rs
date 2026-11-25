//! The Cut timeline: a slider over the toolpath index and a Play button.
//! Only the toolpaths up to the index are drawn. Prepare folds it away.

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

#[cfg(test)]
mod tests {
    use super::*;

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
