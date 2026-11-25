//! Home: the Start page. Start a new project, open a recent one, or try
//! a sample.

use egui::accesskit::Role;
use egui::{Color32, Rect, RichText, Sense, Ui, Vec2, WidgetInfo};

use super::icons::{self, Icon};
use super::theme::Tokens;
use std::path::{Path, PathBuf};

use super::{Action, FlatCraftApp, fonts, groups, lock, project_name};

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

// ---------------------------------------------------------------- recent

/// How many recent projects show before "Show all".
pub const SHOWN_RECENT: usize = 8;
/// The larger side of a project picture, in pixels.
pub const THUMB_PX: u32 = 128;
/// The height of a recent row.
pub const ROW_HEIGHT: f32 = 60.0;
/// The side of the picture box in a recent row.
const THUMB_BOX: f32 = 44.0;

/// Draws the picture of a project file (a PNG), on a worker thread.
pub type Thumbnailer = std::sync::Arc<
    dyn Fn(&flatcraft_device::ProfileStore, &Path) -> Result<Vec<u8>, String> + Send + Sync,
>;

/// The default [`Thumbnailer`]: the project loaded and rebuilt, drawn by
/// the preview renderer the agents use (canvas view).
pub fn render_project(
    profiles: &flatcraft_device::ProfileStore,
    path: &Path,
) -> Result<Vec<u8>, String> {
    let doc = flatcraft_engine::commands::project::load_rebuilt(profiles, path)
        .map_err(|e| e.to_string())?;
    let plotter = profiles
        .plotter(&doc.job.plotter_id)
        .map_err(|e| e.to_string())?;
    flatcraft_engine::preview::render_document_png(&plotter, &doc, THUMB_PX)
        .map_err(|e| e.to_string())
}

/// A picture on the page.
#[derive(Clone)]
pub enum Thumb {
    /// A worker draws it.
    Pending,
    Ready(egui::TextureHandle),
    /// It could not be drawn (a broken file): the row shows a plain tile.
    Failed,
}

impl std::fmt::Debug for Thumb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Thumb::Pending => "Pending",
            Thumb::Ready(_) => "Ready",
            Thumb::Failed => "Failed",
        })
    }
}

/// A picture a worker finished: id, version and the PNG or the reason.
type Done = (String, String, Result<Vec<u8>, String>);

/// What Home keeps between frames.
#[derive(Debug, Default)]
pub struct HomeState {
    /// "Show all" is open.
    pub show_all: bool,
    /// Pictures by id, with the version they show.
    pub thumbs: std::collections::HashMap<String, (String, Thumb)>,
    /// The worker that draws pictures, if one runs.
    pub job: Option<std::sync::mpsc::Receiver<Done>>,
    /// How many pictures the worker still draws.
    pub drawing: usize,
    /// The project and workspace last noted for the recent list.
    pub last_seen: Option<(PathBuf, super::Workspace)>,
}

impl HomeState {
    /// Drops the picture of `id`, so the next frame reads the cache.
    pub fn forget_texture(&mut self, id: &str) {
        self.thumbs.remove(id);
    }

    /// The worker still draws pictures.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Takes the pictures the worker finished.
    fn collect(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.job else {
            return;
        };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok((id, version, result)) => {
                    self.drawing = self.drawing.saturating_sub(1);
                    let thumb = match result.as_deref().map(|png| texture(ctx, &id, png)) {
                        Ok(Some(tex)) => Thumb::Ready(tex),
                        _ => Thumb::Failed,
                    };
                    self.thumbs.insert(id, (version, thumb));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            self.job = None;
            self.drawing = 0;
            // Anything still pending was never sent: try again later.
            self.thumbs.retain(|_, (_, t)| !matches!(t, Thumb::Pending));
        }
    }
}

