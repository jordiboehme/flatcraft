//! Keep mode per object (Task 29): what is kept after cutting. Pieces
//! keeps the shapes, Stencil the sheet, Both everything near the cuts.
//! The plan, the weeds and the cut log read it; linked copies and groups
//! share it as the brief says.

use flatcraft_core::document::{KeepMode, ObjectId};
use flatcraft_device::ProfileStore;
use flatcraft_engine::job::{self, JobInput};
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use kurbo::Point;
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

/// A 50 mm square at (`x`, 20) on a 200 mm page.
fn square(s: &mut Session, x: f64, name: &str) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="{x}" y="20" width="50" height="50"/></svg>"#
    );
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap()
}

fn keep(s: &Session, id: u64) -> KeepMode {
    s.doc().document.get(ObjectId(id)).unwrap().keep
}

#[test]
fn a_group_id_sets_every_member_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let b = square(&mut s, 100.0, "b");
    let c = square(&mut s, 20.0, "c");
    let g = run(&mut s, "object.group", json!({ "ids": [a, b] }))["group_id"]
        .as_u64()
        .unwrap();
    let before = s.doc().undo.undo_len();
    let r = run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [g], "keep": "both" }),
    );
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    assert_eq!((&r["ids"], &r["keep"]), (&json!([a, b]), &json!("both")));
    assert_eq!(r["report"]["issues"], json!([]));
    assert_eq!((keep(&s, a), keep(&s, b)), (KeepMode::Both, KeepMode::Both));
    assert_eq!(keep(&s, c), KeepMode::Pieces);
}

#[test]
fn the_selection_is_the_default_and_bad_values_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    run(&mut s, "object.set_keep", json!({ "keep": "stencil" }));
    assert_eq!(keep(&s, a), KeepMode::Stencil);
    for bad in [
        json!({ "keep": "sheet" }),
        json!({}),
        json!({ "keep": "pieces", "mode": "x" }),
    ] {
        assert!(
            s.execute("object.set_keep", bad.clone(), Origin::User)
                .is_err(),
            "{bad}"
        );
    }
    run(&mut s, "object.select", json!({ "ids": [] }));
    assert!(
        s.execute("object.set_keep", json!({ "keep": "both" }), Origin::User)
            .is_err()
    );
}

#[test]
fn an_agent_sets_and_reads_it_like_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let state = run(&mut s, "document.state", json!({}));
    assert_eq!(state["objects"][0]["keep"], "pieces");
    let by_agent = s
        .execute(
            "object.set_keep",
            json!({ "ids": [a], "keep": "stencil" }),
            Origin::Agent,
        )
        .unwrap();
    assert_eq!(
        (&by_agent["ids"], &by_agent["keep"]),
        (&json!([a]), &json!("stencil"))
    );
    let state = s
        .execute("document.state", json!({}), Origin::Agent)
        .unwrap();
    assert_eq!(state["objects"][0]["keep"], "stencil");
}

#[test]
fn a_project_keeps_it_and_writes_only_what_is_not_pieces() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let b = square(&mut s, 100.0, "b");
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [b], "keep": "stencil" }),
    );
    let doc = s.doc().document.clone();
    let value = flatcraft_core::project::to_value(&doc).unwrap();
    let objects = value["objects"].as_array().unwrap();
    assert_eq!(objects[0].get("keep"), None, "Pieces is not written");
    assert_eq!(objects[1]["keep"], "stencil");
    let path = dir.path().join("keep.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    run(&mut s, "project.open", json!({ "path": path }));
    assert_eq!(keep(&s, a), KeepMode::Pieces);
    assert_eq!(keep(&s, b), KeepMode::Stencil);
}

