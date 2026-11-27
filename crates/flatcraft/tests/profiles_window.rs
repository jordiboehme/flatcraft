//! The preset pages of the Settings window, and the preferences that
//! moved there from the old App tab, driven through egui_kittest.

mod common;
use common::RunPlan;

use common::{AppHarness, FakePicker, app, app_mut, harness_steps, painted, session};
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
    h.run_plan();
    h.get_by_role_and_label(Role::Tab, tab).click();
    h.run_plan();
}

/// Shows another page of the open window.
fn page(h: &mut AppHarness, name: &str) {
    h.get_by_role_and_label(Role::Tab, name).click();
    h.run_plan();
}

fn click(h: &mut AppHarness, label: &str) {
    // The material page is taller than the window: scroll the control into
    // view first, so the click lands on it.
    h.get_by_label(label).scroll_to_me();
    h.run_plan();
    h.get_by_label(label).click();
    h.run_plan();
}

fn type_into(h: &mut AppHarness, label: &str, text: &str) {
    h.get_by_label(label).click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_plan();
    h.get_by_label(label).type_text(text);
    h.run_plan();
    h.key_press(egui::Key::Enter);
    h.run_plan();
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

/// Makes `id` the plotter of the active document, closes the Profiles
/// window and goes to Prepare.
fn use_plotter(h: &mut AppHarness, s: &SharedSession, id: &str) {
    s.lock()
        .unwrap()
        .execute("job.set_plotter", json!({ "id": id }), Origin::User)
        .unwrap();
    app_mut(h).ui.show_settings = false;
    app_mut(h).ui.workspace = Workspace::Prepare;
    h.run_plan();
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
        .find(|p| p.id == "user-my-cameo")
        .unwrap();
    assert!(!info.system);
    assert_eq!(info.name, "My Cameo");
    assert_eq!(info.inherits.as_deref(), Some("silhouette-cameo-2"));
    assert_eq!(st.modified_plotter_fields("user-my-cameo"), ["scale_x"]);
    assert_eq!(st.plotter("user-my-cameo").unwrap().scale_x, 1.01);

    let t = plotter_toml(&dir, "user-my-cameo");
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
        ] {
            assert!(disabled(&h, field), "{preset}: {field} is editable");
        }
        assert!(h.query_by_label("Save preset").is_none());
        assert!(h.query_by_label("Delete preset").is_none());
        assert!(!disabled(&h, "Save as user preset..."));
        // The mats of the same preset, on the Mats page.
        page(&mut h, "Mats");
        for field in ["Mat name", "Grid origin y (mm)"] {
            assert!(disabled(&h, field), "{preset}: {field} is editable");
        }
        page(&mut h, "Plotters");
    }
    open(&mut h, "Materials");
    click(&mut h, "Adhesive vinyl (permanent)");
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
    open(&mut h, "Agents");
    h.get_by_label("Allow agents to start cuts");
    assert!(h.query_by_label_contains("without asking you").is_some());
    click(&mut h, "Allow agents to start cuts");
    // Staged until OK.
    assert!(!s.lock().unwrap().prefs.allow_agent_cuts);
    click(&mut h, "OK");

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
    open(&mut h, "General");
    h.get_by_label(
        "When an imported file changes on disk, FlatCraft reloads it and keeps your placement and cut settings.",
    );
    click(&mut h, "Reload changed files automatically");
    assert!(!s.lock().unwrap().prefs.auto_reload);
    click(&mut h, "Apply");
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
    page(&mut h, "Mats");
    click(&mut h, "Mat 12x12");
    type_into(&mut h, "Grid origin y (mm)", "12.7");
    click(&mut h, "Save preset");

    let p = store(&s).plotter("user-my-cameo").unwrap();
    let ids: Vec<&str> = p.media.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["mat-12x12", "mat-12x24", "no-mat"]);
    assert_eq!(p.media[0].grid.origin_y_mm, 12.7);
    assert_eq!(
        store(&s).modified_plotter_fields("user-my-cameo"),
        ["media.mat-12x12"]
    );

    let t = plotter_toml(&dir, "user-my-cameo");
    let media = t["media"].as_array().unwrap();
    assert_eq!(media.len(), 1);
    let mat = media[0].as_table().unwrap();
    assert_eq!(mat["id"].as_str(), Some("mat-12x12"));
    // A complete entry, not only the changed field.
    for k in ["name", "kind", "width_mm", "height_mm", "grid"] {
        assert!(mat.contains_key(k), "missing {k}");
    }
    // A mat on the default colour stores none.
    assert!(!mat.contains_key("color"));
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
    page(&mut h, "Mats");
    click(&mut h, "Mat 12x24");
    click(&mut h, "Hidden");
    click(&mut h, "Save preset");

    let t = plotter_toml(&dir, "user-my-cameo");
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

    use_plotter(&mut h, &s, "user-my-cameo");
    h.get_by_role_and_label(Role::ComboBox, "Mat").click();
    h.run_plan();
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
    page(&mut h, "Mats");
    click(&mut h, "Mat 12x12");
    let before =
        std::fs::read_to_string(dir.path().join("profiles/plotters/user-my-cameo.toml")).unwrap();
    // 50 + 304.8 is more than the 330.2 mm of the mat.
    type_into(&mut h, "Grid origin y (mm)", "50");
    h.get_by_label("The grid (origin plus size) must lie inside the physical mat.");
    assert!(disabled(&h, "Save preset"));
    h.get_by_label("Save preset").click();
    h.run_plan();
    let after =
        std::fs::read_to_string(dir.path().join("profiles/plotters/user-my-cameo.toml")).unwrap();
    assert_eq!(before, after);
    assert_eq!(
        store(&s).plotter("user-my-cameo").unwrap().media[0]
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
    page(&mut h, "Mats");
    click(&mut h, "Mat 12x12");
    click(&mut h, "Custom");
    type_into(&mut h, "Mat colour (hex)", "#336699");
    click(&mut h, "Save preset");

    let t = plotter_toml(&dir, "user-my-cameo");
    let mat = t["media"].as_array().unwrap()[0].as_table().unwrap();
    assert_eq!(mat["color"].as_str(), Some("#336699"));
    let reloaded = ProfileStore::load(&dir.path().join("profiles")).unwrap();
    assert_eq!(
        reloaded.plotter("user-my-cameo").unwrap().media[0].color,
        Some([0x33, 0x66, 0x99])
    );

    use_plotter(&mut h, &s, "user-my-cameo");
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
    let file = dir.path().join("profiles/plotters/user-my-cameo.toml");
    assert!(file.exists());
    click(&mut h, "Delete preset");
    click(&mut h, "Yes, delete");
    assert!(!file.exists());
    assert!(store(&s).plotter("user-my-cameo").is_err());
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
        .execute(
            "job.set_plotter",
            json!({ "id": "user-my-cameo" }),
            Origin::User,
        )
        .unwrap();
    click(&mut h, "Delete preset");
    click(&mut h, "Yes, delete");
    assert!(
        dir.path()
            .join("profiles/plotters/user-my-cameo.toml")
            .exists()
    );
    assert!(store(&s).plotter("user-my-cameo").is_ok());
    assert!(app(&h).ui.toasts.iter().any(|t| t.text.contains("in use")));
}

