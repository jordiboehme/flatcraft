//! "It didn't cut right": the checklist per symptom, its fixes (job
//! overrides, never the preset, one undo step each), the test shape and
//! the last good cut from the cut log, as agents get them through
//! `job.troubleshoot`.

use flatcraft_device::ProfileStore;
use flatcraft_engine::cutlog::{self, Verdict};
use flatcraft_engine::job::JobInput;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::registry;
use flatcraft_engine::session::{CommandError, Origin, Session};
use flatcraft_engine::troubleshoot::{self, Check, Checklist, Symptom};
use kurbo::Rect;
use serde_json::{Value, json};

fn session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

fn svg(w: f64, body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{w}mm" viewBox="0 0 {w} {w}">{body}</svg>"#
    )
}

/// Imports a square of `side` mm and moves its top-left to `x`, `y`.
fn square_at(s: &mut Session, name: &str, side: f64, x: f64, y: f64) -> u64 {
    let r = run(
        s,
        "object.import_svg",
        json!({ "name": name, "svg": svg(side, &format!(r#"<rect width="{side}" height="{side}"/>"#)) }),
    );
    let id = r["id"].as_u64().unwrap();
    run(
        s,
        "object.move",
        json!({ "ids": [id], "x_mm": x, "y_mm": y }),
    );
    id
}

fn list(s: &mut Session, symptom: Symptom) -> Checklist {
    let v = run(
        s,
        "job.troubleshoot",
        json!({ "symptom": symptom.as_str() }),
    );
    serde_json::from_value(v).unwrap()
}

fn checks(c: &Checklist) -> Vec<Check> {
    c.items.iter().map(|i| i.check).collect()
}

fn item(c: &Checklist, check: Check) -> &troubleshoot::Item {
    c.items
        .iter()
        .find(|i| i.check == check)
        .unwrap_or_else(|| panic!("no {check:?} in {c:?}"))
}

/// Runs the fix `label` of `check` as an agent: one execute of its
/// command and args.
fn apply(s: &mut Session, c: &Checklist, check: Check, label: &str) -> Value {
    let fix = item(c, check)
        .fixes
        .iter()
        .find(|f| f.label == label)
        .unwrap_or_else(|| panic!("no fix {label} on {check:?}"));
    assert!(registry::get(&fix.command).is_some(), "{}", fix.command);
    s.execute(&fix.command, fix.args.clone(), Origin::Agent)
        .unwrap_or_else(|e| panic!("{label} failed: {e}"))
}

fn blocked_reason(c: &Checklist, check: Check, label: &str) -> String {
    let it = item(c, check);
    assert!(
        it.fixes.iter().all(|f| f.label != label),
        "{label} offered: {it:?}"
    );
    it.blocked
        .iter()
        .find(|b| b.label == label)
        .unwrap_or_else(|| panic!("{label} not blocked: {it:?}"))
        .reason
        .clone()
}

fn bounds(s: &mut Session, id: u64) -> Rect {
    let state = run(s, "document.state", json!({}));
    let o = state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()
        .clone();
    let b = &o["bounds_mm"];
    Rect::new(
        b["x0"].as_f64().unwrap(),
        b["y0"].as_f64().unwrap(),
        b["x1"].as_f64().unwrap(),
        b["y1"].as_f64().unwrap(),
    )
}

fn area(s: &mut Session) -> Rect {
    let state = run(s, "document.state", json!({}));
    let a = &state["cutting_area_mm"];
    Rect::new(
        a["x0"].as_f64().unwrap(),
        a["y0"].as_f64().unwrap(),
        a["x1"].as_f64().unwrap(),
        a["y1"].as_f64().unwrap(),
    )
}

/// A send of the document, then its outcome.
fn cut_and_rate(s: &mut Session, dir: &tempfile::TempDir, verdict: Option<Verdict>, note: &str) {
    run(s, "job.send", json!({}));
    let log = cutlog::read(dir.path());
    let id = log.entries.last().unwrap().id.clone();
    if let Some(v) = verdict {
        s.record_outcome(&id, v, note).unwrap();
    }
}

#[test]
fn each_symptom_has_its_checks_in_order() {
    use Check::*;
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    // No notes and no good cut yet: those items are left out.
    assert_eq!(
        checks(&list(&mut s, Symptom::NotThrough)),
        [
            BladeDepth,
            Force,
            Passes,
            Blade,
            Speed,
            Mat,
            MaterialFace,
            TestShape
        ]
    );
    assert_eq!(
        checks(&list(&mut s, Symptom::TornOrDragged)),
        [
            BladeDepth,
            Force,
            Speed,
            Blade,
            TinyDetails,
            Mat,
            MaterialFace,
            TestShape
        ]
    );
    assert_eq!(
        checks(&list(&mut s, Symptom::PiecesMoved)),
        [Mat, Speed, CutOrder, TinyDetails, Blade, TestShape]
    );
    assert_eq!(
        checks(&list(&mut s, Symptom::WrongPlaceOrSize)),
        [Scale, Position, Mirror, MaterialFace, Mat, TestShape]
    );
    for symptom in Symptom::ALL {
        let c = list(&mut s, symptom);
        assert_eq!(c.title, symptom.label());
        assert_eq!(c.symptom, symptom);
    }
}

#[test]
fn notes_and_the_last_good_cut_come_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    s.profiles
        .set_material_notes("copy-paper", "Use the blue mat.")
        .unwrap();
    run(
        &mut s,
        "job.set_overrides",
        json!({ "force": 14, "passes": 2 }),
    );
    cut_and_rate(&mut s, &dir, Some(Verdict::Good), "clean edges");
    let c = list(&mut s, Symptom::NotThrough);
    assert_eq!(
        &checks(&c)[..3],
        [Check::Notes, Check::LastGood, Check::BladeDepth]
    );
    assert!(
        item(&c, Check::Notes)
            .message
            .contains("Your notes on Copy paper: Use the blue mat."),
        "{c:?}"
    );
    // The brief's own words.
    let good = &item(&c, Check::LastGood).message;
    assert!(
        good.starts_with("Last good cut with this material: force 14, 2 passes, speed 5."),
        "{good}"
    );
    assert!(good.contains("You noted: clean edges"), "{good}");
    // The size check has no use for it.
    assert!(!checks(&list(&mut s, Symptom::WrongPlaceOrSize)).contains(&Check::LastGood));
    // Scale is measured on the test sheet's rulers.
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let scale = &item(&c, Check::Scale).message;
    assert!(
        scale.contains("test sheet") && scale.contains("100 mm rulers"),
        "{scale}"
    );
}

#[test]
fn the_last_good_cut_skips_bad_and_unrated_cuts() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    run(&mut s, "job.set_overrides", json!({ "force": 12 }));
    cut_and_rate(&mut s, &dir, Some(Verdict::Good), "");
    run(&mut s, "job.set_overrides", json!({ "force": 20 }));
    cut_and_rate(&mut s, &dir, Some(Verdict::Bad), "tore");
    run(&mut s, "job.set_overrides", json!({ "force": 22 }));
    cut_and_rate(&mut s, &dir, None, "");
    let c = list(&mut s, Symptom::NotThrough);
    let g = c.last_good.as_ref().unwrap();
    assert_eq!(g.force, 12);
    let log = cutlog::read(dir.path());
    assert_eq!(log.last_good("copy-paper").unwrap().force, 12);
    assert_eq!(log.last_good("vinyl"), None);
    // Its settings, back on the job in one step.
    let before = s.doc().undo.undo_len();
    apply(&mut s, &c, Check::LastGood, "Use these settings");
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    let job = &s.doc().document.job;
    assert_eq!(
        (job.speed, job.force, job.passes),
        (Some(5), Some(12), Some(1))
    );
    // Now the job cuts with them: the button says so instead.
    let c = list(&mut s, Symptom::NotThrough);
    let why = blocked_reason(&c, Check::LastGood, "Use these settings");
    assert!(why.contains("already"), "{why}");
}

#[test]
fn override_fixes_change_the_job_not_the_preset_in_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    let preset = run(&mut s, "profiles.get", json!({ "id": "copy-paper" }));
    let cases = [
        (Symptom::NotThrough, Check::Force, "Force +2", "force", 12),
        (
            Symptom::NotThrough,
            Check::Passes,
            "Add a pass",
            "passes",
            2,
        ),
        (Symptom::NotThrough, Check::Speed, "Slower", "speed", 3),
        (Symptom::TornOrDragged, Check::Force, "Force -2", "force", 8),
    ];
    for (symptom, check, label, field, want) in cases {
        let c = list(&mut s, symptom);
        let before = s.doc().undo.undo_len();
        let out = apply(&mut s, &c, check, label);
        assert_eq!(out[field], want, "{label}: {out}");
        assert_eq!(
            s.doc().undo.undo_len(),
            before + 1,
            "{label}: one undo step"
        );
        // The checklist says the new value.
        let again = list(&mut s, symptom);
        let now = match field {
            "force" => again.settings.force,
            "passes" => again.settings.passes,
            _ => again.settings.speed,
        };
        assert_eq!(now, want as u32);
        run(&mut s, "edit.undo", json!({}));
        let job = &s.doc().document.job;
        assert_eq!(
            (job.speed, job.force, job.passes),
            (None, None, None),
            "{label}"
        );
    }
    assert_eq!(
        run(&mut s, "profiles.get", json!({ "id": "copy-paper" })),
        preset,
        "the material preset never changes"
    );
}

#[test]
fn fixes_at_the_plotter_limits_are_blocked_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    run(
        &mut s,
        "job.set_overrides",
        json!({ "force": 33, "passes": 20, "speed": 1 }),
    );
    let c = list(&mut s, Symptom::NotThrough);
    assert!(blocked_reason(&c, Check::Force, "Force +2").contains("maximum, 33"));
    assert!(blocked_reason(&c, Check::Passes, "Add a pass").contains("at most 20"));
    assert!(blocked_reason(&c, Check::Speed, "Slower").contains("slowest, 1"));
    // One step below the top: +2 stops at the maximum.
    run(&mut s, "job.set_overrides", json!({ "force": 32 }));
    let c = list(&mut s, Symptom::NotThrough);
    assert_eq!(apply(&mut s, &c, Check::Force, "Force +2")["force"], 33);
    run(&mut s, "job.set_overrides", json!({ "force": 1 }));
    let c = list(&mut s, Symptom::TornOrDragged);
    assert!(blocked_reason(&c, Check::Force, "Force -2").contains("minimum, 1"));
}

