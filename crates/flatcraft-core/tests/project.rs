use flatcraft_core::document::{Document, JobSettings, Source, TraceSettings};
use flatcraft_core::geometry::{CutPath, Paint, bounds};
use flatcraft_core::project::{self, Archive, ProjectError};
use flatcraft_core::undo::UndoStack;
use kurbo::{BezPath, Point, Rect};

fn square(s: f64) -> CutPath {
    let mut path = BezPath::new();
    path.move_to(Point::new(0.0, 0.0));
    path.line_to(Point::new(s, 0.0));
    path.line_to(Point::new(s, s));
    path.line_to(Point::new(0.0, s));
    path.close_path();
    CutPath {
        path,
        color: None,
        label: Some("sq".into()),
        paint: Paint::default(),
    }
}

fn job() -> JobSettings {
    JobSettings {
        plotter_id: "cameo2".into(),
        material_id: "cardstock".into(),
        media_id: "a4".into(),
        speed: Some(5),
        force: None,
        passes: Some(2),
        strategy: Default::default(),
        weed: Default::default(),
    }
}

#[test]
fn roundtrip_keeps_sources_and_transforms() {
    let mut doc = Document::new(job());
    let a = doc.add(
        "one".into(),
        Source::Svg {
            name: "one.svg".into(),
            bytes: b"<svg/>".to_vec().into(),
        },
        vec![square(10.0)],
    );
    let b = doc.add(
        "two".into(),
        Source::Raster {
            name: "two.png".into(),
            bytes: vec![0, 1, 2, 255].into(),
            trace: TraceSettings {
                threshold: Some(100),
                ..TraceSettings::default()
            },
        },
        vec![],
    );
    doc.get_mut(a).unwrap().transform.rotation_deg = 33.0;
    doc.get_mut(b).unwrap().transform.center_mm = Point::new(12.0, 34.0);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    assert!(!dir.path().join("p.flatcraft.tmp").exists());
    let loaded = project::load(&path).unwrap();

    assert_eq!(loaded.job, doc.job);
    assert_eq!(loaded.objects.len(), 2);
    for (x, y) in loaded.objects.iter().zip(&doc.objects) {
        assert_eq!(x.source, y.source);
        assert_eq!(x.transform, y.transform);
        assert_eq!(x.name, y.name);
        assert!(x.geometry.is_empty());
    }
    assert_eq!(loaded.next_id, doc.next_id);
}

