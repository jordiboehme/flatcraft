//! The Settings window: one frame for the app preferences and the plotter,
//! material and mat presets. It opens from the Settings button and the
//! Settings menu item (Cmd+,).
//!
//! - The header has the title, a search field (Cmd+F, Ctrl+F elsewhere)
//!   and a ×. The sidebar lists the pages under group headings. Each page
//!   starts with its title and one sentence on what it is for, then
//!   collapsible sections with a label on the left and the control on the
//!   right.
//! - Preference changes are staged in a draft copy of [`Preferences`].
//!   Apply and OK store the fields the draft changed (read-modify-write
//!   through `Session::update_prefs`), Cancel, × and Escape drop them. The
//!   theme shows at once while it is staged; Cancel takes it back.
//! - The preset pages keep their own Save preset, Save as, Revert and
//!   Delete. Apply and OK store only the staged preferences; closing with
//!   unsaved preset edits asks "Discard your changes?" first.
//!
//! Every preference has a row in [`ROWS`]: the sidebar pages, the ⓘ
//! cards, search and "Restore page defaults" read it. No registry command
//! changes the preferences: this window is the only way (agent limit).

use crate::ui::metrics;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use egui::{
    Align, Color32, Key, Layout, Modifiers, Rect, Response, RichText, Sense, Ui, Vec2, pos2, vec2,
};
use flatcraft_core::document::CutStrategy;
use flatcraft_engine::prefs::{PanelUser, Preferences, Theme, default_ribbon_pins};

use super::calc;
use super::icons::{self, Icon};
use super::profiles_window::{self, Leave, Request};
use super::settings::set_name;
use super::theme::{self, Tokens};
use super::tools::Setting;
use super::{Action, FlatCraftApp, Toast, fonts, help, lock};

/// The card of Restore page defaults.
pub const RESTORE: &str = "Puts the settings of this page back to how a new install has them. Other pages keep their values. Apply or OK stores it.";
/// Its card on the General page, which holds the hidden tips.
pub const RESTORE_GENERAL: &str = "Puts the settings of this page back to how a new install has them, and shows your hidden tips again. Other pages keep their values. Apply or OK stores it.";

/// The button that shows the hidden tips again.
pub const SHOW_TIPS: &str = "Show hidden tips again";
/// Why it is dimmed.
pub const NO_HIDDEN_TIPS: &str = "No tip is hidden. Hide one with the X in its top right corner.";

/// The accessible name of the × at the top right.
pub const CLOSE_NAME: &str = "Close settings";
/// The accessible name of the search field.
pub const SEARCH_NAME: &str = "Search settings";

/// How long a row a search jumped to stays highlighted.
pub const HIGHLIGHT_FOR: Duration = Duration::from_millis(2500);

/// The pages of the window, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, PartialOrd, Ord)]
pub enum Page {
    #[default]
    General,
    Appearance,
    Layout,
    Cutting,
    Agents,
    Shortcuts,
    Plotters,
    Materials,
    Mats,
}

impl Page {
    pub const ALL: [Page; 9] = [
        Page::General,
        Page::Appearance,
        Page::Layout,
        Page::Cutting,
        Page::Agents,
        Page::Shortcuts,
        Page::Plotters,
        Page::Materials,
        Page::Mats,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Appearance => "Appearance",
            Page::Layout => "Layout",
            Page::Cutting => "Cutting",
            Page::Agents => "Agents",
            Page::Shortcuts => "Shortcuts",
            Page::Plotters => "Plotters",
            Page::Materials => "Materials",
            Page::Mats => "Mats",
        }
    }

    /// The group heading the page sits under in the sidebar.
    pub fn group(self) -> &'static str {
        match self {
            Page::General => "GENERAL",
            Page::Appearance | Page::Layout => "LOOK",
            Page::Cutting | Page::Agents => "WORK",
            Page::Shortcuts => "INPUT",
            Page::Plotters | Page::Materials | Page::Mats => "PROFILES",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Page::General => Icon::Settings,
            Page::Appearance => Icon::Palette,
            Page::Layout => Icon::Panels,
            Page::Cutting => Icon::Cut,
            Page::Agents => Icon::Bot,
            Page::Shortcuts => Icon::Keyboard,
            Page::Plotters => Icon::Plotter,
            Page::Materials => Icon::Layers,
            Page::Mats => Icon::Grid,
        }
    }

    /// One sentence on what the page is for, under its title.
    pub fn purpose(self) -> &'static str {
        match self {
            Page::General => {
                "Where FlatCraft writes its files and what it does when a file changes on disk."
            }
            Page::Appearance => {
                "How FlatCraft looks. You see a change at once, and Cancel takes it back."
            }
            Page::Layout => "Which side panels stay open and which actions sit on the ribbon.",
            Page::Cutting => "What new documents start with, and how arranging treats a selection.",
            Page::Agents => {
                "Let an AI agent drive FlatCraft over MCP, and decide how far it may go."
            }
            Page::Shortcuts => "Every keyboard shortcut FlatCraft knows. Fewer clicks, more cuts.",
            Page::Plotters => "How FlatCraft talks to each plotter and how it steers the blade.",
            Page::Materials => "Speed, force and passes for what you cut, with values per plotter.",
            Page::Mats => "The mats of each plotter and where their printed grid starts.",
        }
    }

    /// A preset page with its own Save buttons.
    pub fn is_profile(self) -> bool {
        matches!(self, Page::Plotters | Page::Materials | Page::Mats)
    }
}

/// The text of a row's hover card.
#[derive(Debug, Clone, Copy)]
pub enum Card {
    Text(&'static str),
    /// A card the ribbon or the panels show too.
    Setting(Setting),
}

impl Card {
    pub fn text(self) -> &'static str {
        match self {
            Card::Text(s) => s,
            Card::Setting(s) => s.card(),
        }
    }
}

/// One setting of the window.
#[derive(Debug, Clone, Copy)]
pub struct Row {
    pub id: &'static str,
    pub page: Page,
    pub section: &'static str,
    /// The label on the left. On a preset page, the label of the form row.
    pub label: &'static str,
    pub card: Card,
    /// More words search finds it by.
    pub words: &'static str,
    /// The `Preferences` field the row edits.
    pub field: Option<&'static str>,
}

const fn pref(
    id: &'static str,
    page: Page,
    section: &'static str,
    label: &'static str,
    card: Card,
    words: &'static str,
) -> Row {
    Row {
        id,
        page,
        section,
        label,
        card,
        words,
        field: Some(id),
    }
}

const fn other(
    id: &'static str,
    page: Page,
    section: &'static str,
    label: &'static str,
    card: Card,
    words: &'static str,
) -> Row {
    Row {
        id,
        page,
        section,
        label,
        card,
        words,
        field: None,
    }
}

