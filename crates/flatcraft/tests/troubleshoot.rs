//! The "It didn't cut right..." helper in the window: it opens from the
//! Send stage and the Help menu, shows the engine's checklist for the
//! picked symptom in order, and its buttons run exactly the engine's
//! fixes (job overrides, one undo step each).

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use flatcraft::ui::Workspace;
use flatcraft::ui::help::HelpItem;
use flatcraft::ui::troubleshoot::{self, LABEL};
use flatcraft_engine::cutlog::{self, Verdict};
use flatcraft_engine::job::JobInput;
use flatcraft_engine::session::Origin;
use flatcraft_engine::troubleshoot::{Checklist, Symptom, checklist};
use serde_json::json;

fn open_from_send(h: &mut AppHarness) {
    app_mut(h).ui.workspace = Workspace::Cut;
    h.run();
    h.get_by_role_and_label(Role::Tab, "Send").click();
    h.run();
    h.get_by_role_and_label(Role::Button, LABEL).click();
    h.run();
}

fn pick(h: &mut AppHarness, symptom: Symptom) {
    h.get_by_role_and_label(Role::Button, symptom.label())
        .click();
    h.run();
}

fn engine_list(s: &flatcraft_mcp::SharedSession, symptom: Symptom) -> Checklist {
    let s = s.lock().unwrap();
    let input = JobInput::of(&s).unwrap();
    let log = s.cut_log();
    checklist(&input, symptom, log.last_good(&input.job.material_id))
}

fn job_force(s: &flatcraft_mcp::SharedSession) -> Option<u8> {
    s.lock().unwrap().doc().document.job.force
}

#[test]
fn it_opens_under_the_send_button_and_asks_what_happened() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    // Nothing to cut: the helper is still there, the cut may be older.
    let mut h = harness(&dir, s.clone(), false);
    open_from_send(&mut h);
    assert!(app(&h).ui.troubleshoot.open);
    h.get_by_label(troubleshoot::TITLE);
    h.get_by_label(troubleshoot::ASK);
    h.get_by_label(troubleshoot::PICK_FIRST);
    for symptom in Symptom::ALL {
        h.get_by_role_and_label(Role::Button, symptom.label());
    }
    // Close keeps nothing open.
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run();
    assert!(!app(&h).ui.troubleshoot.open);
    assert!(h.query_by_label(troubleshoot::ASK).is_none());
}

#[test]
fn each_symptom_shows_the_engine_list_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    for symptom in Symptom::ALL {
        pick(&mut h, symptom);
        let list = engine_list(&s, symptom);
        let mut last_y = f32::MIN;
        for (n, it) in list.items.iter().enumerate() {
            let title = format!("{}. {}", n + 1, it.title);
            let node = h.get_by_label(&title);
            let y = node.accesskit_node().bounding_box().unwrap().y0 as f32;
            assert!(y > last_y, "{title} is out of order");
            last_y = y;
            h.get_by_label(&it.message);
        }
    }
}

#[test]
fn force_plus_two_changes_the_job_once_and_undo_takes_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::NotThrough);
    let want = engine_list(&s, Symptom::NotThrough);
    let fix = want
        .items
        .iter()
        .flat_map(|i| &i.fixes)
        .find(|f| f.label == "Force +2")
        .unwrap()
        .clone();
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Force +2").click();
    h.run();
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    assert_eq!(job_force(&s), Some(12));
    // The window ran the engine's fix, with its args.
    assert_eq!(fix.command, "job.set_overrides");
    assert_eq!(fix.args, json!({ "force": 12 }));
    // The preset stays.
    let preset = s
        .lock()
        .unwrap()
        .execute("profiles.get", json!({ "id": "copy-paper" }), Origin::User)
        .unwrap();
    assert_eq!(preset["force"], 10);
    // The list says the new force.
    h.get_by_label(
        &engine_list(&s, Symptom::NotThrough)
            .items
            .iter()
            .find(|i| i.title == "Force")
            .unwrap()
            .message,
    );
    // Close, then the app bar's Undo.
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Undo").click();
    h.run();
    assert_eq!(job_force(&s), None);
}

#[test]
fn a_fix_at_the_limit_is_dimmed_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    s.lock()
        .unwrap()
        .execute("job.set_overrides", json!({ "force": 33 }), Origin::User)
        .unwrap();
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::NotThrough);
    let b = h.get_by_role_and_label(Role::Button, "Force +2");
    assert!(b.accesskit_node().is_disabled());
    b.hover();
    h.run();
    h.get_by_label("Force is at this plotter's maximum, 33. Add a pass, or check the blade depth.");
}

