//! The app shell: app bar, workspace switcher, Home and toasts. The state and
//! the drawing do not depend on eframe, so tests drive [`shell`] directly.

pub mod about;
pub mod app_bar;
pub mod arrange;
pub mod calc;
pub mod canvas;
pub mod collapse;
pub mod context_menu;
pub mod cut;
pub mod debug;
pub mod dialog;
pub mod export_dialog;
pub mod files;
pub mod fonts;
pub mod groups;
pub mod help;
pub mod home;
pub mod icons;
pub mod import_dialog;
pub mod inspector;
pub mod keep_pick;
pub mod kept;
pub mod long;
pub mod materials;
#[cfg(target_os = "macos")]
pub mod menu;
pub mod menu_model;
pub mod objects;
pub mod outcome;
pub mod prepare;
pub mod profiles_window;
pub mod rails;
pub mod report;
pub mod ribbon;
pub mod rng;
pub mod settings;
pub mod settings_window;
pub mod simulate;
pub mod switcher;
pub mod theme;
pub mod timeline;
pub mod toasts;
pub mod tools;
pub mod troubleshoot;

use flatcraft_engine::commands::group::ReorderTo;
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

/// How many recent projects are kept. Home shows the newest
/// [`home::SHOWN_RECENT`] and the rest under "Show all".
pub const MAX_RECENT: usize = 24;

/// How many recent projects the macOS menu's Open Recent lists.
pub const MENU_RECENT: usize = 10;

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
    /// The Settings window is open.
    pub show_settings: bool,
    /// The keyboard shortcuts sheet is open.
    pub show_shortcuts: bool,
    pub profiles: profiles_window::ProfilesState,
    pub settings: settings_window::SettingsState,
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
    /// The open Export dialog.
    pub export: Option<export_dialog::ExportDialog>,
    /// The person chose to quit (nothing unsaved, saved or discarded): the
    /// window closes and is not asked again.
    pub closing: bool,
    pub about: about::AboutState,
    pub home: home::HomeState,
    /// Documents "Close other tabs" still has to ask about, after the one
    /// the unsaved-changes dialog asks about now.
    pub close_queue: Vec<u64>,
    /// The open "How did it go?" question about a cut.
    pub outcome: Option<outcome::OutcomeDialog>,
    /// The "It didn't cut right..." helper.
    pub troubleshoot: troubleshoot::TroubleshootState,
}

/// What the unsaved-changes dialog asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsaved {
    /// Quitting while documents have unsaved changes: the dialog asks
    /// about `asking`; `answered` are the documents already answered in
    /// this round, each with its revision right after the answer. Every
    /// document with unsaved changes that is not answered yet follows in
    /// turn, also one that became unsaved during the round, and one that
    /// changed again after its answer (its revision moved).
    Quit {
        asking: u64,
        answered: Vec<(u64, u64)>,
    },
    /// Closing this document with unsaved changes.
    Close(u64),
    /// Quitting while an agent waits for the person's OK to send a cut.
    PendingSend,
    /// Quitting while a long command of the window (an import, a trace,
    /// an export) still runs, with the documents this Quit round answered
    /// already (as in `Quit`).
    Busy { answered: Vec<(u64, u64)> },
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
    /// A registry command, as `Origin::User`, in the document it was made
    /// for even when another one became active meanwhile: a value typed
    /// into a Position and size field, which applies when the field is
    /// left (see `ribbon::finish_pending`).
    RunIn {
        doc_id: u64,
        command: &'static str,
        params: Value,
    },
    /// Store the "Each object" switch of the Transform group in the
    /// preferences, so the next start has it as it was.
    RememberEach(bool),
    /// Store what the person chose for the side panels (Objects, Cut and
    /// Job, Cut stage) in the preferences.
    RememberPanels([flatcraft_engine::prefs::PanelUser; 3]),
    /// Store the side panel sections opened or folded since the last time
    /// (merged into `Preferences::panel_sections`).
    RememberSections(std::collections::BTreeMap<String, bool>),
    /// Store tips the person hid (added to `Preferences::hidden_tips`).
    HideTips(std::collections::BTreeSet<String>),
    /// Store the spacing of the Auto arrange panel in the preferences, for
    /// the next panel and the one-click Auto arrange.
    RememberSpacing(f64),
    /// Pin a ribbon action, or unpin it, in the preferences
    /// (`ribbon_pins`, by [`tools::Tool::id`]).
    SetPin {
        id: &'static str,
        pinned: bool,
    },
    /// Select these objects and move the Prepare view to them (an
    /// issue's "Show me").
    ShowMe {
        doc_id: u64,
        ids: Vec<u64>,
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
    /// Open the Export dialog for this document, on the selection when
    /// there is one.
    ExportDialog {
        doc_id: u64,
    },
    /// Ask where to save, then export as the dialog says
    /// (`document.export`, as the person).
    Export(export_dialog::ExportDialog),
    /// Ask for a project file and open it.
    OpenDialog,
    /// Open this project file.
    Open(PathBuf),
    /// Home: take this project off the recent list (and its picture).
    /// Only the window does this; no registry command changes the list.
    RemoveRecent(PathBuf),
    /// Home: ask where a recent project that moved went, open it there and
    /// put it in the old one's place.
    LocateRecent(PathBuf),
    /// Home: open the sample with this index in [`home::SAMPLES`] as a new
    /// untitled document.
    OpenSample(usize),
    /// Home: switch to the open document with this id, in this workspace.
    ShowOpen(u64, Workspace),
    /// The "+" tab: always a new document.
    NewDocument,
    /// Home's New and Cmd+N: reuse the active document if it is pristine,
    /// else open a new one; then go to Prepare.
    NewProject,
    /// A card of Home's New row: the existing New, Import, Open or Test
    /// patterns action, on a new project where it needs one.
    StartWith(home::Start),
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
    /// Copy this path to the clipboard.
    CopyPath(PathBuf),
    /// Ask "How did it go?" about this cut of the cut log.
    AskOutcome(String),
    /// Open the "It didn't cut right..." helper (Cut workspace, Help
    /// menu). From Home it goes to Cut first.
    Troubleshoot,
    /// Record the person's answer about a cut in the cut log.
    RecordOutcome {
        entry: String,
        verdict: flatcraft_engine::cutlog::Verdict,
        note: String,
    },
    /// Show this error as a toast.
    Error(String),
    /// Close every document but this one; each one with unsaved changes
    /// asks first, one after the other.
    CloseOthers(u64),
    /// Ask for a capture fixture and show it in the Debug panel.
    OpenCapture,
    /// Open the Settings window (Settings button, Settings menu item, Cmd+,).
    ShowSettings,
    /// A preset save or delete, or a folder pick, from the Settings window.
    Profiles(profiles_window::Request),
    /// Open the About dialog (Help menu, macOS app menu).
    ShowAbout,
    /// Open the keyboard shortcuts sheet.
    ShowShortcuts,
    /// Open a link in the browser.
    OpenUrl(String),
    /// A macOS menu item that needs the snapshot or the keyboard focus.
    Menu(menu_model::Cmd),
    /// Quit; asks first when a document has unsaved changes.
    Quit,
    /// The answer in the unsaved-changes dialog, with the question it
    /// answers: an answer to a question no longer asked is ignored.
    Unsaved(Unsaved, UnsavedChoice),
}

impl Action {
    /// Whether the action saves, closes or quits, so a value still typed
    /// into a field must count first.
    fn needs_typed_values(&self) -> bool {
        matches!(
            self,
            Action::Save { .. }
                | Action::SaveAs { .. }
                | Action::ExportDialog { .. }
                | Action::SaveTo { .. }
                | Action::Close(_)
                | Action::CloseOthers(_)
                | Action::Quit
        )
    }

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
    /// The window's long commands (import, trace, reload, relink, open,
    /// export), run off the session lock on a worker, one at a time.
    pub long: long::LongJobs,
    /// The wait budget of the frame being drawn (see
    /// [`FlatCraftApp::frame_deadline`]); `None` between frames.
    pub frame_deadline: Option<Instant>,
    /// Rolls the maker quote and the rare "Punch it." toast. Seeded with
    /// [`rng::DEFAULT_SEED`]; `main` seeds it from the clock and tests with
    /// [`FlatCraftApp::with_seed`].
    pub rng: rng::XorShift,
    /// Key events the macOS menu bar passes on to the focused widget. It
    /// owns the Cmd keys, so a click on Undo, Select All and the like
    /// hands the key to the text field itself. They enter the next frame.
    pub injected: Vec<egui::Event>,
    /// The app bar is the window's title bar (macOS, see
    /// [`app_bar::viewport`]): it leaves room for the traffic lights and
    /// moves the window. Tests turn it on with [`Self::with_title_bar`].
    pub title_bar: bool,
    /// What a double-click on the title bar does; `None`: the system's
    /// setting. Tests set it with [`Self::with_double_click`].
    pub double_click: Option<app_bar::DoubleClick>,
    /// The wall clock Home's "2 hours ago" and the recent list go by;
    /// tests replace it.
    pub wall_clock: std::sync::Arc<dyn Fn() -> std::time::SystemTime + Send + Sync>,
    /// Draws the picture of a project file for Home (on a worker thread);
    /// tests replace it.
    pub thumbnailer: home::Thumbnailer,
    /// Home draws its pictures within the frame instead of on a worker
    /// thread. Tests turn it on, so no worker wakes the window at a random
    /// frame; see [`Self::with_inline_pictures`].
    pub inline_pictures: bool,
    /// Whether the system asks for reduced motion (the toasts then draw
    /// no countdown ring); tests replace it.
    pub reduce_motion: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
    /// The macOS menu bar; it must live as long as the app.
    #[cfg(target_os = "macos")]
    menu: Option<menu::AppMenu>,
}