/// Every setting the window shows: each preference (by field name), plus
/// the rows of the other pages search can jump to.
pub const ROWS: [Row; 26] = [
    pref(
        "hidden_tips",
        Page::General,
        "Tips",
        "Hidden tips",
        Card::Text(
            "Tips you hid with the X in their corner stay hidden for every material and document. Show hidden tips again brings them all back; Apply or OK stores it.",
        ),
        "hint tip hide hidden show again dismiss test cut",
    ),
    pref(
        "output_dir",
        Page::General,
        "Files",
        "File Output folder",
        Card::Text(
            "Where File Output writes its files when the plotter preset names no folder of its own. The default is Documents/FlatCraft Output.",
        ),
        "export save directory path location",
    ),
    pref(
        "auto_reload",
        Page::General,
        "Files",
        "Auto reload",
        Card::Text(
            "When an imported file changes on disk, FlatCraft reloads it and keeps your placement and cut settings. Off: FlatCraft lists the file as changed and you reload it yourself.",
        ),
        "reload watch linked changed disk",
    ),
    pref(
        "theme",
        Page::Appearance,
        "Theme",
        "Theme",
        Card::Text(
            "Dark, Light, or System, which follows your computer's appearance. You see the change at once, and Cancel takes it back.",
        ),
        "dark light mode colour color night appearance",
    ),
    pref(
        "mat_color_light",
        Page::Appearance,
        "Mat",
        "Mat colour, light theme",
        Card::Text(
            "The colour of the cutting mat on the canvas while the Light theme is on. It is the colour of every mat that has none of its own: a mat preset can still pick its own colour in Devices. White by default. Pick a tint that looks like your real mat. It only changes how the canvas looks, never the cut.",
        ),
        "mat colour color canvas white cream paper background swatch",
    ),
    pref(
        "mat_color_dark",
        Page::Appearance,
        "Mat",
        "Mat colour, dark theme",
        Card::Text(
            "The colour of the cutting mat on the canvas while the Dark theme is on. It is a light grey by default, a little dimmed so it does not glare in a dim workshop. Make it brighter for contrast or darker if your eyes tire. It only changes how the canvas looks, never the cut.",
        ),
        "mat colour color canvas grey gray dim glare swatch night",
    ),
    pref(
        "panel_objects",
        Page::Layout,
        "Side panels",
        "Objects panel",
        Card::Text(
            "The list of objects on the left of Prepare. Auto opens it when the window is wide enough and folds it to a slim rail when it is not. Open and Collapsed keep your choice at every width.",
        ),
        "left sidebar rail fold",
    ),
    pref(
        "panel_settings",
        Page::Layout,
        "Side panels",
        "Cut and Job panel",
        Card::Text(
            "The cut options and the job on the right of Prepare. Auto opens it when the window is wide enough and folds it to a slim rail when it is not. Open and Collapsed keep your choice at every width.",
        ),
        "right sidebar rail fold",
    ),
    pref(
        "panel_stage",
        Page::Layout,
        "Side panels",
        "Cut stage panel",
        Card::Text(
            "The stage panel of the Cut workspace. Auto opens it when the window is wide enough and folds it to a slim rail when it is not. Open and Collapsed keep your choice at every width.",
        ),
        "cut workspace sidebar rail fold",
    ),
    pref(
        "ribbon_pins",
        Page::Layout,
        "Ribbon",
        "Pinned actions",
        Card::Text(
            "Pinned actions show as buttons on the ribbon; the others wait in their group's list. Reset pins brings back the buttons of a new install.",
        ),
        "toolbar buttons pin unpin reset ribbon",
    ),
    pref(
        "default_plotter",
        Page::Cutting,
        "New documents",
        "Default device",
        Card::Text(
            "The plotter new documents start with. Pick a File Output preset to write files instead of cutting.",
        ),
        "plotter machine cameo start",
    ),
    pref(
        "default_material",
        Page::Cutting,
        "New documents",
        "Default material",
        Card::Text("The material new documents start with. It gives the speed, force and passes."),
        "paper vinyl card start",
    ),
    pref(
        "default_strategy",
        Page::Cutting,
        "New documents",
        "Cut strategy",
        Card::Text(
            "The order new documents cut in. Quality cleans up the corners, Fast keeps the travel short, and Outlines last cuts every outline after the holes and inner pieces. An open document keeps its own.",
        ),
        "order quality fast outlines last travel strategy",
    ),
    pref(
        "rotate_each",
        Page::Cutting,
        "Transform",
        "Each object",
        Card::Setting(Setting::EachObject),
        "rotate mirror scale own centre center",
    ),
    pref(
        "allow_agent_cuts",
        Page::Agents,
        "Agent cuts",
        "Agent cuts",
        Card::Text(
            "With this on, an agent can start a USB cut without asking you first. Turn it on only when the mat is loaded and you trust the agent. File Output never needs it.",
        ),
        "ai mcp usb allow permission ask confirm",
    ),
    other(
        "mcp",
        Page::Agents,
        "Connect an agent",
        "Connect an agent",
        Card::Text(
            "FlatCraft speaks MCP, the protocol AI agents use for tools. Your agent's client starts FlatCraft itself, so there is no server, no port and no token.",
        ),
        "mcp claude ai agent setup stdio server config",
    ),
    other(
        "shortcuts",
        Page::Shortcuts,
        "Keyboard",
        "Keyboard shortcuts",
        Card::Text(
            "The keys that save clicks, from Save to nudging the selection. On a Mac they use Cmd, elsewhere Ctrl.",
        ),
        "keys hotkeys keyboard cmd ctrl",
    ),
    other(
        "plotter.knife",
        Page::Plotters,
        "Preset",
        "Knife compensation",
        Card::Setting(Setting::KnifeCompensation),
        "corner blade fc firmware host",
    ),
    other(
        "plotter.blade_offset",
        Page::Plotters,
        "Preset",
        "Blade offset (mm)",
        Card::Setting(Setting::BladeOffset),
        "blade tip trail",
    ),
    other(
        "plotter.units",
        Page::Plotters,
        "Preset",
        "Units per mm",
        Card::Text(
            "How many plotter steps make one millimetre. The Cameo 2 uses 20. Change it only for another plotter model. For small size errors, use Scale x and Scale y.",
        ),
        "steps resolution scale size",
    ),
    other(
        "material.speed",
        Page::Materials,
        "Preset",
        "Speed",
        Card::Setting(Setting::Speed),
        "fast slow",
    ),
    other(
        "material.force",
        Page::Materials,
        "Preset",
        "Force",
        Card::Setting(Setting::Force),
        "pressure press depth",
    ),
    other(
        "material.passes",
        Page::Materials,
        "Preset",
        "Passes",
        Card::Setting(Setting::Passes),
        "repeat thick",
    ),
    other(
        "material.overcut",
        Page::Materials,
        "Preset",
        "Overcut (mm)",
        Card::Setting(Setting::Overcut),
        "close start",
    ),
    other(
        "mat.origin",
        Page::Mats,
        "Mats",
        "Grid origin x (mm)",
        Card::Text(
            "Where the printed grid starts on the mat, from its top left corner. FlatCraft's 0,0 is the grid's 0,0.",
        ),
        "mat grid origin offset printed",
    ),
    other(
        "mat.calibration",
        Page::Mats,
        "Calibration",
        "Calibration",
        Card::Text(
            "The calibration wizard and the mat assistant arrive in milestone 3. Until then a mat is used 5 mm inside its grid.",
        ),
        "calibrate wizard assistant",
    ),
];

/// The row with this id.
pub fn row(id: &str) -> &'static Row {
    ROWS.iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no settings row {id}"))
}

/// The rows that match every word of `query` in their label, section,
/// page, card or search words, best first: label hits before card hits.
pub fn search(query: &str) -> Vec<&'static Row> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(usize, &'static Row)> = ROWS
        .iter()
        .filter_map(|r| {
            let label = r.label.to_lowercase();
            let all = format!(
                "{label} {} {} {} {}",
                r.section.to_lowercase(),
                r.page.name().to_lowercase(),
                r.card.text().to_lowercase(),
                r.words
            );
            words.iter().all(|w| all.contains(w.as_str())).then(|| {
                let rank = words.iter().filter(|w| !label.contains(w.as_str())).count();
                (rank, r)
            })
        })
        .collect();
    hits.sort_by_key(|(rank, r)| (*rank, r.page));
    hits.into_iter().map(|(_, r)| r).collect()
}

/// Copies preference `field` from `from` into `to`. Unknown names do
/// nothing (a test checks every row's field is known).
pub fn copy_field(field: &str, from: &Preferences, to: &mut Preferences) {
    match field {
        "allow_agent_cuts" => to.allow_agent_cuts = from.allow_agent_cuts,
        "default_plotter" => to.default_plotter.clone_from(&from.default_plotter),
        "default_material" => to.default_material.clone_from(&from.default_material),
        "theme" => to.theme = from.theme,
        "output_dir" => to.output_dir.clone_from(&from.output_dir),
        "auto_reload" => to.auto_reload = from.auto_reload,
        "rotate_each" => to.rotate_each = from.rotate_each,
        "ribbon_pins" => to.ribbon_pins.clone_from(&from.ribbon_pins),
        "panel_objects" => to.panel_objects = from.panel_objects,
        "panel_settings" => to.panel_settings = from.panel_settings,
        "panel_stage" => to.panel_stage = from.panel_stage,
        "default_strategy" => to.default_strategy = from.default_strategy,
        "hidden_tips" => to.hidden_tips.clone_from(&from.hidden_tips),
        "mat_color_light" => to.mat_color_light = from.mat_color_light,
        "mat_color_dark" => to.mat_color_dark = from.mat_color_dark,
        _ => {}
    }
}