/// The cross-object cut modes are part of format version 1: they are
/// written by name and read back, and the file stays version 1.
#[test]
fn the_cross_object_cut_modes_round_trip_in_version_1() {
    use flatcraft_core::document::CutMode;
    let mut doc = Document::new(job());
    let modes = [
        CutMode::AllLines,
        CutMode::ObjectOutlines,
        CutMode::MergedOutline,
        CutMode::VisibleOutline,
    ];
    let ids: Vec<_> = modes
        .iter()
        .map(|m| {
            let id = doc.add(
                m.key().into(),
                Source::Svg {
                    name: "one.svg".into(),
                    bytes: b"<svg/>".to_vec().into(),
                },
                vec![square(10.0)],
            );
            doc.get_mut(id).unwrap().cut.mode = *m;
            id
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    let text = project_text(&path);
    assert!(text.contains("\"merged_outline\"") && text.contains("\"visible_outline\""));
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["format_version"], 1);
    let loaded = project::load(&path).unwrap();
    assert_eq!(loaded.format_version, 1);
    for (id, m) in ids.iter().zip(modes) {
        assert_eq!(loaded.get(*id).unwrap().cut.mode, m);
    }
}

#[test]
fn newer_format_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    write_project(&path, r#"{"format_version": 2, "objects": []}"#);
    match project::load(&path) {
        Err(e @ ProjectError::NewerFormat { found, supported }) => {
            assert_eq!((found, supported), (2, 1));
            assert!(e.to_string().contains("newer FlatCraft"), "{e}");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn unknown_fields_are_tolerated() {
    let mut doc = Document::new(job());
    doc.add(
        "x".into(),
        Source::Svg {
            name: "x.svg".into(),
            bytes: Vec::new().into(),
        },
        vec![],
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    let mut archive = Archive::read(&path).unwrap();
    archive.project["future_field"] = serde_json::json!(1);
    archive.write(&path).unwrap();
    assert_eq!(project::load(&path).unwrap().objects.len(), 1);
}

#[test]
fn undo_redo_cycle() {
    let mut s = UndoStack::new(100);
    s.record("a");
    s.record("b");
    assert_eq!(s.undo("c"), Some("b"));
    assert_eq!(s.undo("b"), Some("a"));
    assert_eq!(s.undo("a"), None);
    assert_eq!(s.redo("a"), Some("b"));
    s.record("x");
    assert!(!s.can_redo());
    assert!(s.can_undo());
}

#[test]
fn undo_len_counts_steps() {
    let mut s = UndoStack::new(2);
    assert_eq!(s.undo_len(), 0);
    s.record(1);
    s.record(2);
    s.record(3);
    assert_eq!(s.undo_len(), 2);
    s.undo(4);
    assert_eq!(s.undo_len(), 1);
}

#[test]
fn undo_limit_drops_oldest() {
    let mut s = UndoStack::new(2);
    s.record(1);
    s.record(2);
    s.record(3);
    assert_eq!(s.undo(4), Some(3));
    assert_eq!(s.undo(3), Some(2));
    assert_eq!(s.undo(2), None);
}

#[test]
fn placed_paths_apply_transform() {
    let mut doc = Document::new(job());
    let id = doc.add(
        "sq".into(),
        Source::Svg {
            name: "sq.svg".into(),
            bytes: Vec::new().into(),
        },
        vec![square(10.0)],
    );
    let obj = doc.get_mut(id).unwrap();
    obj.transform.center_mm = Point::new(50.0, 50.0);
    let placed = obj.placed_paths();
    assert_eq!(bounds(&placed), Some(Rect::new(45.0, 45.0, 55.0, 55.0)));
    assert_eq!(placed[0].label.as_deref(), Some("sq"));
}

#[test]
fn load_clamps_next_id() {
    let mut doc = Document::new(job());
    let svg = || Source::Svg {
        name: "x.svg".into(),
        bytes: Vec::new().into(),
    };
    doc.add("a".into(), svg(), vec![]);
    doc.add("b".into(), svg(), vec![]);
    doc.objects[0].id = flatcraft_core::document::ObjectId(5);
    doc.objects[1].id = flatcraft_core::document::ObjectId(7);
    doc.next_id = 1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    let mut loaded = project::load(&path).unwrap();
    assert_eq!(loaded.format_version, project::FORMAT_VERSION);
    assert_eq!(
        loaded.add("c".into(), svg(), vec![]),
        flatcraft_core::document::ObjectId(8)
    );
}

#[test]
fn ids_stay_unique_after_remove() {
    let mut doc = Document::new(job());
    let svg = || Source::Svg {
        name: "x.svg".into(),
        bytes: Vec::new().into(),
    };
    doc.add("a".into(), svg(), vec![]);
    let second = doc.add("b".into(), svg(), vec![]);
    doc.remove(second);
    let third = doc.add("c".into(), svg(), vec![]);
    assert_eq!(third, flatcraft_core::document::ObjectId(3));
}

#[test]
fn old_project_without_cut_options_loads_defaults() {
    // A format 1 project written before cut options existed.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.flatcraft");
    write_project(&path, &project_with(OLD_OBJECT));
    let doc = project::load(&path).unwrap();
    let cut = doc.objects[0].cut;
    assert_eq!(cut, flatcraft_core::document::CutOptions::default());
    assert_eq!(cut.mode, flatcraft_core::document::CutMode::ObjectOutlines);
    assert_eq!(cut.strokes, flatcraft_core::document::StrokeMode::Auto);
    assert_eq!(
        cut.auto_threshold_mm,
        flatcraft_core::document::DEFAULT_AUTO_THRESHOLD_MM
    );
}

#[test]
fn old_project_without_strategy_loads_quality_and_strategy_roundtrips() {
    use flatcraft_core::document::CutStrategy;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.flatcraft");
    write_project(&path, &project_with(OLD_OBJECT));
    let mut doc = project::load(&path).unwrap();
    assert_eq!(doc.job.strategy, CutStrategy::Quality);
    doc.job.strategy = CutStrategy::OutlinesLast;
    project::save(&doc, &path).unwrap();
    assert!(project_text(&path).contains(r#""outlines_last""#));
    assert_eq!(
        project::load(&path).unwrap().job.strategy,
        CutStrategy::OutlinesLast
    );
}

// ---------------------------------------------------------------- source links

const OLD_OBJECT: &str = r#"{"id":1,"name":"x","source":{"Svg":{"name":"x.svg","file":"EMPTY_SVG"}},"transform":{"center_mm":{"x":10.0,"y":20.0},"scale_x":1.0,"scale_y":1.0,"rotation_deg":0.0,"mirror":false},"visible":true}"#;

/// The file name of an empty SVG source in an archive.
fn empty_svg() -> String {
    project::source_file(&Source::Svg {
        name: "x.svg".into(),
        bytes: Vec::new().into(),
    })
}

/// Writes `json` (with `EMPTY_SVG` standing for [`empty_svg`]) as the
/// project.json of an archive that holds an empty SVG source.
fn write_project(path: &std::path::Path, json: &str) {
    let name = empty_svg();
    let project = serde_json::from_str(&json.replace("EMPTY_SVG", &name)).unwrap();
    let mut sources = std::collections::BTreeMap::new();
    sources.insert(name, std::sync::Arc::<[u8]>::from(Vec::new()));
    Archive {
        project,
        sources,
        thumbnail: None,
    }
    .write(path)
    .unwrap();
}

/// The project.json text of a saved project.
fn project_text(path: &std::path::Path) -> String {
    serde_json::to_string_pretty(&Archive::read(path).unwrap().project).unwrap()
}

fn project_with(object: &str) -> String {
    format!(
        r#"{{"format_version":1,"objects":[{object}],"job":{{"plotter_id":"cameo2","material_id":"cardstock","media_id":"a4","speed":null,"force":null,"passes":null}},"next_id":2}}"#
    )
}

#[test]
fn old_projects_without_link_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.flatcraft");
    write_project(&path, &project_with(OLD_OBJECT));
    let doc = project::load(&path).unwrap();
    assert_eq!(doc.objects[0].link, None);
    assert_eq!(doc.format_version, 1);
    // Saving an unlinked object writes no link field.
    project::save(&doc, &path).unwrap();
    assert!(!project_text(&path).contains("\"link\""));
}

#[test]
fn link_roundtrips_and_a_broken_link_loads_as_none() {
    use flatcraft_core::document::SourceLink;
    let mut doc = Document::new(job());
    let id = doc.add(
        "one".into(),
        Source::Svg {
            name: "one".into(),
            bytes: b"<svg/>".to_vec().into(),
        },
        vec![square(10.0)],
    );
    let link = SourceLink {
        path: "/tmp/art/one.svg".into(),
        relative: Some("art/one.svg".into()),
        sha256: "ab".repeat(32),
        modified: Some("2026-11-24T10:00:00.000000000Z".into()),
        anchor_dir: None,
    };
    doc.get_mut(id).unwrap().link = Some(link.clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    assert_eq!(project::load(&path).unwrap().objects[0].link, Some(link));

    // A link of the wrong shape never stops the project from opening.
    let broken = OLD_OBJECT.replace(r#""visible":true"#, r#""visible":true,"link":{"path":7}"#);
    write_project(&path, &project_with(&broken));
    let doc = project::load(&path).unwrap();
    assert_eq!(doc.objects[0].link, None);
}

#[test]
fn anchored_geometry_keeps_the_frame_on_the_mat() {
    let mut doc = Document::new(job());
    let id = doc.add(
        "sq".into(),
        Source::Svg {
            name: "sq".into(),
            bytes: Vec::new().into(),
        },
        vec![square(10.0)],
    );
    let o = doc.get_mut(id).unwrap();
    o.frame = Rect::new(0.0, 0.0, 100.0, 100.0);
    o.transform.center_mm = Point::new(80.0, 60.0);
    o.transform.rotation_deg = 30.0;
    o.transform.scale_x = 1.5;
    o.transform.mirror = true;
    let before = o.transform.affine(o.local_bounds) * Point::new(10.0, 10.0);
    // The same square plus a new shape far away: the local bounds change.
    let mut far = square(5.0);
    far.path
        .apply_affine(kurbo::Affine::translate((90.0, 90.0)));
    o.set_geometry_anchored(vec![square(10.0), far], Rect::new(0.0, 0.0, 100.0, 100.0));
    let after = o.transform.affine(o.local_bounds) * Point::new(10.0, 10.0);
    assert!((after - before).hypot() < 1e-9, "{before:?} {after:?}");
    assert_eq!(
        (
            o.transform.scale_x,
            o.transform.rotation_deg,
            o.transform.mirror
        ),
        (1.5, 30.0, true)
    );
    assert_eq!(o.anchor_frame(), Rect::new(0.0, 0.0, 100.0, 100.0));
    o.frame = Rect::ZERO;
    assert_eq!(o.anchor_frame(), o.local_bounds);
}

#[test]
fn undo_snapshots_share_the_source_bytes() {
    let mut doc = Document::new(job());
    doc.add(
        "big".into(),
        Source::Svg {
            name: "big.svg".into(),
            bytes: vec![7u8; 4096].into(),
        },
        vec![square(10.0)],
    );
    let mut undo = UndoStack::new(10);
    undo.record(doc.clone());
    let snapshot = undo.undo(doc.clone()).unwrap();
    let bytes = |d: &Document| match &d.objects[0].source {
        Source::Svg { bytes, .. } | Source::Raster { bytes, .. } => bytes.clone(),
    };
    assert!(std::sync::Arc::ptr_eq(&bytes(&doc), &bytes(&snapshot)));
}

#[test]
fn source_bytes_round_trip_byte_identically() {
    let mut doc = Document::new(job());
    doc.add(
        "one".into(),
        Source::Raster {
            name: "one.png".into(),
            bytes: (0..=255u8).collect::<Vec<_>>().into(),
            trace: TraceSettings::default(),
        },
        vec![],
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    let first = std::fs::read(&path).unwrap();
    // The bytes are stored as their own file in the archive.
    let archive = Archive::read(&path).unwrap();
    let file = archive.project["objects"][0]["source"]["Raster"]["file"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        &archive.sources[&file][..],
        &(0..=255u8).collect::<Vec<_>>()[..]
    );
    let loaded = project::load(&path).unwrap();
    assert_eq!(loaded.objects[0].source, doc.objects[0].source);
    let again = dir.path().join("q.flatcraft");
    project::save(&loaded, &again).unwrap();
    assert_eq!(std::fs::read(&again).unwrap(), first);
}

// ---------------------------------------------------------------- skip cutting

#[test]
fn old_project_loads_as_cut_and_saves_without_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.flatcraft");
    write_project(&path, &project_with(OLD_OBJECT));
    let mut doc = project::load(&path).unwrap();
    assert!(doc.objects[0].cut_enabled);
    assert_eq!(doc.format_version, 1);
    project::save(&doc, &path).unwrap();
    let text = project_text(&path);
    assert!(!text.contains("cut_enabled"), "{text}");
    assert!(text.contains("\"format_version\": 1"), "{text}");

    // A skipped object keeps its flag through a save, and format 1 stays.
    doc.objects[0].cut_enabled = false;
    project::save(&doc, &path).unwrap();
    let text = project_text(&path);
    assert!(text.contains("\"cut_enabled\": false"), "{text}");
    assert!(text.contains("\"format_version\": 1"), "{text}");
    assert!(!project::load(&path).unwrap().objects[0].cut_enabled);
}

#[test]
fn description_and_notes_round_trip_in_the_project_file() {
    let mut doc = Document::new(job());
    doc.description = "Birthday card for Oma".into();
    doc.notes = "Use the blue mat.\nCardstock face down.".into();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    let json = Archive::read(&path).unwrap().project;
    assert_eq!(json["description"], "Birthday card for Oma");
    assert_eq!(json["notes"], "Use the blue mat.\nCardstock face down.");
    let loaded = project::load(&path).unwrap();
    assert_eq!(loaded.description, doc.description);
    assert_eq!(loaded.notes, doc.notes);
    assert_eq!(loaded.format_version, project::FORMAT_VERSION);
}

#[test]
fn empty_notes_are_not_written_and_old_files_load_without_them() {
    let doc = Document::new(job());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    let text = project_text(&path);
    assert!(!text.contains("description"), "{text}");
    assert!(!text.contains("notes"), "{text}");

    write_project(
        &path,
        r#"{"format_version":1,"objects":[],"job":{"plotter_id":"cameo2","material_id":"cardstock","media_id":"a4","speed":null,"force":null,"passes":null},"next_id":1}"#,
    );
    let loaded = project::load(&path).unwrap();
    assert_eq!(loaded.description, "");
    assert_eq!(loaded.notes, "");
}

#[test]
fn an_older_build_ignores_the_notes() {
    /// The shape of project.json before notes, as an older build reads it.
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct OldDocument {
        format_version: u32,
        objects: Vec<serde_json::Value>,
        job: JobSettings,
        next_id: u64,
    }
    let mut doc = Document::new(job());
    doc.description = "Labels".into();
    doc.notes = "Weed slowly.".into();
    let value = project::to_value(&doc).unwrap();
    let old: OldDocument = serde_json::from_value(value).unwrap();
    assert_eq!(old.format_version, 1);
    assert_eq!(old.job, doc.job);
}

#[test]
fn the_description_is_read_without_the_sources() {
    let mut doc = Document::new(job());
    doc.description = "Window star for the kitchen".into();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    project::save(&doc, &path).unwrap();
    assert_eq!(
        project::read_description(&path).unwrap(),
        "Window star for the kitchen"
    );
    project::save(&Document::new(job()), &path).unwrap();
    assert_eq!(project::read_description(&path).unwrap(), "");
    std::fs::write(&path, b"not a zip").unwrap();
    assert!(project::read_description(&path).is_err());
}