#[test]
fn material_override_saves_only_the_override() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    h.get_by_label("Preset name").click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_plan();
    h.get_by_label("Preset name").type_text("My Paper");
    h.run_plan();
    click(&mut h, "Create");
    click(&mut h, "Add override for");
    click(&mut h, "Silhouette Cameo 2");
    click(&mut h, "Silhouette Cameo 2 speed set");
    type_into(&mut h, "Silhouette Cameo 2 speed", "3");
    click(&mut h, "Save preset");

    let st = store(&s);
    assert_eq!(
        st.modified_material_fields("user-my-paper"),
        ["overrides.silhouette-cameo-2.speed"]
    );
    let m = st.material("user-my-paper").unwrap();
    assert_eq!(m.for_plotter("silhouette-cameo-2").speed, 3);
}

#[test]
fn theme_switch_applies_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Appearance");
    click(&mut h, "Theme: Light");
    click(&mut h, "OK");
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
    open(&mut h, "Cutting");
    h.get_by_label("Default material").click();
    h.run_plan();
    // The list is longer than the popup: scroll the entry into view.
    h.get_by_label("Adhesive vinyl (permanent)").scroll_to_me();
    h.run_plan();
    click(&mut h, "Adhesive vinyl (permanent)");
    click(&mut h, "OK");
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
    let file = plotters.join("user-my-cameo.toml");
    std::fs::write(&file, "id = [").unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    // The unreadable file keeps its id: the new preset takes the next one.
    assert!(store(&s).plotter("user-my-cameo-2").is_ok());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "id = [");
    // The window's own request is refused by the store too.
    let mut p = store(&s).plotter("silhouette-cameo-2").unwrap();
    p.id = "user-my-cameo".into();
    p.name = "My Cameo".into();
    flatcraft::ui::profiles_window::apply(
        app_mut(&mut h),
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
    h.run_plan();
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
fn reload_presets_clears_a_fixed_file() {
    let dir = tempfile::tempdir().unwrap();
    let plotters = dir.path().join("profiles/plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(plotters.join("fixme.toml"), "id = [").unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Plotters");
    h.get_by_label_contains("This preset file can't be read:");
    std::fs::write(
        plotters.join("fixme.toml"),
        "id = \"fixme\"\nname = \"Fixed\"\ninherits = \"silhouette-cameo-2\"\n",
    )
    .unwrap();
    click(&mut h, "Reload presets");
    assert!(
        h.query_by_label_contains("This preset file can't be read:")
            .is_none()
    );
    assert!(store(&s).diagnostics().is_empty());
    assert_eq!(store(&s).plotter("fixme").unwrap().name, "Fixed");
}

#[test]
fn the_mats_page_says_what_comes_for_calibration() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Mats");
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
    open(&mut h, "General");
    click(&mut h, "Choose folder...");
    // Staged until Apply.
    assert_eq!(
        s.lock().unwrap().prefs.output_dir,
        Some(dir.path().join("output"))
    );
    click(&mut h, "Apply");
    assert_eq!(
        Preferences::load(dir.path()).output_dir,
        Some(chosen.clone())
    );
    assert_eq!(s.lock().unwrap().prefs.output_dir, Some(chosen));
}

#[test]
fn save_as_a_taken_name_gets_the_next_id() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Cameo");
    assert!(store(&s).plotter("user-my-cameo-2").is_ok());
    assert_eq!(store(&s).plotters().len(), 4);
}

