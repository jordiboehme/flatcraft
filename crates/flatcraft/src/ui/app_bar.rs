//! The app bar: the app icon and name, Home, File, Save, Undo, Redo and
//! the document tabs on the left; the agent pill, Settings, Help and the
//! version on the right. On macOS it is the window's title bar: the
//! window content runs under a transparent title bar, the traffic lights
//! stay at the left, and dragging the bar's empty space moves the window.

use std::time::Instant;

use egui::{RichText, Ui};

use super::groups::card;
use super::icons::{self, Icon};
use super::settings::set_name;
use super::theme::Tokens;
use super::{Action, FlatCraftApp, ICON_SIZE, Workspace, display_name, fonts, lock};

/// Height of the app bar. On macOS it is the title bar's height, so the
/// traffic lights sit in its middle.
pub const BAR_HEIGHT: f32 = 32.0;

/// The room the macOS traffic lights take at the left of the title bar.
///
/// Measured on macOS 27.2.1 (`NSWindow standardWindowButton` frames in a
/// window with a full-size content view and a transparent title bar):
/// the title bar is 32 pt tall, the close button starts at x 9 and the
/// zoom button ends at x 69 (three 14 pt buttons, 9 pt apart). The bar
/// keeps the same 9 pt after them: 69 + 9 = 78. Older macOS versions draw
/// them smaller (they end at about x 61), so this leaves room there too.
pub const TRAFFIC_LIGHTS: f32 = 78.0;

/// The window options of FlatCraft's window. With `title_bar` (macOS) the
/// content fills the whole window under a transparent title bar without
/// a title, and the app bar takes the title bar's place.
pub fn viewport(title_bar: bool) -> egui::ViewportBuilder {
    let v = egui::ViewportBuilder::default()
        .with_title("FlatCraft")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size(super::MIN_WINDOW);
    if title_bar {
        v.with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
    } else {
        v
    }
}

/// The room at the left of the bar that stays free for the traffic
/// lights: [`TRAFFIC_LIGHTS`] in the macOS title bar, none in full screen
/// (macOS hides them there) or under a system title bar.
pub fn left_inset(title_bar: bool, ctx: &egui::Context) -> f32 {
    let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
    if title_bar && !fullscreen {
        TRAFFIC_LIGHTS
    } else {
        0.0
    }
}

/// What a double-click on the title bar does (macOS: System Settings,
/// Desktop & Dock, "Double-click a window's title bar to").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubleClick {
    Zoom,
    Minimize,
    Nothing,
}

/// The action of the user default `AppleActionOnDoubleClick` ("Maximize",
/// "Minimize" or "None"), or of the older `AppleMiniaturizeOnDoubleClick`
/// switch when the first is not set. Zoom when neither is.
pub fn double_click_action(action: Option<&str>, miniaturize: bool) -> DoubleClick {
    match action {
        Some("Minimize") => DoubleClick::Minimize,
        Some("None") => DoubleClick::Nothing,
        Some(_) => DoubleClick::Zoom,
        None if miniaturize => DoubleClick::Minimize,
        None => DoubleClick::Zoom,
    }
}

/// The system's title-bar double-click action. Read when it is needed
/// (one defaults lookup per double-click); Zoom off macOS.
pub fn system_double_click() -> DoubleClick {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSString, NSUserDefaults};
        let defaults = NSUserDefaults::standardUserDefaults();
        let action = defaults
            .stringForKey(&NSString::from_str("AppleActionOnDoubleClick"))
            .map(|s| s.to_string());
        let mini = defaults.boolForKey(&NSString::from_str("AppleMiniaturizeOnDoubleClick"));
        double_click_action(action.as_deref(), mini)
    }
    #[cfg(not(target_os = "macos"))]
    {
        DoubleClick::Zoom
    }
}

/// One open document as the tabs show it.
struct Tab {
    id: u64,
    name: String,
    dirty: bool,
}

// The hover cards of the bar (Hints for beginners).
const HOME_HINT: &str = "The start page with your recent projects. Your open documents stay open.";
const FILE_HINT: &str =
    "New, Open, Save and Save As. A project file keeps your designs and where they sit on the mat.";
const SAVE_HINT: &str =
    "Store this document as a project file. The first time, FlatCraft asks where.";