/// A PNG as a texture, `None` when it does not decode.
fn texture(ctx: &egui::Context, id: &str, png: &[u8]) -> Option<egui::TextureHandle> {
    let p = tiny_skia::Pixmap::decode_png(png).ok()?;
    let image = egui::ColorImage::from_rgba_premultiplied(
        [p.width() as usize, p.height() as usize],
        p.data(),
    );
    Some(ctx.load_texture(
        format!("flatcraft-thumb-{id}"),
        image,
        egui::TextureOptions::LINEAR,
    ))
}

/// The picture of `path` at `version`: from memory, from the cache, or
/// queued for the worker (`missing`).
fn thumb_for(
    app: &mut FlatCraftApp,
    ctx: &egui::Context,
    path: &Path,
    version: &str,
    missing: &mut Vec<(String, String, PathBuf)>,
) -> Thumb {
    let id = flatcraft_engine::thumbnails::project_id(path);
    if let Some((v, t)) = app.ui.home.thumbs.get(&id)
        && v == version
    {
        return t.clone();
    }
    let thumb = match flatcraft_engine::thumbnails::cached(&app.config_dir, &id, version)
        .and_then(|png| texture(ctx, &id, &png))
    {
        Some(tex) => Thumb::Ready(tex),
        None if app.ui.home.busy() => return Thumb::Pending,
        None => {
            missing.push((id.clone(), version.to_string(), path.to_path_buf()));
            Thumb::Pending
        }
    };
    app.ui
        .home
        .thumbs
        .insert(id, (version.to_string(), thumb.clone()));
    thumb
}

/// Starts a worker that draws `missing` and stores each picture in the
/// cache.
fn start_worker(
    app: &mut FlatCraftApp,
    ctx: &egui::Context,
    missing: Vec<(String, String, PathBuf)>,
) {
    if missing.is_empty() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let profiles = lock(&app.session).profiles.clone();
    let draw = app.thumbnailer.clone();
    let config = app.config_dir.clone();
    let repaint = ctx.clone();
    app.ui.home.drawing = missing.len();
    app.ui.home.job = Some(rx);
    std::thread::spawn(move || {
        for (id, version, path) in missing {
            let result = draw(&profiles, &path);
            if let Ok(png) = &result
                && let Err(e) = flatcraft_engine::thumbnails::store(&config, &id, &version, png)
            {
                eprintln!(
                    "warning: cannot store the picture of {}: {e}",
                    path.display()
                );
            }
            if tx.send((id, version, result)).is_err() {
                return;
            }
            repaint.request_repaint();
        }
        drop(tx);
        repaint.request_repaint();
    });
}

/// "2 hours ago": the time between `then` and `now` (seconds since 1970)
/// in plain words. Empty when `then` is unknown (0).
pub fn ago(now: u64, then: u64) -> String {
    if then == 0 {
        return String::new();
    }
    let s = now.saturating_sub(then);
    let n = |v: u64, one: &str, many: &str| {
        if v == 1 {
            format!("1 {one} ago")
        } else {
            format!("{v} {many} ago")
        }
    };
    match s {
        0..60 => "Just now".into(),
        60..3600 => n(s / 60, "minute", "minutes"),
        3600..86_400 => n(s / 3600, "hour", "hours"),
        86_400..172_800 => "Yesterday".into(),
        172_800..1_209_600 => n(s / 86_400, "day", "days"),
        1_209_600..5_184_000 => n(s / 604_800, "week", "weeks"),
        5_184_000..31_536_000 => n(s / 2_592_000, "month", "months"),
        _ => "Over a year ago".into(),
    }
}

/// The folder of `path`, with the home folder shown as "~".
pub fn short_folder(path: &Path, home: Option<&Path>) -> String {
    let dir = path.parent().unwrap_or(path);
    if let Some(home) = home
        && let Ok(rest) = dir.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", rest.display())
        };
    }
    dir.display().to_string()
}

/// One recent project, as a row shows it.
struct Row {
    path: PathBuf,
    /// The file's version; `None` when it is missing.
    version: Option<String>,
    /// The open tab that shows it.
    open: Option<u64>,
    opened: u64,
    workspace: Option<super::Workspace>,
}

