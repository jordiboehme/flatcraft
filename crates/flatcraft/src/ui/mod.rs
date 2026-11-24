//! The app shell: app bar, workspace switcher, Home and toasts. The state and
//! the drawing do not depend on eframe, so tests drive [`shell`] directly.

pub mod app_bar;
pub mod fonts;
pub mod home;
pub mod icons;
#[cfg(target_os = "macos")]
pub mod menu;
pub mod switcher;
pub mod theme;
pub mod toasts;

use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use flatcraft_engine::session::{Origin, Session};
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

use icons::Icon;
use theme::Tokens;
pub use toasts::Toast;

/// The project repository, opened by Help.
pub const HELP_URL: &str = "https://github.com/jordiboehme/flatcraft";

/// File extension of FlatCraft projects.
pub const PROJECT_EXTENSION: &str = "flatcraft";

/// How many recent projects Home lists.
pub const MAX_RECENT: usize = 10;

/// An agent call this recent counts as a connected agent.
pub const AGENT_ACTIVE: Duration = Duration::from_secs(60);

/// The three places of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Workspace {
    #[default]
    Home,
    Prepare,
    Cut,
}

impl Workspace {
    pub fn name(self) -> &'static str {
        match self {
            Workspace::Home => "Home",
            Workspace::Prepare => "Prepare",
            Workspace::Cut => "Cut",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Workspace::Home => Icon::Home,
            Workspace::Prepare => Icon::Prepare,
            Workspace::Cut => Icon::Cut,
        }
    }

    pub fn accent(self, t: &Tokens) -> egui::Color32 {
        match self {
            Workspace::Home => t.text,
            Workspace::Prepare => t.accent_prepare,
            Workspace::Cut => t.accent_cut,
        }
    }
}

/// Pan and zoom of the mat canvas. The Prepare workspace uses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasView {
    /// Screen points per millimetre.
    pub zoom: f32,
    /// Offset of the media origin from the canvas centre, in points.
    pub pan: egui::Vec2,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            zoom: 2.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

/// Everything the window shows that is not part of the engine session.
#[derive(Debug, Clone, Default)]
pub struct UiState {
    pub workspace: Workspace,
    pub toasts: Vec<Toast>,
    pub show_debug: bool,
    pub show_profiles: bool,
    pub canvas_view: CanvasView,
}

/// Something the person asked for this frame. The widgets only collect
/// actions; [`FlatCraftApp::apply`] runs them after drawing, so the session
/// lock is never held while a widget draws.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// A registry command as `Origin::User`.
    Execute(&'static str, Value),
    /// Save the active document; asks for a path if it has none.
    Save,
    /// Ask for a path, then save the active document there.
    SaveAs,
    /// Ask for a project file and open it.
    OpenDialog,
    /// Open this project file.
    Open(PathBuf),
    SetWorkspace(Workspace),
    ShowProfiles,
    Help,
    Quit,
}

/// The FlatCraft window.
pub struct FlatCraftApp {
    pub session: SharedSession,
    pub config_dir: PathBuf,
    pub ui: UiState,
    pub tokens: Tokens,
    /// True for a window started by `flatcraft mcp --window`.
    pub agent: bool,
    /// The macOS menu bar; it must live as long as the app.
    #[cfg(target_os = "macos")]
    menu: Option<menu::AppMenu>,
}

impl FlatCraftApp {
    /// Installs the fonts and the theme of the preferences on `ctx`.
    pub fn new(ctx: &egui::Context, session: SharedSession, config_dir: PathBuf) -> Self {
        let theme = lock(&session).prefs.theme;
        let tokens = Tokens::for_theme(theme);
        fonts::install(ctx);
        theme::apply(ctx, &tokens);
        Self {
            session,
            config_dir,
            ui: UiState::default(),
            tokens,
            agent: false,
            #[cfg(target_os = "macos")]
            menu: None,
        }
    }

    /// Marks a window started by `flatcraft mcp --window`.
    pub fn with_agent(mut self) -> Self {
        self.agent = true;
        self
    }

    /// Installs the macOS menu bar. Call it once the event loop runs.
    #[cfg(target_os = "macos")]
    pub fn install_menu(&mut self, ctx: &egui::Context) {
        match menu::AppMenu::install(ctx) {
            Ok(m) => self.menu = Some(m),
            Err(e) => eprintln!("warning: cannot install the menu bar: {e}"),
        }
    }

    pub fn toast(&mut self, toast: Toast) {
        self.ui.toasts.push(toast);
    }

