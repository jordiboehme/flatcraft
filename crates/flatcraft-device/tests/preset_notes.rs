//! Notes on plotter and material presets: user presets keep them in their
//! own file, system presets in a user-side annotation file, so the system
//! presets stay as they ship.

use std::fs;

use flatcraft_device::*;
use tempfile::tempdir;

fn files_under(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in walk(dir) {
        out.push(
            e.strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    out.sort();
    out
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn system_presets_have_no_notes_until_the_person_adds_some() {
    let d = tempdir().unwrap();
    let s = ProfileStore::load(d.path()).unwrap();
    assert_eq!(s.material("vinyl").unwrap().notes, None);
    assert_eq!(s.plotter("silhouette-cameo-2").unwrap().notes, None);
}

#[test]
fn notes_on_a_system_preset_go_into_the_annotation_file() {
    let d = tempdir().unwrap();
    let mut s = ProfileStore::load(d.path()).unwrap();
    s.set_material_notes("vinyl", "Glossy vinyl: force 10 works.")
        .unwrap();
    s.set_plotter_notes("silhouette-cameo-2", "Blade 3 for cardstock.")
        .unwrap();
    assert_eq!(
        s.material("vinyl").unwrap().notes.as_deref(),
        Some("Glossy vinyl: force 10 works.")
    );
    assert_eq!(
        s.plotter("silhouette-cameo-2").unwrap().notes.as_deref(),
        Some("Blade 3 for cardstock.")
    );
    // Only the annotation file: no user preset shadows the system one.
    assert_eq!(files_under(d.path()), [PRESET_NOTES_FILE]);
    assert!(s.materials().iter().any(|m| m.id == "vinyl" && m.system));

    // They come back after a restart and after a reload.
    let mut again = ProfileStore::load(d.path()).unwrap();
    assert_eq!(
        again.material("vinyl").unwrap().notes.as_deref(),
        Some("Glossy vinyl: force 10 works.")
    );
    again.reload().unwrap();
    assert_eq!(
        again
            .plotter("silhouette-cameo-2")
            .unwrap()
            .notes
            .as_deref(),
        Some("Blade 3 for cardstock.")
    );

    // A child does not inherit them: notes belong to one preset.
    assert_eq!(
        again.plotter("file-output-gpgl-cameo2").unwrap().notes,
        None
    );

    // Empty text removes them.
    again.set_material_notes("vinyl", "  ").unwrap();
    assert_eq!(again.material("vinyl").unwrap().notes, None);
}

#[test]
fn a_user_preset_keeps_its_notes_in_its_own_file() {
    let d = tempdir().unwrap();
    let mut s = ProfileStore::load(d.path()).unwrap();
    let mut mine = s.material("vinyl").unwrap();
    mine.id = "my-vinyl".into();
    mine.name = "My vinyl".into();
    mine.notes = Some("Weed within a day.".into());
    s.save_user_material(&mine, "vinyl").unwrap();
    let text = fs::read_to_string(d.path().join("materials/my-vinyl.toml")).unwrap();
    assert!(text.contains("Weed within a day."), "{text}");
    let again = ProfileStore::load(d.path()).unwrap();
    assert_eq!(
        again.material("my-vinyl").unwrap().notes.as_deref(),
        Some("Weed within a day.")
    );

    // A child of a user preset with notes does not inherit them, and
    // clearing a preset's notes is fine.
    let mut child = again.material("my-vinyl").unwrap();
    child.id = "child".into();
    child.name = "Child".into();
    child.notes = None;
    let mut s = again;
    s.save_user_material(&child, "my-vinyl").unwrap();
    assert_eq!(s.material("child").unwrap().notes, None);
    let mut mine = s.material("my-vinyl").unwrap();
    mine.notes = None;
    s.save_user_material(&mine, "vinyl").unwrap();
    assert_eq!(s.material("my-vinyl").unwrap().notes, None);

    // The notes setter works on a user preset too.
    s.set_material_notes("my-vinyl", "Back again.").unwrap();
    let again = ProfileStore::load(d.path()).unwrap();
    assert_eq!(
        again.material("my-vinyl").unwrap().notes.as_deref(),
        Some("Back again.")
    );
    assert!(
        again.modified_material_fields("my-vinyl").is_empty(),
        "notes are no inherited value"
    );
}

#[test]
fn notes_on_an_unknown_preset_are_refused() {
    let d = tempdir().unwrap();
    let mut s = ProfileStore::load(d.path()).unwrap();
    assert!(matches!(
        s.set_material_notes("nope", "x"),
        Err(ProfileError::UnknownPreset { .. })
    ));
    assert!(files_under(d.path()).is_empty());
}

#[test]
fn a_broken_annotation_file_is_reported_and_never_overwritten() {
    let d = tempdir().unwrap();
    fs::write(d.path().join(PRESET_NOTES_FILE), "this is [ not toml").unwrap();
    let mut s = ProfileStore::load(d.path()).unwrap();
    assert_eq!(s.material("vinyl").unwrap().notes, None);
    assert!(
        s.diagnostics()
            .iter()
            .any(|x| x.path == d.path().join(PRESET_NOTES_FILE)),
        "{:?}",
        s.diagnostics()
    );
    assert!(s.set_material_notes("vinyl", "x").is_err());
    assert_eq!(
        fs::read_to_string(d.path().join(PRESET_NOTES_FILE)).unwrap(),
        "this is [ not toml"
    );
}