/// The recent projects, newest first.
fn recent(ui: &mut Ui, app: &mut FlatCraftApp, t: &Tokens, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    app.ui.home.collect(&ctx);
    let rows: Vec<Row> = {
        let s = lock(&app.session);
        let open: Vec<(PathBuf, u64)> = s
            .docs
            .iter()
            .filter_map(|d| {
                let p = d.path.as_deref()?;
                Some((
                    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()),
                    d.id(),
                ))
            })
            .collect();
        s.prefs
            .recent_projects
            .iter()
            .map(|path| {
                let abs = std::path::absolute(path).unwrap_or_else(|_| path.clone());
                let detail = s.prefs.recent_detail(path);
                Row {
                    version: None,
                    open: open.iter().find(|(p, _)| *p == abs).map(|(_, id)| *id),
                    opened: detail.map_or(0, |d| d.opened_unix_s),
                    workspace: detail.and_then(|d| match d.workspace.as_str() {
                        "Prepare" => Some(super::Workspace::Prepare),
                        "Cut" => Some(super::Workspace::Cut),
                        _ => None,
                    }),
                    path: path.clone(),
                }
            })
            .collect()
    };
    if rows.is_empty() {
        ui.label(
            RichText::new("No recent projects yet. Save one and it shows up here.")
                .color(t.text_dim),
        );
        return;
    }
    let total = rows.len();
    let shown = if app.ui.home.show_all {
        total
    } else {
        total.min(SHOWN_RECENT)
    };
    let now = (app.wall_clock)()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let home_dir = std::env::home_dir();
    let mut missing = Vec::new();
    let width = ui.available_width();
    egui::Frame::new()
        .fill(t.panel)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(t.ribbon_radius)
        .show(ui, |ui| {
            ui.set_width(width - 2.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, mut row) in rows.into_iter().take(shown).enumerate() {
                if i > 0 {
                    let y = ui.cursor().top();
                    let x = ui.max_rect().x_range();
                    ui.painter().hline(x, y, egui::Stroke::new(1.0, t.border));
                }
                row.version = flatcraft_engine::thumbnails::project_version(&row.path);
                let thumb = row
                    .version
                    .clone()
                    .map(|v| thumb_for(app, &ctx, &row.path, &v, &mut missing));
                recent_row(ui, t, &row, thumb, now, home_dir.as_deref(), actions);
            }
        });
    start_worker(app, &ctx, missing);
    if total > SHOWN_RECENT {
        ui.add_space(8.0);
        let (label, hint) = if app.ui.home.show_all {
            (
                "Show fewer".to_string(),
                format!("Shows only the {SHOWN_RECENT} newest projects again."),
            )
        } else {
            (
                format!("Show all ({total})"),
                format!("Lists the {} older projects too.", total - SHOWN_RECENT),
            )
        };
        let resp =
            ui.add(egui::Button::new(RichText::new(&label).color(t.accent_prepare)).frame(false));
        groups::card(&resp, &label, &hint, None, false);
        if resp.clicked() {
            app.ui.home.show_all = !app.ui.home.show_all;
        }
    }
}

