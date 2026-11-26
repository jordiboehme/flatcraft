//! The one table of the ribbon: every action with its id, title, icon,
//! group, description and what it runs, and every group with its title,
//! icon and description. The ribbon, the group lists, the pins, search
//! and the context menu all read their titles from here, so the words a
//! person sees for one action are the same everywhere.
//!
//! The descriptions are the text of the hover cards. They follow the
//! hints-for-beginners rule: what the thing does and when to use it, in
//! plain words, with any technical term explained. A dimmed action says
//! why it cannot run and what to do ([`Tool::blocked`]).

use std::sync::LazyLock;

use serde_json::json;

use super::arrange::{self, PanelKind};
use super::canvas::{CanvasView, PX_PER_MM_100};
use super::help::PrepareKey;
use super::icons::Icon;
use super::prepare::{PrepareState, RibbonTab, Snapshot};
use super::rails::SidePanel;
use super::{Action, ribbon};

/// A group of the ribbon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupId {
    Add,
    Edit,
    Files,
    Size,
    Arrange,
    Zoom,
    Show,
    Panels,
}

/// What the table says about a group.
#[derive(Debug, Clone, Copy)]
pub struct GroupDef {
    pub id: GroupId,
    pub tab: RibbonTab,
    pub title: &'static str,
    pub icon: Icon,
    pub description: &'static str,
}

/// Every group, left to right on its tab.
pub const GROUPS: [GroupDef; 8] = [
    GroupDef {
        id: GroupId::Add,
        tab: RibbonTab::Layout,
        title: "Add",
        icon: Icon::Plus,
        description: "Bring designs onto the mat: SVG files (the drawing format most design apps save), pictures to trace, or the test sheet.",
    },
    GroupDef {
        id: GroupId::Edit,
        tab: RibbonTab::Layout,
        title: "Edit",
        icon: Icon::Pencil,
        description: "Copy or remove what you selected. Undo takes back any step.",
    },
    GroupDef {
        id: GroupId::Size,
        tab: RibbonTab::Layout,
        title: "Position and size",
        icon: Icon::Scale,
        description: "Where the selection sits on the mat and how big it is, in millimetres, or its scale in percent. Type a number, then press Tab or Enter or click elsewhere. Esc puts the old value back.",
    },
    GroupDef {
        id: GroupId::Arrange,
        tab: RibbonTab::Layout,
        title: "Arrange",
        icon: Icon::Arrange,
        description: "Move, turn, flip, line up and copy your objects. Pin the ones you use most to the ribbon.",
    },
    GroupDef {
        id: GroupId::Files,
        tab: RibbonTab::Layout,
        title: "Linked files",
        icon: Icon::Linked,
        description: "An imported object stays linked to its file. Change the file in another app, then read it again here.",
    },
    GroupDef {
        id: GroupId::Zoom,
        tab: RibbonTab::View,
        title: "Zoom",
        icon: Icon::ZoomIn,
        description: "How close you look at the mat. It never changes what is cut.",
    },
    GroupDef {
        id: GroupId::Show,
        tab: RibbonTab::View,
        title: "Show",
        icon: Icon::Eye,
        description: "Helpers drawn on the canvas to place things by eye. They are never cut.",
    },
    GroupDef {
        id: GroupId::Panels,
        tab: RibbonTab::View,
        title: "Panels",
        icon: Icon::Panels,
        description: "Show or hide the side panels to make room for the mat.",
    },
];

impl GroupId {
    pub fn def(self) -> &'static GroupDef {
        GROUPS
            .iter()
            .find(|g| g.id == self)
            .expect("every group is in GROUPS")
    }

    /// The actions of the group, in table order.
    pub fn tools(self) -> impl Iterator<Item = &'static Tool> {
        TOOLS.iter().filter(move |t| t.group == self)
    }
}

/// The groups of a tab, left to right.
pub fn groups_of(tab: RibbonTab) -> impl Iterator<Item = &'static GroupDef> {
    GROUPS.iter().filter(move |g| g.tab == tab)
}

/// An edge or centre line Align lines up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignMode {
    Left,
    Centre,
    Right,
    Top,
    Middle,
    Bottom,
}

impl AlignMode {
    /// The `mode` of `arrange.align`.
    pub fn param(self) -> &'static str {
        match self {
            AlignMode::Left => "left",
            AlignMode::Centre => "hcenter",
            AlignMode::Right => "right",
            AlignMode::Top => "top",
            AlignMode::Middle => "vcenter",
            AlignMode::Bottom => "bottom",
        }
    }
}

/// What Align lines up with: the option of the Align rows. A pin keeps
/// the option it was pinned with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignTo {
    Selection,
    Area,
}

impl AlignTo {
    pub fn of(to_area: bool) -> Self {
        if to_area {
            AlignTo::Area
        } else {
            AlignTo::Selection
        }
    }

    /// Its label in the list ("To the selection").
    pub fn label(self) -> &'static str {
        match self {
            AlignTo::Selection => "To the selection",
            AlignTo::Area => "To the cutting area",
        }
    }

    /// Its short name in a pinned title ("Align left · cutting area").
    pub fn short(self) -> &'static str {
        match self {
            AlignTo::Selection => "selection",
            AlignTo::Area => "cutting area",
        }
    }
}

/// What an action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act {
    Import,
    Trace,
    TestPatterns,
    Duplicate,
    Copies,
    Delete,
    Reload,
    MoveCopy,
    Rotate,
    RotateLeft,
    RotateRight,
    MirrorHorizontal,
    MirrorVertical,
    Align(AlignMode, AlignTo),
    DistributeHorizontal,
    DistributeVertical,
    RectangularPattern,
    CircularPattern,
    AutoArrangePanel,
    AutoArrange,
    ZoomFit,
    Zoom100,
    Grid,
    Rulers,
    ObjectsPanel,
    SettingsPanel,
    DebugPanel,
}

/// How a pinned action looks on the ribbon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// A 24 pt icon above a small label.
    Big,
    /// A small icon button, in one or two rows.
    Small,
}

/// A heading in a group's list, before the rows that belong to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Mirror,
    Align,
    Distribute,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Mirror => "Mirror",
            Section::Align => "Align",
            Section::Distribute => "Distribute",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Section::Mirror => Icon::Mirror,
            Section::Align => Icon::Align,
            Section::Distribute => Icon::DistributeHorizontal,
        }
    }
}

/// One action of the ribbon.
#[derive(Debug, Clone, Copy)]
pub struct Tool {
    /// Its pin id in the preferences (`ribbon_pins`).
    pub id: &'static str,
    pub act: Act,
    pub title: &'static str,
    pub icon: Icon,
    pub group: GroupId,
    /// What it does and when to use it, for its hover card.
    pub description: &'static str,
    pub style: Style,
    pub section: Option<Section>,
}

const fn tool(
    id: &'static str,
    act: Act,
    title: &'static str,
    icon: Icon,
    group: GroupId,
    style: Style,
    description: &'static str,
) -> Tool {
    Tool {
        id,
        act,
        title,
        icon,
        group,
        description,
        style,
        section: None,
    }
}

