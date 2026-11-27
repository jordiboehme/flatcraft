//! The workspace switcher: a narrow tile left of the ribbon, as tall as the
//! ribbon block. Its popup lists the workspaces, one row each.

use egui::accesskit::Role;
use egui::{Align2, Color32, CornerRadius, Key, Modifiers, Rect, Sense, Ui, Vec2, WidgetInfo};

use super::icons::{self, Icon};
use super::prepare::RibbonTab;
use super::theme::Tokens;
use super::{Action, FlatCraftApp, Workspace, cut, groups, metrics, ribbon as ribbon_tabs};

/// The workspaces the popup offers.
pub const CHOICES: [Workspace; 2] = [Workspace::Prepare, Workspace::Cut];

// The tile's layout values live in `metrics`.
pub use super::metrics::TILE_HEIGHT;
use super::metrics::TILE_PAD;
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
            left: metrics::SPACE_8 as i8,
            right: metrics::SPACE_8 as i8,
            top: (metrics::SPACE_4) as i8,
            bottom: (metrics::SPACE_4) as i8,
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
                    (Some(snap), Workspace::Cut) => {
                        // The same panel as the ribbon's, without tabs, in
                        // the place of the ribbon panel.
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            egui::Frame::new()
                                .fill(t.panel)
                                .stroke(egui::Stroke::new(1.0, t.border))
                                .corner_radius(t.ribbon_radius)
                                .inner_margin(egui::Margin::symmetric(
                                    ribbon_tabs::PANEL_PAD.x as i8,
                                    ribbon_tabs::PANEL_PAD.y as i8,
                                ))
                                .show(ui, |ui| {
                                    // The panel's height, less its padding
                                    // and its 1 pt border.
                                    let inner = TILE_HEIGHT - 2.0 * ribbon_tabs::PANEL_PAD.y - 2.0;
                                    debug_assert_eq!(
                                        (inner - cut::BAR_HEIGHT) / 2.0,
                                        cut_bar_offset()
                                    );
                                    ui.set_min_height(inner);
                                    // The stage bar sits in the middle of the
                                    // panel's height.
                                    let room = inner - cut::BAR_HEIGHT;
                                    ui.add_space(room / 2.0);
                                    super::cut::stage_bar(
                                        ui,
                                        &mut app.ui.cut,
                                        snap,
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
        ui.spacing_mut().item_spacing.y = metrics::SPACE_4;
        ui.label(
            egui::RichText::new("WORKSPACE")
                .font(metrics::caption())
                .color(t.text_dim),
        );
        // The first frame only opens: the Enter that opened the list must
        // not also pick from it.
        let fresh = ui.data(|d| d.get_temp::<usize>(cursor_id)).is_none();
        let mut cursor = ui
            .data(|d| d.get_temp::<usize>(cursor_id))
            .unwrap_or_else(|| {
                // Start on the row under the pointer, else on the current one.
                let top = ui.cursor().min;
                let pointer = ui.input(|i| i.pointer.hover_pos());
                (0..CHOICES.len())
                    .find(|i| {
                        let y = top.y + *i as f32 * (ROW_HEIGHT + ui.spacing().item_spacing.y);
                        let r = Rect::from_min_size(
                            egui::pos2(top.x, y),
                            Vec2::new(LIST_WIDTH, ROW_HEIGHT),
                        );
                        pointer.is_some_and(|p| r.contains(p))
                    })
                    .or_else(|| CHOICES.iter().position(|w| *w == current))
                    .unwrap_or(0)
            });
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
        ui.add_space(metrics::SPACE_2);
        ui.separator();
        ui.add(
            egui::Label::new(
                egui::RichText::new(FOOTER)
                    .font(metrics::small())
                    .color(t.text_dim),
            )
            .wrap(),
        );
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
///
/// The foot is drawn in the footers' Small style now, but the width is
/// still measured with the 10.5 pt "Workspace" it had before: a narrower
/// tile would move every group of the ribbon and the Cut bar to the left.
/// Change it only together with the ribbon's look.
pub fn tile_width(ui: &Ui, w: Workspace) -> f32 {
    let name = ui.fonts_mut(|f| {
        f.layout_no_wrap(w.name().to_owned(), metrics::heading(), Color32::WHITE)
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

/// The room above the Cut stage bar inside its panel: the panel is as
/// tall as the tile, the bar sits in its middle.
pub fn cut_bar_offset() -> f32 {
    let inner = TILE_HEIGHT - 2.0 * metrics::RIBBON_PAD.y - 2.0;
    (inner - cut::BAR_HEIGHT) / 2.0
}

/// The top of the tile's bottom line ("Workspace" and its chevron) in a
/// tile whose top is `top`: on the baseline of the text beside it. In
/// Prepare that is the group footers ("Add", "Edit"), in Cut the steps'
/// status lines.
pub fn bottom_line_top(ui: &Ui, w: Workspace, top: f32) -> f32 {
    match w {
        Workspace::Cut => {
            // The stage panel's border, its padding, then the bar.
            let bar_top = top + 1.0 + metrics::RIBBON_PAD.y + cut_bar_offset();
            groups::step_detail_top(ui, bar_top + cut::BAR_HEIGHT / 2.0)
        }
        _ => {
            let footer = top
                + metrics::TAB_ROW
                + metrics::RIBBON_PAD.y
                + metrics::BIG_HEIGHT
                + metrics::GROUP_GAP
                + metrics::GROUP_FOOTER / 2.0;
            groups::footer_text_top(ui, footer)
        }
    }
}

/// The closed switcher: a colour bar on top, the icon and the name in the
/// middle, and "Workspace" with a chevron at the foot, on the line of the
/// group footers beside it and drawn like them.
fn tile(ui: &mut Ui, t: &Tokens, w: Workspace) -> egui::Response {
    let size = Vec2::new(tile_width(ui, w), TILE_HEIGHT);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let accent = w.accent(t);
    let radius = t.ribbon_radius;
    let lit = resp.hovered() || resp.has_focus();
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
    let bar = 3.0;
    painter.rect_filled(
        Rect::from_min_size(rect.min, Vec2::new(rect.width(), bar)),
        CornerRadius {
            nw: radius,
            ne: radius,
            sw: 0,
            se: 0,
        },
        accent,
    );
    // The foot, like a group footer: Small text and a chevron, weak until
    // hovered.
    let colour = if lit {
        ui.style().interact(&resp).text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let galley = ui.painter().layout_no_wrap(
        "Workspace".to_owned(),
        egui::TextStyle::Small.resolve(ui.style()),
        colour,
    );
    let sub_y = bottom_line_top(ui, w, rect.top());
    // The icon and the name, centred in the room above the foot.
    let icon_size = metrics::ICON_BIG;
    let gap = 5.0;
    let name = ui
        .painter()
        .layout_no_wrap(w.name().to_owned(), metrics::heading(), t.text);
    let content = icon_size + gap + name.size().y;
    let top = rect.top() + bar + (sub_y - rect.top() - bar - content) / 2.0;
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
    let name_y = top + icon_size + gap;
    painter.galley(
        egui::pos2(rect.center().x - name.size().x / 2.0, name_y),
        name,
        t.text,
    );
    let chevron = metrics::ICON_SMALL;
    let total = galley.size().x + 2.0 + chevron;
    let x0 = rect.center().x - total / 2.0;
    let line_h = galley.size().y;
    let text_w = galley.size().x;
    painter.galley(egui::pos2(x0, sub_y), galley, colour);
    groups::image(ui, Icon::ChevronDown, chevron).paint_at(
        ui,
        Rect::from_center_size(
            egui::pos2(x0 + text_w + 2.0 + chevron / 2.0, sub_y + line_h / 2.0),
            Vec2::splat(chevron),
        ),
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
        metrics::heading(),
        t.text,
    );
    if current {
        let badge = painter.layout_no_wrap("Current".to_owned(), metrics::caption(), accent);
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
        metrics::secondary(),
        t.text,
    );
    painter.text(
        egui::pos2(x, rect.top() + 51.0),
        Align2::LEFT_TOP,
        holds(w),
        metrics::small(),
        t.text_dim,
    );
    resp
}
