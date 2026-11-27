//! The Help menu (the app bar's Help button and the macOS menu bar), the
//! keyboard shortcut table that drives both the key handler and the
//! shortcuts sheet, and the "Report a problem" link.

use crate::ui::metrics;
use egui::{Key, KeyboardShortcut, Modifiers, RichText};

use super::Action;
use super::about::BUILD;

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
    Troubleshoot,
    ReportProblem,
    AskPlotter,
    About,
}

impl HelpItem {
    pub const ALL: [HelpItem; 6] = [
        HelpItem::UserGuide,
        HelpItem::Shortcuts,
        HelpItem::Troubleshoot,
        HelpItem::ReportProblem,
        HelpItem::AskPlotter,
        HelpItem::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HelpItem::UserGuide => "User guide",
            HelpItem::Shortcuts => "Keyboard shortcuts",
            HelpItem::Troubleshoot => super::troubleshoot::LABEL,
            HelpItem::ReportProblem => "Report a problem",
            HelpItem::AskPlotter => "Ask for your plotter",
            HelpItem::About => "About FlatCraft",
        }
    }

    /// What the item does, for its hover card in the app bar's Help menu.
    pub fn hint(self) -> &'static str {
        match self {
            HelpItem::UserGuide => {
                "Opens the FlatCraft guide (the read-me on GitHub) in your browser."
            }
            HelpItem::Shortcuts => {
                "Shows every keyboard shortcut and mouse trick, with the keys of this computer."
            }
            HelpItem::Troubleshoot => super::troubleshoot::OPEN_HINT,
            HelpItem::ReportProblem => {
                "Opens the bug form on GitHub in your browser, with the FlatCraft version and your system filled in. Say what you did and what went wrong."
            }
            HelpItem::AskPlotter => {
                "Opens a form on GitHub in your browser to ask for support of your cutting plotter. Name the maker and model."
            }
            HelpItem::About => {
                "Shows the FlatCraft version and build, the credits and the licences of the parts it is built from."
            }
        }
    }

    pub fn action(self) -> Action {
        match self {
            HelpItem::UserGuide => Action::OpenUrl(GUIDE_URL.to_string()),
            HelpItem::Shortcuts => Action::ShowShortcuts,
            HelpItem::Troubleshoot => Action::Troubleshoot,
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
        let r = ui.button(item.label());
        super::groups::card(&r, item.label(), item.hint(), None, true);
        if r.clicked() {
            actions.push(item.action());
        }
    }
}

/// The name of the Settings item: the shortcut row and the macOS menu use it.
pub const SETTINGS_LABEL: &str = "Settings...";

/// What a Prepare key does. [`SHORTCUTS`] names the key, the handlers in
/// `prepare_keys`, `canvas::nudge_keys` and `canvas::pointer` ask the table
/// for it, so the sheet and the handlers cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareKey {
    SelectAll,
    Import,
    Duplicate,
    Group,
    Ungroup,
    /// The stacking order: Shift+Cmd+], Cmd+], Cmd+[ and Shift+Cmd+[.
    BringToFront,
    BringForward,
    SendBackward,
    SendToBack,
    /// One row per arrow key; Shift and Alt change the step.
    Nudge,
    Delete,
    ClearSelection,
    /// Held while dragging.
    Pan,
    /// Held while dragging a handle of the selection: the opposite of
    /// Keep aspect for that drag.
    FlipKeepAspect,
    /// Held while dragging a handle of the selection: it scales about the
    /// centre instead of the opposite corner or side.
    ScaleFromCentre,
    /// Dragging just outside a corner of the selection rotates it (no key).
    RotateZone,
    /// Held when a corner handle drag starts: it rotates instead.
    RotateOnCorner,
    /// Held while rotating: the angle snaps to 15 degree steps.
    SnapAngle,
    /// The Select tool: a drag on the empty canvas draws a selection box.
    SelectTool,
    /// The Lasso tool: a drag draws a free shape.
    LassoTool,
    /// Dragging on the empty canvas draws a selection box (no key).
    SelectBox,
    /// Held when a drag on the empty canvas starts: it draws a lasso.
    Lasso,
    /// Held while clicking or while drawing a box or lasso: each picked
    /// object flips in or out of the selection.
    ToggleInSelection,
}