const fn in_section(mut t: Tool, section: Section) -> Tool {
    t.section = Some(section);
    t
}

const fn align(
    id: &'static str,
    mode: AlignMode,
    to: AlignTo,
    title: &'static str,
    icon: Icon,
    description: &'static str,
) -> Tool {
    in_section(
        tool(
            id,
            Act::Align(mode, to),
            title,
            icon,
            GroupId::Arrange,
            Style::Small,
            description,
        ),
        Section::Align,
    )
}

use AlignMode as M;
use AlignTo::{Area, Selection};
use GroupId as G;
use Style::{Big, Small};

/// Every action, in the order of the ribbon and of the lists. The ids are
/// `flatcraft_engine::prefs::RIBBON_PIN_IDS`, in the same order.
pub const TOOLS: [Tool; 38] = [
    tool(
        "import",
        Act::Import,
        "Import...",
        Icon::Import,
        G::Add,
        Big,
        "Bring a design or a picture onto the mat. A design is an SVG file, the drawing format most design apps save. A picture gets traced into cut lines.",
    ),
    tool(
        "trace",
        Act::Trace,
        "Trace...",
        Icon::Trace,
        G::Add,
        Big,
        "Turn the selected picture into cut lines with your own settings. Tracing follows the edges between dark and light.",
    ),
    tool(
        "test_patterns",
        Act::TestPatterns,
        "Test patterns",
        Icon::TestPatterns,
        G::Add,
        Big,
        "Add the test sheet: small shapes that show how well blade and settings cut. Cut it first on a new machine or material.",
    ),
    tool(
        "duplicate",
        Act::Duplicate,
        "Duplicate",
        Icon::Duplicate,
        G::Edit,
        Big,
        "Make one copy of the selection, a little to the side.",
    ),
    tool(
        "copies",
        Act::Copies,
        "Copies...",
        Icon::Copies,
        G::Edit,
        Big,
        "Many copies of one object in columns and rows. It opens the Rectangular pattern panel.",
    ),
    tool(
        "delete",
        Act::Delete,
        "Delete",
        Icon::Delete,
        G::Edit,
        Big,
        "Remove the selection from the mat. Undo brings it back.",
    ),
    tool(
        "reload",
        Act::Reload,
        "Reload",
        Icon::Reload,
        G::Files,
        Big,
        "Read the linked files again after you changed them in another app. Works on the selection, or on every linked object when nothing is selected.",
    ),
    tool(
        "move_copy",
        Act::MoveCopy,
        "Move / Copy...",
        Icon::Move,
        G::Arrange,
        Big,
        "Move the selection by an exact distance in mm, or leave copies behind at every step.",
    ),
    tool(
        "rotate",
        Act::Rotate,
        "Rotate...",
        Icon::Rotate,
        G::Arrange,
        Big,
        "Turn the selection by an exact angle in degrees. A positive angle turns clockwise.",
    ),
    tool(
        "rotate.left",
        Act::RotateLeft,
        "Rotate -90",
        Icon::RotateLeft,
        G::Arrange,
        Small,
        "A quarter turn anticlockwise. With Each object on, every object turns about its own centre.",
    ),
    tool(
        "rotate.right",
        Act::RotateRight,
        "Rotate +90",
        Icon::Rotate,
        G::Arrange,
        Small,
        "A quarter turn clockwise. With Each object on, every object turns about its own centre.",
    ),
    in_section(
        tool(
            "mirror.horizontal",
            Act::MirrorHorizontal,
            "Mirror horizontally",
            Icon::Mirror,
            G::Arrange,
            Small,
            "Flip left and right. Use it for iron-on vinyl, which is cut from the back.",
        ),
        Section::Mirror,
    ),
    in_section(
        tool(
            "mirror.vertical",
            Act::MirrorVertical,
            "Mirror vertically",
            Icon::MirrorVertical,
            G::Arrange,
            Small,
            "Flip top and bottom, like a reflection in a lake.",
        ),
        Section::Mirror,
    ),
    align(
        "align.left.selection",
        M::Left,
        Selection,
        "Align left",
        Icon::AlignLeft,
        "Moves the selected objects so their left edges line up with the leftmost one.",
    ),
    align(
        "align.centre.selection",
        M::Centre,
        Selection,
        "Align centre",
        Icon::AlignCentre,
        "Moves the selected objects so their centres sit on one vertical line, the middle of the selection.",
    ),
    align(
        "align.right.selection",
        M::Right,
        Selection,
        "Align right",
        Icon::AlignRight,
        "Moves the selected objects so their right edges line up with the rightmost one.",
    ),
    align(
        "align.top.selection",
        M::Top,
        Selection,
        "Align top",
        Icon::AlignTop,
        "Moves the selected objects so their top edges line up with the highest one.",
    ),
    align(
        "align.middle.selection",
        M::Middle,
        Selection,
        "Align middle",
        Icon::Align,
        "Moves the selected objects so their centres sit on one horizontal line, the middle of the selection.",
    ),
    align(
        "align.bottom.selection",
        M::Bottom,
        Selection,
        "Align bottom",
        Icon::AlignBottom,
        "Moves the selected objects so their bottom edges line up with the lowest one.",
    ),
    align(
        "align.left.area",
        M::Left,
        Area,
        "Align left",
        Icon::AlignLeftArea,
        "Moves the selection to the left edge of the cutting area, the part of the mat the blade can reach.",
    ),
    align(
        "align.centre.area",
        M::Centre,
        Area,
        "Align centre",
        Icon::AlignCentreArea,
        "Moves the selection to the middle of the cutting area, from left to right. The cutting area is the part of the mat the blade can reach.",
    ),
    align(
        "align.right.area",
        M::Right,
        Area,
        "Align right",
        Icon::AlignRightArea,
        "Moves the selection to the right edge of the cutting area, the part of the mat the blade can reach.",
    ),
    align(
        "align.top.area",
        M::Top,
        Area,
        "Align top",
        Icon::AlignTopArea,
        "Moves the selection to the top edge of the cutting area, the part of the mat the blade can reach.",
    ),
    align(
        "align.middle.area",
        M::Middle,
        Area,
        "Align middle",
        Icon::AlignMiddleArea,
        "Moves the selection to the middle of the cutting area, from top to bottom. The cutting area is the part of the mat the blade can reach.",
    ),
    align(
        "align.bottom.area",
        M::Bottom,
        Area,
        "Align bottom",
        Icon::AlignBottomArea,
        "Moves the selection to the bottom edge of the cutting area, the part of the mat the blade can reach.",
    ),
    in_section(
        tool(
            "distribute.horizontal",
            Act::DistributeHorizontal,
            "Distribute horizontally",
            Icon::DistributeHorizontal,
            G::Arrange,
            Small,
            "Equal gaps between 3 or more objects, from left to right. The two outer objects stay where they are.",
        ),
        Section::Distribute,
    ),
    in_section(
        tool(
            "distribute.vertical",
            Act::DistributeVertical,
            "Distribute vertically",
            Icon::DistributeVertical,
            G::Arrange,
            Small,
            "Equal gaps between 3 or more objects, from top to bottom. The two outer objects stay where they are.",
        ),
        Section::Distribute,
    ),
    tool(
        "pattern.rectangular",
        Act::RectangularPattern,
        "Rectangular pattern...",
        Icon::RectangularPattern,
        G::Arrange,
        Big,
        "Copies of one object in columns and rows, with the gap you choose. Sticker sheet, engage!",
    ),
    tool(
        "pattern.circular",
        Act::CircularPattern,
        "Circular pattern...",
        Icon::CircularPattern,
        G::Arrange,
        Big,
        "Copies around a centre point, like the numbers on a clock.",
    ),
    tool(
        "auto_arrange.panel",
        Act::AutoArrangePanel,
        "Auto arrange...",
        Icon::AutoArrangePanel,
        G::Arrange,
        Big,
        "Packs the selected objects onto the cutting area, Tetris style, with the gap you choose. Everything else stays where you put it, and the packed objects fit around it. Nothing selected: it packs every visible object. A preview shows where they land.",
    ),
    tool(
        "auto_arrange",
        Act::AutoArrange,
        "Auto arrange",
        Icon::Arrange,
        G::Arrange,
        Big,
        "Packs the selected objects onto the cutting area, Tetris style, in one click. Everything else stays where you put it, and the packed objects fit around it. Nothing selected: it packs every visible object. It uses the gap you last chose in the Auto arrange panel.",
    ),
    tool(
        "zoom.fit",
        Act::ZoomFit,
        "Zoom to fit",
        Icon::ZoomFit,
        G::Zoom,
        Big,
        "Show the whole mat in the window.",
    ),
    tool(
        "zoom.100",
        Act::Zoom100,
        "Zoom 100%",
        Icon::ZoomActual,
        G::Zoom,
        Big,
        "About real size on a typical screen, to judge how big a design really is.",
    ),
    tool(
        "view.grid",
        Act::Grid,
        "Grid",
        Icon::Grid,
        G::Show,
        Big,
        "Show the grid printed on the mat, to place things by eye.",
    ),
    tool(
        "view.rulers",
        Act::Rulers,
        "Rulers",
        Icon::Ruler,
        G::Show,
        Big,
        "Show rulers in millimetres along the edges of the canvas.",
    ),
    tool(
        "panel.objects",
        Act::ObjectsPanel,
        "Objects panel",
        Icon::PanelLeft,
        G::Panels,
        Big,
        "The list of everything on the mat. Hide, skip, rename or pick objects there.",
    ),
    tool(
        "panel.settings",
        Act::SettingsPanel,
        "Settings panel",
        Icon::PanelRight,
        G::Panels,
        Big,
        "What is cut and how: cut mode, plotter, mat, material, speed and force.",
    ),
    tool(
        "panel.debug",
        Act::DebugPanel,
        "Debug panel",
        Icon::Debug,
        G::Panels,
        Big,
        "For the curious: what the app planned and what goes to the plotter. You do not need it to cut.",
    ),
];

