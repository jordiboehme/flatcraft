//! Toasts (Task 40): lifetimes per kind on the window's clock, hover
//! pauses, errors stay, a click shows the file, the × closes, the stack
//! has the newest at the bottom, and the window repaints often only
//! while a ring moves.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::*;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use flatcraft::ui::files::{Revealer, reveal_label};
use flatcraft::ui::toasts::{CLOSE_NAME, LIFETIME, ToastKind, WARNING_LIFETIME};
use flatcraft::ui::{Action, NOTHING_TO_UNDO, Toast};

#[derive(Clone, Default)]
struct FakeRevealer(Arc<Mutex<Vec<PathBuf>>>);

impl Revealer for FakeRevealer {
    fn reveal(&mut self, file: &Path) -> Result<(), String> {
        self.0.lock().unwrap().push(file.to_path_buf());
        Ok(())
    }
}

struct Rig {
    h: AppHarness,
    now: Arc<Mutex<Instant>>,
    revealed: FakeRevealer,
    _dir: tempfile::TempDir,
}

fn rig(reduce_motion: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let now = Arc::new(Mutex::new(Instant::now()));
    let clock = now.clone();
    let revealed = FakeRevealer::default();
    let r = revealed.clone();
    let h = harness_with(&dir, s, move |a| {
        a.with_clock(move || *clock.lock().unwrap())
            .with_revealer(r)
            .with_reduce_motion(move || reduce_motion)
    });
    Rig {
        h,
        now,
        revealed,
        _dir: dir,
    }
}

impl Rig {
    fn show(&mut self, t: Toast) {
        app_mut(&mut self.h).toast(t);
        self.h.run();
    }

    fn later(&mut self, d: Duration) {
        *self.now.lock().unwrap() += d;
        self.h.run();
    }

    fn titles(&self) -> Vec<String> {
        app(&self.h)
            .ui
            .toasts
            .iter()
            .map(|t| t.text.clone())
            .collect()
    }

    fn repaint_delay(&self) -> Duration {
        self.h
            .output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |v| v.repaint_delay)
    }
}

#[test]
fn each_kind_has_its_lifetime_and_errors_stay() {
    let mut r = rig(false);
    r.show(Toast::info("info"));
    r.show(Toast::success("Saved"));
    r.show(Toast::warning("careful"));
    r.show(Toast::error("broken"));
    r.later(LIFETIME - Duration::from_millis(100));
    assert_eq!(r.titles().len(), 4);
    r.later(Duration::from_millis(100));
    assert_eq!(r.titles(), ["careful", "broken"]);
    r.later(WARNING_LIFETIME - LIFETIME);
    assert_eq!(r.titles(), ["broken"]);
    r.later(Duration::from_secs(3600));
    assert_eq!(r.titles(), ["broken"], "an error stays until closed");
    // Its × closes it.
    r.h.get_by_role_and_label(Role::Button, CLOSE_NAME).click();
    r.h.run();
    assert!(r.titles().is_empty());
}

#[test]
fn hovering_a_toast_pauses_it() {
    let mut r = rig(false);
    r.show(Toast::info("Saved"));
    r.later(Duration::from_secs(2));
    let at = r.h.get_by_label("Saved").rect().center();
    r.h.hover_at(at);
    r.h.run();
    r.later(Duration::from_secs(30));
    assert_eq!(r.titles(), ["Saved"], "the clock stands while hovered");
    // The pointer leaves: 3 s are left.
    r.h.hover_at(egui::pos2(10.0, 400.0));
    r.h.run();
    r.later(Duration::from_millis(2900));
    assert_eq!(r.titles(), ["Saved"]);
    r.later(Duration::from_millis(100));
    assert!(r.titles().is_empty());
}

#[test]
fn a_click_on_a_toast_with_a_file_shows_the_file() {
    let mut r = rig(false);
    let file = PathBuf::from("/tmp/flatcraft-test/kept.flatcraft");
    r.show(
        Toast::success("Saved")
            .with_detail(file.display().to_string())
            .with_reveal(file.clone()),
    );
    // The detail is the whole path.
    r.h.get_by_label(&file.display().to_string());
    r.h.get_by_role_and_label(Role::Button, reveal_label())
        .click();
    r.h.run();
    assert_eq!(*r.revealed.0.lock().unwrap(), vec![file]);
    // A toast without a file is no button.
    r.show(Toast::info("Nothing to see"));
    assert_eq!(r.h.query_all_by_label(reveal_label()).count(), 1);
}

