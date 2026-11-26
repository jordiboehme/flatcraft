//! A group of the ribbon, drawn from the table in [`super::tools`].
//!
//! A group shows its pinned actions as buttons (big or small, see
//! [`Style`]) and its inline settings, and under them a group button as
//! wide as the group, with the group's title and a chevron. The group
//! button opens the group's list: every action with its icon, title and
//! shortcut, a pin toggle on hover, and the group's options (what Align
//! lines up with). A group with no pins, or one the ribbon collapsed to
//! save room, is only its tile: the group's icon as a big button with its
//! title and a small chevron, which opens the same list. The group never
//! disappears.
//!
//! Hovering an action or a group button shows its card: the title and
//! the description of the table. An action that cannot run now stays
//! visible but dimmed; its card starts with the reason.

use egui::{Id, RectAlign, Response, RichText, Ui};

use super::collapse::Mode;
use super::help;
use super::icons::{self, Icon};
use super::prepare::{PrepareState, Snapshot};
use super::settings::set_name;
use super::tools::{AlignTo, GroupDef, GroupId, Section, Style, Tool};
use super::{Action, ICON_SIZE};

/// Height of the row of pinned buttons, as tall as a big button.
pub const BIG_HEIGHT: f32 = 48.0;
/// Height of the group button under the pinned buttons.
pub const GROUP_BUTTON_HEIGHT: f32 = 16.0;
/// Room between the pinned buttons and the group button.
pub const GROUP_GAP: f32 = 2.0;
/// Height of the tools row of the ribbon on every tab.
pub const ROW_HEIGHT: f32 = BIG_HEIGHT + GROUP_GAP + GROUP_BUTTON_HEIGHT;

/// The footer of every list.
pub const FOOTER: &str = "Pinned actions show on the ribbon. Hover a row to pin or unpin it.";
/// The tooltips of the pin toggle.
pub const PIN_TIP: &str = "Add to toolbar";
pub const UNPIN_TIP: &str = "Remove from toolbar";

/// What the groups draw with and work on.
pub struct Cx<'a> {
    pub st: &'a mut PrepareState,
    pub snap: &'a Snapshot,
    pub show_debug: &'a mut bool,
    pub actions: &'a mut Vec<Action>,
}

/// The accessible name of the toggle that pins `tool` (or unpins it).
pub fn pin_name(tool: &Tool, pinned: bool) -> String {
    let what = tool.pin_label();
    if pinned {
        format!("Unpin {what}")
    } else {
        format!("Pin {what}")
    }
}

// ---------------------------------------------------------------- cards

/// A hover card: the title, then the description. With a `reason` (the
/// thing cannot run now), the reason comes first and the description is
/// dimmed under it. `beside` puts the card next to a list row instead of
/// under the widget. Shows after the normal tooltip delay, on enabled and
/// on dimmed widgets.
pub fn card(resp: &Response, title: &str, description: &str, reason: Option<&str>, beside: bool) {
    for tip in [
        egui::Tooltip::for_enabled(resp),
        egui::Tooltip::for_disabled(resp),
    ] {
        let mut tip = tip;
        if beside {
            tip.popup = tip
                .popup
                .align(RectAlign::RIGHT_START)
                .align_alternatives(&[RectAlign::LEFT_START, RectAlign::BOTTOM_START]);
        }
        tip.show(|ui| card_ui(ui, title, description, reason));
    }
}

/// A card next to the pointer, for a part of a large widget (a handle on
/// the canvas): a card placed by the widget's rect would sit far away.
/// With `above` it opens above the pointer (at a top corner of the
/// selection, so it does not cover it), else below.
pub fn card_at_pointer(
    resp: &Response,
    title: &str,
    description: &str,
    reason: Option<&str>,
    above: bool,
) {
    for tip in [
        egui::Tooltip::for_enabled(resp),
        egui::Tooltip::for_disabled(resp),
    ] {
        let mut tip = tip;
        tip.popup = tip.popup.at_pointer();
        if above {
            tip.popup = tip
                .popup
                .align(RectAlign::TOP_START)
                .align_alternatives(&[RectAlign::TOP_END, RectAlign::BOTTOM_START]);
        }
        tip.show(|ui| card_ui(ui, title, description, reason));
    }
}

