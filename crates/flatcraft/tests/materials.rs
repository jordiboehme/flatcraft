//! The material picker in groups with a card per material, the "Start
//! with a test cut" hint, and the Cut stage's mirror reminder for iron-on
//! material, driven through egui_kittest.

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft::ui::materials;
use flatcraft_engine::session::Origin;
use serde_json::json;

fn tall(dir: &tempfile::TempDir, s: &flatcraft_mcp::SharedSession) -> AppHarness {
    harness_sized(dir, s.clone(), [1400.0, 1200.0], 8, |a| a)
}

/// The Cut workspace at `stage`.
fn cut_stage(h: &mut AppHarness, stage: &str) {
    app_mut(h).ui.workspace = Workspace::Cut;
    h.run();
    h.get_by_role_and_label(Role::Tab, stage).click();
    h.run();
}

fn set_material(s: &flatcraft_mcp::SharedSession, id: &str) {
    s.lock()
        .unwrap()
        .execute("job.set_material", json!({ "id": id }), Origin::User)
        .unwrap();
}

/// True when some node's label contains `text` (a hover card is one label).
fn shows(h: &AppHarness, text: &str) -> bool {
    h.root().children_recursive().any(|n| {
        let a = n.accesskit_node();
        a.label().is_some_and(|l| l.contains(text)) || a.value().is_some_and(|v| v.contains(text))
    })
}

fn open_picker(h: &mut AppHarness) {
    h.get_by_label("Material").click();
    h.run();
}

#[test]
fn the_picker_lists_materials_in_their_groups() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    open_picker(&mut h);
    let y = |h: &AppHarness, label: &str| h.get_by_label(label).rect().min.y;
    let order = [
        "Vinyl",
        "Adhesive vinyl (permanent)",
        "Adhesive vinyl (removable)",
        "Iron-on",
        "Heat transfer vinyl (smooth)",
        "Heat transfer vinyl (glitter)",
        "Heat transfer vinyl (flock)",
        "Paper and card",
        "Sticker paper",
        "Copy paper",
        "Cardstock (light, about 160-200 g/m2)",
        "Cardstock (heavy, about 250-300 g/m2)",
        "Kraft paper",
        "Vellum",
        "Film and other",
        "Stencil and masking film",
        "Window cling",
        "Printable magnet sheet (thin)",
    ];
    let ys: Vec<f32> = order.iter().map(|l| y(&h, l)).collect();
    for (i, w) in ys.windows(2).enumerate() {
        assert!(w[0] < w[1], "{} is not above {}", order[i], order[i + 1]);
    }
    // Picking one sets the document's material, also one far down the
    // list (scrolled into view first).
    h.get_by_label("Adhesive vinyl (removable)").click();
    h.run();
    assert_eq!(
        s.lock().unwrap().doc().document.job.material_id,
        "vinyl-removable"
    );
    open_picker(&mut h);
    h.get_by_label("Vellum").scroll_to_me();
    h.run();
    h.get_by_label("Vellum").click();
    h.run();
    assert_eq!(s.lock().unwrap().doc().document.job.material_id, "vellum");
}

#[test]
fn a_material_card_explains_tips_values_mat_and_backing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    open_picker(&mut h);
    h.get_by_label("Heat transfer vinyl (glitter)").hover();
    h.run();
    assert!(shows(
        &h,
        "Mirror the design and cut with the shiny carrier side down"
    ));
    assert!(shows(&h, "Starts at speed 4, force 18, 1 pass, blade 3."));
    assert!(shows(
        &h,
        "Mat: standard grip. The mat's tack is how sticky it is"
    ));
    assert!(shows(&h, materials::backing_text(true)));
    assert!(shows(&h, materials::MIRROR_TEXT));
    assert!(shows(&h, materials::NEVER_CUT_TEXT));
    h.get_by_label("Copy paper").hover();
    h.run();
    assert!(shows(&h, materials::backing_text(false)));
    assert!(shows(&h, "Mat: light grip."));
}