/// One recent row: the picture, the name and folder, and when it was last
/// opened and in which workspace, or "Open now". A missing file offers
/// Locate and Remove from list instead. Right-click offers Remove from
/// list.
fn recent_row(
    ui: &mut Ui,
    t: &Tokens,
    row: &Row,
    thumb: Option<Thumb>,
    now: u64,
    home_dir: Option<&Path>,
    actions: &mut Vec<Action>,
) {
    let name = project_name(&row.path);
    let gone = row.version.is_none();
    let sense = if gone { Sense::hover() } else { Sense::click() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), sense);
    if !gone {
        // Not the bare name: an open project's tab has that one.
        let label = if row.open.is_some() {
            format!("Go to {name}")
        } else {
            format!("Open {name}")
        };
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
    }
    let hot = !gone && (resp.hovered() || resp.has_focus());
    let painter = ui.painter().clone();
    if hot {
        painter.rect_filled(rect.shrink(1.0), t.ribbon_radius, t.surface);
    }
    // The picture, fitted in its box on a tile.
    let tile = Rect::from_min_size(
        egui::pos2(rect.left() + 10.0, rect.center().y - THUMB_BOX / 2.0),
        Vec2::splat(THUMB_BOX),
    );
    painter.rect_filled(tile, t.ribbon_radius, t.surface);
    match thumb {
        Some(Thumb::Ready(tex)) => {
            let [w, h] = tex.size();
            let s = (THUMB_BOX - 4.0) / (w.max(h) as f32);
            let img = Rect::from_center_size(tile.center(), Vec2::new(w as f32 * s, h as f32 * s));
            painter.image(
                tex.id(),
                img,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            painter.rect_stroke(
                img,
                0,
                egui::Stroke::new(1.0, t.border),
                egui::StrokeKind::Outside,
            );
        }
        _ => {
            let icon = if gone {
                Icon::BrokenLink
            } else {
                Icon::SvgFile
            };
            let color = if gone { t.warn } else { t.text_dim };
            let tex = icons::texture_colored(ui.ctx(), icon, 20.0, color);
            painter.image(
                tex.id(),
                Rect::from_center_size(tile.center(), Vec2::splat(20.0)),
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }
    let x = tile.right() + 12.0;
    let right_room = if gone { 250.0 } else { 150.0 };
    let text_w = (rect.right() - right_room - x).max(60.0);
    let name_g = painter.layout_job(trunc(
        &name,
        fonts::semibold(14.0),
        if gone { t.text_dim } else { t.text },
        text_w,
    ));
    let (second, second_colour) = if gone {
        ("File moved or deleted".to_string(), t.warn)
    } else {
        (short_folder(&row.path, home_dir), t.text_dim)
    };
    let second_g = painter.layout_job(trunc(
        &second,
        egui::FontId::proportional(12.0),
        second_colour,
        text_w,
    ));
    let top = rect.center().y - (name_g.size().y + 2.0 + second_g.size().y) / 2.0;
    painter.galley(egui::pos2(x, top), name_g.clone(), t.text);
    painter.galley(
        egui::pos2(x, top + name_g.size().y + 2.0),
        second_g,
        second_colour,
    );

    if gone {
        let h = 24.0;
        let remove_w = 128.0;
        let locate_w = 82.0;
        let remove = Rect::from_min_size(
            egui::pos2(rect.right() - 12.0 - remove_w, rect.center().y - h / 2.0),
            Vec2::new(remove_w, h),
        );
        let locate = Rect::from_min_size(
            egui::pos2(remove.left() - 8.0 - locate_w, remove.top()),
            Vec2::new(locate_w, h),
        );
        let l = ui.place(locate, egui::Button::new("Locate..."));
        groups::card(
            &l,
            "Locate...",
            &format!(
                "{name} is no longer at {}. Show FlatCraft where it went, and the list points there from now on.",
                row.path.display()
            ),
            None,
            false,
        );
        if l.clicked() {
            actions.push(Action::LocateRecent(row.path.clone()));
        }
        let r = ui.place(remove, egui::Button::new("Remove from list"));
        groups::card(
            &r,
            "Remove from list",
            "Takes it off the recent list. No file is deleted.",
            None,
            false,
        );
        if r.clicked() {
            actions.push(Action::RemoveRecent(row.path.clone()));
        }
    } else {
        let right = rect.right() - 14.0;
        if let Some(doc) = row.open {
            let badge =
                painter.layout_no_wrap("Open now".into(), fonts::semibold(11.0), t.accent_prepare);
            let pad = Vec2::new(7.0, 2.0);
            let b = Rect::from_min_size(
                egui::pos2(
                    right - badge.size().x - 2.0 * pad.x,
                    rect.center().y - badge.size().y / 2.0 - pad.y,
                ),
                badge.size() + 2.0 * pad,
            );
            painter.rect_filled(b, t.ribbon_radius, t.accent_prepare.gamma_multiply(0.18));
            painter.galley(b.min + pad, badge, t.accent_prepare);
            if resp.clicked() {
                let w = row.workspace.unwrap_or(super::Workspace::Prepare);
                actions.push(Action::ShowOpen(doc, w));
            }
        } else {
            let when = ago(now, row.opened);
            let ws = row.workspace;
            let when_g =
                painter.layout_no_wrap(when.clone(), egui::FontId::proportional(12.0), t.text_dim);
            let ws_g = ws.map(|w| {
                painter.layout_no_wrap(
                    w.name().into(),
                    egui::FontId::proportional(12.0),
                    w.accent(t),
                )
            });
            let h = when_g.size().y + ws_g.as_ref().map_or(0.0, |g| g.size().y + 2.0);
            let top = rect.center().y - h / 2.0;
            painter.galley(
                egui::pos2(right - when_g.size().x, top),
                when_g.clone(),
                t.text_dim,
            );
            if let (Some(g), Some(w)) = (ws_g, ws) {
                let y = top + when_g.size().y + 2.0;
                let x = right - g.size().x;
                painter.circle_filled(egui::pos2(x - 7.0, y + g.size().y / 2.0), 3.0, w.accent(t));
                painter.galley(egui::pos2(x, y), g, w.accent(t));
            }
            if resp.clicked() {
                actions.push(Action::Open(row.path.clone()));
            }
        }
    }
    let hint = if gone {
        format!(
            "FlatCraft cannot find {name} at {}. Locate it or take it off the list. Right-click for more.",
            row.path.display()
        )
    } else if row.open.is_some() {
        format!(
            "{name} is open in a tab. Click to go there. Right-click to take it off the list.\n{}",
            row.path.display()
        )
    } else {
        format!(
            "Opens {name} in a new tab. Right-click to take it off the list.\n{}",
            row.path.display()
        )
    };
    groups::card(&resp, &name, &hint, None, false);
    resp.context_menu(|ui| {
        let r = ui.button("Remove from list");
        groups::card(
            &r,
            "Remove from list",
            "Takes it off the recent list. No file is deleted.",
            None,
            false,
        );
        if r.clicked() {
            actions.push(Action::RemoveRecent(row.path.clone()));
            ui.close();
        }
    });
}

/// One line of text cut with "…" at `width`.
fn trunc(text: &str, font: egui::FontId, colour: Color32, width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, colour);
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    job
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
    fn times_read_in_plain_words() {
        let now = 1_000_000_000;
        assert_eq!(ago(now, 0), "");
        assert_eq!(ago(now, now - 5), "Just now");
        assert_eq!(ago(now, now - 60), "1 minute ago");
        assert_eq!(ago(now, now - 7200), "2 hours ago");
        assert_eq!(ago(now, now - 90_000), "Yesterday");
        assert_eq!(ago(now, now - 3 * 86_400), "3 days ago");
        assert_eq!(ago(now, now - 15 * 86_400), "2 weeks ago");
        assert_eq!(ago(now, now - 90 * 86_400), "3 months ago");
        assert_eq!(ago(now, now - 400 * 86_400), "Over a year ago");
        // A clock that went back is "Just now", not a panic.
        assert_eq!(ago(now, now + 50), "Just now");
    }

    #[test]
    fn folders_shorten_the_home_folder() {
        let home = Path::new("/Users/maker");
        assert_eq!(
            short_folder(Path::new("/Users/maker/Crafts/a.flatcraft"), Some(home)),
            "~/Crafts"
        );
        assert_eq!(
            short_folder(Path::new("/Users/maker/a.flatcraft"), Some(home)),
            "~"
        );
        assert_eq!(
            short_folder(Path::new("/Volumes/USB/a.flatcraft"), Some(home)),
            "/Volumes/USB"
        );
        assert_eq!(
            short_folder(Path::new("/Users/makerspace/a.flatcraft"), Some(home)),
            "/Users/makerspace"
        );
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
