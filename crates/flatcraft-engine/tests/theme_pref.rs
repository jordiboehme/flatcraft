//! The theme preference: new installs follow the system, old files keep
//! their value.

use flatcraft_engine::prefs::{Preferences, Theme};

#[test]
fn a_new_install_follows_the_system() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Preferences::load(dir.path()).theme, Theme::System);
    assert_eq!(Preferences::default().theme, Theme::System);
}

#[test]
fn system_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let p = Preferences {
        theme: Theme::System,
        ..Preferences::default()
    };
    p.save(dir.path()).unwrap();
    let text = std::fs::read_to_string(dir.path().join("preferences.toml")).unwrap();
    assert!(text.contains("theme = \"System\""), "{text}");
    assert_eq!(Preferences::load(dir.path()).theme, Theme::System);
}

#[test]
fn old_files_keep_their_theme() {
    for (text, want) in [
        ("theme = \"Dark\"\n", Theme::Dark),
        ("theme = \"Light\"\n", Theme::Light),
        // A file that exists but has no theme key came from a build that
        // only knew Dark.
        ("allow_agent_cuts = true\n", Theme::Dark),
        ("", Theme::Dark),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("preferences.toml"), text).unwrap();
        assert_eq!(Preferences::load(dir.path()).theme, want, "{text:?}");
    }
}
