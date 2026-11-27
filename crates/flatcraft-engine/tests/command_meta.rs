//! The parameter metadata of the tool commands: one short plain label and
//! a unit per parameter (`commands::meta`), with the type, the default
//! and the range read from the command's schema.

use flatcraft_engine::commands::meta::{self, ParamKind, Unit};
use flatcraft_engine::registry;

#[test]
fn every_tool_param_has_label_unit_and_range() {
    assert!(!meta::PARAMS.is_empty());
    for m in meta::PARAMS {
        let what = format!("{} {}", m.command, m.path);
        assert!(registry::get(m.command).is_some(), "{what}: no command");
        assert!(!m.label.is_empty(), "{what}: no label");
        assert!(!m.label.contains('\u{2014}'), "{what}: an em-dash");
        assert!(m.label.len() <= 24, "{what}: a short label");
        let info = meta::param_info(m.command, m.path)
            .unwrap_or_else(|| panic!("{what}: not in the schema"));
        assert!(!info.description.is_empty(), "{what}: no doc comment");
        match &info.kind {
            ParamKind::Number | ParamKind::Integer => {
                assert!(
                    info.min.is_some() || info.max.is_some(),
                    "{what}: no range in the schema"
                );
                assert_ne!(m.unit, Unit::None, "{what}: a number has a unit");
            }
            ParamKind::Bool => assert_eq!(m.unit, Unit::None, "{what}"),
            ParamKind::Enum(values) => {
                assert_eq!(m.unit, Unit::None, "{what}");
                // Every value has a plain label, and only those.
                let labelled: Vec<&str> = m.choices.iter().map(|c| c.0).collect();
                assert_eq!(&labelled, values, "{what}");
                for (_, label) in m.choices {
                    assert!(!label.is_empty() && !label.contains('\u{2014}'), "{what}");
                }
            }
        }
    }
}

#[test]
fn ranges_match_what_the_commands_accept() {
    let range = |c: &str, p: &str| {
        let i = meta::param_info(c, p).unwrap();
        (i.min, i.max)
    };
    let both = |lo: f64, hi: f64| (Some(lo), Some(hi));
    assert_eq!(range("object.copies", "count"), both(1.0, 500.0));
    assert_eq!(range("object.duplicate", "count"), both(1.0, 500.0));
    assert_eq!(range("object.circular_copies", "count"), both(2.0, 501.0));
    assert_eq!(
        range("object.circular_copies", "total_deg"),
        both(-360.0, 360.0)
    );
    assert_eq!(range("object.rotate", "degrees"), both(-360.0, 360.0));
    assert_eq!(
        range("object.copies", "layout.grid.columns"),
        both(1.0, 501.0)
    );
    assert_eq!(range("object.copies", "gap_x_mm"), both(0.0, 1000.0));
    assert_eq!(range("object.copies", "gap_y_mm"), both(0.0, 1000.0));
    assert_eq!(range("arrange.auto", "spacing_mm"), both(0.0, 100.0));
    assert_eq!(range("object.move", "dx_mm"), both(-10000.0, 10000.0));
    assert_eq!(
        range("object.duplicate", "offset_mm[1]"),
        both(-10000.0, 10000.0)
    );
    // Absolute places: the session's 1 km limit.
    assert_eq!(
        range("object.circular_copies", "center_mm[1]"),
        both(-1e6, 1e6)
    );
    assert_eq!(range("object.move", "x_mm"), both(-1e6, 1e6));
    assert_eq!(range("object.move", "y_mm"), both(-1e6, 1e6));
    assert_eq!(range("object.move", "center_mm[0]"), both(-1e6, 1e6));
}

/// A field starts with the schema's default; where the command has none,
/// with the table's start value, which the doc comment states for agents.
/// A field that is the same as another command's parameter takes that
/// one's start value.
#[test]
fn every_field_has_one_start_value() {
    for m in meta::PARAMS {
        let what = format!("{} {}", m.command, m.path);
        let info = meta::param_info(m.command, m.path).unwrap();
        let schema = info.default.as_ref().filter(|d| !d.is_null());
        if let Some((c, p)) = m.same_as {
            assert!(
                schema.is_none() && m.start.is_none(),
                "{what}: its own start"
            );
            let other = meta::meta(c, p).unwrap_or_else(|| panic!("{what}: {c} {p}"));
            assert_eq!(other.label, m.label, "{what}: one field, one label");
            assert_eq!(other.unit, m.unit, "{what}");
            assert_eq!(
                meta::param_info(c, p).unwrap().min,
                info.min,
                "{what}: one range"
            );
            assert_eq!(meta::param_info(c, p).unwrap().max, info.max);
            assert_eq!(
                meta::start_value(m.command, m.path),
                meta::start_value(c, p)
            );
            continue;
        }
        match (schema, m.start) {
            (Some(_), Some(_)) => panic!("{what}: two start values"),
            (Some(_), None) => {}
            (None, Some(n)) => assert!(
                info.description
                    .contains(&format!("The window starts with {n}.")),
                "{what}: the doc comment does not say it starts with {n}"
            ),
            // The centre of a circle starts at the selection's, Turn about
            // with the window's Each object switch.
            (None, None) => assert!(
                m.path.starts_with("center_mm") || m.path == "about",
                "{what}: no start"
            ),
        }
    }
    assert_eq!(
        meta::start_value("object.move", "dx_mm"),
        Some(serde_json::json!(10.0))
    );
    assert_eq!(
        meta::start_value("object.move", "dy_mm"),
        meta::start_value("object.duplicate", "offset_mm[1]")
    );
}

