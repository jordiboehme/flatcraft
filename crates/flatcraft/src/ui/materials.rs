//! The material picker: presets in groups, a hover card per material with
//! its tips and values, the "Start with a test cut" hint for a material
//! never cut, and the Cut stage's mirror reminder for iron-on material.

use crate::ui::metrics;
use egui::{RichText, Ui};
use flatcraft_device::{MatTack, MaterialGroup, MaterialProfile, ProfileStore, presets};
use flatcraft_engine::cutlog::CutLog;
use flatcraft_engine::troubleshoot::Item;
use serde_json::json;

use super::Action;
use super::inspector::blocks::HintButton;
use super::prepare::Snapshot;
use super::theme::Tokens;

/// One entry of the material picker.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialCard {
    pub id: String,
    pub name: String,
    pub system: bool,
    pub group: MaterialGroup,
    /// The preset with inheritance resolved; `None` when it cannot be read.
    pub profile: Option<MaterialProfile>,
    /// The cut log has a cut with this material.
    pub cut_before: bool,
}

/// The picker entries in picker order: by group, the built-in presets in
/// the order they ship ([`presets::MATERIALS`]), then the person's own by
/// name.
pub fn cards(profiles: &ProfileStore, log: &CutLog) -> Vec<MaterialCard> {
    let rank = |id: &str| {
        presets::MATERIALS
            .iter()
            .position(|(p, _)| *p == id)
            .unwrap_or(usize::MAX)
    };
    let mut out: Vec<MaterialCard> = profiles
        .materials()
        .into_iter()
        .map(|i| {
            let profile = profiles.material(&i.id).ok();
            MaterialCard {
                group: profile.as_ref().map_or(MaterialGroup::Other, |m| m.group),
                cut_before: log.has_cut(&i.id),
                id: i.id,
                name: i.name,
                system: i.system,
                profile,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        let g = |c: &MaterialCard| MaterialGroup::ALL.iter().position(|x| *x == c.group);
        g(a).cmp(&g(b))
            .then(b.system.cmp(&a.system))
            .then(rank(&a.id).cmp(&rank(&b.id)))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// The picker entries, built again only when the presets (their store
/// revision) or the cut log change, not on every frame.
#[derive(Debug, Clone, Default)]
pub struct CardsCache {
    key: Option<(u64, std::sync::Arc<CutLog>)>,
    cards: Vec<MaterialCard>,
    /// How often the entries were built (for tests).
    pub builds: u64,
}

impl CardsCache {
    pub fn get(
        &mut self,
        profiles: &ProfileStore,
        log: &std::sync::Arc<CutLog>,
    ) -> Vec<MaterialCard> {
        let fresh = self
            .key
            .as_ref()
            .is_some_and(|(r, l)| *r == profiles.revision() && std::sync::Arc::ptr_eq(l, log));
        if !fresh {
            self.cards = cards(profiles, log);
            self.builds += 1;
            self.key = Some((profiles.revision(), log.clone()));
        }
        self.cards.clone()
    }
}

/// What a mat tack means for this material, with the term explained.
pub fn tack_text(t: MatTack) -> &'static str {
    match t {
        MatTack::Light => {
            "Mat: light grip. The mat's tack is how sticky it is; a light one lets thin paper go without tearing."
        }
        MatTack::Standard => {
            "Mat: standard grip. The mat's tack is how sticky it is; the standard one holds most materials."
        }
        MatTack::Strong => {
            "Mat: strong grip. The mat's tack is how sticky it is; a strong one keeps stiff sheets from shifting."
        }
    }
}

/// What the backing sheet means for this material.
pub fn backing_text(backing: bool) -> &'static str {
    if backing {
        "It sits on a backing sheet: small loose parts stay on it until you lift them (with transfer tape for vinyl)."
    } else {
        "It has no backing sheet, so small loose parts, like the middle of an O, fall out."
    }
}

/// The sentence on mirroring iron-on material.
pub const MIRROR_TEXT: &str = "Iron-on is cut from the back, so the design is cut mirrored (flipped left to right) and reads right once it is ironed on.";

/// The hint for a material never cut.
pub const NEVER_CUT_TEXT: &str = "You have not cut it on this computer yet: start with a test cut.";

/// The hover card of a picker entry: the tips, the starting values, the
/// mat, the backing sheet, mirroring for iron-on, and the test cut hint.
pub fn card_text(c: &MaterialCard) -> String {
    let Some(m) = &c.profile else {
        return "This preset cannot be read. Open Settings, Presets to see why and fix it.".into();
    };
    let mut parts = Vec::new();
    if let Some(tips) = m.tips.as_deref().filter(|s| !s.trim().is_empty()) {
        parts.push(tips.trim().to_string());
    }
    let passes = if m.passes == 1 {
        "1 pass".to_string()
    } else {
        format!("{} passes", m.passes)
    };
    let blade = m
        .blade_depth_hint
        .as_deref()
        .map(|h| format!(", {h} (the number you turn the blade to by hand)"))
        .unwrap_or_default();
    parts.push(format!(
        "Starts at speed {}, force {}, {passes}{blade}.",
        m.speed, m.force
    ));
    parts.push(tack_text(m.mat_tack).to_string());
    parts.push(backing_text(m.backing).to_string());
    if flatcraft_engine::troubleshoot::iron_on(m) {
        parts.push(MIRROR_TEXT.to_string());
    }
    if !c.cut_before {
        parts.push(NEVER_CUT_TEXT.to_string());
    }
    parts.join("\n\n")
}

/// The card of a group heading in the picker.
pub fn group_hint(g: MaterialGroup) -> &'static str {
    match g {
        MaterialGroup::Vinyl => {
            "Self-adhesive vinyl on a backing sheet, for decals, signs and labels. You lift the finished design with transfer tape."
        }
        MaterialGroup::IronOn => {
            "Heat transfer vinyl that you iron onto fabric. It is cut from the back, so the design is cut mirrored (flipped left to right)."
        }
        MaterialGroup::PaperAndCard => {
            "Paper, card, kraft, vellum and sticker paper. Most have no backing sheet, so small loose parts fall out."
        }
        MaterialGroup::Other => {
            "Stencil film, window cling, magnet sheets, and your own materials that have no group."
        }
    }
}

/// The card of the picker itself.
pub const PICKER_HINT: &str = "What you cut. It sets the starting speed, force and passes, and the card of each material says which side goes up and which mat to use. Hover a material in the list to read it.";

/// The material picker: a combo grouped by kind of material. Returns the
/// combo and the picked material's id when it differs from the
/// document's. Its card is [`picker_card`]; the row that holds it shows it.
pub fn picker(ui: &mut Ui, snap: &Snapshot, t: &Tokens) -> (egui::Response, Option<String>) {
    let current = &snap.job.material_id;
    let text = snap
        .material
        .as_ref()
        .map_or(current.as_str(), |m| m.name.as_str());
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt("Material")
        .selected_text(text)
        .width(ui.available_width())
        .truncate()
        .height(600.0)
        .show_ui(ui, |ui| {
            for group in MaterialGroup::ALL {
                let mut entries = snap
                    .material_cards
                    .iter()
                    .filter(|c| c.group == group)
                    .peekable();
                if entries.peek().is_none() {
                    continue;
                }
                let h = ui.label(
                    RichText::new(group.label())
                        .font(metrics::subsection_title())
                        .color(t.text_dim),
                );
                super::groups::card(&h, group.label(), group_hint(group), None, true);
                for c in entries {
                    let r = ui.selectable_label(&c.id == current, &c.name);
                    super::groups::card(&r, &c.name, &card_text(c), None, true);
                    if r.clicked() && &c.id != current {
                        picked = Some(c.id.clone());
                    }
                }
            }
        });
    (r.response, picked)
}

/// The card of the material picker, with the picked material's card.
pub fn picker_card(snap: &Snapshot) -> String {
    let current = &snap.job.material_id;
    match snap.material_cards.iter().find(|c| &c.id == current) {
        Some(c) => format!("{PICKER_HINT}\n\n{}: {}", c.name, card_text(c)),
        None => PICKER_HINT.to_string(),
    }
}

/// A preset file for the document's material that was skipped at load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub reason: String,
    /// Skipped because a built-in preset has its id.
    pub shadows_system: bool,
}

