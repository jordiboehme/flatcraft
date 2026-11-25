//! The Profiles window, driven through egui_kittest.

mod common;

use common::{AppHarness, FakePicker, app, app_mut, harness_steps, session};
use egui::accesskit::{Role, Toggled};
use egui::{Color32, Modifiers, Shape};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::json;

/// The app, with room for the window's grids to settle.
fn harness(dir: &tempfile::TempDir, s: &SharedSession) -> AppHarness {
    harness_steps(dir, s.clone(), 8, |app| app)
}

fn open(h: &mut AppHarness, tab: &str) {
    h.get_by_role_and_label(Role::Button, "Settings").click();
    h.run();
    h.get_by_role_and_label(Role::Tab, tab).click();
    h.run();
}

fn click(h: &mut AppHarness, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn type_into(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.get_by_label(label).type_text(text);
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
}

fn disabled(h: &AppHarness, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

/// Selects the system Cameo 2 and saves it as the user preset "My Cameo".
fn save_as_my_cameo(h: &mut AppHarness) {
    open(h, "Plotters");
    click(h, "Silhouette Cameo 2");
    click(h, "Save as user preset...");
    // Enter in the name field creates it.
    type_into(h, "Preset name", "My Cameo");
}

fn plotter_toml(dir: &tempfile::TempDir, id: &str) -> toml::Table {
    let path = dir
        .path()
        .join("profiles/plotters")
        .join(format!("{id}.toml"));
    std::fs::read_to_string(path).unwrap().parse().unwrap()
}

fn keys(t: &toml::Table) -> Vec<&str> {
    t.keys().map(String::as_str).collect()
}

fn store(s: &SharedSession) -> ProfileStore {
    s.lock().unwrap().profiles.clone()
}

/// Every shape painted last frame, with nested lists flattened.
fn painted(h: &AppHarness) -> Vec<Shape> {
    fn walk(shape: &Shape, out: &mut Vec<Shape>) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// Makes `id` the plotter of the active document, closes the Profiles
/// window and goes to Prepare.
fn use_plotter(h: &mut AppHarness, s: &SharedSession, id: &str) {
    s.lock()
        .unwrap()
        .execute("job.set_plotter", json!({ "id": id }), Origin::User)
        .unwrap();
    app_mut(h).ui.show_profiles = false;
    app_mut(h).ui.workspace = Workspace::Prepare;
    h.run();
}

#[test]
fn save_as_user_preset_inherits() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    type_into(&mut h, "Scale x", "1.01");
    click(&mut h, "Save preset");

    let st = store(&s);
    let info = st
        .plotters()
        .into_iter()
        .find(|p| p.id == "my-cameo")
        .unwrap();
    assert!(!info.system);
    assert_eq!(info.name, "My Cameo");
    assert_eq!(info.inherits.as_deref(), Some("silhouette-cameo-2"));
    assert_eq!(st.modified_plotter_fields("my-cameo"), ["scale_x"]);
    assert_eq!(st.plotter("my-cameo").unwrap().scale_x, 1.01);

    let t = plotter_toml(&dir, "my-cameo");
    assert_eq!(keys(&t), ["id", "inherits", "name", "scale_x"]);
    assert_eq!(t["inherits"].as_str(), Some("silhouette-cameo-2"));
    assert_eq!(t["scale_x"].as_float(), Some(1.01));
    // The modified dot.
    h.get_by_label("Scale x (modified)");
    assert!(h.query_by_label("Scale y (modified)").is_none());
}

#[test]
fn system_preset_fields_are_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Plotters");
    for preset in ["Silhouette Cameo 2", "File Output (GPGL Cameo 2)"] {
        click(&mut h, preset);
        for field in [
            "Name",
            "Vendor",
            "Scale x",
            "Units per mm",
            "Knife compensation",
            "Mirror x",
            "Mat name",
            "Grid origin y (mm)",
        ] {
            assert!(disabled(&h, field), "{preset}: {field} is editable");
        }
        assert!(h.query_by_label("Save preset").is_none());
        assert!(h.query_by_label("Delete preset").is_none());
        assert!(!disabled(&h, "Save as user preset..."));
    }
    open(&mut h, "Materials");
    click(&mut h, "Vinyl");
    assert!(disabled(&h, "Speed"));
    assert!(h.query_by_label("Save preset").is_none());
    // Nothing was written.
    assert!(!dir.path().join("profiles").exists());
}

#[test]
fn allow_agent_cuts_persists() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "App");
    h.get_by_label("Allow agents to start cuts");
    assert!(h.query_by_label_contains("without asking you").is_some());
    click(&mut h, "Allow agents to start cuts");

    assert!(s.lock().unwrap().prefs.allow_agent_cuts);
    let stored = Preferences::load(dir.path());
    assert!(stored.allow_agent_cuts);
    // Other preferences are kept.
    assert_eq!(stored.output_dir, Some(dir.path().join("output")));
}