/// How an icon is coloured (Jordi's "Paper (crisp)" style): which of its
/// SVG children (in file order) are paper, the shapes that get cut, filled
/// with the paper colour and outlined with the cut colour, and which are
/// the one accent part that tells what happens, in the accent colour.
/// Every other child is ink. Every icon of an action, a group or a
/// switch has an entry; the framed Align icons use their base icon's.
pub const ICON_ROLES: [(Icon, &[usize], &[usize]); 44] = [
    (Icon::Import, &[2], &[0, 1]),
    (Icon::Trace, &[], &[2]),
    (Icon::TestPatterns, &[0], &[1]),
    (Icon::Duplicate, &[0], &[1]),
    (Icon::Copies, &[2], &[0, 1]),
    (Icon::Delete, &[], &[0, 1]),
    (Icon::Reload, &[], &[0, 1, 2, 3]),
    (Icon::Move, &[], &[0, 1, 2, 3, 4, 5]),
    (Icon::Rotate, &[], &[1]),
    (Icon::RotateLeft, &[], &[1]),
    (Icon::Mirror, &[4, 5], &[0, 1, 2, 3]),
    (Icon::MirrorVertical, &[2, 3], &[0, 1, 4, 5]),
    (Icon::AlignLeft, &[0, 1], &[2]),
    (Icon::AlignCentre, &[1, 2, 3, 4], &[0]),
    (Icon::AlignRight, &[0, 1], &[2]),
    (Icon::AlignTop, &[0, 1], &[2]),
    (Icon::Align, &[1, 2, 3, 4], &[0]),
    (Icon::AlignBottom, &[0, 1], &[2]),
    (Icon::DistributeHorizontal, &[0, 1], &[2, 3, 4, 5]),
    (Icon::DistributeVertical, &[4, 5], &[0, 1, 2, 3]),
    (Icon::RectangularPattern, &[0], &[1, 2]),
    (Icon::CircularPattern, &[1], &[0]),
    (Icon::Arrange, &[0, 2], &[1, 3]),
    (Icon::ZoomFit, &[], &[0, 1, 2, 3]),
    (Icon::ZoomActual, &[], &[0]),
    (Icon::Grid, &[0], &[1, 2, 3, 4]),
    (Icon::Ruler, &[0], &[1, 2, 3, 4]),
    (Icon::PanelLeft, &[0], &[1]),
    (Icon::PanelRight, &[0], &[1]),
    (Icon::Debug, &[], &[0]),
    (Icon::Plus, &[], &[0, 1]),
    (Icon::Pencil, &[], &[1]),
    (Icon::Linked, &[], &[0]),
    (Icon::Scale, &[], &[1, 2, 3]),
    (Icon::ZoomIn, &[], &[2, 3]),
    (Icon::Eye, &[], &[1]),
    (Icon::Panels, &[0], &[1, 2]),
    (Icon::EachObject, &[1], &[0]),
    (Icon::AsGroup, &[4, 5], &[0, 1, 2, 3]),
    (Icon::AutoArrangePanel, &[0, 2], &[1, 3]),
    // Home's New cards.
    (Icon::NewFile, &[0, 1], &[2, 3]),
    (Icon::FolderOpen, &[], &[0]),
    // The Cut bar's stages: the blade, and the two cuts around the travel.
    (Icon::Plotter, &[], &[2]),
    (Icon::RouteTravel, &[], &[0, 2]),
];

/// The paper and accent children of `icon` (see [`ICON_ROLES`]); `None`
/// for icons drawn in one colour (the app bar's, the pin, the chevron).
pub fn icon_roles(icon: Icon) -> Option<(&'static [usize], &'static [usize])> {
    let icon = icon.framed_from().unwrap_or(icon);
    ICON_ROLES
        .iter()
        .find(|(i, _, _)| *i == icon)
        .map(|(_, paper, accent)| (*paper, *accent))
}

/// The action with this pin id.
pub fn by_id(id: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.id == id)
}

/// The action that does `act`.
pub fn by_act(act: Act) -> &'static Tool {
    TOOLS
        .iter()
        .find(|t| t.act == act)
        .expect("every act is in TOOLS")
}