/// The systems FlatCraft names keys for. Each has its own table of drag
/// modifiers ([`drag_modifiers`]); the window uses [`Os::CURRENT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

impl Os {
    pub const ALL: [Os; 3] = [Os::MacOs, Os::Windows, Os::Linux];

    /// The system this build runs on (any other Unix counts as Linux).
    pub const CURRENT: Os = if cfg!(target_os = "macos") {
        Os::MacOs
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        Os::Linux
    };
}

/// The modifiers held while dragging on the canvas, per system, following
/// each system's convention: Shift keeps or frees proportions and snaps
/// angles everywhere; Option (macOS) or Alt scales from the centre; Cmd
/// (macOS, as in Keynote and Sketch) or Ctrl (Windows, Linux) on a corner
/// rotates. The rotate zone needs no key. Key rows get `NONE`.
///
/// Selecting follows the apps people know: a drag on the empty canvas
/// draws a box that takes what is fully inside (Inkscape, LightBurn);
/// Option or Alt at the start of that drag draws a lasso that takes what
/// its line touches or encloses (Inkscape's Alt-drag touch path, the
/// Silhouette Studio lasso); Shift flips what is clicked or framed in or
/// out of the selection (Illustrator, Figma, Inkscape). The Lasso tool is
/// L (Photoshop), the Select tool V (Illustrator, Figma).
pub const fn drag_modifiers(os: Os, action: PrepareKey) -> Modifiers {
    match action {
        PrepareKey::FlipKeepAspect | PrepareKey::SnapAngle | PrepareKey::ToggleInSelection => {
            Modifiers::SHIFT
        }
        PrepareKey::ScaleFromCentre | PrepareKey::Lasso => Modifiers::ALT,
        PrepareKey::RotateOnCorner => match os {
            Os::MacOs => Modifiers::MAC_CMD,
            Os::Windows | Os::Linux => Modifiers::CTRL,
        },
        _ => Modifiers::NONE,
    }
}

/// The names of the modifiers `m` on `os`, joined by "+": "Cmd", "Option",
/// "Ctrl", "Alt", "Shift". The fonts have no key symbols, so they are
/// words.
pub fn modifier_names(m: Modifiers, os: Os) -> String {
    let mac = os == Os::MacOs;
    let mut names = Vec::new();
    if mac && (m.command || m.mac_cmd) {
        names.push("Cmd");
    }
    if m.ctrl || (!mac && m.command) {
        names.push("Ctrl");
    }
    if m.shift {
        names.push("Shift");
    }
    if m.alt {
        names.push(if mac { "Option" } else { "Alt" });
    }
    names.join("+")
}

/// The name of the key of a drag action on `os` ("Option", "Ctrl").
fn drag_key(os: Os, action: PrepareKey) -> String {
    modifier_names(drag_modifiers(os, action), os)
}

/// The hover card of the corner and side handles on `os`.
pub fn handle_card(os: Os) -> String {
    let centre = drag_key(os, PrepareKey::ScaleFromCentre);
    let flip = drag_key(os, PrepareKey::FlipKeepAspect);
    let rotate = drag_key(os, PrepareKey::RotateOnCorner);
    let mut s = format!(
        "Drag to resize the selection. A corner changes width and height, a side only its own, unless Keep aspect is on. Hold {flip} to get the opposite of Keep aspect for this drag, and {centre} to grow from the centre instead of the far side. Hold {rotate} when you start dragging a corner to rotate instead, like a dial."
    );
    if os == Os::Linux {
        s.push_str(&format!(
            " Some desktops use {centre}-drag to move windows: if so, change that key in your desktop settings."
        ));
    }
    s
}