#[test]
fn auto_reload_switch_persists() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "App");
    h.get_by_label(
        "When an imported file changes on disk, FlatCraft reloads it and keeps your placement and cut settings.",
    );
    click(&mut h, "Reload changed files automatically");
    assert!(s.lock().unwrap().prefs.auto_reload);
    assert!(Preferences::load(dir.path()).auto_reload);
    let node = h.get_by_label("Reload changed files automatically");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn edit_one_mat_keeps_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Mat 12x12");
    type_into(&mut h, "Grid origin y (mm)", "12.7");
    click(&mut h, "Save preset");

    let p = store(&s).plotter("my-cameo").unwrap();
    let ids: Vec<&str> = p.media.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["mat-12x12", "mat-12x24", "no-mat"]);
    assert_eq!(p.media[0].grid.origin_y_mm, 12.7);
    assert_eq!(
        store(&s).modified_plotter_fields("my-cameo"),
        ["media.mat-12x12"]
    );

    let t = plotter_toml(&dir, "my-cameo");
    let media = t["media"].as_array().unwrap();
    assert_eq!(media.len(), 1);
    let mat = media[0].as_table().unwrap();
    assert_eq!(mat["id"].as_str(), Some("mat-12x12"));
    // A complete entry, not only the changed field.
    for k in ["name", "kind", "width_mm", "height_mm", "grid", "color"] {
        assert!(mat.contains_key(k), "missing {k}");
    }
    h.get_by_label("Mat 12x12 (modified)");
    // The dot sits on the field that differs from the built-in mat.
    h.get_by_label("Grid origin y (mm) (modified)");
    h.get_by_label("Grid origin x (mm)");
}

#[test]
fn hide_builtin_mat() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Mat 12x24");
    click(&mut h, "Hidden");
    click(&mut h, "Save preset");

    let t = plotter_toml(&dir, "my-cameo");
    let media = t["media"].as_array().unwrap();
    assert_eq!(media.len(), 1);
    let mat = media[0].as_table().unwrap();
    assert_eq!(keys(mat), ["hidden", "id"]);
    assert_eq!(mat["id"].as_str(), Some("mat-12x24"));
    // A built-in mat can be hidden, never deleted.
    assert!(
        h.query_by_label("Delete mat")
            .is_none_or(|n| n.accesskit_node().is_disabled())
    );

    use_plotter(&mut h, &s, "my-cameo");
    h.get_by_role_and_label(Role::ComboBox, "Mat").click();
    h.run();
    // The popup is open: "No mat" is only listed there.
    assert!(h.query_by_label("No mat").is_some());
    assert!(h.query_by_label("Mat 12x24").is_none());
}

