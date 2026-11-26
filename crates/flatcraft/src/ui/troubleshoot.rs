//! "It didn't cut right...": the person says what went wrong, and the
//! helper shows what to check, in order, with buttons for what FlatCraft
//! can change. The list is the engine's
//! (`flatcraft_engine::troubleshoot::checklist`), the same that agents get
//! from `job.troubleshoot`; a button runs exactly its fix's registry
//! command with its arguments. Speed, force and passes buttons change the
//! job's overrides, never the material preset, one undo step each.
//!
//! It opens from the Cut workspace (under the send button) and from the
//! Help menu, as one of the shared dialogs.

use egui::{RichText, Ui};
use flatcraft_engine::cutlog::CutEntry;
use flatcraft_engine::job::JobInput;
use flatcraft_engine::troubleshoot::{self, Check, Checklist, Item, Symptom};

use super::dialog::{Dialog, Kind};
use super::groups::card;
use super::prepare::Snapshot;
use super::theme::Tokens;
use super::{Action, fonts};

/// The text of the button and the Help menu item.
pub const LABEL: &str = "It didn't cut right...";
/// The dialog's title.
pub const TITLE: &str = "It didn't cut right?";
/// The card of the button that opens the helper.
pub const OPEN_HINT: &str = "Tell FlatCraft what went wrong with a cut, and get a list of what to check, in order. Buttons in the list change this job's speed, force or passes, never the material preset.";
/// The line under the title.
pub const ASK: &str = "What happened? Pick the closest.";
/// Shown before a symptom is picked.
pub const PICK_FIRST: &str = "Pick what happened to see what to check.";
/// The card of Close.
pub const CLOSE_HINT: &str =
    "Closes the helper. What you changed with its buttons stays, and Undo takes it back.";
/// The marker of an item FlatCraft sees as the likely cause.
pub const LIKELY: &str = "Likely the cause";

/// The open helper.
#[derive(Debug, Clone, Default)]
pub struct TroubleshootState {
    pub open: bool,
    pub symptom: Option<Symptom>,
    /// The list shown, and what it was made from.
    cache: Option<(Key, Checklist)>,
}

/// What a checklist was made from: it is made again only when one of
/// these changed.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    doc_id: u64,
    input: JobInput,
    symptom: Symptom,
    last_good: Option<CutEntry>,
}

impl TroubleshootState {
    /// Opens the helper with no symptom picked yet.
    pub fn open(&mut self) {
        self.open = true;
        self.symptom = None;
    }

    /// The checklist for the picked symptom on this snapshot's job, made
    /// again only when the job input, the symptom or the last good cut
    /// changed. `None` before a symptom is picked or without a job input.
    pub fn checklist(&mut self, snap: &Snapshot) -> Option<&Checklist> {
        let symptom = self.symptom?;
        let Some(Ok(input)) = &snap.job_input else {
            self.cache = None;
            return None;
        };
        let key = Key {
            doc_id: snap.doc_id,
            input: input.clone(),
            symptom,
            last_good: snap.last_good.clone(),
        };
        if self.cache.as_ref().map(|(k, _)| k) != Some(&key) {
            let list = troubleshoot::checklist(input, symptom, snap.last_good.as_ref());
            self.cache = Some((key, list));
        }
        self.cache.as_ref().map(|(_, c)| c)
    }
}

