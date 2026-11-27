//! The design rules of `docs/design/ui-rules.md` on the side panels and
//! the workspace tile: one row height and one padding in both panels,
//! value widgets that fill their column, no reserved empty slot, no
//! selection block on the left, and the tile's foot on the footers' line.

mod common;

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;
use flatcraft::ui::metrics;
use flatcraft_engine::prefs::PanelUser;
use flatcraft_engine::session::Origin;
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

fn run(s: &SharedSession, id: &str, params: Value) -> Value {
    s.lock().unwrap().execute(id, params, Origin::User).unwrap()
}

/// A Prepare window with both panels open and the sections `open` open;
/// `setup` runs on the session (one square is imported first).
fn window(
    dir: &tempfile::TempDir,
    open: &[&str],
    setup: impl FnOnce(&SharedSession, u64),
) -> AppHarness {
    let s = session(dir);
    {
        let mut st = s.lock().unwrap();
        st.prefs.panel_settings = PanelUser::Open;
        st.prefs.panel_objects = PanelUser::Open;
        for id in open {
            st.prefs.panel_sections.insert((*id).into(), true);
        }
    }
    let a = import_square(&s);
    setup(&s, a);
    let mut h = harness_sized(dir, s, [1400.0, 1200.0], 8, |a| a);
    to_prepare(&mut h);
    wait_for_workers(&mut h);
    h
}

fn rect(h: &AppHarness, label: &str) -> egui::Rect {
    let c = app(h).ui.prepare.canvas_rect;
    let found: Vec<_> = h
        .query_all_by_label(label)
        .filter(|n| n.rect().left() >= c.right())
        .map(|n| n.rect())
        .collect();
    assert_eq!(found.len(), 1, "{label} in the inspector");
    found[0]
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

/// The rows of one section: each control is `CONTROL` tall, centred in
/// its row, and the rows follow each other at the `ROW` pitch.
fn assert_rows(h: &AppHarness, labels: &[&str]) {
    let rects: Vec<_> = labels.iter().map(|l| rect(h, l)).collect();
    for (r, l) in rects.iter().zip(labels) {
        assert!(near(r.height(), metrics::CONTROL), "{l}: {r:?}");
    }
    for w in rects.windows(2) {
        assert!(near(w[1].top() - w[0].top(), metrics::ROW), "{w:?}");
    }
}

#[test]
fn inspector_rows_share_one_height() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(&dir, &["object.position"], |_, _| {});
    assert_rows(&h, &["Cut mode", "Keep", "Strokes"]);
    assert_rows(&h, &["X", "Y", "Width", "Height", "Scale", "Angle"]);
    // Headers are one height too.
    for l in ["Cut", "Position and size", "Hints"] {
        assert!(near(rect(&h, l).height(), metrics::SECTION_HEADER), "{l}");
    }
    // The Job panel's rows, through the same blocks.
    run_select(&mut h, &[]);
    assert_rows(&h, &["Plotter", "Mat", "Material"]);
    assert_rows(&h, &["Speed", "Force", "Passes"]);
}

fn run_select(h: &mut AppHarness, ids: &[u64]) {
    let s = app(h).session.clone();
    run(&s, "object.select", json!({ "ids": ids }));
    h.run_plan();
    wait_for_workers(h);
}

#[test]
fn value_widgets_fill_the_value_column() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(&dir, &["object.position"], |_, _| {});
    // The column ends before the reset slot, at the same x in every row.
    let header = rect(&h, "Cut");
    let end = header.right() - metrics::CONTROL - metrics::SPACE_4;
    let check = |h: &AppHarness, full: &[&str]| {
        let left = rect(h, full[0]).left();
        for l in full {
            let r = rect(h, l);
            assert!(near(r.left(), left), "{l} starts with the column: {r:?}");
            assert!(near(r.right(), end), "{l} fills the column: {r:?} {end}");
        }
    };
    check(&h, &["Cut mode", "Strokes"]);
    // Keep shares its row with the pick button, which ends the column.
    let keep = rect(&h, "Keep");
    let pick = rect(&h, "Pick on the mat...");
    assert!(near(keep.left(), rect(&h, "Cut mode").left()));
    assert!(near(pick.right(), end), "{pick:?}");
    assert!(near(pick.left() - keep.right(), metrics::SPACE_4));
    check(&h, &["X", "Y", "Width", "Height", "Scale", "Angle"]);
    run_select(&mut h, &[]);
    check(&h, &["Plotter", "Mat", "Material"]);
    check(&h, &["Speed", "Force", "Passes"]);
    check(&h, &["Strategy"]);
    // The label column fits the section's longest label.
    let col = rect(&h, "Plotter").left() - header.left() - metrics::DOT_SLOT;
    assert!(
        (metrics::LABEL_MIN..=metrics::LABEL_MAX).contains(&col),
        "{col}"
    );
}

