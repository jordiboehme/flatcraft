//! The macOS menu bar as data. [`MenuModel::build`] is a pure function of
//! what the window knows (the workspace, the snapshot of the document, the
//! Prepare state), so it is tested without NSApp. The glue in `menu`
//! (macOS only) turns a model into muda items and updates them when the
//! next model differs from the last one.
//!
//! The menu holds no rules of its own. Titles, keys and the reasons an
//! item cannot run come from the same tables as the ribbon and the context
//! menu: [`tools::TOOLS`], [`help::SHORTCUTS`], [`context_menu::Item`]. A
//! click becomes an [`Action`] ([`Cmd::action`]), so [`FlatCraftApp::menu_gate`]
//! covers every item. macOS shows a dimmed item without a reason; the
//! reasons stay in the in-app UI.
//!
//! Keys: only keys with Cmd become accelerators. Delete, Backspace, the
//! arrows, Escape and Space never do, because a menu would take them away
//! from the text fields.
//!
//! [`FlatCraftApp::menu_gate`]: super::FlatCraftApp::menu_gate

// The menu bar itself exists on macOS only; the model is built everywhere
// so that its tests run on every system.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::PathBuf;

use egui::Key;

use super::context_menu::{self, Item as Obj};
use super::help::{self, HelpItem, SHORTCUTS};
use super::prepare::{PrepareState, Snapshot};
use super::tools::{self, Act, AlignTo, Section, Tool};
use super::{Action, MENU_RECENT, Workspace, cut, project_name};

/// The title of File > Export... (menu bar and app bar).
pub const EXPORT_TITLE: &str = "Export...";

/// A key with Cmd held, plus Shift or Alt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accel {
    pub key: Key,
    pub shift: bool,
    pub alt: bool,
}

impl Accel {
    /// The accelerator of the shortcut row `what`, if it has Cmd.
    fn of(what: &str) -> Option<Accel> {
        let row = SHORTCUTS.iter().find(|s| s.what == what)?;
        Accel::of_row(row)
    }

    fn of_row(row: &help::Shortcut) -> Option<Accel> {
        // Bare keys stay with the key handler (see the module docs).
        let key = row.key()?;
        row.modifiers.command.then_some(Accel {
            key,
            shift: row.modifiers.shift,
            alt: row.modifiers.alt,
        })
    }

    const fn cmd(key: Key) -> Accel {
        Accel {
            key,
            shift: false,
            alt: false,
        }
    }
}

/// What a menu item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    About,
    Settings,
    Quit,
    New,
    Open,
    OpenRecent(usize),
    /// The dimmed line of an empty Open Recent list.
    NoRecent,
    Save,
    SaveAs,
    /// File > Export...: the Export dialog.
    Export,
    Close,
    SendToFile,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    /// A ribbon action, by [`Tool::id`].
    Tool(&'static str),
    LinkCopies,
    Unlink,
    /// Skip cutting, or Cut again: the title says which.
    ToggleCut,
    /// Hide, or Show.
    ToggleHide,
    Rename,
    Reconnect,
    RevealSource,
    CopySourcePath,
    Workspace(Workspace),
    Help(HelpItem),
}

impl Cmd {
    /// The id of its muda item.
    pub fn id(&self) -> String {
        let fixed = match self {
            Cmd::About => "about",
            Cmd::Settings => "preferences",
            Cmd::Quit => "quit",
            Cmd::New => "new",
            Cmd::Open => "open",
            Cmd::NoRecent => "no-recent",
            Cmd::Save => "save",
            Cmd::SaveAs => "save-as",
            Cmd::Export => "export",
            Cmd::Close => "close",
            Cmd::SendToFile => "send",
            Cmd::Undo => "undo",
            Cmd::Redo => "redo",
            Cmd::Cut => "cut",
            Cmd::Copy => "copy",
            Cmd::Paste => "paste",
            Cmd::SelectAll => "select-all",
            Cmd::LinkCopies => "link-copies",
            Cmd::Unlink => "unlink",
            Cmd::ToggleCut => "toggle-cut",
            Cmd::ToggleHide => "toggle-hide",
            Cmd::Rename => "rename",
            Cmd::Reconnect => "reconnect",
            Cmd::RevealSource => "reveal-source",
            Cmd::CopySourcePath => "copy-source-path",
            Cmd::OpenRecent(i) => return format!("flatcraft.recent.{i}"),
            Cmd::Tool(id) => return format!("flatcraft.tool.{id}"),
            Cmd::Workspace(w) => return format!("flatcraft.workspace.{}", w.name()),
            Cmd::Help(item) => return format!("flatcraft.help.{}", item.label()),
        };
        format!("flatcraft.{fixed}")
    }