const UNDO_HINT: &str = "Take back the last change to this document.";
const REDO_HINT: &str = "Bring back the change that Undo just took back.";
const HELP_HINT: &str = "The user guide, the keyboard shortcuts and where to report a problem.";
const SETTINGS_HINT: &str = "Your plotters, mats and materials, and how FlatCraft looks.";
const NEW_HINT: &str = "Open an empty mat in a new tab. Your other documents stay open.";
const ALL_HINT: &str = "Every open document, also the ones the tabs have no room for.";
const TAB_HINT: &str = "Click to show this document.";
/// The card of a tab with unsaved changes: it explains the dot.
pub const TAB_DIRTY_HINT: &str =
    "Click to show this document. The dot means it has changes that are not saved yet.";
const CLOSE_HINT: &str =
    "Close this document. If it has changes that are not saved, FlatCraft asks first.";

/// The mark after the name of a document with unsaved changes.
pub const DIRTY: &str = "\u{2022}";

/// What a tab says: the name, and the dot when it has unsaved changes.
pub fn tab_label(name: &str, dirty: bool) -> String {
    if dirty {
        format!("{name} {DIRTY}")
    } else {
        name.to_string()
    }
}

/// The id of the bar's background, which moves the window.
pub fn drag_id() -> egui::Id {
    egui::Id::unique("flatcraft-title-bar-drag")
}