#[test]
fn add_copy_rename_and_delete_own_mats() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    page(&mut h, "Mats");
    click(&mut h, "Add mat");
    h.get_by_label("New mat");
    type_into(&mut h, "Mat name", "Scrap mat");
    click(&mut h, "Copy mat");
    h.get_by_label("Scrap mat copy");
    click(&mut h, "Delete mat");
    assert!(h.query_by_label("Scrap mat copy").is_none());
    click(&mut h, "Save preset");

    let p = store(&s).plotter("user-my-cameo").unwrap();
    let names: Vec<&str> = p.media.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Mat 12x12", "Mat 12x24", "No mat", "Scrap mat"]);
    let t = plotter_toml(&dir, "user-my-cameo");
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
    let file = dir.path().join("profiles/plotters/user-my-cameo.toml");
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
    page(&mut h, "Mats");
    for (cmd, p) in [
        ("job.set_plotter", json!({ "id": "user-my-cameo" })),
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
    let file = dir.path().join("profiles/plotters/user-my-cameo.toml");
    let before = std::fs::read_to_string(&file).unwrap();
    // The default device starts new documents on its default mat, mat-12x12.
    s.lock().unwrap().prefs.default_plotter = "user-my-cameo".into();
    let mut p = store(&s).plotter("user-my-cameo").unwrap();
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
    page(&mut h, "Mats");
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
    assert_eq!((d.id.as_str(), d.edit.scale_x), ("user-my-cameo", 1.5));
    click(&mut h, "Silhouette Cameo 2");
    click(&mut h, "Discard");
    let d = app(&h).ui.profiles.plotter.as_ref().unwrap();
    assert_eq!(d.id, "silhouette-cameo-2");
    assert_eq!(store(&s).plotter("user-my-cameo").unwrap().scale_x, 1.0);
}

#[test]
fn closing_asks_before_dropping_edits() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    type_into(&mut h, "Scale x", "1.5");
    click(&mut h, flatcraft::ui::settings_window::CLOSE_NAME);
    assert!(app(&h).ui.show_settings);
    h.get_by_label("Discard your changes?");
    click(&mut h, "Discard");
    assert!(!app(&h).ui.show_settings);
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
    click(&mut h, "Adhesive vinyl (permanent)");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Cameo");
    assert!(store(&s).material("user-my-cameo").is_ok());
    assert!(store(&s).plotter("user-my-cameo").is_ok());
    assert_eq!(
        store(&s).modified_material_fields("user-my-cameo"),
        Vec::<String>::new()
    );
}