#[test]
fn the_plan_classifies_by_the_keep_mode_and_replans_when_it_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let inside = Point::new(45.0, 45.0);
    let outside = Point::new(100.0, 45.0);
    let pieces = JobInput::of(&s).unwrap();
    let plan = job::build_input(&pieces).unwrap().plan;
    assert!(plan.kept.is_kept(inside) && !plan.kept.is_kept(outside));

    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [a], "keep": "stencil" }),
    );
    let stencil = JobInput::of(&s).unwrap();
    assert_ne!(stencil, pieces, "a keep change is a new job input");
    let plan = job::build_input(&stencil).unwrap().plan;
    assert!(!plan.kept.is_kept(inside) && plan.kept.is_kept(outside));
    // The toolpath's keep side follows: a stencil keeps the outside.
    assert!(
        plan.toolpaths
            .iter()
            .all(|t| t.keep == flatcraft_path::KeepSide::Outside)
    );
}

#[test]
fn a_stencil_keeps_the_sheet_so_nothing_is_weeded() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    square(&mut s, 100.0, "b");
    run(
        &mut s,
        "job.set_weeding",
        json!({ "box": true, "lines": "both", "spacing_mm": 10.0 }),
    );
    let weeds = |s: &Session| {
        let r = job::build(s).unwrap();
        let n = r
            .plan
            .toolpaths
            .iter()
            .filter(|t| t.kind == flatcraft_path::ToolpathKind::Weed)
            .count();
        (n, r.plan.blocked_weed_boxes)
    };
    let (n, blocked) = weeds(&s);
    assert!(n > 1 && blocked == 0, "pieces are weeded: {n} {blocked}");
    // One stencil keeps the whole sheet (kept wins), so no weed path
    // may cut anywhere outside its shapes.
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [a], "keep": "stencil" }),
    );
    assert_eq!(weeds(&s), (0, 1));
}

/// An O (a ring with its counter) or an L (one ring) on a 200 mm page.
fn letter(s: &mut Session, o: bool) -> u64 {
    let d = if o {
        "M20 20 H60 V70 H20 Z M30 30 H50 V60 H30 Z"
    } else {
        "M100 20 H110 V60 H130 V70 H100 Z"
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path fill-rule="evenodd" d="{d}"/></svg>"#
    );
    let name = if o { "O" } else { "L" };
    run(s, "object.import_svg", json!({ "svg": svg, "name": name }))["id"]
        .as_u64()
        .unwrap()
}

fn islands(r: &Value) -> Vec<Value> {
    r["issues"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["code"] == "stencil_island")
        .cloned()
        .collect()
}

#[test]
fn a_paper_stencil_says_that_the_middle_of_an_o_falls_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "job.set_material", json!({ "id": "copy-paper" }));
    let o = letter(&mut s, true);
    let l = letter(&mut s, false);
    let r = run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [o, l], "keep": "stencil" }),
    );
    let found = islands(&r["report"]);
    assert_eq!(found.len(), 1, "{r}");
    let i = &found[0];
    assert_eq!(i["severity"], "info");
    assert_eq!(i["object_ids"], json!([o]), "the O, not the L");
    assert_eq!(i["paths"], 1);
    assert_eq!(i["fixes"], json!([]));
    assert_eq!(i["hint"], flatcraft_engine::report::STENCIL_ISLAND_HINT);
    assert_eq!(islands(&run(&mut s, "document.check", json!({}))).len(), 1);
    // The counter stays kept material, and the hint never blocks the cut.
    let job = job::build(&s).unwrap();
    assert!(job.plan.kept.is_kept(Point::new(40.0, 45.0)), "the counter");
    assert!(!job.plan.kept.is_kept(Point::new(25.0, 45.0)), "the letter");
    // As pieces there is nothing to say.
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [o], "keep": "pieces" }),
    );
    assert!(islands(&run(&mut s, "document.check", json!({}))).is_empty());
}

