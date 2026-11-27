//! Helpers shared by the window tests.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use flatcraft::ui::files::{FilePicker, FileRequest};
use flatcraft::ui::{self, FlatCraftApp, fonts};
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::Preferences;
use flatcraft_engine::session::{Origin, Session};
use flatcraft_mcp::SharedSession;
use serde_json::json;

pub type AppHarness = Harness<'static, Option<FlatCraftApp>>;

/// A session on a tempdir config dir. Output never goes to the real
/// Documents folder.
pub fn session(dir: &tempfile::TempDir) -> SharedSession {
    let profiles = ProfileStore::load(&dir.path().join("profiles")).unwrap();
    let prefs = Preferences {
        output_dir: Some(dir.path().join("output")),
        ..Preferences::default()
    };
    Arc::new(Mutex::new(Session::new(
        profiles,
        prefs,
        true,
        dir.path().to_path_buf(),
    )))
}

/// A harness around `ui::shell`. The app is built with the harness context
/// after the fonts are in, as `main` does with eframe's context.
pub fn harness(dir: &tempfile::TempDir, s: SharedSession, agent: bool) -> AppHarness {
    harness_with(dir, s, |app| if agent { app.with_agent() } else { app })
}

/// Like [`harness`], with a hook to adjust the app before the first frame.
pub fn harness_with(
    dir: &tempfile::TempDir,
    s: SharedSession,
    setup: impl FnOnce(FlatCraftApp) -> FlatCraftApp,
) -> AppHarness {
    harness_steps(dir, s, 4, setup)
}

/// Like [`harness_with`], allowing `max_steps` frames to settle. Windows
/// with nested grids need a few frames to measure their columns.
pub fn harness_steps(
    dir: &tempfile::TempDir,
    s: SharedSession,
    max_steps: u64,
    setup: impl FnOnce(FlatCraftApp) -> FlatCraftApp,
) -> AppHarness {
    harness_sized(dir, s, [1400.0, 900.0], max_steps, setup)
}

/// Like [`harness_steps`], with a window of `size` points.
pub fn harness_sized(
    dir: &tempfile::TempDir,
    s: SharedSession,
    size: [f32; 2],
    max_steps: u64,
    setup: impl FnOnce(FlatCraftApp) -> FlatCraftApp,
) -> AppHarness {
    let mut h = Harness::builder()
        .with_size(size)
        .with_max_steps(max_steps)
        .with_context_setup(fonts::install)
        .build_ui_state(
            |ui, app: &mut Option<FlatCraftApp>| {
                if let Some(app) = app {
                    ui::shell(ui, app);
                }
            },
            None,
        );
    let ctx = h.ctx.clone();
    fonts::install(&ctx);
    // Home's pictures are drawn within the frame: a worker that finishes
    // at a random frame would make every test that starts on Home racy.
    // The Home tests that check the worker turn this off.
    let mut app = FlatCraftApp::new(&ctx, s, dir.path().to_path_buf()).with_inline_pictures(true);
    if !worker_delay().is_zero() {
        let repaint = ctx.clone();
        app = app.with_source_check(move |session| {
            slow_worker(&repaint);
            flatcraft_engine::commands::sources::check_unlocked(session)
        });
        app.ui.prepare.cache.set_builder(|input| {
            std::thread::sleep(worker_delay());
            ui::prepare::build_shape(input)
        });
        let repaint = ctx.clone();
        app.ui.cut.set_builder(move |input| {
            slow_worker(&repaint);
            flatcraft_engine::job::build_classified(input)
        });
        let repaint = ctx.clone();
        app.ui.cut.sim.set_computer(move |input| {
            slow_worker(&repaint);
            flatcraft_engine::simulate::compute(input)
        });
        let repaint = ctx.clone();
        app.ui
            .prepare
            .object_hints
            .set_checker(std::sync::Arc::new(move |input, ids, stop| {
                slow_worker(&repaint);
                flatcraft_engine::report::check_selection_until(input, ids, stop)
            }));
        let repaint = ctx.clone();
        app.long.set_runner(move |session, work| {
            slow_worker(&repaint);
            ui::long::run(session, work)
        });
    }
    *h.state_mut() = Some(setup(app));
    settle(&mut h);
    h
}