/// Whether two pin lists pin the same actions. Their order does not
/// count: the ribbon shows pins in table order.
pub fn same_pins(a: &[String], b: &[String]) -> bool {
    let sorted = |v: &[String]| {
        let mut v = v.to_vec();
        v.sort();
        v.dedup();
        v
    };
    sorted(a) == sorted(b)
}

/// Whether `a` and `b` differ in preference `field`.
fn differs(field: &str, a: &Preferences, b: &Preferences) -> bool {
    if field == "ribbon_pins" {
        return !same_pins(&a.ribbon_pins, &b.ribbon_pins);
    }
    let mut x = b.clone();
    copy_field(field, a, &mut x);
    x != *b
}

fn fields() -> impl Iterator<Item = &'static str> {
    ROWS.iter().filter_map(|r| r.field)
}

/// Writes into `into` the fields `draft` changed against `base`, and only
/// those, so a preference stored meanwhile by someone else is kept.
pub fn merge(draft: &Preferences, base: &Preferences, into: &mut Preferences) {
    for f in fields() {
        if differs(f, draft, base) {
            copy_field(f, draft, into);
        }
    }
}

/// Whether any preference of `page` differs from the defaults.
fn page_differs(page: Page, d: &Preferences) -> bool {
    let def = Preferences::default();
    ROWS.iter()
        .filter(|r| r.page == page)
        .filter_map(|r| r.field)
        .any(|f| differs(f, &def, d))
}

/// Puts the preferences of `page` back to their defaults in `d`.
pub fn restore_page(page: Page, d: &mut Preferences) {
    let def = Preferences::default();
    for f in ROWS
        .iter()
        .filter(|r| r.page == page)
        .filter_map(|r| r.field)
    {
        copy_field(f, &def, d);
    }
}

/// The name of a cut strategy, as the window and the panel show it.
pub fn strategy_name(s: CutStrategy) -> &'static str {
    match s {
        CutStrategy::Quality => "Quality",
        CutStrategy::Fast => "Fast",
        CutStrategy::OutlinesLast => "Outlines last",
    }
}

/// What the window shows besides the session.
#[derive(Debug, Default)]
pub struct SettingsState {
    pub page: Page,
    /// The staged preferences: the edits wait here for Apply or OK.
    pub draft: Option<Preferences>,
    /// The preferences when the window opened or the last Apply ran.
    pub base: Option<Preferences>,
    pub search: String,
    /// The row a search jumped to, and when.
    pub highlight: Option<(&'static str, Instant)>,
    /// The row to scroll into view on the next frame.
    scroll_to: Option<&'static str>,
    /// Sections the person folded, as "page/section".
    collapsed: BTreeSet<String>,
    /// The staged theme, shown while the window is open.
    pub preview_theme: Option<Theme>,
    /// Store the staged preferences once "Discard your changes?" lets the
    /// window close.
    close_apply: bool,
    focus_search: bool,
}

impl SettingsState {
    /// Whether staged preferences wait for Apply or OK.
    pub fn dirty(&self) -> bool {
        match (&self.draft, &self.base) {
            (Some(d), Some(b)) => fields().any(|f| differs(f, d, b)),
            _ => false,
        }
    }

    /// Opens the page of `row`, unfolds its section and highlights it.
    pub fn jump(&mut self, row: &'static Row, now: Instant) {
        self.page = row.page;
        self.collapsed
            .remove(&format!("{}/{}", row.page.name(), row.section));
        self.highlight = Some((row.id, now));
        self.scroll_to = Some(row.id);
        self.search.clear();
    }

    fn reset(&mut self) {
        let page = self.page;
        let collapsed = std::mem::take(&mut self.collapsed);
        *self = Self {
            page,
            collapsed,
            ..Self::default()
        };
    }
}

/// What the footer asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Footer {
    Restore,
    Cancel,
    Apply,
    Ok,
}

/// The size of the window on a screen of `screen`: room for the pages,
/// never more than the screen minus a margin.
pub fn window_size(screen: egui::Vec2) -> egui::Vec2 {
    SIZE.min(screen - vec2(2.0 * MARGIN, 2.0 * MARGIN))
}

/// The window's size on a large screen (a fixed size, see
/// docs/design/ui-rules.md).
const SIZE: Vec2 = vec2(1040.0, 760.0);
/// Room between the window and the screen's edge.
const MARGIN: f32 = metrics::SPACE_16;
const HEADER: f32 = metrics::WINDOW_BAR;
const FOOTER: f32 = metrics::WINDOW_BAR;
/// The page list (a fixed size, see docs/design/ui-rules.md).
const SIDEBAR: f32 = 196.0;
const PAD: f32 = metrics::WINDOW_PAD;
/// The label column of a page (a fixed size, see docs/design/ui-rules.md).
const LABEL_W: f32 = 200.0;

/// Draws the window when it is open.
pub fn show(ctx: &egui::Context, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let view = profiles_window::load(app);
    if app.ui.settings.draft.is_none() {
        let p = lock(&app.session).prefs.clone();
        app.ui.settings.base = Some(p.clone());
        app.ui.settings.draft = Some(p);
    }
    let t = app.tokens;
    let now = (app.clock)();
    // Keys this window reads before its widgets take them.
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
        app.ui.settings.focus_search = true;
    }
    // Esc in a number field only puts its value back (egui has already
    // dropped the focus, so ask whether one was being typed into).
    let idle = ctx.memory(|m| m.focused().is_none())
        && !egui::Popup::is_any_open(ctx)
        && calc::editing(ctx).is_none();
    let st = &mut app.ui.settings;
    let active = st
        .highlight
        .filter(|(_, at)| now.saturating_duration_since(*at) < HIGHLIGHT_FOR);
    if let Some((_, at)) = active {
        ctx.request_repaint_after(HIGHLIGHT_FOR.saturating_sub(now.saturating_duration_since(at)));
    }
    let lit = active.map(|(id, _)| id);
    // The preset forms highlight by their row label.
    let ps = &mut app.ui.profiles;
    let label = lit
        .map(row)
        .filter(|r| r.page == st.page && r.page.is_profile())
        .map(|r| r.label);
    if ps.highlight.label != label {
        ps.highlight.label = label;
        ps.highlight.scroll.set(label.is_some());
    }

    let mut draft = st.draft.take().unwrap_or_default();
    let base = st.base.clone().unwrap_or_default();
    let dirty = fields().any(|f| differs(f, &draft, &base));
    let size = window_size(ctx.content_rect().size());
    let mut footer: Option<Footer> = None;
    let mut close = false;
    let frame = egui::Frame::new()
        .fill(t.panel)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(t.ribbon_radius * 3)
        .inner_margin(0);
    let modal = egui::Modal::new(egui::Id::unique("flatcraft-settings"))
        .frame(frame)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let header = Rect::from_min_size(rect.min, vec2(size.x, HEADER));
            let foot = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOOTER), rect.max);
            let side = Rect::from_min_max(
                pos2(rect.left(), header.bottom()),
                pos2(rect.left() + SIDEBAR, foot.top()),
            );
            let content = Rect::from_min_max(
                pos2(side.right(), header.bottom()),
                pos2(rect.right(), foot.top()),
            );
            paint_frame(ui, rect, header, side, foot, &t);
            let (x, enter) = header_ui(ui, header, st, &t);
            close |= x;
            if let Some(first) = enter {
                st.jump(first, now);
            }
            sidebar(ui, side, st, &t);
            let body = content.shrink2(vec2(PAD, metrics::SPACE_12));
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(body)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.set_clip_rect(content);
                    if ps.leave.is_some() {
                        profiles_window::discard_banner(ui, ps, &t);
                    }
                    if st.search.trim().is_empty() {
                        page_ui(ui, st, ps, &mut draft, &view, lit, &t, actions);
                    } else {
                        results(ui, st, now, &t);
                    }
                },
            );
            footer = footer_ui(ui, foot, st.page, &draft, dirty, &t);
        });
    // The staged theme shows while the window is open.
    st.preview_theme = (draft.theme != base.theme).then_some(draft.theme);
    st.draft = Some(draft);
    profiles_window::dialogs(ctx, ps, &view, &t, actions);

    if idle && modal.is_top_modal && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        if app.ui.profiles.leave.is_some() {
            // Escape on "Discard your changes?" means Keep editing.
            app.ui.profiles.leave = None;
        } else {
            close = true;
        }
    }
    if let Some(n) = profiles_window::take_notice(&mut app.ui.profiles) {
        app.toast(Toast::warning(n));
    }
    match footer {
        Some(Footer::Restore) => {
            let page = app.ui.settings.page;
            if let Some(d) = app.ui.settings.draft.as_mut() {
                restore_page(page, d);
            }
        }
        Some(Footer::Apply) => apply_staged(app, ctx),
        Some(Footer::Ok) => request_close(app, ctx, true),
        Some(Footer::Cancel) => close = true,
        None => {}
    }
    if close {
        request_close(app, ctx, false);
    }
    // "Discard" answered the question a close asked.
    if app.ui.profiles.closing {
        let apply = app.ui.settings.close_apply;
        finish(app, ctx, apply);
    }
}

