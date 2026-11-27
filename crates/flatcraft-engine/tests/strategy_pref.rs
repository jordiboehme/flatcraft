//! The default cut strategy of new documents: a preference only the
//! Settings window writes.

use std::sync::{Arc, Mutex};

use flatcraft_core::document::CutStrategy;
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use serde_json::json;

#[test]
fn old_files_get_quality() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("preferences.toml"),
        "allow_agent_cuts = false\n",
    )
    .unwrap();
    assert_eq!(
        Preferences::load(dir.path()).default_strategy,
        CutStrategy::Quality
    );
    assert_eq!(
        Preferences::default().default_strategy,
        CutStrategy::Quality
    );
}

#[test]
fn the_stored_strategy_round_trips_and_new_documents_use_it() {
    let dir = tempfile::tempdir().unwrap();
    let profiles = ProfileStore::load(&dir.path().join("profiles")).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    let s = Arc::new(Mutex::new(Session::new(
        profiles,
        prefs,
        false,
        dir.path().to_path_buf(),
    )));
    let mut s = s.lock().unwrap();
    s.update_prefs(|p| p.default_strategy = CutStrategy::OutlinesLast)
        .unwrap();
    let text = std::fs::read_to_string(dir.path().join("preferences.toml")).unwrap();
    assert!(
        text.contains("default_strategy = \"outlines_last\""),
        "{text}"
    );
    assert_eq!(
        Preferences::load(dir.path()).default_strategy,
        CutStrategy::OutlinesLast
    );
    s.execute("document.new", json!({}), Origin::Agent).unwrap();
    assert_eq!(s.doc().document.job.strategy, CutStrategy::OutlinesLast);
}

#[test]
fn an_unknown_stored_strategy_gives_quality_and_keeps_the_rest() {
    // "keep_sheet" was renamed to "outlines_last"; the old value is not
    // read, but it never resets the other preferences.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("preferences.toml"),
        "default_strategy = \"keep_sheet\"\nrotate_each = true\n",
    )
    .unwrap();
    let (prefs, warning) = Preferences::load_reporting(dir.path());
    assert_eq!(warning, None);
    assert_eq!(prefs.default_strategy, CutStrategy::Quality);
    assert!(prefs.rotate_each);
}