#[test]
fn theme_buttons_offer_system_and_follow_the_os() {
    use flatcraft_engine::prefs::Theme;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    s.lock()
        .unwrap()
        .update_prefs(|p| p.theme = Theme::Dark)
        .unwrap();
    let mut h = harness(&dir, &s);
    open(&mut h, "Appearance");
    h.input_mut().system_theme = Some(egui::Theme::Light);
    click(&mut h, "Theme: System");
    // The picked option explains itself under the buttons.
    assert!(
        h.query_by_label_contains(flatcraft::ui::profiles_window::THEME_HELP)
            .is_some()
    );
    click(&mut h, "OK");
    assert_eq!(Preferences::load(dir.path()).theme, Theme::System);
    assert_eq!(app(&h).tokens, flatcraft::ui::theme::Tokens::light());
}

/// One control of a window: its accessible name, its centre and whether
/// it is dimmed.
struct Control {
    /// The accesskit node id, as text.
    id: String,
    label: String,
    disabled: bool,
}

/// The controls of the window (an accesskit dialog) that holds a node
/// named `anchor`: buttons, text and number fields, choices and
/// checkboxes. Only that window's own nodes count, so a control of the
/// same name elsewhere in the app neither hides nor adds one.
fn window_controls(h: &AppHarness, anchor: &str) -> Vec<Control> {
    let window = h
        .root()
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::Dialog)
        .find(|d| {
            d.children_recursive()
                .any(|n| n.accesskit_node().label().as_deref() == Some(anchor))
        })
        .unwrap_or_else(|| panic!("no window holds {anchor}"));
    window
        .children_recursive()
        .filter(|n| {
            matches!(
                n.accesskit_node().role(),
                Role::Button | Role::TextInput | Role::SpinButton | Role::ComboBox | Role::CheckBox
            )
        })
        .map(|n| {
            let a = n.accesskit_node();
            Control {
                id: format!("{:?}", a.id()),
                label: a.label().unwrap_or_default(),
                disabled: a.is_disabled(),
            }
        })
        .collect()
}

/// Every text the app shows now (a label node keeps its text as value).
fn texts(h: &AppHarness) -> std::collections::BTreeSet<String> {
    h.root()
        .children_recursive()
        .filter_map(|n| {
            let a = n.accesskit_node();
            if a.role() == Role::Label {
                a.value()
            } else {
                a.label()
            }
        })
        .collect()
}

/// Moves the pointer to an empty corner and waits until no hover card is
/// left, so the next card can only come from the next hovered control.
fn clear_hover(h: &mut AppHarness) -> std::collections::BTreeSet<String> {
    h.hover_at(egui::pos2(1395.0, 2395.0));
    let mut before = texts(h);
    for _ in 0..10 {
        h.run_plan();
        let now = texts(h);
        if now == before {
            return now;
        }
        before = now;
    }
    panic!("a hover card did not go away");
}

/// Hovers every control of the window that holds `anchor` and returns how
/// many there were (and how many dimmed) and the ones that show no card.
fn controls_without_a_card(h: &mut AppHarness, anchor: &str) -> (usize, usize, Vec<String>) {
    let controls = window_controls(h, anchor);
    let mut missing = Vec::new();
    for c in &controls {
        // Scroll the control into view, then find where it is now.
        let node = |h: &AppHarness| {
            h.root()
                .children_recursive()
                .find(|n| format!("{:?}", n.accesskit_node().id()) == c.id)
                .map(|n| n.rect().center())
        };
        if let Some(n) = h
            .root()
            .children_recursive()
            .find(|n| format!("{:?}", n.accesskit_node().id()) == c.id)
        {
            n.scroll_to_me();
        }
        h.run_plan();
        let before = clear_hover(h);
        let Some(at) = node(h) else {
            missing.push(format!("{} (gone)", c.label));
            continue;
        };
        h.hover_at(at);
        // The card is the new text the hover brought: its title and body.
        // A card waits for the tooltip delay, so give it some frames.
        let shown = (0..40).any(|_| {
            h.run_plan();
            texts(h)
                .difference(&before)
                .any(|t| t.len() > 20 && t.ends_with(['.', '!', '?']))
        });
        if !shown {
            missing.push(format!("{} (dimmed {})", c.label, c.disabled));
        }
    }
    clear_hover(h);
    let dimmed = controls.iter().filter(|c| c.disabled).count();
    (controls.len(), dimmed, missing)
}

