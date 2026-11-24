use std::fs;

use flatcraft_device::*;
use kurbo::Rect;
use tempfile::tempdir;

fn write(dir: &std::path::Path, sub: &str, id: &str, body: &str) {
    let d = dir.join(sub);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(format!("{id}.toml")), body).unwrap();
}

fn store() -> (tempfile::TempDir, ProfileStore) {
    let d = tempdir().unwrap();
    let s = ProfileStore::load(d.path()).unwrap();
    (d, s)
}

#[test]
fn system_presets_load() {
    let (_d, s) = store();
    assert_eq!(s.plotters().len(), 2);
    assert_eq!(s.materials().len(), 4);
    assert!(s.plotters().iter().all(|p| p.system));
}

#[test]
fn file_output_inherits_cameo() {
    let (_d, s) = store();
    let p = s.plotter("file-output-gpgl-cameo2").unwrap();
    assert_eq!(p.speed_range, [1, 10]);
    assert_eq!(p.knife_compensation, KnifeCompensation::Firmware { fc: 18 });
    assert_eq!(p.name, "File Output (GPGL Cameo 2)");
    assert_eq!(p.inherits.as_deref(), Some("silhouette-cameo-2"));
    assert!(matches!(
        p.transport,
        TransportSpec::File {
            directory: None,
            ..
        }
    ));
}

#[test]
fn user_override_merges_and_reports_modified() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "mine",
        "id = \"mine\"\nname = \"Mine\"\ninherits = \"silhouette-cameo-2\"\nscale_x = 1.002\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let p = s.plotter("mine").unwrap();
    assert_eq!(p.scale_x, 1.002);
    assert_eq!(p.scale_y, 1.0);
    assert_eq!(s.modified_fields("mine"), vec!["scale_x"]);
    assert!(s.modified_fields("silhouette-cameo-2").is_empty());
}

#[test]
fn nested_table_merges_key_by_key() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "mine",
        "id = \"mine\"\nname = \"M\"\ninherits = \"silhouette-cameo-2\"\n[axis]\nmirror_x = true\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let p = s.plotter("mine").unwrap();
    assert!(p.axis.mirror_x);
    assert!(!p.axis.mirror_y);
    assert_eq!(s.modified_fields("mine"), vec!["axis.mirror_x"]);
}

#[test]
fn old_profile_with_margins_still_loads() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "old",
        "id = \"old\"\nname = \"Old\"\ninherits = \"silhouette-cameo-2\"\n[margins]\ntop = 9.0\nright = 9.0\nbottom = 9.0\nleft = 9.0\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let p = s.plotter("old").unwrap();
    // margins are ignored: the uncalibrated mat keeps the 5 mm safety band
    let r = p.cutting_area("mat-12x12").unwrap();
    assert!((r.x0 - 5.0).abs() < 1e-9 && (r.x1 - 299.8).abs() < 1e-9);
    // the legacy key is gone, not a ghost modified field
    assert!(
        s.modified_fields("old").is_empty(),
        "{:?}",
        s.modified_fields("old")
    );
}

#[test]
fn old_media_without_grid_loads_with_defaults() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "oldmedia",
        "id = \"oldmedia\"\nname = \"O\"\ninherits = \"silhouette-cameo-2\"\n\
         media = [{ id = \"a4\", name = \"A4\", width_mm = 210.0, height_mm = 297.0 }]\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let p = s.plotter("oldmedia").unwrap();
    let m = p.media.iter().find(|m| m.id == "a4").unwrap();
    assert_eq!(m.kind, MediaKind::Mat);
    assert!(!m.calibrated);
    assert_eq!(
        m.grid,
        Grid {
            origin_x_mm: 0.0,
            origin_y_mm: 0.0,
            width_mm: 210.0,
            height_mm: 297.0
        }
    );
}