/// The pinned actions of a group, in table order.
pub fn pinned(snap: &Snapshot, group: GroupId) -> Vec<&'static Tool> {
    group
        .tools()
        .filter(|t| snap.ribbon_pins.iter().any(|p| p == t.id))
        .collect()
}

impl Tool {
    /// The option the action carries (what Align lines up with).
    pub fn option(&self) -> Option<AlignTo> {
        match self.act {
            Act::Align(_, to) => Some(to),
            _ => None,
        }
    }

    /// The title with its option: "Align left · cutting area". It names a
    /// pinned button and a search hit, where the option is not shown.
    pub fn full_title(&self) -> String {
        match self.option() {
            Some(to) => format!("{} · {}", self.title, to.short()),
            None => self.title.to_string(),
        }
    }

    /// The label of a big button: the title without "...", unless another
    /// action has that title ("Auto arrange..." next to "Auto arrange").
    pub fn short_title(&self) -> &'static str {
        let short = self.title.trim_end_matches("...");
        let taken = TOOLS
            .iter()
            .any(|t| t.id != self.id && t.title.trim_end_matches("...") == short);
        if taken { self.title } else { short }
    }

    /// The accessible name of its pinned button.
    pub fn button_name(&self) -> String {
        match self.style {
            Style::Big => self.short_title().to_string(),
            _ => self.full_title(),
        }
    }

    /// What its pin toggle names: like the pinned button, without "..."
    /// unless that would clash.
    pub fn pin_label(&self) -> String {
        match self.option() {
            Some(_) => self.full_title(),
            None => self.short_title().to_string(),
        }
    }

    /// The accessible name of its row in the group's list.
    pub fn row_name(&self) -> String {
        format!("{} ({})", self.title, self.group.def().title)
    }

    /// The title line of its hover card. Auto arrange also names the gap
    /// it runs with ("Auto arrange · 5 mm").
    pub fn card_title(&self, snap: &Snapshot) -> String {
        match self.act {
            Act::AutoArrange => {
                format!("{} · {} mm", self.title, trim_number(snap.auto_spacing_mm))
            }
            _ => self.full_title(),
        }
    }

    /// Its keyboard shortcut: the row of `help::SHORTCUTS` that names this
    /// action.
    pub fn shortcut(&self) -> Option<PrepareKey> {
        super::help::SHORTCUTS
            .iter()
            .find(|s| s.tool == Some(self.id))
            .and_then(|s| s.prepare)
    }

    /// The registry command the action runs, if any. Search lists the
    /// action instead of the command.
    pub fn command(&self) -> Option<&'static str> {
        Some(match self.act {
            Act::Import => "object.import",
            Act::Trace => "object.retrace",
            Act::TestPatterns => "tools.test_patterns",
            Act::Duplicate => "object.duplicate",
            Act::Copies | Act::RectangularPattern => "object.copies",
            Act::Delete => "object.delete",
            Act::Reload => "object.reload",
            Act::MoveCopy => "object.move",
            Act::Rotate | Act::RotateLeft | Act::RotateRight => "object.rotate",
            Act::MirrorHorizontal | Act::MirrorVertical => "object.mirror",
            Act::Align(..) => "arrange.align",
            Act::DistributeHorizontal | Act::DistributeVertical => "arrange.distribute",
            Act::CircularPattern => "object.circular_copies",
            Act::AutoArrangePanel | Act::AutoArrange => "arrange.auto",
            _ => return None,
        })
    }

    /// Why the action cannot run now, as a full sentence that says what to
    /// do, or `None` when it can.
    pub fn blocked(&self, snap: &Snapshot) -> Option<&'static str> {
        self.blocked_for(snap, snap.selection.len())
    }

    /// Like [`Self::blocked`], for `n` objects (the context menu works on
    /// the objects it was opened for).
    pub fn blocked_for(&self, snap: &Snapshot, n: usize) -> Option<&'static str> {
        const SELECT: &str = "Select something first.";
        // The engine refuses these without a cutting area.
        let needs_area = matches!(
            self.act,
            Act::Align(_, AlignTo::Area)
                | Act::AutoArrange
                | Act::AutoArrangePanel
                | Act::TestPatterns
        );
        if needs_area && snap.area.is_none() {
            return Some(NO_AREA);
        }
        match self.act {
            Act::Import
            | Act::TestPatterns
            | Act::ZoomFit
            | Act::Zoom100
            | Act::Grid
            | Act::Rulers
            | Act::ObjectsPanel
            | Act::SettingsPanel
            | Act::DebugPanel => None,
            Act::Trace => ribbon::single_raster(snap)
                .is_none()
                .then_some("Select one picture first. Only pictures can be traced."),
            Act::Reload => ribbon::reload_ids(snap)
                .is_none()
                .then_some("Select an object imported from a file first. Only those have a file to read again."),
            Act::Copies | Act::RectangularPattern => {
                (n != 1).then_some("Select one object first. The pattern copies one object at a time.")
            }
            Act::Align(_, AlignTo::Selection) => {
                (n < 2).then_some("Select 2 or more objects first. They line up with each other.")
            }
            Act::DistributeHorizontal | Act::DistributeVertical => {
                (n < 3).then_some("Select 3 or more objects first.")
            }
            Act::AutoArrangePanel | Act::AutoArrange => {
                if snap.objects.is_empty() {
                    Some("Import something first. There is nothing to arrange yet.")
                } else if !snap.selection.is_empty() && snap.selected().all(|o| !o.visible) {
                    Some(ONLY_HIDDEN_SELECTED)
                } else {
                    None
                }
            }
            Act::Duplicate
            | Act::Delete
            | Act::MoveCopy
            | Act::Rotate
            | Act::RotateLeft
            | Act::RotateRight
            | Act::MirrorHorizontal
            | Act::MirrorVertical
            | Act::Align(_, AlignTo::Area)
            | Act::CircularPattern => (n == 0).then_some(SELECT),
        }
    }

    /// On or off, for the actions that switch something.
    pub fn selected(&self, st: &PrepareState, show_debug: bool) -> Option<bool> {
        Some(match self.act {
            Act::Grid => st.show_grid,
            Act::Rulers => st.show_rulers,
            Act::ObjectsPanel => !st.panels.collapsed(SidePanel::Objects),
            Act::SettingsPanel => !st.panels.collapsed(SidePanel::Settings),
            Act::DebugPanel => show_debug,
            _ => return None,
        })
    }

    /// Runs the action on the selection: pushes its command, opens its
    /// panel or dialog, or switches what it switches. A blocked action
    /// does nothing.
    pub fn run(
        &self,
        snap: &Snapshot,
        st: &mut PrepareState,
        show_debug: &mut bool,
        actions: &mut Vec<Action>,
    ) {
        if self.blocked(snap).is_some() {
            return;
        }
        let doc_id = snap.doc_id;
        let ids: Vec<u64> = snap.selection.iter().copied().collect();
        let each = st.fields.each;
        let push = |actions: &mut Vec<Action>, a: Action| actions.push(a);
        match self.act {
            Act::Import => push(actions, Action::Import { doc_id }),
            Act::Trace => {
                if let Some(id) = ribbon::single_raster(snap) {
                    push(actions, Action::TraceDialog { doc_id, id });
                }
            }
            Act::TestPatterns => push(
                actions,
                Action::run(doc_id, "tools.test_patterns", json!({})),
            ),
            Act::Duplicate => push(actions, ribbon::duplicate_action(doc_id, &ids)),
            Act::Delete => push(actions, ribbon::delete_action(doc_id, &ids)),
            Act::Reload => {
                if let Some(ids) = ribbon::reload_ids(snap) {
                    push(actions, ribbon::reload_action(doc_id, ids));
                }
            }
            Act::Copies | Act::RectangularPattern => {
                arrange::open(st, snap, PanelKind::RectangularPattern)
            }
            Act::MoveCopy => arrange::open(st, snap, PanelKind::MoveCopy),
            Act::Rotate => arrange::open(st, snap, PanelKind::Rotate),
            Act::CircularPattern => arrange::open(st, snap, PanelKind::CircularPattern),
            Act::AutoArrangePanel => arrange::open(st, snap, PanelKind::AutoArrange),
            Act::RotateLeft => push(actions, ribbon::rotate_action(doc_id, &ids, -90.0, each)),
            Act::RotateRight => push(actions, ribbon::rotate_action(doc_id, &ids, 90.0, each)),
            Act::MirrorHorizontal => push(
                actions,
                ribbon::mirror_action(doc_id, &ids, "horizontal", each),
            ),
            Act::MirrorVertical => push(
                actions,
                ribbon::mirror_action(doc_id, &ids, "vertical", each),
            ),
            // A pinned combination runs with its own option, whatever the
            // list shows now.
            Act::Align(mode, to) => push(
                actions,
                ribbon::align_action(doc_id, &ids, mode.param(), to == AlignTo::Area),
            ),
            Act::DistributeHorizontal => push(
                actions,
                ribbon::distribute_action(doc_id, &ids, "horizontal"),
            ),
            Act::DistributeVertical => {
                push(actions, ribbon::distribute_action(doc_id, &ids, "vertical"))
            }
            Act::AutoArrange => push(actions, auto_arrange_action(snap)),
            Act::ZoomFit => st.fit_pending = true,
            Act::Zoom100 => {
                let at = st.canvas_rect.center();
                let f = PX_PER_MM_100 / st.view.px_per_mm;
                let mut v: CanvasView = st.view;
                v.zoom_about(at, f);
                st.view = v;
            }
            Act::Grid => st.show_grid = !st.show_grid,
            Act::Rulers => st.show_rulers = !st.show_rulers,
            Act::ObjectsPanel => st.panels.toggle(SidePanel::Objects),
            Act::SettingsPanel => st.panels.toggle(SidePanel::Settings),
            Act::DebugPanel => *show_debug = !*show_debug,
        }
    }
}

