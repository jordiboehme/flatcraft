//! The app shell: app bar, workspace switcher, Home and toasts. The state and
//! the drawing do not depend on eframe, so tests drive [`shell`] directly.

pub mod about;
pub mod app_bar;
pub mod arrange;
pub mod canvas;
pub mod context_menu;
pub mod cut;
pub mod debug;
pub mod files;
pub mod fonts;
pub mod help;
pub mod home;
pub mod icons;
pub mod import_dialog;
#[cfg(target_os = "macos")]
pub mod menu;
pub mod objects;
pub mod prepare;
pub mod profiles_window;
pub mod ribbon;
pub mod rng;
pub mod settings;
pub mod switcher;
pub mod theme;
pub mod timeline;
pub mod toasts;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use flatcraft_engine::prefs::Theme;
use flatcraft_engine::session::{CommandError, JobSummary, Origin, PENDING_TTL, Session};
use flatcraft_mcp::SharedSession;
use serde_json::{Value, json};

use icons::Icon;
use theme::Tokens;
pub use toasts::Toast;

/// The toast of Undo with nothing to undo.
pub const NOTHING_TO_UNDO: &str = "Great Scott! Nothing to undo.";

/// The toast of about one successful send in [`PUNCH_IT_ODDS`].
pub const PUNCH_IT: &str = "Punch it.";
pub const PUNCH_IT_ODDS: usize = 5;

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
    /// The keyboard shortcuts sheet is open.
    pub show_shortcuts: bool,
    pub profiles: profiles_window::ProfilesState,
    pub prepare: prepare::PrepareState,
    /// The number of each untitled document's name, by stable document id,
    /// so "Untitled 3" keeps its name when "Untitled 2" closes.
    pub untitled: BTreeMap<u64, usize>,
    /// The open Trace dialog.
    pub trace: Option<import_dialog::TraceDialog>,
    pub cut: cut::CutState,
    pub debug: debug::DebugState,
    /// The open unsaved-changes dialog.
    pub unsaved: Option<Unsaved>,
    /// The person chose to quit (nothing unsaved, saved or discarded): the
    /// window closes and is not asked again.
    pub closing: bool,
    pub about: about::AboutState,
}

/// What the unsaved-changes dialog asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsaved {
    /// Quitting while these documents have unsaved changes.
    Quit(Vec<u64>),
    /// Closing this document with unsaved changes.
    Close(u64),
}

/// The answer in the unsaved-changes dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedChoice {
    /// Save (asks for a path for an untitled document), then go on.
    Save,
    /// Go on and lose the changes.
    Discard,
    /// Stay as it is.
    Cancel,
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
    /// Store the "Each object" switch of the Arrange tab in the
    /// preferences, so the next start has it as it was.
    RememberEach(bool),
    /// Pin an Arrange command to the ribbon, or unpin it, in the
    /// preferences (`arrange_pins`, by [`arrange::ArrangeCmd::id`]).
    SetPin {
        id: &'static str,
        pinned: bool,
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
    /// Open the Profiles window (Settings button, Settings menu item, Cmd+,).
    ShowProfiles,
    /// A save, delete or preference change from the Profiles window.
    Profiles(profiles_window::Request),
    /// Open the About dialog (Help menu, macOS app menu).
    ShowAbout,
    /// Open the keyboard shortcuts sheet.
    ShowShortcuts,
    /// Open a link in the browser.
    OpenUrl(String),
    /// Quit; asks first when a document has unsaved changes.
    Quit,
    /// The answer in the unsaved-changes dialog.
    Unsaved(UnsavedChoice),
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
    /// egui's own theme preference is System (the preference is System).
    pub follows_system: bool,
    /// True for a window started by `flatcraft mcp --window`.
    pub agent: bool,
    /// File dialogs; tests replace them.
    pub files: Box<dyn files::FilePicker>,
    /// Traces the Trace dialog's previews; tests replace it.
    pub tracer: import_dialog::Tracer,
    /// Shows sent files in the file manager; tests replace it.
    pub revealer: Box<dyn files::Revealer>,
    /// The monotonic clock that pending sends, toasts, the source-check
    /// debounce and arrow-key bursts go by; tests replace it.
    pub clock: Clock,
    /// Runs a check of linked files on a worker thread; tests replace it.
    pub source_check: flatcraft_mcp::server::SourceChecker,
    /// The check of linked files that is running, if any.
    pub source_check_job: Option<std::sync::mpsc::Receiver<prepare::CheckDone>>,
    /// Rolls the maker quote and the rare "Punch it." toast. Seeded with
    /// [`rng::DEFAULT_SEED`]; `main` seeds it from the clock and tests with
    /// [`FlatCraftApp::with_seed`].
    pub rng: rng::XorShift,
    /// The macOS menu bar; it must live as long as the app.
    #[cfg(target_os = "macos")]
    menu: Option<menu::AppMenu>,
}