#[test]
fn invalid_mat_disables_save() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Mat 12x12");
    let before =
        std::fs::read_to_string(dir.path().join("profiles/plotters/my-cameo.toml")).unwrap();
    // 50 + 304.8 is more than the 330.2 mm of the mat.
    type_into(&mut h, "Grid origin y (mm)", "50");
    h.get_by_label("The grid (origin plus size) must lie inside the physical mat.");
    assert!(disabled(&h, "Save preset"));
    h.get_by_label("Save preset").click();
    h.run();
    let after =
        std::fs::read_to_string(dir.path().join("profiles/plotters/my-cameo.toml")).unwrap();
    assert_eq!(before, after);
    assert_eq!(
        store(&s).plotter("my-cameo").unwrap().media[0]
            .grid
            .origin_y_mm,
        0.0
    );
    // Back to a valid value: Save works again.
    type_into(&mut h, "Grid origin y (mm)", "10");
    assert!(!disabled(&h, "Save preset"));
}

#[test]
fn mat_colour_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Mat 12x12");
    h.get_by_label("Mat colour");
    type_into(&mut h, "Mat colour (hex)", "#336699");
    click(&mut h, "Save preset");

    let t = plotter_toml(&dir, "my-cameo");
    let mat = t["media"].as_array().unwrap()[0].as_table().unwrap();
    assert_eq!(mat["color"].as_str(), Some("#336699"));
    let reloaded = ProfileStore::load(&dir.path().join("profiles")).unwrap();
    assert_eq!(
        reloaded.plotter("my-cameo").unwrap().media[0].color,
        [0x33, 0x66, 0x99]
    );

    use_plotter(&mut h, &s, "my-cameo");
    let colour = Color32::from_rgb(0x33, 0x66, 0x99);
    let hit = painted(&h).iter().any(|shape| match shape {
        Shape::Rect(r) => r.fill == colour,
        _ => false,
    });
    assert!(hit, "no rect in the new mat colour");
}

#[test]
fn delete_user_preset() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    let file = dir.path().join("profiles/plotters/my-cameo.toml");
    assert!(file.exists());
    click(&mut h, "Delete preset");
    click(&mut h, "Yes, delete");
    assert!(!file.exists());
    assert!(store(&s).plotter("my-cameo").is_err());
    assert!(h.query_by_label("My Cameo").is_none());
}

#[test]
fn preset_in_use_is_not_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    s.lock()
        .unwrap()
        .execute("job.set_plotter", json!({ "id": "my-cameo" }), Origin::User)
        .unwrap();
    click(&mut h, "Delete preset");
    click(&mut h, "Yes, delete");
    assert!(dir.path().join("profiles/plotters/my-cameo.toml").exists());
    assert!(store(&s).plotter("my-cameo").is_ok());
    assert!(app(&h).ui.toasts.iter().any(|t| t.text.contains("in use")));
}

#[test]
fn material_override_saves_only_the_override() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Copy Paper");
    click(&mut h, "Save as user preset...");
    h.get_by_label("Preset name").click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run();
    h.get_by_label("Preset name").type_text("My Paper");
    h.run();
    click(&mut h, "Create");
    h.get_by_label("Add override for").click();
    h.run();
    click(&mut h, "Silhouette Cameo 2");
    click(&mut h, "Silhouette Cameo 2 speed set");
    type_into(&mut h, "Silhouette Cameo 2 speed", "3");
    click(&mut h, "Save preset");

    let st = store(&s);
    assert_eq!(
        st.modified_material_fields("my-paper"),
        ["overrides.silhouette-cameo-2.speed"]
    );
    let m = st.material("my-paper").unwrap();
    assert_eq!(m.for_plotter("silhouette-cameo-2").speed, 3);
}

#[test]
fn theme_switch_applies_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "App");
    h.get_by_label("Theme").click();
    h.run();
    click(&mut h, "Light");
    assert_eq!(
        Preferences::load(dir.path()).theme,
        flatcraft_engine::prefs::Theme::Light
    );
    assert_eq!(app(&h).tokens, flatcraft::ui::theme::Tokens::light());
}