/// The hover card of the rotate zone just outside a corner on `os`.
pub fn rotate_card(os: Os) -> String {
    let rotate = drag_key(os, PrepareKey::RotateOnCorner);
    let snap = drag_key(os, PrepareKey::SnapAngle);
    format!(
        "Drag to rotate the selection about its centre, clockwise or the other way round. Hold {snap} to turn in 15 degree steps. You can also hold {rotate} and drag a corner handle, or type the angle in Position and size."
    )
}

/// The hover card of the Angle field on `os`.
pub fn angle_card(os: Os) -> String {
    let rotate = drag_key(os, PrepareKey::RotateOnCorner);
    format!(
        "How far the selection is turned, in degrees clockwise (0 to 360). Type an angle to turn each selected object about its own centre; with several objects turned different ways the field says Mixed. It counts when you leave the field (Tab, Enter or a click elsewhere); Esc puts the old value back. To turn by hand, drag just outside a corner, or hold {rotate} and drag a corner."
    )
}

/// The hover card of the Select tool on `os`.
pub fn select_card(os: Os) -> String {
    let lasso = drag_key(os, PrepareKey::Lasso);
    let toggle = drag_key(os, PrepareKey::ToggleInSelection);
    let key = key_name(PrepareKey::SelectTool);
    format!(
        "Click an object to select it: on its line, or inside it where it is filled. Drag on the empty mat, or in the open middle of a frame, to draw a box: it selects every object that lies fully inside. Hold {toggle} when you click or start the drag to flip objects in or out of the selection, and {lasso} when you start the drag to draw a lasso instead. Key: {key}."
    )
}

/// The hover card of the Lasso tool on `os`.
pub fn lasso_card(os: Os) -> String {
    let lasso = drag_key(os, PrepareKey::Lasso);
    let toggle = drag_key(os, PrepareKey::ToggleInSelection);
    let key = key_name(PrepareKey::LassoTool);
    let back = key_name(PrepareKey::SelectTool);
    let mut s = format!(
        "Draw a free shape around what you want, like a cowboy with a rope: when you let go, every object the dashed line touches or encloses is selected. Good for packed layouts where a box would catch too much. A lasso drawn wholly inside a shape does not pick it: cross its line. Hold {toggle} when you start to flip objects in or out of the selection. Esc cancels the lasso and goes back to the Select tool. You can also hold {lasso} and drag on the empty mat with the Select tool. Key: {key} (back with {back} or Esc)."
    );
    if os == Os::Linux {
        s.push_str(&format!(
            " Some desktops use {lasso}-drag to move windows: if so, use this tool or change that key in your desktop settings."
        ));
    }
    s
}

/// The key of a key row of the table ("L").
fn key_name(action: PrepareKey) -> String {
    rows(action)
        .find_map(Shortcut::key)
        .map(|k| k.name().to_string())
        .unwrap_or_default()
}

/// The arrow keys of [`PrepareKey::Nudge`] (the table lists them as one row).
pub const NUDGE_KEYS: [Key; 4] = [
    Key::ArrowLeft,
    Key::ArrowRight,
    Key::ArrowUp,
    Key::ArrowDown,
];

/// What starts a shortcut besides its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A key press.
    Key(Key),
    /// Dragging a handle of the selection: only the modifiers count, held
    /// while dragging (see [`held`]).
    Drag,
}