/// Lets the window settle like `Harness::run`, but a worker that started
/// in the first frames (Home's picture worker, a source check) may keep
/// asking for repaints for a while: then it steps one frame at a time
/// until the window settles. Fails after [`WORKER_TIMEOUT`], never after a
/// count of frames, so a busy machine only makes it slower.
pub fn settle(h: &mut AppHarness) {
    let start = std::time::Instant::now();
    while h.run_ok().is_none() || landing(app(h)) {
        assert!(
            start.elapsed() < WORKER_TIMEOUT,
            "the window did not settle within {WORKER_TIMEOUT:?} ({})",
            report(h)
        );
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The workers that still run, by name: the source check, the shape
/// worker, the Cut plan, the simulation, the kept mask, a long command,
/// the object inspector's hints and Home's pictures. Each one asks for a
/// frame when it ends, and the window takes its result on every frame,
/// whatever workspace shows.
pub fn workers(app: &FlatCraftApp) -> Vec<&'static str> {
    [
        ("source check", app.source_check_job.is_some()),
        ("shapes", app.ui.prepare.cache.building() > 0),
        ("plan", app.ui.cut.planning()),
        ("simulation", app.ui.cut.sim.busy()),
        ("kept", app.ui.cut.kept.pending()),
        ("long command", app.long.busy()),
        ("hints", app.ui.prepare.object_hints.busy()),
        ("home pictures", app.ui.home.busy()),
    ]
    .into_iter()
    .filter_map(|(name, busy)| busy.then_some(name))
    .collect()
}

/// The workers whose results land in the window (the plan, the
/// simulation, the kept mask, a long command, the hints): a test waits
/// for them before it reads what they change.
pub fn landing(app: &FlatCraftApp) -> bool {
    app.ui.cut.planning()
        || app.ui.cut.sim.busy()
        || app.ui.cut.kept.pending()
        || app.long.busy()
        || app.ui.prepare.object_hints.busy()
}

/// Which workers run and why the last frame was painted, for a timeout.
fn report(h: &AppHarness) -> String {
    format!(
        "busy {:?}, workspace {:?}, repaint causes {:?}",
        workers(app(h)),
        app(h).ui.workspace,
        h.ctx.repaint_causes()
    )
}

/// Selects nothing, so the inspector shows the Job panel (with a
/// selection it shows the object inspector).
pub fn select_none(s: &SharedSession) {
    s.lock()
        .unwrap()
        .execute("object.select", json!({ "ids": [] }), Origin::User)
        .unwrap();
}

/// Shows the Job panel over the selection: a click on the job line.
pub fn show_job(h: &mut AppHarness) {
    use egui_kittest::kittest::Queryable as _;
    h.get_by_label_contains("Job: ").click();
    h.run_plan();
}

/// Shows Prepare and waits for the source check it starts.
pub fn to_prepare(h: &mut AppHarness) {
    app_mut(h).ui.workspace = ui::Workspace::Prepare;
    wait_for_source_check(h);
}

/// Steps the active document can undo.
pub fn undo_len(s: &SharedSession) -> usize {
    s.lock().unwrap().doc().undo.undo_len()
}

/// Every shape painted last frame, with nested lists flattened.
pub fn painted(h: &AppHarness) -> Vec<egui::Shape> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            other => out.push(other.clone()),
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

pub fn app(h: &AppHarness) -> &FlatCraftApp {
    h.state().as_ref().unwrap()
}

pub fn app_mut(h: &mut AppHarness) -> &mut FlatCraftApp {
    h.state_mut().as_mut().unwrap()
}

/// A 50 mm square at 20,20 on a 200 mm page.
pub const SQUARE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200mm" height="200mm" viewBox="0 0 200 200"><rect x="20" y="20" width="50" height="50"/></svg>"#;

/// Imports [`SQUARE_SVG`] as "square" and returns its id.
pub fn import_square(s: &SharedSession) -> u64 {
    let r = s
        .lock()
        .unwrap()
        .execute(
            "object.import_svg",
            json!({ "svg": SQUARE_SVG, "name": "square" }),
            Origin::User,
        )
        .unwrap();
    r["id"].as_u64().unwrap()
}

/// A file picker that answers with queued paths and records the requests.
#[derive(Clone, Default)]
pub struct FakePicker {
    pub answers: Arc<Mutex<Vec<PathBuf>>>,
    pub requests: Arc<Mutex<Vec<FileRequest>>>,
}

impl FakePicker {
    pub fn answering(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let p = Self::default();
        p.answers.lock().unwrap().extend(paths);
        p
    }

    fn next(&self, req: &FileRequest) -> Option<PathBuf> {
        self.requests.lock().unwrap().push(req.clone());
        let mut a = self.answers.lock().unwrap();
        (!a.is_empty()).then(|| a.remove(0))
    }
}

impl FilePicker for FakePicker {
    fn pick_file(&mut self, req: &FileRequest) -> Option<PathBuf> {
        self.next(req)
    }

    fn save_file(&mut self, req: &FileRequest) -> Option<PathBuf> {
        self.next(req)
    }

    fn pick_folder(&mut self, req: &FileRequest) -> Option<PathBuf> {
        self.next(req)
    }
}

/// A delay every source check and shape build waits first, from
/// `FLATCRAFT_TEST_WORKER_DELAY_MS` (read only, never set by a test). It
/// makes the workers as slow as a busy CI runner, so tests that do not
/// wait for them properly fail locally too. Zero when unset.
pub fn worker_delay() -> std::time::Duration {
    let ms = std::env::var("FLATCRAFT_TEST_WORKER_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    std::time::Duration::from_millis(ms)
}

/// Waits [`worker_delay`] and keeps asking for repaints meanwhile, like a busy worker on a slow runner whose frames keep
/// coming. A helper that settles with `Harness::run` while a worker is
/// busy fails under this.
pub fn slow_worker(ctx: &egui::Context) {
    let end = std::time::Instant::now() + worker_delay();
    while std::time::Instant::now() < end {
        ctx.request_repaint();
        std::thread::yield_now();
    }
}

/// Longest wait for the source check and the shape worker.
pub const WORKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Runs frames until no worker runs (see [`workers`]), the shape worker
/// included, then lets the window settle. See [`wait_until`].
pub fn wait_for_workers(h: &mut AppHarness) {
    wait_until(h, |app| !workers(app).is_empty());
}

/// Runs frames until no source check and no long command is in flight and
/// the workers that land in the window ended (see [`landing`]), then lets
/// the window settle. For frames that may start a check (entering
/// Prepare, a new or switched document). Opening a project or a sample is
/// a long command on a worker that switches the document only when it
/// lands, so it is waited for too. The shape worker may stay busy.
pub fn wait_for_source_check(h: &mut AppHarness) {
    wait_until(h, |app| app.source_check_job.is_some() || landing(app));
}

/// Runs frames until the long commands (import, trace, reload, open,
/// export) the window queued have landed, then lets the window settle.
/// `FlatCraftApp::apply` waits only [`flatcraft::ui::long::LONG_WAIT`] for
/// one, so a test reads their result only after this.
pub fn wait_for_long(h: &mut AppHarness) {
    wait_until(h, |app| app.long.busy());
}

/// Runs frames until `busy` is false and the window settled. Every worker
/// asks for a frame when it ends, so this waits for that event, frame by
/// frame (`Harness::step`), because `Harness::run` fails while a worker
/// keeps asking for repaints. [`WORKER_TIMEOUT`] only ends a wait that
/// would never end; it then names the workers that still run.
pub fn wait_until(h: &mut AppHarness, busy: impl Fn(&FlatCraftApp) -> bool) {
    let start = std::time::Instant::now();
    loop {
        h.step();
        // Settled, and settling started no new work.
        if !busy(app(h)) && h.run_ok().is_some() && !busy(app(h)) {
            return;
        }
        assert!(
            start.elapsed() < WORKER_TIMEOUT,
            "the workers did not finish within {WORKER_TIMEOUT:?} ({})",
            report(h)
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Records the files it is asked to show instead of starting the file
/// manager.
#[derive(Clone, Default)]
pub struct FakeRevealer(pub Arc<Mutex<Vec<PathBuf>>>);

impl flatcraft::ui::files::Revealer for FakeRevealer {
    fn reveal(&mut self, file: &std::path::Path) -> Result<(), String> {
        self.0.lock().unwrap().push(file.to_path_buf());
        Ok(())
    }
}

/// The text the last frame put on the clipboard, if any.
pub fn copied(h: &AppHarness) -> Option<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
}

/// The text a field shows (its accessible value).
pub fn field_text(h: &AppHarness, label: &str) -> String {
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    h.get_by_label(label)
        .accesskit_node()
        .value()
        .unwrap_or_default()
}

/// Two clicks at `at` in a fixed number of frames: each press and
/// release takes one frame (`Harness::step`), never a wait for workers in
/// between. A test frame lasts a quarter second of egui time, and a wait
/// steps as many frames as a busy worker needs, so the second click would
/// come late on a slow machine. Set `max_double_click_delay` (about 2 s)
/// and let the last click age out first.
pub fn double_click_at(h: &mut AppHarness, at: egui::Pos2) {
    for _ in 0..2 {
        h.drag_at(at);
        h.step();
        h.drop_at(at);
        h.step();
    }
}

/// A click at `at`: the press and the release take one frame each, then
/// the window settles and the workers that land in it end. egui counts a
/// press longer than `max_click_duration` (0.8 s, about three test frames)
/// as no click, so no wait for workers may come between the two.
pub fn click_at(h: &mut AppHarness, at: egui::Pos2) {
    h.drag_at(at);
    h.step();
    h.drop_at(at);
    h.step();
    wait_until(h, landing);
}

/// [`click_at`] that ends with `Harness::run`.
fn press_at(h: &mut AppHarness, at: egui::Pos2) {
    h.drag_at(at);
    h.step();
    h.drop_at(at);
    h.step();
    h.run();
}

/// Checks the click rules of the number field `label` (see
/// `ui::calc`): a single click places the cursor, a double-click and Tab
/// (here Tab out and Shift-Tab back) select the whole value, so typing
/// replaces it. Each step ends with Esc, so nothing changes.
pub fn check_click_rules(h: &mut AppHarness, label: &str) {
    use egui_kittest::kittest::Queryable as _;
    let shown = field_text(h, label);
    let esc = |h: &mut AppHarness| {
        h.key_press(egui::Key::Escape);
        h.run();
        h.run();
    };
    // A single click past the end: the digit is added.
    let r = h.get_by_label(label).rect();
    let end = egui::pos2(r.right() - 2.0, r.center().y);
    h.hover_at(end);
    h.run();
    press_at(h, end);
    h.event(egui::Event::Text("5".into()));
    h.run();
    assert_eq!(
        field_text(h, label),
        format!("{shown}5"),
        "{label}: single click"
    );
    esc(h);
    assert_eq!(field_text(h, label), shown, "{label}: Esc");
    // A double-click on the first digit: all of it is replaced.
    let r = h.get_by_label(label).rect();
    let start = egui::pos2(r.left() + 6.0, r.center().y);
    h.hover_at(start);
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 2.0);
    for _ in 0..8 {
        h.step();
    }
    double_click_at(h, start);
    h.run();
    h.event(egui::Event::Text("7".into()));
    h.run();
    assert_eq!(field_text(h, label), "7", "{label}: double-click");
    esc(h);
    h.ctx
        .options_mut(|o| o.input_options.max_double_click_delay = 0.3);
    for _ in 0..8 {
        h.step();
    }
    // Tab out and Shift-Tab back in: all of it is replaced.
    h.get_by_label(label).click();
    h.run();
    h.key_press(egui::Key::Tab);
    h.run();
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
    h.run();
    h.event(egui::Event::Text("7".into()));
    h.run();
    assert_eq!(field_text(h, label), "7", "{label}: Tab");
    esc(h);
    assert_eq!(field_text(h, label), shown, "{label}: Esc");
}

/// Settling that also waits for the Cut plan worker.
pub trait RunPlan {
    /// Runs frames until the Cut plan of the current input landed and the
    /// window settled, by the clock (see [`wait_until`]).
    fn run_plan(&mut self);
}

impl RunPlan for AppHarness {
    fn run_plan(&mut self) {
        wait_until(self, landing);
    }
}