#[test]
fn default_material_persists_and_new_documents_use_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "App");
    h.get_by_label("Default material").click();
    h.run();
    click(&mut h, "Vinyl");
    assert_eq!(Preferences::load(dir.path()).default_material, "vinyl");
    let mut session = s.lock().unwrap();
    session
        .execute("document.new", serde_json::json!({}), Origin::User)
        .unwrap();
    assert_eq!(session.doc().document.job.material_id, "vinyl");
}

#[test]
fn a_broken_preset_file_is_named_in_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let plotters = dir.path().join("profiles/plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(plotters.join("broken.toml"), "id = [").unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Plotters");
    h.get_by_label_contains("This preset file can't be read:");
    h.get_by_label_contains("broken.toml");
}

#[test]
fn a_huge_flatten_tolerance_blocks_save() {
    let p = {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::load(dir.path()).unwrap();
        store.plotter("silhouette-cameo-2").unwrap()
    };
    let mut wild = p.clone();
    wild.flatten_tolerance_mm = 50.0;
    let problems = flatcraft::ui::profiles_window::plotter_problems(&wild);
    assert!(
        problems.iter().any(|x| x.contains("flatten tolerance")),
        "{problems:?}"
    );
    assert!(flatcraft::ui::profiles_window::plotter_problems(&p).is_empty());
}

#[test]
fn save_as_treats_an_unreadable_file_as_taken() {
    let dir = tempfile::tempdir().unwrap();
    let plotters = dir.path().join("profiles/plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    let file = plotters.join("my-cameo.toml");
    std::fs::write(&file, "id = [").unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    h.get_by_label("A preset with the id 'my-cameo' already exists. Pick another name.");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "id = [");
    // The window's own request is refused by the store too.
    let mut p = store(&s).plotter("silhouette-cameo-2").unwrap();
    p.id = "my-cameo".into();
    p.name = "My Cameo".into();
    let ctx = h.ctx.clone();
    flatcraft::ui::profiles_window::apply(
        app_mut(&mut h),
        &ctx,
        flatcraft::ui::profiles_window::Request::SavePlotter {
            profile: Box::new(p),
            parent: "silhouette-cameo-2".into(),
            new: true,
        },
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "id = [");
}

#[test]
fn an_out_of_range_preset_can_be_fixed_in_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let materials = dir.path().join("profiles/materials");
    std::fs::create_dir_all(&materials).unwrap();
    std::fs::write(
        materials.join("wild-vinyl.toml"),
        "id = \"wild-vinyl\"\nname = \"Wild vinyl\"\ninherits = \"vinyl\"\novercut_mm = 8.0\n",
    )
    .unwrap();
    let s = session(&dir);
    // A document already on it shows why instead of a blank canvas.
    s.lock().unwrap().doc_mut().document.job.material_id = "wild-vinyl".into();
    let mut h = harness(&dir, &s);
    app_mut(&mut h).ui.workspace = Workspace::Prepare;
    h.run();
    h.get_by_label_contains("Open Settings to fix it.");
    // Picking it for a job is refused with the same hint.
    let e = s
        .lock()
        .unwrap()
        .execute(
            "job.set_material",
            json!({ "id": "wild-vinyl" }),
            Origin::User,
        )
        .unwrap_err();
    assert!(e.to_string().contains("Open Settings to fix it."), "{e}");
    // The window loads it, shows the problem and blocks Save until fixed.
    open(&mut h, "Materials");
    click(&mut h, "Wild vinyl");
    h.get_by_label_contains("The overcut must be 0 to 5 mm");
    type_into(&mut h, "Overcut (mm) (modified)", "2");
    click(&mut h, "Save preset");
    assert_eq!(store(&s).material("wild-vinyl").unwrap().overcut_mm, 2.0);
    s.lock()
        .unwrap()
        .execute(
            "job.set_material",
            json!({ "id": "wild-vinyl" }),
            Origin::User,
        )
        .unwrap();
}