    /// Runs the actions the widgets collected.
    pub fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for action in actions {
            self.apply_one(ctx, action);
        }
    }

    fn execute(&mut self, id: &str, params: Value) -> Option<Value> {
        let result = lock(&self.session).execute(id, params, Origin::User);
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                self.toast(Toast::error(e.to_string()));
                None
            }
        }
    }

    fn apply_one(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Execute(id, params) => {
                self.execute(id, params);
            }
            Action::Save => {
                let has_path = lock(&self.session).doc().path.is_some();
                if has_path {
                    if let Some(v) = self.execute("project.save", json!({})) {
                        self.saved(&v);
                    }
                } else {
                    self.apply_one(ctx, Action::SaveAs);
                }
            }
            Action::SaveAs => {
                let file = rfd::FileDialog::new()
                    .add_filter("FlatCraft project", &[PROJECT_EXTENSION])
                    .set_file_name(format!("Untitled.{PROJECT_EXTENSION}"))
                    .save_file();
                if let Some(path) = file
                    && let Some(v) = self.execute("project.save", json!({ "path": path }))
                {
                    self.saved(&v);
                }
            }
            Action::OpenDialog => {
                let file = rfd::FileDialog::new()
                    .add_filter("FlatCraft project", &[PROJECT_EXTENSION])
                    .pick_file();
                if let Some(path) = file {
                    self.apply_one(ctx, Action::Open(path));
                }
            }
            Action::Open(path) => {
                if let Some(v) = self.execute("project.open", json!({ "path": path })) {
                    self.remember_recent(&path);
                    for w in v["warnings"].as_array().into_iter().flatten() {
                        if let Some(w) = w.as_str() {
                            self.toast(Toast::error(w.to_string()));
                        }
                    }
                    self.ui.workspace = Workspace::Prepare;
                }
            }
            Action::SetWorkspace(w) => self.ui.workspace = w,
            Action::ShowProfiles => self.ui.show_profiles = true,
            Action::Help => ctx.open_url(egui::OpenUrl::new_tab(HELP_URL)),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn saved(&mut self, result: &Value) {
        if let Some(path) = result["path"].as_str() {
            let path = PathBuf::from(path);
            let name = display_name(&path);
            self.remember_recent(&path);
            self.toast(Toast::info(format!("Saved {name}")));
        }
    }

    /// Puts the project first in the recent list and stores the preferences.
    fn remember_recent(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let result = {
            let mut s = lock(&self.session);
            let recent = &mut s.prefs.recent_projects;
            recent.retain(|p| p != &path);
            recent.insert(0, path);
            recent.truncate(MAX_RECENT);
            s.prefs.save(&self.config_dir)
        };
        if let Err(e) = result {
            self.toast(Toast::error(format!(
                "Cannot store the recent projects: {e}"
            )));
        }
    }

    /// The label of the agent pill, or `None` in a normal window.
    pub fn agent_status(&self, now: Instant) -> Option<&'static str> {
        if !self.agent {
            return None;
        }
        let last = lock(&self.session).last_agent_activity;
        Some(match last {
            Some(t) if now.saturating_duration_since(t) < AGENT_ACTIVE => "Agent connected",
            _ => "Agent idle",
        })
    }
}

/// Locks the session. A panic in another holder does not take the window down.
pub fn lock(session: &SharedSession) -> MutexGuard<'_, Session> {
    session.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The file name of a project path, for tabs and lists.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Size of icons in buttons, in points.
pub const ICON_SIZE: f32 = 18.0;

/// An icon-only button. `name` is its accessible name and its tooltip.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, name: &str, enabled: bool) -> egui::Response {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ))
    .alt_text(name);
    ui.add_enabled(enabled, egui::Button::image(image))
        .on_hover_text(name)
        .on_disabled_hover_text(name)
}

/// Keyboard shortcuts. On macOS the menu bar owns Undo and Redo.
fn shortcuts(ctx: &egui::Context, actions: &mut Vec<Action>) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
    let cmd_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
    ctx.input_mut(|i| {
        if i.consume_shortcut(&cmd_shift(Key::S)) {
            actions.push(Action::SaveAs);
        }
        if i.consume_shortcut(&cmd(Key::S)) {
            actions.push(Action::Save);
        }
        if i.consume_shortcut(&cmd(Key::O)) {
            actions.push(Action::OpenDialog);
        }
        if i.consume_shortcut(&cmd(Key::N)) {
            actions.push(Action::Execute("document.new", json!({})));
        }
        if !cfg!(target_os = "macos") {
            if i.consume_shortcut(&cmd_shift(Key::Z)) || i.consume_shortcut(&cmd(Key::Y)) {
                actions.push(Action::Execute("edit.redo", json!({})));
            }
            if i.consume_shortcut(&cmd(Key::Z)) {
                actions.push(Action::Execute("edit.undo", json!({})));
            }
        }
    });
}

/// Draws the whole window into `ui` and runs what the person asked for.
pub fn shell(ui: &mut egui::Ui, app: &mut FlatCraftApp) {
    let ctx = ui.ctx().clone();
    let mut actions = Vec::new();
    shortcuts(&ctx, &mut actions);

    app_bar::show(ui, app, &mut actions);
    switcher::ribbon(ui, app, &mut actions);
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(app.tokens.bg))
        .show(ui, |ui| match app.ui.workspace {
            Workspace::Home => home::show(ui, app, &mut actions),
            w @ (Workspace::Prepare | Workspace::Cut) => workspace_placeholder(ui, app, w),
        });

    app.apply(&ctx, actions);

    let now = Instant::now();
    if let Some(next) = toasts::prune(&mut app.ui.toasts, now) {
        ctx.request_repaint_after(next);
    }
    toasts::show(&ctx, &app.ui.toasts, &app.tokens);
    if app.agent {
        // Keeps the agent pill current.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

/// The empty mat area until the Prepare and Cut workspaces fill it.
fn workspace_placeholder(ui: &mut egui::Ui, app: &FlatCraftApp, w: Workspace) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new(format!("{} workspace", w.name()))
                .color(app.tokens.text_dim)
                .font(fonts::medium(16.0)),
        );
    });
}

impl eframe::App for FlatCraftApp {
    #[cfg(target_os = "macos")]
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let actions = self.menu.as_ref().map(menu::AppMenu::actions);
        if let Some(actions) = actions {
            self.apply(ctx, actions);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        shell(ui, self);
    }
}
