//! The Help menu (the app bar's Help button and the macOS menu bar), the
//! keyboard shortcut table that drives both the key handler and the
//! shortcuts sheet, and the "Report a problem" link.

use egui::{Key, KeyboardShortcut, Modifiers, RichText};

use super::about::BUILD;
use super::theme::Tokens;
use super::{Action, fonts};

pub const GUIDE_URL: &str = "https://github.com/jordiboehme/flatcraft#readme";
pub const BUG_URL: &str =
    "https://github.com/jordiboehme/flatcraft/issues/new?template=bug_report.yml";
pub const PLOTTER_URL: &str =
    "https://github.com/jordiboehme/flatcraft/issues/new?template=plotter_support.yml";

/// The items of the Help menu, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpItem {
    UserGuide,
    Shortcuts,
    ReportProblem,
    AskPlotter,
    About,
}

impl HelpItem {
    pub const ALL: [HelpItem; 5] = [
        HelpItem::UserGuide,
        HelpItem::Shortcuts,
        HelpItem::ReportProblem,
        HelpItem::AskPlotter,
        HelpItem::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HelpItem::UserGuide => "User guide",
            HelpItem::Shortcuts => "Keyboard shortcuts",
            HelpItem::ReportProblem => "Report a problem",
            HelpItem::AskPlotter => "Ask for your plotter",
            HelpItem::About => "About FlatCraft",
        }
    }

    pub fn action(self) -> Action {
        match self {
            HelpItem::UserGuide => Action::OpenUrl(GUIDE_URL.to_string()),
            HelpItem::Shortcuts => Action::ShowShortcuts,
            HelpItem::ReportProblem => Action::OpenUrl(bug_url()),
            HelpItem::AskPlotter => Action::OpenUrl(PLOTTER_URL.to_string()),
            HelpItem::About => Action::ShowAbout,
        }
    }
}

/// The text FlatCraft puts in the bug form's "System" field.
pub fn system() -> String {
    format!(
        "{} {}",
        match std::env::consts::OS {
            "macos" => "macOS",
            "windows" => "Windows",
            "linux" => "Linux",
            other => other,
        },
        std::env::consts::ARCH
    )
}

/// Percent-encodes a query value (RFC 3986 unreserved characters stay).
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The bug form with the version, build and system filled in (the ids are
/// the field ids of `.github/ISSUE_TEMPLATE/bug_report.yml`).
pub fn bug_url() -> String {
    let version = format!("{} ({BUILD})", env!("CARGO_PKG_VERSION"));
    format!(
        "{BUG_URL}&version={}&system={}",
        encode(&version),
        encode(&system())
    )
}

/// The Help popup's items. Draws them into `ui` and collects the actions.
pub fn menu(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    for item in HelpItem::ALL {
        if ui.button(item.label()).clicked() {
            actions.push(item.action());
        }
    }
}

/// One keyboard shortcut of the window.
pub struct Shortcut {
    pub modifiers: Modifiers,
    pub key: Key,
    pub what: &'static str,
    /// The action it runs, for the document it is pressed in. `None` for
    /// the Prepare keys, which [`super::prepare_keys`] handles.
    pub run: Option<fn(u64) -> Action>,
    /// On macOS the menu bar owns this key, not the key handler.
    pub menu_on_macos: bool,
}

const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers {
    command: true,
    shift: true,
    ..Modifiers::NONE
};

const fn global(
    modifiers: Modifiers,
    key: Key,
    what: &'static str,
    run: fn(u64) -> Action,
    menu_on_macos: bool,
) -> Shortcut {
    Shortcut {
        modifiers,
        key,
        what,
        run: Some(run),
        menu_on_macos,
    }
}

const fn prepare(modifiers: Modifiers, key: Key, what: &'static str) -> Shortcut {
    Shortcut {
        modifiers,
        key,
        what,
        run: None,
        menu_on_macos: false,
    }
}

/// Every shortcut the app has. The key handler runs the entries with a
/// `run`, in this order (a Shift variant before its plain key), and the
/// shortcuts sheet lists all of them.
pub const SHORTCUTS: [Shortcut; 13] = [
    global(
        CMD_SHIFT,
        Key::S,
        "Save As",
        |d| Action::SaveAs { doc_id: d },
        false,
    ),
    global(CMD, Key::S, "Save", |d| Action::Save { doc_id: d }, false),
    global(CMD, Key::O, "Open", |_| Action::OpenDialog, false),
    global(CMD, Key::N, "New project", |_| Action::NewProject, false),
    global(CMD, Key::Comma, "Settings", |_| Action::ShowProfiles, true),
    global(CMD_SHIFT, Key::Z, "Redo", Action::Redo, true),
    global(CMD, Key::Y, "Redo", Action::Redo, true),
    global(CMD, Key::Z, "Undo", Action::Undo, true),
    prepare(CMD, Key::A, "Select all objects"),
    prepare(
        Modifiers::NONE,
        Key::ArrowLeft,
        "Nudge the selection by 1 mm (Shift 10 mm, Alt 0.1 mm)",
    ),
    prepare(Modifiers::NONE, Key::Delete, "Delete the selection"),
    prepare(Modifiers::NONE, Key::Backspace, "Delete the selection"),
    prepare(Modifiers::NONE, Key::Escape, "Clear the selection"),
];

impl Shortcut {
    /// "Cmd+Shift+S" on macOS, "Ctrl+Shift+S" elsewhere.
    pub fn keys(&self) -> String {
        let mut s = String::new();
        if self.modifiers.command {
            s.push_str(if cfg!(target_os = "macos") {
                "Cmd+"
            } else {
                "Ctrl+"
            });
        }
        if self.modifiers.shift {
            s.push_str("Shift+");
        }
        s.push_str(match self.key {
            Key::Comma => ",",
            Key::ArrowLeft => "Arrow keys",
            Key::Escape => "Esc",
            other => other.name(),
        });
        s
    }
}

/// Runs the key handler's part of [`SHORTCUTS`]. On macOS the menu bar owns
/// Undo, Redo and Settings.
pub fn run_shortcuts(ctx: &egui::Context, active: u64, actions: &mut Vec<Action>) {
    ctx.input_mut(|i| {
        for s in &SHORTCUTS {
            let Some(run) = s.run else { continue };
            if s.menu_on_macos && cfg!(target_os = "macos") {
                continue;
            }
            if i.consume_shortcut(&KeyboardShortcut::new(s.modifiers, s.key)) {
                actions.push(run(active));
            }
        }
    });
}

/// The shortcuts sheet. Returns false once it should close.
pub fn show_shortcuts(ctx: &egui::Context, t: &Tokens) -> bool {
    let modal = egui::Modal::new(egui::Id::unique("flatcraft-shortcuts")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new("Keyboard shortcuts").font(fonts::semibold(16.0)));
        ui.add_space(t.spacing);
        egui::Grid::new("flatcraft-shortcut-grid")
            .num_columns(2)
            .spacing([24.0, 4.0])
            .show(ui, |ui| {
                for s in &SHORTCUTS {
                    ui.label(RichText::new(s.keys()).monospace());
                    ui.label(s.what);
                    ui.end_row();
                }
            });
        ui.add_space(t.spacing);
        ui.button("Close").clicked()
    });
    !(modal.inner || modal.should_close())
}
