//! `flatcraft` opens the window; `flatcraft mcp` serves the MCP tools on
//! stdin and stdout, headless or with the window (`--window`). In the MCP
//! modes stdout carries only the protocol; logs go to stderr.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use clap::Parser as _;
use flatcraft::cli::{Cli, Command};
use flatcraft::ui::FlatCraftApp;
use flatcraft_device::ProfileStore;
use flatcraft_engine::prefs::{self, Preferences};
use flatcraft_engine::session::Session;
use flatcraft_mcp::{OnChange, SharedSession, stdio};

const APP_ID: &str = "io.github.jordiboehme.flatcraft";
const ICON_PNG: &[u8] = include_bytes!("../../../assets/branding/icon-dark.png");

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config_dir = prefs::config_dir();
    match cli.command {
        None => run_window(load_session(config_dir.clone(), true)?, config_dir, None),
        Some(Command::Mcp { window: false }) => run_headless(config_dir),
        Some(Command::Mcp { window: true }) => run_mcp_window(config_dir),
    }
}

/// The session with the presets in `<config_dir>/profiles` and the stored
/// preferences. It watches the preferences file, because another FlatCraft
/// process (a window or a headless `flatcraft mcp`) may change it.
fn load_session(config_dir: PathBuf, has_ui: bool) -> anyhow::Result<SharedSession> {
    let profiles = ProfileStore::load(&config_dir.join("profiles"))
        .context("cannot load the plotter and material profiles")?;
    let (prefs, warning) = Preferences::load_reporting(&config_dir);
    let mut session = Session::new(profiles, prefs, has_ui, config_dir);
    if let Some(w) = warning {
        // stderr only: stdout is the MCP channel.
        eprintln!("warning: {w}");
        session.note_prefs_warning(w);
    }
    session.watch_prefs_file();
    Ok(Arc::new(Mutex::new(session)))
}

/// `flatcraft mcp`: no window, the server on the main thread.
fn run_headless(config_dir: PathBuf) -> anyhow::Result<()> {
    let session = load_session(config_dir, false)?;
    stdio::serve(session, Arc::new(|| {}))
}

/// `flatcraft mcp --window`: the server on a background thread, the window
/// on the main thread (macOS needs that). When the client goes away the
/// window stays; closing the window ends the process.
fn run_mcp_window(config_dir: PathBuf) -> anyhow::Result<()> {
    let session = load_session(config_dir.clone(), true)?;
    let repaint: Arc<Mutex<Option<egui::Context>>> = Arc::default();
    let on_change: OnChange = {
        let repaint = repaint.clone();
        Arc::new(move || {
            let ctx = repaint
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        })
    };
    let server_session = session.clone();
    std::thread::Builder::new()
        .name("mcp-stdio".into())
        .spawn(move || match stdio::serve(server_session, on_change) {
            Ok(()) => eprintln!("flatcraft: the MCP client closed the connection"),
            Err(e) => eprintln!("flatcraft: the MCP server stopped: {e:#}"),
        })
        .context("cannot start the MCP server thread")?;
    // The server thread is not joined: a pending stdin read never ends.
    run_window(session, config_dir, Some(repaint))
}

fn run_window(
    session: SharedSession,
    config_dir: PathBuf,
    agent: Option<Arc<Mutex<Option<egui::Context>>>>,
) -> anyhow::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("FlatCraft")
        .with_app_id(APP_ID)
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([800.0, 520.0]);
    match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => eprintln!("warning: cannot load the app icon: {e}"),
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "FlatCraft",
        options,
        Box::new(move |cc| {
            let mut app = FlatCraftApp::new(&cc.egui_ctx, session, config_dir);
            #[cfg(target_os = "macos")]
            app.install_menu(&cc.egui_ctx);
            if let Some(slot) = agent {
                *slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cc.egui_ctx.clone());
                app = app.with_agent();
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("cannot open the window: {e}"))
}
