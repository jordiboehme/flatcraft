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
use super::collapse::Fold;
use super::help::PrepareKey;
use super::icons::Icon;
use super::prepare::{PrepareState, RibbonTab, Snapshot};
use super::rails::SidePanel;
use super::{Action, ribbon};
use flatcraft_engine::commands::group::ReorderTo;

/// A group of the ribbon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupId {
    Add,
    Edit,
    Files,
    Size,
    Transform,
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
    /// Its places in its tab's fold order: the step at which it goes
    /// icon-only and the one at which it becomes its tile (see
    /// [`super::collapse`]). The least used group folds first.
    pub fold: Fold,
}

const fn fold(icons: u8, tile: Option<u8>) -> Fold {
    Fold { icons, tile }
}

/// Every group, left to right on its tab.
pub const GROUPS: [GroupDef; 9] = [
    GroupDef {
        id: GroupId::Add,
        tab: RibbonTab::Layout,
        title: "Add",
        icon: Icon::Plus,
        description: "Bring designs onto the mat: SVG files (the drawing format most design apps save), pictures to trace, or the test sheet.",
        fold: fold(8, Some(11)),
    },
    GroupDef {
        id: GroupId::Edit,
        tab: RibbonTab::Layout,
        title: "Edit",
        icon: Icon::Pencil,
        description: "Pick what to work on, then copy or remove it: one copy, many copies, or copies in rows or around a centre. The Select and Lasso switches decide what a drag on the empty mat draws: a box or a free shape. Undo takes back any step.",
        fold: fold(7, Some(9)),
    },
    GroupDef {
        id: GroupId::Size,
        tab: RibbonTab::Layout,
        title: "Position and size",
        icon: Icon::Scale,
        description: "Where the selection sits on the mat and how big it is, in millimetres, or its scale in percent. Type a number, then press Tab or Enter or click elsewhere. Esc puts the old value back.",
        fold: fold(6, None),
    },
    GroupDef {
        id: GroupId::Transform,
        tab: RibbonTab::Layout,
        title: "Transform",
        icon: Icon::Transform,
        description: "Move, turn and flip the selection by exact amounts; scale it in the Position and size fields. The Each object switch decides how scale, rotate and mirror work on several objects: each about its own centre, or the selection as one piece. Moving always moves the selection as one piece. Pin the ones you use most to the ribbon.",
        fold: fold(5, Some(10)),
    },
    GroupDef {
        id: GroupId::Arrange,
        tab: RibbonTab::Layout,
        title: "Arrange",
        icon: Icon::Arrange,
        description: "Line objects up, space them evenly, pack them onto the cutting area, or group them so they act as one. Pin the ones you use most to the ribbon.",
        fold: fold(3, Some(4)),
    },
    GroupDef {
        id: GroupId::Files,
        tab: RibbonTab::Layout,
        title: "Linked files",
        icon: Icon::Linked,
        description: "An imported object stays linked to its file. Change the file in another app, then read it again here.",
        fold: fold(1, Some(2)),
    },
    GroupDef {
        id: GroupId::Zoom,
        tab: RibbonTab::View,
        title: "Zoom",
        icon: Icon::ZoomIn,
        description: "How close you look at the mat. It never changes what is cut.",
        fold: fold(4, Some(6)),
    },
    GroupDef {
        id: GroupId::Show,
        tab: RibbonTab::View,
        title: "Show",
        icon: Icon::Eye,
        description: "Helpers drawn on the canvas to place things by eye. They are never cut.",
        fold: fold(3, Some(5)),
    },
    GroupDef {
        id: GroupId::Panels,
        tab: RibbonTab::View,
        title: "Panels",
        icon: Icon::Panels,
        description: "Show or hide the side panels to make room for the mat.",
        fold: fold(1, Some(2)),
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

/// The fold table of a tab: which of its groups (by place, left to right)
/// goes icon-only or to its tile at each step, from [`GroupDef::fold`].
pub fn fold_table(tab: RibbonTab) -> Vec<(usize, super::collapse::Mode)> {
    let folds: Vec<Option<Fold>> = groups_of(tab).map(|g| Some(g.fold)).collect();
    super::collapse::ordered_steps(&folds).expect("every group has its fold places")
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
    Group,
    Ungroup,
    /// Bring to front, Bring forward, Send backward, Send to back.
    Order(ReorderTo),
    RectangularPattern,
    CircularPattern,
    AutoArrangePanel,
    AutoArrange,
    ZoomFit,
    Zoom100,
    Grid,
    Rulers,
    KeptWaste,
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
    Group,
    Order,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Mirror => "Mirror",
            Section::Align => "Align",
            Section::Distribute => "Distribute",
            Section::Group => "Groups",
            Section::Order => "Order",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Section::Mirror => Icon::Mirror,
            Section::Align => Icon::Align,
            Section::Distribute => Icon::DistributeHorizontal,
            Section::Group => Icon::Group,
            Section::Order => Icon::BringToFront,
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
    /// The label of its big button when the title is too long for the
    /// ribbon ("Pattern" for "Rectangular pattern..."); see
    /// [`Tool::ribbon_label`].
    pub label: Option<&'static str>,
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
        label: None,
    }
}

const fn in_section(mut t: Tool, section: Section) -> Tool {
    t.section = Some(section);
    t
}

/// A big button whose title is too long for the ribbon: it shows `label`
/// there, and its full title everywhere else.
const fn labelled(mut t: Tool, label: &'static str) -> Tool {
    t.label = Some(label);
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
pub const TOOLS: [Tool; 45] = [
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
    labelled(
        tool(
            "test_patterns",
            Act::TestPatterns,
            "Test patterns",
            Icon::TestPatterns,
            G::Add,
            Big,
            "Add the test sheet: small shapes that show how well blade and settings cut. Cut it first on a new machine or material.",
        ),
        "Tests",
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
    labelled(
        tool(
            "pattern.rectangular",
            Act::RectangularPattern,
            "Rectangular pattern...",
            Icon::RectangularPattern,
            G::Edit,
            Big,
            "Copies of one object in columns and rows, with the gap you choose. Sticker sheet, engage!",
        ),
        "Pattern",
    ),
    labelled(
        tool(
            "pattern.circular",
            Act::CircularPattern,
            "Circular pattern...",
            Icon::CircularPattern,
            G::Edit,
            Big,
            "Copies around a centre point, like the numbers on a clock.",
        ),
        "Circular",
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
        G::Transform,
        Big,
        "Move the selection by an exact distance in mm, or leave copies behind at every step.",
    ),
    tool(
        "rotate",
        Act::Rotate,
        "Rotate...",
        Icon::Rotate,
        G::Transform,
        Big,
        "Turn the selection by an exact angle in degrees. A positive angle turns clockwise.",
    ),
    tool(
        "rotate.left",
        Act::RotateLeft,
        "Rotate -90",
        Icon::RotateLeft,
        G::Transform,
        Small,
        "A quarter turn anticlockwise. With Each object on, every object turns about its own centre.",
    ),
    tool(
        "rotate.right",
        Act::RotateRight,
        "Rotate +90",
        Icon::Rotate,
        G::Transform,
        Small,
        "A quarter turn clockwise. With Each object on, every object turns about its own centre.",
    ),
    in_section(
        tool(
            "mirror.horizontal",
            Act::MirrorHorizontal,
            "Mirror horizontally",
            Icon::Mirror,
            G::Transform,
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
            G::Transform,
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
        "auto_arrange",
        Act::AutoArrange,
        "Auto arrange",
        Icon::Arrange,
        G::Arrange,
        Big,
        "Packs the selected objects onto the cutting area, Tetris style, in one click. Everything else stays where you put it, and the packed objects fit around it. Nothing selected: it packs every visible object. It uses the gap you last chose in its card: the small arrow on its right opens it.",
    ),
    labelled(
        tool(
            "auto_arrange.panel",
            Act::AutoArrangePanel,
            "Auto arrange...",
            Icon::AutoArrangePanel,
            G::Arrange,
            Big,
            "Packs the selected objects onto the cutting area, Tetris style, with the gap you choose. Everything else stays where you put it, and the packed objects fit around it. Nothing selected: it packs every visible object. A preview shows where they land. On the ribbon it is the small arrow on the right of Auto arrange.",
        ),
        // No button of its own: it is the chevron part of the Auto
        // arrange split button (`groups::split_button`), named by its card.
        "",
    ),
    in_section(
        tool(
            "group",
            Act::Group,
            "Group",
            Icon::Group,
            G::Arrange,
            Small,
            "Put the selected objects together so they act as one: one click selects them all, and they move, turn, line up and get packed as one piece. Each keeps its own cut settings. Double-click a member to change it alone. Like the Fellowship: many members, one quest.",
        ),
        Section::Group,
    ),
    in_section(
        tool(
            "ungroup",
            Act::Ungroup,
            "Ungroup",
            Icon::Ungroup,
            G::Arrange,
            Small,
            "Take the selected group apart: its members stay where they are and act on their own again. A group inside it stays a group. With members picked inside a group (double-click), only those leave it.",
        ),
        Section::Group,
    ),
    in_section(
        tool(
            "order.front",
            Act::Order(ReorderTo::Front),
            "Bring to front",
            Icon::BringToFront,
            G::Arrange,
            Small,
            "Put the selection in front of every other object. Front and back matter where objects overlap: the canvas draws the front one on top, and an object behind in Visible outline is not cut where the front one covers it. A group moves as one.",
        ),
        Section::Order,
    ),
    in_section(
        tool(
            "order.forward",
            Act::Order(ReorderTo::Forward),
            "Bring forward",
            Icon::BringForward,
            G::Arrange,
            Small,
            "Move the selection one step toward the front: in front of the next object, or of the next whole group. Inside a group it steps among the group's members only. Use it to sort a few overlapping layers, one step at a time.",
        ),
        Section::Order,
    ),
    in_section(
        tool(
            "order.backward",
            Act::Order(ReorderTo::Backward),
            "Send backward",
            Icon::SendBackward,
            G::Arrange,
            Small,
            "Move the selection one step toward the back: behind the object, or the whole group, right behind it. Inside a group it steps among the group's members only.",
        ),
        Section::Order,
    ),
    in_section(
        tool(
            "order.back",
            Act::Order(ReorderTo::Back),
            "Send to back",
            Icon::SendToBack,
            G::Arrange,
            Small,
            "Put the selection behind every other object, like the bottom layer of a layered sticker.",
        ),
        Section::Order,
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
    labelled(
        tool(
            "view.kept",
            Act::KeptWaste,
            "Show kept and waste",
            Icon::Layers,
            G::Show,
            Big,
            "Shows what you keep after cutting as plain paper and the waste, the part you throw away, with a light hatch. Each object's Keep setting decides it: a sticker keeps its shape, a stencil keeps the sheet. The Cut workspace always shows it. It needs a job that can be planned.",
        ),
        "Kept & waste",
    ),
    labelled(
        tool(
            "panel.objects",
            Act::ObjectsPanel,
            "Objects panel",
            Icon::PanelLeft,
            G::Panels,
            Big,
            "The list of everything on the mat. Hide, skip, rename or pick objects there.",
        ),
        "Objects",
    ),
    labelled(
        tool(
            "panel.settings",
            Act::SettingsPanel,
            "Settings panel",
            Icon::PanelRight,
            G::Panels,
            Big,
            "What is cut and how: cut mode, plotter, mat, material, speed and force.",
        ),
        "Settings",
    ),
    labelled(
        tool(
            "panel.debug",
            Act::DebugPanel,
            "Debug panel",
            Icon::Debug,
            G::Panels,
            Big,
            "For the curious: what the app planned and what goes to the plotter. You do not need it to cut.",
        ),
        "Debug",
    ),
];

/// How an icon is coloured (Jordi's "Paper (crisp)" style): which of its
/// SVG children (in file order) are paper, the shapes that get cut, filled
/// with the paper colour and outlined with the cut colour, and which are
/// the one accent part that tells what happens, in the accent colour.
/// Every other child is ink. Every icon of an action, a group or a
/// switch has an entry; the framed Align icons use their base icon's.
pub const ICON_ROLES: [(Icon, &[usize], &[usize]); 55] = [
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
    // The top sheet is the paper you keep, the ones below the rest.
    (Icon::Layers, &[0], &[1, 2]),
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
    (Icon::EachObject, &[1, 2], &[0]),
    (Icon::AsGroup, &[], &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
    (Icon::Group, &[4, 5], &[0, 1, 2, 3]),
    (Icon::Ungroup, &[1], &[0]),
    // The square that moves is the accent, the other two are paper.
    (Icon::BringToFront, &[1, 2], &[0]),
    (Icon::SendToBack, &[0, 1], &[2, 3]),
    // The arrow head shows the way.
    (Icon::BringForward, &[], &[0]),
    (Icon::SendBackward, &[], &[0]),
    (Icon::AutoArrangePanel, &[0, 2], &[1, 3]),
    // The four handles of the frame.
    (Icon::Transform, &[], &[4, 5, 6, 7]),
    // Home's New cards.
    (Icon::NewFile, &[0, 1], &[2, 3]),
    (Icon::FolderOpen, &[], &[0]),
    // The Cut bar's stages: the blade, and the two cuts around the travel.
    (Icon::Plotter, &[], &[2]),
    (Icon::RouteTravel, &[], &[0, 2]),
    // The keep picker: the glass tube is paper, the drop at its tip the
    // accent (the part you pick).
    (Icon::Pipette, &[0], &[2]),
    (Icon::Pointer, &[], &[0]),
    (Icon::Lasso, &[], &[0]),
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

    /// What its big button shows on the ribbon: its short label, or else
    /// [`Tool::short_title`]. The card, the lists, the pins and search keep
    /// the full title, and the button keeps [`Tool::button_name`] as its
    /// accessible name. An empty label makes an icon-only big button.
    pub fn ribbon_label(&self) -> &'static str {
        self.label.unwrap_or_else(|| self.short_title())
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
            Act::Group => "object.group",
            Act::Ungroup => "object.ungroup",
            Act::Order(_) => "object.reorder",
            Act::CircularPattern => "object.circular_copies",
            Act::AutoArrangePanel | Act::AutoArrange => "arrange.auto",
            _ => return None,
        })
    }

    /// Why the action cannot run now, as a full sentence that says what to
    /// do, or `None` when it can.
    pub fn blocked(&self, snap: &Snapshot) -> Option<&'static str> {
        let ids: Vec<u64> = snap.selection.iter().copied().collect();
        self.blocked_for(snap, &ids)
    }

    /// The title it shows for the objects `ids`: Ungroup says what it will
    /// do ("Remove 2 objects from group").
    pub fn live_title(&self, snap: &Snapshot, ids: &[u64]) -> String {
        match self.act {
            Act::Ungroup => ungroup_plan(snap, ids).title(),
            _ => self.title.to_string(),
        }
    }

    /// The text of its hover card for the objects `ids`.
    pub fn live_description(&self, snap: &Snapshot, ids: &[u64]) -> &'static str {
        match self.act {
            Act::Ungroup if !ungroup_plan(snap, ids).dissolve.is_empty() => self.description,
            Act::Ungroup if !ungroup_plan(snap, ids).release.is_empty() => RELEASE_CARD,
            _ => self.description,
        }
    }

    /// Like [`Self::blocked`], for the objects `ids` (the context menu
    /// works on the objects it was opened for).
    pub fn blocked_for(&self, snap: &Snapshot, ids: &[u64]) -> Option<&'static str> {
        // Counted as the engine counts: a whole group is one object.
        let n = if matches!(
            self.act,
            Act::Group
                | Act::Ungroup
                | Act::Align(_, AlignTo::Selection)
                | Act::DistributeHorizontal
                | Act::DistributeVertical
                | Act::Copies
                | Act::RectangularPattern
        ) {
            snap.units_of(ids).len()
        } else {
            ids.len()
        };
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
        // One long command at a time (`super::long`).
        if snap.busy && matches!(self.act, Act::Import | Act::Trace | Act::Reload) {
            return Some(super::long::BUSY_CARD);
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
            Act::KeptWaste => kept_waste_blocked(snap),
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
            Act::Group => flatcraft_engine::commands::group::group_refusal(n),
            Act::Order(to) => {
                if n == 0 {
                    Some(SELECT)
                } else {
                    order_refusal(snap, ids, to)
                }
            }
            Act::Ungroup => ungroup_plan(snap, ids)
                .is_empty()
                .then_some(flatcraft_engine::commands::group::UNGROUP_REFUSAL),
            Act::AutoArrangePanel | Act::AutoArrange => {
                if snap.objects.is_empty() {
                    Some("Import something first. There is nothing to arrange yet.")
                } else if n > 0 && snap.selected().all(|o| !o.visible) {
                    // `n` 0 is the empty mat's menu: it clears the
                    // selection and packs every visible object, so the
                    // selection the snapshot still holds does not count.
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
            Act::KeptWaste => st.show_kept_waste,
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
            Act::Group => push(actions, group_action(doc_id, &ids)),
            Act::Ungroup => push(actions, ungroup_action(doc_id, &ids)),
            Act::Order(to) => push(actions, reorder_action(doc_id, &ids, to)),
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
            Act::KeptWaste => st.show_kept_waste = !st.show_kept_waste,
            Act::ObjectsPanel => st.panels.toggle(SidePanel::Objects),
            Act::SettingsPanel => {
                // An open tool card holds the panel open: hiding the
                // panel cancels the card.
                let shown = !st.panels.collapsed(SidePanel::Settings);
                if shown {
                    st.panel = None;
                    st.panels.card_open = false;
                }
                st.panels.set_collapsed(SidePanel::Settings, shown);
            }
            Act::DebugPanel => *show_debug = !*show_debug,
        }
    }
}

/// Why Show kept and waste has nothing to show: no object, or none that
/// is cut. `None` when a job may be planned (a plan that still fails says
/// why on the canvas).
pub fn kept_waste_blocked(snap: &Snapshot) -> Option<&'static str> {
    if snap.objects.is_empty() {
        Some(
            "Nothing can be planned yet: import something first. Then this shows what you keep and what is waste.",
        )
    } else if !snap.objects.iter().any(|o| o.visible && o.cut_enabled) {
        Some(
            "Nothing can be planned yet: every object is hidden or skipped. Show an object or turn its cut back on first.",
        )
    } else {
        None
    }
}

/// What Ungroup does with the objects `ids` (see `UngroupPlan`).
pub fn ungroup_plan(
    snap: &Snapshot,
    ids: &[u64],
) -> flatcraft_engine::commands::group::UngroupPlan {
    let units: Vec<flatcraft_core::document::ObjectId> = snap
        .units_of(ids)
        .into_iter()
        .map(flatcraft_core::document::ObjectId)
        .collect();
    flatcraft_engine::commands::group::UngroupPlan::of(&snap.tree, &units)
}

/// The card of Ungroup when it takes members out of their group.
pub const RELEASE_CARD: &str = "Take the picked members out of their group: they stay where they are and act on their own again. The rest of the group stays together. A group left with one member is no group.";

/// The `object.group` of Group (the ribbon, the context menu, Cmd+G).
pub fn group_action(doc_id: u64, ids: &[u64]) -> Action {
    Action::run(doc_id, "object.group", json!({ "ids": ids }))
}

/// The `object.ungroup` of Ungroup (the ribbon, the context menu,
/// Shift+Cmd+G).
pub fn ungroup_action(doc_id: u64, ids: &[u64]) -> Action {
    Action::run(doc_id, "object.ungroup", json!({ "ids": ids }))
}

/// Why the order action `to` changes nothing for the objects `ids` (they
/// are already in front, or at the back), or `None` when it moves them.
/// The same rule as `object.reorder`.
pub fn order_refusal(snap: &Snapshot, ids: &[u64], to: ReorderTo) -> Option<&'static str> {
    use flatcraft_core::document::ObjectId;
    let order: Vec<ObjectId> = snap.objects.iter().map(|o| ObjectId(o.id)).collect();
    let leaves: Vec<ObjectId> = ids
        .iter()
        .flat_map(|id| snap.leaves_of(*id))
        .map(ObjectId)
        .collect();
    let moved = flatcraft_engine::commands::group::reordered(&order, &snap.tree, &leaves, to);
    (moved == order).then_some(to.refusal())
}

