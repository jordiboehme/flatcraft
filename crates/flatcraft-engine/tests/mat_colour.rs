//! The mat colour: a mat without a colour follows the Settings default of
//! the theme, a mat with one keeps it in both themes.

use flatcraft_device::{DEFAULT_MAT_COLOR_DARK, DEFAULT_MAT_COLOR_LIGHT, Grid, Media, MediaKind};
use flatcraft_engine::prefs::{Preferences, resolved_mat_colour};

fn mat(color: Option<[u8; 3]>) -> Media {
    Media {
        id: "m".into(),
        name: "M".into(),
        kind: MediaKind::Mat,
        width_mm: 300.0,
        height_mm: 300.0,
        grid: Grid {
            origin_x_mm: 0.0,
            origin_y_mm: 0.0,
            width_mm: 300.0,
            height_mm: 300.0,
        },
        calibrated: false,
        color,
    }
}

#[test]
fn default_mat_follows_preferences_per_theme() {
    let mut p = Preferences::default();
    assert_eq!(p.mat_color_light, [0xFF, 0xFF, 0xFF]);
    assert_eq!(p.mat_color_dark, [0xE8, 0xE9, 0xEB]);
    let m = mat(None);
    assert_eq!(resolved_mat_colour(&m, &p, false), DEFAULT_MAT_COLOR_LIGHT);
    assert_eq!(resolved_mat_colour(&m, &p, true), DEFAULT_MAT_COLOR_DARK);
    p.mat_color_light = [1, 2, 3];
    p.mat_color_dark = [4, 5, 6];
    assert_eq!(resolved_mat_colour(&m, &p, false), [1, 2, 3]);
    assert_eq!(resolved_mat_colour(&m, &p, true), [4, 5, 6]);
}

#[test]
fn custom_mat_colour_overrides_default() {
    let p = Preferences::default();
    let m = mat(Some([0x2E, 0x7D, 0x32]));
    for dark in [false, true] {
        assert_eq!(resolved_mat_colour(&m, &p, dark), [0x2E, 0x7D, 0x32]);
    }
}

#[test]
fn mat_colours_round_trip_and_old_files_load() {
    let dir = tempfile::tempdir().unwrap();
    let p = Preferences {
        mat_color_light: [0xFA, 0xFA, 0xF0],
        mat_color_dark: [0x20, 0x21, 0x22],
        ..Preferences::default()
    };
    p.save(dir.path()).unwrap();
    let text = std::fs::read_to_string(dir.path().join("preferences.toml")).unwrap();
    assert!(text.contains("#FAFAF0"), "{text}");
    assert_eq!(Preferences::load(dir.path()), p);
    // A file without the keys, or with a bad one, gets the defaults.
    std::fs::write(
        dir.path().join("preferences.toml"),
        "theme = \"Light\"\nmat_color_light = \"red\"\n",
    )
    .unwrap();
    let q = Preferences::load(dir.path());
    assert_eq!(q.mat_color_light, DEFAULT_MAT_COLOR_LIGHT);
    assert_eq!(q.mat_color_dark, DEFAULT_MAT_COLOR_DARK);
}