/// The registry refuses a value outside the schema's range before the
/// command runs, with the parameter's name and its range.
#[test]
fn the_registry_refuses_values_outside_the_schema_range() {
    let dir = tempfile::tempdir().unwrap();
    let profiles = flatcraft_device::ProfileStore::load(&dir.path().join("profiles")).unwrap();
    let mut s = flatcraft_engine::Session::new(
        profiles,
        flatcraft_engine::Preferences::default(),
        true,
        dir.path().to_path_buf(),
    );
    for (cmd, params, says) in [
        (
            "arrange.auto",
            serde_json::json!({ "spacing_mm": 101.0 }),
            "spacing_mm must be from 0 to 100; it was 101",
        ),
        (
            "object.copies",
            serde_json::json!({ "id": 1, "count": 2, "layout": { "grid": { "columns": 0 } } }),
            "layout.grid.columns must be from 1 to 501; it was 0",
        ),
        (
            "object.duplicate",
            serde_json::json!({ "offset_mm": [0.0, 20000.0] }),
            "offset_mm[1] must be from -10000 to 10000; it was 20000",
        ),
        (
            "object.rotate",
            serde_json::json!({ "degrees": 720.0 }),
            "degrees must be from -360 to 360; it was 720",
        ),
        (
            "object.copies",
            serde_json::json!({ "id": 1, "count": 1, "gap_x_mm": -1.0 }),
            "gap_x_mm must be from 0 to 1000; it was -1",
        ),
    ] {
        let e = s
            .execute(cmd, params.clone(), flatcraft_engine::Origin::Agent)
            .unwrap_err();
        assert!(
            matches!(e, flatcraft_engine::CommandError::InvalidParams(_)),
            "{cmd} {params}: {e}"
        );
        assert!(e.to_string().contains(says), "{cmd}: {e}");
    }
    // In range passes the check (and fails later for its own reason).
    let schema = meta::schema_of("arrange.auto").unwrap();
    assert!(meta::out_of_range(&schema, &serde_json::json!({ "spacing_mm": 100.0 })).is_none());
}

#[test]
fn fields_come_in_card_order() {
    let paths: Vec<&str> = meta::fields_of("object.copies").map(|m| m.path).collect();
    assert_eq!(
        paths,
        [
            "layout.grid.columns",
            "count",
            "gap_x_mm",
            "gap_y_mm",
            "linked"
        ]
    );
}

#[test]
fn types_and_defaults_come_from_the_schema() {
    let i = meta::param_info("arrange.auto", "spacing_mm").unwrap();
    assert_eq!(i.kind, ParamKind::Number);
    assert_eq!(i.default, Some(serde_json::json!(5.0)));
    let i = meta::param_info("object.copies", "layout.grid.columns").unwrap();
    assert_eq!(i.kind, ParamKind::Integer);
    let i = meta::param_info("object.circular_copies", "rotate_copies").unwrap();
    assert_eq!(i.kind, ParamKind::Bool);
    assert_eq!(i.default, Some(serde_json::json!(true)));
    let i = meta::param_info("object.rotate", "about").unwrap();
    assert_eq!(i.kind, ParamKind::Enum(vec!["group".into(), "each".into()]));
    let i = meta::param_info("object.duplicate", "offset_mm[0]").unwrap();
    assert_eq!(i.kind, ParamKind::Number);
    assert_eq!(i.default, Some(serde_json::json!(10.0)));
    assert!(meta::param_info("object.rotate", "nothing").is_none());
    assert!(meta::param_info("no.such", "count").is_none());
}

#[test]
fn the_label_and_unit_of_a_param() {
    let m = meta::meta("object.circular_copies", "total_deg").unwrap();
    assert_eq!((m.label, m.unit), ("Total angle", Unit::Deg));
    assert_eq!(Unit::Deg.symbol(), "deg");
    assert_eq!(Unit::Mm.symbol(), "mm");
    assert_eq!(Unit::Count.symbol(), "");
    assert!(meta::meta("object.rotate", "nothing").is_none());
}