/// Why Auto arrange cannot run when every selected object is hidden.
pub const ONLY_HIDDEN_SELECTED: &str = "The selected objects are hidden, so there is nothing to pack. Show them, or select nothing to pack every visible object.";

/// Why an action that needs the cutting area cannot run.
pub const NO_AREA: &str =
    "There is no cutting area yet. Pick a plotter and a mat in the Settings panel first.";

/// The one-click Auto arrange, with the spacing of the preferences.
pub fn auto_arrange_action(snap: &Snapshot) -> Action {
    Action::run(
        snap.doc_id,
        "arrange.auto",
        json!({ "spacing_mm": snap.auto_spacing_mm }),
    )
}

/// "5", "2.5" or "0.25": a number without trailing zeros.
pub fn trim_number(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------------------------------------------------------------- settings

/// The cards that name drag modifiers, from this system's table (see
/// [`super::help::drag_modifiers`]).
static HANDLE_CARD: LazyLock<String> =
    LazyLock::new(|| super::help::handle_card(super::help::Os::CURRENT));
static ROTATE_CARD: LazyLock<String> =
    LazyLock::new(|| super::help::rotate_card(super::help::Os::CURRENT));
static ANGLE_CARD: LazyLock<String> =
    LazyLock::new(|| super::help::angle_card(super::help::Os::CURRENT));

/// A setting or field that is not an action. Each has a hover card in
/// plain words, like the actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    PositionX,
    PositionY,
    Width,
    Height,
    ScalePercent,
    Angle,
    KeepAspect,
    KeepAspectLinked,
    ScaleHandle,
    RotateZone,
    EachObject,
    Search,
    CutMode,
    Strokes,
    Speed,
    Force,
    Passes,
    ReverseAlternatePasses,
    Overcut,
    BladeOffset,
    Keep,
    Threshold,
    AutoThreshold,
    Invert,
    Despeckle,
    CornerThreshold,
    Smoothing,
    MinFeature,
    AutoMinFeature,
    AutoArrangeSpacing,
    KnifeCompensation,
    MirrorX,
    MirrorY,
    TrackEnhancingSupport,
    RegistrationMarks,
    AutoBlade,
    Calibrated,
    HiddenMat,
    TrackEnhancing,
    OverrideSet,
    PresetName,
    Vendor,
    Model,
    Protocol,
    Dialect,
    Transport,
    UsbVendorId,
    UsbProductId,
    OutputFolder,
    FileExtension,
    UnitsPerMm,
    ScaleX,
    ScaleY,
    SpeedRange,
    ForceRange,
    PassesRange,
    FlattenTolerance,
    FcValue,
    HostOffset,
    PivotThreshold,
    MatName,
    MatKind,
    MatWidth,
    MatHeight,
    GridOriginX,
    GridOriginY,
    GridWidth,
    GridHeight,
    MatColour,
    MatColourHex,
    BladeDepthHint,
    AddOverride,
    PresetNotes,
    WeedBox,
    WeedMargin,
    WeedPerObject,
    WeedLines,
    WeedSpacing,
}