    /// The command of a muda item id.
    pub fn parse(id: &str) -> Option<Cmd> {
        let rest = id.strip_prefix("flatcraft.")?;
        if let Some(i) = rest.strip_prefix("recent.") {
            return i.parse().ok().map(Cmd::OpenRecent);
        }
        if let Some(t) = rest.strip_prefix("tool.") {
            return tools::by_id(t).map(|t| Cmd::Tool(t.id));
        }
        if let Some(w) = rest.strip_prefix("workspace.") {
            return [Workspace::Prepare, Workspace::Cut]
                .into_iter()
                .find(|x| x.name() == w)
                .map(Cmd::Workspace);
        }
        if let Some(h) = rest.strip_prefix("help.") {
            return HelpItem::ALL
                .into_iter()
                .find(|i| i.label() == h)
                .map(Cmd::Help);
        }
        [
            Cmd::About,
            Cmd::Settings,
            Cmd::Quit,
            Cmd::New,
            Cmd::Open,
            Cmd::NoRecent,
            Cmd::Save,
            Cmd::SaveAs,
            Cmd::Export,
            Cmd::Close,
            Cmd::SendToFile,
            Cmd::Undo,
            Cmd::Redo,
            Cmd::Cut,
            Cmd::Copy,
            Cmd::Paste,
            Cmd::SelectAll,
            Cmd::LinkCopies,
            Cmd::Unlink,
            Cmd::ToggleCut,
            Cmd::ToggleHide,
            Cmd::Rename,
            Cmd::Reconnect,
            Cmd::RevealSource,
            Cmd::CopySourcePath,
        ]
        .into_iter()
        .find(|c| c.id() == id)
    }

    /// The Edit items: with a text field focused they act on its text.
    pub fn edits_text(&self) -> bool {
        matches!(
            self,
            Cmd::Undo | Cmd::Redo | Cmd::Cut | Cmd::Copy | Cmd::Paste | Cmd::SelectAll
        )
    }

    /// The action of a click, for the document `active` and the recent
    /// list the menu shows. The items that need the snapshot or the
    /// keyboard focus become [`Action::Menu`].
    pub fn action(&self, active: u64, recent: &[PathBuf]) -> Option<Action> {
        Some(match self {
            Cmd::About => Action::ShowAbout,
            Cmd::Settings => Action::ShowSettings,
            Cmd::Quit => Action::Quit,
            Cmd::New => Action::NewProject,
            Cmd::Open => Action::OpenDialog,
            Cmd::OpenRecent(i) => Action::Open(recent.get(*i)?.clone()),
            Cmd::NoRecent => return None,
            Cmd::Save => Action::Save { doc_id: active },
            Cmd::SaveAs => Action::SaveAs { doc_id: active },
            Cmd::Export => Action::ExportDialog { doc_id: active },
            Cmd::Close => Action::Close(active),
            Cmd::SendToFile => Action::Send { doc_id: active },
            Cmd::Workspace(w) => Action::SetWorkspace(*w),
            Cmd::Help(item) => item.action(),
            other => Action::Menu(other.clone()),
        })
    }
}

/// A menu item that needs no code of ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predef {
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    BringAllToFront,
}

/// One clickable item.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    pub cmd: Cmd,
    pub title: String,
    pub enabled: bool,
    /// `Some` for an item with a check mark.
    pub checked: Option<bool>,
    pub accel: Option<Accel>,
}

/// What macOS does with a menu besides showing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plain,
    /// The Window menu: macOS lists the open windows in it.
    Window,
    /// The Help menu: macOS adds its search field.
    Help,
}

/// A menu or a submenu.
#[derive(Debug, Clone, PartialEq)]
pub struct Sub {
    /// Stable, so an update finds the menu again.
    pub id: String,
    pub title: String,
    pub enabled: bool,
    pub role: Role,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Item(Leaf),
    Separator,
    Predef(Predef),
    Sub(Sub),
}