#[test]
fn the_blade_depth_is_only_a_hint() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "job.set_material", json!({ "id": "cardstock" }));
    let c = list(&mut s, Symptom::NotThrough);
    let it = item(&c, Check::BladeDepth);
    assert!(
        it.message
            .starts_with("Cardstock (light, about 160-200 g/m2) says blade 3."),
        "{it:?}"
    );
    assert!(it.fixes.is_empty() && it.blocked.is_empty());
    assert!(it.hint.contains("sends nothing about it"), "{it:?}");
}

#[test]
fn the_test_shape_avoids_objects_and_stays_inside_the_area() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 60.0, 5.0, 5.0);
    let b = square_at(&mut s, "b", 40.0, 90.0, 10.0);
    let hidden = square_at(&mut s, "hidden", 30.0, 160.0, 5.0);
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [hidden], "visible": false }),
    );
    let c = list(&mut s, Symptom::NotThrough);
    let before = s.doc().undo.undo_len();
    let out = apply(&mut s, &c, Check::TestShape, "Add a test shape");
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    assert_eq!(out["added"], true);
    let id = out["id"].as_u64().unwrap();
    let shape = bounds(&mut s, id);
    assert!((shape.width() - 30.0).abs() < 1e-6, "{shape:?}");
    assert!((shape.height() - 30.0).abs() < 1e-6, "{shape:?}");
    let area = area(&mut s);
    assert_eq!(area.x0, 5.0, "an uncalibrated mat keeps 5 mm");
    assert!(
        flatcraft_engine::commands::inside(shape, area),
        "{shape:?} in {area:?}"
    );
    for other in [a, b, hidden] {
        let o = bounds(&mut s, other);
        let apart = shape.x0 >= o.x1 + 5.0 - 1e-6
            || shape.x1 <= o.x0 - 5.0 + 1e-6
            || shape.y0 >= o.y1 + 5.0 - 1e-6
            || shape.y1 <= o.y0 - 5.0 + 1e-6;
        assert!(apart, "test shape {shape:?} too close to {o:?}");
    }
    // The first free spot from the top-left: right of the hidden square.
    assert_eq!((shape.x0, shape.y0), (195.0, 5.0), "{shape:?}");
    assert_eq!(
        s.doc().selection.iter().map(|i| i.0).collect::<Vec<_>>(),
        [id]
    );
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().document.objects.len(), 3);
}