/// One keyboard shortcut of the window.
pub struct Shortcut {
    pub modifiers: Modifiers,
    pub trigger: Trigger,
    pub what: &'static str,
    /// The action it runs, for the document it is pressed in. `None` for
    /// the Prepare keys, which [`super::prepare_keys`] handles.
    pub run: Option<fn(u64) -> Action>,
    /// What a Prepare key does, `None` for the global keys.
    pub prepare: Option<PrepareKey>,
    /// On macOS the menu bar owns this key, not the key handler.
    pub menu_on_macos: bool,
    /// The ribbon action (`tools::Tool::id`) whose shortcut this is, so
    /// its row and menu item show the key.
    pub tool: Option<&'static str>,
    /// Only the macOS menu bar has this key (Windows and Linux have no such
    /// row and no handler).
    pub mac_only: bool,
}

impl Shortcut {
    /// The same shortcut, as the key of the ribbon action `id`.
    const fn for_tool(mut self, id: &'static str) -> Self {
        self.tool = Some(id);
        self
    }

    /// The same shortcut, for macOS only.
    const fn mac_only(mut self) -> Self {
        self.mac_only = true;
        self
    }

    /// Its key, `None` for a drag row.
    pub fn key(&self) -> Option<Key> {
        match self.trigger {
            Trigger::Key(k) => Some(k),
            Trigger::Drag => None,
        }
    }

    /// Whether it is held while dragging a handle, not pressed.
    pub fn is_drag(&self) -> bool {
        self.trigger == Trigger::Drag
    }

    /// Whether a text field wants this key for its own text (undo and redo):
    /// the key handler leaves it to the field while one has the keyboard.
    pub fn field_key(&self) -> bool {
        self.run.is_some() && matches!(self.what, "Undo" | "Redo")
    }

    /// Whether the key handler or the sheet has this row on this system.
    pub fn here(&self) -> bool {
        !self.mac_only || cfg!(target_os = "macos")
    }
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
        trigger: Trigger::Key(key),
        what,
        run: Some(run),
        prepare: None,
        menu_on_macos,
        tool: None,
        mac_only: false,
    }
}

const fn prepare(
    modifiers: Modifiers,
    key: Key,
    what: &'static str,
    action: PrepareKey,
) -> Shortcut {
    Shortcut {
        modifiers,
        trigger: Trigger::Key(key),
        what,
        run: None,
        prepare: Some(action),
        menu_on_macos: false,
        tool: None,
        mac_only: false,
    }
}

/// A row for modifiers held while dragging on the canvas; the modifiers
/// come from this system's [`drag_modifiers`].
const fn while_dragging(what: &'static str, action: PrepareKey) -> Shortcut {
    Shortcut {
        modifiers: drag_modifiers(Os::CURRENT, action),
        trigger: Trigger::Drag,
        what,
        run: None,
        prepare: Some(action),
        menu_on_macos: false,
        tool: None,
        mac_only: false,
    }
}