#[test]
fn saving_shows_a_saved_toast_with_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let path = dir.path().join("kept.flatcraft");
    let picker = FakePicker::answering([path.clone()]);
    let mut h = harness_with(&dir, s.clone(), |a| a.with_file_picker(picker.clone()));
    let doc = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Save { doc_id: doc }]);
    h.run();
    let t = app(&h).ui.toasts.last().cloned().unwrap();
    assert_eq!((t.text.as_str(), t.kind), ("Saved", ToastKind::Success));
    let full = std::path::absolute(&path).unwrap();
    assert_eq!(
        t.detail.as_deref(),
        Some(full.display().to_string().as_str())
    );
    assert_eq!(t.reveal, Some(full));
}

#[test]
fn the_newest_toast_is_at_the_bottom() {
    let mut r = rig(false);
    r.show(Toast::info("first"));
    r.show(Toast::warning("second"));
    r.show(Toast::error("third"));
    let y = |r: &Rig, t: &str| r.h.get_by_label(t).rect().top();
    assert!(y(&r, "first") < y(&r, "second"));
    assert!(y(&r, "second") < y(&r, "third"));
    // In the bottom-right corner.
    let third = r.h.get_by_label("third").rect();
    let window = r.h.ctx.content_rect();
    assert!(third.right() > window.center().x && third.bottom() > window.center().y);
}

#[test]
fn the_window_repaints_often_only_while_a_ring_moves() {
    let mut r = rig(false);
    r.show(Toast::info("moving"));
    assert!(r.repaint_delay() <= Duration::from_millis(50));
    // An error has no ring: no busy repaint.
    let mut r = rig(false);
    r.show(Toast::error("still"));
    assert!(r.repaint_delay() > Duration::from_secs(1));
    // A hovered toast's ring stands still.
    let mut r = rig(false);
    r.show(Toast::info("rest"));
    let at = r.h.get_by_label("rest").rect().center();
    r.h.hover_at(at);
    r.h.run();
    assert!(r.repaint_delay() > Duration::from_secs(1));
    // Reduced motion: no ring, the same lifetime, a repaint only when it
    // expires.
    let mut r = rig(true);
    r.show(Toast::info("calm"));
    let d = r.repaint_delay();
    assert!(d > Duration::from_secs(4) && d <= LIFETIME, "{d:?}");
    r.later(LIFETIME);
    assert!(r.titles().is_empty());
}

#[test]
fn the_easter_egg_toasts_still_show() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    let mut h = harness(&dir, s.clone(), false);
    let doc = s.lock().unwrap().doc().id();
    let ctx = h.ctx.clone();
    app_mut(&mut h).apply(&ctx, vec![Action::Undo(doc)]);
    h.run();
    h.get_by_label(NOTHING_TO_UNDO);
}

#[test]
fn closing_a_toast_never_hits_its_neighbour() {
    let mut r = rig(false);
    let a = PathBuf::from("/tmp/flatcraft-test/a.flatcraft");
    let b = PathBuf::from("/tmp/flatcraft-test/b.flatcraft");
    r.show(Toast::success("A").with_reveal(a));
    r.show(Toast::success("B").with_reveal(b.clone()));
    let ids: Vec<u64> = app(&r.h).ui.toasts.iter().map(|t| t.id).collect();
    assert_ne!(ids[0], ids[1]);
    // Close A with its ×: the top one, with the pointer.
    let x =
        r.h.query_all_by_label(CLOSE_NAME)
            .map(|n| n.rect())
            .min_by(|x, y| x.top().total_cmp(&y.top()))
            .unwrap()
            .center();
    r.h.hover_at(x);
    r.h.run();
    r.h.drag_at(x);
    r.h.run();
    r.h.drop_at(x);
    r.h.run();
    assert_eq!(r.titles(), ["B"]);
    assert_eq!(app(&r.h).ui.toasts[0].id, ids[1], "B keeps its id");
    // A click on B shows B's file.
    r.h.get_by_role_and_label(Role::Button, reveal_label())
        .click();
    r.h.run();
    assert_eq!(*r.revealed.0.lock().unwrap(), vec![b]);
}