/// The contents of a card.
pub fn card_ui(ui: &mut Ui, title: &str, description: &str, reason: Option<&str>) {
    ui.set_max_width(280.0);
    ui.label(RichText::new(title).strong());
    match reason {
        Some(why) => {
            ui.label(why);
            ui.label(RichText::new(description).weak());
        }
        None => {
            ui.label(description);
        }
    }
}

/// The card of an action, with the reason when it cannot run now.
pub fn tool_card(resp: &Response, tool: &Tool, snap: &Snapshot, beside: bool) {
    card(
        resp,
        &tool.card_title(snap),
        tool.description,
        tool.blocked(snap),
        beside,
    );
}

// ---------------------------------------------------------------- buttons

pub(crate) fn image(ui: &Ui, icon: Icon, size: f32) -> egui::Image<'static> {
    let tex = icons::texture(ui.ctx(), icon, size);
    egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(size),
    ))
}

/// A big ribbon button: a 24 pt icon above a small label (only the icon
/// when `show_label` is off), [`BIG_HEIGHT`] tall. Its accessible name is
/// `name`. A dimmed one (`enabled` off) does nothing when clicked.
/// `selected` makes it a toggle that shows whether it is on.
pub fn big_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    name: &str,
    show_label: bool,
    enabled: bool,
    selected: Option<bool>,
) -> Response {
    const ICON: f32 = 24.0;
    ui.add_enabled_ui(enabled, |ui| {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let text = if show_label { label } else { "" };
        let galley =
            ui.painter()
                .layout_no_wrap(text.to_string(), font, egui::Color32::PLACEHOLDER);
        let width = if show_label {
            (galley.size().x + 12.0).max(BIG_HEIGHT)
        } else {
            ICON + 12.0
        };
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(width, BIG_HEIGHT), egui::Sense::click());
        let on = ui.is_enabled();
        resp.widget_info(|| match selected {
            Some(sel) => egui::WidgetInfo::selected(egui::accesskit::Role::Button, on, sel, name),
            None => egui::WidgetInfo::labeled(egui::accesskit::Role::Button, on, name),
        });
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&resp);
            paint_state(ui, rect, &resp, selected, on);
            let icon_y = if show_label {
                rect.top() + 4.0 + ICON / 2.0
            } else {
                rect.center().y
            };
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, icon_y),
                egui::Vec2::splat(ICON),
            );
            // A disabled Ui fades what its painter draws.
            image(ui, icon, ICON).paint_at(ui, icon_rect);
            let at = egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.bottom() - 4.0 - galley.size().y,
            );
            ui.painter().galley(at, galley, visuals.text_color());
        }
        resp
    })
    .inner
}

/// The background of a custom ribbon button: the hover fill, and the
/// selection fill of a toggle that is on. A dimmed toggle that is on gets
/// only a faint fill, so it does not look active.
fn paint_state(ui: &Ui, rect: egui::Rect, resp: &Response, selected: Option<bool>, enabled: bool) {
    let visuals = ui.style().interact(resp);
    let sel = &ui.visuals().selection;
    if selected == Some(true) {
        let fill = if enabled {
            sel.bg_fill
        } else {
            sel.bg_fill.gamma_multiply(0.3)
        };
        let stroke = if enabled {
            sel.stroke
        } else {
            egui::Stroke::NONE
        };
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            fill,
            stroke,
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() || resp.has_focus() {
        ui.painter().rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
}

/// A step of the Cut bar, as tall as the ribbon's tools row
/// ([`ROW_HEIGHT`]) and drawn like a big ribbon button: the 24 pt icon,
/// under it `title`, and under that `detail` in the dim text colour (left
/// out when `None`). It is a tab named `name`; `selected` gets the
/// selection fill. A dimmed one (`enabled` off) does nothing when clicked.
pub fn step_button(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    detail: Option<&str>,
    name: &str,
    enabled: bool,
    selected: bool,
) -> Response {
    const ICON: f32 = 24.0;
    ui.add_enabled_ui(enabled, |ui| {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let layout = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.to_string(), font.clone(), egui::Color32::PLACEHOLDER)
        };
        let title_g = layout(title);
        let detail_g = detail.map(layout);
        let text_w = detail_g
            .as_ref()
            .map_or(title_g.size().x, |d| d.size().x.max(title_g.size().x));
        let width = (text_w + 16.0).max(BIG_HEIGHT);
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::click());
        let on = ui.is_enabled();
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::accesskit::Role::Tab, on, selected, name)
        });
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&resp);
            paint_state(ui, rect, &resp, Some(selected), on);
            let gap = 3.0;
            let block =
                ICON + gap + title_g.size().y + detail_g.as_ref().map_or(0.0, |d| d.size().y);
            let top = rect.center().y - block / 2.0;
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.center().x, top + ICON / 2.0),
                egui::Vec2::splat(ICON),
            );
            // A disabled Ui fades what its painter draws.
            image(ui, icon, ICON).paint_at(ui, icon_rect);
            let mut y = icon_rect.bottom() + gap;
            let p = ui.painter();
            let h = title_g.size().y;
            p.galley(
                egui::pos2(rect.center().x - title_g.size().x / 2.0, y),
                title_g,
                visuals.text_color(),
            );
            y += h;
            if let Some(d) = detail_g {
                let dim = if on {
                    ui.visuals().weak_text_color()
                } else {
                    visuals.text_color()
                };
                p.galley(egui::pos2(rect.center().x - d.size().x / 2.0, y), d, dim);
            }
        }
        resp
    })
    .inner
}