#[test]
fn builtin_mats_match_the_brief() {
    let (_d, s) = store();
    let p = s.plotter("silhouette-cameo-2").unwrap();
    let get = |id: &str| p.media.iter().find(|m| m.id == id).unwrap().clone();
    let m = get("mat-12x12");
    assert_eq!((m.width_mm, m.height_mm), (304.8, 330.2));
    assert_eq!((m.grid.width_mm, m.grid.height_mm), (304.8, 304.8));
    let m = get("mat-12x24");
    assert_eq!((m.width_mm, m.height_mm), (304.8, 635.0));
    assert_eq!((m.grid.width_mm, m.grid.height_mm), (304.8, 609.6));
    let m = get("no-mat");
    assert_eq!(m.kind, MediaKind::NoMat);
    assert_eq!((m.width_mm, m.height_mm), (304.8, 3000.0));
    assert_eq!((m.grid.width_mm, m.grid.height_mm), (304.8, 3000.0));
    assert!(p.media.iter().all(|m| !m.calibrated));
}

#[test]
fn fc_value_is_zero_unless_firmware() {
    assert_eq!(KnifeCompensation::Firmware { fc: 18 }.fc_value(), 18);
    assert_eq!(
        KnifeCompensation::Host {
            offset_mm: 0.25,
            pivot_threshold_deg: 10.0
        }
        .fc_value(),
        0
    );
    assert_eq!(KnifeCompensation::None.fc_value(), 0);
}

#[test]
fn override_enum_replaces_variant() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "mine",
        "id = \"mine\"\nname = \"M\"\ninherits = \"silhouette-cameo-2\"\n\
         transport = { File = { extension = \"gpgl\" } }\n\
         knife_compensation = \"None\"\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let p = s.plotter("mine").unwrap();
    assert_eq!(
        p.transport,
        TransportSpec::File {
            directory: None,
            extension: "gpgl".into()
        }
    );
    assert_eq!(p.knife_compensation, KnifeCompensation::None);
    assert_eq!(
        s.modified_fields("mine"),
        vec!["knife_compensation", "transport"]
    );
}

#[test]
fn cycle_is_an_error() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "a",
        "id = \"a\"\nname = \"A\"\ninherits = \"b\"\n",
    );
    write(
        d.path(),
        "plotters",
        "b",
        "id = \"b\"\nname = \"B\"\ninherits = \"a\"\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    assert!(matches!(
        s.plotter("a"),
        Err(ProfileError::Inheritance { .. })
    ));
}

#[test]
fn unknown_parent_is_an_error() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "a",
        "id = \"a\"\nname = \"A\"\ninherits = \"nope\"\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    assert!(matches!(
        s.plotter("a"),
        Err(ProfileError::Inheritance { .. })
    ));
    assert!(matches!(
        s.plotter("zzz"),
        Err(ProfileError::UnknownPreset { .. })
    ));
}

#[test]
fn user_preset_cannot_shadow_system() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "silhouette-cameo-2",
        "id = \"silhouette-cameo-2\"\nname = \"X\"\ninherits = \"file-output-gpgl-cameo2\"\n",
    );
    assert!(matches!(
        ProfileStore::load(d.path()),
        Err(ProfileError::ShadowsSystem { .. })
    ));
}

#[test]
fn uncalibrated_mat_keeps_safety_margin() {
    let (_d, s) = store();
    let p = s.plotter("silhouette-cameo-2").unwrap();
    let r = p.cutting_area("mat-12x12").unwrap();
    let want = Rect::new(5.0, 5.0, 299.8, 299.8);
    assert!((r.x0 - want.x0).abs() < 1e-9 && (r.y0 - want.y0).abs() < 1e-9);
    assert!((r.x1 - want.x1).abs() < 1e-9 && (r.y1 - want.y1).abs() < 1e-9);
    assert!(p.cutting_area("nope").is_none());
}

#[test]
fn calibrated_mat_uses_full_grid() {
    let (_d, s) = store();
    let mut p = s.plotter("silhouette-cameo-2").unwrap();
    p.media[0].calibrated = true;
    let r = p.cutting_area("mat-12x12").unwrap();
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (0.0, 0.0, 304.8, 304.8));
}