/// Closes the window, storing the staged preferences when `apply`. With
/// unsaved preset edits it asks "Discard your changes?" first.
fn request_close(app: &mut FlatCraftApp, ctx: &egui::Context, apply: bool) {
    if profiles_window::unsaved(&app.ui.profiles) {
        app.ui.profiles.leave = Some(Leave::Close);
        // The staged preferences wait for the answer: Keep editing keeps
        // them, Discard closes (and stores them after OK).
        app.ui.settings.close_apply = apply;
    } else {
        finish(app, ctx, apply);
    }
}

fn finish(app: &mut FlatCraftApp, ctx: &egui::Context, apply: bool) {
    if apply {
        apply_staged(app, ctx);
    }
    app.ui.settings.reset();
    profiles_window::closed(&mut app.ui.profiles);
    app.ui.show_settings = false;
}

/// Stores the staged preferences: only the fields the draft changed, read
/// again from the file first. The window state that mirrors preferences
/// (Each object, the side panels, the theme) follows at once.
pub fn apply_staged(app: &mut FlatCraftApp, ctx: &egui::Context) {
    let st = &mut app.ui.settings;
    let (Some(draft), Some(base)) = (st.draft.clone(), st.base.clone()) else {
        return;
    };
    if !st.dirty() {
        return;
    }
    let (result, fresh) = {
        let mut s = lock(&app.session);
        let result = s.update_prefs(|p| merge(&draft, &base, p));
        (result, s.prefs.clone())
    };
    st.draft = Some(fresh.clone());
    st.base = Some(fresh.clone());
    st.preview_theme = None;
    app.ui.prepare.fields.each = fresh.rotate_each;
    app.ui.prepare.panels.adopt(&fresh);
    super::inspector::blocks::seed_tips(ctx, fresh.hidden_tips.clone());
    app.tokens = Tokens::for_theme(fresh.theme, ctx.system_theme());
    app.follows_system = fresh.theme == Theme::System;
    theme::apply(ctx, &app.tokens, app.follows_system);
    if let Err(e) = result {
        app.toast(Toast::error(format!(
            "Cannot store the preferences: {e}. The change holds until FlatCraft closes."
        )));
    }
}

// ---------------------------------------------------------------- frame

fn paint_frame(ui: &Ui, rect: Rect, header: Rect, side: Rect, foot: Rect, t: &Tokens) {
    let p = ui.painter();
    let r = t.ribbon_radius * 3;
    p.rect_filled(side, 0.0, t.rail);
    p.rect_filled(
        foot,
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: r,
            se: r,
        },
        t.rail,
    );
    let line = egui::Stroke::new(1.0, t.border);
    p.hline(rect.x_range(), header.bottom(), line);
    p.hline(rect.x_range(), foot.top(), line);
    p.vline(side.right(), side.y_range(), line);
}

fn icon_image(ui: &Ui, icon: Icon, size: f32, tint: Color32) -> egui::Image<'static> {
    let tex = icons::texture_colored(ui.ctx(), icon, size, tint);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(size),
    ))
}

/// Text painted without an accessibility node.
fn paint(
    ui: &Ui,
    at: egui::Pos2,
    align: egui::Align2,
    text: &str,
    font: egui::FontId,
    color: Color32,
) -> Rect {
    ui.painter().text(at, align, text, font, color)
}

/// Title, search and ×. Returns whether the × was clicked, and the best
/// search result when Enter was pressed in the search field.
fn header_ui(
    ui: &mut Ui,
    rect: Rect,
    st: &mut SettingsState,
    t: &Tokens,
) -> (bool, Option<&'static Row>) {
    let icon = Rect::from_min_size(
        pos2(
            rect.left() + PAD,
            rect.center().y - metrics::ICON_COMPACT / 2.0,
        ),
        egui::Vec2::splat(metrics::ICON_COMPACT),
    );
    icon_image(ui, Icon::Settings, metrics::ICON_COMPACT, t.icon_accent).paint_at(ui, icon);
    paint(
        ui,
        pos2(icon.right() + metrics::SPACE_8, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Settings",
        metrics::title(),
        t.text,
    );
    let mut clicked = false;
    let mut enter = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(metrics::CLOSE_INSET, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
        |ui| {
            let x = ui.add(egui::Button::image(icon_image(ui, Icon::Close, metrics::ICON_COMPACT, t.text_dim).alt_text(CLOSE_NAME)).frame_when_inactive(false));
            set_name(ui, &x, CLOSE_NAME);
            super::groups::card(&x, "Close", "Close Settings and drop the changes you did not apply.", None, false);
            clicked = x.clicked();
            ui.add_space(metrics::SPACE_8);
            let key = if cfg!(target_os = "macos") { "Cmd+F" } else { "Ctrl+F" };
            ui.label(RichText::new(key).color(t.text_dim).font(metrics::small()));
            let r = ui.add(
                egui::TextEdit::singleline(&mut st.search)
                    .id_salt("flatcraft-settings-search")
                    .hint_text(SEARCH_NAME)
                    .desired_width(220.0),
            );
            set_name(ui, &r, SEARCH_NAME);
            super::groups::card(
                &r,
                "Search settings",
                "Type a word, like theme or blade, then pick a result or press Enter. Search looks at the names and at the hover cards.",
                None,
                false,
            );
            if std::mem::take(&mut st.focus_search) {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                enter = search(&st.search).first().copied();
            }
        },
    );
    (clicked, enter)
}

/// The pages under their group headings.
fn sidebar(ui: &mut Ui, rect: Rect, st: &mut SettingsState, t: &Tokens) {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(metrics::SPACE_8, metrics::SPACE_12)))
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.y = metrics::SPACE_2;
            let mut group = "";
            for page in Page::ALL {
                if page.group() != group {
                    group = page.group();
                    if page != Page::General {
                        ui.add_space(metrics::SPACE_8);
                    }
                    let (r, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), metrics::SPACE_16),
                        Sense::hover(),
                    );
                    paint(
                        ui,
                        r.left_center() + vec2(metrics::SPACE_8, 0.0),
                        egui::Align2::LEFT_CENTER,
                        group,
                        metrics::caption(),
                        t.text_dim,
                    );
                }
                let on = st.page == page;
                let (r, resp) = ui
                    .allocate_exact_size(vec2(ui.available_width(), metrics::ROW), Sense::click());
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(egui::accesskit::Role::Tab, true, on, page.name())
                });
                let fill = if on {
                    t.accent_prepare.gamma_multiply(0.2)
                } else if resp.hovered() {
                    t.surface
                } else {
                    Color32::TRANSPARENT
                };
                ui.painter().rect_filled(r, t.ribbon_radius, fill);
                if on {
                    ui.painter().rect_filled(
                        Rect::from_min_size(r.min, vec2(2.0, r.height())),
                        0.0,
                        t.accent_prepare,
                    );
                }
                let color = if on { t.text } else { t.text_dim };
                let icon = Rect::from_center_size(
                    pos2(
                        r.left() + metrics::SPACE_8 + metrics::ICON_COMPACT / 2.0,
                        r.center().y,
                    ),
                    egui::Vec2::splat(metrics::ICON_COMPACT),
                );
                let tint = if on { t.icon_accent } else { t.text_dim };
                icon_image(ui, page.icon(), metrics::ICON_COMPACT, tint).paint_at(ui, icon);
                paint(
                    ui,
                    pos2(icon.right() + metrics::SPACE_8, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    page.name(),
                    if on {
                        metrics::section_title()
                    } else {
                        fonts::medium(metrics::TEXT_SECTION)
                    },
                    color,
                );
                super::groups::card(&resp, page.name(), page.purpose(), None, true);
                if resp.clicked() {
                    st.page = page;
                    st.search.clear();
                }
            }
        },
    );
}