#[test]
fn a_stencil_on_a_backing_sheet_keeps_its_counters_without_a_hint() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    run(&mut s, "job.set_material", json!({ "id": "vinyl" }));
    let o = letter(&mut s, true);
    let r = run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [o], "keep": "stencil" }),
    );
    assert!(islands(&r["report"]).is_empty(), "{r}");
    assert!(islands(&run(&mut s, "document.check", json!({}))).is_empty());
    let job = job::build(&s).unwrap();
    assert!(job.plan.kept.is_kept(Point::new(40.0, 45.0)), "the counter");
}

#[test]
fn a_project_with_the_old_keep_sheet_strategy_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    square(&mut s, 20.0, "a");
    let path = dir.path().join("old labels.flatcraft");
    run(&mut s, "project.save", json!({ "path": path }));
    flatcraft_core::project::Archive::edit(&path, |a| {
        a.project["job"]["strategy"] = json!("keep_sheet");
    })
    .unwrap();
    let e = s
        .execute("project.open", json!({ "path": path }), Origin::User)
        .unwrap_err()
        .to_string();
    assert!(e.contains("old labels.flatcraft"), "{e}");
    assert!(e.contains("keep_sheet"), "{e}");
}

/// The bounds of object `id` in mm, from document.state.
fn bounds(s: &mut Session, id: u64) -> [f64; 4] {
    let state = run(s, "document.state", json!({}));
    let o = state["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == id)
        .unwrap()
        .clone();
    let b = &o["bounds_mm"];
    ["x0", "y0", "x1", "y1"].map(|k| b[k].as_f64().unwrap())
}

/// A 40 x 50 mm O with a 20 x 30 mm middle, at (`x`, 20) on the page.
fn letter_o(s: &mut Session, x: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><path fill-rule="evenodd" d="M{x} 20 h40 v50 h-40 Z M{} 30 h20 v30 h-20 Z"/></svg>"#,
        x + 10.0
    );
    run(s, "object.import_svg", json!({ "svg": svg, "name": "O" }))["id"]
        .as_u64()
        .unwrap()
}

fn keep_at(s: &mut Session, ids: &[u64], points: &[(f64, f64)]) -> Result<Value, String> {
    let points: Vec<Value> = points
        .iter()
        .map(|(x, y)| json!({ "x_mm": x, "y_mm": y }))
        .collect();
    s.execute(
        "object.keep_at",
        json!({ "ids": ids, "points": points }),
        Origin::Agent,
    )
    .map_err(|e| e.to_string())
}

#[test]
fn keep_at_maps_the_points_that_stay_to_the_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let [x0, y0, x1, y1] = bounds(&mut s, a);
    let inside = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let outside = (x1 + 20.0, (y0 + y1) / 2.0);
    for (points, want, regions) in [
        (vec![inside], KeepMode::Pieces, json!(["shape"])),
        (vec![outside], KeepMode::Stencil, json!(["sheet"])),
        (
            vec![inside, outside],
            KeepMode::Both,
            json!(["shape", "sheet"]),
        ),
        (
            vec![inside, inside],
            KeepMode::Pieces,
            json!(["shape", "shape"]),
        ),
    ] {
        let r = keep_at(&mut s, &[a], &points).unwrap();
        assert_eq!(keep(&s, a), want, "{points:?}");
        assert_eq!((&r["keep"], &r["regions"]), (&json!(want.key()), &regions));
        assert_eq!(r["ids"], json!([a]));
    }
}

