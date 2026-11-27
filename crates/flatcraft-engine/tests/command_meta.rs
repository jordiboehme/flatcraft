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
    assert_eq!(range("object.copies", "count"), (Some(1.0), Some(500.0)));
    assert_eq!(range("object.duplicate", "count"), (Some(1.0), Some(500.0)));
    assert_eq!(
        range("object.circular_copies", "count"),
        (Some(2.0), Some(501.0))
    );
    assert_eq!(
        range("object.circular_copies", "total_deg"),
        (Some(-360.0), Some(360.0))
    );
    assert_eq!(range("object.rotate", "degrees"), (Some(-1e9), Some(1e9)));
    assert_eq!(
        range("object.copies", "layout.grid.columns"),
        (Some(1.0), None)
    );
    assert_eq!(range("object.copies", "gap_x_mm"), (Some(0.0), None));
    assert_eq!(range("arrange.auto", "spacing_mm"), (Some(0.0), None));
    assert_eq!(
        range("object.circular_copies", "center_mm[1]"),
        (Some(-1e6), Some(1e6))
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