/// Restore page defaults on the left; Cancel, Apply and OK on the right.
fn footer_ui(
    ui: &mut Ui,
    rect: Rect,
    page: Page,
    draft: &Preferences,
    dirty: bool,
    t: &Tokens,
) -> Option<Footer> {
    let mut out = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(PAD, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let can_restore = !page.is_profile() && page != Page::Shortcuts && page_differs(page, draft);
            let why = if page.is_profile() {
                "Presets have no page defaults. Revert drops your edits to a preset, and a system preset always keeps its values."
            } else if page == Page::Shortcuts {
                "Shortcuts cannot be changed yet, so there is nothing to restore."
            } else {
                "This page already shows the defaults."
            };
            let r = ui.add_enabled(can_restore, egui::Button::new("Restore page defaults"));
            let card = if page == Page::General {
                RESTORE_GENERAL
            } else {
                RESTORE
            };
            super::groups::card(
                &r,
                "Restore page defaults",
                card,
                (!can_restore).then_some(why),
                false,
            );
            if r.clicked() {
                out = Some(Footer::Restore);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (fill, ink) = if t.is_dark() {
                    (t.accent_prepare, t.bg)
                } else {
                    (t.icon_accent, Color32::WHITE)
                };
                let ok = ui.add(
                    egui::Button::new(RichText::new("OK").color(ink).strong())
                        .fill(fill)
                        .min_size(vec2(metrics::BUTTON_MIN_W, 0.0)),
                );
                super::groups::card(&ok, "OK", "Store your changes and close Settings.", None, false);
                if calc::click(ui, &ok) {
                    out = Some(Footer::Ok);
                }
                let apply = ui.add_enabled(dirty, egui::Button::new("Apply"));
                let apply_why = "Nothing to store yet. Change a setting first.";
                super::groups::card(
                    &apply,
                    "Apply",
                    "Store your changes and keep Settings open. Presets have their own Save preset button.",
                    (!dirty).then_some(apply_why),
                    false,
                );
                if calc::click(ui, &apply) {
                    out = Some(Footer::Apply);
                }
                let cancel = ui.button("Cancel");
                super::groups::card(&cancel, "Cancel", "Close Settings and drop the changes you did not apply.", None, false);
                if cancel.clicked() {
                    out = Some(Footer::Cancel);
                }
            });
        },
    );
    out
}

// ---------------------------------------------------------------- pages

/// What a preference page draws with.
struct Cx<'a> {
    t: &'a Tokens,
    page: Page,
    lit: Option<&'static str>,
    scroll_to: &'a mut Option<&'static str>,
    collapsed: &'a mut BTreeSet<String>,
}

#[allow(clippy::too_many_arguments)]
fn page_ui(
    ui: &mut Ui,
    st: &mut SettingsState,
    ps: &mut profiles_window::ProfilesState,
    draft: &mut Preferences,
    view: &profiles_window::View,
    lit: Option<&'static str>,
    t: &Tokens,
    actions: &mut Vec<Action>,
) {
    let page = st.page;
    // The title is painted, so the sidebar tab keeps the only node with
    // the page's name.
    let (r, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), metrics::CONTROL), Sense::hover());
    paint(
        ui,
        r.left_center(),
        egui::Align2::LEFT_CENTER,
        page.name(),
        metrics::title(),
        t.text,
    );
    ui.add(egui::Label::new(RichText::new(page.purpose()).color(t.text_dim)).wrap());
    ui.add_space(metrics::SPACE_8);
    if page.is_profile() {
        profiles_window::diagnostics(ui, view, t, actions);
        match page {
            Page::Plotters => profiles_window::plotters_page(ui, ps, view, t, actions),
            Page::Materials => profiles_window::materials_page(ui, ps, view, t, actions),
            _ => profiles_window::mats_page(ui, ps, view, t, actions),
        }
        return;
    }
    let mut cx = Cx {
        t,
        page,
        lit,
        scroll_to: &mut st.scroll_to,
        collapsed: &mut st.collapsed,
    };
    egui::ScrollArea::vertical()
        .id_salt(("flatcraft-settings-page", page))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Rows carry their own room: a control is CONTROL tall and
            // two rows are ROW apart, as in the side panels.
            super::inspector::blocks::panel_spacing(ui);
            match page {
                Page::General => general(ui, &mut cx, draft, actions),
                Page::Appearance => appearance(ui, &mut cx, draft),
                Page::Layout => layout(ui, &mut cx, draft),
                Page::Cutting => cutting(ui, &mut cx, draft, view),
                Page::Agents => agents(ui, &mut cx, draft),
                _ => shortcuts(ui, &mut cx),
            }
        });
}

/// A section: an uppercase heading with a rule, folded with its chevron.
fn section(ui: &mut Ui, cx: &mut Cx<'_>, title: &str, add: impl FnOnce(&mut Ui, &mut Cx<'_>)) {
    let key = format!("{}/{title}", cx.page.name());
    let open = !cx.collapsed.contains(&key);
    let t = cx.t;
    let (rect, resp) = ui.allocate_exact_size(
        vec2(ui.available_width(), metrics::SECTION_HEADER),
        Sense::click(),
    );
    let name = format!("{title} section");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    ui.ctx()
        .accesskit_node_builder(resp.id, |n| n.set_expanded(open));
    let chevron = Rect::from_min_size(
        pos2(rect.left(), rect.center().y - metrics::ICON_SMALL / 2.0),
        egui::Vec2::splat(metrics::ICON_SMALL),
    );
    let icon = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let ink = if resp.hovered() { t.text } else { t.text_dim };
    icon_image(ui, icon, metrics::ICON_SMALL, ink).paint_at(ui, chevron);
    let text = paint(
        ui,
        pos2(chevron.right() + metrics::SPACE_4, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &title.to_uppercase(),
        metrics::caption(),
        ink,
    );
    ui.painter().hline(
        (text.right() + metrics::SPACE_8)..=rect.right(),
        rect.center().y,
        egui::Stroke::new(1.0, t.border),
    );
    let what = if open {
        "Click to fold this section away."
    } else {
        "Click to show this section again."
    };
    super::groups::card(&resp, title, what, None, false);
    if resp.clicked() {
        if open {
            cx.collapsed.insert(key);
        } else {
            cx.collapsed.remove(&key);
        }
    }
    if open {
        add(ui, cx);
    }
    ui.add_space(metrics::SPACE_8);
}

/// Paints the outline and tint of a row a search jumped to.
pub(crate) fn paint_highlight(ui: &Ui, rect: Rect, t: &Tokens) {
    let r = rect.expand(4.0);
    ui.painter().rect(
        r,
        t.ribbon_radius,
        t.accent_prepare.gamma_multiply(0.12),
        egui::Stroke::new(1.5, t.accent_prepare),
        egui::StrokeKind::Outside,
    );
}

/// A row: the label with its ⓘ on the left, the control on the right. The
/// control's response gets the row's card.
fn row_ui(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    id: &'static str,
    control: impl FnOnce(&mut Ui) -> Option<Response>,
) {
    let r = row(id);
    let t = cx.t;
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = ui.horizontal_top(|ui| {
        let (lr, _) = ui.allocate_exact_size(vec2(LABEL_W, metrics::CONTROL), Sense::hover());
        let text = paint(
            ui,
            lr.left_center(),
            egui::Align2::LEFT_CENTER,
            r.label,
            egui::TextStyle::Body.resolve(ui.style()),
            t.text,
        );
        info(
            ui,
            Rect::from_center_size(
                pos2(
                    text.right() + metrics::SPACE_4 + metrics::ICON_SMALL / 2.0,
                    lr.center().y,
                ),
                egui::Vec2::splat(metrics::ICON_SMALL),
            ),
            r,
            t,
        );
        ui.vertical(|ui| {
            ui.set_max_width(ui.available_width().min(520.0));
            // Lines inside one control (a choice and its explanation).
            ui.spacing_mut().item_spacing.y = metrics::SPACE_2;
            control(ui)
        })
        .inner
    });
    if let Some(resp) = inner.inner {
        // A number field says once that it takes a calculation.
        let text = match r.card {
            Card::Setting(s) => match s.number_kind() {
                Some(kind) => calc::with_hint(s.card(), kind),
                None => s.card().to_string(),
            },
            Card::Text(t) => t.to_string(),
        };
        super::groups::card(&resp, r.label, &text, None, false);
    }
    let rect = inner.response.rect;
    if cx.lit == Some(id) {
        let rr = rect.expand(4.0);
        ui.painter().set(
            bg,
            egui::Shape::rect_filled(rr, t.ribbon_radius, t.accent_prepare.gamma_multiply(0.12)),
        );
        ui.painter().rect_stroke(
            rr,
            t.ribbon_radius,
            egui::Stroke::new(1.5, t.accent_prepare),
            egui::StrokeKind::Outside,
        );
    }
    if *cx.scroll_to == Some(id) {
        ui.scroll_to_rect(rect, Some(Align::Center));
        *cx.scroll_to = None;
    }
    ui.add_space(metrics::ROW_GAP);
}

/// A note on a whole section, under its rows: a little room above it, so
/// it does not read as part of the last row.
fn page_note(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add_space(metrics::SPACE_4);
    ui.add(egui::Label::new(RichText::new(text).color(t.text_dim)).wrap());
}

/// The ⓘ beside a label: hovering it shows the row's card.
fn info(ui: &mut Ui, rect: Rect, r: &Row, t: &Tokens) {
    let resp = ui.interact(
        rect,
        egui::Id::unique(("flatcraft-settings-info", r.id)),
        Sense::hover(),
    );
    let name = format!("About {}", r.label);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &name));
    let tint = if resp.hovered() {
        t.icon_accent
    } else {
        t.text_dim
    };
    icon_image(ui, Icon::Info, metrics::ICON_SMALL, tint).paint_at(ui, rect);
    super::groups::card(&resp, r.label, r.card.text(), None, true);
}

