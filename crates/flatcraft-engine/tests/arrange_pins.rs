//! The Arrange commands pinned to the ribbon: stored in the preferences,
//! read-modify-write, with defaults for files that do not have them.

use flatcraft_engine::prefs::{
    ARRANGE_PIN_IDS, Preferences, default_arrange_pins, migrate_arrange_pins,
};

const DEFAULTS: [&str; 6] = [
    "move_copy",
    "rectangular_pattern",
    "rotate_left",
    "rotate_right",
    "mirror_horizontal",
    "mirror_vertical",
];

#[test]
fn a_new_install_pins_six() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Preferences::load(dir.path()).arrange_pins, DEFAULTS);
    assert!(DEFAULTS.iter().all(|d| ARRANGE_PIN_IDS.contains(d)));
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
    let mut want: Vec<&str> = DEFAULTS.to_vec();
    want.push("circular_pattern");
    assert_eq!(loaded.arrange_pins, want);
    let (_, written) = Preferences::update(dir.path(), &loaded, |p| {
        p.arrange_pins.retain(|id| id != "mirror_vertical");
    });
    written.unwrap();
    want.retain(|id| *id != "mirror_vertical");
    assert_eq!(Preferences::load(dir.path()).arrange_pins, want);
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

#[test]
fn old_whole_command_pins_become_their_directions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("preferences.toml"),
        "arrange_pins = [\"move_copy\", \"mirror\", \"rectangular_pattern\", \"teleport\"]\n",
    )
    .unwrap();
    assert_eq!(
        Preferences::load(dir.path()).arrange_pins,
        [
            "move_copy",
            "mirror_horizontal",
            "mirror_vertical",
            "rectangular_pattern"
        ],
        "Mirror becomes both directions, unknown ids are dropped"
    );
    let old =
        |ids: &[&str]| migrate_arrange_pins(&ids.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(
        old(&["distribute"]),
        ["distribute_horizontal", "distribute_vertical"]
    );
    assert_eq!(old(&["align"]).len(), 6);
    assert_eq!(
        old(&["mirror", "mirror_vertical"]),
        ["mirror_horizontal", "mirror_vertical"]
    );
    assert_eq!(
        old(&["nope", ""]),
        default_arrange_pins(),
        "only unknown ids: the defaults, not an empty ribbon"
    );
    assert!(old(&[]).is_empty());
}

#[test]
fn a_file_with_only_unknown_pins_gets_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("preferences.toml"),
        "arrange_pins = [\"teleport\", \"warp_drive\"]\n",
    )
    .unwrap();
    assert_eq!(Preferences::load(dir.path()).arrange_pins, DEFAULTS);
    std::fs::write(dir.path().join("preferences.toml"), "arrange_pins = []\n").unwrap();
    assert!(Preferences::load(dir.path()).arrange_pins.is_empty());
}
