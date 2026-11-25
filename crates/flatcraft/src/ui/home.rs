//! Home: the Start page. Start a new project, open a recent one, or try
//! a sample.

use egui::accesskit::Role;
use egui::{Color32, Rect, RichText, Sense, Ui, Vec2, WidgetInfo};

use super::icons::{self, Icon};
use super::theme::Tokens;
use super::{Action, FlatCraftApp, display_name, fonts, groups, lock};

/// The widest the page gets; wider windows centre it.
pub const MAX_WIDTH: f32 = 1080.0;
/// Room left and right of the page.
pub const SIDE: f32 = 24.0;
/// Room between cards.
pub const GAP: f32 = 12.0;
/// The height of a New card.
pub const NEW_CARD_HEIGHT: f32 = 76.0;
/// A New card is never narrower than this while there are 4 columns.
const NEW_CARD_MIN: f32 = 230.0;

/// The page title and the line under it.
pub const TITLE: &str = "Start";
pub const SUBTITLE: &str = "Create a project, open a recent one, or try a sample.";

/// What a New card starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// An empty project on the default mat and plotter, in Prepare.
    Empty,
    /// Pick an SVG or an image, then import it into a new project.
    Import,
    /// Pick a project file and open it.
    Open,
    /// A new project with the test sheet on it.
    TestPatterns,
}

/// One card of the New row.
pub struct NewCard {
    pub start: Start,
    pub icon: Icon,
    /// The title, also the accessible name.
    pub title: &'static str,
    /// The line under the title.
    pub line: fn() -> String,
    /// The hover card.
    pub hint: &'static str,
}

fn open_line() -> String {
    let key = if cfg!(target_os = "macos") {
        "Cmd+O"
    } else {
        "Ctrl+O"
    };
    format!("A .flatcraft project ({key})")
}

/// The New row, in order.
pub const NEW_CARDS: [NewCard; 4] = [
    NewCard {
        start: Start::Empty,
        icon: Icon::NewFile,
        title: "New project",
        line: || "Your mat and plotter \u{b7} opens in Prepare".into(),
        hint: "Starts an empty project on your default mat, plotter and material, and opens Prepare. An empty start document is used for it, so no extra tab appears.",
    },
    NewCard {
        start: Start::Import,
        icon: Icon::Import,
        title: "Import design...",
        line: || "SVG, PNG, JPEG, WebP or GIF into a new project".into(),
        hint: "Pick a design file. It lands in a new project, ready to place. SVG shapes are cut as drawn; a picture (PNG, JPEG, WebP or GIF) is traced into cut lines first.",
    },
    NewCard {
        start: Start::Open,
        icon: Icon::FolderOpen,
        title: "Open...",
        line: open_line,
        hint: "Opens a FlatCraft project file you saved before. It opens in its own tab.",
    },
    NewCard {
        start: Start::TestPatterns,
        icon: Icon::TestPatterns,
        title: "Test patterns",
        line: || "Check scale, mirroring and your blade".into(),
        hint: "A new project with the FlatCraft test sheet: squares, circles, sharp tips, rulers and an F that shows mirroring. Cut it first on a new machine or blade, then measure. Trust, but verify.",
    },
];

/// How many cards of at least `min` points fit in a row `width` wide,
/// from `choices` (the first that fits, else the last).
pub fn columns(width: f32, min: f32, choices: &[usize]) -> usize {
    choices
        .iter()
        .copied()
        .find(|&n| n as f32 * min + (n as f32 - 1.0) * GAP <= width)
        .or_else(|| choices.last().copied())
        .unwrap_or(1)
        .max(1)
}

/// The Start page.
pub fn show(ui: &mut Ui, app: &mut FlatCraftApp, actions: &mut Vec<Action>) {
    let t = app.tokens;
    egui::ScrollArea::vertical()
        .id_salt("flatcraft-home")
        .auto_shrink(false)
        .show(ui, |ui| {
            let avail = ui.available_width();
            let width = (avail - 2.0 * SIDE).clamp(200.0, MAX_WIDTH);
            let left = ((avail - width) / 2.0).max(0.0);
            ui.horizontal_top(|ui| {
                ui.add_space(left);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.add_space(28.0);
                    page(ui, app, &t, width, actions);
                    ui.add_space(32.0);
                });
            });
        });
}

fn page(ui: &mut Ui, app: &mut FlatCraftApp, t: &Tokens, width: f32, actions: &mut Vec<Action>) {
    ui.label(
        RichText::new(TITLE)
            .font(fonts::semibold(26.0))
            .color(t.text),
    );
    ui.add_space(2.0);
    ui.label(RichText::new(SUBTITLE).size(14.0).color(t.text_dim));
    ui.add_space(24.0);

    heading(ui, t, "New");
    let cols = columns(width, NEW_CARD_MIN, &[4, 2, 1]);
    card_grid(
        ui,
        width,
        cols,
        NEW_CARD_HEIGHT,
        NEW_CARDS.len(),
        |ui, i, size| {
            let card = &NEW_CARDS[i];
            let resp = new_card(ui, t, card, size);
            groups::card(&resp, card.title, card.hint, None, false);
            if resp.clicked() {
                actions.push(Action::StartWith(card.start));
            }
        },
    );
    ui.add_space(28.0);

    heading(ui, t, "Recent");
    recent(ui, app, t, actions);
}