/// Segmented buttons for a few choices, with the picked one's explanation
/// under them. Each segment is a radio button named "<name>: <option>"
/// and carries the row's card.
fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    t: &Tokens,
    row: &Row,
    value: &mut T,
    options: &[(T, &str, &str)],
    explain: bool,
) {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label, _)| {
            ui.painter()
                .layout_no_wrap(label.to_string(), font.clone(), t.text)
                .size()
                .x
                + metrics::SPACE_24
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(total, metrics::CONTROL), Sense::hover());
    let mut x = rect.left();
    for (i, ((v, label, _), w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(*w, rect.height()));
        x += w;
        let resp = ui.interact(
            r,
            egui::Id::unique(("flatcraft-segment", row.id, i)),
            Sense::click(),
        );
        let on = *value == *v;
        let name = format!("{}: {label}", row.label);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, &name));
        let fill = if on {
            t.accent_prepare.gamma_multiply(0.25)
        } else if resp.hovered() {
            t.surface
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(r.shrink(1.0), t.ribbon_radius, fill);
        if i > 0 {
            ui.painter()
                .vline(r.left(), r.y_range(), egui::Stroke::new(1.0, t.border));
        }
        let ink = if on { t.text } else { t.text_dim };
        let f = if on {
            fonts::semibold(font.size)
        } else {
            font.clone()
        };
        paint(ui, r.center(), egui::Align2::CENTER_CENTER, label, f, ink);
        super::groups::card(&resp, row.label, row.card.text(), None, false);
        if resp.clicked() {
            *value = *v;
        }
    }
    ui.painter().rect_stroke(
        rect,
        t.ribbon_radius,
        egui::Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    if !explain {
        return;
    }
    if let Some((_, _, why)) = options.iter().find(|(v, _, _)| *v == *value) {
        ui.add(egui::Label::new(RichText::new(*why).color(t.text_dim)).wrap());
    }
}

fn general(ui: &mut Ui, cx: &mut Cx<'_>, d: &mut Preferences, actions: &mut Vec<Action>) {
    section(ui, cx, "Tips", |ui, cx| {
        let t = *cx.t;
        row_ui(ui, cx, "hidden_tips", |ui| {
            // Only tips this version has count: an id of a tip that is
            // gone hides nothing.
            let n = super::inspector::KNOWN_TIPS
                .iter()
                .filter(|id| d.hidden_tips.contains(**id))
                .count();
            let note = match n {
                0 => "No tips are hidden.".to_string(),
                1 => "1 tip is hidden.".to_string(),
                n => format!("{n} tips are hidden."),
            };
            ui.label(RichText::new(note).color(t.text_dim));
            let r = ui.add_enabled(n > 0, egui::Button::new(SHOW_TIPS));
            super::groups::card(
                &r,
                SHOW_TIPS,
                "Shows every tip you hid again, like Start with a test cut. Apply or OK stores it.",
                (n == 0).then_some(NO_HIDDEN_TIPS),
                false,
            );
            if r.clicked() {
                d.hidden_tips.clear();
            }
            None
        });
    });
    section(ui, cx, "Files", |ui, cx| {
        let t = cx.t;
        row_ui(ui, cx, "output_dir", |ui| {
            let shown = d.output_dir.as_ref().map_or_else(
                || "Documents/FlatCraft Output (the default)".to_string(),
                |p| p.display().to_string(),
            );
            ui.add(egui::Label::new(RichText::new(shown).monospace()).wrap());
            ui.horizontal(|ui| {
                let choose = ui.button("Choose folder...");
                if choose.clicked() {
                    actions.push(Action::Profiles(Request::ChooseOutputFolder));
                }
                super::groups::card(
                    &choose,
                    "Choose folder",
                    "Pick the folder File Output writes to. Apply or OK stores it.",
                    None,
                    false,
                );
                let default =
                    ui.add_enabled(d.output_dir.is_some(), egui::Button::new("Use the default"));
                super::groups::card(
                    &default,
                    "Use the default",
                    "Write to Documents/FlatCraft Output again.",
                    d.output_dir
                        .is_none()
                        .then_some("FlatCraft already uses the default folder."),
                    false,
                );
                if default.clicked() {
                    d.output_dir = None;
                }
                Some(choose)
            })
            .inner
        });
        row_ui(ui, cx, "auto_reload", |ui| {
            let r = ui.checkbox(&mut d.auto_reload, "Reload changed files automatically");
            ui.add(
                egui::Label::new(
                    RichText::new(profiles_window::AUTO_RELOAD_HELP).color(t.text_dim),
                )
                .wrap(),
            );
            Some(r)
        });
    });
}

fn appearance(ui: &mut Ui, cx: &mut Cx<'_>, d: &mut Preferences) {
    section(ui, cx, "Theme", |ui, cx| {
        let t = *cx.t;
        row_ui(ui, cx, "theme", |ui| {
            segmented(
                ui,
                &t,
                row("theme"),
                &mut d.theme,
                &[
                    (
                        Theme::Dark,
                        "Dark",
                        "Dark panels, easy on the eyes in a dim workshop.",
                    ),
                    (Theme::Light, "Light", "Light panels for a bright room."),
                    (Theme::System, "System", profiles_window::THEME_HELP),
                ],
                true,
            );
            None
        });
    });
    section(ui, cx, "Mat", |ui, cx| {
        let t = *cx.t;
        for (id, name, colour, default) in [
            (
                "mat_color_light",
                "Mat colour, light theme",
                &mut d.mat_color_light,
                flatcraft_device::DEFAULT_MAT_COLOR_LIGHT,
            ),
            (
                "mat_color_dark",
                "Mat colour, dark theme",
                &mut d.mat_color_dark,
                flatcraft_device::DEFAULT_MAT_COLOR_DARK,
            ),
        ] {
            row_ui(ui, cx, id, |ui| {
                ui.horizontal(|ui| {
                    let r = ui.color_edit_button_srgb(colour);
                    set_name(ui, &r, name);
                    ui.label(RichText::new(profiles_window::hex(*colour)).color(t.text_dim));
                    if *colour != default {
                        ui.label(RichText::new("(changed)").color(t.text_dim));
                    }
                    Some(r)
                })
                .inner
            });
        }
        page_note(
            ui,
            &t,
            "A mat preset can still have its own colour. Pick Custom on its Mat colour row under Profiles, Mats.",
        );
    });
}

