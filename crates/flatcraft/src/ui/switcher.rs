//! The workspace switcher: a narrow tile left of the ribbon, as tall as the
//! ribbon block. Its popup lists the workspaces, one row each.

use egui::accesskit::Role;
use egui::{Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Ui, Vec2, WidgetInfo};

use super::icons::{self, Icon};
use super::prepare::RibbonTab;
use super::theme::Tokens;
use super::{Action, FlatCraftApp, Workspace, cut, fonts, groups, ribbon as ribbon_tabs};

/// The workspaces the popup offers.
pub const CHOICES: [Workspace; 2] = [Workspace::Prepare, Workspace::Cut];

/// The height of the tile: the tab row and the ribbon panel.
pub const TILE_HEIGHT: f32 =
    ribbon_tabs::TAB_ROW + ribbon_tabs::PANEL_PAD.y + groups::ROW_HEIGHT + ribbon_tabs::PANEL_PAD.y;
/// Room between the tile's edge and its text.
const TILE_PAD: f32 = 14.0;
/// The width of the rows of the list.
const LIST_WIDTH: f32 = 380.0;
/// The height of one row of the list.
const ROW_HEIGHT: f32 = 76.0;

/// The one sentence of what a workspace is for.
pub fn purpose(w: Workspace) -> &'static str {
    match w {
        Workspace::Home => "Open a project or start a new one.",
        Workspace::Prepare => "Import, place and size your designs.",
        Workspace::Cut => "Choose plotter and material, check the path and send.",
    }
}

/// What a workspace holds: its ribbon tabs or its stages, joined with dots.
pub fn holds(w: Workspace) -> String {
    let names: Vec<&str> = match w {
        Workspace::Home => vec![],
        Workspace::Prepare => RibbonTab::ALL.iter().map(|t| t.name()).collect(),
        Workspace::Cut => cut::Stage::ALL.iter().map(|s| s.name()).collect(),
    };
    names.join(" \u{b7} ")
}

/// The note under the list.
pub const FOOTER: &str = "Both workspaces work on the same document. Only the tools change.";

/// The ribbon row: the switcher, then the workspace's tools.
pub fn ribbon(
    ui: &mut Ui,
    app: &mut FlatCraftApp,
    snap: Option<&super::prepare::Snapshot>,
    actions: &mut Vec<Action>,
) {
    let t = app.tokens;
    egui::Panel::top("flatcraft-ribbon")
        .frame(egui::Frame::new().fill(t.rail).inner_margin(egui::Margin {
            left: t.spacing as i8,
            right: t.spacing as i8,
            top: (t.spacing / 2.0) as i8,
            bottom: (t.spacing / 2.0) as i8,
        }))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                switcher(ui, app, actions);
                match (snap, app.ui.workspace) {
                    (Some(snap), Workspace::Prepare) => {
                        super::ribbon::show(
                            ui,
                            &mut app.ui.prepare,
                            snap,
                            &mut app.ui.show_debug,
                            actions,
                        );
                    }
                    (Some(_), Workspace::Cut) => {
                        // The same panel as the ribbon's, without tabs, in
                        // the place of the ribbon panel.
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add_space(ribbon_tabs::TAB_ROW);
                            egui::Frame::new()
                                .fill(t.panel)
                                .stroke(egui::Stroke::new(1.0, t.border))
                                .corner_radius(t.ribbon_radius)
                                .inner_margin(egui::Margin::symmetric(6, 4))
                                .show(ui, |ui| {
                                    ui.set_min_height(groups::ROW_HEIGHT - 2.0);
                                    // The stage bar sits in the middle of the
                                    // panel's height.
                                    let room =
                                        groups::ROW_HEIGHT - 2.0 - ui.spacing().interact_size.y;
                                    ui.add_space(room / 2.0);
                                    super::cut::stage_bar(
                                        ui,
                                        &mut app.ui.cut,
                                        &mut app.ui.show_debug,
                                        &t,
                                    );
                                });
                        });
                    }
                    _ => {}
                }
            });
        });
}