#[test]
fn no_reserved_empty_slot_in_cut() {
    let dir = tempfile::tempdir().unwrap();
    let mut copy = 0;
    let mut h = window(&dir, &[], |s, a| {
        let d = run(s, "object.duplicate", json!({ "ids": [a] }));
        copy = d["ids"][0].as_u64().or_else(|| d["id"].as_u64()).unwrap();
        run(s, "object.link", json!({ "ids": [a, copy] }));
        run(s, "object.select", json!({ "ids": [a] }));
    });
    // The next header follows the last row of Cut at once, also for a
    // linked copy (its note is in the heading and the card).
    let gap = |h: &AppHarness| rect(h, "Position and size").top() - rect(h, "Strokes").bottom();
    let linked = gap(&h);
    assert!(
        near(linked, (metrics::ROW - metrics::CONTROL) / 2.0),
        "{linked}"
    );
    let s = app(&h).session.clone();
    let lone = import_square(&s);
    run_select(&mut h, &[lone]);
    assert!(near(gap(&h), linked));
}

#[test]
fn panels_share_row_height_and_padding() {
    let dir = tempfile::tempdir().unwrap();
    let h = window(&dir, &[], |s, a| {
        let b = import_square(s);
        run(s, "object.rename", json!({ "id": a, "name": "alpha" }));
        run(s, "object.rename", json!({ "id": b, "name": "beta" }));
        run(s, "object.select", json!({ "ids": [a] }));
    });
    let c = app(&h).ui.prepare.canvas_rect;
    // Objects rows are panel rows.
    let row = h.get_by_role_and_label(Role::Button, "beta").rect();
    assert!(near(row.height(), metrics::ROW), "{row:?}");
    let alpha = h.get_by_role_and_label(Role::Button, "alpha").rect();
    assert!(near(row.top() - alpha.top(), metrics::ROW));
    // The same padding on the canvas side of both panels.
    let left_pad = c.left() - row.right();
    let right_pad = rect(&h, "Cut").left() - c.right();
    assert!(near(left_pad, right_pad), "{left_pad} {right_pad}");
    assert!(
        (metrics::PANEL_PAD_X..=metrics::PANEL_PAD_X + 1.0).contains(&left_pad),
        "{left_pad}"
    );
    // The panels' headers are a row high, with the same compact button.
    let objects = h
        .get_by_role_and_label(Role::Button, "Collapse the Objects panel")
        .rect();
    let job = h
        .get_by_role_and_label(Role::Button, "Collapse the Cut and Job panel")
        .rect();
    assert!(near(objects.height(), metrics::ICON_BUTTON), "{objects:?}");
    let line = h.get_by_label_contains("Job: ").rect();
    assert!(near(line.height(), metrics::ROW), "{line:?}");
    // The job line and its collapse button are one element: side by side,
    // one row high.
    assert!(near(job.top(), line.top()) && near(job.bottom(), line.bottom()));
    assert!(near(job.left(), line.right()), "{job:?} {line:?}");
}

#[test]
fn left_panel_has_no_selection_info_block() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = window(&dir, &[], |s, a| {
        run(s, "object.select", json!({ "ids": [a] }));
    });
    let c = app(&h).ui.prepare.canvas_rect;
    let left_texts = |h: &AppHarness| -> Vec<String> {
        painted(h)
            .into_iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) if t.pos.x < c.left() => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    };
    let texts = left_texts(&h);
    for gone in ["Not linked to a file", "Linked file", "50.0 x 50.0 mm"] {
        assert!(!texts.iter().any(|t| t.contains(gone)), "{gone}: {texts:?}");
    }
    // A group: no "Group of 2 objects" block either.
    let s = app(&h).session.clone();
    let b = import_square(&s);
    run(&s, "object.group", json!({ "ids": [1, b] }));
    run_select(&mut h, &[1, b]);
    let texts = left_texts(&h);
    assert!(
        !texts.iter().any(|t| t.starts_with("Group of")),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("It moves, turns")),
        "{texts:?}"
    );
    // The Notes stay.
    h.get_by_role_and_label(Role::Button, "Notes");
}

/// The top of the first glyph of the painted text `text` and its size.
fn glyph_top(h: &AppHarness, text: &str) -> Option<(f32, f32)> {
    painted(h).into_iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text => {
            let row = &t.galley.rows[0];
            let g = row.row.glyphs.first()?;
            Some((t.pos.y + row.pos.y + g.pos.y, g.font_face_height))
        }
        _ => None,
    })
}

#[test]
fn workspace_tile_foot_sits_on_the_footer_line() {
    for w in [900.0f32, 1400.0] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        import_square(&s);
        s.lock()
            .unwrap()
            .execute("object.select", json!({ "ids": [] }), Origin::User)
            .unwrap();
        let mut h = harness_sized(&dir, s, [w, 900.0], 8, |a| a);
        to_prepare(&mut h);
        settle(&mut h);
        let tile = glyph_top(&h, "Workspace").expect("the tile's foot");
        let footer = glyph_top(&h, "Add").expect("a group footer");
        assert!(
            near(tile.0, footer.0),
            "Prepare at {w}: {tile:?} {footer:?}"
        );
        assert!(near(tile.1, footer.1), "same size at {w}");
        app_mut(&mut h).ui.workspace = Workspace::Cut;
        settle(&mut h);
        let tile = glyph_top(&h, "Workspace").expect("the tile's foot");
        let detail = painted(&h)
            .into_iter()
            .find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text().contains("pass") => {
                    Some(t.galley.text().to_string())
                }
                _ => None,
            })
            .expect("the Path step's status line");
        let step = glyph_top(&h, &detail).unwrap();
        assert!(near(tile.0, step.0), "Cut at {w}: {tile:?} {step:?}");
        assert!(near(tile.1, step.1), "same size at {w}");
    }
}