/// What the side panel choices do, once under the panel rows.
pub const PANELS_HELP: &str = "Auto opens a panel when the window is wide enough and folds it to a slim rail when it is not. Open and Collapsed keep your choice at every width; click a rail to open it for a moment.";

const PANEL_CHOICES: [(PanelUser, &str, &str); 3] = [
    (
        PanelUser::None,
        "Auto",
        "Opens when the window is wide enough and folds to a rail when it is not.",
    ),
    (PanelUser::Open, "Open", "Stays open at every window width."),
    (
        PanelUser::Collapsed,
        "Collapsed",
        "Stays folded to a slim rail. Click the rail to open it for a moment.",
    ),
];

fn layout(ui: &mut Ui, cx: &mut Cx<'_>, d: &mut Preferences) {
    section(ui, cx, "Side panels", |ui, cx| {
        let t = *cx.t;
        for (id, v) in [
            ("panel_objects", &mut d.panel_objects),
            ("panel_settings", &mut d.panel_settings),
            ("panel_stage", &mut d.panel_stage),
        ] {
            row_ui(ui, cx, id, |ui| {
                segmented(ui, &t, row(id), v, &PANEL_CHOICES, false);
                None
            });
        }
        // One explanation for the three rows.
        page_note(ui, &t, PANELS_HELP);
    });
    section(ui, cx, "Ribbon", |ui, cx| {
        let t = *cx.t;
        row_ui(ui, cx, "ribbon_pins", |ui| {
            let n = d.ribbon_pins.len();
            let all = flatcraft_engine::prefs::RIBBON_PIN_IDS.len();
            ui.label(format!("{n} of {all} actions are pinned to the ribbon."));
            let defaults = default_ribbon_pins();
            let differs = !same_pins(&d.ribbon_pins, &defaults);
            let r = ui.add_enabled(differs, egui::Button::new("Reset pins"));
            super::groups::card(
                &r,
                "Reset pins",
                "Pins the buttons a new install shows and unpins the rest.",
                (!differs).then_some("The ribbon already shows the buttons of a new install."),
                false,
            );
            if r.clicked() {
                d.ribbon_pins = defaults;
            }
            ui.add(
                egui::Label::new(
                    RichText::new("Pin or unpin one action from its group's list on the ribbon.")
                        .color(t.text_dim),
                )
                .wrap(),
            );
            None
        });
    });
}

fn combo(ui: &mut Ui, name: &str, value: &mut String, choices: &[(String, String)]) -> Response {
    let text = choices
        .iter()
        .find(|(id, _)| id == value)
        .map_or(value.as_str(), |(_, n)| n.as_str())
        .to_string();
    let r = egui::ComboBox::from_id_salt(("flatcraft-settings", name))
        .selected_text(text)
        .width(240.0)
        .show_ui(ui, |ui| {
            for (id, n) in choices {
                ui.selectable_value(value, id.clone(), n);
            }
        })
        .response;
    set_name(ui, &r, name);
    r
}

fn cutting(ui: &mut Ui, cx: &mut Cx<'_>, d: &mut Preferences, view: &profiles_window::View) {
    section(ui, cx, "New documents", |ui, cx| {
        let t = *cx.t;
        let plotters: Vec<(String, String)> = view
            .plotters
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        row_ui(ui, cx, "default_plotter", |ui| {
            Some(combo(
                ui,
                "Default device",
                &mut d.default_plotter,
                &plotters,
            ))
        });
        let materials: Vec<(String, String)> = view
            .materials
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        row_ui(ui, cx, "default_material", |ui| {
            Some(combo(
                ui,
                "Default material",
                &mut d.default_material,
                &materials,
            ))
        });
        row_ui(ui, cx, "default_strategy", |ui| {
            segmented(
                ui,
                &t,
                row("default_strategy"),
                &mut d.default_strategy,
                &[
                    (
                        CutStrategy::Quality,
                        strategy_name(CutStrategy::Quality),
                        "Clean corners first: few blade turns, and each cut starts on the waste side.",
                    ),
                    (
                        CutStrategy::Fast,
                        strategy_name(CutStrategy::Fast),
                        "Short travel first. Good for test cuts and simple shapes.",
                    ),
                    (
                        CutStrategy::OutlinesLast,
                        strategy_name(CutStrategy::OutlinesLast),
                        "Like Quality, but every outline comes last, so the sheet holds together until the end.",
                    ),
                ],
                true,
            );
            None
        });
        page_note(
            ui,
            &t,
            "New documents start with these. An open document keeps its own.",
        );
    });
    section(ui, cx, "Transform", |ui, cx| {
        row_ui(ui, cx, "rotate_each", |ui| {
            Some(ui.checkbox(
                &mut d.rotate_each,
                "Turn and scale each object about its own centre",
            ))
        });
    });
}

/// The Claude Code command that adds FlatCraft, with the binary path in
/// double quotes for the shell (paths with spaces work).
pub fn claude_code_command(bin: &str) -> String {
    let mut quoted = String::from('"');
    for c in bin.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    format!("claude mcp add flatcraft -- {quoted} mcp --window")
}

/// The Claude Desktop config entry, with the binary path as a JSON string.
pub fn claude_desktop_config(bin: &str) -> String {
    let command = serde_json::to_string(bin).unwrap_or_else(|_| "\"flatcraft\"".into());
    format!(
        "{{\n  \"mcpServers\": {{\n    \"flatcraft\": {{\n      \"command\": {command},\n      \"args\": [\"mcp\", \"--window\"]\n    }}\n  }}\n}}"
    )
}

/// The path of this FlatCraft binary for the setup snippets.
fn binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/path/to/flatcraft".into())
}

fn agents(ui: &mut Ui, cx: &mut Cx<'_>, d: &mut Preferences) {
    section(ui, cx, "Agent cuts", |ui, cx| {
        let t = *cx.t;
        row_ui(ui, cx, "allow_agent_cuts", |ui| {
            let r = ui.checkbox(&mut d.allow_agent_cuts, "Allow agents to start cuts");
            let warn = if d.allow_agent_cuts {
                t.accent_cut
            } else {
                t.text_dim
            };
            ui.add(
                egui::Label::new(RichText::new(profiles_window::AGENT_CUTS_WARNING).color(warn))
                    .wrap(),
            );
            Some(r)
        });
    });
    section(ui, cx, "Connect an agent", |ui, cx| {
        let t = *cx.t;
        row_ui(ui, cx, "mcp", |ui| {
            ui.add(
                egui::Label::new(
                    "FlatCraft speaks MCP over stdio. Your agent's client starts it, so there is no server to run, no port and no token.",
                )
                .wrap(),
            );
            let bin = binary_path();
            ui.add_space(metrics::SPACE_4);
            ui.label(RichText::new("For Claude Code:").strong());
            snippet(
                ui,
                &claude_code_command(&bin),
                "Copy the Claude Code command",
                &t,
            );
            ui.label(RichText::new("For Claude Desktop, in claude_desktop_config.json:").strong());
            snippet(
                ui,
                &claude_desktop_config(&bin),
                "Copy the Claude Desktop config",
                &t,
            );
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "With --window you watch the agent work. An agent can never change these settings, and a USB cut it starts waits for your click unless you allow it above.",
                    )
                    .color(t.text_dim),
                )
                .wrap(),
            );
            None
        });
    });
}

/// A block of text to paste somewhere else, with a Copy button.
fn snippet(ui: &mut Ui, text: &str, copy: &str, t: &Tokens) {
    egui::Frame::new()
        .fill(t.surface)
        .corner_radius(t.ribbon_radius)
        .inner_margin(egui::Margin::same(metrics::SPACE_8 as i8))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .monospace()
                        .size(metrics::TEXT_SECONDARY),
                )
                .wrap(),
            );
        });
    let r = ui.button("Copy");
    set_name(ui, &r, copy);
    super::groups::card(
        &r,
        copy,
        "Copies the text above, so you can paste it into your agent's setup.",
        None,
        false,
    );
    if r.clicked() {
        ui.ctx().copy_text(text.to_string());
    }
}