#[test]
fn every_control_in_the_preset_pages_has_a_hover_card() {
    let close = flatcraft::ui::settings_window::CLOSE_NAME;
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    // Tall enough that every control of a page is on screen to hover.
    let mut h = common::harness_sized(&dir, s.clone(), [1400.0, 2400.0], 8, |app| app);
    let (mut seen, mut dimmed) = (0, 0);
    let mut check = |h: &mut AppHarness, anchor: &str, what: &str| {
        let (n, d, missing) = controls_without_a_card(h, anchor);
        assert_eq!(missing, Vec::<String>::new(), "{what}");
        seen += n;
        dimmed += d;
    };
    // A user material with one plotter override: Save preset is dimmed
    // (nothing changed), the override boxes and values show.
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Paper");
    click(&mut h, "Add override for");
    click(&mut h, "Silhouette Cameo 2");
    check(&mut h, close, "Materials, with an override");
    // The Delete question.
    click(&mut h, "Delete preset");
    check(&mut h, "Yes, delete", "the Delete question");
    click(&mut h, "Keep it");
    // Both system plotters: everything dimmed.
    page(&mut h, "Plotters");
    for preset in ["Silhouette Cameo 2", "File Output (GPGL Cameo 2)"] {
        click(&mut h, preset);
        check(&mut h, close, preset);
    }
    // The Save as question.
    click(&mut h, "Save as user preset...");
    check(&mut h, "Create", "the Save as question");
    h.root()
        .children_recursive()
        .filter(|n| n.accesskit_node().role() == Role::Dialog)
        .find(|d| d.query_by_label("Create").is_some())
        .unwrap()
        .get_by_label("Cancel")
        .click();
    h.run_plan();
    // The system mats: Add mat and Copy mat dimmed.
    page(&mut h, "Mats");
    check(&mut h, close, "Mats of a system preset");
    // A user plotter with a changed field and a mat of its own: Save
    // preset, Revert and Delete mat show.
    page(&mut h, "Plotters");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Cameo");
    type_into(&mut h, "Scale x", "1.01");
    check(&mut h, close, "a changed user plotter");
    page(&mut h, "Mats");
    click(&mut h, "Add mat");
    check(&mut h, close, "Mats of a user preset");
    // Picking another preset with unsaved changes asks first.
    page(&mut h, "Plotters");
    click(&mut h, "Silhouette Cameo 2");
    check(&mut h, "Discard", "the Discard question");
    // 242 controls today, 83 of them dimmed: the counts catch a page that
    // stopped showing its controls.
    assert!(seen >= 242, "{seen}");
    assert!(dimmed >= 83, "{dimmed}");
}

#[test]
fn dimmed_fields_say_why_on_their_card() {
    use flatcraft::ui::profiles_window::{
        MIN_RADIUS_OFF, OVERRIDE_OFF, OVERSHOOT_OFF, SYSTEM_READ_ONLY,
    };
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = common::harness_sized(&dir, s.clone(), [1400.0, 2400.0], 8, |app| app);
    let hover = |h: &mut AppHarness, label: &str| {
        let at = h.get_by_label(label).rect().center();
        h.hover_at(egui::pos2(at.x, at.y));
        h.run_plan();
        h.run_plan();
    };
    // A system preset is read-only.
    open(&mut h, "Plotters");
    click(&mut h, "Silhouette Cameo 2");
    assert!(disabled(&h, "Track enhancing"));
    hover(&mut h, "Track enhancing");
    assert!(h.query_by_label(SYSTEM_READ_ONLY).is_some());
    // An override value counts only while its box is ticked.
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Paper");
    click(&mut h, "Add override for");
    click(&mut h, "Silhouette Cameo 2");
    assert!(disabled(&h, "Silhouette Cameo 2 force"));
    hover(&mut h, "Silhouette Cameo 2 force");
    assert!(h.query_by_label(OVERRIDE_OFF).is_some());
    assert!(h.query_by_label(SYSTEM_READ_ONLY).is_none());
    // The overshoot only counts with Lift and overshoot, the minimum
    // radius only with its own value.
    for (field, reason) in [
        ("Corner overshoot (mm)", OVERSHOOT_OFF),
        ("Minimum radius (mm)", MIN_RADIUS_OFF),
    ] {
        assert!(disabled(&h, field), "{field}");
        hover(&mut h, field);
        assert!(h.query_by_label(reason).is_some(), "{field}");
    }
}