#[test]
fn in_prepare_and_cut_notices_sit_over_the_canvas() {
    for ws in [
        flatcraft::ui::Workspace::Prepare,
        flatcraft::ui::Workspace::Cut,
    ] {
        let mut r = rig(false);
        app_mut(&mut r.h).ui.workspace = ws;
        settle(&mut r.h);
        r.show(Toast::info("over the canvas"));
        r.h.run();
        let toast = r.h.get_by_label("over the canvas").rect();
        let canvas = match ws {
            flatcraft::ui::Workspace::Cut => app(&r.h).ui.cut.canvas_rect,
            _ => app(&r.h).ui.prepare.canvas_rect,
        };
        // Inside the canvas, so never over a side panel's buttons.
        assert!(
            canvas.contains_rect(toast),
            "{ws:?}: {toast:?} in {canvas:?}"
        );
    }
}

/// The notice stack's rect this frame.
fn stack_rect(h: &AppHarness) -> egui::Rect {
    h.ctx
        .memory(|m| m.area_rect(egui::Id::unique("flatcraft-toasts")))
        .unwrap()
}

#[test]
fn many_notices_on_a_short_window_stay_inside_the_canvas() {
    for ws in [
        flatcraft::ui::Workspace::Prepare,
        flatcraft::ui::Workspace::Cut,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let s = session(&dir);
        let mut h = harness_sized(&dir, s, [1000.0, 520.0], 8, |a| a);
        app_mut(&mut h).ui.workspace = ws;
        settle(&mut h);
        // Errors stay until closed: ten of them, one with a long path.
        for i in 0..10 {
            app_mut(&mut h).toast(Toast::error(format!("error {i}")));
        }
        app_mut(&mut h).toast(Toast::error("long").with_detail("/a/very/long/path/".repeat(12)));
        settle(&mut h);
        let canvas = match ws {
            flatcraft::ui::Workspace::Cut => app(&h).ui.cut.canvas_rect,
            _ => app(&h).ui.prepare.canvas_rect,
        };
        let stack = stack_rect(&h);
        // Never past the canvas: not onto the Cut timeline below it.
        assert!(
            stack.bottom() <= canvas.bottom() + 0.5,
            "{ws:?}: {stack:?} in {canvas:?}"
        );
        assert!(stack.top() >= canvas.top(), "{ws:?}");
        // Above the status pills ("This mat is not calibrated yet").
        if let Some(pills) = app(&h)
            .ui
            .prepare
            .pills_rect
            .filter(|_| ws == flatcraft::ui::Workspace::Prepare)
        {
            assert!(stack.bottom() <= pills.top(), "{stack:?} over {pills:?}");
        }
        // The newest is in view and can be closed.
        let newest = h.get_by_label("long").rect();
        assert!(canvas.contains_rect(newest), "{ws:?}: {newest:?}");
    }
}

/// A notice with a long title and path, to show its width.
fn long_notice() -> Toast {
    Toast::error("Cannot write the job to the output folder you picked").with_detail(
        "/Users/me/Documents/FlatCraft Output/a rather long project name/untitled-1.gpgl",
    )
}

#[test]
fn on_the_smallest_window_notices_keep_the_side_panels_free() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    import_square(&s);
    let mut h = harness_sized(&dir, s.clone(), [900.0, 700.0], 8, |a| a);
    // Prepare with both side panels open.
    to_prepare(&mut h);
    for p in [
        flatcraft::ui::rails::SidePanel::Objects,
        flatcraft::ui::rails::SidePanel::Settings,
    ] {
        app_mut(&mut h).ui.prepare.panels.set_collapsed(p, false);
    }
    settle(&mut h);
    assert!(
        !app(&h)
            .ui
            .prepare
            .panels
            .collapsed(flatcraft::ui::rails::SidePanel::Objects)
    );
    assert!(
        !app(&h)
            .ui
            .prepare
            .panels
            .collapsed(flatcraft::ui::rails::SidePanel::Settings)
    );
    app_mut(&mut h).toast(long_notice());
    settle(&mut h);
    let canvas = app(&h).ui.prepare.canvas_rect;
    let stack = stack_rect(&h);
    assert!(
        stack.right() <= canvas.right() + 0.5,
        "{stack:?} past {canvas:?}"
    );
    assert!(stack.left() >= canvas.left(), "{stack:?}");
    // The canvas is narrower than a full card here: the card shrank.
    assert!(
        stack.width() < flatcraft::ui::toasts::WIDTH,
        "{stack:?} in {canvas:?}"
    );
    // Cut, Send stage: the stack stays off the stage panel and Send
    // takes a click while the notice shows.
    app_mut(&mut h).ui.workspace = flatcraft::ui::Workspace::Cut;
    app_mut(&mut h)
        .ui
        .prepare
        .panels
        .set_collapsed(flatcraft::ui::rails::SidePanel::Stage, false);
    settle(&mut h);
    h.get_by_role_and_label(Role::Tab, "Send").click();
    settle(&mut h);
    let canvas = app(&h).ui.cut.canvas_rect;
    let stack = stack_rect(&h);
    assert!(
        stack.right() <= canvas.right() + 0.5,
        "{stack:?} past {canvas:?}"
    );
    assert!(!app(&h).ui.toasts.is_empty());
    h.get_by_role_and_label(Role::Button, "Send to File Output")
        .click();
    settle(&mut h);
    assert_eq!(
        flatcraft_engine::cutlog::read(dir.path()).entries.len(),
        1,
        "Send took the click"
    );
}

