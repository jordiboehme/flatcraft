//! The Arrange commands pinned to the ribbon: stored in the preferences,
//! read-modify-write, with defaults for files that do not have them.

use flatcraft_engine::prefs::{Preferences, default_arrange_pins};

#[test]
fn a_new_install_pins_three() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        Preferences::load(dir.path()).arrange_pins,
        ["move_copy", "mirror", "rectangular_pattern"]
    );
    assert_eq!(Preferences::default().arrange_pins, default_arrange_pins());
}

#[test]
fn pin_and_unpin_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let current = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    let (p, written) = Preferences::update(dir.path(), &current, |p| {
        p.arrange_pins.push("circular_pattern".into());
    });
    written.unwrap();
    assert!(p.arrange_pins.contains(&"circular_pattern".to_string()));
    let loaded = Preferences::load(dir.path());
    assert_eq!(
        loaded.arrange_pins,
        [
            "move_copy",
            "mirror",
            "rectangular_pattern",
            "circular_pattern"
        ]
    );
    let (_, written) = Preferences::update(dir.path(), &loaded, |p| {
        p.arrange_pins.retain(|id| id != "mirror");
    });
    written.unwrap();
    assert_eq!(
        Preferences::load(dir.path()).arrange_pins,
        ["move_copy", "rectangular_pattern", "circular_pattern"]
    );
}

#[test]
fn old_files_get_the_defaults_and_an_empty_list_stays_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("preferences.toml"), "rotate_each = true\n").unwrap();
    let p = Preferences::load(dir.path());
    assert!(p.rotate_each);
    assert_eq!(p.arrange_pins, default_arrange_pins());
    std::fs::write(dir.path().join("preferences.toml"), "arrange_pins = []\n").unwrap();
    assert!(Preferences::load(dir.path()).arrange_pins.is_empty());
}