/// Height of a compact button: two of them, or a row of small buttons,
/// stack inside [`BIG_HEIGHT`].
pub const COMPACT_HEIGHT: f32 = 22.0;

/// A compact switch ("Each object", "Keep aspect"): a 16 pt icon, with
/// its label at its right when `show_label` is on. It is a checkbox named
/// `name` with its state, so it reads as a switch to a screen reader.
pub fn compact_switch(
    ui: &mut Ui,
    icon: Icon,
    name: &str,
    show_label: bool,
    enabled: bool,
    on: bool,
) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let text = if show_label { name } else { "" };
        let galley =
            ui.painter()
                .layout_no_wrap(text.to_string(), font, egui::Color32::PLACEHOLDER);
        // An icon-only toggle is as narrow as its icon allows.
        let pad = if show_label { 4.0 } else { 2.0 };
        let width = if show_label {
            pad + SMALL_ICON + pad + galley.size().x + pad + 2.0
        } else {
            SMALL_ICON + 2.0 * pad
        };
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(width, COMPACT_HEIGHT), egui::Sense::click());
        let live = ui.is_enabled();
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::accesskit::Role::CheckBox, live, on, name)
        });
        if ui.is_rect_visible(rect) {
            let visuals = ui.style().interact(&resp);
            paint_state(ui, rect, &resp, Some(on), live);
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.left() + pad + SMALL_ICON / 2.0, rect.center().y),
                egui::Vec2::splat(SMALL_ICON),
            );
            image(ui, icon, SMALL_ICON).paint_at(ui, icon_rect);
            ui.painter().galley(
                egui::pos2(
                    icon_rect.right() + pad,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                visuals.text_color(),
            );
        }
        resp
    })
    .inner
}

/// Size of the icons of small buttons; two rows of them fit [`BIG_HEIGHT`].
const SMALL_ICON: f32 = 16.0;

/// A small icon button for a pinned direction. Its accessible name is
/// `name`.
fn small_button(ui: &mut Ui, icon: Icon, name: &str, enabled: bool) -> Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(3.0, 2.0);
        ui.add_enabled(
            enabled,
            egui::Button::image(image(ui, icon, SMALL_ICON).alt_text(name)),
        )
    })
    .inner
}

/// A wide button: icon and text, or only the icon (`Mode::Icons`, the
/// text stays its accessible name). `selected` makes it a toggle.
pub fn wide_button(
    ui: &mut Ui,
    mode: Mode,
    icon: Icon,
    text: &str,
    enabled: bool,
    selected: Option<bool>,
) -> Response {
    let img = image(ui, icon, ICON_SIZE);
    let mut b = match mode {
        Mode::Icons | Mode::Collapsed => egui::Button::image(img.alt_text(text)),
        Mode::Full | Mode::List => egui::Button::image_and_text(img, text),
    };
    match selected {
        // A dimmed switch that is on must not look like an active one:
        // only a faint fill says it is on.
        Some(true) if !enabled => {
            b = b.fill(ui.visuals().selection.bg_fill.gamma_multiply(0.3));
        }
        Some(on) => b = b.selected(on),
        None => {}
    }
    ui.add_enabled(enabled, b)
}