pub fn show(ui: &mut Ui, app: &FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let (tabs, active, can_undo, can_redo) = {
        let s = lock(&app.session);
        let tabs: Vec<Tab> = s
            .docs
            .iter()
            .map(|d| Tab {
                id: d.id(),
                name: d.path.as_deref().map(display_name).unwrap_or_else(|| {
                    let n = app.ui.untitled.get(&d.id()).copied().unwrap_or(1);
                    format!("Untitled {n}")
                }),
                dirty: d.dirty,
            })
            .collect();
        let doc = s.doc();
        (tabs, doc.id(), doc.undo.can_undo(), doc.undo.can_redo())
    };
    let agent = app.agent_status(Instant::now());
    let inset = left_inset(app.title_bar, ui.ctx());
    // The two sides are separate closures; the right one collects here.
    let mut right_actions = Vec::new();

    egui::Panel::top("flatcraft-app-bar")
        .exact_size(BAR_HEIGHT)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(t.rail).inner_margin(egui::Margin {
            left: (t.spacing + inset) as i8,
            right: t.spacing as i8,
            top: 0,
            bottom: 0,
        }))
        .show(ui, |ui| {
            // The empty bar, under every widget: a drag moves the window and
            // a double-click zooms it, as the system title bar does. The
            // widgets come later, so they win the clicks on them.
            let whole = ui.max_rect().expand2(egui::vec2(t.spacing + inset, 0.0));
            let empty = ui.interact(whole, drag_id(), egui::Sense::click_and_drag());
            // The widgets that take a click: a drag that starts on one of
            // them is theirs, not the window's (egui hands a drag to the
            // background when the widget on top only takes clicks).
            let mut hot: Vec<egui::Rect> = Vec::new();
            let mut hot_right: Vec<egui::Rect> = Vec::new();
            ui.set_height(BAR_HEIGHT);
            ui.spacing_mut().item_spacing.x = 2.0;
            egui::Sides::new().shrink_left().height(BAR_HEIGHT).show(
                ui,
                |ui| {
                    app_name(ui, &t);
                    ui.add_space(t.spacing);
                    let home = flat_icon(ui, Icon::Home, "Home", HOME_HINT, None);
                    hot.push(home.rect);
                    if home.clicked() {
                        actions.push(Action::SetWorkspace(Workspace::Home));
                    }
                    let file = egui::Button::new("File").frame_when_inactive(false);
                    let file = egui::containers::menu::MenuButton::from_button(file).ui(ui, |ui| {
                        if ui.button("New").clicked() {
                            actions.push(Action::NewProject);
                        }
                        if ui.button("Open").clicked() {
                            actions.push(Action::OpenDialog);
                        }
                        if ui.button("Save").clicked() {
                            actions.push(Action::Save { doc_id: active });
                        }
                        if ui.button("Save As").clicked() {
                            actions.push(Action::SaveAs { doc_id: active });
                        }
                    });
                    card(&file.0, "File", FILE_HINT, None, false);
                    hot.push(file.0.rect);
                    let save = flat_icon(ui, Icon::Save, "Save", SAVE_HINT, None);
                    let undo = flat_icon(
                        ui,
                        Icon::Undo,
                        "Undo",
                        UNDO_HINT,
                        (!can_undo).then_some("There is nothing to undo yet."),
                    );
                    let redo = flat_icon(
                        ui,
                        Icon::Redo,
                        "Redo",
                        REDO_HINT,
                        (!can_redo).then_some(
                            "There is nothing to redo. Redo brings back what Undo took back.",
                        ),
                    );
                    hot.extend([save.rect, undo.rect, redo.rect]);
                    if save.clicked() {
                        actions.push(Action::Save { doc_id: active });
                    }
                    if undo.clicked() {
                        actions.push(Action::Undo(active));
                    }
                    if redo.clicked() {
                        actions.push(Action::Redo(active));
                    }
                    ui.add_space(t.spacing / 2.0);
                    thin_separator(ui, &t);
                    ui.add_space(t.spacing / 2.0);
                    doc_tabs(ui, &t, &tabs, active, actions, &mut hot);
                },
                |ui| {
                    let actions = &mut right_actions;
                    let version = egui::Label::new(
                        RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(t.text_dim),
                    )
                    .sense(egui::Sense::click());
                    let version = ui
                        .add(version)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("About FlatCraft");
                    if version.clicked() {
                        actions.push(Action::ShowAbout);
                    }
                    ui.add_space(t.spacing / 2.0);
                    let help = flat_icon(ui, Icon::Help, "Help", HELP_HINT, None);
                    egui::Popup::menu(&help).show(|ui| super::help::menu(ui, actions));
                    let settings = flat_icon(ui, Icon::Settings, "Settings", SETTINGS_HINT, None);
                    hot_right.extend([version.rect, help.rect, settings.rect]);
                    if settings.clicked() {
                        actions.push(Action::ShowProfiles);
                    }
                    if let Some(status) = agent {
                        let color = if status == "Agent connected" {
                            t.accent_prepare
                        } else {
                            t.text_dim
                        };
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1.0, color))
                            .corner_radius(t.radius * 2)
                            .inner_margin(egui::Margin::symmetric(t.spacing as i8, 2))
                            .show(ui, |ui| {
                                ui.label(RichText::new(status).color(color));
                            });
                    }
                },
            );
            hot.extend(hot_right);
            if app.title_bar {
                let on_empty = |pos: Option<egui::Pos2>| {
                    pos.is_some_and(|p| !hot.iter().any(|r| r.contains(p)))
                };
                let origin = ui.input(|i| i.pointer.press_origin());
                if empty.drag_started() && on_empty(origin) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if empty.double_clicked() && on_empty(empty.interact_pointer_pos()) {
                    let action = app.double_click.unwrap_or_else(system_double_click);
                    match action {
                        DoubleClick::Zoom => {
                            let zoomed =
                                ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Maximized(!zoomed));
                        }
                        DoubleClick::Minimize => {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        DoubleClick::Nothing => {}
                    }
                }
            }
        });
    actions.extend(right_actions);
}

/// The app icon and the name "FlatCraft".
fn app_name(ui: &mut Ui, t: &Tokens) {
    let tex = app_icon(ui.ctx(), t.is_dark());
    ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    )));
    ui.add_space(2.0);
    ui.label(RichText::new("FlatCraft").font(fonts::semibold(14.0)));
}