#[test]
fn dimmed_buttons_say_why_on_their_card() {
    use flatcraft::ui::profiles_window::{FIX_PROBLEMS_FIRST, NOTHING_TO_SAVE, SYSTEM_READ_ONLY};
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = common::harness_sized(&dir, s.clone(), [1400.0, 2400.0], 8, |app| app);
    let hover = |h: &mut AppHarness, label: &str| {
        h.hover_at(egui::pos2(1395.0, 2395.0));
        h.run_plan();
        let at = h.get_by_label(label).rect().center();
        h.hover_at(egui::pos2(at.x, at.y));
        for _ in 0..10 {
            h.run_plan();
        }
    };
    // A system preset's mats cannot be added to.
    open(&mut h, "Mats");
    assert!(disabled(&h, "Add mat"));
    hover(&mut h, "Add mat");
    assert!(h.query_by_label(SYSTEM_READ_ONLY).is_some());
    // A fresh user preset has nothing to save.
    page(&mut h, "Plotters");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Cameo");
    assert!(disabled(&h, "Save preset"));
    hover(&mut h, "Save preset");
    assert!(h.query_by_label(NOTHING_TO_SAVE).is_some());
    // A field that needs a fix blocks both saves.
    type_into(&mut h, "Scale x", "0");
    h.get_by_label("The scale must be above 0.");
    hover(&mut h, "Save preset");
    assert!(h.query_by_label(FIX_PROBLEMS_FIRST).is_some());
    assert!(h.query_by_label(NOTHING_TO_SAVE).is_none());
    hover(&mut h, "Save as user preset...");
    assert!(h.query_by_label(FIX_PROBLEMS_FIRST).is_some());
}

/// Moves the pointer away, then onto the control named `label` with
/// `role`, and lets its card show.
fn hover_role(h: &mut AppHarness, role: Role, label: &str) {
    clear_hover(h);
    let at = h.get_by_role_and_label(role, label).rect().center();
    h.hover_at(egui::pos2(at.x, at.y));
    for _ in 0..10 {
        h.run_plan();
    }
}

#[test]
fn copy_mat_says_why_it_is_dimmed() {
    use flatcraft::ui::profiles_window::{HIDDEN_NO_COPY, PICK_A_MAT};
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = common::harness_sized(&dir, s.clone(), [1400.0, 2400.0], 8, |app| app);
    save_as_my_cameo(&mut h);
    page(&mut h, "Mats");
    // A hidden built-in mat is picked: it cannot be copied.
    h.get_by_role_and_label(Role::Button, "Mat 12x24").click();
    h.run_plan();
    click(&mut h, "Hidden");
    assert!(disabled(&h, "Copy mat"));
    hover_role(&mut h, Role::Button, "Copy mat");
    assert!(h.query_by_label(HIDDEN_NO_COPY).is_some());
    assert!(h.query_by_label(PICK_A_MAT).is_none());
    // Hide the other built-in mats, add one of your own and delete it:
    // no mat is picked any more.
    for mat in ["Mat 12x12", "No mat"] {
        h.get_by_role_and_label(Role::Button, mat).click();
        h.run_plan();
        click(&mut h, "Hidden");
    }
    click(&mut h, "Add mat");
    click(&mut h, "Delete mat");
    assert!(disabled(&h, "Copy mat"));
    hover_role(&mut h, Role::Button, "Copy mat");
    assert!(h.query_by_label(PICK_A_MAT).is_some());
    assert!(h.query_by_label(HIDDEN_NO_COPY).is_none());
}