/// The whole menu bar: the app menu first, then the menus in order.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuModel {
    pub menus: Vec<Sub>,
    /// The projects of the Open Recent items, by item number.
    pub recent: Vec<PathBuf>,
}

/// What the model is built from.
pub struct MenuInput<'a> {
    pub workspace: Workspace,
    /// The document as Prepare draws it; `None` on Home.
    pub snap: Option<&'a Snapshot>,
    pub prepare: &'a PrepareState,
    pub show_debug: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// A text field has the keyboard: the Edit items work on its text.
    pub text_focused: bool,
    /// The Cut stage has a job that can be sent.
    pub send_ready: bool,
    pub recent: &'a [PathBuf],
    /// A dialog owns the keyboard (see `ui::modal_open`): only Quit, and
    /// the Edit items of a focused text field, stay enabled. A dimmed
    /// item lets its key through to the dialog (Cmd+D is Don't Save there,
    /// not Duplicate).
    pub modal: bool,
}

impl MenuInput<'_> {
    /// The ids of the selected objects (none on Home).
    fn selection(&self) -> Vec<u64> {
        self.snap
            .map(|s| s.selection.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Object and View items work in Prepare, where the canvas is.
    fn prepare_open(&self) -> bool {
        self.workspace == Workspace::Prepare && self.snap.is_some()
    }
}

/// Hide while any of the objects is shown, else Show (Hide for none).
fn hide_item(snap: &Snapshot, ids: &[u64]) -> Obj {
    let mut objs = ids.iter().filter_map(|id| snap.object(*id)).peekable();
    if objs.peek().is_none() || objs.any(|o| o.visible) {
        Obj::Hide
    } else {
        Obj::Show
    }
}

/// Cut again when the first object is skipped, else Skip cutting.
fn cut_item(snap: &Snapshot, ids: &[u64]) -> Obj {
    match ids.first().and_then(|id| snap.object(*id)) {
        Some(o) if !o.cut_enabled => Obj::CutAgain,
        _ => Obj::SkipCutting,
    }
}

/// The context-menu item a click on `cmd` runs on `ids` right now, if it
/// can run (the same rules that enable the menu item). The menu bar and
/// its click handler both ask here.
pub fn object_item(cmd: &Cmd, snap: &Snapshot, st: &PrepareState, ids: &[u64]) -> Option<Obj> {
    let want = match cmd {
        Cmd::LinkCopies => Obj::LinkCopies,
        Cmd::Unlink => Obj::UnlinkCopies,
        Cmd::Rename => Obj::Rename,
        Cmd::Reconnect => Obj::Reconnect,
        Cmd::RevealSource => Obj::RevealSource,
        Cmd::CopySourcePath => Obj::CopySourcePath,
        Cmd::ToggleHide => hide_item(snap, ids),
        Cmd::ToggleCut => cut_item(snap, ids),
        _ => return None,
    };
    let offered = context_menu::items(snap, st, ids)
        .into_iter()
        .flatten()
        .any(|i| i == want);
    (offered && want.why(snap, st, ids).is_none()).then_some(want)
}

impl MenuModel {
    pub fn build(inp: &MenuInput) -> MenuModel {
        let mut model = Self::build_open(inp);
        if inp.modal {
            fn gate(entries: &mut [Entry], text_focused: bool) {
                for e in entries {
                    match e {
                        Entry::Item(l) => {
                            l.enabled &= l.cmd == Cmd::Quit || (text_focused && l.cmd.edits_text());
                        }
                        Entry::Sub(s) => gate(&mut s.entries, text_focused),
                        _ => {}
                    }
                }
            }
            for m in &mut model.menus {
                gate(&mut m.entries, inp.text_focused);
            }
        }
        model
    }

    /// The model while no dialog is open.
    fn build_open(inp: &MenuInput) -> MenuModel {
        MenuModel {
            recent: inp.recent.iter().take(MENU_RECENT).cloned().collect(),
            menus: vec![
                app_menu(),
                file_menu(inp),
                edit_menu(inp),
                object_menu(inp),
                view_menu(inp),
                window_menu(),
                help_menu(),
            ],
        }
    }

    /// Every clickable item, in menu order.
    pub fn leaves(&self) -> Vec<&Leaf> {
        let mut out = Vec::new();
        for m in &self.menus {
            collect_leaves(&m.entries, &mut out);
        }
        out
    }

    /// Every menu and submenu, in order.
    pub fn subs(&self) -> Vec<&Sub> {
        let mut out = Vec::new();
        for m in &self.menus {
            out.push(m);
            collect_subs(&m.entries, &mut out);
        }
        out
    }

    /// What the muda tree is made of: the ids in order, with their nesting.
    /// Two models with the same shape differ only in titles, enabled and
    /// checked states, which muda changes in place.
    pub fn shape(&self) -> Vec<String> {
        fn walk(entries: &[Entry], out: &mut Vec<String>) {
            for e in entries {
                match e {
                    Entry::Item(l) => out.push(l.cmd.id()),
                    Entry::Separator => out.push("-".into()),
                    Entry::Predef(p) => out.push(format!("{p:?}")),
                    Entry::Sub(s) => {
                        out.push(format!("[{}", s.id));
                        walk(&s.entries, out);
                        out.push("]".into());
                    }
                }
            }
        }
        let mut out = Vec::new();
        for m in &self.menus {
            out.push(format!("[{}", m.id));
            walk(&m.entries, &mut out);
            out.push("]".into());
        }
        out
    }
}

fn collect_leaves<'a>(entries: &'a [Entry], out: &mut Vec<&'a Leaf>) {
    for e in entries {
        match e {
            Entry::Item(l) => out.push(l),
            Entry::Sub(s) => collect_leaves(&s.entries, out),
            _ => {}
        }
    }
}