/// A section heading: small, dim capitals.
fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .font(fonts::semibold(11.0))
            .color(t.text_dim),
    );
    ui.add_space(8.0);
}

/// Lays out `n` cards in rows of `cols`, each `height` tall, filling
/// `width`.
pub fn card_grid(
    ui: &mut Ui,
    width: f32,
    cols: usize,
    height: f32,
    n: usize,
    mut card: impl FnMut(&mut Ui, usize, Vec2),
) {
    let w = ((width - (cols as f32 - 1.0) * GAP) / cols as f32).floor();
    for row in 0..n.div_ceil(cols) {
        if row > 0 {
            ui.add_space(GAP);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for i in row * cols..((row + 1) * cols).min(n) {
                card(ui, i, Vec2::new(w, height));
            }
        });
    }
}

/// The frame of a card: the panel colour, a thin border, 2 pt corners. A
/// hovered or focused card gets the surface colour and an accent border.
pub fn card_frame(ui: &Ui, t: &Tokens, rect: Rect, resp: &egui::Response) {
    let hot = resp.hovered() || resp.has_focus();
    let r = t.ribbon_radius;
    let fill = if hot { t.surface } else { t.panel };
    let stroke = if hot { t.accent_prepare } else { t.border };
    ui.painter().rect(
        rect,
        r,
        fill,
        egui::Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
}

/// One New card: the icon in a tile, the title and one line.
fn new_card(ui: &mut Ui, t: &Tokens, card: &NewCard, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, card.title));
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    card_frame(ui, t, rect, &resp);
    const TILE: f32 = 40.0;
    const ICON: f32 = 24.0;
    let tile = Rect::from_min_size(
        egui::pos2(rect.left() + 14.0, rect.center().y - TILE / 2.0),
        Vec2::splat(TILE),
    );
    ui.painter()
        .rect_filled(tile, t.ribbon_radius, t.accent_prepare.gamma_multiply(0.12));
    let tex = icons::texture(ui.ctx(), card.icon, ICON);
    ui.painter().image(
        tex.id(),
        Rect::from_center_size(tile.center(), Vec2::splat(ICON)),
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let x = tile.right() + 12.0;
    let text_w = (rect.right() - 12.0 - x).max(40.0);
    let painter = ui.painter();
    let title = painter.layout(
        card.title.to_string(),
        fonts::semibold(14.0),
        t.text,
        text_w,
    );
    let line = painter.layout(
        (card.line)(),
        egui::FontId::proportional(12.0),
        t.text_dim,
        text_w,
    );
    let total = title.size().y + 2.0 + line.size().y;
    let top = rect.center().y - total / 2.0;
    painter.galley(egui::pos2(x, top), title.clone(), t.text);
    painter.galley(egui::pos2(x, top + title.size().y + 2.0), line, t.text_dim);
    resp
}

/// The recent projects, newest first.
fn recent(ui: &mut Ui, app: &mut FlatCraftApp, t: &Tokens, actions: &mut Vec<Action>) {
    let recent = lock(&app.session).prefs.recent_projects.clone();
    if recent.is_empty() {
        ui.label(
            RichText::new("No recent projects yet. Save one and it shows up here.")
                .color(t.text_dim),
        );
        return;
    }
    for path in recent {
        let name = display_name(&path);
        let resp = ui.add(egui::Button::new(name).frame(false));
        groups::card(
            &resp,
            &display_name(&path),
            &format!("Opens {}.", path.display()),
            None,
            false,
        );
        if resp.clicked() {
            actions.push(Action::Open(path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_wrap_to_fewer_columns() {
        assert_eq!(columns(1352.0, NEW_CARD_MIN, &[4, 2, 1]), 4);
        assert_eq!(columns(852.0, NEW_CARD_MIN, &[4, 2, 1]), 2);
        assert_eq!(columns(300.0, NEW_CARD_MIN, &[4, 2, 1]), 1);
        assert_eq!(columns(100.0, NEW_CARD_MIN, &[4, 2, 1]), 1);
    }

    #[test]
    fn texts_are_plain() {
        let mut all = vec![TITLE.to_string(), SUBTITLE.to_string()];
        for c in &NEW_CARDS {
            all.extend([c.title.to_string(), (c.line)(), c.hint.to_string()]);
        }
        for s in all {
            assert!(!s.contains('\u{2014}'), "em-dash in {s}");
            assert!(
                !s.contains('\u{2318}'),
                "no glyph for the command key in {s}"
            );
        }
    }
}