#[test]
fn on_a_canvas_too_short_for_a_card_the_notices_fold_into_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    // So short that the pills leave less than one card's room.
    let mut h = harness_sized(&dir, s, [1000.0, 250.0], 8, |a| a);
    app_mut(&mut h).ui.workspace = flatcraft::ui::Workspace::Prepare;
    settle(&mut h);
    app_mut(&mut h).toast(Toast::error("first"));
    app_mut(&mut h).toast(Toast::error("second"));
    settle(&mut h);
    let label = flatcraft::ui::toasts::folded_label(2);
    assert_eq!(label, "2 notices");
    assert!(h.query_by_label("second").is_none(), "folded");
    let pills = app(&h).ui.prepare.pills_rect.unwrap();
    assert!(!stack_rect(&h).intersects(pills));
    // A click opens them; Hide folds them again.
    h.get_by_role_and_label(Role::Button, &label).click();
    settle(&mut h);
    h.get_by_label("second");
    // Open, it stays inside the canvas: it scrolls instead of growing
    // past the top.
    let canvas = app(&h).ui.prepare.canvas_rect;
    let stack = stack_rect(&h);
    assert!(stack.top() >= canvas.top(), "{stack:?} above {canvas:?}");
    assert!(stack.bottom() <= canvas.bottom() + 0.5, "{stack:?}");
    h.get_by_role_and_label(Role::Button, flatcraft::ui::toasts::FOLD_LABEL)
        .click();
    settle(&mut h);
    assert!(h.query_by_label("second").is_none());
}

#[test]
fn the_zoom_hint_moves_above_the_notices() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(&dir);
    // More starts than the order numbers can show: the hint appears.
    let mut body = String::new();
    for i in 0..320 {
        let (x, y) = (10 + (i % 20) * 14, 10 + (i / 20) * 14);
        body.push_str(&format!(r#"<rect x="{x}" y="{y}" width="6" height="6"/>"#));
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="300mm" viewBox="0 0 300 300">{body}</svg>"#
    );
    s.lock()
        .unwrap()
        .execute(
            "object.import_svg",
            serde_json::json!({ "svg": svg, "name": "dots" }),
            flatcraft_engine::session::Origin::User,
        )
        .unwrap();
    let mut h = harness_sized(&dir, s.clone(), [1400.0, 900.0], 8, |a| a);
    app_mut(&mut h).ui.workspace = flatcraft::ui::Workspace::Cut;
    settle(&mut h);
    h.get_by_role_and_label(Role::Tab, "Path").click();
    wait_for_workers(&mut h);
    settle(&mut h);
    let hint = |h: &AppHarness| {
        painted(h).into_iter().find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == flatcraft::ui::cut::ZOOM_HINT => {
                Some(t.galley.rect.translate(t.pos.to_vec2()))
            }
            _ => None,
        })
    };
    assert!(hint(&h).is_some(), "the hint shows");
    app_mut(&mut h).toast(Toast::error("over the hint"));
    settle(&mut h);
    h.run();
    let stack = stack_rect(&h);
    let at = hint(&h).unwrap();
    assert!(!at.intersects(stack), "{at:?} under {stack:?}");
}