#[test]
fn the_test_shape_moves_instead_of_doubling() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let first = run(&mut s, "tools.test_shape", json!({}));
    let id = first["id"].as_u64().unwrap();
    // Running it again changes nothing.
    let again = run(&mut s, "tools.test_shape", json!({}));
    assert_eq!(again["id"], id);
    assert_eq!(again["added"], false);
    assert_eq!(again["bounds_mm"], first["bounds_mm"]);
    // A design dropped on top of it: it moves away, still one test shape.
    let at = bounds(&mut s, id);
    let design = square_at(&mut s, "design", 50.0, at.x0, at.y0);
    let moved = run(&mut s, "tools.test_shape", json!({}));
    assert_eq!(moved["id"], id);
    let shape = bounds(&mut s, id);
    let d = bounds(&mut s, design);
    assert!(
        shape.x0 >= d.x1 + 5.0 - 1e-6 || shape.y0 >= d.y1 + 5.0 - 1e-6,
        "{shape:?} {d:?}"
    );
    let count = s
        .doc()
        .document
        .objects
        .iter()
        .filter(|o| o.name == "Test shape")
        .count();
    assert_eq!(count, 1);
}

#[test]
fn an_object_named_test_shape_is_not_taken_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let mine = square_at(&mut s, "Test shape", 20.0, 5.0, 5.0);
    let out = run(&mut s, "tools.test_shape", json!({}));
    assert_eq!(out["added"], true);
    assert_ne!(out["id"], mine);
    let b = bounds(&mut s, mine);
    assert_eq!((b.x0, b.y0), (5.0, 5.0), "the person's object stays");
}

#[test]
fn a_full_mat_has_no_room_for_the_test_shape() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let area = area(&mut s);
    let side = area.width().min(area.height());
    square_at(&mut s, "big", side, area.x0, area.y0);
    let c = list(&mut s, Symptom::NotThrough);
    let why = blocked_reason(&c, Check::TestShape, "Add a test shape");
    assert!(why.contains("no free spot"), "{why}");
    match s.execute("tools.test_shape", json!({}), Origin::Agent) {
        Err(CommandError::Refused(text)) => assert!(text.contains("no free spot"), "{text}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn iron_on_that_is_not_mirrored_is_the_likely_cause() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    let b = square_at(&mut s, "b", 20.0, 50.0, 10.0);
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a], "axis": "horizontal" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(it.likely, "{it:?}");
    assert_eq!(it.object_ids, [b]);
    assert!(
        it.message
            .contains("1 of 2 is not mirrored, so it comes out backwards"),
        "{it:?}"
    );
    // A mixed layout gets the hint, not a button: the whole layout must
    // flip together.
    assert!(it.fixes.is_empty(), "{it:?}");
    assert!(it.message.contains(troubleshoot::MIXED_MIRROR), "{it:?}");
    assert_eq!(
        blocked_reason(&c, Check::Mirror, "Mirror for iron-on"),
        troubleshoot::MIXED_MIRROR
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a], "axis": "horizontal" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(
        it.message
            .contains("2 of 2 are not mirrored, so they come out"),
        "{c:?}"
    );
    // The likely cause comes first, with an absolute fix for the whole
    // layout.
    assert_eq!(c.items[0].check, Check::Mirror, "{c:?}");
    assert_eq!(it.fixes[0].label, "Mirror for iron-on");
    assert_eq!(it.fixes[0].args["ids"], json!([a, b]));
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a, b], "axis": "horizontal" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    assert!(!item(&c, Check::Mirror).likely);
    // Mirrored on paper is the mistake the other way round.
    run(&mut s, "job.set_material", json!({ "id": "copy-paper" }));
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(it.likely && it.object_ids == [a, b], "{it:?}");
    assert_eq!(it.fixes[0].label, "Mirror back");
    assert_eq!(it.fixes[0].args["ids"], json!([a, b]));
    // Not iron-on: the mirror check keeps its place.
    assert_eq!(c.items[0].check, Check::Scale, "{c:?}");
}

#[test]
fn mirror_for_iron_on_twice_changes_nothing_the_second_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    let b = square_at(&mut s, "b", 20.0, 50.0, 10.0);
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let before = s.doc().undo.undo_len();
    apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    let doc = s.doc().document.clone();
    assert!(doc.objects.iter().all(|o| o.transform.mirror));
    // The same (now stale) fix again: nothing flips, no undo step.
    let out = apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    assert_eq!(out["ids"], json!([]));
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    let again = &s.doc().document;
    for (x, y) in doc.objects.iter().zip(&again.objects) {
        assert_eq!(x.transform, y.transform);
    }
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let why = blocked_reason(&c, Check::Mirror, "Mirror for iron-on");
    assert!(why.contains("mirrored already"), "{why}");
    assert_eq!(c.items[0].check, Check::Scale);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().document.objects.iter().all(|o| !o.transform.mirror));
    let _ = (a, b);
}