impl Setting {
    pub const ALL: [Setting; 78] = [
        Setting::PositionX,
        Setting::PositionY,
        Setting::Width,
        Setting::Height,
        Setting::ScalePercent,
        Setting::Angle,
        Setting::KeepAspect,
        Setting::KeepAspectLinked,
        Setting::ScaleHandle,
        Setting::RotateZone,
        Setting::EachObject,
        Setting::Search,
        Setting::CutMode,
        Setting::Strokes,
        Setting::Speed,
        Setting::Force,
        Setting::Passes,
        Setting::ReverseAlternatePasses,
        Setting::Overcut,
        Setting::BladeOffset,
        Setting::Keep,
        Setting::Threshold,
        Setting::AutoThreshold,
        Setting::Invert,
        Setting::Despeckle,
        Setting::CornerThreshold,
        Setting::Smoothing,
        Setting::MinFeature,
        Setting::AutoMinFeature,
        Setting::AutoArrangeSpacing,
        Setting::KnifeCompensation,
        Setting::MirrorX,
        Setting::MirrorY,
        Setting::TrackEnhancingSupport,
        Setting::RegistrationMarks,
        Setting::AutoBlade,
        Setting::Calibrated,
        Setting::HiddenMat,
        Setting::TrackEnhancing,
        Setting::OverrideSet,
        Setting::PresetName,
        Setting::Vendor,
        Setting::Model,
        Setting::Protocol,
        Setting::Dialect,
        Setting::Transport,
        Setting::UsbVendorId,
        Setting::UsbProductId,
        Setting::OutputFolder,
        Setting::FileExtension,
        Setting::UnitsPerMm,
        Setting::ScaleX,
        Setting::ScaleY,
        Setting::SpeedRange,
        Setting::ForceRange,
        Setting::PassesRange,
        Setting::FlattenTolerance,
        Setting::FcValue,
        Setting::HostOffset,
        Setting::PivotThreshold,
        Setting::MatName,
        Setting::MatKind,
        Setting::MatWidth,
        Setting::MatHeight,
        Setting::GridOriginX,
        Setting::GridOriginY,
        Setting::GridWidth,
        Setting::GridHeight,
        Setting::MatColour,
        Setting::MatColourHex,
        Setting::BladeDepthHint,
        Setting::AddOverride,
        Setting::PresetNotes,
        Setting::WeedBox,
        Setting::WeedMargin,
        Setting::WeedPerObject,
        Setting::WeedLines,
        Setting::WeedSpacing,
    ];

    /// The kind of number a field of this setting holds, for the
    /// calculation hint on its card; `None` when it is no number field.
    pub fn number_kind(self) -> Option<flatcraft_core::numexpr::Kind> {
        use Setting::*;
        use flatcraft_core::numexpr::Kind;
        Some(match self {
            PositionX | PositionY | Width | Height | BladeOffset | Overcut | MinFeature
            | AutoArrangeSpacing | FlattenTolerance | HostOffset | MatWidth | MatHeight
            | GridOriginX | GridOriginY | GridWidth | GridHeight | WeedMargin | WeedSpacing => {
                Kind::Length
            }
            Angle | CornerThreshold | PivotThreshold => Kind::Angle,
            ScalePercent => Kind::Percent,
            Speed | Force | Passes | Threshold | Despeckle | Smoothing | UnitsPerMm | ScaleX
            | ScaleY | SpeedRange | ForceRange | PassesRange | FcValue => Kind::Plain,
            _ => return None,
        })
    }