/// The app icon for a dark or a light bar, decoded once per context.
fn app_icon(ctx: &egui::Context, dark: bool) -> egui::TextureHandle {
    const DARK: &[u8] = include_bytes!("../../../../assets/branding/icon-dark-256.png");
    const LIGHT: &[u8] = include_bytes!("../../../../assets/branding/icon-light-256.png");
    let id = egui::Id::unique(("flatcraft-app-icon", dark));
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return tex;
    }
    let png = if dark { DARK } else { LIGHT };
    let image = match tiny_skia::Pixmap::decode_png(png) {
        Ok(p) => egui::ColorImage::from_rgba_premultiplied(
            [p.width() as usize, p.height() as usize],
            p.data(),
        ),
        // The PNG is built in, so this does not happen.
        Err(_) => egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
    };
    let tex = ctx.load_texture("flatcraft-app-icon", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// A flat icon button: no box, only a hover fill. `name` is its
/// accessible name and the title of its hover card, `description` says
/// what it does; a dimmed one (`why` set) says why first.
pub fn flat_icon(
    ui: &mut Ui,
    icon: Icon,
    name: &str,
    description: &str,
    why: Option<&str>,
) -> egui::Response {
    let tex = icons::texture(ui.ctx(), icon, ICON_SIZE);
    let image = egui::Image::from_texture(egui::load::SizedTexture::new(
        tex.id(),
        egui::Vec2::splat(ICON_SIZE),
    ))
    .alt_text(name);
    let r = ui.add_enabled(
        why.is_none(),
        egui::Button::image(image).frame_when_inactive(false),
    );
    card(&r, name, description, why, false);
    r
}

/// A thin vertical line between the commands and the tabs.
fn thin_separator(ui: &mut Ui, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 18.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, t.border);
}

/// The narrowest a tab's name gets before tabs move into the "»" list.
pub const MIN_TAB_LABEL: f32 = 72.0;

/// The accessible name of the button that lists every open document.
pub const ALL_DOCUMENTS: &str = "All documents";

/// How the document tabs fit their room.
#[derive(Debug, Clone, PartialEq)]
pub struct TabFit {
    /// The widest a tab's name button may be; `None`: as wide as it likes.
    pub cap: Option<f32>,
    /// The tabs shown, in order.
    pub shown: Vec<usize>,
    /// Whether the "»" list is needed.
    pub more: bool,
}

/// Fits tabs whose name buttons are `natural` wide into `room`. Each tab
/// also takes `extra` (its close button and the spacing). First the names
/// get narrower (down to [`MIN_TAB_LABEL`]), then only the tabs that fit
/// at that width are shown, with the "»" button (`more` wide). The active
/// tab is always among them.
pub fn fit_tabs(natural: &[f32], extra: f32, room: f32, more: f32, active: usize) -> TabFit {
    if natural.is_empty() {
        return TabFit {
            cap: None,
            shown: Vec::new(),
            more: false,
        };
    }
    let total =
        |idx: &[usize], cap: f32| -> f32 { idx.iter().map(|&i| natural[i].min(cap) + extra).sum() };
    // The largest cap that fits `room`, by bisection.
    let cap_for = |idx: &[usize], room: f32| -> Option<f32> {
        let widest = idx.iter().map(|&i| natural[i]).fold(0.0, f32::max);
        if total(idx, widest) <= room {
            return None;
        }
        let (mut lo, mut hi) = (0.0f32, widest);
        for _ in 0..30 {
            let mid = (lo + hi) / 2.0;
            if total(idx, mid) <= room {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(lo.floor())
    };
    let all: Vec<usize> = (0..natural.len()).collect();
    match cap_for(&all, room) {
        None => {
            return TabFit {
                cap: None,
                shown: all,
                more: false,
            };
        }
        Some(c) if c >= MIN_TAB_LABEL => {
            return TabFit {
                cap: Some(c),
                shown: all,
                more: false,
            };
        }
        Some(_) => {}
    }
    let room = room - more;
    // At least the active tab, even when not one tab fits at the narrowest
    // name: its name then gets narrower still, down to nothing, so the tab
    // never runs into the right side.
    let fits = ((room / (MIN_TAB_LABEL + extra)).floor().max(1.0) as usize).min(natural.len());
    let mut shown: Vec<usize> = (0..fits).collect();
    if active >= fits {
        shown[fits - 1] = active.min(natural.len() - 1);
    }
    let cap = cap_for(&shown, room).map(|c| c.max(0.0));
    TabFit {
        cap,
        shown,
        more: true,
    }
}

/// Height of a document tab.
const TAB_HEIGHT: f32 = 26.0;
/// Room left of a tab's name.
const TAB_PAD: f32 = 10.0;
/// The close button inside a tab, and the room around it.
const CLOSE: f32 = 16.0;
const CLOSE_PAD: f32 = 5.0;

/// What a tab takes besides its name and dot: the padding and the close
/// button, which always has its room so a tab does not grow on hover.
fn tab_extra() -> f32 {
    TAB_PAD + CLOSE_PAD + CLOSE + CLOSE_PAD
}

/// The width of a tab's name, cut to `cap` with an ellipsis.
fn name_galley(
    ui: &Ui,
    name: &str,
    cap: Option<f32>,
    color: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), font, color);
    if let Some(cap) = cap {
        job.wrap = egui::text::TextWrapping::truncate_at_width(cap.max(0.0));
    }
    ui.painter().layout_job(job)
}