#[test]
fn the_cut_order_fix_sets_outlines_last_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    let c = list(&mut s, Symptom::PiecesMoved);
    let before = s.doc().undo.undo_len();
    apply(&mut s, &c, Check::CutOrder, "Cut outlines last");
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    assert_eq!(
        s.doc().document.job.strategy,
        flatcraft_core::document::CutStrategy::OutlinesLast
    );
    let c = list(&mut s, Symptom::PiecesMoved);
    assert!(blocked_reason(&c, Check::CutOrder, "Cut outlines last").contains("already"));
    assert!(item(&c, Check::CutOrder).message.contains("Outlines last"));
    // Running it again changes nothing.
    run(
        &mut s,
        "job.set_strategy",
        json!({ "strategy": "outlines_last" }),
    );
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(
        s.doc().document.job.strategy,
        flatcraft_core::document::CutStrategy::Quality
    );
    assert!(matches!(
        s.execute(
            "job.set_strategy",
            json!({ "strategy": "slow" }),
            Origin::Agent
        ),
        Err(CommandError::InvalidParams(_))
    ));
}

#[test]
fn cut_only_the_test_shape_skips_the_rest_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    let b = square_at(&mut s, "b", 20.0, 50.0, 10.0);
    let hidden = square_at(&mut s, "hidden", 20.0, 90.0, 10.0);
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [hidden], "visible": false }),
    );
    let c = list(&mut s, Symptom::NotThrough);
    let why = blocked_reason(&c, Check::TestShape, "Cut only the test shape");
    assert!(why.contains("Add the test shape first"), "{why}");
    let shape = apply(&mut s, &c, Check::TestShape, "Add a test shape")["id"]
        .as_u64()
        .unwrap();
    let c = list(&mut s, Symptom::NotThrough);
    let before = s.doc().undo.undo_len();
    apply(&mut s, &c, Check::TestShape, "Cut only the test shape");
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    let cut: Vec<u64> = s
        .doc()
        .document
        .objects
        .iter()
        .filter(|o| o.cut_enabled)
        .map(|o| o.id.0)
        .collect();
    assert_eq!(
        cut,
        [hidden, shape],
        "only the test shape is cut (hidden stays as it was)"
    );
    // The job cuts only the test shape.
    let p = run(&mut s, "job.preview", json!({}));
    assert_eq!(p["toolpaths"], 2, "{p}");
    // A second click changes nothing.
    apply(&mut s, &c, Check::TestShape, "Cut only the test shape");
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    let c = list(&mut s, Symptom::NotThrough);
    assert!(blocked_reason(&c, Check::TestShape, "Cut only the test shape").contains("already"));
    // Undo turns the design back on.
    run(&mut s, "edit.undo", json!({}));
    let doc = &s.doc().document;
    assert!(
        doc.objects
            .iter()
            .filter(|o| o.id.0 == a || o.id.0 == b)
            .all(|o| o.cut_enabled)
    );
}

#[test]
fn tiny_details_bring_the_report_fixes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let id = run(
        &mut s,
        "object.import_svg",
        json!({ "name": "frame", "svg": svg(100.0, r#"<rect x="10" y="10" width="60" height="60" fill="none" stroke="black" stroke-width="1"/>"#) }),
    )["id"]
        .clone();
    // A 1 mm stroke is cut once by default; cut around it, it is a hairline.
    run(
        &mut s,
        "object.set_cut_options",
        json!({ "ids": [id], "strokes": "outline" }),
    );
    let c = list(&mut s, Symptom::TornOrDragged);
    let it = item(&c, Check::TinyDetails);
    assert!(it.likely, "{it:?}");
    assert!(
        it.fixes.iter().any(|f| f.label == "Cut as single lines"),
        "{it:?}"
    );
    apply(&mut s, &c, Check::TinyDetails, "Cut as single lines");
    let c = list(&mut s, Symptom::TornOrDragged);
    let it = item(&c, Check::TinyDetails);
    assert!(!it.likely && it.message.contains("finds none"), "{it:?}");
}

#[test]
fn the_command_returns_what_the_window_shows() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    for symptom in Symptom::ALL {
        let got = run(
            &mut s,
            "job.troubleshoot",
            json!({ "symptom": symptom.as_str() }),
        );
        let input = JobInput::of(&s).unwrap();
        let log = s.cut_log();
        let want = troubleshoot::checklist(&input, symptom, log.last_good("copy-paper"));
        assert_eq!(got, serde_json::to_value(&want).unwrap());
        assert_eq!(got["symptom"], symptom.as_str());
        assert_eq!(Symptom::parse(symptom.as_str()), Some(symptom));
    }
    assert!(matches!(
        s.execute(
            "job.troubleshoot",
            json!({ "symptom": "exploded" }),
            Origin::Agent
        ),
        Err(CommandError::InvalidParams(_))
    ));
    assert!(matches!(
        s.execute(
            "job.troubleshoot",
            json!({ "symptom": "not_through", "force": 3 }),
            Origin::Agent
        ),
        Err(CommandError::InvalidParams(_))
    ));
    let cmd = registry::get("job.troubleshoot").unwrap();
    assert!(!cmd.mutates_document());
    for name in [
        "not_through",
        "tore_or_dragged",
        "pieces_moved",
        "wrong_place_or_size",
        "blade_depth",
        "test_shape",
    ] {
        assert!(cmd.description().contains(name), "{name}");
    }
}

#[test]
fn no_text_has_an_em_dash_and_every_fix_is_a_real_command() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    s.profiles
        .set_plotter_notes("file-output-gpgl-cameo2", "Rollers at the marks.")
        .unwrap();
    cut_and_rate(&mut s, &dir, Some(Verdict::Good), "fine");
    for material in ["copy-paper", "heat-transfer-vinyl", "cardstock", "vinyl"] {
        run(&mut s, "job.set_material", json!({ "id": material }));
        for media in ["mat-12x12", "no-mat"] {
            run(&mut s, "job.set_media", json!({ "id": media }));
            for symptom in Symptom::ALL {
                let v = run(
                    &mut s,
                    "job.troubleshoot",
                    json!({ "symptom": symptom.as_str() }),
                );
                let text = v.to_string();
                assert!(!text.contains('\u{2014}'), "{text}");
                let c: Checklist = serde_json::from_value(v).unwrap();
                for it in &c.items {
                    assert!(!it.message.is_empty() && !it.hint.is_empty(), "{it:?}");
                    for f in &it.fixes {
                        assert!(registry::get(&f.command).is_some(), "{f:?}");
                    }
                }
            }
        }
    }
    for symptom in Symptom::ALL {
        assert!(!symptom.label().contains('\u{2014}'));
        assert!(!symptom.description().contains('\u{2014}'));
    }
    assert!(
        !registry::get("job.troubleshoot")
            .unwrap()
            .description()
            .contains('\u{2014}')
    );
    assert!(
        !registry::get("tools.test_shape")
            .unwrap()
            .description()
            .contains('\u{2014}')
    );
}