impl FlatCraftApp {
    /// Installs the fonts and the theme of the preferences on `ctx`.
    pub fn new(ctx: &egui::Context, session: SharedSession, config_dir: PathBuf) -> Self {
        let (theme, rotate_each) = {
            let s = lock(&session);
            (s.prefs.theme, s.prefs.rotate_each)
        };
        let tokens = Tokens::for_theme(theme, ctx.system_theme());
        fonts::install(ctx);
        theme::apply(ctx, &tokens, theme == Theme::System);
        let mut ui = UiState::default();
        ui.prepare.fields.each = rotate_each;
        Self {
            session,
            config_dir,
            ui,
            tokens,
            follows_system: theme == Theme::System,
            agent: false,
            files: Box::new(files::NativePicker),
            tracer: std::sync::Arc::new(flatcraft_import::trace::trace),
            revealer: Box::new(files::NativeRevealer),
            clock: std::sync::Arc::new(Instant::now),
            source_check: std::sync::Arc::new(flatcraft_engine::commands::sources::check_unlocked),
            source_check_job: None,
            rng: rng::XorShift::default(),
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

    /// Checks linked files with `f` (on the worker thread) instead of
    /// `sources::check_unlocked` (tests).
    pub fn with_source_check(
        mut self,
        f: impl Fn(
            &std::sync::Mutex<Session>,
        ) -> (flatcraft_engine::commands::sources::CheckResult, bool)
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.source_check = std::sync::Arc::new(f);
        self
    }

    /// Seeds the random rolls of the easter eggs.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = rng::XorShift::new(seed);
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

    /// Shows a toast. It ages by [`FlatCraftApp::clock`].
    pub fn toast(&mut self, mut toast: Toast) {
        toast.created = (self.clock)();
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
                Some(s.execute(command, params, Origin::User))
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
                self.after_command(command, &v);
                Some(v)
            }
        }
    }