#[test]
fn calibration_tab_says_what_comes() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Calibration");
    h.get_by_label("The calibration wizard arrives in milestone 3.");
    assert!(h.query_by_label_contains("mat assistant").is_some());
}

#[test]
fn output_folder_choose_persists() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let chosen = dir.path().join("cuts");
    let picker = FakePicker::answering([chosen.clone()]);
    let mut h = harness_steps(&dir, s.clone(), 8, |app| app.with_file_picker(picker));
    open(&mut h, "App");
    click(&mut h, "Choose folder...");
    assert_eq!(
        Preferences::load(dir.path()).output_dir,
        Some(chosen.clone())
    );
    assert_eq!(s.lock().unwrap().prefs.output_dir, Some(chosen));
}

#[test]
fn save_as_refuses_a_taken_name() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Cameo");
    h.get_by_label("A preset with the id 'my-cameo' already exists. Pick another name.");
    assert_eq!(store(&s).plotters().len(), 3);
}

#[test]
fn add_copy_rename_and_delete_own_mats() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Add mat");
    h.get_by_label("New mat");
    type_into(&mut h, "Mat name", "Scrap mat");
    click(&mut h, "Copy mat");
    h.get_by_label("Scrap mat copy");
    click(&mut h, "Delete mat");
    assert!(h.query_by_label("Scrap mat copy").is_none());
    click(&mut h, "Save preset");

    let p = store(&s).plotter("my-cameo").unwrap();
    let names: Vec<&str> = p.media.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Mat 12x12", "Mat 12x24", "No mat", "Scrap mat"]);
    let t = plotter_toml(&dir, "my-cameo");
    let media = t["media"].as_array().unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0]["id"].as_str(), Some("my-mat"));
}

#[test]
fn invalid_field_blocks_save_and_save_as() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    let file = dir.path().join("profiles/plotters/my-cameo.toml");
    let before = std::fs::read_to_string(&file).unwrap();
    type_into(&mut h, "Scale x", "0");
    h.get_by_label("The scale must be above 0.");
    assert!(disabled(&h, "Save preset"));
    assert!(disabled(&h, "Save as user preset..."));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    assert_eq!(store(&s).plotters().len(), 3);
}

#[test]
fn apply_refuses_an_invalid_preset() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    let mut p = store(&s).plotter("silhouette-cameo-2").unwrap();
    p.id = "broken".into();
    p.name = "Broken".into();
    p.speed_range = [5, 2];
    let req = flatcraft::ui::profiles_window::Request::SavePlotter {
        profile: Box::new(p),
        parent: "silhouette-cameo-2".into(),
        new: true,
    };
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Profiles(req)]);
    assert!(store(&s).plotter("broken").is_err());
    assert!(!dir.path().join("profiles/plotters/broken.toml").exists());
    assert!(
        app(&h)
            .ui
            .toasts
            .iter()
            .any(|t| t.text.contains("speed range"))
    );
}

const MAT_IN_USE: &str = "This mat is in use. Pick another mat in the job first.";

#[test]
fn hiding_a_mat_in_use_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    for (cmd, p) in [
        ("job.set_plotter", json!({ "id": "my-cameo" })),
        ("job.set_media", json!({ "id": "mat-12x24" })),
    ] {
        s.lock().unwrap().execute(cmd, p, Origin::User).unwrap();
    }
    click(&mut h, "Mat 12x24");
    click(&mut h, "Hidden");
    assert!(app(&h).ui.toasts.iter().any(|t| t.text == MAT_IN_USE));
    let node = h.get_by_label("Hidden");
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::False));
    assert!(disabled(&h, "Save preset"));
}