/// A 200 x 100 mm panel (on a 200 mm page, so its centre is not the
/// middle of what is cut) and a 20 mm square, at `x`, `y`.
fn uneven_pair(s: &mut Session, x: f64, y: f64) -> (u64, u64) {
    let r = run(
        s,
        "object.import_svg",
        json!({ "name": "panel", "svg": svg(200.0, r#"<rect width="200" height="100"/>"#) }),
    );
    let big = r["id"].as_u64().unwrap();
    run(
        s,
        "object.move",
        json!({ "ids": [big], "x_mm": x, "y_mm": y }),
    );
    let small = square_at(s, "dot", 20.0, x + 220.0, y + 40.0);
    (big, small)
}

fn union(s: &mut Session, ids: &[u64]) -> Rect {
    ids.iter()
        .map(|i| bounds(s, *i))
        .reduce(|a, b| a.union(b))
        .unwrap()
}

fn same_box(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 1e-6
        && (a.x1 - b.x1).abs() < 1e-6
        && (a.y0 - b.y0).abs() < 1e-6
        && (a.y1 - b.y1).abs() < 1e-6
}

#[test]
fn mirror_for_iron_on_keeps_an_uneven_design_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let (big, small) = uneven_pair(&mut s, 10.0, 20.0);
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let before = union(&mut s, &[big, small]);
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    let after = union(&mut s, &[big, small]);
    assert!(same_box(before, after), "{before:?} vs {after:?}");
    // The small square swapped sides inside the box.
    let dot = bounds(&mut s, small);
    assert!((dot.x0 - before.x0).abs() < 1e-6, "{dot:?}");
    let p = run(&mut s, "job.preview", json!({}));
    assert_eq!(p["ready"], true, "{p}");
}

#[test]
fn the_test_shape_never_counts_for_the_mirror() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let (big, small) = uneven_pair(&mut s, 10.0, 20.0);
    let shape = run(&mut s, "tools.test_shape", json!({}))["id"]
        .as_u64()
        .unwrap();
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(it.message.contains("2 are not mirrored"), "{it:?}");
    assert!(!it.object_ids.contains(&shape));
    // The whole layout flips, the test shape with it.
    assert_eq!(it.fixes[0].args["ids"], json!([big, small]));
    assert_eq!(it.fixes[0].args["follow"], json!([shape]));
    let all = [big, small, shape];
    let before = union(&mut s, &all);
    apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    assert!(same_box(before, union(&mut s, &all)));
    assert_no_overlap(&mut s);
    // No false alarm: everything is mirrored, and the test shape does not
    // count.
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(!it.likely, "{it:?}");
    assert!(it.message.contains("All 2 objects are mirrored"), "{it:?}");
    assert_eq!(run(&mut s, "job.preview", json!({}))["ready"], true);
}

/// No two visible objects overlap (touching is fine), skipped ones
/// included: they are cut once Skip is off.
fn assert_no_overlap(s: &mut Session) {
    let state = run(s, "document.state", json!({}));
    let boxes: Vec<(u64, Rect)> = state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["visible"] == true)
        .map(|o| {
            let b = &o["bounds_mm"];
            (
                o["id"].as_u64().unwrap(),
                Rect::new(
                    b["x0"].as_f64().unwrap(),
                    b["y0"].as_f64().unwrap(),
                    b["x1"].as_f64().unwrap(),
                    b["y1"].as_f64().unwrap(),
                ),
            )
        })
        .collect();
    for (i, (ia, a)) in boxes.iter().enumerate() {
        for (ib, b) in &boxes[i + 1..] {
            let overlap = a.intersect(*b);
            assert!(
                overlap.width() <= 1e-6 || overlap.height() <= 1e-6,
                "{ia} {a:?} overlaps {ib} {b:?}"
            );
        }
    }
}

