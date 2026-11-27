//! Home's Start page: the New cards, the recent projects and the samples.

mod common;
use common::RunPlan;

use common::{
    FakePicker, SQUARE_SVG, app, harness_sized, harness_with, session, wait_for_long,
    wait_for_source_check,
};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::Workspace;

#[test]
fn new_project_card_starts_an_empty_project_in_prepare() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_with(&dir, s.clone(), |a| a);
    h.get_by_role_and_label(Role::Button, "New project").click();
    wait_for_source_check(&mut h);
    let st = s.lock().unwrap();
    assert_eq!(st.docs.len(), 1, "the empty start document is used");
    assert!(st.doc().document.objects.is_empty());
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
}

#[test]
fn import_card_asks_first_and_imports_into_a_new_project() {
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("star.svg");
    std::fs::write(&svg, SQUARE_SVG).unwrap();
    let s = session(&dir);
    // The first document has content, so the import gets a new one.
    s.lock().unwrap().doc_mut().dirty = true;
    let picker = FakePicker::answering([svg.clone()]);
    let requests = picker.requests.clone();
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    h.get_by_role_and_label(Role::Button, "Import design...")
        .click();
    wait_for_source_check(&mut h);
    {
        let st = s.lock().unwrap();
        assert_eq!(st.docs.len(), 2);
        assert_eq!(st.doc().document.objects.len(), 1);
        assert!(st.docs[0].document.objects.is_empty());
    }
    assert_eq!(requests.lock().unwrap()[0].title, "Import");
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);

    // Cancelled: no new project, Home stays.
    h.get_by_role_and_label(Role::Button, "Home").click();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Import design...")
        .click();
    h.run_plan();
    assert_eq!(s.lock().unwrap().docs.len(), 2);
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
}

#[test]
fn open_card_opens_the_file_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("coasters.flatcraft");
    s.lock()
        .unwrap()
        .execute(
            "project.save",
            serde_json::json!({ "path": path }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
    let picker = FakePicker::answering([path.clone()]);
    let requests = picker.requests.clone();
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    h.get_by_role_and_label(Role::Button, "Open...").click();
    wait_for_source_check(&mut h);
    assert_eq!(requests.lock().unwrap()[0].title, "Open");
    assert_eq!(
        s.lock().unwrap().doc().path.as_deref(),
        Some(path.as_path())
    );
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
}

#[test]
fn test_patterns_card_makes_the_test_sheet_in_a_new_project() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_with(&dir, s.clone(), |a| a);
    h.get_by_role_and_label(Role::Button, "Test patterns")
        .click();
    wait_for_source_check(&mut h);
    let st = s.lock().unwrap();
    assert_eq!(st.doc().document.objects.len(), 1);
    assert_eq!(st.doc().document.objects[0].name, "Test patterns");
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    assert!(app(&h).ui.toasts.is_empty(), "{:?}", app(&h).ui.toasts);
}

#[test]
fn the_new_cards_wrap_at_the_smallest_window() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness_sized(&dir, session(&dir), [900.0, 600.0], 8, |a| a);
    let new = h.get_by_role_and_label(Role::Button, "New project").rect();
    let import = h
        .get_by_role_and_label(Role::Button, "Import design...")
        .rect();
    let open = h.get_by_role_and_label(Role::Button, "Open...").rect();
    // Two columns: Open starts the second row.
    assert!((new.top() - import.top()).abs() < 0.5);
    assert!(open.top() > new.bottom());
    assert!((open.left() - new.left()).abs() < 0.5);
    assert!(import.right() <= 900.0);
}

// ---------------------------------------------------------------- recent

use common::{app_mut, painted, wait_until};
use flatcraft_engine::session::Origin;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A fixed wall clock: 2026-11-25 12:00 UTC.
const NOW: u64 = 1_795_608_000;

fn fixed_clock() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(NOW)
}

