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