#[test]
fn material_override_applies_per_plotter() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "materials",
        "my-card",
        "id = \"my-card\"\nname = \"My Card\"\ninherits = \"cardstock\"\n\
         [overrides.silhouette-cameo-2]\nforce = 25\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let m = s.material("my-card").unwrap();
    assert_eq!(m.force, 20);
    assert_eq!(m.for_plotter("silhouette-cameo-2").force, 25);
    assert_eq!(m.for_plotter("other").force, 20);
    assert_eq!(m.speed, 3);
}

#[test]
fn save_writes_only_differences_and_round_trips() {
    let (d, mut s) = store();
    let mut p = s.plotter("silhouette-cameo-2").unwrap();
    p.id = "tuned".into();
    p.name = "Tuned".into();
    p.scale_x = 1.002;
    p.media[0].calibrated = true;
    p.knife_compensation = KnifeCompensation::None;
    s.save_user_plotter(&p, "silhouette-cameo-2").unwrap();
    assert_eq!(
        s.modified_fields("tuned"),
        vec!["knife_compensation", "media.mat-12x12", "scale_x"]
    );
    let text = fs::read_to_string(d.path().join("plotters/tuned.toml")).unwrap();
    assert!(text.contains("inherits = \"silhouette-cameo-2\""));
    assert!(!text.contains("speed_range"));
    let s2 = ProfileStore::load(d.path()).unwrap();
    let q = s2.plotter("tuned").unwrap();
    assert_eq!(q.scale_x, 1.002);
    assert!(q.media[0].calibrated);
    assert_eq!(q.knife_compensation, KnifeCompensation::None);
    assert_eq!(q.inherits.as_deref(), Some("silhouette-cameo-2"));
}

#[test]
fn save_material_and_reject_system_id() {
    let (d, mut s) = store();
    let mut m = s.material("vinyl").unwrap();
    assert!(matches!(
        s.save_user_material(&m, "cardstock"),
        Err(ProfileError::ShadowsSystem { .. })
    ));
    m.id = "my-vinyl".into();
    m.name = "My Vinyl".into();
    m.force = 12;
    s.save_user_material(&m, "vinyl").unwrap();
    assert_eq!(s.modified_fields("my-vinyl"), vec!["force"]);
    let s2 = ProfileStore::load(d.path()).unwrap();
    assert_eq!(s2.material("my-vinyl").unwrap().force, 12);
    assert_eq!(s2.materials().len(), 5);
}

#[test]
fn missing_user_dir_is_fine() {
    let d = tempdir().unwrap();
    let s = ProfileStore::load(&d.path().join("nope")).unwrap();
    assert_eq!(s.plotters().len(), 2);
}

#[test]
fn save_rejects_path_like_id() {
    let (d, mut s) = store();
    for bad in ["../x", "a/b", "A b", "x.y", "", "-a", "a--b"] {
        let mut p = s.plotter("silhouette-cameo-2").unwrap();
        p.id = bad.into();
        let err = s.save_user_plotter(&p, "silhouette-cameo-2").unwrap_err();
        assert!(matches!(err, ProfileError::Invalid { .. }), "{bad}");
    }
    assert!(!d.path().join("plotters").exists());
}

#[test]
fn load_rejects_stem_id_mismatch() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "file-name",
        "id = \"other\"\nname = \"A\"\n",
    );
    assert!(matches!(
        ProfileStore::load(d.path()),
        Err(ProfileError::Invalid { .. })
    ));
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "Bad Id",
        "id = \"Bad Id\"\nname = \"A\"\n",
    );
    assert!(matches!(
        ProfileStore::load(d.path()),
        Err(ProfileError::Invalid { .. })
    ));
}