/// The width of a group's tile: the same numbers [`tile`] sizes itself
/// with, so the collapsing ribbon can count on it before it draws one.
pub fn tile_width(ui: &Ui, title: &str) -> f32 {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_string(), font, egui::Color32::PLACEHOLDER);
    (galley.size().x + 12.0).max(BIG_HEIGHT)
}

/// The id of a group's button and tile: one id, so the list stays open
/// when the group switches between the two.
fn group_id(group: GroupId) -> Id {
    Id::unique(("flatcraft-group", group))
}

/// A group's tile: its icon, title and a small chevron, [`ROW_HEIGHT`]
/// tall and [`tile_width`] wide.
fn tile(ui: &mut Ui, def: &GroupDef) -> Response {
    const ICON: f32 = 24.0;
    let width = tile_width(ui, def.title);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
    let resp = ui.interact(rect, group_id(def.id), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, def.title));
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&resp);
        if resp.hovered() || resp.has_focus() {
            ui.painter().rect(
                rect,
                visuals.corner_radius,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.top() + 6.0 + ICON / 2.0),
            egui::Vec2::splat(ICON),
        );
        image(ui, def.icon, ICON).paint_at(ui, icon_rect);
        let font = egui::TextStyle::Small.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(def.title.to_string(), font, visuals.text_color());
        let text_top = icon_rect.bottom() + 4.0;
        ui.painter().galley(
            egui::pos2(rect.center().x - galley.size().x / 2.0, text_top),
            galley.clone(),
            visuals.text_color(),
        );
        let chevron = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, text_top + galley.size().y + 7.0),
            egui::Vec2::splat(12.0),
        );
        image(ui, Icon::ChevronDown, 12.0).paint_at(ui, chevron);
    }
    resp
}

/// The group button under the pinned buttons: the title and a chevron,
/// `width` wide (at least as wide as its text).
fn group_button(ui: &mut Ui, def: &GroupDef, width: f32) -> Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley =
        ui.painter()
            .layout_no_wrap(def.title.to_string(), font, egui::Color32::PLACEHOLDER);
    let chevron = 12.0;
    let width = width.max(galley.size().x + chevron + 12.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(width, GROUP_BUTTON_HEIGHT), egui::Sense::hover());
    let resp = ui.interact(rect, group_id(def.id), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, def.title));
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&resp);
        let fill = if resp.hovered() || resp.has_focus() {
            visuals.weak_bg_fill
        } else {
            ui.visuals().faint_bg_color
        };
        ui.painter().rect_filled(rect, visuals.corner_radius, fill);
        let text_w = galley.size().x + 2.0 + chevron;
        let left = rect.center().x - text_w / 2.0;
        let font = egui::TextStyle::Small.resolve(ui.style());
        let galley = ui
            .painter()
            .layout_no_wrap(def.title.to_string(), font, visuals.text_color());
        ui.painter().galley(
            egui::pos2(left, rect.center().y - galley.size().y / 2.0),
            galley.clone(),
            visuals.text_color(),
        );
        let c = egui::Rect::from_center_size(
            egui::pos2(
                left + galley.size().x + 2.0 + chevron / 2.0,
                rect.center().y,
            ),
            egui::Vec2::splat(chevron),
        );
        image(ui, Icon::ChevronDown, chevron).paint_at(ui, c);
    }
    resp
}

// ---------------------------------------------------------------- group

/// What a group adds to the ribbon besides its actions.
#[derive(Clone, Copy, Default)]
pub struct Extras {
    /// Settings drawn after the pinned buttons ("Each object").
    pub inline: Option<fn(&mut Ui, Mode, &mut Cx)>,
    /// The same settings at the top of the list, while the group is a
    /// tile and does not show them.
    pub in_list: Option<fn(&mut Ui, &mut Cx)>,
}