/// Saves a project with one square, outside the window, and closes it.
fn saved_project(s: &flatcraft_mcp::SharedSession, path: &Path) {
    let mut st = s.lock().unwrap();
    st.execute(
        "object.import_svg",
        json!({ "svg": SQUARE_SVG, "name": "square" }),
        Origin::User,
    )
    .unwrap();
    st.execute("project.save", json!({ "path": path }), Origin::User)
        .unwrap();
    let id = st.doc().id();
    st.execute("document.new", json!({}), Origin::User).unwrap();
    st.execute("document.close", json!({ "doc_id": id }), Origin::User)
        .unwrap();
}

fn texts(h: &common::AppHarness) -> Vec<String> {
    painted(h)
        .into_iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

/// Steps until Home's picture worker is done.
fn wait_for_pictures(h: &mut common::AppHarness) {
    wait_until(h, |a| a.ui.home.busy());
}

#[test]
fn a_recent_row_opens_its_project_and_says_when_and_where() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("coasters.flatcraft");
    saved_project(&s, &path);
    {
        let mut st = s.lock().unwrap();
        st.prefs.recent_projects = vec![path.clone()];
        st.prefs.recent_details = vec![flatcraft_engine::prefs::RecentDetail {
            path: path.clone(),
            opened_unix_s: NOW - 2 * 3600 - 60,
            workspace: "Cut".into(),
        }];
    }
    let mut h = harness_with(&dir, s.clone(), |a| a.with_wall_clock(fixed_clock));
    wait_for_pictures(&mut h);
    let t = texts(&h);
    assert!(t.contains(&"2 hours ago".to_string()), "{t:?}");
    assert!(t.contains(&"Cut".to_string()), "{t:?}");
    h.get_by_role_and_label(Role::Button, "Open coasters")
        .click();
    wait_for_source_check(&mut h);
    assert_eq!(
        s.lock().unwrap().doc().path.as_deref(),
        Some(path.as_path())
    );
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    // Opening made it the newest, opened now.
    let st = s.lock().unwrap();
    let d = st.prefs.recent_detail(&path).unwrap();
    assert_eq!(d.opened_unix_s, NOW);
}

#[test]
fn a_recent_project_open_in_a_tab_switches_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("coasters.flatcraft");
    saved_project(&s, &path);
    let picker = FakePicker::default();
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Open(path.clone())]);
    common::wait_for_long(&mut h);
    let opened = s.lock().unwrap().doc().id();
    // Go to Cut, then a new document, then Home.
    app_mut(&mut h).ui.workspace = Workspace::Cut;
    wait_for_source_check(&mut h);
    s.lock()
        .unwrap()
        .execute("document.new", json!({}), Origin::User)
        .unwrap();
    app_mut(&mut h).ui.workspace = Workspace::Home;
    wait_for_pictures(&mut h);
    assert!(texts(&h).contains(&"Open now".to_string()));
    let docs = s.lock().unwrap().docs.len();
    h.get_by_role_and_label(Role::Button, "Go to coasters")
        .click();
    wait_for_source_check(&mut h);
    assert_eq!(s.lock().unwrap().doc().id(), opened);
    assert_eq!(s.lock().unwrap().docs.len(), docs, "no second copy opened");
    // It was last in Cut, so it comes back in Cut.
    assert_eq!(app(&h).ui.workspace, Workspace::Cut);
    let st = s.lock().unwrap();
    assert_eq!(st.prefs.recent_detail(&path).unwrap().workspace, "Cut");
}