    /// The text of its hover card: what it does, when to change it, and
    /// any term explained.
    pub fn card(self) -> &'static str {
        match self {
            Setting::PositionX => {
                "How far the left edge of the selection is from the 0 of the grid printed on the mat, in millimetres. Type a number to move the selection there. It counts when you leave the field (Tab, Enter or a click elsewhere); Esc puts the old value back."
            }
            Setting::PositionY => {
                "How far the top edge of the selection is from the 0 of the grid printed on the mat, in millimetres (down is more). Type a number to move the selection there. It counts when you leave the field (Tab, Enter or a click elsewhere); Esc puts the old value back."
            }
            Setting::Width => {
                "The width of the selection in millimetres. Type a number, then press Tab or Enter or click elsewhere. Esc puts the old value back."
            }
            Setting::Height => {
                "The height of the selection in millimetres. Type a number, then press Tab or Enter or click elsewhere. Esc puts the old value back."
            }
            Setting::ScalePercent => {
                "Scale the selection: 200 makes it twice as big, 50 half as big. Type a number, then press Tab or Enter or click elsewhere. With Each object on, every object grows or shrinks about its own centre."
            }
            Setting::KeepAspect => {
                "The lock on the bracket beside W and H. Click the lock or the bracket. Closed: width and height change together, so the shape does not get squashed, in the fields and with the handles on the mat. Open: change one without the other. Hold Shift while you drag a handle to get the opposite for that drag."
            }
            Setting::KeepAspectLinked => {
                "The lock on the bracket beside W and H. Click the lock or the bracket. Closed: width and height change together, so the shape does not get squashed, in the fields and with the handles on the mat. Open: change one without the other. Hold Shift while you drag a handle to get the opposite for that drag. Linked copies share one shape: stretching one stretches every copy along its own sides, so a copy turned by 90 degrees stretches the other way on the mat. Unlink copies first if they should not follow."
            }
            Setting::ScaleHandle => HANDLE_CARD.as_str(),
            Setting::RotateZone => ROTATE_CARD.as_str(),
            Setting::Angle => ANGLE_CARD.as_str(),
            Setting::EachObject => {
                "Two separate boxes (on): the Rotate and Mirror buttons and the % field work on each selected object about its own centre. Boxes in one frame (off): the selection moves as one piece, like Voltron. The Angle field always sets the angle of each object on its own, and turning by hand on the mat always turns the selection as one piece."
            }
            Setting::Search => {
                "Type a few letters of what you want to do, then pick it from the list or press Enter."
            }
            Setting::CutMode => {
                "Object outlines cuts only the outside shape of each object. All lines also cuts the lines inside it, like the holes in a letter O."
            }
            Setting::Strokes => {
                "How a drawn line (a stroke) is cut. Centre line cuts along its middle. Outline cuts around its thickness. Auto picks one for each line."
            }
            Setting::Speed => {
                "How fast the blade moves. Slower cuts are cleaner in thick or delicate material."
            }
            Setting::Force => {
                "How hard the blade presses. Too little and the cut does not go through; too much and it cuts into the backing sheet."
            }
            Setting::Passes => {
                "How often the plotter cuts each line. Thick material may need 2 or 3 passes."
            }
            Setting::ReverseAlternatePasses => {
                "Cuts every second pass the other way round, which can help with thick or fibrous material. With it on, closed shapes lift the blade between passes, because the blade cannot turn around while it is down. With it off, closed shapes are cut in one go, round and round without lifting."
            }
            Setting::Overcut => {
                "Overcut: the blade cuts a little past the start, so the shape closes cleanly and the piece comes off the mat."
            }
            Setting::BladeOffset => {
                "Blade offset: the tip of a drag blade trails behind the centre of the holder by this much. FlatCraft steers around corners to make up for it."
            }
            Setting::Keep => {
                "What you keep after cutting. Pieces keeps the shapes, Stencil keeps the sheet with holes, Both keeps everything."
            }
            Setting::Threshold => {
                "Tracing turns the picture into black and white first. Pixels darker than this become cut shapes."
            }
            Setting::AutoThreshold => {
                "Let FlatCraft pick the threshold: it counts the dark and the light pixels and splits them where they separate best (this is called Otsu's method). A good start for most pictures."
            }
            Setting::Invert => "Cut the light parts instead of the dark ones.",
            Setting::Despeckle => {
                "Specks smaller than this area are dropped, so dust and noise in the picture are not cut."
            }
            Setting::CornerThreshold => {
                "How sharp a turn must be to stay a corner, in degrees. A smaller value keeps more corners; a bigger value gives rounder shapes."
            }
            Setting::Smoothing => {
                "How much the traced lines are smoothed. 0 follows the pixels, 10 gives the smoothest curves."
            }
            Setting::MinFeature => {
                "Shapes thinner than this are dropped, because the blade cannot cut such thin parts cleanly. They would tear or stay stuck."
            }
            Setting::AutoMinFeature => {
                "Use 6 times the blade offset of the plotter as the smallest shape."
            }
            Setting::AutoArrangeSpacing => {
                "The gap left between objects, in millimetres. A pinned Auto arrange uses it too."
            }
            Setting::KnifeCompensation => {
                "Who steers the blade around corners. Firmware lets the plotter do it, Host lets FlatCraft do it, None cuts the plain lines."
            }
            Setting::MirrorX => {
                "Flips the x axis: x runs from the far edge of the mat instead of the near one. Turn it on only when this plotter counts x the other way round (without it, its cuts come out mirrored left to right). The Cameo 2 leaves it off."
            }
            Setting::MirrorY => {
                "Flips the y axis: y runs from the far end of the mat instead of the start. Turn it on only when this plotter counts y the other way round (without it, its cuts come out upside down). The Cameo 2 leaves it off."
            }
            Setting::TrackEnhancingSupport => {
                "Says whether this plotter has track enhancing. Track enhancing is a mode for the feed rollers (the rubber wheels that move the mat or sheet) that is meant to help long or heavy material run straight. A material can only use it when this is on."
            }
            Setting::RegistrationMarks => {
                "Says whether this plotter can find registration marks: small printed marks near the corners of a sheet that the plotter's light sensor looks for, so it can cut around a picture you printed. FlatCraft does not use registration marks yet, so this changes nothing today."
            }
            Setting::AutoBlade => {
                "Says whether this plotter sets the blade depth by itself (an auto blade). Without one, like on the Cameo 2, you turn the blade cap by hand to set the depth. FlatCraft does not use this yet, so this changes nothing today."
            }
            Setting::Calibrated => {
                "Turn this on once you know the grid values above match this mat exactly. While it is off, FlatCraft keeps every cut 5 mm inside the printed grid, to be safe. The calibration wizard that checks this for you arrives in milestone 3."
            }
            Setting::HiddenMat => {
                "Hides this built-in mat from the mat lists, so you only see the mats you own. Untick it to bring the mat back. A mat in use cannot be hidden: that is a mat an open document cuts on, or the mat new documents start on."
            }
            Setting::TrackEnhancing => {
                "Turns on track enhancing for this material: a mode for the feed rollers (the rubber wheels that move the mat or sheet) that is meant to help long or heavy material run straight. It only works when the plotter's own Track enhancing box is on too."
            }
            Setting::PresetName => {
                "The name this preset shows in the lists, so you can find it again. It does not change the cut."
            }
            Setting::Vendor => {
                "Who makes this plotter, for your own reference. It does not change the cut."
            }
            Setting::Model => {
                "The model name of this plotter, for your own reference. It does not change the cut."
            }
            Setting::Protocol => {
                "The command language the plotter understands. GPGL is the language of Silhouette plotters, and the only one FlatCraft speaks."
            }
            Setting::Dialect => {
                "Which family of GPGL this plotter speaks. FlatCraft builds jobs for Cameo 1 and 2 today; a job for Cameo 3 and later cannot be built yet."
            }
            Setting::Transport => {
                "How jobs reach the plotter. USB sends them down the cable to the plotter; USB cutting arrives in milestone 3. File writes each job into a folder as files, so you can look at it or send it yourself."
            }
            Setting::UsbVendorId => {
                "The number a USB device uses to say who made it (the vendor id), in hex. The Cameo 2 uses 0x0B4D. Change it only for another plotter model."
            }
            Setting::UsbProductId => {
                "The number a USB device uses to say which model it is (the product id), in hex. The Cameo 2 uses 0x112B. Change it only for another plotter model."
            }
            Setting::OutputFolder => {
                "The folder File Output writes its files into. Leave it empty to use the output folder from the General page, or Documents/FlatCraft Output when that is empty too."
            }
            Setting::FileExtension => {
                "The ending of the job file names File Output writes, for example gpgl. The readable listing gets .txt added after it."
            }
            Setting::UnitsPerMm => {
                "How many plotter steps make one millimetre. The Cameo 2 uses 20. Change it only for another plotter model. For small size errors, use Scale x and Scale y."
            }
            Setting::ScaleX => {
                "Stretches every cut along x (across the carriage) by this factor: 1 is true size, 1.01 is 1 % longer. Change it when a measured test cut is a little too long or too short that way."
            }
            Setting::ScaleY => {
                "Stretches every cut along y (the direction the mat feeds) by this factor: 1 is true size, 1.01 is 1 % longer. Change it when a measured test cut is a little too long or too short that way."
            }
            Setting::SpeedRange => {
                "The lowest or highest speed this plotter accepts. A material or job asking for a speed outside it is refused. The Cameo 2 takes 1 to 10."
            }
            Setting::ForceRange => {
                "The lowest or highest force this plotter accepts. A material or job asking for a force outside it is refused. The Cameo 2 takes 1 to 33."
            }
            Setting::PassesRange => {
                "The fewest or most passes this plotter allows. A material or job asking for a number outside it is refused. The Cameo 2 allows 1 to 20."
            }
            Setting::FlattenTolerance => {
                "Curves are cut as many short straight lines. This is how far those lines may stray from the true curve, in millimetres. Smaller is smoother but sends more points. Filled shapes thinner than this are not cut."
            }
            Setting::FcValue => {
                "The number FlatCraft sends to the plotter (the FC command) so its firmware steers the blade around corners. 18 is the usual value for the standard blade; 0 turns it off. FlatCraft accepts 0 to 100."
            }
            Setting::HostOffset => {
                "How far the blade tip trails behind the centre of the holder, in millimetres, for Host compensation (FlatCraft steers the blade itself). A job with Host compensation cannot be built yet, so this changes nothing today."
            }
            Setting::PivotThreshold => {
                "For Host compensation: at a corner that turns more than this many degrees, the blade swivels before the next line. 0 swivels at every corner. A job with Host compensation cannot be built yet, so this changes nothing today."
            }
            Setting::MatName => "The name this mat shows in the mat lists.",
            Setting::MatKind => {
                "Mat is a sticky cutting mat with a printed grid. No mat is material fed in on its own; FlatCraft then cuts it in bands from front to back, so the rollers never pull it back by more than a band."
            }
            Setting::MatWidth => {
                "The real width of the whole mat or sheet, edge to edge, in millimetres. Measure it with a ruler."
            }
            Setting::MatHeight => {
                "The real length of the whole mat or sheet, edge to edge, in millimetres. Measure it with a ruler."
            }
            Setting::GridOriginX => {
                "Where the printed grid starts on the mat, from its left edge, in millimetres. FlatCraft's 0,0 is the grid's 0,0, so a wrong value moves every cut."
            }
            Setting::GridOriginY => {
                "Where the printed grid starts on the mat, from its top edge, in millimetres. FlatCraft's 0,0 is the grid's 0,0, so a wrong value moves every cut."
            }
            Setting::GridWidth => {
                "The width of the printed grid, in millimetres. Cuts stay inside the grid, 5 mm inside while the mat is not calibrated."
            }
            Setting::GridHeight => {
                "The height of the printed grid, in millimetres. Cuts stay inside the grid, 5 mm inside while the mat is not calibrated."
            }
            Setting::MatColour => {
                "The colour FlatCraft paints this mat in on screen, so it looks like yours. It does not change the cut."
            }
            Setting::MatColourHex => {
                "The mat colour as a hex code, like #336699: two digits each for red, green and blue. Type one to change the colour."
            }
            Setting::BladeDepthHint => {
                "A note on how far to set a manual blade for this material, for example 3. The job settings show it as a reminder; FlatCraft does not send it to the plotter."
            }
            Setting::WeedBox => {
                "Weeding means peeling away the vinyl you do not need, so only your design stays on the backing. The weed box is a rectangle cut around the whole design: the waste outside it stays on the sheet for the next job, and the waste inside comes off in one go. Turn it on for stickers, decals and iron-on vinyl."
            }
            Setting::WeedMargin => {
                "How far the weed box stays from the design, in millimetres. A few millimetres give you an edge to grab when you start peeling. It is at least 1 mm, so the box never touches the design. It also sets how far the weed lines reach past the design, even with the box off."
            }
            Setting::WeedPerObject => {
                "One weed box around each object instead of one around everything. Handy when the objects lie far apart: each one is weeded on its own and less waste is lifted. Boxes that would overlap, touch another object or come closer than 5 mm to each other become one box, so no thin strip is left between two cuts."
            }
            Setting::WeedLines => {
                "Weed lines are straight cuts through the waste, so a big area of waste comes off in strips instead of one sheet that tears and sticks to itself. Lines run across the waste around your design, up to the weed box margin; with one box per object, only inside the boxes. They stop more than 1 mm before every cut of the design and never cut inside a shape: letters keep their insides, weed those by hand. Horizontal lines run across the mat, vertical lines along it, and Both makes a grid."
            }
            Setting::WeedSpacing => {
                "The distance between weed lines, in millimetres. Smaller gives smaller strips that are easier to peel, but more cutting. 20 mm suits most vinyl."
            }
            Setting::PresetNotes => {
                "Your own notes on this preset: what worked, what to watch out for. The job settings show them whenever this preset is picked. On a system preset they are kept in your own settings folder, so the preset itself stays as it ships. Agents can read them but never change them."
            }
            Setting::AddOverride => {
                "Pick a plotter to give this material its own speed, force, passes or overcut on that plotter only. Everything you leave alone keeps the material's value."
            }
            Setting::OverrideSet => {
                "Tick this to use a different value with this one plotter. While it is ticked, the value next to it counts for that plotter; untick it and the material's own value applies again."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_ids_are_the_preference_ids() {
        let ids: Vec<&str> = TOOLS.iter().map(|t| t.id).collect();
        assert_eq!(ids, flatcraft_engine::prefs::RIBBON_PIN_IDS);
        for id in flatcraft_engine::prefs::default_ribbon_pins() {
            assert!(by_id(&id).is_some(), "{id}");
        }
    }

    #[test]
    fn every_action_group_and_setting_has_an_icon_and_a_plain_card() {
        let plain = |s: &str, what: &str| {
            assert!(!s.trim().is_empty(), "{what} has no card text");
            assert!(!s.contains('\u{2014}'), "{what} has an em-dash");
            assert!(s.ends_with(['.', '!']), "{what}: full sentences");
        };
        for g in GROUPS {
            plain(g.description, g.title);
            assert!(!g.icon.svg().is_empty());
            assert!(
                g.id.tools().count() > 0 || g.id == GroupId::Size,
                "{}",
                g.title
            );
        }
        for t in TOOLS {
            plain(t.description, t.id);
            assert!(!t.icon.svg().is_empty());
            assert!(!t.title.contains('\u{2014}'));
        }
        for s in Setting::ALL {
            plain(s.card(), &format!("{s:?}"));
        }
    }

    #[test]
    fn align_pins_carry_their_option_in_icon_and_title() {
        for mode in [M::Left, M::Centre, M::Right, M::Top, M::Middle, M::Bottom] {
            let sel = by_act(Act::Align(mode, Selection));
            let area = by_act(Act::Align(mode, Area));
            assert_eq!(sel.title, area.title);
            assert_ne!(sel.icon, area.icon, "{mode:?}");
            assert_eq!(area.icon.framed_from(), Some(sel.icon));
            assert_ne!(sel.full_title(), area.full_title());
            assert!(area.full_title().ends_with("· cutting area"));
            assert!(area.id.ends_with(".area") && sel.id.ends_with(".selection"));
        }
    }

    #[test]
    fn every_action_icon_has_a_colour_role() {
        let icons = TOOLS
            .iter()
            .map(|t| t.icon)
            .chain(GROUPS.iter().map(|g| g.icon))
            .chain([Icon::EachObject, Icon::AsGroup]);
        for icon in icons {
            let (paper, accent) = icon_roles(icon).unwrap_or_else(|| panic!("{icon:?}"));
            assert!(!accent.is_empty(), "{icon:?} has no accent part");
            let n = super::super::icons::children(icon.framed_from().unwrap_or(icon)).len();
            for i in paper.iter().chain(accent) {
                assert!(*i < n, "{icon:?} has no child {i}");
            }
            assert!(
                paper.iter().all(|p| !accent.contains(p)),
                "{icon:?}: a child is paper and accent"
            );
        }
    }

    #[test]
    fn shortcuts_name_actions_of_the_table() {
        for s in super::super::help::SHORTCUTS.iter() {
            if let Some(id) = s.tool {
                let t = by_id(id).unwrap_or_else(|| panic!("{id}"));
                assert_eq!(t.shortcut(), s.prepare, "{id}");
            }
        }
        assert_eq!(by_act(Act::Import).shortcut(), Some(PrepareKey::Import));
        assert_eq!(
            by_act(Act::Duplicate).shortcut(),
            Some(PrepareKey::Duplicate)
        );
        assert_eq!(by_act(Act::Delete).shortcut(), Some(PrepareKey::Delete));
        assert_eq!(by_act(Act::Trace).shortcut(), None);
    }

    #[test]
    fn every_name_is_unique() {
        let mut names: Vec<String> = Vec::new();
        for t in TOOLS {
            names.push(t.button_name());
            // The rows of the two Align options never show at once.
            if t.option() != Some(AlignTo::Area) {
                names.push(t.row_name());
            }
            names.push(format!("Pin {}", t.pin_label()));
        }
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "{names:?}");
        assert_eq!(
            by_act(Act::AutoArrangePanel).button_name(),
            "Auto arrange..."
        );
        assert_eq!(by_act(Act::AutoArrange).button_name(), "Auto arrange");
    }

    #[test]
    fn numbers_lose_their_trailing_zeros() {
        assert_eq!(trim_number(5.0), "5");
        assert_eq!(trim_number(2.5), "2.5");
        assert_eq!(trim_number(0.25), "0.25");
    }
}