#[test]
fn the_test_shape_button_puts_it_beside_the_design() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let square = import_square(&s);
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::NotThrough);
    let before = undo_len(&s);
    // The last item: scroll the list down to it first.
    h.get_by_role_and_label(Role::Button, "Add a test shape")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(Role::Button, "Add a test shape")
        .click();
    h.run();
    assert_eq!(undo_len(&s), before + 1);
    let state = s
        .lock()
        .unwrap()
        .execute("document.state", json!({}), Origin::User)
        .unwrap();
    let objects = state["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 2);
    let rect = |o: &serde_json::Value| {
        let b = &o["bounds_mm"];
        kurbo::Rect::new(
            b["x0"].as_f64().unwrap(),
            b["y0"].as_f64().unwrap(),
            b["x1"].as_f64().unwrap(),
            b["y1"].as_f64().unwrap(),
        )
    };
    let shape = objects.iter().find(|o| o["name"] == "Test shape").unwrap();
    let design = objects.iter().find(|o| o["id"] == square).unwrap();
    let (a, b) = (rect(shape), rect(design));
    assert!(
        a.intersect(b.inflate(5.0, 5.0)).area() < 1e-9,
        "{a:?} too close to {b:?}"
    );
}

#[test]
fn the_last_good_cut_shows_with_its_day() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    {
        let mut g = s.lock().unwrap();
        g.execute(
            "job.set_overrides",
            json!({ "force": 14, "passes": 2 }),
            Origin::User,
        )
        .unwrap();
        g.execute("job.send", json!({}), Origin::User).unwrap();
        let id = cutlog::read(dir.path()).entries[0].id.clone();
        g.record_outcome(&id, Verdict::Good, "").unwrap();
        g.execute("job.set_overrides", json!({ "clear": true }), Origin::User)
            .unwrap();
    }
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::NotThrough);
    h.get_by_label("Last good cut with this material: force 14, 2 passes, speed 5.");
    let e = &cutlog::read(dir.path()).entries[0];
    h.get_by_label(&format!("Cut on {}", flatcraft::ui::outcome::day(&e.date)));
    h.get_by_role_and_label(Role::Button, "Use these settings")
        .click();
    h.run();
    let job = s.lock().unwrap().doc().document.job.clone();
    assert_eq!(
        (job.speed, job.force, job.passes),
        (Some(5), Some(14), Some(2))
    );
}

#[test]
fn the_help_menu_opens_it_from_home() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    app_mut(&mut h).ui.workspace = Workspace::Home;
    h.run();
    h.get_by_role_and_label(Role::Button, "Help").click();
    h.run();
    h.get_by_label(HelpItem::Troubleshoot.label()).click();
    h.run();
    assert_eq!(app(&h).ui.workspace, Workspace::Cut);
    assert!(app(&h).ui.troubleshoot.open);
    h.get_by_label(troubleshoot::ASK);
    assert_eq!(HelpItem::Troubleshoot.label(), LABEL);
}

#[test]
fn show_me_points_at_the_objects_that_are_not_mirrored() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let id = import_square(&s);
    s.lock()
        .unwrap()
        .execute(
            "job.set_material",
            json!({ "id": "heat-transfer-vinyl" }),
            Origin::User,
        )
        .unwrap();
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::WrongPlaceOrSize);
    h.get_by_label(troubleshoot::LIKELY);
    h.get_by_role_and_label(Role::Button, "Show me").click();
    h.run();
    assert!(!app(&h).ui.troubleshoot.open, "it leaves for Prepare");
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    let sel: Vec<u64> = s
        .lock()
        .unwrap()
        .doc()
        .selection
        .iter()
        .map(|i| i.0)
        .collect();
    assert_eq!(sel, [id]);
}

#[test]
fn no_em_dash_in_the_helper_texts() {
    for text in [
        troubleshoot::LABEL,
        troubleshoot::TITLE,
        troubleshoot::OPEN_HINT,
        troubleshoot::ASK,
        troubleshoot::PICK_FIRST,
        troubleshoot::CLOSE_HINT,
        troubleshoot::LIKELY,
    ] {
        assert!(!text.contains('\u{2014}'), "{text}");
    }
}

#[test]
fn mirror_for_iron_on_comes_first_and_a_second_click_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    s.lock()
        .unwrap()
        .execute(
            "job.set_material",
            json!({ "id": "heat-transfer-vinyl" }),
            Origin::User,
        )
        .unwrap();
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::WrongPlaceOrSize);
    h.get_by_label("1. Mirror for iron-on");
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Mirror for iron-on")
        .click();
    h.run();
    assert_eq!(undo_len(&s), before + 1);
    assert!(
        s.lock()
            .unwrap()
            .doc()
            .document
            .objects
            .iter()
            .all(|o| o.transform.mirror)
    );
    // Now it is dimmed and says why; the list order is back to normal.
    let b = h.get_by_role_and_label(Role::Button, "Mirror for iron-on");
    assert!(b.accesskit_node().is_disabled());
    h.get_by_label("1. Size");
    assert_eq!(undo_len(&s), before + 1);
}

