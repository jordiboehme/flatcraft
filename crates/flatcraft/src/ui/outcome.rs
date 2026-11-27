//! "How did it go?": the person says how a cut went (thumbs up or down and
//! a few words), and the cut log keeps it. The Job card recalls it the next
//! time that material is picked ("Last time with this material: ...").
//!
//! The question opens from the Job card's "Rate the cut of ..." button,
//! for the newest cut with the material that nobody rated yet. Right after a
//! send it opens only for a cut on a plotter (milestone 3), never after
//! File Output, which cut nothing yet ([`cutlog::asks_after`]).

use egui::{RichText, Ui};
use flatcraft_engine::cutlog::{self, CutEntry, Verdict};

use super::dialog::{Answer, Dialog, Kind};
use super::icons::{self, Icon};
use super::settings::set_name;
use super::theme::Tokens;
use super::{Action, ICON_SIZE};

/// The dialog's title.
pub const TITLE: &str = "How did it go?";
/// Why Save is dimmed before a thumb is picked.
pub const PICK_FIRST: &str = "Pick thumbs up or thumbs down first.";
/// The card of the Job card's button that opens the question.
pub const ASK_HINT: &str = "Say how this cut went: thumbs up or down and a few words. Next time you pick this material, the job settings remind you. It stays on this computer. Agents you connect to FlatCraft can read it.";

/// The Job card's button for the newest cut nobody rated yet: "Rate the
/// cut of 2026-11-26", in the local day.
pub fn rate_label(e: &CutEntry) -> String {
    format!("Rate the cut of {}", day(&e.date))
}

/// The open question: the cut it is about and the answer so far.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeDialog {
    pub entry: String,
    /// What the cut was, in one line.
    pub about: String,
    pub verdict: Option<Verdict>,
    pub note: String,
    /// The cut is a calibration cut: nothing in the question can be
    /// picked, and the dimmed Save says why
    /// ([`cutlog::CALIBRATION_NOT_RATED`]).
    pub calibration: bool,
}

impl OutcomeDialog {
    /// The question about `entry`, with its earlier answer when it has one.
    pub fn about(entry: &CutEntry) -> Self {
        Self {
            entry: entry.id.clone(),
            about: cut_text(entry),
            verdict: entry.outcome.as_ref().map(|o| o.verdict),
            note: entry
                .outcome
                .as_ref()
                .map(|o| o.note.clone())
                .unwrap_or_default(),
            calibration: entry.is_calibration(),
        }
    }
}

/// The local day of a cut log time, like "2026-11-26": the system time
/// zone's offset at the cut's own instant, so summer time is right for
/// that day.
pub fn day(date: &str) -> String {
    day_in(date, &jiff::tz::TimeZone::system())
}

/// The day of a cut log time in time zone `tz`, with the zone's offset at
/// that instant ([`cutlog::day_at`]). A text that is not a time shows its
/// first ten characters.
pub fn day_in(date: &str, tz: &jiff::tz::TimeZone) -> String {
    let offset = date
        .parse::<jiff::Timestamp>()
        .map_or(0, |ts| tz.to_offset(ts).seconds());
    cutlog::day_at(date, offset)
}

fn passes(n: u32) -> String {
    if n == 1 {
        "1 pass".into()
    } else {
        format!("{n} passes")
    }
}

/// "2026-11-26, Vinyl on File Output (GPGL Cameo 2), force 12, 2 passes",
/// with "offset test series, " after the day for a calibration cut.
pub fn cut_text(e: &CutEntry) -> String {
    let what = if e.is_calibration() {
        "offset test series, "
    } else {
        ""
    };
    format!(
        "{}, {what}{} on {}, force {}, {}",
        day(&e.date),
        e.material,
        e.plotter,
        e.force,
        passes(e.passes)
    )
}

/// The Job card's line: "Last time with this material: 2026-11-26, force
/// 12, 2 passes - good: clean edges", or "- not rated yet".
pub fn last_time_text(e: &CutEntry) -> String {
    let outcome = match &e.outcome {
        Some(o) if o.note.is_empty() => o.verdict.word().to_string(),
        Some(o) => format!("{}: {}", o.verdict.word(), o.note),
        None => "not rated yet".to_string(),
    };
    format!(
        "Last time with this material: {}, force {}, {} - {outcome}",
        day(&e.date),
        e.force,
        passes(e.passes)
    )
}

/// Opens the question after a send when the cut was on a plotter. A
/// calibration cut is never asked about: the person picks the best tile
/// of it, it is not good or bad.
pub fn after_send(entry: &CutEntry) -> Option<OutcomeDialog> {
    (cutlog::asks_after(&entry.target) && !entry.is_calibration())
        .then(|| OutcomeDialog::about(entry))
}

/// A thumb button that shows whether it is picked.
fn thumb(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    on: bool,
    hint: &str,
    why_not: Option<&str>,
) -> bool {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ));
    let r = ui.add_enabled(
        why_not.is_none(),
        egui::Button::image_and_text(image, label).selected(on),
    );
    set_name(ui, &r, label);
    super::groups::card(&r, label, hint, why_not, false);
    r.clicked()
}

/// Shows the question while it is open. Save records the answer; Not now,
/// Escape and the × close it and record nothing.
pub fn show(
    ctx: &egui::Context,
    slot: &mut Option<OutcomeDialog>,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let Some(d) = slot else {
        return;
    };
    let dialog = Dialog::new("outcome", Kind::Info, TITLE)
        .body(d.about.clone())
        .primary("Save")
        .primary_hint("Keeps your answer in the cut log on this computer. The job settings show it the next time you pick this material.")
        .cancel("Not now")
        .cancel_hint("Closes the question without saving anything. You can answer later from the job settings.");
    let why_not = d.calibration.then_some(cutlog::CALIBRATION_NOT_RATED);
    let answer = dialog.show_with(ctx, t, |ui| {
        ui.add_space(t.spacing);
        ui.horizontal(|ui| {
            if thumb(
                ui,
                Icon::ThumbsUp,
                "It went well",
                d.verdict == Some(Verdict::Good),
                "The cut came out as it should: clean edges, the pieces weed well.",
                why_not,
            ) {
                d.verdict = Some(Verdict::Good);
            }
            if thumb(
                ui,
                Icon::ThumbsDown,
                "It did not go well",
                d.verdict == Some(Verdict::Bad),
                "Something was off: it did not cut through, it tore, or the pieces moved. Say what in the note, so you know what to change next time.",
                why_not,
            ) {
                d.verdict = Some(Verdict::Bad);
            }
        });
        ui.add_space(t.spacing / 2.0);
        ui.label(RichText::new("A few words (you may leave it empty)").color(t.text_dim));
        let r = ui.add_enabled(
            why_not.is_none(),
            egui::TextEdit::singleline(&mut d.note)
                .hint_text("Force 14 cut clean, weed slowly")
                .desired_width(f32::INFINITY),
        );
        set_name(ui, &r, "Outcome note");
        super::groups::card(
            &r,
            "Note",
            "What you want to remember about this cut: what worked, what to change next time.",
            why_not,
            false,
        );
        if let Some(why) = why_not {
            return Some(why.to_string());
        }
        d.verdict.is_none().then(|| PICK_FIRST.to_string())
    });
    match answer {
        Some(Answer::Primary) => {
            if let Some(verdict) = d.verdict.filter(|_| !d.calibration) {
                actions.push(Action::RecordOutcome {
                    entry: d.entry.clone(),
                    verdict,
                    note: d.note.clone(),
                });
            }
            *slot = None;
        }
        Some(_) => *slot = None,
        None => {}
    }
}