    /// Updates what Prepare shows after a command ran.
    fn after_command(&mut self, command: &str, result: &Value) {
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
            "object.import" if result.get("imported").is_some() => {
                // Several files: their warnings, and one toast that names
                // the files that could not be imported.
                p.notices = warnings;
                let failed: Vec<String> = result["failed"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f["path"].as_str())
                    .map(|path| display_name(Path::new(path)))
                    .collect();
                if !failed.is_empty() {
                    let first = result["failed"][0]["reason"].as_str().unwrap_or_default();
                    self.toast(Toast::error(format!(
                        "Could not import {}. {}",
                        failed.join(", "),
                        if failed.len() == 1 {
                            first.to_string()
                        } else {
                            format!("The first one says: {first}")
                        }
                    )));
                }
            }
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
                let done: Vec<u64> = [&result["reloaded"], &result["unchanged"]]
                    .into_iter()
                    .flat_map(ids)
                    .collect();
                p.sources.forget(&done);
                p.sources.also_found = result["also_found"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| Some((f["id"].as_u64()?, PathBuf::from(f["path"].as_str()?))))
                    .collect();
                for w in warnings {
                    self.toast(Toast::warning(w));
                }
                for f in result["failed"].as_array().into_iter().flatten() {
                    if let Some(e) = f["reason"].as_str() {
                        self.toast(Toast::error(e.to_string()));
                    }
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
                // Files next to each other in the drop are imported as one
                // command (one undo step); a project in between opens and
                // the files after it go into the document active then.
                let mut files: Vec<PathBuf> = Vec::new();
                for path in paths {
                    match import_dialog::drop_kind(&path) {
                        import_dialog::DropKind::Project => {
                            self.import_dropped(std::mem::take(&mut files));
                            self.apply_one(ctx, Action::Open(path));
                        }
                        import_dialog::DropKind::Import => files.push(path),
                        import_dialog::DropKind::Unsupported => {
                            self.toast(Toast::warning(import_dialog::unsupported_text(&path)));
                        }
                    }
                }
                self.import_dropped(files);
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
                let items: Vec<Value> = items
                    .into_iter()
                    .map(|(id, path)| json!({ "id": id, "path": path }))
                    .collect();
                self.run_on(doc_id, "object.relink", json!({ "items": items }));
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
                let result = {
                    let mut s = lock(&self.session);
                    match s.doc_index(doc_id) {
                        Ok(i) if s.docs[i].dirty => None,
                        _ => Some(s.execute(
                            "document.close",
                            json!({ "doc_id": doc_id }),
                            Origin::User,
                        )),
                    }
                };
                match result {
                    None => self.ui.unsaved = Some(Unsaved::Close(doc_id)),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => self.toast(Toast::error(person_text(&e))),
                }
            }
            Action::Undo(doc_id) => {
                // An empty stack is no error (the macOS menu item and the
                // shortcut are always there).
                let nothing = {
                    let s = lock(&self.session);
                    s.doc_index(doc_id)
                        .is_ok_and(|i| !s.docs[i].undo.can_undo())
                };
                if nothing {
                    self.toast(Toast::info(NOTHING_TO_UNDO));
                } else {
                    self.execute("edit.undo", json!({ "doc_id": doc_id }));
                }
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
            Action::RememberEach(each) => {
                let result = lock(&self.session).update_prefs(|p| p.rotate_each = each);
                if let Err(e) = result {
                    self.toast(Toast::error(format!(
                        "Cannot store the Each object setting: {e}"
                    )));
                }
            }
            Action::SetPin { id, pinned } => {
                let result = lock(&self.session).update_prefs(|p| {
                    p.arrange_pins.retain(|x| x != id);
                    if pinned {
                        p.arrange_pins.push(id.to_string());
                    }
                });
                if let Err(e) = result {
                    self.toast(Toast::error(format!("Cannot store the pins: {e}")));
                }
            }
            Action::Profiles(req) => profiles_window::apply(self, ctx, req),
            Action::ShowAbout => self.ui.about.open(&mut self.rng),
            Action::ShowShortcuts => self.ui.show_shortcuts = true,
            Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Action::Quit => {
                let dirty = self.dirty_docs();
                if dirty.is_empty() {
                    self.close_window(ctx);
                } else {
                    self.ui.unsaved = Some(Unsaved::Quit(dirty));
                }
            }
            Action::Unsaved(choice) => self.answer_unsaved(ctx, choice),
        }
    }

    /// Stable ids of the documents with unsaved changes.
    pub fn dirty_docs(&self) -> Vec<u64> {
        let s = lock(&self.session);
        s.docs.iter().filter(|d| d.dirty).map(|d| d.id()).collect()
    }