/// The Job card line when the person's own preset file for the
/// document's material id was not loaded (see `ProfileStore::diagnostics`):
/// worded after why.
pub fn skipped_text(shadows_system: bool) -> &'static str {
    if shadows_system {
        "Your own preset file with this id is not loaded, because a built-in preset has the id now. The built-in values are used."
    } else {
        "Your own preset file for this material could not be read, so it is not used."
    }
}

/// Shows [`skipped_text`] with the reason on its card.
pub fn skipped_note(ui: &mut Ui, snap: &Snapshot, t: &Tokens) {
    let Some(skipped) = &snap.material_skipped else {
        return;
    };
    let reason = &skipped.reason;
    ui.add_space(metrics::SPACE_4);
    let text = skipped_text(skipped.shadows_system);
    let r = ui.add(egui::Label::new(RichText::new(text).color(t.danger)).wrap());
    super::groups::card(
        &r,
        "Preset file not loaded",
        &format!("{reason}. Settings, Materials lists the file and why."),
        None,
        false,
    );
}

/// The test cut hint's heading.
pub const TEST_CUT_TITLE: &str = "Start with a test cut";
/// The id of the test cut hint, for hiding it (`Preferences.hidden_tips`).
pub const TEST_CUT_TIP: &str = "job.test_cut";
/// The test cut hint's button.
pub const TEST_CUT_BUTTON: &str = "Add a test shape";