#[test]
fn saving_without_a_mat_in_use_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    let file = dir.path().join("profiles/plotters/my-cameo.toml");
    let before = std::fs::read_to_string(&file).unwrap();
    // The default device starts new documents on mat-12x12.
    s.lock().unwrap().prefs.default_plotter = "my-cameo".into();
    let mut p = store(&s).plotter("my-cameo").unwrap();
    p.media.retain(|m| m.id != "mat-12x12");
    let req = flatcraft::ui::profiles_window::Request::SavePlotter {
        profile: Box::new(p),
        parent: "silhouette-cameo-2".into(),
        new: false,
    };
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Profiles(req)]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    assert!(
        app(&h)
            .ui
            .toasts
            .iter()
            .any(|t| t.text.contains(MAT_IN_USE))
    );
}

#[test]
fn mat_errors_land_on_their_row() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Mat 12x12");
    let rows = [
        (
            "Physical width (mm)",
            "304.8",
            "The physical size must be above 0.",
        ),
        (
            "Physical height (mm)",
            "330.2",
            "The physical size must be above 0.",
        ),
        (
            "Grid origin x (mm)",
            "0",
            "The grid origin must not be negative.",
        ),
        (
            "Grid origin y (mm)",
            "0",
            "The grid origin must not be negative.",
        ),
        ("Grid width (mm)", "304.8", "The grid size must be above 0."),
        (
            "Grid height (mm)",
            "304.8",
            "The grid size must be above 0.",
        ),
    ];
    let next = [
        "Physical height (mm)",
        "Grid origin x (mm)",
        "Grid origin y (mm)",
        "Grid width (mm)",
        "Grid height (mm)",
        "Calibrated",
    ];
    for ((field, good, text), below) in rows.into_iter().zip(next) {
        let bad = if field.starts_with("Grid origin") {
            "-1"
        } else {
            "0"
        };
        type_into(&mut h, field, bad);
        let err = h.get_by_label(text).rect();
        let f = h.get_by_label(field).rect();
        let n = h.get_by_label(below).rect();
        assert!(
            err.top() >= f.bottom() && err.bottom() <= n.top(),
            "{text} is not between {field} and {below}"
        );
        type_into(&mut h, field, good);
        assert!(
            h.query_by_label(text).is_none(),
            "{field} still shows an error"
        );
    }
}

#[test]
fn switching_presets_asks_before_dropping_edits() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    type_into(&mut h, "Scale x", "1.5");
    click(&mut h, "Silhouette Cameo 2");
    h.get_by_label("Discard your changes?");
    click(&mut h, "Keep editing");
    assert!(h.query_by_label("Discard your changes?").is_none());
    let d = app(&h).ui.profiles.plotter.as_ref().unwrap();
    assert_eq!((d.id.as_str(), d.edit.scale_x), ("my-cameo", 1.5));
    click(&mut h, "Silhouette Cameo 2");
    click(&mut h, "Discard");
    let d = app(&h).ui.profiles.plotter.as_ref().unwrap();
    assert_eq!(d.id, "silhouette-cameo-2");
    assert_eq!(store(&s).plotter("my-cameo").unwrap().scale_x, 1.0);
}

#[test]
fn closing_asks_before_dropping_edits() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    type_into(&mut h, "Scale x", "1.5");
    click(&mut h, "Close window");
    assert!(app(&h).ui.show_profiles);
    h.get_by_label("Discard your changes?");
    click(&mut h, "Discard");
    assert!(!app(&h).ui.show_profiles);
    // Opening again shows the stored values.
    open(&mut h, "Plotters");
    assert_eq!(
        app(&h).ui.profiles.plotter.as_ref().unwrap().edit.scale_x,
        1.0
    );
}

#[test]
fn save_as_names_clash_only_within_a_kind() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    open(&mut h, "Materials");
    click(&mut h, "Vinyl");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Cameo");
    assert!(store(&s).material("my-cameo").is_ok());
    assert!(store(&s).plotter("my-cameo").is_ok());
    assert_eq!(
        store(&s).modified_material_fields("my-cameo"),
        Vec::<String>::new()
    );
}