fn collect_subs<'a>(entries: &'a [Entry], out: &mut Vec<&'a Sub>) {
    for e in entries {
        if let Entry::Sub(s) = e {
            out.push(s);
            collect_subs(&s.entries, out);
        }
    }
}

// ----------------------------------------------------------------- builders

fn leaf(cmd: Cmd, title: impl Into<String>, enabled: bool) -> Leaf {
    Leaf {
        cmd,
        title: title.into(),
        enabled,
        checked: None,
        accel: None,
    }
}

fn item(cmd: Cmd, title: impl Into<String>, enabled: bool, accel: Option<Accel>) -> Entry {
    Entry::Item(Leaf {
        accel,
        ..leaf(cmd, title, enabled)
    })
}

/// The item of a ribbon action: its title, key and reason come from the
/// table.
fn tool_item(id: &'static str, inp: &MenuInput) -> Entry {
    let t = tools::by_id(id).expect("the menu names actions of the table");
    // A text field wants the keys of Duplicate, Import and Delete.
    let field_has_keys = inp.text_focused && has_shortcut(t);
    let enabled =
        inp.prepare_open() && !field_has_keys && inp.snap.is_some_and(|s| t.blocked(s).is_none());
    Entry::Item(Leaf {
        checked: t.selected(inp.prepare, inp.show_debug).filter(|_| {
            matches!(
                t.act,
                Act::Grid | Act::Rulers | Act::ObjectsPanel | Act::SettingsPanel | Act::DebugPanel
            )
        }),
        accel: tool_accel(t),
        ..leaf(Cmd::Tool(t.id), t.title, enabled)
    })
}

/// Whether the action has a row in the shortcut table.
pub fn has_shortcut(t: &Tool) -> bool {
    SHORTCUTS.iter().any(|s| s.tool == Some(t.id))
}

/// The accelerator of an action: its row in the shortcut table, when the
/// row has Cmd.
fn tool_accel(t: &Tool) -> Option<Accel> {
    SHORTCUTS
        .iter()
        .find(|s| s.tool == Some(t.id))
        .and_then(Accel::of_row)
}

fn sub(id: impl Into<String>, title: impl Into<String>, entries: Vec<Entry>) -> Sub {
    let enabled = entries.iter().any(|e| match e {
        Entry::Item(l) => l.enabled,
        Entry::Sub(s) => s.enabled,
        _ => false,
    });
    Sub {
        id: id.into(),
        title: title.into(),
        enabled,
        role: Role::Plain,
        entries,
    }
}