/// Draws a group in `mode`: its pinned actions, its inline settings and
/// its group button, or only its tile (collapsed, or nothing pinned).
pub fn show(ui: &mut Ui, mode: Mode, group: GroupId, extras: Extras, cx: &mut Cx) {
    let def = group.def();
    let pins = super::tools::pinned(cx.snap, group);
    let collapsed = mode == Mode::Collapsed || pins.is_empty();
    let resp = if collapsed {
        tile(ui, def)
    } else {
        let inner = ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = GROUP_GAP;
            let top = pins_row(ui, |ui| {
                pinned_buttons(ui, mode, &pins, cx);
                if let Some(inline) = extras.inline {
                    inline(ui, mode, cx);
                }
            });
            group_button(ui, def, top.width())
        });
        inner.inner
    };
    card(&resp, def.title, def.description, None, false);
    // Search can open the list (`reveal`).
    let open = if cx.st.open_group == Some(group) {
        cx.st.open_group = None;
        Some(egui::SetOpenCommand::Bool(true))
    } else {
        resp.clicked().then_some(egui::SetOpenCommand::Toggle)
    };
    let config = egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    // The list stays open while pinning, so several actions can be pinned
    // in a row; a click on a row runs it and closes the list.
    egui::Popup::menu(&resp)
        .style(super::theme::tight_menu)
        .open_memory(open)
        .close_behavior(config.close_behavior)
        .info(
            egui::UiStackInfo::new(egui::UiKind::Menu)
                .with_tag_value(egui::containers::menu::MenuConfig::MENU_CONFIG_TAG, config),
        )
        .show(|ui| {
            if collapsed && let Some(f) = extras.in_list {
                f(ui, cx);
                ui.separator();
            }
            list(ui, group, cx);
        });
}

/// A row [`BIG_HEIGHT`] tall whose widgets are centred in it, from the
/// left. Returns the rect it used.
pub fn pins_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> egui::Rect {
    let width = ui.available_width();
    let r = ui.allocate_ui_with_layout(
        egui::vec2(width, BIG_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_height(BIG_HEIGHT);
            add(ui);
        },
    );
    r.response.rect
}

/// The pinned actions: big buttons, then the small ones in one row, or
/// two when there are more than three. Every action of every tab looks the
/// same way, so a group's pins fill the row's height.
fn pinned_buttons(ui: &mut Ui, mode: Mode, pins: &[&'static Tool], cx: &mut Cx) {
    for t in pins.iter().filter(|t| t.style == Style::Big) {
        let on = t.blocked(cx.snap).is_none();
        let sel = t.selected(cx.st, *cx.show_debug);
        let name = t.button_name();
        let r = big_button(ui, t.icon, &name, &name, mode == Mode::Full, on, sel);
        tool_card(&r, t, cx.snap, false);
        if r.clicked() {
            t.run(cx.snap, cx.st, cx.show_debug, cx.actions);
        }
    }
    let small: Vec<&Tool> = pins
        .iter()
        .copied()
        .filter(|t| t.style == Style::Small)
        .collect();
    if small.is_empty() {
        return;
    }
    let rows = if small.len() > 3 { 2 } else { 1 };
    let per_row = small.len().div_ceil(rows);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for chunk in small.chunks(per_row) {
            ui.horizontal(|ui| {
                for t in chunk {
                    let on = t.blocked(cx.snap).is_none();
                    let r = small_button(ui, t.icon, &t.button_name(), on);
                    tool_card(&r, t, cx.snap, false);
                    if r.clicked() {
                        t.run(cx.snap, cx.st, cx.show_debug, cx.actions);
                    }
                }
            });
        }
    });
}

/// The heading of a section of the list. Align also asks what to line up
/// with; switching it switches the rows under it at once.
fn heading(ui: &mut Ui, section: Section, st: &mut PrepareState) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add(image(ui, section.icon(), ICON_SIZE));
        ui.label(RichText::new(section.title()).strong());
    });
    if section == Section::Align {
        ui.indent("flatcraft-align-to", |ui| {
            let to_area = &mut st.fields.align_to_area;
            ui.radio_value(to_area, false, AlignTo::Selection.label());
            ui.radio_value(to_area, true, AlignTo::Area.label());
        });
    }
}

/// Width of a column of a list.
const COLUMN: f32 = 270.0;