#[test]
fn cut_only_the_test_shape_from_the_helper() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let square = import_square(&s);
    s.lock()
        .unwrap()
        .execute("tools.test_shape", json!({}), Origin::User)
        .unwrap();
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::NotThrough);
    let before = undo_len(&s);
    h.get_by_role_and_label(Role::Button, "Cut only the test shape")
        .scroll_to_me();
    h.run();
    h.get_by_role_and_label(Role::Button, "Cut only the test shape")
        .click();
    h.run();
    assert_eq!(undo_len(&s), before + 1);
    let doc = s.lock().unwrap().doc().document.clone();
    for o in &doc.objects {
        assert_eq!(o.cut_enabled, o.id.0 != square, "{}", o.name);
    }
    // Undo turns the design back on.
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Undo").click();
    h.run();
    assert!(
        s.lock()
            .unwrap()
            .doc()
            .document
            .objects
            .iter()
            .all(|o| o.cut_enabled)
    );
}

#[test]
fn the_list_is_made_again_only_when_something_changed() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::NotThrough);
    let first = app(&h).ui.troubleshoot.builds();
    assert!(first >= 1);
    for _ in 0..5 {
        h.step();
    }
    assert_eq!(
        app(&h).ui.troubleshoot.builds(),
        first,
        "no change, no rebuild"
    );
    h.get_by_role_and_label(Role::Button, "Force +2").click();
    h.run();
    assert_eq!(app(&h).ui.troubleshoot.builds(), first + 1);
    // A preset note changes the list without a document change.
    s.lock()
        .unwrap()
        .profiles
        .set_material_notes("copy-paper", "Use the blue mat.")
        .unwrap();
    h.run();
    h.get_by_label("1. Your notes");
}

#[test]
fn mirror_for_iron_on_keeps_an_uneven_design_in_place_in_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let (big, small) = {
        let mut g = s.lock().unwrap();
        let panel = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect width="200" height="100"/></svg>"#;
        let dot = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm" viewBox="0 0 20 20"><rect width="20" height="20"/></svg>"#;
        let big = g
            .execute(
                "object.import_svg",
                json!({ "name": "panel", "svg": panel }),
                Origin::User,
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        g.execute(
            "object.move",
            json!({ "ids": [big], "x_mm": 10.0, "y_mm": 20.0 }),
            Origin::User,
        )
        .unwrap();
        let small = g
            .execute(
                "object.import_svg",
                json!({ "name": "dot", "svg": dot }),
                Origin::User,
            )
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        g.execute(
            "object.move",
            json!({ "ids": [small], "x_mm": 230.0, "y_mm": 60.0 }),
            Origin::User,
        )
        .unwrap();
        g.execute(
            "job.set_material",
            json!({ "id": "heat-transfer-vinyl" }),
            Origin::User,
        )
        .unwrap();
        (big, small)
    };
    let union = |s: &flatcraft_mcp::SharedSession| {
        let v = s
            .lock()
            .unwrap()
            .execute("document.state", json!({}), Origin::User)
            .unwrap();
        v["objects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                let b = &o["bounds_mm"];
                kurbo::Rect::new(
                    b["x0"].as_f64().unwrap(),
                    b["y0"].as_f64().unwrap(),
                    b["x1"].as_f64().unwrap(),
                    b["y1"].as_f64().unwrap(),
                )
            })
            .reduce(|a, b| a.union(b))
            .unwrap()
    };
    let before = union(&s);
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1200.0], 8, |a| a);
    open_from_send(&mut h);
    pick(&mut h, Symptom::WrongPlaceOrSize);
    h.get_by_role_and_label(Role::Button, "Mirror for iron-on")
        .click();
    h.run();
    let after = union(&s);
    assert!(
        (after.x0 - before.x0).abs() < 1e-6 && (after.x1 - before.x1).abs() < 1e-6,
        "{before:?} vs {after:?}"
    );
    assert!((after.y0 - before.y0).abs() < 1e-6 && (after.y1 - before.y1).abs() < 1e-6);
    let p = s
        .lock()
        .unwrap()
        .execute("job.preview", json!({}), Origin::User)
        .unwrap();
    assert_eq!(p["ready"], true, "{p}");
    let _ = (big, small);
}