/// The reviewer's layout: the test shape sits in the gap between a wide
/// panel and a small square. Flipping only the design would put the
/// square on the test shape; the whole layout flips together instead.
#[test]
fn the_test_shape_in_the_gap_stays_clear() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let r = run(
        &mut s,
        "object.import_svg",
        json!({ "name": "panel", "svg": svg(200.0, r#"<rect width="200" height="100"/>"#) }),
    );
    let big = r["id"].as_u64().unwrap();
    run(
        &mut s,
        "object.move",
        json!({ "ids": [big], "x_mm": 10.0, "y_mm": 20.0 }),
    );
    let small = square_at(&mut s, "dot", 20.0, 270.0, 20.0);
    let shape = run(&mut s, "tools.test_shape", json!({}))["id"]
        .as_u64()
        .unwrap();
    let t = bounds(&mut s, shape);
    assert!(
        t.x0 > 210.0 && t.x1 < 270.0,
        "the test shape is in the gap: {t:?}"
    );
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    assert_no_overlap(&mut s);
    assert_eq!(run(&mut s, "job.preview", json!({}))["ready"], true);
    let _ = small;
}

/// A plain, B mirrored, C plain: no button, the hint says what to do.
#[test]
fn a_mixed_layout_gets_the_hint_and_no_button() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let _a = square_at(&mut s, "A", 20.0, 10.0, 10.0);
    let b = square_at(&mut s, "B", 20.0, 50.0, 10.0);
    let _c = square_at(&mut s, "C", 20.0, 90.0, 10.0);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [b], "axis": "horizontal" }),
    );
    // One wording per material: iron-on advice only on iron-on.
    for (material, hint) in [
        ("heat-transfer-vinyl", troubleshoot::MIXED_MIRROR),
        ("copy-paper", troubleshoot::MIXED_MIRROR_PLAIN),
    ] {
        run(&mut s, "job.set_material", json!({ "id": material }));
        let c = list(&mut s, Symptom::WrongPlaceOrSize);
        let it = item(&c, Check::Mirror);
        assert!(it.likely, "{material}: {it:?}");
        assert!(it.fixes.is_empty(), "{material}: {it:?}");
        assert!(it.message.ends_with(hint), "{it:?}");
        assert_eq!(it.blocked[0].reason, hint);
    }
    assert!(!troubleshoot::MIXED_MIRROR_PLAIN.contains("iron-on"));
}

/// Property check: random layouts (sizes, places, mirror states, skipped
/// objects, with or without a test shape, iron-on or not). Whenever a mirror fix is
/// offered, running it leaves no two cut objects overlapping and the job
/// ready.
#[test]
fn any_offered_mirror_fix_keeps_the_layout_clear() {
    let mut seed: u64 = 0x5eed_f1a7;
    let mut rnd = |n: u64| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % n
    };
    let mut offered = 0;
    for case in 0..120 {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(&dir);
        let mut placed: Vec<Rect> = Vec::new();
        let count = 2 + rnd(3);
        let mut tries = 0;
        while (placed.len() as u64) < count && tries < 200 {
            tries += 1;
            let side = 10.0 + rnd(50) as f64;
            let x = 5.0 + rnd(220) as f64;
            let y = 5.0 + rnd(220) as f64;
            let r = Rect::new(x, y, x + side, y + side);
            if placed
                .iter()
                .any(|p| p.inflate(1.0, 1.0).intersect(r).area() > 0.0)
            {
                continue;
            }
            let id = square_at(&mut s, &format!("o{}", placed.len()), side, x, y);
            if rnd(2) == 0 {
                run(
                    &mut s,
                    "object.mirror",
                    json!({ "ids": [id], "axis": "horizontal", "about": "each" }),
                );
            }
            // Some are skipped: still on the sheet, cut later.
            if rnd(3) == 0 {
                run(
                    &mut s,
                    "object.set_cut",
                    json!({ "ids": [id], "cut": false }),
                );
            }
            placed.push(r);
        }
        let add_shape = rnd(2) == 0;
        if add_shape
            && let Ok(v) = s.execute("tools.test_shape", json!({}), Origin::User)
            && rnd(2) == 0
        {
            // Its flag says nothing: it is symmetric.
            run(
                &mut s,
                "object.mirror",
                json!({ "ids": [v["id"]], "axis": "horizontal", "about": "each" }),
            );
        }
        let material = if rnd(2) == 0 {
            "heat-transfer-vinyl"
        } else {
            "copy-paper"
        };
        run(&mut s, "job.set_material", json!({ "id": material }));
        assert_no_overlap(&mut s);
        let c = list(&mut s, Symptom::WrongPlaceOrSize);
        let it = item(&c, Check::Mirror).clone();
        for fix in &it.fixes {
            offered += 1;
            s.execute(&fix.command, fix.args.clone(), Origin::Agent)
                .unwrap_or_else(|e| panic!("case {case}: {e}"));
            assert_no_overlap(&mut s);
            assert_eq!(
                run(&mut s, "job.preview", json!({}))["ready"],
                true,
                "case {case}"
            );
            // Afterwards the item no longer offers a fix.
            let again = list(&mut s, Symptom::WrongPlaceOrSize);
            assert!(item(&again, Check::Mirror).fixes.is_empty(), "case {case}");
        }
    }
    assert!(offered >= 5, "the cases offered a fix {offered} times");
}

#[test]
fn the_mirror_fix_waits_for_a_design_on_the_mat() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [a], "x_mm": -15.0, "y_mm": 10.0 }),
    );
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    assert!(item(&c, Check::Mirror).likely);
    let why = blocked_reason(&c, Check::Mirror, "Mirror for iron-on");
    assert!(why.contains("outside the cutting area"), "{why}");
}

#[test]
fn one_mirrored_object_reads_well() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a], "axis": "horizontal" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    assert_eq!(
        item(&c, Check::Mirror).message,
        "The object is mirrored, as heat transfer vinyl needs."
    );
}