    /// Closes the window without asking again.
    fn close_window(&mut self, ctx: &egui::Context) {
        self.ui.closing = true;
        self.ui.unsaved = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Runs the answer of the unsaved-changes dialog.
    fn answer_unsaved(&mut self, ctx: &egui::Context, choice: UnsavedChoice) {
        let Some(asked) = self.ui.unsaved.take() else {
            return;
        };
        match (asked, choice) {
            (_, UnsavedChoice::Cancel) => {}
            (Unsaved::Close(doc_id), UnsavedChoice::Discard) => {
                self.execute(
                    "document.close",
                    json!({ "doc_id": doc_id, "discard_changes": true }),
                );
            }
            (Unsaved::Close(doc_id), UnsavedChoice::Save) => {
                if self.save_for_close(doc_id) {
                    self.execute("document.close", json!({ "doc_id": doc_id }));
                }
            }
            (Unsaved::Quit(_), UnsavedChoice::Discard) => self.close_window(ctx),
            (Unsaved::Quit(_), UnsavedChoice::Save) => {
                // The documents dirty now: an agent may have changed more.
                for doc_id in self.dirty_docs() {
                    if !self.save_for_close(doc_id) {
                        self.toast(Toast::warning(
                            "Not everything was saved, so FlatCraft stays open.",
                        ));
                        return;
                    }
                }
                self.close_window(ctx);
            }
        }
    }

    /// Saves a document before it closes, asking for a path when it has
    /// none. False when the person cancelled the dialog or the save failed
    /// (a toast says why). A document that is gone counts as done.
    fn save_for_close(&mut self, doc_id: u64) -> bool {
        let has_path = {
            let s = lock(&self.session);
            match s.doc_index(doc_id) {
                Ok(i) => s.docs[i].path.is_some(),
                Err(_) => return true,
            }
        };
        let params = if has_path {
            json!({ "doc_id": doc_id })
        } else {
            // The dialog blocks without the lock.
            let (name, dir) = self.save_as_suggestion(doc_id);
            let req = files::FileRequest {
                title: format!("Save {}", self.doc_name(doc_id)),
                filters: vec![files::filter("FlatCraft project", &[PROJECT_EXTENSION])],
                file_name: Some(name),
                directory: dir,
            };
            match self.files.save_file(&req) {
                Some(path) => json!({ "doc_id": doc_id, "path": path }),
                None => return false,
            }
        };
        match self.execute("project.save", params) {
            Some(v) => {
                self.saved(&v);
                true
            }
            None => false,
        }
    }

    /// The name a tab shows for a document: its file name, or "Untitled N".
    pub fn doc_name(&self, doc_id: u64) -> String {
        let path = {
            let s = lock(&self.session);
            s.doc_index(doc_id)
                .ok()
                .and_then(|i| s.docs[i].path.clone())
        };
        path.as_deref().map(display_name).unwrap_or_else(|| {
            let n = self.ui.untitled.get(&doc_id).copied().unwrap_or(1);
            format!("Untitled {n}")
        })
    }

    /// Imports dropped files into the active document, as one command.
    fn import_dropped(&mut self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        // The active document now: an earlier file of the same drop may
        // have opened a project.
        let doc_id = lock(&self.session).doc().id();
        let params = match files.as_slice() {
            [one] => json!({ "path": one }),
            _ => json!({ "paths": files }),
        };
        if self.run_on(doc_id, "object.import", params).is_some() {
            self.ui.workspace = Workspace::Prepare;
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
        // About one send in five, the toast quotes Star Wars instead.
        let punch = self.rng.one_in(PUNCH_IT_ODDS);
        let Some(first) = files.first().cloned() else {
            self.toast(Toast::info(if punch { PUNCH_IT } else { "Sent. Ship it!" }));
            return;
        };
        if punch {
            self.toast(Toast::info(PUNCH_IT).with_reveal(first));
            return;
        }
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

    /// Puts the project first in the recent list and stores it. Only the
    /// recent list changes: the file is read again first, so a preference
    /// another FlatCraft process stored meanwhile is kept.
    fn remember_recent(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let result = lock(&self.session).update_prefs(|p| {
            let recent = &mut p.recent_projects;
            recent.retain(|p| p != &path);
            recent.insert(0, path);
            recent.truncate(MAX_RECENT);
        });
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
    icon_button_hint(ui, icon, name, enabled, name)
}

/// An icon-only button with the accessible name `name` and the tooltip
/// `hint`.
pub fn icon_button_hint(
    ui: &mut egui::Ui,
    icon: Icon,
    name: &str,
    enabled: bool,
    hint: &str,
) -> egui::Response {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ))
    .alt_text(name);
    ui.add_enabled(enabled, egui::Button::image(image))
        .on_hover_text(hint)
        .on_disabled_hover_text(hint)
}

/// Keyboard shortcuts: the table in [`help::SHORTCUTS`]. On macOS the menu
/// bar owns Undo, Redo and Settings.
fn shortcuts(ctx: &egui::Context, active: u64, actions: &mut Vec<Action>) {
    help::run_shortcuts(ctx, active, actions);
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
    guard_close(&ctx, app);
    refresh_prefs(&ctx, app);
    // A dialog keeps the keys: Escape closes or cancels it, and Delete or
    // Cmd+Z must not change the document behind it.
    let modal = modal_open(app);
    if !modal {
        shortcuts(&ctx, active, &mut actions);
    }
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
    let snap = (workspace != Workspace::Home).then(|| prepare::snapshot_with(app, &ctx, wants_job));
    if let Some(snap) = &snap {
        app.ui.cut.update(snap);
    }
    // The speed joke ends once its clamped value is no longer the speed
    // (reset, undo, an agent, another plotter, material or document). This
    // runs before the widgets, so an attempt made last frame has had its
    // command applied.
    let eleven_gone = match (&app.ui.prepare.speed_eleven, &snap) {
        (Some(e), Some(snap)) => !e.current(snap),
        (Some(_), None) => true,
        (None, _) => false,
    };
    if eleven_gone {
        app.ui.prepare.speed_eleven = None;
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
            // A dialog owns Escape, and Delete must not remove what is
            // behind it (for the Trace dialog, the image it traces).
            if !modal {
                let now = (app.clock)();
                prepare_keys(&ctx, snap, &mut app.ui.prepare, now, &mut actions);
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
    if app.ui.show_profiles {
        profiles_window::show(&ctx, app, &mut actions);
    }
    let now = (app.clock)();
    let t = app.tokens;
    about::show(&ctx, &mut app.ui.about, &t, now, &mut actions);
    if app.ui.show_shortcuts {
        app.ui.show_shortcuts = help::show_shortcuts(&ctx, &t);
    }
    pending_modal(&ctx, app, &mut actions);
    unsaved_modal(&ctx, app, &mut actions);

    app.apply(&ctx, actions);

    let now = (app.clock)();
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

impl FlatCraftApp {
    /// The menu bar's actions that may run now. The menu lives outside
    /// egui, so a dialog does not block it: while one is open (see
    /// [`modal_open`]) only Quit goes through, which asks about unsaved
    /// changes itself.
    pub fn menu_gate(&self, actions: Vec<Action>) -> Vec<Action> {
        if !modal_open(self) {
            return actions;
        }
        actions
            .into_iter()
            .filter(|a| matches!(a, Action::Quit))
            .collect()
    }
}

/// A dialog that owns the keyboard is open: About, Shortcuts, Trace, Rename, unsaved changes
/// or the agent's pending send. The docked Arrange panels are not dialogs:
/// the document stays usable behind them.
pub fn modal_open(app: &FlatCraftApp) -> bool {
    app.ui.about.open
        || app.ui.show_shortcuts
        || app.ui.trace.is_some()
        || app.ui.prepare.rename.is_some()
        || app.ui.unsaved.is_some()
        || lock(&app.session).pending_send().is_some()
}

/// Keeps the window open while documents have unsaved changes: a close
/// request (the window's close button, Cmd+Q) is cancelled and the
/// unsaved-changes dialog asks. Once the person chose to quit
/// (`UiState::closing`), the close goes ahead. Runs in `logic` (also while
/// the window is hidden) and in [`shell`].
pub fn guard_close(ctx: &egui::Context, app: &mut FlatCraftApp) {
    if app.ui.closing || !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    let dirty = app.dirty_docs();
    if dirty.is_empty() {
        return;
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    if !matches!(app.ui.unsaved, Some(Unsaved::Quit(_))) {
        app.ui.unsaved = Some(Unsaved::Quit(dirty));
    }
}

/// The unsaved-changes dialog: Save (all) / Discard / Cancel.
fn unsaved_modal(ctx: &egui::Context, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let Some(asked) = app.ui.unsaved.clone() else {
        return;
    };
    let (text, save) = match &asked {
        Unsaved::Quit(docs) if docs.len() == 1 => (
            "You have unsaved changes in 1 document.".to_string(),
            "Save all",
        ),
        Unsaved::Quit(docs) => (
            format!("You have unsaved changes in {} documents.", docs.len()),
            "Save all",
        ),
        Unsaved::Close(doc_id) => (
            format!("You have unsaved changes in {}.", app.doc_name(*doc_id)),
            "Save",
        ),
    };
    let t = app.tokens;
    let modal = egui::Modal::new(egui::Id::unique("flatcraft-unsaved")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(text).font(fonts::semibold(15.0)));
        ui.add_space(t.spacing);
        ui.label(
            egui::RichText::new("Discard drops them for good. No undo past this point.")
                .color(t.text_dim),
        );
        ui.add_space(t.spacing);
        ui.horizontal(|ui| {
            if ui.button(save).clicked() {
                actions.push(Action::Unsaved(UnsavedChoice::Save));
            }
            if ui.button("Discard").clicked() {
                actions.push(Action::Unsaved(UnsavedChoice::Discard));
            }
            if ui.button("Cancel").clicked() {
                actions.push(Action::Unsaved(UnsavedChoice::Cancel));
            }
        });
    });
    if modal.should_close() && actions.iter().all(|a| !matches!(a, Action::Unsaved(_))) {
        actions.push(Action::Unsaved(UnsavedChoice::Cancel));
    }
}

/// Picks up preferences another FlatCraft process stored (at most once a
/// second, see `Session::refresh_prefs`), applies the theme of the
/// session's preferences when the colours differ (also after
/// `Session::update_prefs` read back another window's theme) and shows a
/// warning about a broken preferences file as a toast.
fn refresh_prefs(ctx: &egui::Context, app: &mut FlatCraftApp) {
    let now = (app.clock)();
    let (theme, warning) = {
        let mut s = lock(&app.session);
        s.refresh_prefs(now);
        (s.prefs.theme, s.take_prefs_warning())
    };
    // Every frame: with System the egui input carries the live appearance
    // of the operating system, and the colours follow it.
    let tokens = Tokens::for_theme(theme, ctx.system_theme());
    let follow = theme == Theme::System;
    if tokens != app.tokens || follow != app.follows_system {
        app.tokens = tokens;
        app.follows_system = follow;
        theme::apply(ctx, &app.tokens, follow);
    }
    if let Some(w) = warning {
        app.toast(Toast::warning(w));
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

/// Prepare's keys, unless a text field (or another widget) has the
/// keyboard or a menu is open (Escape closes that first): Cmd+A (Ctrl+A)
/// selects every visible object, Cmd+I imports, Cmd+D duplicates the
/// selection, the arrow keys nudge it (see [`canvas::nudge_keys`]), Delete
/// removes it and Escape clears it (or closes an open Arrange panel). The
/// keys come from [`help::SHORTCUTS`].
fn prepare_keys(
    ctx: &egui::Context,
    snap: &prepare::Snapshot,
    st: &mut prepare::PrepareState,
    now: Instant,
    actions: &mut Vec<Action>,
) {
    if ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) {
        return;
    }
    // Escape closes an open Arrange panel first, and changes nothing. One
    // press is enough while a panel field has the keys: egui drops the
    // focus on Escape before this runs.
    if st.panel.is_some() && help::pressed(ctx, help::PrepareKey::ClearSelection) {
        st.panel = None;
        return;
    }
    let select_all = help::pressed(ctx, help::PrepareKey::SelectAll);
    if select_all {
        actions.push(context_menu::select_all_action(snap));
        return;
    }
    if help::pressed(ctx, help::PrepareKey::Import) {
        actions.push(Action::Import {
            doc_id: snap.doc_id,
        });
        return;
    }
    if snap.selection.is_empty() {
        return;
    }
    let ids: Vec<u64> = snap.selection.iter().copied().collect();
    if help::pressed(ctx, help::PrepareKey::Duplicate) {
        actions.push(ribbon::duplicate_action(snap.doc_id, &ids));
        return;
    }
    canvas::nudge_keys(ctx, snap, st, now, actions);
    let delete = help::pressed(ctx, help::PrepareKey::Delete);
    let escape = help::pressed(ctx, help::PrepareKey::ClearSelection);
    if delete {
        actions.push(ribbon::delete_action(snap.doc_id, &ids));
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
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            let active = lock(&self.session).doc().id();
            let actions = self.menu.as_ref().map(|m| m.actions(active));
            if let Some(actions) = actions {
                let actions = self.menu_gate(actions);
                self.apply(ctx, actions);
            }
        }
        guard_close(ctx, self);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        shell(ui, self);
    }
}