#[test]
fn a_missing_recent_project_can_be_located_or_removed() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let moved = dir.path().join("new place.flatcraft");
    saved_project(&s, &moved);
    let gone = dir.path().join("old place.flatcraft");
    let other = dir.path().join("lost.flatcraft");
    s.lock().unwrap().prefs.recent_projects = vec![gone.clone(), other.clone()];
    let picker = FakePicker::answering([moved.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    assert!(texts(&h).contains(&"File moved or deleted".to_string()));
    // Locate the first one.
    h.get_all_by_role_and_label(Role::Button, "Locate...")
        .next()
        .unwrap()
        .click();
    wait_for_source_check(&mut h);
    assert_eq!(
        s.lock().unwrap().doc().path.as_deref(),
        Some(moved.as_path())
    );
    assert_eq!(
        s.lock().unwrap().prefs.recent_projects,
        [moved.clone(), other.clone()]
    );
    // Remove the other one.
    app_mut(&mut h).ui.workspace = Workspace::Home;
    wait_for_pictures(&mut h);
    h.get_by_role_and_label(Role::Button, "Remove from list")
        .click();
    h.run_plan();
    assert_eq!(
        s.lock().unwrap().prefs.recent_projects,
        std::slice::from_ref(&moved)
    );
    // Stored, details too.
    let stored = flatcraft_engine::Preferences::load(dir.path());
    assert_eq!(stored.recent_projects, std::slice::from_ref(&moved));
    assert_eq!(stored.recent_details.len(), 1);
    assert_eq!(stored.recent_details[0].path, moved);
}

#[test]
fn right_click_removes_a_recent_project() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("coasters.flatcraft");
    saved_project(&s, &path);
    s.lock().unwrap().prefs.recent_projects = vec![path.clone()];
    let mut h = harness_with(&dir, s.clone(), |a| a);
    wait_for_pictures(&mut h);
    h.get_by_role_and_label(Role::Button, "Open coasters")
        .click_secondary();
    h.run_plan();
    h.get_by_role_and_label(Role::Button, "Remove from list")
        .click();
    h.run_plan();
    assert!(s.lock().unwrap().prefs.recent_projects.is_empty());
    assert!(path.exists(), "no file is deleted");
}

/// The picture Home shows for `path`: the version it was read at and the
/// texture size, or `None` while there is none.
fn shown(h: &common::AppHarness, path: &Path) -> Option<(String, [usize; 2])> {
    let id = flatcraft_engine::thumbnails::project_id(path);
    match app(h).ui.home.thumbs.get(&id)? {
        (v, flatcraft::ui::home::Thumb::Ready(tex)) => Some((v.clone(), tex.size())),
        _ => None,
    }
}

#[test]
fn pictures_come_from_the_project_file_and_refresh_on_save() {
    // The config dir and the project's folder are apart, so a picture
    // next to the project would show.
    let dir = tempfile::tempdir().unwrap();
    let projects = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = projects.path().join("coasters.flatcraft");
    saved_project(&s, &path);
    s.lock().unwrap().prefs.recent_projects = vec![path.clone()];
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut h = harness_with(&dir, s.clone(), move |a| {
        a.with_inline_pictures(false)
            .with_thumbnailer(move |p, f, m| {
                counted.fetch_add(1, Ordering::SeqCst);
                flatcraft::ui::home::render_project(p, f, m)
            })
    });
    wait_for_pictures(&mut h);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let v1 = flatcraft_engine::thumbnails::project_version(&path).unwrap();
    // The picture the save put into the project file, 512 px.
    let (version, size) = shown(&h, &path).expect("a picture");
    assert_eq!(version, v1);
    assert_eq!(size[0].max(size[1]), 512);
    let png = flatcraft_core::project::read_thumbnail(&path)
        .unwrap()
        .unwrap();
    // Nothing next to the project and nothing in the config dir.
    let next_to: Vec<_> = std::fs::read_dir(projects.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(next_to, ["coasters.flatcraft"]);
    assert!(!dir.path().join("thumbnails").exists());
    for _ in 0..5 {
        h.step();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "read once");

    // Open, change and save in the window: the file carries a new
    // picture and Home reads it again.
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Open(path.clone())]);
    common::wait_for_long(&mut h);
    let doc = s.lock().unwrap().doc().id();
    s.lock()
        .unwrap()
        .execute(
            "object.move",
            json!({ "ids": [1], "dx_mm": 100.0, "dy_mm": 100.0 }),
            Origin::User,
        )
        .unwrap();
    // A file system with coarse times still sees a new size or time.
    std::thread::sleep(Duration::from_millis(20));
    app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Save { doc_id: doc }]);
    let v2 = flatcraft_engine::thumbnails::project_version(&path).unwrap();
    assert_ne!(v1, v2);
    let png2 = flatcraft_core::project::read_thumbnail(&path)
        .unwrap()
        .unwrap();
    assert_ne!(png, png2, "the picture shows the moved square");
    app_mut(&mut h).ui.workspace = Workspace::Home;
    wait_for_pictures(&mut h);
    assert_eq!(shown(&h, &path).map(|(v, _)| v), Some(v2));
    assert_eq!(calls.load(Ordering::SeqCst), 2, "read again after the save");
}

