//! The app shell: app bar, workspace switcher, Home and toasts. The state and
//! the drawing do not depend on eframe, so tests drive [`shell`] directly.

pub mod app_bar;
pub mod canvas;
pub mod cut;
pub mod debug;
pub mod files;
pub mod fonts;
pub mod home;
pub mod icons;
pub mod import_dialog;
#[cfg(target_os = "macos")]
pub mod menu;
pub mod objects;
pub mod prepare;
pub mod ribbon;
pub mod settings;
pub mod switcher;
pub mod theme;
pub mod timeline;
pub mod toasts;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use flatcraft_engine::session::{CommandError, JobSummary, Origin, PENDING_TTL, Session};
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

/// Everything the window shows that is not part of the engine session.
#[derive(Debug, Default)]
pub struct UiState {
    pub workspace: Workspace,
    pub toasts: Vec<Toast>,
    pub show_debug: bool,
    pub show_profiles: bool,
    pub prepare: prepare::PrepareState,
    /// The number of each untitled document's name, by stable document id,
    /// so "Untitled 3" keeps its name when "Untitled 2" closes.
    pub untitled: BTreeMap<u64, usize>,
    /// The open Trace dialog.
    pub trace: Option<import_dialog::TraceDialog>,
    pub cut: cut::CutState,
    pub debug: debug::DebugState,
}

impl UiState {
    /// Numbers documents seen for the first time and forgets closed ones.
    pub fn number_documents(&mut self, open: &[u64]) {
        self.untitled.retain(|id, _| open.contains(id));
        for id in open {
            if !self.untitled.contains_key(id) {
                let next = self.untitled.values().max().map_or(1, |n| n + 1);
                self.untitled.insert(*id, next);
            }
        }
    }
}

/// Something the person asked for this frame. The widgets only collect
/// actions; [`FlatCraftApp::apply`] runs them after drawing, so the session
/// lock is never held while a widget draws.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// A registry command, as `Origin::User`, on the document it was made
    /// for. If another document became active meanwhile (an agent opened
    /// one), it is not run.
    Run {
        doc_id: u64,
        command: &'static str,
        params: Value,
    },
    /// Ask for a file and import it into this document.
    Import {
        doc_id: u64,
    },
    /// Open the Trace dialog for this raster object.
    TraceDialog {
        doc_id: u64,
        id: u64,
    },
    /// Files dropped on the window: projects open, SVGs and images are
    /// imported into the active document, one `object.import` each.
    Dropped(Vec<PathBuf>),
    /// Ask where an object's linked file went, then `object.relink`.
    Reconnect {
        doc_id: u64,
        id: u64,
    },
    /// Relink these objects to these files (`also_found` of a relink).
    ReconnectAll {
        doc_id: u64,
        items: Vec<(u64, PathBuf)>,
    },
    /// Check the linked files of the active document soon.
    CheckSources,
    /// Save this document; asks for a path if it has none.
    Save {
        doc_id: u64,
    },
    /// Ask for a path, then save this document there.
    SaveAs {
        doc_id: u64,
    },
    /// Save this document to this path.
    SaveTo {
        doc_id: u64,
        path: PathBuf,
    },
    /// Ask for a project file and open it.
    OpenDialog,
    /// Open this project file.
    Open(PathBuf),
    /// The "+" tab: always a new document.
    NewDocument,
    /// Home's New and Cmd+N: reuse the active document if it is pristine,
    /// else open a new one; then go to Prepare.
    NewProject,
    /// Make this document the active one.
    Switch(u64),
    /// Close this document.
    Close(u64),
    /// Undo the last change of this document.
    Undo(u64),
    /// Redo the last undone change of this document.
    Redo(u64),
    SetWorkspace(Workspace),
    /// Send this document's job (`job.send`, as the person).
    Send {
        doc_id: u64,
    },
    /// The person allowed the agent's pending send with this id.
    ConfirmPending(u64),
    /// The person said no to the agent's pending send with this id.
    DenyPending(u64),
    /// Show this file in the file manager.
    Reveal(PathBuf),
    /// Ask for a capture fixture and show it in the Debug panel.
    OpenCapture,
    ShowProfiles,
    Help,
    Quit,
}