/// The reviewer's layout: a panel, the test shape in the gap, a square.
fn gap_layout(s: &mut Session) -> (u64, u64, u64) {
    let r = run(
        s,
        "object.import_svg",
        json!({ "name": "panel", "svg": svg(200.0, r#"<rect width="200" height="100"/>"#) }),
    );
    let big = r["id"].as_u64().unwrap();
    run(
        s,
        "object.move",
        json!({ "ids": [big], "x_mm": 10.0, "y_mm": 20.0 }),
    );
    let small = square_at(s, "dot", 20.0, 270.0, 20.0);
    let shape = run(s, "tools.test_shape", json!({}))["id"]
        .as_u64()
        .unwrap();
    (big, small, shape)
}

#[test]
fn a_design_mirrored_by_hand_on_paper_goes_back_with_the_test_shape() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    // The design was mirrored by hand, then the test shape was added at
    // a free spot.
    let (big, small, _) = gap_layout(&mut s);
    run(&mut s, "edit.undo", json!({}));
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [big, small], "axis": "horizontal" }),
    );
    let shape = run(&mut s, "tools.test_shape", json!({}))["id"]
        .as_u64()
        .unwrap();
    assert_no_overlap(&mut s);
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(!it.message.contains("Some objects are mirrored"), "{it:?}");
    assert_eq!(it.fixes[0].label, "Mirror back");
    assert_eq!(it.fixes[0].args["follow"], json!([shape]));
    apply(&mut s, &c, Check::Mirror, "Mirror back");
    assert_no_overlap(&mut s);
    assert_eq!(run(&mut s, "job.preview", json!({}))["ready"], true);
    assert!(!item(&list(&mut s, Symptom::WrongPlaceOrSize), Check::Mirror).likely);
}

#[test]
fn on_iron_on_only_the_test_shape_differing_is_not_mixed() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let (_big, _small, shape) = gap_layout(&mut s);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [shape], "axis": "horizontal" }),
    );
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(!it.message.contains("Some objects are mirrored"), "{it:?}");
    let before = bounds(&mut s, shape);
    apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    // The test shape moved with the group, though its flag already said
    // mirrored.
    assert!((bounds(&mut s, shape).x0 - before.x0).abs() > 1.0);
    assert_no_overlap(&mut s);
    assert_eq!(run(&mut s, "job.preview", json!({}))["ready"], true);
    // A second click changes nothing.
    let undo = s.doc().undo.undo_len();
    apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    assert_eq!(s.doc().undo.undo_len(), undo);
}

#[test]
fn skipped_objects_flip_with_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    let b = square_at(&mut s, "b", 40.0, 50.0, 10.0);
    let hidden = square_at(&mut s, "hidden", 20.0, 120.0, 10.0);
    run(
        &mut s,
        "object.set_cut",
        json!({ "ids": [b], "cut": false }),
    );
    run(
        &mut s,
        "object.set_visible",
        json!({ "ids": [hidden], "visible": false }),
    );
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    // The message counts what is cut; the flip takes the whole sheet.
    assert!(it.message.contains("the object is not mirrored"), "{it:?}");
    assert_eq!(it.object_ids, [a]);
    assert_eq!(it.fixes[0].args["ids"], json!([a, b]));
    apply(&mut s, &c, Check::Mirror, "Mirror for iron-on");
    let doc = &s.doc().document;
    let flag = |id: u64| {
        doc.get(flatcraft_core::document::ObjectId(id))
            .unwrap()
            .transform
            .mirror
    };
    assert!(flag(a) && flag(b) && !flag(hidden));
    assert_no_overlap(&mut s);
}

#[test]
fn only_the_test_shape_cut_reads_the_same_either_way() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "a", 20.0, 10.0, 10.0);
    run(&mut s, "tools.test_shape", json!({}));
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::NotThrough);
    apply(&mut s, &c, Check::TestShape, "Cut only the test shape");
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert_eq!(it.message, troubleshoot::ONLY_TEST_SHAPE);
    assert!(!it.likely && it.fixes.is_empty(), "{it:?}");
}

#[test]
fn a_skipped_object_off_the_mat_blocks_the_flip_and_show_me_finds_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    let off = square_at(&mut s, "off", 20.0, 50.0, 10.0);
    run(
        &mut s,
        "object.move",
        json!({ "ids": [off], "x_mm": -15.0, "y_mm": 10.0 }),
    );
    run(
        &mut s,
        "object.set_cut",
        json!({ "ids": [off], "cut": false }),
    );
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl" }),
    );
    let c = list(&mut s, Symptom::WrongPlaceOrSize);
    let it = item(&c, Check::Mirror);
    assert!(it.likely && it.fixes.is_empty(), "{it:?}");
    assert_eq!(
        blocked_reason(&c, Check::Mirror, "Mirror for iron-on"),
        troubleshoot::OFF_MAT_MIRROR
    );
    assert_eq!(it.object_ids, [off], "Show me points at the object to move");
    let _ = a;
}

#[test]
fn follow_ids_must_exist_whatever_the_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    run(
        &mut s,
        "object.mirror",
        json!({ "ids": [a], "axis": "horizontal" }),
    );
    // Nothing would flip (a is mirrored already), yet an unknown follow id
    // is refused.
    for params in [
        json!({ "ids": [a], "follow": [999], "axis": "horizontal", "mirrored": true }),
        json!({ "ids": [a], "follow": [999], "axis": "horizontal", "mirrored": false }),
    ] {
        assert!(
            matches!(
                s.execute("object.mirror", params.clone(), Origin::Agent),
                Err(CommandError::NotFound(_)) | Err(CommandError::InvalidParams(_))
            ),
            "{params}"
        );
    }
    assert!(
        s.doc().document.objects[0].transform.mirror,
        "nothing changed"
    );
}

