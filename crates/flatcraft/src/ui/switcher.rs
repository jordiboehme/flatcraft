//! The workspace switcher: a card left of the ribbon. Its popup holds one
//! descriptive card per workspace.

use egui::accesskit::Role;
use egui::{Align2, Color32, Rect, Response, Sense, Ui, Vec2, WidgetInfo};

use super::icons::{self, Icon};
use super::theme::Tokens;
use super::{Action, FlatCraftApp, ICON_SIZE, Workspace, fonts};

/// The workspaces the popup offers, with their descriptions.
pub const CHOICES: [(Workspace, &str); 2] = [
    (Workspace::Prepare, "import, place and size your designs"),
    (Workspace::Cut, "plotter, path, send"),
];

/// The ribbon row: the switcher, then the workspace's tools.
pub fn ribbon(
    ui: &mut Ui,
    app: &mut FlatCraftApp,
    snap: Option<&super::prepare::Snapshot>,
    actions: &mut Vec<Action>,
) {
    let t = app.tokens;
    egui::Panel::top("flatcraft-ribbon")
        .frame(
            egui::Frame::new()
                .fill(t.panel)
                .inner_margin(egui::Margin::same(t.spacing as i8)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                switcher(ui, app, actions);
                match (snap, app.ui.workspace) {
                    (Some(snap), Workspace::Prepare) => {
                        ui.separator();
                        super::ribbon::show(
                            ui,
                            &mut app.ui.prepare,
                            snap,
                            &mut app.ui.show_debug,
                            actions,
                        );
                    }
                    (Some(_), Workspace::Cut) => {
                        ui.separator();
                        super::cut::stage_bar(ui, &mut app.ui.cut, &mut app.ui.show_debug, &t);
                    }
                    _ => {}
                }
            });
        });
}

fn switcher(ui: &mut Ui, app: &FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    let current = app.ui.workspace;
    let resp = card(
        ui,
        &t,
        current.icon(),
        current.name(),
        "Workspace",
        current.accent(&t),
        Vec2::new(180.0, 40.0),
    )
    .on_hover_text("Switch workspace");
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, "Workspace"));

    egui::Popup::menu(&resp).show(|ui| {
        for (w, description) in CHOICES {
            let r = card(
                ui,
                &t,
                w.icon(),
                w.name(),
                description,
                w.accent(&t),
                Vec2::new(340.0, 56.0),
            )
            .on_hover_text(format!("{} - {description}", w.name()));
            r.widget_info(|| WidgetInfo::selected(Role::Button, true, w == current, w.name()));
            if r.clicked() {
                actions.push(Action::SetWorkspace(w));
                ui.close();
            }
        }
    });
}

/// A clickable card with an icon, a title and a dim second line. The caller
/// sets its accessible name with `widget_info`.
fn card(
    ui: &mut Ui,
    t: &Tokens,
    icon: Icon,
    title: &str,
    subtitle: &str,
    accent: Color32,
    size: Vec2,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if resp.hovered() {
            t.surface.gamma_multiply(1.4)
        } else {
            t.surface
        };
        painter.rect(
            rect,
            t.radius,
            fill,
            egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        let pad = t.spacing;
        let tex = icons::texture_colored(ui.ctx(), icon, ICON_SIZE, accent);
        let icon_rect = Rect::from_center_size(
            egui::pos2(rect.left() + pad + ICON_SIZE / 2.0, rect.center().y),
            Vec2::splat(ICON_SIZE),
        );
        painter.image(
            tex.id(),
            icon_rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let x = icon_rect.right() + pad;
        painter.text(
            egui::pos2(x, rect.center().y - 1.0),
            Align2::LEFT_BOTTOM,
            title,
            fonts::semibold(14.0),
            accent,
        );
        painter.text(
            egui::pos2(x, rect.center().y + 1.0),
            Align2::LEFT_TOP,
            subtitle,
            egui::FontId::proportional(12.0),
            t.text_dim,
        );
    }
    resp
}