impl Action {
    /// A registry command on the document `doc_id`.
    pub fn run(doc_id: u64, command: &'static str, params: Value) -> Self {
        Action::Run {
            doc_id,
            command,
            params,
        }
    }
}

/// A monotonic clock.
pub type Clock = std::sync::Arc<dyn Fn() -> Instant + Send + Sync>;

/// The FlatCraft window.
pub struct FlatCraftApp {
    pub session: SharedSession,
    pub config_dir: PathBuf,
    pub ui: UiState,
    pub tokens: Tokens,
    /// True for a window started by `flatcraft mcp --window`.
    pub agent: bool,
    /// File dialogs; tests replace them.
    pub files: Box<dyn files::FilePicker>,
    /// Traces the Trace dialog's previews; tests replace it.
    pub tracer: import_dialog::Tracer,
    /// Shows sent files in the file manager; tests replace it.
    pub revealer: Box<dyn files::Revealer>,
    /// The monotonic clock pending sends expire by; tests replace it.
    pub clock: Clock,
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
            files: Box::new(files::NativePicker),
            tracer: std::sync::Arc::new(flatcraft_import::trace::trace),
            revealer: Box::new(files::NativeRevealer),
            clock: std::sync::Arc::new(Instant::now),
            #[cfg(target_os = "macos")]
            menu: None,
        }
    }

    /// Marks a window started by `flatcraft mcp --window`.
    pub fn with_agent(mut self) -> Self {
        self.agent = true;
        self
    }

    /// Traces previews with `f` instead of the real tracer (tests).
    pub fn with_tracer(
        mut self,
        f: impl Fn(
            &[u8],
            &flatcraft_core::document::TraceSettings,
            f64,
        )
            -> Result<flatcraft_import::trace::TraceResult, flatcraft_import::ImportError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.tracer = std::sync::Arc::new(f);
        self
    }

    /// Reveals files with `r` instead of the file manager (tests).
    pub fn with_revealer(mut self, r: impl files::Revealer + 'static) -> Self {
        self.revealer = Box::new(r);
        self
    }

    /// Reads the time from `clock` instead of `Instant::now` (tests).
    pub fn with_clock(mut self, clock: impl Fn() -> Instant + Send + Sync + 'static) -> Self {
        self.clock = std::sync::Arc::new(clock);
        self
    }

    /// Uses these file dialogs instead of the platform ones.
    pub fn with_file_picker(mut self, picker: impl files::FilePicker + 'static) -> Self {
        self.files = Box::new(picker);
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
                self.toast(Toast::error(person_text(&e)));
                None
            }
        }
    }

    /// Runs a command on the document `doc_id`, under one lock, only while
    /// it is still the active document. Errors become toasts.
    fn run_on(&mut self, doc_id: u64, command: &'static str, params: Value) -> Option<Value> {
        let result = {
            let mut s = lock(&self.session);
            if s.doc().id() != doc_id {
                None
            } else {
                Some(s.execute(command, params.clone(), Origin::User))
            }
        };
        match result {
            None => {
                self.toast(Toast::warning(
                    "Another document became active, so nothing was changed.",
                ));
                None
            }
            Some(Err(e)) => {
                self.toast(Toast::error(person_text(&e)));
                None
            }
            Some(Ok(v)) => {
                self.after_command(command, &params, &v);
                Some(v)
            }
        }
    }

    /// Updates what Prepare shows after a command ran.
    fn after_command(&mut self, command: &str, params: &Value, result: &Value) {
        let ids = |v: &Value| -> Vec<u64> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect()
        };
        let warnings: Vec<String> = result["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w.as_str().map(str::to_string))
            .collect();
        let p = &mut self.ui.prepare;
        match command {
            "object.import" => p.notices = warnings,
            "object.reload" => {
                let done: Vec<u64> = [&result["reloaded"], &result["unchanged"]]
                    .into_iter()
                    .flat_map(ids)
                    .collect();
                p.sources.changed.retain(|i| !done.contains(i));
                for id in ids(&result["missing"]) {
                    p.sources.changed.retain(|i| *i != id);
                    if !p.sources.missing.contains(&id) {
                        p.sources.missing.push(id);
                    }
                }
                for w in warnings {
                    self.toast(Toast::warning(w));
                }
            }
            "object.relink" => {
                if let Some(id) = params["id"].as_u64() {
                    p.sources.forget(&[id]);
                }
                p.sources.also_found = result["also_found"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| Some((f["id"].as_u64()?, PathBuf::from(f["path"].as_str()?))))
                    .collect();
                for w in warnings {
                    self.toast(Toast::warning(w));
                }
            }
            "job.send" => self.sent(result),
            "object.unlink" => p.sources.forget(&ids(&result["ids"])),
            "object.delete" => p.sources.forget(&ids(&result["deleted"])),
            _ => {}
        }
    }

    fn apply_one(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Run {
                doc_id,
                command,
                params,
            } => {
                self.run_on(doc_id, command, params);
            }
            Action::Import { doc_id } => {
                if let Some(path) = self.files.pick_file(&import_dialog::import_request()) {
                    self.run_on(doc_id, "object.import", json!({ "path": path }));
                }
            }
            Action::TraceDialog { doc_id, id } => self.open_trace(ctx, doc_id, id),
            Action::Dropped(paths) => {
                for path in paths {
                    match import_dialog::drop_kind(&path) {
                        import_dialog::DropKind::Project => self.apply_one(ctx, Action::Open(path)),
                        import_dialog::DropKind::Import => {
                            // The active document now: an earlier file of
                            // the same drop may have opened a project.
                            let doc_id = lock(&self.session).doc().id();
                            if self
                                .run_on(doc_id, "object.import", json!({ "path": path }))
                                .is_some()
                            {
                                self.ui.workspace = Workspace::Prepare;
                            }
                        }
                        import_dialog::DropKind::Unsupported => {
                            self.toast(Toast::warning(import_dialog::unsupported_text(&path)));
                        }
                    }
                }
            }
            Action::Reconnect { doc_id, id } => {
                let raster = {
                    let s = lock(&self.session);
                    s.doc_index(doc_id).ok().and_then(|i| {
                        s.docs[i]
                            .document
                            .get(flatcraft_core::document::ObjectId(id))
                            .map(|o| {
                                matches!(o.source, flatcraft_core::document::Source::Raster { .. })
                            })
                    })
                };
                let Some(raster) = raster else {
                    self.toast(Toast::error("That object is gone."));
                    return;
                };
                use flatcraft_engine::commands::sources::FileKind;
                let filters = if raster {
                    import_dialog::import_filters("Image", Some(FileKind::Raster))
                } else {
                    import_dialog::import_filters("SVG", Some(FileKind::Svg))
                };
                let req = files::FileRequest {
                    title: "Reconnect file".into(),
                    filters,
                    ..Default::default()
                };
                if let Some(path) = self.files.pick_file(&req) {
                    self.run_on(doc_id, "object.relink", json!({ "id": id, "path": path }));
                }
            }
            Action::ReconnectAll { doc_id, items } => {
                for (id, path) in items {
                    self.run_on(doc_id, "object.relink", json!({ "id": id, "path": path }));
                }
                self.ui.prepare.sources.also_found.clear();
            }
            Action::CheckSources => self.ui.prepare.sources.check_pending = true,
            Action::Save { doc_id } => {
                let path = {
                    let s = lock(&self.session);
                    s.doc_index(doc_id).map(|i| s.docs[i].path.is_some())
                };
                match path {
                    Ok(true) => {
                        if let Some(v) = self.execute("project.save", json!({ "doc_id": doc_id })) {
                            self.saved(&v);
                        }
                    }
                    Ok(false) => self.apply_one(ctx, Action::SaveAs { doc_id }),
                    Err(e) => self.toast(Toast::error(person_text(&e))),
                }
            }
            Action::SaveAs { doc_id } => {
                // The dialog blocks without the lock; an agent may change the
                // active document meanwhile, so the save names `doc_id`.
                let (name, dir) = self.save_as_suggestion(doc_id);
                let req = files::FileRequest {
                    title: "Save As".into(),
                    filters: vec![files::filter("FlatCraft project", &[PROJECT_EXTENSION])],
                    file_name: Some(name),
                    directory: dir,
                };
                if let Some(path) = self.files.save_file(&req) {
                    self.apply_one(ctx, Action::SaveTo { doc_id, path });
                }
            }
            Action::SaveTo { doc_id, path } => {
                if let Some(v) =
                    self.execute("project.save", json!({ "doc_id": doc_id, "path": path }))
                {
                    self.saved(&v);
                }
            }
            Action::OpenDialog => {
                let req = files::FileRequest {
                    title: "Open".into(),
                    filters: vec![files::filter("FlatCraft project", &[PROJECT_EXTENSION])],
                    ..Default::default()
                };
                if let Some(path) = self.files.pick_file(&req) {
                    self.apply_one(ctx, Action::Open(path));
                }
            }
            Action::Open(path) => {
                if let Some(v) = self.execute("project.open", json!({ "path": path })) {
                    self.remember_recent(&path);
                    for w in v["warnings"].as_array().into_iter().flatten() {
                        if let Some(w) = w.as_str() {
                            self.toast(Toast::warning(w.to_string()));
                        }
                    }
                    self.ui.workspace = Workspace::Prepare;
                    self.ui.prepare.sources.check_pending = true;
                }
            }
            Action::NewDocument => {
                self.execute("document.new", json!({}));
            }
            Action::NewProject => {
                // Checked and acted on under one lock.
                let result = {
                    let mut s = lock(&self.session);
                    if s.doc().is_pristine() {
                        Ok(Value::Null)
                    } else {
                        s.execute("document.new", json!({}), Origin::User)
                    }
                };
                match result {
                    Ok(_) => self.ui.workspace = Workspace::Prepare,
                    Err(e) => self.toast(Toast::error(person_text(&e))),
                }
            }
            Action::Switch(doc_id) => {
                self.execute("document.switch", json!({ "doc_id": doc_id }));
            }
            Action::Close(doc_id) => {
                let result = lock(&self.session).execute(
                    "document.close",
                    json!({ "doc_id": doc_id }),
                    Origin::User,
                );
                match result {
                    Ok(_) => {}
                    // The unsaved-changes dialog replaces this toast later.
                    Err(CommandError::Refused(_)) => self.toast(Toast::error(
                        "This document has unsaved changes. Save it first.",
                    )),
                    Err(e) => self.toast(Toast::error(person_text(&e))),
                }
            }
            Action::Undo(doc_id) => {
                self.execute("edit.undo", json!({ "doc_id": doc_id }));
            }
            Action::Redo(doc_id) => {
                self.execute("edit.redo", json!({ "doc_id": doc_id }));
            }
            Action::SetWorkspace(w) => self.ui.workspace = w,
            Action::Send { doc_id } => {
                self.run_on(doc_id, "job.send", json!({}));
            }
            Action::ConfirmPending(id) => {
                let now = (self.clock)();
                let result = lock(&self.session).confirm_pending(id, now);
                match result {
                    Ok(v) => self.sent(&v),
                    Err(e) => self.toast(Toast::error(person_text(&e))),
                }
            }
            Action::DenyPending(id) => {
                if lock(&self.session).deny_pending(id) {
                    self.toast(Toast::info("Cut request denied. Nothing was sent."));
                }
            }
            Action::Reveal(file) => {
                if let Err(e) = self.revealer.reveal(&file) {
                    self.toast(Toast::error(format!(
                        "Cannot open the folder of {}: {e}",
                        display_name(&file)
                    )));
                }
            }
            Action::OpenCapture => {
                let req = files::FileRequest {
                    title: "Open capture".into(),
                    filters: vec![files::filter("Capture fixture", &["jsonl"])],
                    ..Default::default()
                };
                if let Some(path) = self.files.pick_file(&req) {
                    match debug::read_capture(&path) {
                        Ok(c) => {
                            self.ui.debug.capture = Some(c);
                            self.ui.debug.tab = debug::DebugTab::Capture;
                        }
                        Err(e) => self.toast(Toast::error(format!(
                            "Cannot read the capture {}: {e}",
                            display_name(&path)
                        ))),
                    }
                }
            }
            Action::ShowProfiles => self.ui.show_profiles = true,
            Action::Help => ctx.open_url(egui::OpenUrl::new_tab(HELP_URL)),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Copies what the Trace dialog needs under one lock and opens it.
    fn open_trace(&mut self, ctx: &egui::Context, doc_id: u64, id: u64) {
        let src = {
            let s = lock(&self.session);
            if s.doc().id() != doc_id {
                None
            } else {
                s.doc()
                    .document
                    .get(flatcraft_core::document::ObjectId(id))
                    .and_then(|o| match &o.source {
                        flatcraft_core::document::Source::Raster { bytes, trace, .. } => {
                            Some(import_dialog::TraceSource {
                                doc_id,
                                id,
                                name: o.name.clone(),
                                bytes: bytes.clone(),
                                settings: trace.clone(),
                                rev: import_dialog::SourceRev::of(bytes, trace, &o.geometry),
                                blade_offset_mm:
                                    flatcraft_engine::commands::object::blade_offset_mm(&s),
                            })
                        }
                        _ => None,
                    })
            }
        };
        match src {
            Some(src) => {
                self.ui.trace = Some(import_dialog::TraceDialog::open(
                    ctx,
                    src,
                    self.tracer.clone(),
                ));
            }
            None => self.toast(Toast::warning(
                "That image is gone, so there is nothing to trace.",
            )),
        }
    }

    /// The file name and folder the Save As dialog suggests: the
    /// document's own file if it has one, else "Untitled N.flatcraft".
    pub fn save_as_suggestion(&self, doc_id: u64) -> (String, Option<PathBuf>) {
        let current = {
            let s = lock(&self.session);
            s.doc_index(doc_id)
                .ok()
                .and_then(|i| s.docs[i].path.clone())
        };
        match (current, self.ui.untitled.get(&doc_id)) {
            (Some(path), _) => (display_name(&path), path.parent().map(Path::to_path_buf)),
            (None, Some(n)) => (format!("Untitled {n}.{PROJECT_EXTENSION}"), None),
            (None, None) => (format!("Untitled.{PROJECT_EXTENSION}"), None),
        }
    }

    /// The toast of a finished send: the written files and a link that
    /// shows them in the file manager.
    fn sent(&mut self, result: &Value) {
        let files: Vec<PathBuf> = result["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str().map(PathBuf::from))
            .collect();
        let Some(first) = files.first().cloned() else {
            self.toast(Toast::info("Sent. Ship it!"));
            return;
        };
        let names: Vec<String> = files.iter().map(|f| display_name(f)).collect();
        let dir = first
            .parent()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        self.toast(
            Toast::info(format!("Sent! Wrote {} to {dir}", names.join(", "))).with_reveal(first),
        );
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

/// The text of a command error for a person. Missing things and refusals
/// read without their "Not found: " or "Refused: " prefix, so a refusal
/// shows as "USB cutting arrives in milestone 3."
pub fn person_text(e: &CommandError) -> String {
    match e {
        CommandError::NotFound(m) | CommandError::Refused(m) => m.clone(),
        other => other.to_string(),
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
fn shortcuts(ctx: &egui::Context, active: u64, actions: &mut Vec<Action>) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
    let cmd_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
    ctx.input_mut(|i| {
        if i.consume_shortcut(&cmd_shift(Key::S)) {
            actions.push(Action::SaveAs { doc_id: active });
        }
        if i.consume_shortcut(&cmd(Key::S)) {
            actions.push(Action::Save { doc_id: active });
        }
        if i.consume_shortcut(&cmd(Key::O)) {
            actions.push(Action::OpenDialog);
        }
        if i.consume_shortcut(&cmd(Key::N)) {
            actions.push(Action::NewProject);
        }
        if !cfg!(target_os = "macos") {
            if i.consume_shortcut(&cmd_shift(Key::Z)) || i.consume_shortcut(&cmd(Key::Y)) {
                actions.push(Action::Redo(active));
            }
            if i.consume_shortcut(&cmd(Key::Z)) {
                actions.push(Action::Undo(active));
            }
        }
    });
}

/// Draws the whole window into `ui` and runs what the person asked for.
pub fn shell(ui: &mut egui::Ui, app: &mut FlatCraftApp) {
    let ctx = ui.ctx().clone();
    let mut actions = Vec::new();
    let (active, open) = {
        let s = lock(&app.session);
        let open: Vec<u64> = s.docs.iter().map(|d| d.id()).collect();
        (s.doc().id(), open)
    };
    app.ui.number_documents(&open);
    shortcuts(&ctx, active, &mut actions);
    dropped_files(&ctx, &app.tokens, &mut actions);
    let focused = ctx.input(|i| {
        i.raw
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::WindowFocused(true)))
    });
    if focused {
        app.ui.prepare.sources.check_pending = true;
    }

    let workspace = app.ui.workspace;
    let wants_job = workspace == Workspace::Cut || app.ui.show_debug;
    let snap = (workspace != Workspace::Home).then(|| prepare::snapshot_with(app, wants_job));
    if let Some(snap) = &snap {
        app.ui.cut.update(snap);
    }
    if app.ui.show_debug && workspace != Workspace::Home {
        app.ui.debug.refresh_log(&app.session);
        app.ui.debug.refresh_last_sent(&app.session);
    }
    app_bar::show(ui, app, &mut actions);
    switcher::ribbon(ui, app, snap.as_ref(), &mut actions);
    if workspace != Workspace::Prepare {
        // The Trace dialog lives in Prepare.
        app.ui.trace = None;
    }
    match (&snap, workspace) {
        (Some(snap), Workspace::Cut) => cut::show(ui, app, snap, &mut actions),
        (Some(snap), _) => {
            // The Trace dialog owns Escape, and Delete must not remove
            // the image it traces.
            if app.ui.trace.is_none() {
                prepare_keys(&ctx, snap, &mut actions);
            }
            prepare::show(ui, app, snap, &mut actions);
            let t = app.tokens;
            if let Some(why) = import_dialog::show(&ctx, &mut app.ui.trace, snap, &t, &mut actions)
            {
                app.toast(Toast::info(why));
            }
        }
        (None, _) => {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(app.tokens.bg))
                .show(ui, |ui| home::show(ui, app, &mut actions));
        }
    }
    if app.ui.show_debug && workspace != Workspace::Home {
        debug::show(&ctx, app, &mut actions);
    }
    pending_modal(&ctx, app, &mut actions);

    app.apply(&ctx, actions);

    let now = Instant::now();
    app.check_sources(&ctx, now);
    if let Some(next) = toasts::prune(&mut app.ui.toasts, now) {
        ctx.request_repaint_after(next);
    }
    if let Some(file) = toasts::show(&ctx, &app.ui.toasts, &app.tokens) {
        app.apply(&ctx, vec![Action::Reveal(file)]);
    }
    if app.agent {
        // Keeps the agent pill current.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

/// Files dropped on the window this frame become one action; while files
/// hover over the window, a hint says what dropping does.
fn dropped_files(ctx: &egui::Context, t: &Tokens, actions: &mut Vec<Action>) {
    let (dropped, hovering) = ctx.input(|i| {
        let dropped: Vec<PathBuf> = i
            .raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect();
        (dropped, !i.raw.hovered_files.is_empty())
    });
    if !dropped.is_empty() {
        actions.push(Action::Dropped(dropped));
    }
    if hovering {
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("flatcraft-drop-hint"),
        ));
        let rect = ctx.content_rect();
        painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(140));
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop it! SVGs and images are imported, projects open.",
            fonts::medium(20.0),
            t.text,
        );
    }
}