#[test]
fn save_refuses_clearing_inherited_value() {
    let (d, mut s) = store();
    let mut m = s.material("vinyl").unwrap();
    m.id = "my-vinyl".into();
    m.blade_depth_hint = None;
    let err = s.save_user_material(&m, "vinyl").unwrap_err();
    assert!(err.to_string().contains(
        "Clearing the inherited value 'blade_depth_hint' is not supported yet. Set a value instead."
    ));
    assert!(!d.path().join("materials/my-vinyl.toml").exists());
    // a removed per-plotter override entry
    let mut a = s.material("vinyl").unwrap();
    a.id = "a-vinyl".into();
    a.overrides.insert(
        "silhouette-cameo-2".into(),
        MaterialOverride {
            force: Some(9),
            ..Default::default()
        },
    );
    s.save_user_material(&a, "vinyl").unwrap();
    let mut b = s.material("a-vinyl").unwrap();
    b.id = "b-vinyl".into();
    b.overrides.clear();
    let err = s.save_user_material(&b, "a-vinyl").unwrap_err();
    assert!(err.to_string().contains("'overrides'"));
    assert!(!d.path().join("materials/b-vinyl.toml").exists());
}

#[test]
fn save_refuses_parent_cycle() {
    let (_d, mut s) = store();
    let mut a = s.plotter("silhouette-cameo-2").unwrap();
    a.id = "a".into();
    s.save_user_plotter(&a, "silhouette-cameo-2").unwrap();
    let mut b = a.clone();
    b.id = "b".into();
    s.save_user_plotter(&b, "a").unwrap();
    let mut a2 = a.clone();
    a2.scale_x = 1.5;
    assert!(matches!(
        s.save_user_plotter(&a2, "b"),
        Err(ProfileError::Inheritance { .. })
    ));
}

#[test]
fn material_cycle_is_an_error() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "materials",
        "a",
        "id = \"a\"\nname = \"A\"\ninherits = \"b\"\n",
    );
    write(
        d.path(),
        "materials",
        "b",
        "id = \"b\"\nname = \"B\"\ninherits = \"a\"\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    assert!(matches!(
        s.material("a"),
        Err(ProfileError::Inheritance { .. })
    ));
}

fn mat(id: &str) -> Media {
    Media {
        id: id.into(),
        name: id.into(),
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
        color: DEFAULT_MAT_COLOR,
    }
}

#[test]
fn media_validate_refuses_bad_values() {
    assert!(mat("a").validate().is_ok());
    type Case = (&'static str, Box<dyn Fn(&mut Media)>);
    let cases: Vec<Case> = vec![
        ("nan width", Box::new(|m| m.width_mm = f64::NAN)),
        ("inf height", Box::new(|m| m.height_mm = f64::INFINITY)),
        ("zero width", Box::new(|m| m.width_mm = 0.0)),
        ("negative height", Box::new(|m| m.height_mm = -1.0)),
        ("nan origin", Box::new(|m| m.grid.origin_x_mm = f64::NAN)),
        ("negative origin x", Box::new(|m| m.grid.origin_x_mm = -0.1)),
        ("negative origin y", Box::new(|m| m.grid.origin_y_mm = -0.1)),
        ("zero grid width", Box::new(|m| m.grid.width_mm = 0.0)),
        ("zero grid height", Box::new(|m| m.grid.height_mm = 0.0)),
        ("inf grid", Box::new(|m| m.grid.width_mm = f64::INFINITY)),
        (
            "grid sticks out right",
            Box::new(|m| m.grid.origin_x_mm = 10.0),
        ),
        (
            "grid sticks out bottom",
            Box::new(|m| m.grid.origin_y_mm = 0.5),
        ),
    ];
    for (name, f) in cases {
        let mut m = mat("a");
        f(&mut m);
        assert!(m.validate().is_err(), "{name} should be refused");
    }
}

#[test]
fn invalid_media_is_refused_on_load_and_save() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "bad",
        "id = \"bad\"\nname = \"B\"\ninherits = \"silhouette-cameo-2\"\n\
         media = [{ id = \"m\", name = \"M\", width_mm = 100.0, height_mm = 100.0, \
         grid = { origin_x_mm = 60.0, origin_y_mm = 0.0, width_mm = 50.0, height_mm = 50.0 } }]\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    assert!(matches!(
        s.plotter("bad"),
        Err(ProfileError::Invalid { .. })
    ));

    let (_d, mut s) = store();
    let mut p = s.plotter("silhouette-cameo-2").unwrap();
    p.id = "mine".into();
    p.media[0].grid.width_mm = 0.0;
    assert!(matches!(
        s.save_user_plotter(&p, "silhouette-cameo-2"),
        Err(ProfileError::Invalid { .. })
    ));
}

