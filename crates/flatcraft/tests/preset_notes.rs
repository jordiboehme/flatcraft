//! Notes on presets: edited in the Profiles window, shown in the Job card
//! when the preset is picked.

mod common;
use common::RunPlan;

use common::{AppHarness, harness_sized, session, to_prepare};
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

fn harness(dir: &tempfile::TempDir, s: &SharedSession) -> AppHarness {
    harness_sized(dir, s.clone(), [1400.0, 900.0], 8, |app| app)
}

fn open(h: &mut AppHarness, tab: &str) {
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, tab).click();
    h.run_plan();
}

/// Scrolls `label` into view and clicks it.
fn click(h: &mut AppHarness, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_plan();
    h.get_by_label(label).click();
    h.run_plan();
}

/// Clicks into the field and types `text` without leaving it.
fn type_into(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_plan();
    h.get_by_label(label).click();
    h.run_plan();
    h.get_by_label(label).type_text(text);
    h.run_plan();
}

fn disabled(h: &AppHarness, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

#[test]
fn notes_on_a_system_preset_are_kept_beside_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Adhesive vinyl (permanent)");
    assert!(disabled(&h, "Speed"), "the preset stays read-only");
    assert!(
        !disabled(&h, "Preset notes"),
        "but its notes are the person's"
    );
    assert!(disabled(&h, "Save notes"), "nothing to save yet");
    type_into(&mut h, "Preset notes", "Glossy vinyl: force 10 works.");
    assert!(!disabled(&h, "Save notes"));
    click(&mut h, "Save notes");
    let st = s.lock().unwrap();
    assert_eq!(
        st.profiles.material("vinyl").unwrap().notes.as_deref(),
        Some("Glossy vinyl: force 10 works.")
    );
    let profiles = dir.path().join("profiles");
    assert!(profiles.join(flatcraft_device::PRESET_NOTES_FILE).exists());
    assert!(
        !profiles.join("materials").exists(),
        "no user preset was made"
    );
}

#[test]
fn notes_on_a_user_preset_save_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    h.get_by_label("Preset name").click();
    h.run_plan();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_plan();
    h.get_by_label("Preset name").type_text("My Paper");
    h.run_plan();
    h.key_press(egui::Key::Enter);
    h.run_plan();
    h.run_plan();
    assert!(
        h.query_by_label("Save notes").is_none(),
        "Save preset does it"
    );
    type_into(&mut h, "Preset notes", "Grain runs down.");
    click(&mut h, "Save preset");
    let text =
        std::fs::read_to_string(dir.path().join("profiles/materials/user-my-paper.toml")).unwrap();
    assert!(text.contains("Grain runs down."), "{text}");
}

#[test]
fn the_job_card_shows_the_notes_of_the_picked_presets() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    {
        let mut st = s.lock().unwrap();
        st.profiles
            .set_material_notes("vinyl", "Weed within a day.")
            .unwrap();
        st.profiles
            .set_plotter_notes("file-output-gpgl-cameo2", "Check the output folder.")
            .unwrap();
        st.execute("job.set_material", json!({ "id": "vinyl" }), Origin::User)
            .unwrap();
        st.execute(
            "job.set_plotter",
            json!({ "id": "file-output-gpgl-cameo2" }),
            Origin::User,
        )
        .unwrap();
    }
    let mut h = harness(&dir, &s);
    to_prepare(&mut h);
    h.run_plan();
    h.get_by_label("Weed within a day.");
    h.get_by_label("Check the output folder.");
    s.lock()
        .unwrap()
        .execute(
            "job.set_material",
            json!({ "id": "cardstock" }),
            Origin::User,
        )
        .unwrap();
    h.run_plan();
    assert!(h.query_by_label("Weed within a day.").is_none());
    h.get_by_label("Check the output folder.");
}

#[test]
fn a_user_preset_without_a_parent_saves_its_notes_too() {
    let dir = tempfile::tempdir().unwrap();
    let materials = dir.path().join("profiles/materials");
    std::fs::create_dir_all(&materials).unwrap();
    std::fs::write(
        materials.join("solo.toml"),
        "id = \"solo\"\nname = \"Solo\"\nspeed = 4\nforce = 9\npasses = 1\nreverse_alternate_passes = false\novercut_mm = 0.5\ntrack_enhancing = false\n",
    )
    .unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Solo");
    assert!(disabled(&h, "Save notes"), "nothing to save yet");
    type_into(&mut h, "Preset notes", "Felt, cut twice.");
    click(&mut h, "Save notes");
    let text = std::fs::read_to_string(materials.join("solo.toml")).unwrap();
    assert!(text.contains("Felt, cut twice."), "{text}");
    assert!(text.contains("force = 9"), "{text}");
    assert_eq!(
        s.lock()
            .unwrap()
            .profiles
            .material("solo")
            .unwrap()
            .notes
            .as_deref(),
        Some("Felt, cut twice.")
    );
}

#[test]
fn save_notes_keeps_the_other_edits_that_are_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let materials = dir.path().join("profiles/materials");
    std::fs::create_dir_all(&materials).unwrap();
    std::fs::write(
        materials.join("solo.toml"),
        "id = \"solo\"\nname = \"Solo\"\nspeed = 4\nforce = 9\npasses = 1\nreverse_alternate_passes = false\novercut_mm = 0.5\ntrack_enhancing = false\n",
    )
    .unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Solo");
    // An edit of the force that is not saved, then the notes.
    h.get_by_label("Force (modified)").click();
    h.run_plan();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_plan();
    h.get_by_label("Force (modified)").type_text("12");
    h.run_plan();
    h.key_press(egui::Key::Enter);
    h.run_plan();
    type_into(&mut h, "Preset notes", "Felt, cut twice.");
    click(&mut h, "Save notes");
    h.run_plan();
    let text = std::fs::read_to_string(materials.join("solo.toml")).unwrap();
    assert!(text.contains("Felt, cut twice."), "{text}");
    assert!(
        text.contains("force = 9"),
        "only the notes were written: {text}"
    );
    let d = common::app(&h).ui.profiles.material.as_ref().unwrap();
    assert_eq!(d.edit.force, 12, "the unsaved force edit stays");
    assert_eq!(d.saved.force, 9);
    assert_eq!(d.edit.notes.as_deref(), Some("Felt, cut twice."));
    assert_eq!(d.saved.notes, d.edit.notes, "the notes count as saved");
    assert!(d.changed(), "the force edit is still unsaved");
}