/// Delete removes the selection and Escape clears it, unless a text field
/// has the keyboard or a menu is open (Escape closes that first).
fn prepare_keys(ctx: &egui::Context, snap: &prepare::Snapshot, actions: &mut Vec<Action>) {
    if ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) || snap.selection.is_empty()
    {
        return;
    }
    let (delete, escape) = ctx.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
        )
    });
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    if delete {
        actions.push(Action::run(
            snap.doc_id,
            "object.delete",
            json!({ "ids": ids }),
        ));
    } else if escape {
        actions.push(Action::run(
            snap.doc_id,
            "object.select",
            json!({ "ids": [] }),
        ));
    }
}

/// What the confirmation dialog shows of an agent's pending send.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingView {
    pub id: u64,
    pub plotter: String,
    pub summary: JobSummary,
    /// Time left until it expires.
    pub left: Duration,
}

/// Expires an old pending send and copies what the dialog shows, under
/// one lock. Only the summary is copied, never the job bytes.
fn pending_view(app: &mut FlatCraftApp) -> Option<PendingView> {
    let now = (app.clock)();
    let (expired, view) = {
        let mut s = lock(&app.session);
        let expired = s.expire_pending(now);
        let view = s.pending_send().map(|p| PendingView {
            id: p.id,
            plotter: s
                .profiles
                .plotter(&p.plotter_id)
                .map_or_else(|_| p.plotter_id.clone(), |pr| pr.name),
            summary: p.summary.clone(),
            left: PENDING_TTL.saturating_sub(now.saturating_duration_since(p.requested_at)),
        });
        (expired, view)
    };
    if expired {
        app.toast(Toast::warning(
            "The agent's cut request expired after 5 minutes. Nothing was sent.",
        ));
    }
    view
}