/// The `object.reorder` of the order actions (the ribbon, the context
/// menu, Cmd+] and Cmd+[).
pub fn reorder_action(doc_id: u64, ids: &[u64], to: ReorderTo) -> Action {
    Action::run(
        doc_id,
        "object.reorder",
        json!({ "ids": ids, "to": to.key() }),
    )
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
static SELECT_CARD: LazyLock<String> =
    LazyLock::new(|| super::help::select_card(super::help::Os::CURRENT));
static LASSO_CARD: LazyLock<String> =
    LazyLock::new(|| super::help::lasso_card(super::help::Os::CURRENT));
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
    SelectTool,
    LassoTool,
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
    Backing,
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
    RingDirection,
    CornerMode,
    CornerOvershoot,
    MinRadius,
    MinRadiusOwn,
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
    MaterialGroup,
    MatTack,
    PresetTips,
    AddOverride,
    PresetNotes,
    WeedBox,
    WeedMargin,
    WeedPerObject,
    WeedLines,
    WeedSpacing,
}

impl Setting {
    pub const ALL: [Setting; 89] = [
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
        Setting::SelectTool,
        Setting::LassoTool,
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
        Setting::Backing,
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
        Setting::RingDirection,
        Setting::CornerMode,
        Setting::CornerOvershoot,
        Setting::MinRadius,
        Setting::MinRadiusOwn,
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
        Setting::MaterialGroup,
        Setting::MatTack,
        Setting::PresetTips,
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
            | GridOriginX | GridOriginY | GridWidth | GridHeight | WeedMargin | WeedSpacing
            | CornerOvershoot | MinRadius => Kind::Length,
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
            Setting::SelectTool => SELECT_CARD.as_str(),
            Setting::LassoTool => LASSO_CARD.as_str(),
            Setting::Angle => ANGLE_CARD.as_str(),
            Setting::EachObject => {
                "Separate shapes (on): the Rotate and Mirror buttons and the % field work on each selected object about its own centre. One dashed frame (off): the selection moves as one piece, like Voltron. A group always counts as one object and turns about its own centre. The Angle field sets the angle of each object on its own, and a group turns as one piece to its own angle. Turning by hand on the mat always turns the selection as one piece."
            }
            Setting::Search => {
                "Type a few letters of what you want to do, then pick it from the list or press Enter."
            }
            Setting::CutMode => {
                "What the knife follows. Object outlines cuts around every shape you see, holes included. All lines cuts every line as drawn. Merged outline welds overlapping objects into one piece (they need the same Keep choice, and the weld belongs to the object furthest back). Visible outline leaves out what objects in front cover, so a shared edge is cut once. Hover a choice in the list to read more."
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
                "What you keep after cutting, so the blade starts and turns only in the waste, the part you throw away. Pieces keeps the cut shapes, like stickers you peel off the sheet. Stencil keeps the sheet with the holes, like a stencil you paint through: the shapes fall out. Both keeps both sides: the shapes and the sheet around them, like a puzzle you cut but take apart later. The blade never starts or overshoots there, it only turns on the spot. The sheet is shared, so other objects on it keep both sides too. A stencil with a cut frame around it uses Pieces: Stencil is for shapes cut out of a sheet that stays. Where objects meet, kept wins. Turn on Show kept and waste in View to see it on the mat. Not sure? The pipette button next to it lets you click the part that stays, right on the mat."
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
            Setting::Backing => {
                "The material sits on a backing sheet, like vinyl. Small loose parts stay on it until you lift them with transfer tape. Vinyl, sticker paper and masking film sit on a backing sheet; copy paper and cardstock do not, so there the middle of a stencil's O falls out."
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
                "How far the blade tip trails behind the centre of the holder, in millimetres, for Host compensation (FlatCraft steers the blade itself). FlatCraft steers with this value. The simulator and the tight curve warning use the blade offset, your real blade; when the two differ, the job says so."
            }
            Setting::PivotThreshold => {
                "For Host compensation: at a corner that turns more than this many degrees, the blade swivels before the next line. 0 swivels at every corner. The Fast cut strategy uses at least 0.5 degrees."
            }
            Setting::RingDirection => {
                "Which way round the blade cuts closed shapes (rings, like a circle or the outline of a letter), as you see them on the canvas: counter-clockwise (against the hands of a clock) or clockwise. A drag blade trails behind its holder, so cutting every ring the same way gives the same kind of corner everywhere. Counter-clockwise is the usual choice. Change it only when test cuts come out cleaner the other way. It only changes the order of the points, never the shape."
            }
            Setting::CornerMode => {
                "What the blade does at a sharp corner when FlatCraft steers it (Host knife compensation). Swivel (arc): the blade stays down and turns on the spot around the corner point. Fine for vinyl and thin paper. Lift and overshoot: the blade cuts a little past the corner, lifts, and starts the next line fresh. Thick or dense material like heavy card holds a buried blade so tight that it turns badly, and this gives cleaner corners there. FlatCraft only overshoots where the extra cut lands in the waste, and curves are never lifted. With Firmware compensation the plotter handles corners itself, so this changes nothing there."
            }
            Setting::CornerOvershoot => {
                "For Lift and overshoot: how far the blade cuts past a sharp corner before it lifts, in millimetres. The overshoot is the small extra cut beyond the corner; it makes sure the corner is cut through. 0.1 mm is the usual value. It always lands in the waste."
            }
            Setting::MinRadius => {
                "The smallest curve radius this material cuts cleanly, in millimetres. The radius says how tight a curve is: a smaller radius is a tighter curve. Curves tighter than this get a warning before the cut, because the blade may round or tear them. This works with every knife compensation. Raise it after a test cut shows torn small curves."
            }
            Setting::MinRadiusOwn => {
                "Tick this to give this material its own minimum radius. It starts at 0.25 mm, the usual blade offset. While it is off, the field shows blade offset: the blade offset of the plotter you cut with is used, which suits most materials."
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
                "The colour FlatCraft paints this mat in on screen, so it looks like yours. Use default follows the mat colour from Settings, Appearance, which differs for the light and the dark theme. Pick Custom when this mat has its own look, for example a green cutting mat. It does not change the cut."
            }
            Setting::MatColourHex => {
                "The mat colour as a hex code, like #336699: two digits each for red, green and blue. Type one to change the colour. Dimmed while the mat uses the default colour."
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
            Setting::MaterialGroup => {
                "Where this material shows in the material picker: Vinyl, Iron-on, Paper and card, or Film and other. Iron-on materials are cut mirrored, so the Cut stage reminds you to mirror the design when nothing is mirrored."
            }
            Setting::MatTack => {
                "The mat this material likes. Tack is how sticky the cutting mat is: light for thin paper and vellum, so it comes off without tearing; standard for vinyl and most card; strong for heavy card and stiff sheets that would shift. Only a recommendation: FlatCraft shows it and never sends it."
            }
            Setting::PresetTips => {
                "Tips that come with this built-in material: which side goes up, and how to weed. A preset made from it keeps them. Write your own experience in the notes below."
            }
            Setting::PresetNotes => {
                "Your own notes on this preset: what worked, what to watch out for. The job settings show them whenever this preset is picked. On a system preset they are kept in your own settings folder, so the preset itself stays as it ships. Agents can read them but never change them."
            }
            Setting::AddOverride => {
                "Pick a plotter to give this material its own speed, force, passes, overcut, corner mode or minimum radius on that plotter only. Everything you leave alone keeps the material's value."
            }
            Setting::OverrideSet => {
                "Tick this to use a different value with this one plotter. While it is ticked, the value next to it counts for that plotter; untick it and the material's own value applies again."
            }
        }
    }
}

// ---------------------------------------------------------------- tool cards

/// The hover cards of the fields of the tool cards (see
/// `inspector::tool_card`), in plain words for beginners. A command's
/// parameter is keyed by the command id and the parameter's path; a
/// window-only field by its card and its own id. The label, the unit and
/// the range come from the engine (`commands::meta` and the schema).
pub const FIELD_CARDS: &[(&str, &str, &str)] = &[
    (
        "object.move",
        "dx_mm",
        "How far the selection moves to the right. A minus number moves it to the left. With Copy on, each copy sits this far from the one before.",
    ),
    (
        "object.move",
        "dy_mm",
        "How far the selection moves down. A minus number moves it up. With Copy on, each copy sits this far from the one before.",
    ),
    (
        "object.duplicate",
        "offset_mm[0]",
        "How far each copy sits to the right of the one before. A minus number puts it to the left.",
    ),
    (
        "object.duplicate",
        "offset_mm[1]",
        "How far each copy sits below the one before. A minus number puts it above.",
    ),
    (
        "object.duplicate",
        "count",
        "How many copies to add of each object. Copy 2 sits two offsets away, copy 3 three, like footprints in the sand.",
    ),
    (
        "move_copy",
        "copy",
        "On: the originals stay where they are and copies are added, each one offset further. Off: the selection itself moves.",
    ),
    (
        "object.rotate",
        "degrees",
        "How far to turn. A positive angle turns clockwise, like the hands of a clock; a minus angle turns the other way.",
    ),
    (
        "object.rotate",
        "about",
        "As one group: the selection turns about its common centre, like Voltron, so the layout turns with it. Each about its own centre: every object spins where it is. OK also sets the Each object switch on the ribbon.",
    ),
    (
        "object.copies",
        "count",
        "How many copies to add next to the original.",
    ),
    (
        "object.copies",
        "layout.grid.columns",
        "How many cells sit side by side in each row. The original is the top-left cell.",
    ),
    (
        "pattern.rectangular",
        "rows",
        "How many rows of cells. Columns times rows is the whole pattern, the original included.",
    ),
    (
        "object.copies",
        "gap_x_mm",
        "The gap between two neighbours side by side, from edge to edge. 0 makes them touch.",
    ),
    (
        "object.copies",
        "gap_y_mm",
        "The gap between two rows, from edge to edge. 0 makes them touch.",
    ),
    (
        "object.copies",
        "linked",
        "One design, many bodies, like the Clone Army: the copies share the source, the cut options, the keep mode and the scale. Each keeps its own place, rotation and mirror.",
    ),
    (
        "object.circular_copies",
        "count",
        "How many objects sit around the circle, the original included. 12 makes the numbers on a clock.",
    ),
    (
        "object.circular_copies",
        "total_deg",
        "How far around the circle the copies spread. Only a full turn (360 deg) shares them out evenly. A smaller angle puts the last copy at the end of the arc.",
    ),
    (
        "pattern.circular",
        "centre",
        "What the circle turns about: the centre of the selection, or a point you type in Centre X and Centre Y.",
    ),
    (
        "object.circular_copies",
        "center_mm[0]",
        "Where the centre of the circle is, measured from the left edge of the mat's printed grid.",
    ),
    (
        "object.circular_copies",
        "center_mm[1]",
        "Where the centre of the circle is, measured down from the top edge of the mat's printed grid.",
    ),
    (
        "object.circular_copies",
        "rotate_copies",
        "On: each copy turns with the circle, like the spokes of a wheel. Off: they all keep the original's angle.",
    ),
    (
        "object.circular_copies",
        "linked",
        "The copies share the source, the cut options, the keep mode and the scale with their original. Each keeps its own place, rotation and mirror.",
    ),
    (
        "arrange.auto",
        "spacing_mm",
        "The gap left between packed objects, and between them and the objects that stay. A pinned Auto arrange uses it too.",
    ),
];

/// The hover card of the tool card field `key` of `owner` (a command id,
/// or a card's own id for a window-only field).
pub fn field_card(owner: &str, key: &str) -> Option<&'static str> {
    FIELD_CARDS
        .iter()
        .find(|(o, k, _)| *o == owner && *k == key)
        .map(|(_, _, c)| *c)
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
        // Every group has an icon of its own: no other group and no
        // action uses it (Arrange shares its icon with the one-click Auto
        // arrange, its namesake).
        for g in GROUPS {
            let others = GROUPS.iter().filter(|o| o.icon == g.icon).count();
            assert_eq!(others, 1, "{}", g.title);
            let actions: Vec<&str> = TOOLS
                .iter()
                .filter(|t| t.icon == g.icon && t.act != Act::AutoArrange)
                .map(|t| t.id)
                .collect();
            assert!(actions.is_empty(), "{}: {actions:?}", g.title);
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
            .chain([Icon::EachObject, Icon::AsGroup, Icon::Pointer, Icon::Lasso]);
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