/// The rows of a group's list. Each row runs its action; hovering it shows
/// its card beside the list and its pin toggle. Align and Distribute go
/// into a second column, so a long list fits a small window; a list taller
/// than the window scrolls.
pub fn list(ui: &mut Ui, group: GroupId, cx: &mut Cx) {
    let to = AlignTo::of(cx.st.fields.align_to_area);
    // Rows with an option show the ones of the option picked above.
    let rows: Vec<&'static Tool> = group
        .tools()
        .filter(|t| t.option().is_none_or(|o| o == to))
        .collect();
    let second = |t: &Tool| matches!(t.section, Some(Section::Align | Section::Distribute));
    let left: Vec<&'static Tool> = rows.iter().copied().filter(|t| !second(t)).collect();
    let right: Vec<&'static Tool> = rows.iter().copied().filter(|t| second(t)).collect();
    let room = ui.ctx().content_rect().bottom() - ui.next_widget_position().y - 48.0;
    egui::ScrollArea::vertical()
        .max_height(room.max(120.0))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                column(ui, &left, cx);
                if !right.is_empty() {
                    ui.separator();
                    column(ui, &right, cx);
                }
            });
        });
    ui.separator();
    ui.label(RichText::new(FOOTER).weak().small());
}

/// One column of a list, with the section headings.
fn column(ui: &mut Ui, rows: &[&'static Tool], cx: &mut Cx) {
    ui.vertical(|ui| {
        ui.set_width(COLUMN);
        let mut last = None;
        for t in rows {
            if t.section != last
                && let Some(s) = t.section
            {
                heading(ui, s, cx.st);
            }
            last = t.section;
            row(ui, t, cx);
        }
    });
}

/// One row: the action's button (dimmed when it cannot run now) and its
/// pin toggle.
fn row(ui: &mut Ui, t: &'static Tool, cx: &mut Cx) {
    let row_id = ui.make_persistent_id(("flatcraft-tool-row", t.id));
    let last: Option<egui::Rect> = ui.data(|d| d.get_temp(row_id));
    let hovered = last.is_some_and(|r| ui.rect_contains_pointer(r));
    // The row or its pin toggle had the keyboard focus last frame: the
    // toggle stays, so Tab reaches it.
    let focus_id = row_id.with("focus");
    let focused: bool = ui.data(|d| d.get_temp(focus_id)).unwrap_or(false);
    let is_pinned = cx.snap.ribbon_pins.iter().any(|p| p == t.id);
    let indent = t.section.is_some();
    let mut has_focus = false;
    let row = ui.horizontal(|ui| {
        if indent {
            ui.add_space(ICON_SIZE);
        }
        let main_width = ui.available_width() - ICON_SIZE - 3.0 * ui.spacing().item_spacing.x;
        let mut b = egui::Button::image_and_text(image(ui, t.icon, ICON_SIZE), t.title)
            .min_size(egui::vec2(main_width, 0.0));
        // The shortcut at the right; without one, an empty right part keeps
        // the icon and title at the left.
        b = match t.shortcut() {
            Some(key) => b.shortcut_text(help::hint(key)),
            None => b.right_text(""),
        };
        if let Some(on) = t.selected(cx.st, *cx.show_debug) {
            b = b.selected(on);
        }
        let r = ui.add_enabled(t.blocked(cx.snap).is_none(), b);
        set_name(ui, &r, &t.row_name());
        tool_card(&r, t, cx.snap, true);
        if r.clicked() {
            t.run(cx.snap, cx.st, cx.show_debug, cx.actions);
            ui.close();
        }
        has_focus = r.has_focus();
        if hovered || is_pinned || focused || has_focus {
            let (icon, tip) = match (is_pinned, hovered) {
                (true, true) => (Icon::PinOff, UNPIN_TIP),
                (true, false) => (Icon::Pin, UNPIN_TIP),
                (false, _) => (Icon::Pin, PIN_TIP),
            };
            let r = super::icon_button_hint(ui, icon, &pin_name(t, is_pinned), true, tip);
            has_focus |= r.has_focus();
            if r.clicked() {
                cx.actions.push(Action::SetPin {
                    id: t.id,
                    pinned: !is_pinned,
                });
            }
        }
    });
    ui.data_mut(|d| {
        d.insert_temp(row_id, row.response.rect);
        d.insert_temp(focus_id, has_focus);
    });
}