fn shortcuts(ui: &mut Ui, cx: &mut Cx<'_>) {
    section(ui, cx, "Keyboard", |ui, cx| {
        row_ui(ui, cx, "shortcuts", |ui| {
            egui::Grid::new("flatcraft-settings-shortcuts")
                .num_columns(2)
                .spacing([metrics::SPACE_16, metrics::SPACE_4])
                .show(ui, |ui| {
                    for s in help::SHORTCUTS.iter().filter(|s| s.here()) {
                        ui.label(RichText::new(s.keys()).monospace());
                        ui.add(egui::Label::new(s.what).wrap());
                        ui.end_row();
                    }
                });
            None
        });
    });
}

/// The search results in place of the page.
fn results(ui: &mut Ui, st: &mut SettingsState, now: Instant, t: &Tokens) {
    let hits = search(&st.search);
    let (r, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), metrics::CONTROL), Sense::hover());
    paint(
        ui,
        r.left_center(),
        egui::Align2::LEFT_CENTER,
        "Search results",
        metrics::title(),
        t.text,
    );
    if hits.is_empty() {
        ui.label(
            RichText::new("Nothing found. Try another word, like theme, blade or folder.")
                .color(t.text_dim),
        );
        return;
    }
    ui.add_space(metrics::SPACE_4);
    let mut picked = None;
    egui::ScrollArea::vertical()
        .id_salt("flatcraft-settings-results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for hit in hits {
                let name = format!("{} ({})", hit.label, hit.page.name());
                let (rect, resp) = ui.allocate_exact_size(
                    vec2(ui.available_width(), metrics::ROW + metrics::SPACE_16),
                    Sense::click(),
                );
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                if resp.hovered() {
                    ui.painter().rect_filled(rect, t.ribbon_radius, t.surface);
                }
                paint(
                    ui,
                    rect.left_top() + vec2(metrics::SPACE_8, metrics::SPACE_4),
                    egui::Align2::LEFT_TOP,
                    hit.label,
                    metrics::section_title(),
                    t.text,
                );
                paint(
                    ui,
                    rect.left_top() + vec2(metrics::SPACE_8, metrics::SPACE_24),
                    egui::Align2::LEFT_TOP,
                    &format!("{} > {}", hit.page.name(), hit.section),
                    fonts::medium(metrics::TEXT_SMALL),
                    t.text_dim,
                );
                super::groups::card(&resp, hit.label, hit.card.text(), None, true);
                if resp.clicked() {
                    picked = Some(hit);
                }
            }
        });
    if let Some(hit) = picked {
        st.jump(hit, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key of the preferences file, from the defaults with every
    /// optional field set.
    fn pref_keys() -> Vec<String> {
        let p = Preferences {
            output_dir: Some("/x".into()),
            ..Preferences::default()
        };
        let table: toml::Table = toml::from_str(&toml::to_string(&p).unwrap()).unwrap();
        table.keys().cloned().collect()
    }

    #[test]
    fn every_preference_has_a_row() {
        let rows: Vec<&str> = fields().collect();
        for key in pref_keys() {
            // The recent projects list and its details are history, not
            // settings; the version of the default pins is bookkeeping
            // that goes with the pins; the open panel sections are where
            // the person left the panels, changed by a click on them; the
            // tool card values are what the cards last ran with, kept by
            // the cards themselves.
            if matches!(
                key.as_str(),
                "recent_projects"
                    | "recent_details"
                    | "pin_defaults_version"
                    | "panel_sections"
                    | "tool_cards"
            ) {
                continue;
            }
            assert!(rows.contains(&key.as_str()), "no settings row for {key}");
        }
        for f in &rows {
            assert!(pref_keys().iter().any(|k| k == f), "{f} is no preference");
        }
    }

    #[test]
    fn pins_in_another_order_are_the_same_pins() {
        let mut d = Preferences::default();
        d.ribbon_pins.reverse();
        assert!(!differs("ribbon_pins", &d, &Preferences::default()));
        // Reset on the Layout page stays dimmed.
        assert!(!page_differs(Page::Layout, &d));
        d.ribbon_pins.pop();
        assert!(differs("ribbon_pins", &d, &Preferences::default()));
    }

    #[test]
    fn copy_field_knows_every_row() {
        let def = Preferences::default();
        let other = Preferences {
            allow_agent_cuts: true,
            default_plotter: "x".into(),
            default_material: "y".into(),
            theme: Theme::Light,
            recent_projects: Vec::new(),
            output_dir: Some("/x".into()),
            auto_reload: true,
            rotate_each: true,
            ribbon_pins: Vec::new(),
            pin_defaults_version: 0,
            tool_cards: Default::default(),
            panel_objects: PanelUser::Open,
            panel_settings: PanelUser::Collapsed,
            panel_stage: PanelUser::Open,
            default_strategy: CutStrategy::Fast,
            recent_details: Vec::new(),
            panel_sections: Default::default(),
            hidden_tips: ["x".to_string()].into(),
            mat_color_light: [1, 2, 3],
            mat_color_dark: [4, 5, 6],
        };
        for f in fields() {
            assert!(differs(f, &other, &def), "{f} is not copied");
        }
    }

    #[test]
    fn rows_have_plain_cards() {
        let mut ids = BTreeSet::new();
        for r in ROWS {
            assert!(ids.insert(r.id), "{} twice", r.id);
            let card = r.card.text();
            assert!(!card.is_empty(), "{}", r.id);
            assert!(card.ends_with('.'), "{}: {card}", r.id);
            assert!(card.starts_with(char::is_uppercase), "{}: {card}", r.id);
            for text in [card, r.label, r.section, r.page.purpose()] {
                assert!(!text.contains('\u{2014}'), "{}: em-dash", r.id);
            }
        }
        for p in Page::ALL {
            assert!(p.purpose().ends_with('.'));
        }
    }

    #[test]
    fn merge_writes_only_what_the_draft_changed() {
        let base = Preferences::default();
        let mut draft = base.clone();
        draft.theme = Theme::Light;
        // Someone else stored another field meanwhile.
        let mut stored = base.clone();
        stored.auto_reload = true;
        merge(&draft, &base, &mut stored);
        assert_eq!(stored.theme, Theme::Light);
        assert!(stored.auto_reload);
    }

    #[test]
    fn search_finds_by_label_and_by_card() {
        assert_eq!(search("theme")[0].id, "theme");
        assert_eq!(search("voltron")[0].id, "rotate_each");
        assert_eq!(search("trust")[0].id, "allow_agent_cuts");
        assert!(search("").is_empty());
        assert!(search("zzzz").is_empty());
    }

    #[test]
    fn restore_touches_only_its_page() {
        let mut d = Preferences {
            theme: Theme::Light,
            auto_reload: true,
            ..Preferences::default()
        };
        restore_page(Page::Appearance, &mut d);
        assert_eq!(d.theme, Theme::System);
        assert!(d.auto_reload);
    }

    #[test]
    fn setup_snippets_quote_a_path_with_spaces() {
        let bin = "/Applications/Flat Craft.app/Contents/MacOS/flatcraft";
        assert_eq!(
            claude_code_command(bin),
            "claude mcp add flatcraft -- \"/Applications/Flat Craft.app/Contents/MacOS/flatcraft\" mcp --window"
        );
        assert_eq!(
            claude_code_command("/a/\"$x\""),
            "claude mcp add flatcraft -- \"/a/\\\"\\$x\\\"\" mcp --window"
        );
        let json: serde_json::Value = serde_json::from_str(&claude_desktop_config(bin)).unwrap();
        assert_eq!(json["mcpServers"]["flatcraft"]["command"], bin);
        let windows = r"C:\Program Files\FlatCraft\flatcraft.exe";
        let json: serde_json::Value =
            serde_json::from_str(&claude_desktop_config(windows)).unwrap();
        assert_eq!(json["mcpServers"]["flatcraft"]["command"], windows);
    }

    #[test]
    fn the_window_fits_small_screens() {
        let s = window_size(vec2(900.0, 600.0));
        assert!(s.x <= 900.0 - 2.0 * MARGIN && s.y <= 600.0 - 2.0 * MARGIN);
        assert_eq!(window_size(vec2(1920.0, 1200.0)), vec2(1040.0, 760.0));
    }
}
