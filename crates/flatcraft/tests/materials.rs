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
    assert!(shows(
        &h,
        "Starts at speed 4, force 18, 1 pass, blade 3 (the number you turn the blade to by hand)."
    ));
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
        &materials::test_cut_text("Vellum", false),
        &materials::test_cut_text("Vinyl", true),
    ];
    for t in texts {
        assert!(!t.contains('\u{2014}'), "{t}");
    }
    for t in flatcraft_device::MatTack::ALL {
        assert!(!materials::tack_text(t).contains('\u{2014}'));
    }
}

#[test]
fn the_test_shape_room_is_measured_once_per_revision() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    let builds = |h: &AppHarness| app(h).ui.prepare.test_cut_cache.builds;
    let first = builds(&h);
    assert!(first >= 1);
    // Frames without a change reuse the answer.
    for _ in 0..5 {
        h.run();
    }
    assert_eq!(builds(&h), first);
    // Another material (a new revision) measures again, once.
    set_material(&s, "kraft-paper");
    h.run();
    h.run();
    assert_eq!(builds(&h), first + 1);
}

#[test]
fn the_test_cut_card_follows_the_backing_sheet() {
    // On a backing sheet the blade must not go through it: nothing drops
    // out, the backing shows a light line only.
    for (id, backing) in [
        ("vinyl", true),
        ("sticker-paper", true),
        ("copy-paper", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        set_material(&s, id);
        let mut h = tall(&dir, &s);
        cut_stage(&mut h, "Device & Material");
        h.get_by_label(materials::TEST_CUT_TITLE).hover();
        h.run();
        assert_eq!(shows(&h, "drops out"), !backing, "{id}");
        assert_eq!(
            shows(&h, "only a light line, not a cut through it"),
            backing,
            "{id}"
        );
    }
    assert!(!materials::test_cut_text("Vinyl", true).contains("drops out"));
    assert!(materials::test_cut_text("Copy paper", false).contains("drops out"));
}

#[test]
fn every_group_heading_has_a_card() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    open_picker(&mut h);
    for g in flatcraft_device::MaterialGroup::ALL {
        // The last heading sits below the popup's fold.
        h.get_by_label(g.label()).scroll_to_me();
        h.run();
        h.get_by_label(g.label()).hover();
        h.run();
        assert!(shows(&h, materials::group_hint(g)), "{g:?}");
    }
    assert!(materials::group_hint(flatcraft_device::MaterialGroup::IronOn).contains("mirrored"));
}

#[test]
fn a_skipped_user_file_for_the_material_shows_in_the_job_card() {
    let dir = tempfile::tempdir().unwrap();
    let materials_dir = dir.path().join("profiles/materials");
    std::fs::create_dir_all(&materials_dir).unwrap();
    std::fs::write(
        materials_dir.join("vellum.toml"),
        "id = \"vellum\"\nname = \"My vellum\"\ninherits = \"copy-paper\"\n",
    )
    .unwrap();
    let s = session(&dir);
    set_material(&s, "vellum");
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    h.get_by_label(materials::skipped_text(true)).hover();
    h.run();
    assert!(shows(&h, "new id"));
    // Another material: no note.
    set_material(&s, "kraft-paper");
    h.run();
    assert!(h.query_by_label(materials::skipped_text(true)).is_none());
}

#[test]
fn the_picker_entries_are_built_once_until_something_changes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    let builds = |h: &AppHarness| app(h).ui.prepare.cards_cache.builds;
    let first = builds(&h);
    for _ in 0..5 {
        h.run();
    }
    assert_eq!(builds(&h), first);
    // A cut changes the log: the "never cut" marks are built again.
    s.lock()
        .unwrap()
        .execute("job.send", json!({}), Origin::User)
        .unwrap();
    h.run();
    h.run();
    assert_eq!(builds(&h), first + 1);
}

/// Brand and product words of cutting machines and their materials.
const BRANDS: &[&str] = &[
    "silhouette",
    "cameo",
    "portrait",
    "curio",
    "cricut",
    "brother",
    "scanncut",
    "siser",
    "easyweed",
    "stahls",
    "oracal",
    "orafol",
    "avery",
    "onlinelabels",
    "mylar",
    "thermoflex",
    "glitterflex",
    "graphtec",
    "sizzix",
    "singer",
    "roland",
    "heat n bond",
    "teckwrap",
    "hotmarks",
];

#[test]
fn the_new_texts_name_no_brand() {
    use flatcraft::ui::tools::Setting;
    let mut texts: Vec<String> = vec![
        materials::PICKER_HINT.into(),
        materials::NEVER_CUT_TEXT.into(),
        materials::MIRROR_TEXT.into(),
        materials::NO_ROOM.into(),
        materials::TEST_CUT_TITLE.into(),
        materials::TEST_CUT_BUTTON.into(),
        materials::MIRROR_TITLE.into(),
        materials::MIRROR_BUTTON.into(),
        materials::backing_text(true).into(),
        materials::backing_text(false).into(),
        materials::test_cut_text("x", true),
        materials::test_cut_text("x", false),
        materials::skipped_text(true).into(),
        materials::skipped_text(false).into(),
        Setting::MatTack.card().into(),
        Setting::MaterialGroup.card().into(),
        Setting::PresetTips.card().into(),
        flatcraft::ui::settings::TIPS_HEADING.into(),
    ];
    for g in flatcraft_device::MaterialGroup::ALL {
        texts.push(g.label().into());
        texts.push(materials::group_hint(g).into());
    }
    for t in flatcraft_device::MatTack::ALL {
        texts.push(materials::tack_text(t).into());
    }
    for text in &texts {
        let low = text.to_lowercase();
        for b in BRANDS {
            assert!(!low.contains(b), "'{b}' in: {text}");
        }
    }
}

#[test]
fn a_skipped_file_is_worded_after_why_it_was_skipped() {
    // Unreadable, with the id as its name: "could not be read".
    let dir = tempfile::tempdir().unwrap();
    let m = dir.path().join("profiles/materials");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(m.join("vellum.toml"), "id = [").unwrap();
    let s = session(&dir);
    set_material(&s, "vellum");
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    h.get_by_label(materials::skipped_text(false));
    assert!(h.query_by_label(materials::skipped_text(true)).is_none());
    // The id inside a file of another name is found too.
    let dir = tempfile::tempdir().unwrap();
    let m = dir.path().join("profiles/materials");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(
        m.join("old-vellum.toml"),
        "id = \"vellum\"\nname = \"Old\"\ninherits = \"copy-paper\"\n",
    )
    .unwrap();
    let s = session(&dir);
    set_material(&s, "vellum");
    let mut h = tall(&dir, &s);
    cut_stage(&mut h, "Device & Material");
    h.get_by_label(materials::skipped_text(false));
}
