//! `report::check_selection` builds the cut geometry of fewer objects
//! than `report::check`, but gives the same report for the objects asked
//! for: pairs, Move apart fixes, merged and visible outlines, groups and
//! linked copies included.

use flatcraft_core::document::ObjectId;
use flatcraft_device::ProfileStore;
use flatcraft_engine::job::JobInput;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::report;
use flatcraft_engine::session::{Origin, Session};
use serde_json::{Value, json};

fn session(dir: &tempfile::TempDir) -> Session {
    let profiles = ProfileStore::load(dir.path()).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("out")),
        ..Preferences::default()
    };
    Session::new(profiles, prefs, false, dir.path().to_path_buf())
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, params, Origin::User)
        .unwrap_or_else(|e| panic!("{id} failed: {e}"))
}

/// Imports `body` (in a 100 mm page) and puts its box at `x`, `y`.
fn put(s: &mut Session, name: &str, body: &str, x: f64, y: f64) -> u64 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">{body}</svg>"#
    );
    let id = run(s, "object.import_svg", json!({ "name": name, "svg": svg }))["id"]
        .as_u64()
        .unwrap();
    run(
        s,
        "object.move",
        json!({ "ids": [id], "x_mm": x, "y_mm": y }),
    );
    id
}

const SQUARE: &str = r#"<rect x="0" y="0" width="20" height="20"/>"#;
const RING: &str = r#"<circle cx="10" cy="10" r="10"/><circle cx="10" cy="10" r="5"/>"#;
const TINY: &str =
    r#"<rect x="0" y="0" width="20" height="20"/><rect x="30" y="0" width="0.5" height="0.5"/>"#;

/// A mat of objects with every kind of neighbourhood: close pairs, an
/// overlapping pair, a merged weld chain, a visible outline under two
/// objects, a group, linked copies, a tiny shape, an object off the mat
/// and lone objects far away.
fn mat(s: &mut Session) -> Vec<u64> {
    // A close pair and an overlapping pair.
    let mut ids = vec![
        put(s, "a", SQUARE, 10.0, 10.0),
        put(s, "b", SQUARE, 30.4, 10.0),
        put(s, "c", SQUARE, 60.0, 10.0),
        put(s, "d", SQUARE, 70.0, 15.0),
    ];
    // A weld chain: e-f-g overlap one after the other.
    let e = put(s, "e", RING, 10.0, 60.0);
    let f = put(s, "f", RING, 25.0, 60.0);
    let g = put(s, "g", RING, 40.0, 60.0);
    run(
        s,
        "object.set_cut_options",
        json!({ "ids": [e, f, g], "mode": "merged_outline" }),
    );
    ids.extend([e, f, g]);
    // A visible outline under two objects, one of them close to a third.
    let v = put(s, "v", SQUARE, 100.0, 60.0);
    run(
        s,
        "object.set_cut_options",
        json!({ "ids": [v], "mode": "visible_outline" }),
    );
    ids.push(v);
    ids.push(put(s, "w", SQUARE, 110.0, 65.0));
    ids.push(put(s, "x", SQUARE, 95.0, 70.0));
    ids.push(put(s, "y", SQUARE, 115.3, 85.5));
    // A group of two close letters and a loose object near it.
    let h = put(s, "h", SQUARE, 10.0, 120.0);
    let i = put(s, "i", SQUARE, 30.3, 120.0);
    run(s, "object.group", json!({ "ids": [h, i] }));
    ids.extend([h, i]);
    ids.push(put(s, "j", SQUARE, 50.6, 120.0));
    // Linked copies, one close to another object.
    let k = put(s, "k", TINY, 100.0, 120.0);
    let copy = run(s, "object.duplicate", json!({ "ids": [k] }));
    let k2 = copy["ids"][0]
        .as_u64()
        .or_else(|| copy["id"].as_u64())
        .unwrap();
    run(
        s,
        "object.move",
        json!({ "ids": [k2], "x_mm": 150.0, "y_mm": 200.0 }),
    );
    run(s, "object.link", json!({ "ids": [k, k2] }));
    ids.extend([k, k2]);
    ids.push(put(s, "l", SQUARE, 171.0, 200.0));
    // Off the mat, and far away from everything.
    ids.push(put(s, "off", SQUARE, 500.0, 500.0));
    ids.push(put(s, "far", SQUARE, 250.0, 250.0));
    ids
}

fn same(input: &JobInput, ids: &[u64]) -> bool {
    let ids: Vec<ObjectId> = ids.iter().map(|i| ObjectId(*i)).collect();
    let full = report::check(input, Some(&ids)).to_json();
    let (narrow, whole) = report::check_selection(input, &ids);
    assert_eq!(narrow.to_json(), full, "{ids:?}");
    whole
}

#[test]
fn every_selection_gets_the_same_report_as_the_full_check() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let ids = mat(&mut s);
    let input = JobInput::of(&s).unwrap();
    // The mat has every kind of issue.
    let all = report::check(&input, None).to_json().to_string();
    for code in ["objects_close", "objects_overlap", "below_min_feature"] {
        assert!(all.contains(code), "{code}: {all}");
    }
    let mut narrowed = 0;
    for id in &ids {
        if !same(&input, &[*id]) {
            narrowed += 1;
        }
    }
    // A lone object far away checks only its neighbourhood.
    assert!(narrowed > 0, "check_selection never narrowed");
    for set in [
        &ids[0..2],
        &ids[4..7],
        &ids[7..11],
        &ids[11..14],
        &ids[14..17],
        &ids[..],
    ] {
        same(&input, set);
    }
    // The whole selection is the whole document: its geometry is the
    // document's.
    assert!(same(&input, &ids));
}

#[test]
fn a_far_object_does_not_build_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    let ids = mat(&mut s);
    let input = JobInput::of(&s).unwrap();
    let far = *ids.last().unwrap();
    let (r, whole) = report::check_selection(&input, &[ObjectId(far)]);
    assert!(!whole);
    assert!(r.issues.is_empty(), "{:?}", r.to_json());
}

#[test]
fn random_mats_get_the_same_report() {
    let mut seed: u64 = 0x5eed_1234;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for _ in 0..4 {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(&dir);
        let mut ids = Vec::new();
        for k in 0..30 {
            let body = [SQUARE, RING, TINY][next(3) as usize];
            let x = next(280) as f64 + next(10) as f64 / 10.0;
            let y = next(280) as f64 + next(10) as f64 / 10.0;
            let id = put(&mut s, &format!("o{k}"), body, x, y);
            let mode = [
                "object_outlines",
                "all_lines",
                "merged_outline",
                "visible_outline",
            ][next(4) as usize];
            run(
                &mut s,
                "object.set_cut_options",
                json!({ "ids": [id], "mode": mode }),
            );
            ids.push(id);
        }
        run(
            &mut s,
            "object.group",
            json!({ "ids": [ids[0], ids[1], ids[2]] }),
        );
        let input = JobInput::of(&s).unwrap();
        for id in &ids {
            same(&input, &[*id]);
        }
        same(&input, &ids[3..9]);
    }
}
