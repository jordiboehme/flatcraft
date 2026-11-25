//! The ribbon actions pinned as buttons: stored in the preferences,
//! read-modify-write, with defaults for files that do not have them and a
//! migration of the Arrange pins of earlier versions.

use flatcraft_engine::prefs::{
    DEFAULT_AUTO_SPACING_MM, Preferences, RIBBON_PIN_IDS, clean_ribbon_pins, default_ribbon_pins,
    migrate_arrange_pins,
};

fn strings(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn write(dir: &tempfile::TempDir, text: &str) {
    std::fs::write(dir.path().join("preferences.toml"), text).unwrap();
}

/// What Home and View pin on a new install: every button they had.
const HOME_VIEW: [&str; 14] = [
    "import",
    "trace",
    "test_patterns",
    "duplicate",
    "copies",
    "delete",
    "reload",
    "zoom.fit",
    "zoom.100",
    "view.grid",
    "view.rulers",
    "panel.objects",
    "panel.settings",
    "panel.debug",
];

const ARRANGE: [&str; 6] = [
    "move_copy",
    "pattern.rectangular",
    "rotate.left",
    "rotate.right",
    "mirror.horizontal",
    "mirror.vertical",
];

fn with_home_view(arrange: &[&str]) -> Vec<String> {
    let mut v = strings(&HOME_VIEW);
    v.extend(strings(arrange));
    v
}

#[test]
fn a_new_install_pins_what_the_ribbon_showed() {
    let dir = tempfile::tempdir().unwrap();
    let p = Preferences::load(dir.path());
    assert_eq!(p.ribbon_pins, with_home_view(&ARRANGE));
    assert_eq!(Preferences::default().ribbon_pins, default_ribbon_pins());
    assert!(
        p.ribbon_pins
            .iter()
            .all(|d| RIBBON_PIN_IDS.contains(&d.as_str()))
    );
    assert_eq!(p.auto_arrange_spacing_mm, DEFAULT_AUTO_SPACING_MM);
}

#[test]
fn pin_and_unpin_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let current = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    let (p, written) = Preferences::update(dir.path(), &current, |p| {
        p.ribbon_pins.push("align.left.area".into());
        p.ribbon_pins.retain(|id| id != "view.grid");
    });
    written.unwrap();
    assert!(p.ribbon_pins.contains(&"align.left.area".to_string()));
    let loaded = Preferences::load(dir.path());
    assert_eq!(loaded.ribbon_pins, p.ribbon_pins);
    assert!(!loaded.ribbon_pins.contains(&"view.grid".to_string()));
    // The new key is written, the old one is not.
    let text = std::fs::read_to_string(dir.path().join("preferences.toml")).unwrap();
    assert!(
        text.contains("ribbon_pins") && !text.contains("arrange_pins"),
        "{text}"
    );
}

#[test]
fn an_empty_list_stays_empty() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "ribbon_pins = []\n");
    assert!(Preferences::load(dir.path()).ribbon_pins.is_empty());
}

#[test]
fn old_files_without_pins_get_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "rotate_each = true\n");
    let p = Preferences::load(dir.path());
    assert!(p.rotate_each);
    assert_eq!(p.ribbon_pins, default_ribbon_pins());
}

#[test]
fn task_33_arrange_pins_move_over_with_their_align_choice() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir,
        "arrange_pins = [\"move_copy\", \"align_left\", \"align_vcenter\", \"distribute_vertical\", \"auto_arrange\", \"teleport\"]\n",
    );
    // The old builds never stored the Align choice: every session started
    // with "To the selection", so that is what the old pins ran with.
    assert_eq!(
        Preferences::load(dir.path()).ribbon_pins,
        with_home_view(&[
            "move_copy",
            "align.left.selection",
            "align.middle.selection",
            "distribute.vertical",
            "auto_arrange",
        ])
    );
    let old = |ids: &[&str], area: bool| migrate_arrange_pins(&strings(ids), area);
    assert_eq!(
        old(&["align_left", "align_hcenter"], true),
        ["align.left.area", "align.centre.area"]
    );
    assert_eq!(old(&["align_bottom"], false), ["align.bottom.selection"]);
    assert_eq!(old(&["align"], false).len(), 6);
    assert_eq!(
        old(&["mirror", "mirror_vertical"], false),
        ["mirror.horizontal", "mirror.vertical"]
    );
    assert_eq!(
        old(&["distribute"], false),
        ["distribute.horizontal", "distribute.vertical"]
    );
    assert_eq!(
        old(
            &["rotate", "rectangular_pattern", "circular_pattern"],
            false
        ),
        ["rotate", "pattern.rectangular", "pattern.circular"]
    );
    // Only unknown ids: the Arrange defaults, not an empty tab.
    assert_eq!(old(&["nope", ""], false), strings(&ARRANGE));
    assert!(old(&[], false).is_empty());
}

#[test]
fn an_old_empty_arrange_list_empties_only_the_arrange_tab() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "arrange_pins = []\n");
    assert_eq!(
        Preferences::load(dir.path()).ribbon_pins,
        strings(&HOME_VIEW)
    );
}

#[test]
fn a_new_key_wins_over_an_old_one() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir,
        "ribbon_pins = [\"zoom.fit\"]\narrange_pins = [\"mirror\"]\n",
    );
    assert_eq!(Preferences::load(dir.path()).ribbon_pins, ["zoom.fit"]);
}

#[test]
fn unknown_ids_and_repeats_are_dropped() {
    assert_eq!(
        clean_ribbon_pins(&strings(&["zoom.fit", "warp_drive", "zoom.fit"])),
        ["zoom.fit"]
    );
    // Only unknown ids: the defaults.
    assert_eq!(
        clean_ribbon_pins(&strings(&["teleport"])),
        default_ribbon_pins()
    );
    let dir = tempfile::tempdir().unwrap();
    write(&dir, "ribbon_pins = [\"teleport\", \"warp_drive\"]\n");
    assert_eq!(
        Preferences::load(dir.path()).ribbon_pins,
        default_ribbon_pins()
    );
}

#[test]
fn the_auto_arrange_spacing_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let current = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    let (_, written) = Preferences::update(dir.path(), &current, |p| {
        p.auto_arrange_spacing_mm = 2.5;
    });
    written.unwrap();
    assert_eq!(Preferences::load(dir.path()).auto_arrange_spacing_mm, 2.5);
    // A value no one can use falls back to the default.
    write(&dir, "auto_arrange_spacing_mm = -3.0\n");
    assert_eq!(
        Preferences::load(dir.path()).auto_arrange_spacing_mm,
        DEFAULT_AUTO_SPACING_MM
    );
}