fn app_menu() -> Sub {
    Sub {
        enabled: true,
        ..sub(
            "app",
            "FlatCraft",
            vec![
                item(Cmd::About, "About FlatCraft", true, None),
                Entry::Separator,
                item(
                    Cmd::Settings,
                    help::SETTINGS_LABEL,
                    true,
                    Accel::of(help::SETTINGS_LABEL),
                ),
                Entry::Separator,
                Entry::Predef(Predef::Services),
                Entry::Separator,
                Entry::Predef(Predef::Hide),
                Entry::Predef(Predef::HideOthers),
                Entry::Predef(Predef::ShowAll),
                Entry::Separator,
                item(Cmd::Quit, "Quit FlatCraft", true, Some(Accel::cmd(Key::Q))),
            ],
        )
    }
}

fn file_menu(inp: &MenuInput) -> Sub {
    let recent: Vec<Entry> = if inp.recent.is_empty() {
        vec![item(Cmd::NoRecent, "No recent projects yet", false, None)]
    } else {
        inp.recent
            .iter()
            .take(MENU_RECENT)
            .enumerate()
            .map(|(i, p)| item(Cmd::OpenRecent(i), project_name(p), true, None))
            .collect()
    };
    let recent = Sub {
        enabled: !inp.recent.is_empty(),
        ..sub("open-recent", "Open Recent", recent)
    };
    let send_title = match inp.snap {
        Some(snap) => cut::send_label(snap),
        None => "Send to File Output".to_string(),
    };
    let import = tool_item("import", inp);
    Sub {
        enabled: true,
        ..sub(
            "file",
            "File",
            vec![
                item(Cmd::New, "New", true, Accel::of("New project")),
                item(Cmd::Open, "Open...", true, Accel::of("Open")),
                Entry::Sub(recent),
                Entry::Separator,
                item(Cmd::Save, "Save", true, Accel::of("Save")),
                item(Cmd::SaveAs, "Save As...", true, Accel::of("Save As")),
                item(
                    Cmd::Export,
                    EXPORT_TITLE,
                    super::export_dialog::menu_why(inp.snap).is_none(),
                    None,
                ),
                item(
                    Cmd::Close,
                    "Close",
                    inp.workspace != Workspace::Home,
                    Accel::of("Close the document"),
                ),
                Entry::Separator,
                import,
                tool_item("test_patterns", inp),
                item(
                    Cmd::SendToFile,
                    send_title,
                    inp.workspace == Workspace::Cut && inp.send_ready,
                    None,
                ),
            ],
        )
    }
}

fn edit_menu(inp: &MenuInput) -> Sub {
    let text = inp.text_focused;
    let can_select =
        text || (inp.prepare_open() && inp.snap.is_some_and(|s| !s.objects.is_empty()));
    Sub {
        enabled: true,
        ..sub(
            "edit",
            "Edit",
            vec![
                item(Cmd::Undo, "Undo", inp.can_undo || text, Accel::of("Undo")),
                item(
                    Cmd::Redo,
                    "Redo",
                    inp.can_redo || text,
                    SHORTCUTS
                        .iter()
                        .find(|s| s.what == "Redo")
                        .and_then(Accel::of_row),
                ),
                Entry::Separator,
                // The keys go to a text field only while it has the focus;
                // a dimmed item lets the key through to the window.
                item(Cmd::Cut, "Cut", text, Some(Accel::cmd(Key::X))),
                item(Cmd::Copy, "Copy", text, Some(Accel::cmd(Key::C))),
                item(Cmd::Paste, "Paste", text, Some(Accel::cmd(Key::V))),
                item(
                    Cmd::SelectAll,
                    "Select All",
                    can_select,
                    Accel::of("Select all objects"),
                ),
            ],
        )
    }
}