/// Every shortcut the app has. The key handler runs the entries with a
/// `run`, in this order (a Shift variant before its plain key), and the
/// shortcuts sheet lists all of them.
pub const SHORTCUTS: [Shortcut; 36] = [
    global(
        CMD_SHIFT,
        Key::S,
        "Save As",
        |d| Action::SaveAs { doc_id: d },
        true,
    ),
    global(CMD, Key::S, "Save", |d| Action::Save { doc_id: d }, true),
    global(CMD, Key::O, "Open", |_| Action::OpenDialog, true),
    global(CMD, Key::N, "New project", |_| Action::NewProject, true),
    global(CMD, Key::W, "Close the document", Action::Close, true).mac_only(),
    global(
        CMD,
        Key::Comma,
        SETTINGS_LABEL,
        |_| Action::ShowSettings,
        true,
    ),
    global(CMD_SHIFT, Key::Z, "Redo", Action::Redo, true),
    // Not a menu key: the key handler redoes on every system.
    global(CMD, Key::Y, "Redo", Action::Redo, false),
    global(CMD, Key::Z, "Undo", Action::Undo, true),
    prepare(CMD, Key::A, "Select all objects", PrepareKey::SelectAll),
    prepare(CMD, Key::I, "Import a design", PrepareKey::Import).for_tool("import"),
    prepare(
        CMD,
        Key::D,
        "Duplicate the selection",
        PrepareKey::Duplicate,
    )
    .for_tool("duplicate"),
    // Shift+Cmd+G before Cmd+G, as the other Shift variants.
    prepare(
        CMD_SHIFT,
        Key::G,
        "Ungroup the selection",
        PrepareKey::Ungroup,
    )
    .for_tool("ungroup"),
    prepare(CMD, Key::G, "Group the selection", PrepareKey::Group).for_tool("group"),
    // The stacking order, Shift variants first.
    prepare(
        CMD_SHIFT,
        Key::CloseBracket,
        "Bring the selection to the front",
        PrepareKey::BringToFront,
    )
    .for_tool("order.front"),
    prepare(
        CMD_SHIFT,
        Key::OpenBracket,
        "Send the selection to the back",
        PrepareKey::SendToBack,
    )
    .for_tool("order.back"),
    prepare(
        CMD,
        Key::CloseBracket,
        "Bring the selection forward one step",
        PrepareKey::BringForward,
    )
    .for_tool("order.forward"),
    prepare(
        CMD,
        Key::OpenBracket,
        "Send the selection backward one step",
        PrepareKey::SendBackward,
    )
    .for_tool("order.backward"),
    prepare(
        Modifiers::NONE,
        Key::ArrowLeft,
        "Nudge the selection left by 1 mm (Shift 10 mm, Alt 0.1 mm)",
        PrepareKey::Nudge,
    ),
    prepare(
        Modifiers::NONE,
        Key::ArrowRight,
        "Nudge the selection right by 1 mm (Shift 10 mm, Alt 0.1 mm)",
        PrepareKey::Nudge,
    ),
    prepare(
        Modifiers::NONE,
        Key::ArrowUp,
        "Nudge the selection up by 1 mm (Shift 10 mm, Alt 0.1 mm)",
        PrepareKey::Nudge,
    ),
    prepare(
        Modifiers::NONE,
        Key::ArrowDown,
        "Nudge the selection down by 1 mm (Shift 10 mm, Alt 0.1 mm)",
        PrepareKey::Nudge,
    ),
    prepare(
        Modifiers::NONE,
        Key::Delete,
        "Delete the selection",
        PrepareKey::Delete,
    )
    .for_tool("delete"),
    prepare(
        Modifiers::NONE,
        Key::Backspace,
        "Delete the selection",
        PrepareKey::Delete,
    )
    .for_tool("delete"),
    prepare(
        Modifiers::NONE,
        Key::Escape,
        "Close the panel, cancel the lasso, go back to the Select tool, leave the group, or clear the selection",
        PrepareKey::ClearSelection,
    ),
    prepare(
        Modifiers::NONE,
        Key::V,
        "Select tool: a drag on the empty mat draws a box",
        PrepareKey::SelectTool,
    ),
    prepare(
        Modifiers::NONE,
        Key::L,
        "Lasso tool: a drag draws a free shape",
        PrepareKey::LassoTool,
    ),
    prepare(
        Modifiers::NONE,
        Key::Space,
        "Pan the canvas (hold it and drag, or drag with the middle button)",
        PrepareKey::Pan,
    ),
    while_dragging(
        "Drag just outside a corner of the selection: rotate it",
        PrepareKey::RotateZone,
    ),
    while_dragging(
        "While dragging a handle: the opposite of Keep aspect, for this drag",
        PrepareKey::FlipKeepAspect,
    ),
    while_dragging(
        "While dragging a handle: scale from the centre",
        PrepareKey::ScaleFromCentre,
    ),
    while_dragging(
        "While dragging a corner handle: rotate instead of resize",
        PrepareKey::RotateOnCorner,
    ),
    while_dragging(
        "While rotating: turn in 15 degree steps",
        PrepareKey::SnapAngle,
    ),
    while_dragging(
        "Drag on the empty mat: select what lies fully inside the box",
        PrepareKey::SelectBox,
    ),
    while_dragging(
        "Start a drag on the empty mat: draw a lasso, it selects all it touches",
        PrepareKey::Lasso,
    ),
    while_dragging(
        "While clicking, or drawing a box or lasso: flip objects in or out of the selection",
        PrepareKey::ToggleInSelection,
    ),
];