fn switcher(ui: &mut Ui, app: &FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let current = app.ui.workspace;
    let resp = tile(ui, &t, current);
    let hint = match current {
        Workspace::Home => "You are on Home. Pick Prepare or Cut to start working.".to_string(),
        w => format!(
            "You are in {}. Click to switch to the other workspace. Both work on the same document.",
            w.name()
        ),
    };
    groups::card(&resp, "Workspace", &hint, None, false);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Workspace"));

    let cursor_id = egui::Id::unique("flatcraft-workspace-cursor");
    let open = egui::Popup::menu(&resp).show(|ui| {
        ui.set_width(LIST_WIDTH);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(
            egui::RichText::new("WORKSPACE")
                .font(fonts::semibold(11.0))
                .color(t.text_dim),
        );
        // The first frame only opens: the Enter that opened the list must
        // not also pick from it.
        let fresh = ui.data(|d| d.get_temp::<usize>(cursor_id)).is_none();
        let mut cursor = ui
            .data(|d| d.get_temp::<usize>(cursor_id))
            .unwrap_or_else(|| CHOICES.iter().position(|w| *w == current).unwrap_or(0));
        let mut picked = None;
        if !fresh {
            let (down, up, enter) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                )
            });
            if down {
                cursor = (cursor + 1).min(CHOICES.len() - 1);
            }
            if up {
                cursor = cursor.saturating_sub(1);
            }
            if enter {
                picked = Some(CHOICES[cursor]);
            }
        }
        for (i, w) in CHOICES.into_iter().enumerate() {
            let r = row(ui, &t, w, w == current, i == cursor);
            r.widget_info(|| WidgetInfo::selected(Role::Button, true, w == current, w.name()));
            if r.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) {
                cursor = i;
            }
            if r.clicked() {
                picked = Some(w);
            }
        }
        ui.add_space(2.0);
        ui.separator();
        ui.add(egui::Label::new(egui::RichText::new(FOOTER).size(11.0).color(t.text_dim)).wrap());
        ui.data_mut(|d| d.insert_temp(cursor_id, cursor));
        if let Some(w) = picked {
            actions.push(Action::SetWorkspace(w));
            ui.close();
        }
    });
    if open.is_none() {
        ui.data_mut(|d| d.remove::<usize>(cursor_id));
    }
}

/// The width of the tile for the workspace `w`: its widest line and the
/// padding, nothing more.
pub fn tile_width(ui: &Ui, w: Workspace) -> f32 {
    let name = ui.fonts_mut(|f| {
        f.layout_no_wrap(w.name().to_owned(), fonts::semibold(14.0), Color32::WHITE)
            .size()
            .x
    });
    let sub = ui.fonts_mut(|f| {
        f.layout_no_wrap(
            "Workspace".to_owned(),
            egui::FontId::proportional(10.5),
            Color32::WHITE,
        )
        .size()
        .x
    });
    (name.max(sub + 14.0)).ceil() + 2.0 * TILE_PAD
}