#[test]
fn reload_presets_has_a_hover_card() {
    let dir = tempfile::tempdir().unwrap();
    let plotters = dir.path().join("profiles/plotters");
    std::fs::create_dir_all(&plotters).unwrap();
    std::fs::write(plotters.join("broken.toml"), "id = [").unwrap();
    let s = session(&dir);
    let mut h = common::harness_sized(&dir, s.clone(), [1400.0, 2400.0], 8, |app| app);
    open(&mut h, "Plotters");
    hover_role(&mut h, Role::Button, "Reload presets");
    h.get_by_label("Fixed the file? This reads the preset folder again.");
}

#[test]
fn a_mat_field_takes_a_calculation_with_units() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    page(&mut h, "Mats");
    click(&mut h, "Mat 12x12");
    let x = store(&s).plotter("user-my-cameo").unwrap().media[0]
        .grid
        .origin_x_mm;
    // One it cannot read, kept by Enter: it says why.
    type_into(&mut h, "Grid origin x (mm)", "5+");
    h.get_by_label("Can't read '5+': something is missing after +. Press Esc to go back.");
    // Typed and not left: Save preset counts it first. (The click into
    // the next field gives up the bad text with a toast.)
    h.get_by_label("Grid origin y (mm)").click();
    h.run_plan();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_plan();
    h.get_by_label("Grid origin y (mm)").type_text("1/2in");
    h.run_plan();
    click(&mut h, "Save preset");
    for _ in 0..3 {
        h.run_plan();
    }
    let p = store(&s).plotter("user-my-cameo").unwrap();
    assert!((p.media[0].grid.origin_y_mm - 12.7).abs() < 1e-9);
    assert_eq!(p.media[0].grid.origin_x_mm, x);
}

#[test]
fn a_usb_id_field_reads_hex_and_follows_the_click_rules() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    let vid = |s: &SharedSession| match store(s).plotter("user-my-cameo").unwrap().transport {
        flatcraft_device::profile::TransportSpec::Usb { vid, .. } => vid,
        _ => panic!("no USB"),
    };
    let before = vid(&s);
    common::check_click_rules(&mut h, "USB vendor id");
    // Hex digits count; a calculation is no hex id.
    type_into(&mut h, "USB vendor id", "1+1");
    h.get_by_label("Can't read '1+1': type up to 4 hex digits, 0-9 and A-F. Press Esc to go back.");
    h.key_press(egui::Key::Escape);
    h.run_plan();
    type_into(&mut h, "USB vendor id", "0x1a2b");
    assert_eq!(common::field_text(&h, "USB vendor id"), "1A2B");
    click(&mut h, "Save preset");
    for _ in 0..3 {
        h.run_plan();
    }
    assert_ne!(before, 0x1a2b);
    assert_eq!(vid(&s), 0x1a2b);
}

#[test]
fn save_as_never_takes_a_built_in_id() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My vellum");
    click(&mut h, "Save as user preset...");
    // A name that slugs to a built-in id gets the user prefix.
    type_into(&mut h, "Preset name", "Vellum");
    let st = store(&s);
    let mine = st.material("user-vellum").unwrap();
    assert_eq!(mine.inherits.as_deref(), Some("user-my-vellum"));
    let built_in = st.material("vellum").unwrap();
    assert!(built_in.inherits.is_none());
    assert!(st.materials().iter().any(|p| p.id == "vellum" && p.system));
    assert!(st.diagnostics().is_empty());
}

#[test]
fn corner_mode_saves_to_user_material() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Paper");
    // Copy paper swivels: the overshoot is dimmed.
    assert!(disabled(&h, "Corner overshoot (mm)"));
    click(&mut h, "Corner mode");
    click(&mut h, "Lift and overshoot");
    type_into(&mut h, "Corner overshoot (mm)", "0.2");
    click(&mut h, "Save preset");

    let st = store(&s);
    assert_eq!(
        st.modified_material_fields("user-my-paper"),
        ["corner_mode"]
    );
    let m = st.material("user-my-paper").unwrap();
    assert_eq!(
        m.corner_mode,
        flatcraft_device::CornerModeSpec::LiftOvershoot { overshoot_mm: 0.2 }
    );
    let text =
        std::fs::read_to_string(dir.path().join("profiles/materials/user-my-paper.toml")).unwrap();
    let t: toml::Table = text.parse().unwrap();
    assert_eq!(
        keys(&t),
        ["corner_mode", "id", "inherits", "name"],
        "{text}"
    );
}

