use flatcraft_core::document::{Document, JobSettings, Source, TraceSettings};
use flatcraft_core::geometry::{CutPath, Paint, bounds};
use flatcraft_core::project::{self, ProjectError};
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
    }
}

#[test]
fn roundtrip_keeps_sources_and_transforms() {
    let mut doc = Document::new(job());
    let a = doc.add(
        "one".into(),
        Source::Svg {
            name: "one.svg".into(),
            bytes: b"<svg/>".to_vec(),
        },
        vec![square(10.0)],
    );
    let b = doc.add(
        "two".into(),
        Source::Raster {
            name: "two.png".into(),
            bytes: vec![0, 1, 2, 255],
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

#[test]
fn newer_format_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    std::fs::write(&path, r#"{"format_version": 2, "objects": []}"#).unwrap();
    match project::load(&path) {
        Err(ProjectError::NewerFormat { found, supported }) => {
            assert_eq!((found, supported), (2, 1));
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
            bytes: vec![],
        },
        vec![],
    );
    let mut v = serde_json::to_value(&doc).unwrap();
    v["future_field"] = serde_json::json!(1);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.flatcraft");
    std::fs::write(&path, serde_json::to_string(&v).unwrap()).unwrap();
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
            bytes: vec![],
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
        bytes: vec![],
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
        bytes: vec![],
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
    let text = r#"{"format_version":1,"objects":[{"id":1,"name":"x","source":{"Svg":{"name":"x.svg","bytes":""}},"transform":{"center_mm":{"x":10.0,"y":20.0},"scale_x":1.0,"scale_y":1.0,"rotation_deg":0.0,"mirror":false},"visible":true}],"job":{"plotter_id":"cameo2","material_id":"cardstock","media_id":"a4","speed":null,"force":null,"passes":null},"next_id":2}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.flatcraft");
    std::fs::write(&path, text).unwrap();
    let doc = project::load(&path).unwrap();
    let cut = doc.objects[0].cut;
    assert_eq!(cut, flatcraft_core::document::CutOptions::default());
    assert_eq!(cut.mode, flatcraft_core::document::CutMode::ObjectOutlines);
    assert_eq!(cut.strokes, flatcraft_core::document::StrokeMode::Auto);
    assert_eq!(cut.auto_threshold_mm, 0.5);
}