#[test]
fn keep_at_refuses_nothing_a_cut_line_and_off_the_mat() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let [x0, y0, _, y1] = bounds(&mut s, a);
    let before = s.doc().undo.undo_len();
    let nothing = keep_at(&mut s, &[a], &[]).unwrap_err();
    assert!(nothing.contains("at least one point"), "{nothing}");
    let on_cut = keep_at(&mut s, &[a], &[(x0, (y0 + y1) / 2.0)]).unwrap_err();
    assert!(on_cut.contains("cut line"), "{on_cut}");
    let off = keep_at(&mut s, &[a], &[(-500.0, 10.0)]).unwrap_err();
    assert!(off.contains("outside the cutting area"), "{off}");
    run(
        &mut s,
        "object.set_cut",
        json!({ "ids": [a], "cut": false }),
    );
    let before_skip = s.doc().undo.undo_len();
    let skipped = keep_at(&mut s, &[a], &[(x0 + 5.0, y0 + 5.0)]).unwrap_err();
    assert!(skipped.contains("hidden or skipped"), "{skipped}");
    assert_eq!(s.doc().undo.undo_len(), before_skip);
    assert_eq!(before + 1, before_skip, "only set_cut changed anything");
    assert_eq!(keep(&s, a), KeepMode::Pieces);
}

#[test]
fn keep_at_the_middle_of_an_o_is_the_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let o = letter_o(&mut s, 20.0);
    let [x0, y0, x1, y1] = bounds(&mut s, o);
    let middle = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let stroke = (x0 + 5.0, (y0 + y1) / 2.0);
    let r = keep_at(&mut s, &[o], &[middle]).unwrap();
    assert_eq!(
        (r["keep"].as_str(), &r["regions"]),
        (Some("stencil"), &json!(["sheet"]))
    );
    let r = keep_at(&mut s, &[o], &[stroke]).unwrap();
    assert_eq!(r["keep"], "pieces");
}

#[test]
fn keep_at_is_one_undo_step_and_reaches_groups_and_linked_copies() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let b = square(&mut s, 100.0, "b");
    let copy = run(
        &mut s,
        "object.copies",
        json!({ "id": a, "count": 1, "linked": true }),
    )["ids"][0]
        .as_u64()
        .unwrap();
    let g = run(&mut s, "object.group", json!({ "ids": [a, b] }))["group_id"]
        .as_u64()
        .unwrap();
    let [_, y0, x1, y1] = bounds(&mut s, b);
    let before = s.doc().undo.undo_len();
    // Inside b, right of the linked copy of a that overlaps its left half.
    let r = keep_at(&mut s, &[g], &[(x1 - 5.0, (y0 + y1) / 2.0)]).unwrap();
    assert_eq!(r["keep"], "pieces", "inside b: the shape of the group");
    assert_eq!(s.doc().undo.undo_len(), before, "Pieces already: no step");
    // Below everything: the sheet (the linked copy of a sits between a
    // and b and counts as the shape too).
    keep_at(&mut s, &[g], &[(x1 + 5.0, y1 + 10.0)]).unwrap();
    assert_eq!(s.doc().undo.undo_len(), before + 1);
    for id in [a, b, copy] {
        assert_eq!(keep(&s, id), KeepMode::Stencil, "object {id}");
    }
    run(&mut s, "edit.undo", json!({}));
    for id in [a, b, copy] {
        assert_eq!(keep(&s, id), KeepMode::Pieces, "object {id} after undo");
    }
}

#[test]
fn keep_at_a_linked_copy_counts_as_the_shape() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let a = square(&mut s, 20.0, "a");
    let copy = run(
        &mut s,
        "object.copies",
        json!({ "id": a, "count": 1, "linked": true }),
    )["ids"][0]
        .as_u64()
        .unwrap();
    run(
        &mut s,
        "object.set_keep",
        json!({ "ids": [a], "keep": "stencil" }),
    );
    // Only a is given, the point lies in its linked copy: the shape.
    let [x0, y0, x1, y1] = bounds(&mut s, copy);
    let r = keep_at(&mut s, &[a], &[((x0 + x1) / 2.0, (y0 + y1) / 2.0)]).unwrap();
    assert_eq!(
        (r["keep"].as_str(), &r["regions"]),
        (Some("pieces"), &json!(["shape"]))
    );
    assert_eq!(
        (keep(&s, a), keep(&s, copy)),
        (KeepMode::Pieces, KeepMode::Pieces)
    );
}