/// The card of the test cut hint. What a good test cut looks like
/// depends on the backing sheet: on one, the blade must not go through it.
pub fn test_cut_text(material: &str, backing: bool) -> String {
    let good = if backing {
        "a good cut lets the square peel off its backing cleanly, the hole lifts out of the square, and the backing shows only a light line, not a cut through it. If the backing is cut through, lower the force"
    } else {
        "a good cut goes all the way through: the square comes free and the hole drops out. If a corner still hangs on, raise the force a little or cut again"
    };
    format!(
        "You have not cut {material} on this computer yet, so these are starting values, not your values. Cut a small test shape first and look at it: {good}. A test cut is cheaper than a ruined sheet."
    )
}

/// Why the test shape button is dimmed.
pub const NO_ROOM: &str = "There is no free spot on the mat for the test shape. Move your design closer together, or pick a larger mat.";

/// "Start with a test cut" in the Job panel's Machine and material
/// section (a hint card), for a material the cut log
/// has never seen. The button runs `tools.test_shape`; it is dimmed, with
/// the reason, when the mat has no room for the shape.
pub fn test_cut_hint(ui: &mut Ui, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    let Some(tc) = &snap.test_cut else {
        return;
    };
    let name = snap
        .material
        .as_ref()
        .map_or(snap.job.material_id.as_str(), |m| m.name.as_str());
    let backing = snap.material.as_ref().is_none_or(|m| m.backing);
    let text = test_cut_text(name, backing);
    ui.add_space(metrics::SPACE_4);
    let issue =
        format!("You have not cut {name} yet. Cut a small test shape first, then the design.");
    let button = super::inspector::blocks::HintButton {
        label: TEST_CUT_BUTTON,
        card: "Puts a 30 mm square with a 10 mm hole on a free spot of the mat, away from your design. Cut it first, check it, then cut the rest. Undo takes it back.",
        why: (!tc.room).then_some(NO_ROOM),
    };
    let clicked = super::inspector::blocks::hint_card(
        ui,
        t,
        Some(TEST_CUT_TIP),
        Some((TEST_CUT_TITLE, &text)),
        &issue,
        &[button],
    );
    if clicked == Some(0) {
        actions.push(Action::run(snap.doc_id, "tools.test_shape", json!({})));
    }
}