#[test]
fn home_shows_the_eight_newest_and_show_all_lists_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let paths: Vec<PathBuf> = (1..=11)
        .map(|i| dir.path().join(format!("p{i:02}.flatcraft")))
        .collect();
    s.lock().unwrap().prefs.recent_projects = paths.clone();
    // Tall enough that "Show fewer" is on screen under all eleven rows.
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 1600.0], 8, |a| a);
    // All missing: one Remove button per shown row.
    assert_eq!(
        h.get_all_by_role_and_label(Role::Button, "Remove from list")
            .count(),
        8
    );
    h.get_by_role_and_label(Role::Button, "Show all (11)")
        .click();
    h.run_plan();
    assert_eq!(
        h.get_all_by_role_and_label(Role::Button, "Remove from list")
            .count(),
        11
    );
    h.get_by_role_and_label(Role::Button, "Show fewer").click();
    h.run_plan();
    assert_eq!(
        h.get_all_by_role_and_label(Role::Button, "Remove from list")
            .count(),
        8
    );
}

#[test]
fn the_recent_list_keeps_more_than_the_menu_shows() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness_with(&dir, s.clone(), |a| a);
    let ctx = h.ctx.clone();
    for i in 0..flatcraft::ui::MAX_RECENT + 2 {
        let p = dir.path().join(format!("p{i:02}.flatcraft"));
        s.lock()
            .unwrap()
            .execute("project.save", json!({ "path": p }), Origin::User)
            .unwrap();
        app_mut(&mut h).apply(&ctx, vec![flatcraft::ui::Action::Open(p)]);
        common::wait_for_long(&mut h);
    }
    let st = s.lock().unwrap();
    assert_eq!(st.prefs.recent_projects.len(), flatcraft::ui::MAX_RECENT);
    assert_eq!(st.prefs.recent_details.len(), flatcraft::ui::MAX_RECENT);
    const { assert!(flatcraft::ui::MAX_RECENT > flatcraft::ui::home::SHOWN_RECENT) };
    const { assert!(flatcraft::ui::MENU_RECENT < flatcraft::ui::MAX_RECENT) };
}

#[test]
fn right_click_removes_a_missing_recent_project() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let gone = dir.path().join("gone.flatcraft");
    s.lock().unwrap().prefs.recent_projects = vec![gone];
    let mut h = harness_with(&dir, s.clone(), |a| a);
    let at = h
        .get_by_role_and_label(Role::Button, "Locate...")
        .rect()
        .left_center()
        - egui::vec2(200.0, 0.0);
    h.hover_at(at);
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_plan();
    // The row's own button and the menu's.
    let removes: Vec<_> = h
        .get_all_by_role_and_label(Role::Button, "Remove from list")
        .collect();
    assert_eq!(removes.len(), 2, "the menu opened");
    removes.last().unwrap().click();
    h.run_plan();
    assert!(s.lock().unwrap().prefs.recent_projects.is_empty());
}

#[test]
fn a_picture_of_an_old_version_never_replaces_the_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    let projects = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = projects.path().join("coasters.flatcraft");
    saved_project(&s, &path);
    s.lock().unwrap().prefs.recent_projects = vec![path.clone()];
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    // While the worker reads the picture, the project is saved again
    // with another picture.
    let mut h = harness_with(&dir, s.clone(), move |a| {
        a.with_inline_pictures(false)
            .with_thumbnailer(move |p, f, m| {
                let png = flatcraft::ui::home::render_project(p, f, m)?;
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    flatcraft_core::project::Archive::edit(f, |a| a.thumbnail = Some(tiny_png()))
                        .unwrap();
                }
                Ok(png)
            })
    });
    wait_for_pictures(&mut h);
    // The first picture showed an old version and was dropped; the next
    // frame read the new one.
    let start = std::time::Instant::now();
    while calls.load(Ordering::SeqCst) < 2 || app(&h).ui.home.busy() {
        assert!(
            start.elapsed() < common::WORKER_TIMEOUT,
            "the newer picture was not read within {:?}",
            common::WORKER_TIMEOUT
        );
        h.step();
        wait_for_pictures(&mut h);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let now = flatcraft_engine::thumbnails::project_version(&path).unwrap();
    assert_eq!(shown(&h, &path), Some((now, [2, 2])), "the newer picture");
}