/// The rows of [`SHORTCUTS`] for a Prepare key.
fn rows(action: PrepareKey) -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS.iter().filter(move |s| s.prepare == Some(action))
}

/// Whether the modifiers of a drag row of the table (Shift, Option, Cmd
/// or Ctrl) are held in `now`. A row without modifiers (the rotate zone)
/// is never held.
pub fn held(now: Modifiers, action: PrepareKey) -> bool {
    rows(action).any(|s| {
        let m = s.modifiers;
        s.is_drag()
            && !m.is_none()
            && (!m.shift || now.shift)
            && (!m.alt || now.alt)
            && (!m.command || now.command)
            && (!m.mac_cmd || now.mac_cmd)
            && (!m.ctrl || now.ctrl)
    })
}

/// Consumes the keys of a Prepare key from this frame's input. True if one
/// of them was pressed. The modifiers must match exactly: Cmd+Shift+D is
/// not Cmd+D.
pub fn pressed(ctx: &egui::Context, action: PrepareKey) -> bool {
    ctx.input_mut(|i| {
        let mut hit = false;
        for s in rows(action) {
            let Some(want) = s.key() else { continue };
            let at = i.events.iter().position(|e| {
                matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
                    if same_key(*key, want, s.modifiers.shift) && modifiers.matches_exact(s.modifiers))
            });
            if let Some(at) = at {
                i.events.remove(at);
                hit = true;
            }
        }
        hit
    })
}

/// Whether the pressed `key` is the key `want` of a row. With Shift held,
/// some keyboards report Shift+] as "}" and Shift+[ as "{": those count as
/// the bracket keys.
fn same_key(key: Key, want: Key, shift: bool) -> bool {
    key == want
        || (shift
            && matches!(
                (key, want),
                (Key::CloseCurlyBracket, Key::CloseBracket)
                    | (Key::OpenCurlyBracket, Key::OpenBracket)
            ))
}

/// Whether `key` is one of the nudge keys of the table.
pub fn is_nudge_key(key: Key) -> bool {
    rows(PrepareKey::Nudge).any(|s| s.key() == Some(key))
}

/// The key hint of a Prepare key for menus ("Cmd+D"), from the table.
pub fn hint(action: PrepareKey) -> String {
    // On a Mac the key that deletes is labelled Backspace (the delete key).
    let mac_delete = cfg!(target_os = "macos") && action == PrepareKey::Delete;
    rows(action)
        .find(|s| !mac_delete || s.key() == Some(Key::Backspace))
        .map(Shortcut::keys)
        .unwrap_or_default()
}

/// The key that is held to pan with the pointer.
pub fn pan_key() -> Key {
    rows(PrepareKey::Pan)
        .find_map(Shortcut::key)
        .unwrap_or(Key::Space)
}

impl Shortcut {
    /// "Cmd+Shift+S" on macOS, "Ctrl+Shift+S" elsewhere.
    pub fn keys(&self) -> String {
        self.keys_on(Os::CURRENT)
    }

    /// Its keys with the names of `os`.
    pub fn keys_on(&self, os: Os) -> String {
        let mut s = modifier_names(self.modifiers, os);
        if !s.is_empty() {
            s.push('+');
        }
        let key = match self.trigger {
            Trigger::Key(key) => key,
            Trigger::Drag => {
                s.push_str("drag");
                return s;
            }
        };
        s.push_str(match key {
            Key::Comma => ",",
            Key::OpenBracket => "[",
            Key::CloseBracket => "]",
            Key::ArrowLeft => "Left",
            Key::ArrowRight => "Right",
            Key::ArrowUp => "Up",
            Key::ArrowDown => "Down",
            Key::Backspace => "Backspace",
            Key::Escape => "Esc",
            Key::Delete => "Del",
            Key::Space => "Space",
            other => other.name(),
        });
        s
    }
}