#[test]
fn user_media_merge_by_id() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "mine",
        "id = \"mine\"\nname = \"M\"\ninherits = \"silhouette-cameo-2\"\n\
         media = [\n\
         { id = \"mat-12x12\", name = \"My mat\", width_mm = 304.8, height_mm = 330.2, calibrated = true, \
         grid = { origin_x_mm = 3.0, origin_y_mm = 4.0, width_mm = 300.0, height_mm = 300.0 } },\n\
         { id = \"mat-12x24\", hidden = true },\n\
         { id = \"a4\", name = \"A4\", width_mm = 210.0, height_mm = 297.0 },\n\
         ]\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let p = s.plotter("mine").unwrap();
    let ids: Vec<&str> = p.media.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["mat-12x12", "no-mat", "a4"]);
    assert_eq!(p.media[0].name, "My mat");
    assert!(p.media[0].calibrated);
    assert_eq!(p.media[1].name, "No mat");
    assert_eq!(
        s.modified_fields("mine"),
        vec!["media.a4", "media.mat-12x12", "media.mat-12x24"]
    );
    // the parent is untouched
    assert_eq!(s.plotter("silhouette-cameo-2").unwrap().media.len(), 3);
}

#[test]
fn save_writes_only_changed_and_new_mats() {
    let (d, mut s) = store();
    let mut p = s.plotter("silhouette-cameo-2").unwrap();
    p.id = "tuned".into();
    p.name = "Tuned".into();
    p.media[0].calibrated = true;
    p.media.remove(1); // hide the 12x24
    p.media.push(mat("extra"));
    s.save_user_plotter(&p, "silhouette-cameo-2").unwrap();
    assert_eq!(
        s.modified_fields("tuned"),
        vec!["media.extra", "media.mat-12x12", "media.mat-12x24"]
    );
    let text = fs::read_to_string(d.path().join("plotters/tuned.toml")).unwrap();
    assert!(!text.contains("no-mat"), "{text}");
    assert!(text.contains("hidden = true"), "{text}");
    let s2 = ProfileStore::load(d.path()).unwrap();
    let q = s2.plotter("tuned").unwrap();
    let ids: Vec<&str> = q.media.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["mat-12x12", "no-mat", "extra"]);
    assert!(q.media[0].calibrated);
}

#[test]
fn mat_color_defaults_roundtrips_and_validates() {
    let (_d, s) = store();
    let p = s.plotter("silhouette-cameo-2").unwrap();
    assert_eq!(p.media[0].color, [0x8F, 0xA8, 0xB8]);
    assert_eq!(p.media[2].color, [0xF4, 0xF1, 0xEA]);

    // old files without color get the default of their kind
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "oldmedia",
        "id = \"oldmedia\"\nname = \"O\"\ninherits = \"silhouette-cameo-2\"\n\
         media = [{ id = \"a\", name = \"A\", width_mm = 10.0, height_mm = 10.0 },\n\
         { id = \"b\", name = \"B\", kind = \"NoMat\", width_mm = 10.0, height_mm = 10.0 }]\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let o = s.plotter("oldmedia").unwrap();
    let get = |id: &str| o.media.iter().find(|m| m.id == id).unwrap().color;
    assert_eq!(get("a"), DEFAULT_MAT_COLOR);
    assert_eq!(get("b"), DEFAULT_NO_MAT_COLOR);

    // save writes a changed colour as #RRGGBB and it comes back
    let (d, mut s) = store();
    let mut p = s.plotter("silhouette-cameo-2").unwrap();
    p.id = "tuned".into();
    p.media[0].color = [0x12, 0xAB, 0xEF];
    s.save_user_plotter(&p, "silhouette-cameo-2").unwrap();
    let text = fs::read_to_string(d.path().join("plotters/tuned.toml")).unwrap();
    assert!(text.contains("#12ABEF"), "{text}");
    let s2 = ProfileStore::load(d.path()).unwrap();
    assert_eq!(
        s2.plotter("tuned").unwrap().media[0].color,
        [0x12, 0xAB, 0xEF]
    );
}