#[test]
fn the_mirror_fix_card_names_only_what_is_there() {
    let card = |skip: bool, shape: bool| {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(&dir);
        square_at(&mut s, "a", 20.0, 10.0, 10.0);
        let b = square_at(&mut s, "b", 20.0, 50.0, 10.0);
        if skip {
            run(
                &mut s,
                "object.set_cut",
                json!({ "ids": [b], "cut": false }),
            );
        }
        if shape {
            run(&mut s, "tools.test_shape", json!({}));
        }
        run(
            &mut s,
            "job.set_material",
            json!({ "id": "heat-transfer-vinyl" }),
        );
        let c = list(&mut s, Symptom::WrongPlaceOrSize);
        item(&c, Check::Mirror).fixes[0].description.clone()
    };
    let plain = card(false, false);
    assert!(
        plain.starts_with("Mirrors the whole layout (2 objects) left"),
        "{plain}"
    );
    let skipped = card(true, false);
    assert!(
        skipped.contains("(2 objects, skipped ones too)"),
        "{skipped}"
    );
    let shape = card(false, true);
    assert!(shape.contains("(3 objects, the test shape too)"), "{shape}");
    let both = card(true, true);
    assert!(
        both.contains("(3 objects, skipped ones and the test shape too)"),
        "{both}"
    );
}

// ------------------------------------------------ Task 60: material presets

#[test]
fn every_iron_on_preset_and_its_children_count_as_iron_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    for id in [
        "heat-transfer-vinyl",
        "heat-transfer-vinyl-glitter",
        "heat-transfer-vinyl-flock",
    ] {
        assert!(
            troubleshoot::iron_on(&s.profiles.material(id).unwrap()),
            "{id}"
        );
    }
    for id in ["vinyl", "stencil-film", "copy-paper", "window-cling"] {
        assert!(
            !troubleshoot::iron_on(&s.profiles.material(id).unwrap()),
            "{id}"
        );
    }
    // A child with a name that says nothing about iron-on still is.
    let mut m = s.profiles.material("heat-transfer-vinyl-glitter").unwrap();
    m.id = "sparkle".into();
    m.name = "My sparkle".into();
    s.profiles
        .save_user_material(&m, "heat-transfer-vinyl-glitter")
        .unwrap();
    assert!(troubleshoot::iron_on(
        &s.profiles.material("sparkle").unwrap()
    ));
}

#[test]
fn the_iron_on_reminder_shows_until_everything_is_mirrored() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square_at(&mut s, "a", 20.0, 10.0, 10.0);
    // Vinyl: never a reminder.
    run(&mut s, "job.set_material", json!({ "id": "vinyl" }));
    assert!(troubleshoot::iron_on_reminder(&JobInput::of(&s).unwrap()).is_none());
    // Glitter iron-on, nothing mirrored: the reminder with its fix.
    run(
        &mut s,
        "job.set_material",
        json!({ "id": "heat-transfer-vinyl-glitter" }),
    );
    let it = troubleshoot::iron_on_reminder(&JobInput::of(&s).unwrap()).unwrap();
    assert_eq!(it.check, Check::Mirror);
    assert_eq!(it.fixes.len(), 1);
    assert_eq!(it.fixes[0].command, "object.mirror");
    assert_eq!(it.fixes[0].args["ids"], json!([a]));
    // It never blocks: the job still goes out.
    run(&mut s, "job.send", json!({}));
    run(&mut s, "object.mirror", it.fixes[0].args.clone());
    assert!(troubleshoot::iron_on_reminder(&JobInput::of(&s).unwrap()).is_none());
}

#[test]
fn agents_see_groups_tips_and_mat_tack() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let list = s
        .execute("profiles.list", json!({}), Origin::Agent)
        .unwrap();
    let materials = list["materials"].as_array().unwrap();
    assert_eq!(materials.len(), flatcraft_device::presets::MATERIALS.len());
    let group = |id: &str| materials.iter().find(|m| m["id"] == id).unwrap()["group"].clone();
    assert_eq!(group("heat-transfer-vinyl-flock"), "iron_on");
    assert_eq!(group("vinyl-removable"), "vinyl");
    assert_eq!(group("vellum"), "paper_and_card");
    assert_eq!(group("window-cling"), "other");
    let m = s
        .execute(
            "profiles.get",
            json!({ "id": "heat-transfer-vinyl" }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(m["group"], "iron_on");
    assert_eq!(m["mat_tack"], "standard");
    assert_eq!(m["backing"], true);
    assert!(
        m["tips"]
            .as_str()
            .unwrap()
            .starts_with("Mirror the design and cut with the shiny carrier side down")
    );
    let paper = s
        .execute("profiles.get", json!({ "id": "copy-paper" }), Origin::Agent)
        .unwrap();
    assert_eq!(paper["mat_tack"], "light");
    assert_eq!(paper["backing"], false);
}

#[test]
fn the_cut_log_knows_a_material_was_cut() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    assert!(!s.cut_log().has_cut("copy-paper"));
    cut_and_rate(&mut s, &dir, None, "");
    assert!(s.cut_log().has_cut("copy-paper"));
    assert!(!s.cut_log().has_cut("vellum"));
}

#[test]
fn the_profile_command_texts_name_no_brand() {
    for id in ["profiles.list", "profiles.get"] {
        let text = registry::get(id).unwrap().description().to_lowercase();
        for b in [
            "silhouette",
            "cameo",
            "cricut",
            "siser",
            "oracal",
            "graphtec",
        ] {
            assert!(!text.contains(b), "{id} names {b}");
        }
    }
}

#[test]
fn the_test_shape_item_follows_the_backing_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square_at(&mut s, "sq", 20.0, 40.0, 40.0);
    run(&mut s, "job.set_material", json!({ "id": "vinyl" }));
    let c = list(&mut s, Symptom::NotThrough);
    let m = &item(&c, Check::TestShape).message;
    assert!(
        m.contains("not cut through") && !m.contains("drop out"),
        "{m}"
    );
    run(&mut s, "job.set_material", json!({ "id": "cardstock" }));
    let c = list(&mut s, Symptom::NotThrough);
    assert!(
        item(&c, Check::TestShape)
            .message
            .contains("does the hole drop out")
    );
}