#[test]
fn a_material_never_cut_suggests_a_test_cut() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    h.get_by_label(materials::TEST_CUT_TITLE);
    let b = h.get_by_role_and_label(Role::Button, materials::TEST_CUT_BUTTON);
    assert!(!b.accesskit_node().is_disabled());
    b.click();
    h.run();
    let names: Vec<String> = s
        .lock()
        .unwrap()
        .doc()
        .document
        .objects
        .iter()
        .map(|o| o.name.clone())
        .collect();
    assert!(names.iter().any(|n| n == "Test shape"), "{names:?}");
    // After a cut with this material the hint is gone.
    s.lock()
        .unwrap()
        .execute("job.send", json!({}), Origin::User)
        .unwrap();
    h.run();
    assert!(h.query_by_label(materials::TEST_CUT_TITLE).is_none());
    // Another material that was never cut shows it again.
    set_material(&s, "kraft-paper");
    h.run();
    h.get_by_label(materials::TEST_CUT_TITLE);
}

#[test]
fn the_test_cut_button_is_dimmed_without_room() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    // A square over nearly the whole cutting area leaves no free spot.
    let id = import_square(&s);
    s.lock()
        .unwrap()
        .execute(
            "object.scale",
            json!({ "ids": [id], "width_mm": 270.0, "anchor": "top_left" }),
            Origin::User,
        )
        .unwrap();
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    let b = h.get_by_role_and_label(Role::Button, materials::TEST_CUT_BUTTON);
    assert!(b.accesskit_node().is_disabled());
    b.hover();
    h.run();
    h.get_by_label(materials::NO_ROOM);
}

#[test]
fn iron_on_without_mirror_gets_a_reminder_that_never_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    set_material(&s, "heat-transfer-vinyl");
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Send");
    h.get_by_label(materials::MIRROR_TITLE);
    // The send button stays usable.
    let send = h.get_by_role_and_label(Role::Button, "Send to File Output");
    assert!(!send.accesskit_node().is_disabled());
    // The Device stage shows it too.
    cut_stage(&mut h, "Device & Material");
    h.get_by_label(materials::MIRROR_TITLE);
    h.get_by_role_and_label(Role::Button, materials::MIRROR_BUTTON)
        .click();
    h.run();
    let mirrored = s
        .lock()
        .unwrap()
        .doc()
        .document
        .objects
        .iter()
        .find(|o| o.id.0 == id)
        .unwrap()
        .transform
        .mirror;
    assert!(mirrored);
    assert!(h.query_by_label(materials::MIRROR_TITLE).is_none());
}

#[test]
fn vinyl_gets_no_mirror_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    set_material(&s, "vinyl");
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Send");
    assert!(h.query_by_label(materials::MIRROR_TITLE).is_none());
}

#[test]
fn the_job_card_shows_the_material_tips() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    set_material(&s, "window-cling");
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    h.get_by_label(flatcraft::ui::settings::TIPS_HEADING);
    assert!(shows(&h, "Cling side up, backing down."));
}

#[test]
fn the_new_texts_have_no_em_dash() {
    let texts = [
        materials::PICKER_HINT,
        materials::NEVER_CUT_TEXT,
        materials::MIRROR_TEXT,
        materials::NO_ROOM,
        materials::TEST_CUT_TITLE,
        materials::TEST_CUT_BUTTON,
        materials::MIRROR_TITLE,
        materials::MIRROR_BUTTON,
        materials::backing_text(true),
        materials::backing_text(false),
        &materials::test_cut_text("Vellum"),
    ];
    for t in texts {
        assert!(!t.contains('\u{2014}'), "{t}");
    }
    for t in flatcraft_device::MatTack::ALL {
        assert!(!materials::tack_text(t).contains('\u{2014}'));
    }
}