/// One document tab: the name (cut with "..." when the room is short),
/// the dot when it has unsaved changes and the close button inside. The
/// active tab is raised in the panel colour; the others are quiet. The
/// close button shows on hover, and always on the active tab. Returns the
/// tab and the close button.
fn doc_tab(
    ui: &mut Ui,
    t: &Tokens,
    tab: &Tab,
    active: bool,
    cap: Option<f32>,
) -> (egui::Response, egui::Response) {
    let color = if active { t.text } else { t.text_dim };
    let name = name_galley(ui, &tab.name, cap, color);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let dot = tab.dirty.then(|| {
        ui.painter()
            .layout_no_wrap(format!(" {DIRTY}"), font, color)
    });
    let dot_w = dot.as_ref().map_or(0.0, |d| d.size().x);
    let width = name.size().x + dot_w + tab_extra();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, TAB_HEIGHT), egui::Sense::click());
    let label = tab_label(&tab.name, tab.dirty);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::accesskit::Role::Button, true, active, &label)
    });
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - CLOSE_PAD - CLOSE / 2.0, rect.center().y),
        egui::Vec2::splat(CLOSE),
    );
    let close = ui.interact(
        close_rect,
        egui::Id::unique(("flatcraft-tab-close", tab.id)),
        egui::Sense::click(),
    );
    let close_name = format!("Close {}", tab.name);
    close.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &close_name)
    });
    let hovered = resp.hovered() || close.hovered();
    if ui.is_rect_visible(rect) {
        let radius = egui::CornerRadius::same(t.ribbon_radius);
        if active {
            ui.painter().rect(
                rect,
                radius,
                t.panel,
                egui::Stroke::new(1.0, t.border),
                egui::StrokeKind::Inside,
            );
        } else if hovered {
            ui.painter()
                .rect_filled(rect, radius, t.surface.gamma_multiply(0.6));
        }
        let text_y = rect.center().y - name.size().y / 2.0;
        let x = rect.left() + TAB_PAD;
        ui.painter()
            .galley(egui::pos2(x, text_y), name.clone(), color);
        if let Some(dot) = dot {
            let at = egui::pos2(x + name.size().x, rect.center().y - dot.size().y / 2.0);
            ui.painter().galley(at, dot, t.accent_cut);
        }
        if active || hovered {
            if close.hovered() {
                ui.painter().rect_filled(close_rect, radius, t.surface);
            }
            let tex = icons::texture(ui.ctx(), Icon::Close, 12.0);
            egui::Image::from_texture(egui::load::SizedTexture::new(
                tex.id(),
                egui::Vec2::splat(12.0),
            ))
            .tint(color)
            .paint_at(
                ui,
                egui::Rect::from_center_size(close_rect.center(), egui::Vec2::splat(12.0)),
            );
        }
    }
    let hint = if tab.dirty { TAB_DIRTY_HINT } else { TAB_HINT };
    card(&resp, &label, hint, None, false);
    card(&close, &close_name, CLOSE_HINT, None, false);
    (resp, close)
}