/// "An agent wants to start a cut on <plotter>. Allow?" in every
/// workspace. Allow confirms exactly the request shown (its id).
fn pending_modal(ctx: &egui::Context, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let Some(p) = pending_view(app) else {
        return;
    };
    ctx.request_repaint_after(p.left);
    let t = app.tokens;
    egui::Modal::new(egui::Id::unique("flatcraft-agent-send")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(
            egui::RichText::new(format!(
                "An agent wants to start a cut on {}. Allow?",
                p.plotter
            ))
            .font(fonts::semibold(15.0)),
        );
        ui.add_space(t.spacing);
        let s = &p.summary;
        let paths = if s.toolpaths == 1 {
            "1 path".to_string()
        } else {
            format!("{} paths", s.toolpaths)
        };
        ui.label(format!("{paths}, {} cut", cut::mm_text(s.cut_mm)));
        let passes = if s.passes == 1 {
            "1 pass".to_string()
        } else {
            format!("{} passes", s.passes)
        };
        ui.label(
            egui::RichText::new(format!("Speed {}, force {}, {passes}", s.speed, s.force))
                .color(t.text_dim),
        );
        for w in &s.warnings {
            ui.label(egui::RichText::new(w).color(t.accent_cut));
        }
        ui.label(egui::RichText::new("The request expires after 5 minutes.").color(t.text_dim));
        ui.add_space(t.spacing);
        ui.horizontal(|ui| {
            if ui.button("Allow").clicked() {
                actions.push(Action::ConfirmPending(p.id));
            }
            if ui.button("Deny").clicked() {
                actions.push(Action::DenyPending(p.id));
            }
        });
    });
}

impl eframe::App for FlatCraftApp {
    #[cfg(target_os = "macos")]
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let active = lock(&self.session).doc().id();
        let actions = self.menu.as_ref().map(|m| m.actions(active));
        if let Some(actions) = actions {
            self.apply(ctx, actions);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        shell(ui, self);
    }
}
