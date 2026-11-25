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
    let mut h = Harness::builder()
        .with_size([1400.0, 900.0])
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
    let mut app = FlatCraftApp::new(&ctx, s, dir.path().to_path_buf());
    if !worker_delay().is_zero() {
        app = app.with_source_check(|session| {
            std::thread::sleep(worker_delay());
            flatcraft_engine::commands::sources::check_unlocked(session)
        });
        app.ui.prepare.cache.set_builder(|input| {
            std::thread::sleep(worker_delay());
            ui::prepare::build_shape(input)
        });
    }
    *h.state_mut() = Some(setup(app));
    h.run();
    h
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

/// Longest wait for the source check and the shape worker.
pub const WORKER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Runs frames until no source check is in flight and the shape worker has
/// nothing left to build. Fails after [`WORKER_TIMEOUT`].
pub fn wait_for_workers(h: &mut AppHarness) {
    let start = std::time::Instant::now();
    h.run();
    while app(h).source_check_job.is_some() || app(h).ui.prepare.cache.building() > 0 {
        assert!(
            start.elapsed() < WORKER_TIMEOUT,
            "the source check or the shape worker did not finish within {WORKER_TIMEOUT:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.run();
    }
}