/// The key of [`TestCutCache`]: the document, its revision, and the
/// plotter and material the room is measured with.
type RoomKey = (
    u64,
    u64,
    Option<flatcraft_device::PlotterProfile>,
    Option<MaterialProfile>,
);

/// Whether the mat has room for the test shape, measured once per
/// document revision, plotter and material: finding the spot builds a job
/// input (a copy of the document), which is too much for every frame.
#[derive(Debug, Clone, Default)]
pub struct TestCutCache {
    key: Option<RoomKey>,
    room: bool,
    /// How often the room was measured (for tests).
    pub builds: u64,
}

impl TestCutCache {
    /// The cached answer for this key, or `measure`'s new one.
    pub fn room(
        &mut self,
        doc_id: u64,
        revision: u64,
        plotter: &Option<flatcraft_device::PlotterProfile>,
        material: &Option<MaterialProfile>,
        measure: impl FnOnce() -> bool,
    ) -> bool {
        let fresh = self.key.as_ref().is_some_and(|(d, r, p, m)| {
            *d == doc_id && *r == revision && p == plotter && m == material
        });
        if !fresh {
            self.room = measure();
            self.builds += 1;
            self.key = Some((doc_id, revision, plotter.clone(), material.clone()));
        }
        self.room
    }
}

/// Whether the Job card offers a test cut (see [`test_cut_hint`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestCut {
    /// The mat has a free spot for the test shape.
    pub room: bool,
}

/// The reminder's heading.
pub const MIRROR_TITLE: &str = "Mirror for iron-on";
/// The reminder's button: the troubleshooter's mirror fix under a name of
/// its own, so the two never read alike when both are open.
pub const MIRROR_BUTTON: &str = "Mirror the layout";

/// The Cut stage's reminder when iron-on material is picked and the
/// design is not (all) mirrored: the troubleshooter's mirror item with
/// its fix. It never blocks the send button.
pub fn iron_on_reminder(ui: &mut Ui, snap: &Snapshot, t: &Tokens, actions: &mut Vec<Action>) {
    let Some(Ok(input)) = &snap.job_input else {
        return;
    };
    let Some(item) = flatcraft_engine::troubleshoot::iron_on_reminder(input) else {
        return;
    };
    show_reminder(ui, &item, snap.doc_id, t, actions);
}

/// The reminder as a hint card: its title, the message, the mirror
/// button, and the same button dimmed with its reason where the fix is
/// blocked.
fn show_reminder(ui: &mut Ui, item: &Item, doc_id: u64, t: &Tokens, actions: &mut Vec<Action>) {
    // The registry's own id: a fix names a real command.
    let fixes: Vec<Action> = item
        .fixes
        .iter()
        .filter_map(|fix| {
            let command = flatcraft_engine::registry::get(&fix.command)?;
            Some(Action::run(doc_id, command.id(), fix.args.clone()))
        })
        .collect();
    let mut buttons: Vec<HintButton> = item
        .fixes
        .iter()
        .filter(|fix| flatcraft_engine::registry::get(&fix.command).is_some())
        .map(|fix| HintButton {
            label: MIRROR_BUTTON,
            card: &fix.description,
            why: None,
        })
        .collect();
    buttons.extend(item.blocked.iter().map(|bl| HintButton {
        label: MIRROR_BUTTON,
        card: &item.hint,
        why: Some(&bl.reason),
    }));
    ui.add_space(metrics::SPACE_4);
    if let Some(i) = super::inspector::blocks::hint_card(
        ui,
        t,
        None,
        Some((MIRROR_TITLE, &item.hint)),
        &item.message,
        &buttons,
    ) && let Some(action) = fixes.get(i)
    {
        actions.push(action.clone());
    }
    ui.add_space(metrics::SPACE_4);
}