/// The closed switcher: a colour bar on top, the icon, the name, and
/// "Workspace" with a chevron, dimmed.
fn tile(ui: &mut Ui, t: &Tokens, w: Workspace) -> egui::Response {
    let size = Vec2::new(tile_width(ui, w), TILE_HEIGHT);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let accent = w.accent(t);
    let radius = t.ribbon_radius;
    let fill = if resp.hovered() { t.surface } else { t.panel };
    let painter = ui.painter();
    painter.rect(
        rect,
        radius,
        fill,
        egui::Stroke::new(1.0, t.border),
        egui::StrokeKind::Inside,
    );
    // The colour bar on top.
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(rect.width(), 3.0)),
        CornerRadius {
            nw: radius,
            ne: radius,
            sw: 0,
            se: 0,
        },
        accent,
    );
    let icon_size = 24.0;
    let content = icon_size + 5.0 + 17.0 + 2.0 + 13.0;
    let top = rect.top() + 3.0 + (rect.height() - 3.0 - content) / 2.0;
    let tex = icons::texture_colored(ui.ctx(), w.icon(), icon_size, accent);
    let painter = ui.painter();
    painter.image(
        tex.id(),
        Rect::from_center_size(
            egui::pos2(rect.center().x, top + icon_size / 2.0),
            Vec2::splat(icon_size),
        ),
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let name_y = top + icon_size + 5.0;
    painter.text(
        egui::pos2(rect.center().x, name_y),
        Align2::CENTER_TOP,
        w.name(),
        fonts::semibold(14.0),
        t.text,
    );
    let sub_y = name_y + 17.0 + 2.0;
    let galley = painter.layout_no_wrap(
        "Workspace".to_owned(),
        egui::FontId::proportional(10.5),
        t.text_dim,
    );
    let chevron = 10.0;
    let total = galley.size().x + 2.0 + chevron;
    let x0 = rect.center().x - total / 2.0;
    painter.galley(egui::pos2(x0, sub_y), galley.clone(), t.text_dim);
    let ctex = icons::texture_colored(ui.ctx(), Icon::ChevronDown, chevron, t.text_dim);
    ui.painter().image(
        ctex.id(),
        Rect::from_center_size(
            egui::pos2(
                x0 + galley.size().x + 2.0 + chevron / 2.0,
                sub_y + galley.size().y / 2.0 + 0.5,
            ),
            Vec2::splat(chevron),
        ),
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    resp
}

/// One row of the list: a colour bar, the icon in a tile, the name with a
/// badge when it is the current one, what it is for and what it holds.
fn row(ui: &mut Ui, t: &Tokens, w: Workspace, current: bool, cursor: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(LIST_WIDTH, ROW_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let accent = w.accent(t);
    let r = t.radius;
    let fill = if cursor || resp.hovered() {
        t.surface
    } else if current {
        accent.gamma_multiply(0.08)
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, r, fill);
    if cursor {
        painter.rect_stroke(
            rect,
            r,
            egui::Stroke::new(1.0, accent),
            egui::StrokeKind::Inside,
        );
    }
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height())),
        CornerRadius {
            nw: r,
            sw: r,
            ne: 0,
            se: 0,
        },
        accent,
    );
    let tile_rect = Rect::from_min_size(
        egui::pos2(rect.left() + 16.0, rect.center().y - 18.0),
        Vec2::splat(36.0),
    );
    painter.rect_filled(tile_rect, r, accent.gamma_multiply(0.18));
    let tex = icons::texture_colored(ui.ctx(), w.icon(), 22.0, accent);
    let painter = ui.painter();
    painter.image(
        tex.id(),
        Rect::from_center_size(tile_rect.center(), Vec2::splat(22.0)),
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let x = tile_rect.right() + 12.0;
    let name = painter.text(
        egui::pos2(x, rect.top() + 10.0),
        Align2::LEFT_TOP,
        w.name(),
        fonts::semibold(14.0),
        t.text,
    );
    if current {
        let badge = painter.layout_no_wrap("Current".to_owned(), fonts::semibold(10.0), accent);
        let pad = Vec2::new(6.0, 1.0);
        let b = Rect::from_min_size(
            egui::pos2(
                name.right() + 8.0,
                name.center().y - badge.size().y / 2.0 - pad.y,
            ),
            badge.size() + 2.0 * pad,
        );
        painter.rect_filled(b, r, accent.gamma_multiply(0.2));
        painter.galley(b.min + pad, badge, accent);
    }
    painter.text(
        egui::pos2(x, rect.top() + 31.0),
        Align2::LEFT_TOP,
        purpose(w),
        egui::FontId::proportional(12.0),
        t.text,
    );
    painter.text(
        egui::pos2(x, rect.top() + 51.0),
        Align2::LEFT_TOP,
        holds(w),
        egui::FontId::proportional(11.0),
        t.text_dim,
    );
    resp
}