#[test]
fn home_looks_at_recent_files_once_per_visit() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("coasters.flatcraft");
    saved_project(&s, &path);
    s.lock().unwrap().prefs.recent_projects = vec![path.clone(), dir.path().join("gone.flatcraft")];
    let mut h = harness_with(&dir, s.clone(), |a| a);
    let checks = app(&h).ui.home.file_checks;
    assert_eq!(checks, 2);
    for _ in 0..10 {
        h.step();
    }
    assert_eq!(
        app(&h).ui.home.file_checks,
        2,
        "no file looked at per frame"
    );
    // A new visit looks again.
    app_mut(&mut h).ui.workspace = Workspace::Prepare;
    wait_for_source_check(&mut h);
    app_mut(&mut h).ui.workspace = Workspace::Home;
    h.run_plan();
    assert_eq!(app(&h).ui.home.file_checks, 4);
    // So does the window getting focus.
    h.event(egui::Event::WindowFocused(true));
    h.run_plan();
    assert_eq!(app(&h).ui.home.file_checks, 6);
}

/// A 2 x 2 PNG, unlike any mat picture.
fn tiny_png() -> Vec<u8> {
    tiny_skia::Pixmap::new(2, 2).unwrap().encode_png().unwrap()
}

/// A broken picture in the project file is not shown: the project is drawn
/// as if it had none.
#[test]
fn a_broken_thumbnail_is_drawn_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let path = dir.path().join("coasters.flatcraft");
    saved_project(&s, &path);
    flatcraft_core::project::Archive::edit(&path, |a| {
        a.thumbnail = Some(b"\x89PNG\r\n\x1a\n not a picture".to_vec());
    })
    .unwrap();
    let profiles = s.lock().unwrap().profiles.clone();
    let png = flatcraft::ui::home::render_project(
        &profiles,
        &path,
        flatcraft_device::DEFAULT_MAT_COLOR_LIGHT,
    )
    .unwrap();
    let p = tiny_skia::Pixmap::decode_png(&png).expect("a picture that decodes");
    assert_eq!(p.width().max(p.height()), 512);
}

#[test]
fn a_hints_check_that_ends_on_home_frees_the_worker() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("star.svg");
    std::fs::write(&svg, SQUARE_SVG).unwrap();
    let s = session(&dir);
    s.lock().unwrap().doc_mut().dirty = true;
    let picker = FakePicker::answering([svg.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker));
    // The check waits for the gate, like a slow machine.
    let open = Arc::new(AtomicBool::new(false));
    let gate = open.clone();
    app_mut(&mut h)
        .ui
        .prepare
        .object_hints
        .set_checker(Arc::new(move |input, ids, stop| {
            while !gate.load(Ordering::SeqCst) {
                if stop() {
                    return None;
                }
                std::thread::yield_now();
            }
            flatcraft_engine::report::check_selection_until(input, ids, stop)
        }));
    h.get_by_role_and_label(Role::Button, "Import design...")
        .click();
    wait_for_long(&mut h);
    assert_eq!(app(&h).ui.workspace, Workspace::Prepare);
    assert!(app(&h).ui.prepare.object_hints.busy(), "the check runs");
    // Home while it runs; it ends there.
    h.get_by_role_and_label(Role::Button, "Home").click();
    h.step();
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
    open.store(true, Ordering::SeqCst);
    wait_until(&mut h, |a| a.ui.prepare.object_hints.busy());
    assert_eq!(app(&h).ui.workspace, Workspace::Home);
}