impl FlatCraftApp {
    /// Installs the fonts and the theme of the preferences on `ctx`.
    pub fn new(ctx: &egui::Context, session: SharedSession, config_dir: PathBuf) -> Self {
        let (theme, rotate_each, panels, sections, tips) = {
            let s = lock(&session);
            (
                s.prefs.theme,
                s.prefs.rotate_each,
                rails::Panels::from_prefs(&s.prefs),
                s.prefs.panel_sections.clone(),
                s.prefs.hidden_tips.clone(),
            )
        };
        inspector::blocks::seed(ctx, sections);
        inspector::blocks::seed_tips(ctx, tips);
        let tokens = Tokens::for_theme(theme, ctx.system_theme());
        fonts::install(ctx);
        theme::apply(ctx, &tokens, theme == Theme::System);
        // A tool bar that collapses or expands discards the pass it measured
        // in (see `collapse`): up to two steps in one frame, so the person
        // never sees a half-collapsed bar.
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(3).expect("3 > 0"));
        let mut ui = UiState::default();
        ui.prepare.fields.each = rotate_each;
        ui.prepare.panels = panels;
        Self {
            session,
            config_dir,
            ui,
            tokens,
            follows_system: theme == Theme::System,
            agent: false,
            files: Box::new(files::NativePicker),
            tracer: std::sync::Arc::new(flatcraft_import::trace::trace),
            revealer: Box::new(files::NativeRevealer::default()),
            clock: std::sync::Arc::new(Instant::now),
            source_check: std::sync::Arc::new(flatcraft_engine::commands::sources::check_unlocked),
            source_check_job: None,
            long: long::LongJobs::new(ctx),
            frame_deadline: None,
            rng: rng::XorShift::default(),
            injected: Vec::new(),
            title_bar: cfg!(target_os = "macos"),
            double_click: None,
            reduce_motion: std::sync::Arc::new(toasts::system_reduce_motion),
            wall_clock: std::sync::Arc::new(std::time::SystemTime::now),
            thumbnailer: std::sync::Arc::new(home::render_project),
            inline_pictures: false,
            #[cfg(target_os = "macos")]
            menu: None,
        }
    }

    /// Replaces the system's title-bar double-click setting.
    pub fn with_double_click(mut self, action: app_bar::DoubleClick) -> Self {
        self.double_click = Some(action);
        self
    }

    /// Reads the wall clock from `f` instead of `SystemTime::now` (tests).
    pub fn with_wall_clock(
        mut self,
        f: impl Fn() -> std::time::SystemTime + Send + Sync + 'static,
    ) -> Self {
        self.wall_clock = std::sync::Arc::new(f);
        self
    }

    /// Draws Home's project pictures with `f` instead of the preview
    /// renderer (tests).
    pub fn with_thumbnailer(
        mut self,
        f: impl Fn(&flatcraft_device::ProfileStore, &Path) -> Result<Vec<u8>, String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.thumbnailer = std::sync::Arc::new(f);
        self
    }

    /// Draws Home's pictures within the frame (`true`) or on a worker
    /// thread (tests).
    pub fn with_inline_pictures(mut self, on: bool) -> Self {
        self.inline_pictures = on;
        self
    }

    /// Replaces the system's reduced-motion setting.
    pub fn with_reduce_motion(mut self, f: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        self.reduce_motion = std::sync::Arc::new(f);
        self
    }

    /// Whether the app bar is the window's title bar (see
    /// [`FlatCraftApp::title_bar`]).
    pub fn with_title_bar(mut self, on: bool) -> Self {
        self.title_bar = on;
        self
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
        let model = self.menu_model(ctx, None);
        match menu::AppMenu::install(ctx, model) {
            Ok(m) => self.menu = Some(m),
            Err(e) => eprintln!("warning: cannot install the menu bar: {e}"),
        }
    }

    /// What the menu bar shows now, for the document `snap` (none on Home).
    pub fn menu_model(
        &self,
        ctx: &egui::Context,
        snap: Option<&prepare::Snapshot>,
    ) -> menu_model::MenuModel {
        let (can_undo, can_redo, recent) = {
            let s = lock(&self.session);
            let doc = s.doc();
            (
                doc.undo.can_undo(),
                doc.undo.can_redo(),
                s.prefs.recent_projects.clone(),
            )
        };
        menu_model::MenuModel::build(&menu_model::MenuInput {
            workspace: self.ui.workspace,
            snap,
            prepare: &self.ui.prepare,
            show_debug: self.ui.show_debug,
            can_undo,
            can_redo,
            text_focused: ctx.text_edit_focused(),
            send_ready: matches!(self.ui.cut.plan, Some(Ok(_))),
            recent: &recent,
            modal: modal_open(self),
        })
    }

    /// Updates the menu bar to the state of this frame. Only a model that
    /// differs from the last one touches the muda items.
    #[cfg(target_os = "macos")]
    fn refresh_menu(&mut self, ctx: &egui::Context, snap: Option<&prepare::Snapshot>) {
        if self.menu.is_none() {
            return;
        }
        let model = self.menu_model(ctx, snap);
        if let Some(menu) = self.menu.as_mut() {
            menu.update(model);
        }
    }

    /// Runs a menu item that needs the snapshot or the keyboard focus.
    fn run_menu(&mut self, ctx: &egui::Context, cmd: menu_model::Cmd) {
        use menu_model::Cmd;
        let key = |key: egui::Key, shift: bool| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                shift,
                ..egui::Modifiers::COMMAND
            },
        };
        let text = ctx.text_edit_focused();
        match cmd {
            // The menu has the keys, so a text field gets them from here.
            Cmd::Undo | Cmd::Redo | Cmd::SelectAll if text => {
                self.injected.push(match cmd {
                    Cmd::SelectAll => key(egui::Key::A, false),
                    _ => key(egui::Key::Z, cmd == Cmd::Redo),
                });
            }
            Cmd::Undo | Cmd::Redo => {
                let active = lock(&self.session).doc().id();
                let a = if cmd == Cmd::Undo {
                    Action::Undo(active)
                } else {
                    Action::Redo(active)
                };
                self.apply_one(ctx, a);
            }
            Cmd::Cut => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut),
            Cmd::Copy => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy),
            Cmd::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Cmd::SelectAll
            | Cmd::Tool(_)
            | Cmd::LinkCopies
            | Cmd::Unlink
            | Cmd::ToggleCut
            | Cmd::ToggleHide
            | Cmd::PickKeep
            | Cmd::Rename
            | Cmd::Reconnect
            | Cmd::RevealSource
            | Cmd::CopySourcePath => {
                // The menu is dimmed outside Prepare; a late click does
                // nothing. A text field keeps the keys of Duplicate, Import
                // and Delete.
                if self.ui.workspace != Workspace::Prepare {
                    return;
                }
                if text
                    && matches!(cmd, Cmd::Tool(id) if tools::by_id(id).is_some_and(menu_model::has_shortcut))
                {
                    return;
                }
                let snap = prepare::snapshot(self, ctx);
                let ids: Vec<u64> = snap.selection.iter().copied().collect();
                let mut actions = Vec::new();
                match cmd {
                    Cmd::SelectAll => actions.push(context_menu::select_all_action(&snap)),
                    Cmd::Tool(id) => {
                        if let Some(t) = tools::by_id(id) {
                            t.run(
                                &snap,
                                &mut self.ui.prepare,
                                &mut self.ui.show_debug,
                                &mut actions,
                            );
                        }
                    }
                    other => {
                        if let Some(item) =
                            menu_model::object_item(&other, &snap, &self.ui.prepare, &ids)
                        {
                            context_menu::run_item(
                                item,
                                &snap,
                                &mut self.ui.prepare,
                                &ids,
                                &mut actions,
                            );
                        }
                    }
                }
                self.apply(ctx, actions);
            }
            // These became their own actions (`Cmd::action`). No `_` arm:
            // a new command must be placed here or above.
            Cmd::About
            | Cmd::Settings
            | Cmd::Quit
            | Cmd::New
            | Cmd::Open
            | Cmd::OpenRecent(_)
            | Cmd::NoRecent
            | Cmd::Save
            | Cmd::SaveAs
            | Cmd::Export
            | Cmd::Close
            | Cmd::SendToFile
            | Cmd::Workspace(_)
            | Cmd::Help(_) => {}
        }
    }

    /// Shows a toast. It ages by [`FlatCraftApp::clock`].
    pub fn toast(&mut self, mut toast: Toast) {
        toast.created = (self.clock)();
        self.ui.toasts.push(toast);
    }

    /// Runs the actions the widgets collected.
    pub fn apply(&mut self, ctx: &egui::Context, mut actions: Vec<Action>) {
        // A value typed into a Position and size field counts before a
        // save, close or quit, also while its field keeps the focus (Cmd+S).
        if self.ui.prepare.fields.editing.is_some()
            && actions.iter().any(Action::needs_typed_values)
        {
            let mut typed = Vec::new();
            ribbon::finish_now(ctx, &mut self.ui.prepare, &mut typed);
            actions.splice(0..0, typed);
        }
        // Typed values first, in the order they came (a stable sort): they
        // were typed before the click that ended them.
        actions.sort_by_key(|a| !matches!(a, Action::RunIn { .. }));
        for action in actions {
            self.apply_one(ctx, action);
        }
    }

    /// `project.save` with the thumbnail drawn and the file written
    /// without the session lock (an agent can work meanwhile). Errors
    /// become toasts.
    fn save_project(&mut self, params: Value) -> Option<Value> {
        let result =
            flatcraft_engine::commands::project::save_unlocked(&self.session, params, Origin::User);
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                self.toast(Toast::error(person_text(&e)));
                None
            }
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
        if flatcraft_engine::unlocked::runs_unlocked(command) {
            // Off the session lock on a worker; its result is taken in
            // `finish_long`, in this frame when it is quick.
            self.run_long(long::LongJob {
                work: long::Work::Command {
                    doc_id: Some(doc_id),
                    command,
                    params,
                },
                then: long::Then::Command {
                    doc_id: Some(doc_id),
                },
            });
            return None;
        }
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

    /// Until when this frame may still wait for work it starts: one budget
    /// of [`long::LONG_WAIT`] per frame, shared by the long commands and
    /// the Cut plan. Outside a frame, a fresh budget.
    pub fn frame_deadline(&self) -> Instant {
        self.frame_deadline
            .unwrap_or_else(|| Instant::now() + long::LONG_WAIT)
    }

    /// Queues a long command and runs what is ready (see [`long`]).
    fn run_long(&mut self, job: long::LongJob) {
        self.long.push(job);
        self.pump_long();
    }

    /// Starts the next long command and takes the ones that finished.
    /// Every frame calls it.
    pub fn pump_long(&mut self) {
        let deadline = self.frame_deadline();
        while let Some((job, result)) = self.long.poll(&self.session, deadline) {
            self.finish_long(job, result);
        }
    }

    /// What follows a long command, as it followed the command when it ran
    /// under the lock.
    fn finish_long(&mut self, job: long::LongJob, result: Result<Value, CommandError>) {
        let command = job.command();
        let v = match result {
            Err(e) if e == flatcraft_engine::unlocked::not_active() => {
                self.toast(Toast::warning(flatcraft_engine::unlocked::NOT_ACTIVE));
                return;
            }
            Err(e) if e == flatcraft_engine::unlocked::closed(command) => {
                self.toast(Toast::warning(person_text(&e)));
                return;
            }
            Err(e) => {
                self.toast(Toast::error(person_text(&e)));
                return;
            }
            Ok(v) => v,
        };
        // The document the result landed in (the engine says so), which
        // is not the active one when the person switched tabs meanwhile.
        let active = lock(&self.session).doc().id();
        let landed = v["doc_id"].as_u64();
        match job.then {
            long::Then::Command { doc_id } => match landed.or(doc_id) {
                Some(d) if d != active => self.landed_elsewhere(command, d, &v),
                _ => self.after_command(command, &v),
            },
            long::Then::ImportToPrepare => match landed {
                Some(d) if d != active => self.landed_elsewhere(command, d, &v),
                _ => {
                    self.after_command(command, &v);
                    self.ui.workspace = Workspace::Prepare;
                }
            },
            long::Then::Export(path) => self.exported(&path, &v),
            long::Then::Open { path, replaces } => {
                self.opened(&path, &v);
                if let Some(old) = replaces {
                    let found = std::path::absolute(&path).unwrap_or(path);
                    if found != old {
                        self.remove_recent(&old);
                    }
                }
            }
            long::Then::Sample => {
                for w in v["warnings"].as_array().into_iter().flatten() {
                    if let Some(w) = w.as_str() {
                        self.toast(Toast::warning(w.to_string()));
                    }
                }
                self.ui.workspace = Workspace::Prepare;
            }
        }
    }

    /// A long command landed in the document `doc_id`, which is not the
    /// active one now: its report card waits for that tab (it shows when
    /// the tab becomes active), and a toast names where it went, with the
    /// files that failed and how many warnings there are. Nothing touches
    /// the active document.
    fn landed_elsewhere(&mut self, command: &str, doc_id: u64, v: &Value) {
        let name = self.doc_name(doc_id);
        let count = |n: usize, one: &str, many: &str| {
            if n == 1 {
                format!("1 {one}")
            } else {
                format!("{n} {many}")
            }
        };
        let failed: Vec<String> = v["failed"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["path"].as_str())
            .map(|path| display_name(Path::new(path)))
            .collect();
        let warnings = v["warnings"].as_array().map_or(0, Vec::len);
        let mut text = match command {
            "object.import" | "object.import_svg" => match v["imported"].as_array() {
                Some(ids) => format!(
                    "Imported {} into {name}.",
                    count(ids.len(), "file", "files")
                ),
                None => format!("Imported into {name}."),
            },
            "object.retrace" => format!("Traced the picture in {name}."),
            "object.reload" => format!("Read the files again in {name}."),
            "object.relink" => format!("Linked the file in {name}."),
            _ => format!("Done in {name}."),
        };
        if !failed.is_empty() {
            text.push_str(&format!(
                " {} could not be imported: {}.",
                count(failed.len(), "file", "files"),
                failed.join(", ")
            ));
        }
        if warnings > 0 {
            text.push_str(&format!(" {}.", count(warnings, "warning", "warnings")));
        }
        // The card waits for its tab.
        let title = match v["imported"].as_array() {
            Some(ids) if ids.len() == 1 => Some("Imported 1 file".to_string()),
            Some(ids) => Some(format!("Imported {} files", ids.len())),
            None if command == "object.import" || command == "object.import_svg" => {
                let id = v["id"].as_u64();
                let s = lock(&self.session);
                let object = s.doc_index(doc_id).ok().and_then(|i| {
                    id.and_then(|id| {
                        s.docs[i]
                            .document
                            .get(flatcraft_core::document::ObjectId(id))
                            .map(|o| o.name.clone())
                    })
                });
                Some(format!("Imported '{}'", object.unwrap_or_default()))
            }
            None => None,
        };
        if let Some(card) = title.and_then(|t| report::ImportCard::from_result(doc_id, t, v)) {
            self.ui.prepare.waiting_reports.insert(doc_id, card);
        }
        let toast = if failed.is_empty() {
            Toast::info(text)
        } else {
            Toast::warning(text)
        };
        self.toast(toast.with_detail("The report shows when you open its tab."));
    }

    /// Opens the stencil hint card (`stencil_island`) for the Stencil
    /// objects with inner parts on a material without a backing sheet,
    /// once per object and material: after a keep mode or a material
    /// changed.
    fn stencil_hint(&mut self, doc_id: u64) {
        // The input is taken under the lock, the check (it builds the cut
        // geometry) runs without it, and the geometry is offered back.
        let (input, material, at) = {
            let s = lock(&self.session);
            let st = s.doc();
            let job = &st.document.job;
            let input = match (
                s.profiles.plotter(&job.plotter_id),
                s.profiles.material(&job.material_id),
            ) {
                (Ok(p), Ok(m)) => Some(
                    flatcraft_engine::job::JobInput::from_parts(&st.document, p, &m)
                        .with_seed(st.cached_cut_geometry()),
                ),
                _ => None,
            };
            (input, job.material_id.clone(), (st.id(), st.revision()))
        };
        let found = input
            .as_ref()
            .map(|input| flatcraft_engine::report::check(input, None))
            .unwrap_or_default();
        if let Some(input) = &input {
            lock(&self.session).offer_cut_geometry(at.0, at.1, input);
        }
        let issues: Vec<_> = found
            .issues
            .into_iter()
            .filter(|i| i.code == flatcraft_engine::report::Code::StencilIsland)
            .collect();
        let ids: Vec<u64> = issues.iter().flat_map(|i| i.object_ids.clone()).collect();
        let p = &mut self.ui.prepare;
        let key = |id: u64| (doc_id, id, material.clone());
        if ids.iter().all(|id| p.stencil_told.contains(&key(*id))) {
            return;
        }
        p.stencil_told.extend(ids.iter().map(|id| key(*id)));
        let result = serde_json::json!({
            "ids": ids,
            "report": flatcraft_engine::report::Report { shapes: found.shapes, issues }.to_json(),
        });
        p.report = report::ImportCard::from_result(doc_id, "Keep is Stencil".into(), &result)
            .map(|c| c.with_header("Stencil on paper"));
    }

    /// Updates what Prepare shows after a command ran.
    fn after_command(&mut self, command: &str, result: &Value) {
        let doc_id = lock(&self.session).doc().id();
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
                // Several files: their report, and one toast that names
                // the files that could not be imported.
                let n = result["imported"].as_array().map_or(0, Vec::len);
                let title = if n == 1 {
                    "Imported 1 file".to_string()
                } else {
                    format!("Imported {n} files")
                };
                p.report = report::ImportCard::from_result(doc_id, title, result);
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
            "object.import" => {
                // The report card says what the warnings say, and more.
                let name = result["id"]
                    .as_u64()
                    .and_then(|id| {
                        lock(&self.session)
                            .doc()
                            .document
                            .get(flatcraft_core::document::ObjectId(id))
                            .map(|o| o.name.clone())
                    })
                    .unwrap_or_default();
                p.report =
                    report::ImportCard::from_result(doc_id, format!("Imported '{name}'"), result);
            }
            "object.set_keep" | "job.set_material" => self.stencil_hint(doc_id),
            "object.reload" => {
                let done: Vec<u64> = [&result["reloaded"], &result["unchanged"]]
                    .into_iter()
                    .flat_map(ids)
                    .collect();
                let reloaded = ids(&result["reloaded"]);
                match &mut p.report {
                    Some(card) if card.doc_id == doc_id => {
                        card.reloaded(&reloaded, result);
                        if card.is_empty() {
                            p.report = None;
                        }
                    }
                    _ if !reloaded.is_empty() => {
                        let title = if reloaded.len() == 1 {
                            "Reloaded 1 object".to_string()
                        } else {
                            format!("Reloaded {} objects", reloaded.len())
                        };
                        p.report = report::ImportCard::from_result(doc_id, title, result)
                            .map(|c| c.with_header("Reload report"));
                    }
                    _ => {}
                }
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
                let reloaded = ids(&result["reloaded"]);
                match &mut p.report {
                    Some(card) if card.doc_id == doc_id => {
                        card.reloaded(&reloaded, result);
                        if card.is_empty() {
                            p.report = None;
                        }
                    }
                    _ if !reloaded.is_empty() => {
                        let title = if reloaded.len() == 1 {
                            "Reloaded 1 object".to_string()
                        } else {
                            format!("Reloaded {} objects", reloaded.len())
                        };
                        p.report = report::ImportCard::from_result(doc_id, title, result)
                            .map(|c| c.with_header("Reload report"));
                    }
                    _ => {}
                }
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
            Action::RunIn {
                doc_id,
                command,
                params,
            } => {
                let result = lock(&self.session).execute_in(doc_id, command, params, Origin::User);
                match result {
                    Err(e) => self.toast(Toast::error(person_text(&e))),
                    Ok(v) => {
                        if lock(&self.session).doc().id() == doc_id {
                            self.after_command(command, &v);
                        }
                    }
                }
            }
            Action::Import { doc_id } => {
                if let Some(path) = self.files.pick_file(&import_dialog::import_request()) {
                    self.run_on(doc_id, "object.import", json!({ "path": path }));
                }
            }
            Action::TraceDialog { doc_id, id } => self.open_trace(ctx, doc_id, id),
            Action::ShowMe { doc_id, ids } => {
                if self
                    .run_on(doc_id, "object.select", json!({ "ids": ids }))
                    .is_some()
                {
                    self.ui.workspace = Workspace::Prepare;
                    self.ui.prepare.center_on = Some(ids);
                }
            }
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
                        if let Some(v) = self.save_project(json!({ "doc_id": doc_id })) {
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
                if let Some(v) = self.save_project(json!({ "doc_id": doc_id, "path": path })) {
                    self.saved(&v);
                }
            }
            Action::ExportDialog { doc_id } => {
                let has_selection = {
                    let s = lock(&self.session);
                    s.doc_index(doc_id)
                        .ok()
                        .map(|i| !s.docs[i].selection.is_empty())
                };
                match has_selection {
                    Some(_) if self.ui.workspace == Workspace::Home => {
                        self.toast(Toast::info(export_dialog::NO_DOCUMENT));
                    }
                    Some(has) => {
                        self.ui.export = Some(export_dialog::ExportDialog::new(doc_id, has));
                    }
                    None => {}
                }
            }
            Action::Export(d) => self.export(d),
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
                self.open_project(&path, None);
            }
            Action::RemoveRecent(path) => self.remove_recent(&path),
            Action::OpenSample(i) => {
                let Some(sample) = home::SAMPLES.get(i) else {
                    return;
                };
                self.run_long(long::LongJob {
                    work: long::Work::Sample {
                        bytes: std::sync::Arc::from(sample.bytes),
                        title: sample.title.to_string(),
                    },
                    then: long::Then::Sample,
                });
            }
            Action::LocateRecent(old) => {
                let req = files::FileRequest {
                    title: format!("Locate {}", display_name(&old)),
                    filters: vec![files::filter("FlatCraft project", &[PROJECT_EXTENSION])],
                    file_name: Some(display_name(&old)),
                    ..Default::default()
                };
                if let Some(found) = self.files.pick_file(&req) {
                    self.open_project(&found, Some(old));
                }
            }
            Action::ShowOpen(doc_id, workspace) => {
                if self
                    .execute("document.switch", json!({ "doc_id": doc_id }))
                    .is_some()
                {
                    self.ui.workspace = workspace;
                }
            }
            Action::NewDocument => {
                self.execute("document.new", json!({}));
            }
            Action::NewProject => {
                self.new_project();
            }
            Action::StartWith(start) => self.start_with(ctx, start),
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
                // An empty stack is no error (the shortcut is always there;
                // the macOS menu item dims).
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
            Action::AskOutcome(entry) => {
                let log = lock(&self.session).cut_log();
                match log.get(&entry) {
                    Some(e) => self.ui.outcome = Some(outcome::OutcomeDialog::about(e)),
                    None => self.toast(Toast::error(
                        "That cut is no longer in the cut log.".to_string(),
                    )),
                }
            }
            Action::RecordOutcome {
                entry,
                verdict,
                note,
            } => {
                let result = lock(&self.session).record_outcome(&entry, verdict, &note);
                match result {
                    Ok(()) => self.toast(Toast::success(
                        "Noted. The job settings remind you next time.",
                    )),
                    Err(e) => self.toast(Toast::error(format!("Cannot note it: {e}"))),
                }
            }
            Action::CopyPath(path) => {
                let text = path.display().to_string();
                ctx.copy_text(text.clone());
                self.toast(Toast::info("Path copied").with_detail(text));
            }
            Action::CloseOthers(keep) => {
                let others: Vec<u64> = {
                    let s = lock(&self.session);
                    s.docs
                        .iter()
                        .map(|d| d.id())
                        .filter(|d| *d != keep)
                        .collect()
                };
                self.ui.close_queue = others;
                self.close_next_queued();
            }
            Action::Error(text) => self.toast(Toast::error(text)),
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
            Action::ShowSettings => self.ui.show_settings = true,
            Action::RememberEach(each) => {
                let result = lock(&self.session).update_prefs(|p| p.rotate_each = each);
                if let Err(e) = result {
                    self.toast(Toast::error(format!(
                        "Cannot store the Each object setting: {e}"
                    )));
                }
            }
            Action::RememberPanels(users) => {
                let result = lock(&self.session).update_prefs(|p| rails::Panels::write(users, p));
                if let Err(e) = result {
                    self.toast(Toast::error(format!("Cannot store the panel choices: {e}")));
                }
            }
            Action::RememberSections(open) => {
                let result = lock(&self.session).update_prefs(|p| p.panel_sections.extend(open));
                if let Err(e) = result {
                    self.toast(Toast::error(format!(
                        "Cannot store which panel sections are open: {e}"
                    )));
                }
            }
            Action::HideTips(ids) => {
                let result = lock(&self.session).update_prefs(|p| p.hidden_tips.extend(ids));
                if let Err(e) = result {
                    self.toast(Toast::error(format!("Cannot store the hidden tip: {e}")));
                }
            }
            Action::RememberSpacing(mm) => {
                let result = lock(&self.session).update_prefs(|p| p.auto_arrange_spacing_mm = mm);
                if let Err(e) = result {
                    self.toast(Toast::error(format!(
                        "Cannot store the Auto arrange spacing: {e}"
                    )));
                }
            }
            Action::SetPin { id, pinned } => {
                let result = lock(&self.session).update_prefs(|p| {
                    p.ribbon_pins.retain(|x| x != id);
                    if pinned {
                        p.ribbon_pins.push(id.to_string());
                    }
                });
                if let Err(e) = result {
                    self.toast(Toast::error(format!("Cannot store the pins: {e}")));
                }
            }
            Action::Profiles(req) => profiles_window::apply(self, req),
            Action::ShowAbout => self.ui.about.open(&mut self.rng),
            Action::ShowShortcuts => self.ui.show_shortcuts = true,
            Action::Troubleshoot => {
                if self.ui.workspace == Workspace::Home {
                    self.ui.workspace = Workspace::Cut;
                }
                self.ui.troubleshoot.open();
            }
            Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            Action::Quit => {
                let dirty = self.dirty_docs();
                if !dirty.is_empty() {
                    self.ui.unsaved = Some(Unsaved::Quit {
                        asking: dirty[0],
                        answered: Vec::new(),
                    });
                } else if self.long.busy() {
                    self.ui.unsaved = Some(Unsaved::Busy {
                        answered: Vec::new(),
                    });
                } else if self.send_waiting() {
                    self.ui.unsaved = Some(Unsaved::PendingSend);
                } else {
                    self.close_window(ctx);
                }
            }
            Action::Unsaved(asked, choice) => self.answer_unsaved(ctx, asked, choice),
            Action::Menu(cmd) => self.run_menu(ctx, cmd),
        }
    }

    /// Reuses the active document if it is pristine, else opens a new one,
    /// and goes to Prepare. Returns the new project's document id, `None`
    /// when it failed (a toast says why).
    fn new_project(&mut self) -> Option<u64> {
        // Checked and acted on under one lock.
        let result = {
            let mut s = lock(&self.session);
            if s.doc().is_pristine() {
                Ok(s.doc().id())
            } else {
                s.execute("document.new", json!({}), Origin::User)
                    .map(|_| s.doc().id())
            }
        };
        match result {
            Ok(id) => {
                self.ui.workspace = Workspace::Prepare;
                Some(id)
            }
            Err(e) => {
                self.toast(Toast::error(person_text(&e)));
                None
            }
        }
    }

    /// Runs a card of Home's New row. Import asks for the file first, so
    /// a cancelled dialog leaves no empty project behind.
    fn start_with(&mut self, ctx: &egui::Context, start: home::Start) {
        match start {
            home::Start::Empty => {
                self.new_project();
            }
            home::Start::Open => self.apply_one(ctx, Action::OpenDialog),
            home::Start::Import => {
                let Some(path) = self.files.pick_file(&import_dialog::import_request()) else {
                    return;
                };
                if let Some(doc_id) = self.new_project() {
                    self.run_on(doc_id, "object.import", json!({ "path": path }));
                }
            }
            home::Start::TestPatterns => {
                if let Some(doc_id) = self.new_project() {
                    self.run_on(doc_id, "tools.test_patterns", json!({}));
                }
            }
        }
    }

    /// Closes the queued documents of "Close other tabs" without unsaved
    /// changes, and asks about the next one that has them.
    fn close_next_queued(&mut self) {
        while !self.ui.close_queue.is_empty() {
            let doc_id = self.ui.close_queue.remove(0);
            let dirty = {
                let s = lock(&self.session);
                match s.doc_index(doc_id) {
                    Ok(i) => s.docs[i].dirty,
                    // Closed meanwhile.
                    Err(_) => continue,
                }
            };
            if dirty {
                self.ui.unsaved = Some(Unsaved::Close(doc_id));
                return;
            }
            self.execute("document.close", json!({ "doc_id": doc_id }));
        }
    }

    /// Stable ids of the documents with unsaved changes.
    pub fn dirty_docs(&self) -> Vec<u64> {
        let s = lock(&self.session);
        s.docs.iter().filter(|d| d.dirty).map(|d| d.id()).collect()
    }

    /// An agent waits for the person's OK to send a cut.
    fn send_waiting(&self) -> bool {
        lock(&self.session).pending_send().is_some()
    }

    /// Closes the window without asking again.
    fn close_window(&mut self, ctx: &egui::Context) {
        self.ui.closing = true;
        self.ui.unsaved = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Runs the answer of the unsaved-changes dialog to `asked`. An answer
    /// to a question that is no longer open does nothing.
    fn answer_unsaved(&mut self, ctx: &egui::Context, asked: Unsaved, choice: UnsavedChoice) {
        if self.ui.unsaved.as_ref() != Some(&asked) {
            return;
        }
        self.ui.unsaved = None;
        let closing = matches!(asked, Unsaved::Close(_));
        match (asked, choice) {
            (_, UnsavedChoice::Cancel) => self.ui.close_queue.clear(),
            (Unsaved::PendingSend | Unsaved::Busy { .. }, _) => self.close_window(ctx),
            (Unsaved::Close(doc_id), UnsavedChoice::Discard) => {
                self.execute(
                    "document.close",
                    json!({ "doc_id": doc_id, "discard_changes": true }),
                );
            }
            (Unsaved::Close(doc_id), UnsavedChoice::Save) => {
                if self.save_for_close(doc_id) {
                    self.execute("document.close", json!({ "doc_id": doc_id }));
                } else {
                    // Not saved: "Close other tabs" stops here, as Quit does.
                    self.ui.close_queue.clear();
                }
            }
            (
                Unsaved::Quit {
                    asking,
                    mut answered,
                },
                choice,
            ) => {
                if choice == UnsavedChoice::Save && !self.save_for_close(asking) {
                    self.toast(Toast::warning(
                        "Not everything was saved, so FlatCraft stays open.",
                    ));
                    return;
                }
                // Changed while it was written (an agent): the file holds
                // the older state, so the question comes again.
                if choice == UnsavedChoice::Save && self.dirty_docs().contains(&asking) {
                    self.toast(Toast::warning(
                        "The document changed while it was saved, so FlatCraft asks again.",
                    ));
                    self.ui.unsaved = Some(Unsaved::Quit { asking, answered });
                    return;
                }
                // The revision after the answer (and after a save), so a
                // later change by an agent is asked about again.
                if let Some(rev) = self.doc_revision(asking) {
                    answered.retain(|(d, _)| *d != asking);
                    answered.push((asking, rev));
                }
                self.ask_next_or_quit(ctx, answered);
            }
        }
        // "Close other tabs" goes on with the next document, unless the
        // person cancelled (the queue is empty then).
        if closing && self.ui.unsaved.is_none() {
            self.close_next_queued();
        }
    }

    /// Quit goes on: the next document with unsaved changes that is not
    /// answered yet is asked about (also one an agent changed during the
    /// round). The window closes only when none is left.
    fn ask_next_or_quit(&mut self, ctx: &egui::Context, answered: Vec<(u64, u64)>) {
        let next = self
            .dirty_docs()
            .into_iter()
            .find(|d| !self.still_answered(&answered, *d));
        match next {
            Some(asking) => self.ui.unsaved = Some(Unsaved::Quit { asking, answered }),
            // Never quit straight away while a long command still runs:
            // ask first. A document it changes is asked about after it.
            None if self.long.busy() => self.ui.unsaved = Some(Unsaved::Busy { answered }),
            // As the plain Quit does: a waiting agent is asked about last.
            None if self.send_waiting() => self.ui.unsaved = Some(Unsaved::PendingSend),
            None => self.close_window(ctx),
        }
    }

    /// The document was answered in this Quit round and has not changed
    /// since.
    fn still_answered(&self, answered: &[(u64, u64)], doc: u64) -> bool {
        let rev = self.doc_revision(doc);
        answered.iter().any(|&(d, r)| d == doc && rev == Some(r))
    }

    /// The change counter of a document, `None` when it is gone.
    fn doc_revision(&self, doc: u64) -> Option<u64> {
        let s = lock(&self.session);
        s.doc_index(doc).ok().map(|i| s.docs[i].revision())
    }

    /// While Quit asks about a document that was saved or closed meanwhile
    /// (by an agent), it moves on to the next one, or quits.
    fn skip_settled_quit_question(&mut self, ctx: &egui::Context) {
        let Some(Unsaved::Quit { asking, answered }) = &self.ui.unsaved else {
            return;
        };
        if self.dirty_docs().contains(asking) {
            return;
        }
        let answered = answered.clone();
        self.ask_next_or_quit(ctx, answered);
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
        match self.save_project(params) {
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
        path.as_deref().map(project_name).unwrap_or_else(|| {
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
        // That project opens before them (one long command at a time), so
        // the files go into the document active when their import begins.
        let params = match files.as_slice() {
            [one] => json!({ "path": one }),
            _ => json!({ "paths": files }),
        };
        self.run_long(long::LongJob {
            work: long::Work::Command {
                doc_id: None,
                command: "object.import",
                params,
            },
            then: long::Then::ImportToPrepare,
        });
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

    /// Asks where to save the export of `d` (the document's name with the
    /// format's extension, next to the project file), then runs
    /// `document.export` as the person. A failed write is an error toast
    /// with the reason.
    fn export(&mut self, d: export_dialog::ExportDialog) {
        let ext = d.format.extension();
        let (name, dir) = self.save_as_suggestion(d.doc_id);
        let name = Path::new(&name)
            .with_extension(ext)
            .to_string_lossy()
            .into_owned();
        let req = files::FileRequest {
            title: "Export".into(),
            filters: vec![files::filter(d.format.label(), &[ext])],
            file_name: Some(name),
            directory: dir,
        };
        let Some(path) = self.files.save_file(&req) else {
            return;
        };
        let scope = match d.scope {
            flatcraft_engine::export::ExportScope::Selected => "selected",
            flatcraft_engine::export::ExportScope::All => "all",
        };
        let params = json!({
            "path": path,
            "format": ext,
            "scope": scope,
            "include_skipped": d.include_skipped,
        });
        self.run_long(long::LongJob {
            work: long::Work::Command {
                doc_id: Some(d.doc_id),
                command: "document.export",
                params,
            },
            then: long::Then::Export(path),
        });
    }

    /// The toast of a finished export to `path`.
    fn exported(&mut self, path: &Path, v: &Value) {
        let path = path.to_path_buf();
        let shapes = v["shapes"].as_u64().unwrap_or(0);
        let what = if shapes == 1 {
            "1 shape".to_string()
        } else {
            format!("{shapes} shapes")
        };
        self.toast(
            Toast::success("Exported. Your shapes are ready for the next adventure!")
                .with_detail(format!("Wrote {what} to {}", display_name(&path)))
                .with_reveal(path),
        );
    }

    /// The toast of a finished send: the written files and a link that
    /// shows them in the file manager.
    fn sent(&mut self, result: &Value) {
        if let Some(e) = result["cut_log_error"].as_str() {
            self.toast(Toast::error(e.to_string()));
        }
        // "How did it go?" right after a cut on a plotter, never after
        // File Output.
        if let Some(id) = result["cut_log_entry"].as_str() {
            let log = lock(&self.session).cut_log();
            if let Some(d) = log.get(id).and_then(outcome::after_send) {
                self.ui.outcome = Some(d);
            }
        }
        let files: Vec<PathBuf> = result["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str().map(PathBuf::from))
            .collect();
        // About one send in five, the toast quotes Star Wars instead.
        let punch = self.rng.one_in(PUNCH_IT_ODDS);
        let Some(first) = files.first().cloned() else {
            self.toast(Toast::success(if punch {
                PUNCH_IT
            } else {
                "Sent. Ship it!"
            }));
            return;
        };
        if punch {
            self.toast(
                Toast::success(PUNCH_IT)
                    .with_detail(first.display().to_string())
                    .with_reveal(first),
            );
            return;
        }
        let names: Vec<String> = files.iter().map(|f| display_name(f)).collect();
        let dir = first
            .parent()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        self.toast(
            Toast::success("Sent! Ship it!")
                .with_detail(format!("Wrote {} to {dir}", names.join(", ")))
                .with_reveal(first),
        );
    }

    /// Opens a project file off the session lock (see [`long`]); then
    /// [`Self::opened`] puts it first in the recent list and goes to
    /// Prepare. A failure is a toast that says why. `replaces` is a
    /// recent entry the file was located for: it leaves the list when the
    /// file opened from another place.
    fn open_project(&mut self, path: &Path, replaces: Option<PathBuf>) {
        self.run_long(long::LongJob {
            work: long::Work::Open(path.to_path_buf()),
            then: long::Then::Open {
                path: path.to_path_buf(),
                replaces,
            },
        });
    }

    /// What follows an open project: the recent list, its picture, the
    /// warnings and Prepare.
    fn opened(&mut self, path: &Path, v: &Value) {
        self.remember_recent(path);
        self.refresh_thumbnail(path);
        for w in v["warnings"].as_array().into_iter().flatten() {
            if let Some(w) = w.as_str() {
                self.toast(Toast::warning(w.to_string()));
            }
        }
        self.ui.workspace = Workspace::Prepare;
        self.ui.prepare.sources.check_pending = true;
    }

    /// Makes Home read the picture of `path` again: a save wrote a new
    /// one into the project file (`thumbnail.png`), and an open may show a
    /// file that changed elsewhere.
    fn refresh_thumbnail(&mut self, path: &Path) {
        let id = flatcraft_engine::thumbnails::project_id(path);
        self.ui.home.forget_texture(&id);
    }

    /// Takes a project off the recent list, with its details and picture.
    fn remove_recent(&mut self, path: &Path) {
        let path = path.to_path_buf();
        let result = lock(&self.session).update_prefs(|p| {
            p.recent_projects.retain(|x| x != &path);
            p.prune_recent_details();
        });
        if let Err(e) = result {
            self.toast(Toast::error(format!(
                "Cannot store the recent projects: {e}"
            )));
        }
        let id = flatcraft_engine::thumbnails::project_id(&path);
        self.ui.home.forget_texture(&id);
    }

    /// Notes the workspace the active project is in, for Home's recent
    /// list. The preferences are written only when it changed.
    fn note_workspace(&mut self) {
        let workspace = self.ui.workspace;
        if workspace == Workspace::Home {
            return;
        }
        let path = lock(&self.session).doc().path.clone();
        let Some(path) = path else {
            return;
        };
        let path = std::path::absolute(&path).unwrap_or(path);
        let seen = Some((path.clone(), workspace));
        if self.ui.home.last_seen == seen {
            return;
        }
        self.ui.home.last_seen = seen;
        let name = workspace.name();
        let stale = {
            let s = lock(&self.session);
            s.prefs.recent_projects.contains(&path)
                && s.prefs.recent_detail(&path).map(|d| d.workspace.as_str()) != Some(name)
        };
        if stale {
            let result = lock(&self.session).update_prefs(|p| {
                if !p.recent_projects.contains(&path) {
                    return;
                }
                match p.recent_details.iter_mut().find(|d| d.path == path) {
                    Some(d) => d.workspace = name.to_string(),
                    None => p
                        .recent_details
                        .push(flatcraft_engine::prefs::RecentDetail {
                            path: path.clone(),
                            opened_unix_s: 0,
                            workspace: name.to_string(),
                        }),
                }
            });
            if let Err(e) = result {
                self.toast(Toast::error(format!(
                    "Cannot store the recent projects: {e}"
                )));
            }
        }
    }

    fn saved(&mut self, result: &Value) {
        if let Some(path) = result["path"].as_str() {
            let path = PathBuf::from(path);
            self.remember_recent(&path);
            self.refresh_thumbnail(&path);
            let full = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
            self.toast(
                Toast::success("Saved")
                    .with_detail(full.display().to_string())
                    .with_reveal(full),
            );
        }
    }

    /// Puts the project first in the recent list and stores it. Only the
    /// recent list changes: the file is read again first, so a preference
    /// another FlatCraft process stored meanwhile is kept.
    fn remember_recent(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let now = (self.wall_clock)()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let workspace = match self.ui.workspace {
            Workspace::Cut => Workspace::Cut,
            _ => Workspace::Prepare,
        };
        let result = lock(&self.session).update_prefs(|p| {
            let recent = &mut p.recent_projects;
            recent.retain(|p| p != &path);
            recent.insert(0, path.clone());
            recent.truncate(MAX_RECENT);
            let detail = match p.recent_details.iter_mut().find(|d| d.path == path) {
                Some(d) => d,
                None => {
                    p.recent_details
                        .push(flatcraft_engine::prefs::RecentDetail {
                            path: path.clone(),
                            ..Default::default()
                        });
                    p.recent_details.last_mut().expect("just pushed")
                }
            };
            detail.opened_unix_s = now;
            detail.workspace = workspace.name().to_string();
            p.prune_recent_details();
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

/// The name of a project for tabs, dialogs and Home's recent list: its
/// file name without a trailing ".flatcraft" (any case), so
/// "my.design.flatcraft" is "my.design". A name that is only the
/// extension stays as it is.
pub fn project_name(path: &Path) -> String {
    let name = display_name(path);
    let ext = format!(".{PROJECT_EXTENSION}");
    let cut = name.len().saturating_sub(ext.len());
    match name.get(cut..) {
        Some(tail) if cut > 0 && tail.eq_ignore_ascii_case(&ext) => name[..cut].to_string(),
        _ => name,
    }
}

/// The file name of a path, for messages and file dialogs.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Size of icons in buttons, in points.
pub const ICON_SIZE: f32 = 18.0;

/// The smallest window (logical points). The ribbon, the app bar and the
/// Cut bar are tested at this width: nothing is cut off or out of reach.
pub const MIN_WINDOW: [f32; 2] = [900.0, 600.0];

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
    calc::note_keys(&ctx);
    let mut actions = Vec::new();
    let (active, open) = {
        let s = lock(&app.session);
        let open: Vec<u64> = s.docs.iter().map(|d| d.id()).collect();
        (s.doc().id(), open)
    };
    app.ui.number_documents(&open);
    guard_close(&ctx, app);
    refresh_prefs(&ctx, app);
    // One wait budget for the whole frame (long commands and the plan).
    app.frame_deadline = Some(Instant::now() + long::LONG_WAIT);
    app.pump_long();
    // An import report that waited for its tab shows now that it is open.
    {
        let active = lock(&app.session).doc().id();
        let p = &mut app.ui.prepare;
        p.waiting_reports.retain(|d, _| open.contains(d));
        if let Some(card) = p.waiting_reports.remove(&active) {
            p.report = Some(card);
        }
    }
    // A quit that waited for a long command goes on once it is done.
    if let Some(Unsaved::Busy { answered }) = &app.ui.unsaved
        && !app.long.busy()
    {
        let answered = answered.clone();
        app.ui.unsaved = None;
        app.ask_next_or_quit(&ctx, answered);
    }
    app.ui.prepare.panels.window_w = ctx.content_rect().width();
    // A dialog keeps the keys: Escape closes or cancels it, and Delete or
    // Cmd+Z must not change the document behind it.
    let modal = modal_open(app);
    if !modal {
        shortcuts(&ctx, active, &mut actions);
    }
    // Keys the menu bar passes on (see `FlatCraftApp::injected`).
    if !app.injected.is_empty() {
        let events = std::mem::take(&mut app.injected);
        ctx.input_mut(|i| i.events.extend(events));
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
    // A collapsed Cut and Job rail shows the job's state dot from the same
    // plan the Cut workspace uses, so the two never disagree. `update`
    // builds only when the input changed.
    // Not while an object is dragged or nudged: the dot keeps its last
    // state and the job is built once when the drag or the burst ends.
    let rail_dot = workspace == Workspace::Prepare
        && app.ui.prepare.panels.collapsed(rails::SidePanel::Settings)
        && app.ui.prepare.drag.is_none()
        && app.ui.prepare.nudge.is_none();
    // The import report checks again when the document changed, after a
    // drag or a nudge burst ended.
    let report_check = workspace == Workspace::Prepare
        && app.ui.prepare.report.is_some()
        && app.ui.prepare.drag.is_none()
        && app.ui.prepare.nudge.is_none();
    // The kept and waste overlay in Prepare reads the plan; not while a
    // drag or a nudge burst changes the document every frame.
    let kept_overlay = workspace == Workspace::Prepare
        && (app.ui.prepare.show_kept_waste || app.ui.prepare.keep_pick.is_some())
        && app.ui.prepare.drag.is_none()
        && app.ui.prepare.nudge.is_none();
    let wants_job = workspace == Workspace::Cut
        || kept_overlay
        || app.ui.show_debug
        || app.ui.troubleshoot.open
        || rail_dot
        || report_check;
    let snap = (workspace != Workspace::Home).then(|| prepare::snapshot_with(app, &ctx, wants_job));
    if let Some(snap) = &snap {
        // A plan that landed this frame was built on the worker; its cut
        // geometry goes back into the document's cache.
        let deadline = app.frame_deadline();
        if let Some((doc, rev, input)) = app.ui.cut.update_until(&ctx, snap, deadline) {
            lock(&app.session).offer_cut_geometry(doc, rev, &input);
        }
        // The Simulate stage simulates on its own worker (see `simulate`).
        let simulating = workspace == Workspace::Cut && app.ui.cut.stage == cut::Stage::Simulate;
        app.ui
            .cut
            .sim
            .update(&ctx, &app.session, snap, deadline, simulating);
        app.ui.prepare.sheet_kept = cut::sheet_kept_now(&app.ui.cut, snap);
        if report_check && let Some(card) = &mut app.ui.prepare.report {
            card.refresh(snap);
            if card.is_empty() {
                app.ui.prepare.report = None;
            }
        }
        // The cut geometry built above (outside the lock) goes back into
        // the document's cache, so the engine and the window share it.
        // Only on a frame that built it: no extra lock otherwise.
        if let Some(Ok(input)) = &snap.job_input
            && input.built_cut_geometry().is_some()
        {
            lock(&app.session).offer_cut_geometry(snap.doc_id, snap.doc_rev, input);
        }
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
    #[cfg(target_os = "macos")]
    app.refresh_menu(&ctx, snap.as_ref());
    if let Some(snap) = &snap {
        // A typed size or position applies before anything else of this
        // frame (the app bar's actions too), also when its group left the
        // screen.
        let p = &mut app.ui.prepare;
        let on_screen = workspace == Workspace::Prepare && p.tab == prepare::RibbonTab::Layout;
        ribbon::finish_pending(&ctx, p, snap, on_screen, &mut actions);
    }
    app_bar::show(ui, app, &mut actions);
    if let (Some(snap), Workspace::Prepare) = (&snap, workspace) {
        // Before the ribbon draws: its Settings panel toggle needs to know
        // whether an Arrange panel takes room.
        arrange::keep_or_close(&mut app.ui.prepare, snap);
        app.ui.prepare.panels.arrange_open = app.ui.prepare.panel.is_some();
    }
    switcher::ribbon(ui, app, snap.as_ref(), &mut actions);
    if workspace != Workspace::Prepare {
        // The Trace dialog and the keep picker live in Prepare.
        app.ui.trace = None;
        app.ui.prepare.keep_pick = None;
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
            let p = &mut app.ui.prepare;
            p.text_focus_elsewhere = text_focus_outside(&ctx, p.panel_rect);
            p.popup_was_open = egui::Popup::is_any_open(&ctx);
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
    if app.ui.show_settings {
        settings_window::show(&ctx, app, &mut actions);
    }
    let now = (app.clock)();
    let t = app.tokens;
    about::show(&ctx, &mut app.ui.about, &t, now, &mut actions);
    if app.ui.show_shortcuts {
        app.ui.show_shortcuts = help::show_shortcuts(&ctx, &t);
    }
    match &snap {
        Some(snap) => {
            let t = app.tokens;
            export_dialog::show(&ctx, &mut app.ui.export, snap, &t, &mut actions);
            troubleshoot::show(&ctx, &mut app.ui.troubleshoot, snap, &t, &mut actions);
        }
        None => app.ui.export = None,
    }
    outcome::show(&ctx, &mut app.ui.outcome, &t, &mut actions);
    pending_modal(&ctx, app, &mut actions);
    app.skip_settled_quit_question(&ctx);
    unsaved_modal(&ctx, app, &mut actions);

    app.note_workspace();
    if app.ui.workspace != Workspace::Home {
        // The next visit of Home looks at the recent files again.
        app.ui.home.versions.clear();
    }
    if let Some(users) = app.ui.prepare.panels.take_changed() {
        actions.push(Action::RememberPanels(users));
    }
    if let Some(open) = inspector::blocks::take_changed(&ctx) {
        actions.push(Action::RememberSections(open));
    }
    if let Some(ids) = inspector::blocks::take_hidden_tips(&ctx) {
        actions.push(Action::HideTips(ids));
    }
    // Number fields left with a text they could not read say so.
    for text in calc::take_toasts(&ctx) {
        actions.push(Action::Error(text));
    }
    // What a click changed shows in the menu bar next frame.
    if !actions.is_empty() {
        ctx.request_repaint();
    }
    app.apply(&ctx, actions);

    // A reveal command that failed after it started (the file manager
    // said no) shows why.
    for why in app.revealer.failures() {
        app.toast(Toast::error(format!("Cannot show the file: {why}")));
    }
    let now = (app.clock)();
    app.check_sources(&ctx, now);
    let next = toasts::prune(&mut app.ui.toasts, now);
    let reduce = !app.ui.toasts.is_empty() && (app.reduce_motion)();
    let t = app.tokens;
    // Over the canvas, so no notice covers Send or a cut's rating.
    let place = match app.ui.workspace {
        Workspace::Prepare => Some(toasts::Place {
            canvas: app.ui.prepare.canvas_rect,
            report: app.ui.prepare.report_rect,
            pills: app.ui.prepare.pills_rect,
            bar: app.ui.prepare.keep_pick.as_ref().and_then(|k| k.bar_rect()),
        }),
        Workspace::Cut => Some(toasts::Place {
            canvas: app.ui.cut.canvas_rect,
            report: None,
            pills: None,
            bar: None,
        }),
        Workspace::Home => None,
    };
    let events = toasts::show(&ctx, &mut app.ui.toasts, &t, now, reduce, place);
    // Repaint often only while a countdown ring moves; otherwise when the
    // next toast expires.
    if !reduce && app.ui.toasts.iter().any(Toast::running) {
        ctx.request_repaint_after(Duration::from_millis(33));
    } else if let Some(next) = next {
        ctx.request_repaint_after(next);
    }
    for e in events {
        match e {
            toasts::Event::Reveal(file) => app.apply(&ctx, vec![Action::Reveal(file)]),
            toasts::Event::Close(id) => app.ui.toasts.retain(|t| t.id != id),
        }
    }
    if app.agent {
        // Keeps the agent pill current.
        ctx.request_repaint_after(Duration::from_secs(1));
    }
    app.frame_deadline = None;
}

impl FlatCraftApp {
    /// The menu bar's actions that may run now. The menu lives outside
    /// egui, so a dialog does not block it: while one is open (see
    /// [`modal_open`]) only Quit goes through, which asks about unsaved
    /// changes itself.
    pub fn menu_gate(&self, actions: Vec<Action>) -> Vec<Action> {
        self.menu_gate_with(actions, false)
    }

    /// [`Self::menu_gate`], knowing whether a text field has the keyboard.
    /// The Edit items then act on that field only (a dialog's name field,
    /// say), never on the document, so they go through a dialog too.
    pub fn menu_gate_with(&self, actions: Vec<Action>, text_focused: bool) -> Vec<Action> {
        if !modal_open(self) {
            return actions;
        }
        actions
            .into_iter()
            .filter(|a| match a {
                Action::Quit => true,
                Action::Menu(cmd) => text_focused && cmd.edits_text(),
                _ => false,
            })
            .collect()
    }
}

/// A dialog that owns the keyboard is open: About, Shortcuts, Trace, Rename, Export, unsaved changes
/// or the agent's pending send. The docked Arrange panels are not dialogs:
/// the document stays usable behind them.
pub fn modal_open(app: &FlatCraftApp) -> bool {
    app.ui.about.open
        || app.ui.show_settings
        || app.ui.show_shortcuts
        || app.ui.trace.is_some()
        || app.ui.prepare.rename.is_some()
        || app.ui.unsaved.is_some()
        || app.ui.export.is_some()
        || app.ui.troubleshoot.open
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
    let waiting = app.send_waiting();
    let busy = app.long.busy();
    if dirty.is_empty() && !waiting && !busy {
        return;
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    if dirty.is_empty() {
        if app.ui.unsaved.is_none() {
            app.ui.unsaved = Some(if busy {
                Unsaved::Busy {
                    answered: Vec::new(),
                }
            } else {
                Unsaved::PendingSend
            });
        }
    } else if !matches!(app.ui.unsaved, Some(Unsaved::Quit { .. })) {
        app.ui.unsaved = Some(Unsaved::Quit {
            asking: dirty[0],
            answered: Vec::new(),
        });
    }
}

/// The cards of the unsaved-changes dialog's buttons.
pub const SAVE_HINT: &str = "Save the changes to the project file, then go on.";
pub const DONT_SAVE_HINT: &str =
    "Throw away the changes since the last save and go on. This cannot be undone.";
pub const KEEP_OPEN_HINT: &str = "Stop here and keep the document open with its changes.";
/// The cards of "Quit while FlatCraft is still working?".
pub const QUIT_BUSY_HINT: &str =
    "Quit FlatCraft now. The import, trace or export that runs is lost.";
pub const STAY_BUSY_HINT: &str = "Keep FlatCraft open and go on working. Nothing quits.";
/// The cards of "Quit while an agent waits?".
pub const QUIT_ANYWAY_HINT: &str =
    "Quit FlatCraft and drop the agent's cut request. Nothing is sent.";
pub const STAY_WAITING_HINT: &str = "Stay in FlatCraft. The agent's request keeps waiting for you.";
/// The cards of the agent's cut request.
pub const ALLOW_HINT: &str = "Send this cut to the plotter now. Check the mat and the blade first.";
pub const DENY_HINT: &str = "Say no. Nothing is sent and the request is dropped.";

/// The unsaved-changes dialog: "Save changes to <name>?" with Don't
/// Save, Cancel and Save, for closing a document and, one document after
/// the other, for quitting. Quitting while an agent waits for a send asks
/// about that.
fn unsaved_modal(ctx: &egui::Context, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let Some(asked) = app.ui.unsaved.clone() else {
        return;
    };
    let d = unsaved_dialog(app, &asked);
    let t = app.tokens;
    if let Some(answer) = d.show(ctx, &t, |_| {}) {
        let choice = match answer {
            dialog::Answer::Primary => UnsavedChoice::Save,
            dialog::Answer::Secondary => UnsavedChoice::Discard,
            dialog::Answer::Cancel => UnsavedChoice::Cancel,
        };
        actions.push(Action::Unsaved(asked, choice));
    }
}

/// The dialog of an unsaved-changes question.
pub fn unsaved_dialog(app: &FlatCraftApp, asked: &Unsaved) -> dialog::Dialog {
    use dialog::{Dialog, Kind};
    let doc = match asked {
        Unsaved::PendingSend => {
            return Dialog::new("unsaved", Kind::Warning, "Quit while an agent waits?")
                .body("An agent is waiting for your OK to send a cut.")
                .body("If you quit, the request is dropped and nothing is sent.")
                .secondary("Quit anyway", false)
                .secondary_hint(QUIT_ANYWAY_HINT)
                .cancel_hint(STAY_WAITING_HINT);
        }
        Unsaved::Busy { .. } => {
            let note = app.long.note().unwrap_or_else(|| "Working...".into());
            return Dialog::new(
                "unsaved",
                Kind::Warning,
                "Quit while FlatCraft is still working?",
            )
            .body(format!("FlatCraft is still at it: {note}"))
            .body("FlatCraft quits by itself when it is done. If you quit now, that work is lost: an import is not added and an export is not written.")
            .secondary("Quit anyway", false)
            .secondary_hint(QUIT_BUSY_HINT)
            .cancel_hint(STAY_BUSY_HINT);
        }
        Unsaved::Close(doc_id) => *doc_id,
        Unsaved::Quit { asking, .. } => *asking,
    };
    let name = app.doc_name(doc);
    let mut d = Dialog::new("unsaved", Kind::Warning, format!("Save changes to {name}?"))
        .about(doc)
        .body(format!(
            "{name} has unsaved changes. If you don't save, they are lost."
        ))
        .secondary("Don't Save", true)
        .secondary_hint(DONT_SAVE_HINT)
        .primary("Save")
        .primary_hint(SAVE_HINT)
        .cancel_hint(KEEP_OPEN_HINT);
    if let Unsaved::Quit { asking, answered } = asked {
        // One more line at most: how many follow, and the waiting agent.
        let more = app
            .dirty_docs()
            .into_iter()
            .filter(|d| d != asking && !app.still_answered(answered, *d))
            .count();
        let waiting = app.send_waiting();
        let count = match more {
            0 => None,
            1 => Some("1 more document has unsaved changes".to_string()),
            n => Some(format!("{n} more documents have unsaved changes")),
        };
        let line = match (count, waiting) {
            (Some(c), true) => Some(format!(
                "{c}, and quitting drops the agent's waiting cut request."
            )),
            (Some(c), false) => Some(format!("{c}.")),
            (None, true) => Some(
                "An agent is also waiting for your OK to send a cut. Quitting drops that request."
                    .to_string(),
            ),
            (None, false) => None,
        };
        if let Some(line) = line {
            d = d.body(line);
        }
    }
    d
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
        // A theme staged in the Settings window shows while it is open.
        let theme = app.ui.settings.preview_theme.unwrap_or(s.prefs.theme);
        (theme, s.take_prefs_warning())
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
    // Escape in another text field (the search box) only leaves it: egui
    // dropped its focus, the panel and the selection stay.
    if st.text_focus_elsewhere && help::pressed(ctx, help::PrepareKey::ClearSelection) {
        return;
    }
    // Escape on an open list (a ribbon group's list) only closes it: egui
    // closed it before this runs, so the last frame says it was open.
    if st.popup_was_open && help::pressed(ctx, help::PrepareKey::ClearSelection) {
        return;
    }
    // Escape while a box or lasso is drawn cancels it: the selection
    // stays as it was. A lasso of the Lasso tool also goes back to the
    // Select tool, in the same press.
    if matches!(st.drag, Some(prepare::Drag::Frame { .. }))
        && help::pressed(ctx, help::PrepareKey::ClearSelection)
    {
        st.drag = None;
        st.select_tool = prepare::SelectTool::Select;
        return;
    }
    // While the keep picker is on, Escape cancels it and the other keys
    // wait: the selection it works on must not change under it.
    if st.keep_pick.is_some() {
        if help::pressed(ctx, help::PrepareKey::ClearSelection) {
            st.keep_pick = None;
        }
        return;
    }
    // Escape closes an open Arrange panel first, and changes nothing. One
    // press is enough while a panel field has the keys: egui drops the
    // focus on Escape before this runs.
    if st.panel.is_some() && help::pressed(ctx, help::PrepareKey::ClearSelection) {
        st.panel = None;
        return;
    }
    // Then Escape takes the Job panel shown over a selection back to the
    // selection, and changes nothing else.
    if inspector::job_over_selection(st, snap)
        && help::pressed(ctx, help::PrepareKey::ClearSelection)
    {
        st.inspector = inspector::InspectorView::Selection;
        return;
    }
    // The Select/Lasso toggle; Escape takes the Lasso tool back to Select.
    if help::pressed(ctx, help::PrepareKey::LassoTool) {
        st.select_tool = prepare::SelectTool::Lasso;
        return;
    }
    if help::pressed(ctx, help::PrepareKey::SelectTool)
        || (st.select_tool == prepare::SelectTool::Lasso
            && help::pressed(ctx, help::PrepareKey::ClearSelection))
    {
        st.select_tool = prepare::SelectTool::Select;
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
    // Shift+Cmd+G first: Cmd+G must not take it.
    for (key, act) in [
        (help::PrepareKey::Ungroup, tools::Act::Ungroup),
        (help::PrepareKey::Group, tools::Act::Group),
    ] {
        if help::pressed(ctx, key) {
            let t = tools::by_act(act);
            if t.blocked(snap).is_none() {
                actions.push(if act == tools::Act::Group {
                    tools::group_action(snap.doc_id, &ids)
                } else {
                    tools::ungroup_action(snap.doc_id, &ids)
                });
            }
            return;
        }
    }
    // The stacking order, Shift variants first: Cmd+] must not take
    // Shift+Cmd+].
    for (key, to) in [
        (help::PrepareKey::BringToFront, ReorderTo::Front),
        (help::PrepareKey::SendToBack, ReorderTo::Back),
        (help::PrepareKey::BringForward, ReorderTo::Forward),
        (help::PrepareKey::SendBackward, ReorderTo::Backward),
    ] {
        if help::pressed(ctx, key) {
            if tools::by_act(tools::Act::Order(to)).blocked(snap).is_none() {
                actions.push(tools::reorder_action(snap.doc_id, &ids, to));
            }
            return;
        }
    }
    canvas::nudge_keys(ctx, snap, st, now, actions);
    let delete = help::pressed(ctx, help::PrepareKey::Delete);
    let escape = help::pressed(ctx, help::PrepareKey::ClearSelection);
    if delete {
        actions.push(ribbon::delete_action(snap.doc_id, &ids));
    } else if escape {
        // Inside a group, Escape leaves it: the whole group is selected.
        let ids = canvas::leave_group(snap, st).unwrap_or_default();
        actions.push(Action::run(
            snap.doc_id,
            "object.select",
            json!({ "ids": ids }),
        ));
    }
}

/// A text field has the keys and it is not in the Arrange panel at
/// `panel` (screen area).
fn text_focus_outside(ctx: &egui::Context, panel: Option<egui::Rect>) -> bool {
    if !ctx.text_edit_focused() {
        return false;
    }
    let rect = ctx
        .memory(|m| m.focused())
        .and_then(|id| ctx.read_response(id))
        .map(|r| r.rect);
    !matches!((panel, rect), (Some(p), Some(r)) if p.contains_rect(r))
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

/// "Allow the agent to cut on <plotter>?" in every workspace, on the
/// shared dialog, safety first: Deny is the Cancel button and the default,
/// Allow is the left action with no key. Enter, Escape, × and a click
/// outside all deny; only a click on Allow allows. Allow confirms exactly
/// the request shown (its id).
fn pending_modal(ctx: &egui::Context, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let Some(p) = pending_view(app) else {
        return;
    };
    ctx.request_repaint_after(p.left);
    let t = app.tokens;
    let d = pending_dialog(&p);
    let answer = d.show(ctx, &t, |ui| {
        for w in &p.summary.warnings {
            ui.add(egui::Label::new(egui::RichText::new(w).color(t.warn_text)).wrap());
        }
    });
    match answer {
        Some(dialog::Answer::Secondary) => actions.push(Action::ConfirmPending(p.id)),
        Some(_) => actions.push(Action::DenyPending(p.id)),
        None => {}
    }
}

/// The dialog of an agent's pending send.
pub fn pending_dialog(p: &PendingView) -> dialog::Dialog {
    let s = &p.summary;
    let paths = if s.toolpaths == 1 {
        "1 path".to_string()
    } else {
        format!("{} paths", s.toolpaths)
    };
    let passes = if s.passes == 1 {
        "1 pass".to_string()
    } else {
        format!("{} passes", s.passes)
    };
    dialog::Dialog::new(
        "agent-send",
        dialog::Kind::Warning,
        format!("Allow the agent to cut on {}?", p.plotter),
    )
    .about(p.id)
    .body(format!(
        "{paths}, {} cut, speed {}, force {}, {passes}.",
        cut::mm_text(s.cut_mm),
        s.speed,
        s.force
    ))
    .body("Check the mat and the blade first. The request expires after 5 minutes.")
    .secondary("Allow", false)
    .secondary_hint(ALLOW_HINT)
    .cancel("Deny")
    .cancel_hint(DENY_HINT)
}

impl eframe::App for FlatCraftApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            let active = lock(&self.session).doc().id();
            let actions = self.menu.as_mut().map(|m| m.actions(active));
            if let Some(actions) = actions {
                let actions = self.menu_gate_with(actions, ctx.text_edit_focused());
                self.apply(ctx, actions);
            }
        }
        guard_close(ctx, self);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        shell(ui, self);
    }
}
