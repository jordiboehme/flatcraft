//! The material picker: presets in groups, a hover card per material with
//! its tips and values, the "Start with a test cut" hint for a material
//! never cut, and the Cut stage's mirror reminder for iron-on material.

use egui::{RichText, Ui};
use flatcraft_device::{MatTack, MaterialGroup, MaterialProfile, ProfileStore, presets};
use flatcraft_engine::cutlog::CutLog;
use flatcraft_engine::troubleshoot::Item;
use serde_json::json;

use super::prepare::Snapshot;
use super::theme::Tokens;
use super::{Action, fonts};

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
        "It sits on a backing sheet: small loose parts stay on it until transfer tape lifts them."
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

/// The card of the picker itself.
pub const PICKER_HINT: &str = "What you cut. It sets the starting speed, force and passes, and the card of each material says which side goes up and which mat to use. Hover a material in the list to read it.";

/// The Material row of the Job grid: the label and the grouped picker.
/// Returns the picked material's id when it differs from the document's.
pub fn picker_row(ui: &mut Ui, snap: &Snapshot, t: &Tokens) -> Option<String> {
    super::settings::painted_label(ui, "Material");
    let current = &snap.job.material_id;
    let shown = snap.material_cards.iter().find(|c| &c.id == current);
    let text = snap
        .material
        .as_ref()
        .map_or(current.as_str(), |m| m.name.as_str());
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt("Material")
        .selected_text(text)
        .width(ui.available_width())
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
                ui.label(
                    RichText::new(group.label())
                        .font(fonts::semibold(12.0))
                        .color(t.text_dim),
                );
                for c in entries {
                    let r = ui.selectable_label(&c.id == current, &c.name);
                    super::groups::card(&r, &c.name, &card_text(c), None, true);
                    if r.clicked() && &c.id != current {
                        picked = Some(c.id.clone());
                    }
                }
            }
        });
    super::settings::set_name(ui, &r.response, "Material");
    let card = match shown {
        Some(c) => format!("{PICKER_HINT}\n\n{}: {}", c.name, card_text(c)),
        None => PICKER_HINT.to_string(),
    };
    super::groups::card(&r.response, "Material", &card, None, false);
    ui.end_row();
    picked
}

/// The test cut hint's heading.
pub const TEST_CUT_TITLE: &str = "Start with a test cut";
/// The test cut hint's button.
pub const TEST_CUT_BUTTON: &str = "Add a test shape";

/// The card of the test cut hint.
pub fn test_cut_text(material: &str) -> String {
    format!(
        "You have not cut {material} on this computer yet, so these are starting values, not your values. Cut a small test shape first: in a minute it shows whether the blade goes through and the hole drops out, before the whole design is at stake. A test cut is cheaper than a ruined sheet."
    )
}

/// Why the test shape button is dimmed.
pub const NO_ROOM: &str = "There is no free spot on the mat for the test shape. Move your design closer together, or pick a larger mat.";

/// "Start with a test cut" under the Job grid, for a material the cut log
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
    let text = test_cut_text(name);
    ui.add_space(t.spacing / 2.0);
    let r = ui.label(
        RichText::new(TEST_CUT_TITLE)
            .font(fonts::semibold(13.0))
            .color(t.accent_cut),
    );
    super::groups::card(&r, TEST_CUT_TITLE, &text, None, false);
    ui.add(
        egui::Label::new(
            RichText::new(format!(
                "You have not cut {name} yet. Cut a small test shape first, then the design."
            ))
            .color(t.text_dim),
        )
        .wrap(),
    );
    let why = (!tc.room).then_some(NO_ROOM);
    let b = ui.add_enabled(why.is_none(), egui::Button::new(TEST_CUT_BUTTON));
    super::settings::set_name(ui, &b, TEST_CUT_BUTTON);
    super::groups::card(
        &b,
        TEST_CUT_BUTTON,
        "Puts a 30 mm square with a 10 mm hole on a free spot of the mat, away from your design. Cut it first, check it, then cut the rest. Undo takes it back.",
        why,
        false,
    );
    if b.clicked() {
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

fn show_reminder(ui: &mut Ui, item: &Item, doc_id: u64, t: &Tokens, actions: &mut Vec<Action>) {
    ui.add_space(t.spacing / 2.0);
    let r = ui.label(
        RichText::new(MIRROR_TITLE)
            .font(fonts::semibold(13.0))
            .color(t.accent_cut),
    );
    super::groups::card(&r, MIRROR_TITLE, &item.hint, None, false);
    ui.add(egui::Label::new(&item.message).wrap());
    for fix in &item.fixes {
        // The registry's own id: a fix names a real command.
        let Some(command) = flatcraft_engine::registry::get(&fix.command) else {
            continue;
        };
        let b = ui.button(MIRROR_BUTTON);
        super::groups::card(&b, MIRROR_BUTTON, &fix.description, None, false);
        if b.clicked() {
            actions.push(Action::run(doc_id, command.id(), fix.args.clone()));
        }
    }
    for bl in &item.blocked {
        let b = ui.add_enabled(false, egui::Button::new(MIRROR_BUTTON));
        super::groups::card(&b, MIRROR_BUTTON, &item.hint, Some(&bl.reason), false);
    }
}