#[test]
fn min_radius_uses_the_blade_offset_until_set() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Paper");
    assert!(disabled(&h, "Minimum radius (mm)"));
    // While the blade offset applies, the field says so instead of a number.
    assert_eq!(
        h.get_by_label("Minimum radius (mm)")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("blade offset")
    );
    click(&mut h, "Minimum radius (mm) own value");
    assert!(!disabled(&h, "Minimum radius (mm)"));
    type_into(&mut h, "Minimum radius (mm)", "0.6");
    click(&mut h, "Save preset");
    let st = store(&s);
    assert_eq!(
        st.modified_material_fields("user-my-paper"),
        ["min_radius_mm"]
    );
    assert_eq!(
        st.material("user-my-paper").unwrap().min_radius_mm,
        Some(0.6)
    );
}

#[test]
fn ring_direction_saves_to_user_plotter() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, &s);
    save_as_my_cameo(&mut h);
    click(&mut h, "Ring direction");
    click(&mut h, "Clockwise");
    click(&mut h, "Save preset");
    let st = store(&s);
    assert_eq!(
        st.modified_plotter_fields("user-my-cameo"),
        ["ring_direction"]
    );
    assert_eq!(
        st.plotter("user-my-cameo").unwrap().ring_direction,
        flatcraft_device::RingDirection::Cw
    );
    let t = plotter_toml(&dir, "user-my-cameo");
    assert_eq!(t["ring_direction"].as_str(), Some("cw"));
}

#[test]
fn corner_mode_and_min_radius_override_per_plotter() {
    use flatcraft::ui::profiles_window::{OVERRIDE_OFF, OVERSHOOT_OFF};
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = common::harness_sized(&dir, s.clone(), [1400.0, 2400.0], 8, |app| app);
    let hover = |h: &mut AppHarness, label: &str| {
        let at = h.get_by_label(label).rect().center();
        h.hover_at(egui::pos2(at.x, at.y));
        h.run_plan();
        h.run_plan();
    };
    open(&mut h, "Materials");
    click(&mut h, "Copy paper");
    click(&mut h, "Save as user preset...");
    type_into(&mut h, "Preset name", "My Paper");
    click(&mut h, "Add override for");
    click(&mut h, "Silhouette Cameo 2");
    // Nothing set yet: every field dimmed, saying why.
    let cameo = "Silhouette Cameo 2";
    for field in ["corner mode", "corner overshoot", "minimum radius"] {
        let label = format!("{cameo} {field}");
        assert!(disabled(&h, &label), "{label}");
        hover(&mut h, &label);
        assert!(h.query_by_label(OVERRIDE_OFF).is_some(), "{label}");
    }
    // Set the corner mode: it still swivels, so the overshoot stays dimmed.
    click(&mut h, "Silhouette Cameo 2 corner mode set");
    assert!(!disabled(&h, "Silhouette Cameo 2 corner mode"));
    assert!(disabled(&h, "Silhouette Cameo 2 corner overshoot"));
    hover(&mut h, "Silhouette Cameo 2 corner overshoot");
    assert!(h.query_by_label(OVERSHOOT_OFF).is_some());
    click(&mut h, "Silhouette Cameo 2 corner mode");
    click(&mut h, "Lift and overshoot");
    type_into(&mut h, "Silhouette Cameo 2 corner overshoot", "0.15");
    click(&mut h, "Silhouette Cameo 2 minimum radius set");
    type_into(&mut h, "Silhouette Cameo 2 minimum radius", "0.7");
    click(&mut h, "Save preset");

    let st = store(&s);
    assert_eq!(
        st.modified_material_fields("user-my-paper"),
        [
            "overrides.silhouette-cameo-2.corner_mode.LiftOvershoot.overshoot_mm",
            "overrides.silhouette-cameo-2.min_radius_mm"
        ]
    );
    let m = st.material("user-my-paper").unwrap();
    assert_eq!(m.corner_mode, flatcraft_device::CornerModeSpec::Arc);
    let c = m.for_plotter("silhouette-cameo-2");
    assert_eq!(
        c.corner_mode,
        flatcraft_device::CornerModeSpec::LiftOvershoot { overshoot_mm: 0.15 }
    );
    assert_eq!(c.min_radius_mm, Some(0.7));
    assert_eq!(
        m.for_plotter("file-output-gpgl-cameo2").corner_mode,
        flatcraft_device::CornerModeSpec::Arc
    );
}