fn object_menu(inp: &MenuInput) -> Sub {
    let ids = inp.selection();
    let on = |cmd: Cmd| match (inp.prepare_open(), inp.snap) {
        (true, Some(snap)) => object_item(&cmd, snap, inp.prepare, &ids).is_some(),
        _ => false,
    };
    // The titles that flip with the state of the selection.
    let (hide, skip) = match inp.snap {
        Some(snap) => (hide_item(snap, &ids), cut_item(snap, &ids)),
        None => (Obj::Hide, Obj::SkipCutting),
    };

    let align = |to: AlignTo| -> Entry {
        let rows = tools::TOOLS
            .iter()
            .filter(|t| matches!(t.act, Act::Align(_, o) if o == to))
            .map(|t| tool_item(t.id, inp))
            .collect();
        Entry::Sub(sub(format!("align-{}", to.short()), to.label(), rows))
    };
    let align_menu = Entry::Sub(sub(
        "align",
        Section::Align.title(),
        vec![align(AlignTo::Selection), align(AlignTo::Area)],
    ));
    let distribute = Entry::Sub(sub(
        "distribute",
        Section::Distribute.title(),
        vec![
            tool_item("distribute.horizontal", inp),
            tool_item("distribute.vertical", inp),
        ],
    ));
    let arrange = Entry::Sub(sub(
        "arrange",
        tools::GroupId::Arrange.def().title,
        vec![
            tool_item("move_copy", inp),
            tool_item("rotate", inp),
            tool_item("rotate.left", inp),
            tool_item("rotate.right", inp),
            Entry::Separator,
            tool_item("mirror.horizontal", inp),
            tool_item("mirror.vertical", inp),
            Entry::Separator,
            align_menu,
            distribute,
            Entry::Separator,
            tool_item("pattern.rectangular", inp),
            tool_item("pattern.circular", inp),
            Entry::Separator,
            tool_item("auto_arrange.panel", inp),
        ],
    ));
    Sub {
        enabled: true,
        ..sub(
            "object",
            "Object",
            vec![
                tool_item("duplicate", inp),
                tool_item("copies", inp),
                arrange,
                Entry::Separator,
                item(
                    Cmd::LinkCopies,
                    Obj::LinkCopies.label(),
                    on(Cmd::LinkCopies),
                    None,
                ),
                item(
                    Cmd::Unlink,
                    Obj::UnlinkCopies.label(),
                    on(Cmd::Unlink),
                    None,
                ),
                Entry::Separator,
                item(Cmd::ToggleCut, skip.label(), on(Cmd::ToggleCut), None),
                item(Cmd::ToggleHide, hide.label(), on(Cmd::ToggleHide), None),
                item(Cmd::Rename, Obj::Rename.label(), on(Cmd::Rename), None),
                Entry::Separator,
                tool_item("trace", inp),
                tool_item("reload", inp),
                item(
                    Cmd::Reconnect,
                    Obj::Reconnect.label(),
                    on(Cmd::Reconnect),
                    None,
                ),
                item(
                    Cmd::RevealSource,
                    Obj::RevealSource.label(),
                    on(Cmd::RevealSource),
                    None,
                ),
                item(
                    Cmd::CopySourcePath,
                    Obj::CopySourcePath.label(),
                    on(Cmd::CopySourcePath),
                    None,
                ),
                Entry::Separator,
                tool_item("delete", inp),
            ],
        )
    }
}

fn view_menu(inp: &MenuInput) -> Sub {
    let workspace = |w: Workspace| {
        Entry::Item(Leaf {
            checked: Some(inp.workspace == w),
            ..leaf(Cmd::Workspace(w), w.name(), true)
        })
    };
    Sub {
        enabled: true,
        ..sub(
            "view",
            "View",
            vec![
                workspace(Workspace::Prepare),
                workspace(Workspace::Cut),
                Entry::Separator,
                tool_item("zoom.fit", inp),
                tool_item("zoom.100", inp),
                Entry::Separator,
                tool_item("view.grid", inp),
                tool_item("view.rulers", inp),
                Entry::Separator,
                tool_item("panel.objects", inp),
                tool_item("panel.settings", inp),
            ],
        )
    }
}

fn window_menu() -> Sub {
    Sub {
        enabled: true,
        role: Role::Window,
        ..sub(
            "window",
            "Window",
            vec![
                Entry::Predef(Predef::Minimize),
                Entry::Predef(Predef::Zoom),
                Entry::Separator,
                Entry::Predef(Predef::BringAllToFront),
            ],
        )
    }
}

fn help_menu() -> Sub {
    Sub {
        enabled: true,
        role: Role::Help,
        ..sub(
            "help",
            "Help",
            HelpItem::ALL
                .into_iter()
                .map(|h| item(Cmd::Help(h), h.label(), true, None))
                .collect(),
        )
    }
}