/// The document tabs, the "»" list when they do not all fit, and the New
/// document button.
fn doc_tabs(
    ui: &mut Ui,
    t: &Tokens,
    tabs: &[Tab],
    active: u64,
    actions: &mut Vec<Action>,
    hot: &mut Vec<egui::Rect>,
) {
    let pad = ui.spacing().button_padding.x;
    let gap = ui.spacing().item_spacing.x;
    let icon_w = ICON_SIZE + 2.0 * pad;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let width = |text: String| {
        ui.painter()
            .layout_no_wrap(text, font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    let dot_w = width(format!(" {DIRTY}"));
    // A tab's name, plus its dot, is what narrows; the rest is fixed.
    let natural: Vec<f32> = tabs
        .iter()
        .map(|tab| width(tab.name.clone()) + if tab.dirty { dot_w } else { 0.0 })
        .collect();
    let more_w = width("»".into()) + 2.0 * pad + gap;
    // Room for the tabs: all that is left but the New document button.
    let room = ui.available_width() - icon_w - gap;
    let active_ix = tabs.iter().position(|t| t.id == active).unwrap_or(0);
    let fit = fit_tabs(&natural, tab_extra() + gap, room, more_w, active_ix);
    for &i in &fit.shown {
        let tab = &tabs[i];
        let cap = fit.cap.map(|c| c - if tab.dirty { dot_w } else { 0.0 });
        let (r, close) = doc_tab(ui, t, tab, tab.id == active, cap);
        hot.push(r.rect);
        if r.clicked() && tab.id != active {
            actions.push(Action::Switch(tab.id));
        }
        if close.clicked() {
            actions.push(Action::Close(tab.id));
        }
    }
    if fit.more {
        let resp = ui.add(egui::Button::new("»").frame_when_inactive(false));
        card(&resp, ALL_DOCUMENTS, ALL_HINT, None, false);
        set_name(ui, &resp, ALL_DOCUMENTS);
        hot.push(resp.rect);
        egui::Popup::menu(&resp).show(|ui| {
            for tab in tabs {
                let label = tab_label(&tab.name, tab.dirty);
                let r = ui.add(egui::Button::selectable(tab.id == active, label.as_str()));
                set_name(ui, &r, &format!("{label} (documents)"));
                if r.clicked() {
                    if tab.id != active {
                        actions.push(Action::Switch(tab.id));
                    }
                    ui.close();
                }
            }
        });
    }
    let plus = flat_icon(ui, Icon::Plus, "New document", NEW_HINT, None);
    hot.push(plus.rect);
    if plus.clicked() {
        actions.push(Action::NewDocument);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_double_click_setting_is_honoured() {
        assert_eq!(
            double_click_action(Some("Maximize"), false),
            DoubleClick::Zoom
        );
        assert_eq!(
            double_click_action(Some("Minimize"), false),
            DoubleClick::Minimize
        );
        assert_eq!(
            double_click_action(Some("None"), true),
            DoubleClick::Nothing
        );
        assert_eq!(double_click_action(None, true), DoubleClick::Minimize);
        assert_eq!(double_click_action(None, false), DoubleClick::Zoom);
    }

    #[test]
    fn tabs_narrow_first_then_move_into_the_list() {
        let natural = [100.0; 4];
        // 4 x (100 + 20) = 480 fits.
        assert_eq!(fit_tabs(&natural, 20.0, 480.0, 20.0, 0).cap, None);
        // Narrower names, all shown.
        let f = fit_tabs(&natural, 20.0, 400.0, 20.0, 0);
        assert!(!f.more && f.shown.len() == 4);
        assert!(f.cap.unwrap() >= MIN_TAB_LABEL && f.cap.unwrap() <= 80.0);
        // Too narrow even at the minimum: the list, and the active tab shown.
        let f = fit_tabs(&natural, 20.0, 250.0, 20.0, 3);
        assert!(f.more);
        assert!(f.shown.contains(&3));
        let used: f32 = f
            .shown
            .iter()
            .map(|&i| natural[i].min(f.cap.unwrap_or(100.0)) + 20.0)
            .sum();
        assert!(used + 20.0 <= 250.0, "{f:?}");
    }

    #[test]
    fn a_room_too_small_for_one_tab_still_shows_the_active_one_narrower() {
        let natural = [100.0; 12];
        // Not even one tab fits at the narrowest name (72 + 20 + 20 > 100).
        let f = fit_tabs(&natural, 20.0, 100.0, 20.0, 7);
        assert_eq!(f.shown, [7]);
        assert!(f.more);
        let cap = f.cap.expect("narrower than its name");
        assert!(cap < MIN_TAB_LABEL && cap + 20.0 + 20.0 <= 100.0, "{f:?}");
        // No tabs at all, and no room: nothing to show, no panic.
        assert_eq!(
            fit_tabs(&[], 20.0, -5.0, 20.0, 0).shown,
            Vec::<usize>::new()
        );
        // No room at all: the active tab with an empty name.
        let f = fit_tabs(&natural, 20.0, 10.0, 20.0, 3);
        assert_eq!(
            (f.shown.as_slice(), f.cap),
            ([3usize].as_slice(), Some(0.0))
        );
    }
}