#[test]
fn invalid_mat_color_is_refused() {
    for bad in ["red", "#12345", "#GGGGGG", "123456"] {
        let d = tempdir().unwrap();
        write(
            d.path(),
            "plotters",
            "bad",
            &format!(
                "id = \"bad\"\nname = \"B\"\ninherits = \"silhouette-cameo-2\"\n\
                 media = [{{ id = \"a\", name = \"A\", width_mm = 10.0, height_mm = 10.0, color = \"{bad}\" }}]\n"
            ),
        );
        let s = ProfileStore::load(d.path()).unwrap();
        assert!(
            matches!(s.plotter("bad"), Err(ProfileError::Invalid { .. })),
            "{bad}"
        );
    }
}

#[test]
fn duplicate_mat_ids_are_refused() {
    let (_d, mut s) = store();
    let mut p = s.plotter("silhouette-cameo-2").unwrap();
    p.id = "dup".into();
    p.media.push(p.media[0].clone());
    let err = s.save_user_plotter(&p, "silhouette-cameo-2").unwrap_err();
    assert!(err.to_string().contains("mat-12x12"), "{err}");
    assert!(matches!(err, ProfileError::Invalid { .. }));
}

#[test]
fn mat_sizes_have_an_upper_bound() {
    let mut m = mat("a");
    m.width_mm = 10_001.0;
    m.grid.width_mm = 10_001.0;
    assert!(m.validate().is_err());
    let mut m = mat("a");
    m.width_mm = 10_000.0;
    m.grid.width_mm = 10_000.0;
    assert!(m.validate().is_ok());
}

#[test]
fn hiding_a_mat_that_does_not_exist_is_a_clear_error() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "ghost",
        "id = \"ghost\"\nname = \"G\"\ninherits = \"silhouette-cameo-2\"\n\
         media = [{ id = \"nope\", hidden = true }]\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let err = s.plotter("ghost").unwrap_err();
    assert!(matches!(err, ProfileError::Invalid { .. }), "{err:?}");
    assert!(
        err.to_string()
            .contains("Mat 'nope' is hidden, but there is no built-in mat with that id to hide."),
        "{err}"
    );
}

#[test]
fn duplicate_ids_in_a_user_media_list_are_reported_on_load() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "twice",
        "id = \"twice\"\nname = \"T\"\ninherits = \"silhouette-cameo-2\"\n\
         media = [{ id = \"a\", name = \"A\", width_mm = 10.0, height_mm = 10.0 },\n\
         { id = \"a\", name = \"A2\", width_mm = 20.0, height_mm = 20.0 }]\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let err = s.plotter("twice").unwrap_err();
    assert!(matches!(err, ProfileError::Invalid { .. }), "{err:?}");
    assert!(err.to_string().contains("'a'"), "{err}");
}

#[test]
fn hidden_mat_without_a_parent_is_a_clear_error() {
    let d = tempdir().unwrap();
    write(
        d.path(),
        "plotters",
        "orphan",
        "id = \"orphan\"\nname = \"O\"\nvendor = \"v\"\nmodel = \"m\"\n\
         protocol = { Gpgl = { dialect = \"Cameo12\" } }\n\
         transport = { File = { extension = \"gpgl\" } }\n\
         media = [{ id = \"nope\", hidden = true }]\n",
    );
    let s = ProfileStore::load(d.path()).unwrap();
    let err = s.plotter("orphan").unwrap_err();
    assert!(
        err.to_string()
            .contains("Mat 'nope' is hidden, but there is no built-in mat with that id to hide."),
        "{err}"
    );
}