/// Draws the open helper and collects what its buttons ask for.
pub fn show(
    ctx: &egui::Context,
    st: &mut TroubleshootState,
    snap: &Snapshot,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    if !st.open {
        return;
    }
    let dialog = Dialog::new("troubleshoot", Kind::Info, TITLE)
        .body(ASK)
        .cancel("Close")
        .cancel_hint(CLOSE_HINT);
    let mut close = false;
    let answer = dialog.show(ctx, t, |ui| {
        ui.add_space(t.spacing / 2.0);
        ui.horizontal_wrapped(|ui| {
            for s in Symptom::ALL {
                let r = ui.add(egui::Button::selectable(st.symptom == Some(s), s.label()));
                card(&r, s.label(), s.description(), None, false);
                if r.clicked() {
                    st.symptom = Some(s);
                }
            }
        });
        ui.add_space(t.spacing);
        // One height for the list, whatever is picked: the dialog keeps
        // its size and place, so the symptom buttons never jump.
        let height = (ui.ctx().content_rect().height() * 0.55).clamp(160.0, 520.0);
        ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
            ui.set_min_height(height);
            ui.set_max_height(height);
            if st.symptom.is_none() {
                ui.label(RichText::new(PICK_FIRST).color(t.text_dim));
                return;
            }
            if let Some(Err(e)) = &snap.job_input {
                ui.add(egui::Label::new(super::person_text(e)).wrap());
                return;
            }
            let doc_id = snap.doc_id;
            let Some(list) = st.checklist(snap) else {
                return;
            };
            ui.add(egui::Label::new(RichText::new(&list.lead).color(t.text_dim)).wrap());
            ui.add_space(t.spacing / 2.0);
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            egui::ScrollArea::vertical()
                .id_salt(("flatcraft-troubleshoot-list", list.symptom.as_str()))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (n, it) in list.items.iter().enumerate() {
                        if n > 0 {
                            ui.add_space(t.spacing);
                        }
                        if item_row(ui, n + 1, it, list, doc_id, t, actions) {
                            close = true;
                        }
                    }
                });
        });
    });
    if answer.is_some() || close {
        st.open = false;
    }
}

/// One item: number and title, the message (its card has the hint), the
/// marker when it is the likely cause, then its buttons. Returns true when
/// "Show me" was clicked (it leaves for Prepare, so the helper closes).
fn item_row(
    ui: &mut Ui,
    number: usize,
    it: &Item,
    list: &Checklist,
    doc_id: u64,
    t: &Tokens,
    actions: &mut Vec<Action>,
) -> bool {
    let mut leave = false;
    let title = format!("{number}. {}", it.title);
    let r = ui.label(RichText::new(&title).font(fonts::semibold(13.0)));
    card(&r, &it.title, &it.hint, None, false);
    if it.likely {
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), egui::Sense::hover());
            ui.painter().circle_filled(dot.center(), 3.5, t.warn);
            ui.label(RichText::new(LIKELY).color(t.warn));
        });
    }
    let r = ui.add(egui::Label::new(it.message.as_str()).wrap());
    card(&r, &it.title, &it.hint, None, false);
    if it.check == Check::LastGood
        && let Some(g) = &list.last_good
    {
        ui.label(
            RichText::new(format!("Cut on {}", super::outcome::day(&g.date))).color(t.text_dim),
        );
    }
    let buttons = !it.fixes.is_empty() || !it.blocked.is_empty() || !it.object_ids.is_empty();
    if !buttons {
        return false;
    }
    ui.horizontal_wrapped(|ui| {
        for fix in &it.fixes {
            // The registry's own id: a fix names a real command.
            let Some(command) = flatcraft_engine::registry::get(&fix.command) else {
                continue;
            };
            let r = ui.button(fix.label.as_str());
            card(&r, &fix.label, &fix.description, None, false);
            if r.clicked() {
                actions.push(Action::run(doc_id, command.id(), fix.args.clone()));
            }
        }
        for b in &it.blocked {
            let r = ui.add_enabled(false, egui::Button::new(b.label.as_str()));
            card(
                &r,
                &b.label,
                &blocked_card(it.check),
                Some(&b.reason),
                false,
            );
        }
        if !it.object_ids.is_empty() {
            let r = ui.button("Show me");
            card(&r, "Show me", super::report::SHOW_ME_HINT, None, false);
            if r.clicked() {
                actions.push(Action::ShowMe {
                    doc_id,
                    ids: it.object_ids.clone(),
                });
                leave = true;
            }
        }
    });
    leave
}

/// What a dimmed button would do, under the reason it cannot.
fn blocked_card(check: Check) -> String {
    match check {
        Check::TestShape => "Adds a small test shape on a free spot of the mat.".into(),
        Check::LastGood => {
            "Sets this job's speed, force and passes to those of your last good cut.".into()
        }
        _ => "Changes this job's setting. Only this job changes, not the material preset.".into(),
    }
}