/// Runs the key handler's part of [`SHORTCUTS`]. On macOS the menu bar owns
/// the rows with `menu_on_macos`: its items have these keys, so the key
/// never reaches the handler twice.
pub fn run_shortcuts(ctx: &egui::Context, active: u64, actions: &mut Vec<Action>) {
    let field = ctx.text_edit_focused();
    ctx.input_mut(|i| {
        for s in &SHORTCUTS {
            let Some(run) = s.run else { continue };
            if !s.here() || (s.menu_on_macos && cfg!(target_os = "macos")) {
                continue;
            }
            if field && s.field_key() {
                continue;
            }
            let Some(key) = s.key() else { continue };
            if i.consume_shortcut(&KeyboardShortcut::new(s.modifiers, key)) {
                actions.push(run(active));
            }
        }
    });
}

/// The title of the shortcuts sheet's section on calculations.
pub const CALC_TITLE: &str = "Calculations in number fields";

/// What the section says first.
pub const CALC_INTRO: &str = "Every number field takes a calculation. It counts when you leave the field; Esc keeps the old value.";

/// What you can type in a number field, and what it gives.
pub const CALC_EXAMPLES: [(&str, &str); 7] = [
    ("100/3", "33.33, a third of 100"),
    ("2*(10+5)", "30: brackets first, then * and /, then + and -"),
    ("2in", "50.8 mm. Lengths take mm, cm and in (or \")"),
    ("3/4in + 1cm", "29.05 mm: units mix"),
    ("+=10", "10 more than now (also -=, *= and /=)"),
    ("12,5", "12.5: a comma works when there is no dot"),
    ("360/8", "45 degrees. Angles take deg (or \u{00b0})"),
];

/// The shortcuts sheet. Returns false once it should close.
pub fn show_shortcuts(ctx: &egui::Context) -> bool {
    let modal = egui::Modal::new(egui::Id::unique("flatcraft-shortcuts")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new("Keyboard shortcuts").font(metrics::title()));
        ui.add_space(metrics::SPACE_8);
        // The list scrolls in a short window, so Close stays in reach.
        let room = (ctx.content_rect().height() * 0.6).max(120.0);
        egui::ScrollArea::vertical()
            .max_height(room)
            .min_scrolled_height(room)
            .show(ui, sheet_body);
        ui.add_space(metrics::SPACE_8);
        ui.button("Close").clicked()
    });
    !(modal.inner || modal.should_close())
}

/// The keys and the calculations of the shortcuts sheet.
fn sheet_body(ui: &mut egui::Ui) {
    {
        egui::Grid::new("flatcraft-shortcut-grid")
            .num_columns(2)
            .spacing([metrics::SPACE_24, metrics::SPACE_4])
            .show(ui, |ui| {
                for s in SHORTCUTS.iter().filter(|s| s.here()) {
                    ui.label(RichText::new(s.keys()).monospace());
                    ui.label(s.what);
                    ui.end_row();
                }
            });
        ui.add_space(metrics::SPACE_8);
        ui.label(RichText::new(CALC_TITLE).font(metrics::heading()));
        ui.label(CALC_INTRO);
        egui::Grid::new("flatcraft-calc-grid")
            .num_columns(2)
            .spacing([metrics::SPACE_24, metrics::SPACE_4])
            .show(ui, |ui| {
                for (typed, what) in CALC_EXAMPLES {
                    ui.label(RichText::new(typed).monospace());
                    ui.label(what);
                    ui.end_row();
                }
            });
    }
}
