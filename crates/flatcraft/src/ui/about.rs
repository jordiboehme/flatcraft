//! The About dialog: banner, version, release motto, tagline, support
//! links, credits, a maker quote and the third-party licences. It also
//! holds two easter eggs: the maker quote changes on every opening, and
//! five clicks on the banner send a simulated knife along its dotted travel
//! path.

use std::time::{Duration, Instant};

use egui::{Color32, CornerRadius, Pos2, RichText, Sense, TextureHandle, Ui, Vec2};
use flatcraft_core::document::ObjectId;
use flatcraft_path::{PathRef, Toolpath};
use flatcraft_sim::{SimSettings, simulate};
use kurbo::Point;

use super::icons::{self, Icon};
use super::rng::XorShift;
use super::theme::Tokens;
use super::{Action, fonts};

/// The release motto under the version. It is set per release.
pub const MOTTO: &str = "Super massive stability stuff";

pub const TAGLINE: &str = "Your cutting plotter, leveled up.";

/// The same words as the README under its title.
pub const PITCH: &str = "You bought it, so you're in control: every setting, every cut, on your own computer. Pro results for every plotter. Agents welcome.";

/// [`PITCH`] as the two lines the dialog draws.
pub const PITCH_LINES: [&str; 2] = [
    "You bought it, so you're in control: every setting, every cut, on your own computer.",
    "Pro results for every plotter. Agents welcome.",
];

pub const KOFI_URL: &str = "https://ko-fi.com/V7V31T6CL9";
pub const PAYPAL_URL: &str = "https://paypal.me/jordiboehmelopez";
pub const GITHUB_URL: &str = "https://github.com/jordiboehme/flatcraft";

pub const BUILT_WITH: &str =
    "Built with egui, kurbo, usvg, visioncortex and i_overlay. Set in IBM Plex, icons by Lucide.";
pub const INSPIRED_BY: &str = "Inspired by PrusaSlicer, SolidCraft and ZigZag.";
/// The credit line after the heart: "Made with ❤️ for fellow makers by ...".
pub const MADE_WITH: &str = "Made with";
pub const MADE_FOR: &str = "for fellow makers by Jordi Böhme · MPL-2.0 ·";
pub const LICENSES: &str = "Licenses";

/// The maker quotes, word for word, with their authors. One shows per
/// opening, never the same one twice in a row.
pub const QUOTES: [(&str, &str); 5] = [
    ("Every defect gets respect.", "Laura Kampf"),
    ("Failure is always an option.", "Adam Savage"),
    ("Super massive stability stuff.", "Felix Schelhasse"),
    (
        "Maybe a toothbrush helmet isn't the answer, but at least you're asking the question.",
        "Simone Giertz, TED 2018",
    ),
    (
        "You want to make something that the very next morning the minute you wake up you want to run to your shop and look at it.",
        "Jimmy DiResta, Making It interview 2017",
    ),
];

/// The third-party crates in the app and their licences, kept in step with
/// `cargo metadata` by the `licenses` test.
pub const THIRD_PARTY: &str = include_str!("../../../../assets/licenses/THIRD-PARTY.txt");

/// The build id: `FLATCRAFT_BUILD` at compile time (a release build sets
/// it), else "dev".
pub const BUILD: &str = match option_env!("FLATCRAFT_BUILD") {
    Some(b) => b,
    None => "dev",
};

/// "Version x.y.z (build)".
pub fn version_line() -> String {
    format!("Version {} ({BUILD})", env!("CARGO_PKG_VERSION"))
}

/// Shown on the banner once the knife is through.
pub const END_OF_LINE: &str = "End of line.";
/// The accessible name of the banner.
pub const BANNER_NAME: &str = "FlatCraft banner";
/// Clicks on the banner that start the knife.
pub const BANNER_CLICKS: u32 = 5;
/// How long the knife takes along the path.
pub const KNIFE_TIME: Duration = Duration::from_secs(3);

/// The banner image (2560 x 640 pixels, drawn on a 1280 x 320 grid).
const BANNER_PNG: &[u8] = include_bytes!("../../../../assets/branding/banner.png");
/// The banner's own coordinate grid (its HTML source).
const BANNER_GRID: Vec2 = Vec2::new(1280.0, 320.0);
/// The banner's dotted travel path, in banner units.
const BANNER_PATH: [(f64, f64); 5] = [
    (690.0, 300.0),
    (814.0, 190.0),
    (850.0, 140.0),
    (1030.0, 128.0),
    (1130.0, 240.0),
];
/// The banner's numbered badges 1 to 4. Badge k lights up when the knife
/// reaches point k of the path.
const BANNER_BADGES: [(f32, f32); 4] = [
    (728.0, 136.0),
    (890.0, 104.0),
    (970.0, 104.0),
    (1112.0, 104.0),
];
/// Banner units per simulated millimetre: the path is about 56 mm long.
const UNITS_PER_MM: f64 = 10.0;
/// Every this many millimetres of the simulated tip, one trail point.
const TRAIL_STEP_MM: f64 = 0.4;
/// The support buttons' colour (Ko-fi and PayPal).
const PINK: Color32 = Color32::from_rgb(0xE2, 0x4A, 0x8D);
const WIDTH: f32 = 560.0;

/// What the dialog shows.
#[derive(Debug, Default)]
pub struct AboutState {
    pub open: bool,
    /// The quote of this opening (index into [`QUOTES`]).
    pub quote: Option<usize>,
    /// The licence list is shown instead of the credits.
    pub licenses: bool,
    /// Banner clicks since the dialog opened or the knife last started.
    pub banner_clicks: u32,
    pub knife: Option<Knife>,
}

/// The banner knife: the simulated blade tip, in banner units.
#[derive(Debug, Clone)]
pub struct Knife {
    pub started: Instant,
    /// The tip along the path and how far the axis had travelled there, in mm.
    pub trail: Vec<(Pos2, f64)>,
    /// Where on the path (axis travel in mm) each badge lights up.
    pub badges: [f64; 4],
    pub length_mm: f64,
}

impl Knife {
    /// The share of the path done at `now`, 0 to 1.
    pub fn progress(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.started).as_secs_f32();
        (t / KNIFE_TIME.as_secs_f32()).min(1.0)
    }

    pub fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }
}

impl AboutState {
    /// Opens the dialog with a quote other than the last one.
    pub fn open(&mut self, rng: &mut XorShift) {
        let quote = match self.quote {
            Some(last) => (last + 1 + rng.below(QUOTES.len() - 1)) % QUOTES.len(),
            None => rng.below(QUOTES.len()),
        };
        *self = Self {
            open: true,
            quote: Some(quote),
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
        self.licenses = false;
        self.banner_clicks = 0;
        self.knife = None;
    }

    /// One click on the banner; the fifth starts the knife.
    pub fn click_banner(&mut self, now: Instant) {
        self.banner_clicks += 1;
        if self.banner_clicks >= BANNER_CLICKS {
            self.banner_clicks = 0;
            self.knife = Some(knife(now));
        }
    }
}

/// Simulates a 0.25 mm drag knife along the banner's dotted path.
pub fn knife(now: Instant) -> Knife {
    let points: Vec<Point> = BANNER_PATH
        .iter()
        .map(|&(x, y)| Point::new(x / UNITS_PER_MM, y / UNITS_PER_MM))
        .collect();
    let mut badges = [0.0; 4];
    let mut length_mm = 0.0;
    for (i, w) in points.windows(2).enumerate() {
        length_mm += w[0].distance(w[1]);
        badges[i] = length_mm;
    }
    let path = Toolpath::new(
        points.clone(),
        false,
        PathRef {
            object: ObjectId(0),
            path_index: 0,
        },
    );
    let trace = simulate(&[path], points[0], &SimSettings::float(0.25));
    let samples: Vec<_> = trace.iter().flat_map(|t| &t.samples).collect();
    let to_banner = |p: Point| Pos2::new((p.x * UNITS_PER_MM) as f32, (p.y * UNITS_PER_MM) as f32);
    let mut trail: Vec<(Pos2, f64)> = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        let last = i + 1 == samples.len();
        if !last
            && trail
                .last()
                .is_some_and(|&(_, t)| s.t_mm - t < TRAIL_STEP_MM)
        {
            continue;
        }
        trail.push((to_banner(s.tip), s.t_mm));
    }
    Knife {
        started: now,
        trail,
        badges,
        length_mm,
    }
}

/// The banner as a texture, decoded and halved once per context.
fn banner_texture(ctx: &egui::Context) -> TextureHandle {
    let id = egui::Id::unique("flatcraft-about-banner");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let image = match tiny_skia::Pixmap::decode_png(BANNER_PNG) {
        Ok(p) => halve(p.width() as usize, p.height() as usize, p.data()),
        // The PNG is built in, so this does not happen; a flat teal keeps
        // the layout.
        Err(_) => egui::ColorImage::filled([2, 1], Color32::from_rgb(0x0E, 0x35, 0x33)),
    };
    let tex = ctx.load_texture("flatcraft-banner", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Halves premultiplied RGBA with a 2x2 box filter, so the 4.5x shrink in
/// the dialog does not alias.
fn halve(w: usize, h: usize, rgba: &[u8]) -> egui::ColorImage {
    let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; hw * hh * 4];
    for y in 0..hh {
        for x in 0..hw {
            for c in 0..4 {
                let px = |dx: usize, dy: usize| {
                    let (sx, sy) = ((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1));
                    u32::from(rgba[(sy * w + sx) * 4 + c])
                };
                let sum = px(0, 0) + px(1, 0) + px(0, 1) + px(1, 1);
                out[(y * hw + x) * 4 + c] = ((sum + 2) / 4) as u8;
            }
        }
    }
    egui::ColorImage::from_rgba_premultiplied([hw, hh], &out)
}

/// Lays a row out centred: a row whose width changed is measured and the
/// frame is laid out again (`request_discard`), so it never shows off-centre.
fn centered_row(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    let key = ui.scope_id().with(id);
    let width: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        let start = ui.cursor().min.x;
        add(ui);
        let used = ui.min_rect().right() - start;
        if (used - width).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(key, used));
            // Lay the frame out again with the measured width.
            ui.ctx().request_discard("centred row measured");
        }
    });
}

/// Draws the dialog while it is open. `now` drives the knife.
pub fn show(
    ctx: &egui::Context,
    st: &mut AboutState,
    t: &Tokens,
    now: Instant,
    actions: &mut Vec<Action>,
) {
    if !st.open {
        return;
    }
    let frame = egui::Frame::popup(&ctx.global_style())
        .inner_margin(0)
        .fill(t.panel);
    let radius = frame.corner_radius;
    let modal = egui::Modal::new(egui::Id::unique("flatcraft-about"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            if st.licenses {
                licenses(ui, st, t);
            } else {
                banner(ui, st, t, radius, now);
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(24, 16))
                    .show(ui, |ui| credits(ui, st, t, actions));
            }
        });
    if modal.should_close() {
        st.close();
    }
}

fn banner(ui: &mut Ui, st: &mut AboutState, t: &Tokens, radius: CornerRadius, now: Instant) {
    let tex = banner_texture(ui.ctx());
    let size = Vec2::new(WIDTH, WIDTH * BANNER_GRID.y / BANNER_GRID.x);
    let top = CornerRadius {
        nw: radius.nw,
        ne: radius.ne,
        sw: 0,
        se: 0,
    };
    let resp = ui.add(
        egui::Image::from_texture(egui::load::SizedTexture::new(tex.id(), size))
            .corner_radius(top)
            .sense(Sense::click())
            .alt_text(BANNER_NAME),
    );
    if resp.clicked() {
        st.click_banner(now);
    }
    let Some(knife) = &st.knife else {
        return;
    };
    let rect = resp.rect;
    let to_screen =
        |p: Pos2| rect.min + Vec2::new(p.x / BANNER_GRID.x, p.y / BANNER_GRID.y) * rect.size();
    let progress = knife.progress(now);
    let at_mm = f64::from(progress) * knife.length_mm;
    let painter = ui.painter_at(rect);
    let trail: Vec<Pos2> = knife
        .trail
        .iter()
        .take_while(|(_, mm)| *mm <= at_mm)
        .map(|(p, _)| to_screen(*p))
        .collect();
    painter.add(egui::Shape::line(
        trail.clone(),
        egui::Stroke::new(2.0, t.accent_cut),
    ));
    for (badge, &(x, y)) in knife.badges.iter().zip(&BANNER_BADGES) {
        if *badge <= at_mm {
            painter.circle_stroke(
                to_screen(Pos2::new(x, y)),
                16.0 * rect.width() / BANNER_GRID.x,
                egui::Stroke::new(2.0, t.accent_cut),
            );
        }
    }
    if let Some(&tip) = trail.last() {
        painter.circle_filled(tip, 4.0, t.accent_cut);
        painter.circle_stroke(tip, 6.0, egui::Stroke::new(1.0, Color32::WHITE));
    }
    if knife.done(now) {
        let label = egui::Rect::from_center_size(
            rect.left_bottom() + Vec2::new(rect.width() * 0.25, -rect.height() * 0.15),
            Vec2::new(rect.width() * 0.4, 24.0),
        );
        ui.put(
            label,
            egui::Label::new(
                RichText::new(END_OF_LINE)
                    .font(fonts::medium(16.0))
                    .color(t.accent_cut),
            ),
        );
    } else {
        // Frames only while the knife moves.
        ui.ctx().request_repaint();
    }
}

fn credits(ui: &mut Ui, st: &mut AboutState, t: &Tokens, actions: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("FlatCraft").font(fonts::semibold(24.0)));
        ui.label(RichText::new(version_line()).color(t.text_dim));
        ui.label(RichText::new(MOTTO).italics().color(t.accent_cut));
        ui.add_space(t.spacing);
        ui.label(RichText::new(TAGLINE).font(fonts::semibold(16.0)));
        for line in PITCH_LINES {
            ui.label(line);
        }
        ui.add_space(t.spacing);
    });
    centered_row(ui, "buttons", |ui| {
        for (name, url) in [("Ko-fi", KOFI_URL), ("PayPal", PAYPAL_URL)] {
            let support = egui::Button::new(RichText::new(name).color(Color32::WHITE)).fill(PINK);
            if ui.add(support).clicked() {
                actions.push(Action::OpenUrl(url.to_string()));
            }
        }
        if ui.button("GitHub").clicked() {
            actions.push(Action::OpenUrl(GITHUB_URL.to_string()));
        }
    });
    ui.add_space(t.spacing);
    let small = |s: &str| RichText::new(s).size(12.0).color(t.text_dim);
    ui.vertical_centered(|ui| {
        ui.label(small(BUILT_WITH));
        ui.label(small(INSPIRED_BY));
    });
    centered_row(ui, "made-with", |ui| {
        ui.label(small(MADE_WITH));
        let heart = icons::texture_colored(ui.ctx(), Icon::Heart, 12.0, PINK);
        ui.add(
            egui::Image::from_texture(egui::load::SizedTexture::new(heart.id(), Vec2::splat(12.0)))
                .alt_text("❤️"),
        );
        ui.label(small(MADE_FOR));
        if ui.link(RichText::new(LICENSES).size(12.0)).clicked() {
            st.licenses = true;
        }
    });
    if let Some((quote, author)) = st.quote.and_then(|i| QUOTES.get(i)) {
        ui.add_space(t.spacing * 1.5);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(format!("\u{201c}{quote}\u{201d}")).italics());
            ui.label(RichText::new(format!("- {author}")).color(t.text_dim));
        });
    }
}

fn licenses(ui: &mut Ui, st: &mut AboutState, t: &Tokens) {
    egui::Frame::new()
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    st.licenses = false;
                }
                ui.label(RichText::new("Third-party licenses").font(fonts::semibold(16.0)));
            });
            ui.add_space(t.spacing);
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.label(RichText::new(THIRD_PARTY).monospace().size(11.0));
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_change_on_every_opening() {
        let mut rng = XorShift::new(3);
        let mut st = AboutState::default();
        let mut last = None;
        for _ in 0..200 {
            st.open(&mut rng);
            let q = st.quote.unwrap();
            assert!(q < QUOTES.len());
            assert_ne!(Some(q), last);
            last = Some(q);
        }
    }

    #[test]
    fn knife_follows_the_dotted_path() {
        let k = knife(Instant::now());
        assert!(k.trail.len() > 50, "{}", k.trail.len());
        let (start, _) = k.trail[0];
        let (end, _) = *k.trail.last().unwrap();
        // The tip trails the axis by the 0.25 mm offset (2.5 banner units).
        assert!(start.distance(Pos2::new(690.0, 300.0)) < 3.0, "{start:?}");
        assert!(end.distance(Pos2::new(1130.0, 240.0)) < 3.0, "{end:?}");
        assert!(k.badges.windows(2).all(|w| w[0] < w[1]));
        assert!((k.badges[3] - k.length_mm).abs() < 1e-9);
    }

    #[test]
    fn the_pitch_lines_make_the_pitch() {
        assert_eq!(PITCH_LINES.join(" "), PITCH);
    }

    #[test]
    fn no_em_dash_in_the_dialog() {
        let mut all = vec![
            MOTTO,
            TAGLINE,
            PITCH,
            PITCH_LINES[0],
            PITCH_LINES[1],
            BUILT_WITH,
            INSPIRED_BY,
            MADE_WITH,
            MADE_FOR,
            LICENSES,
            END_OF_LINE,
            THIRD_PARTY,
        ];
        for (q, a) in QUOTES {
            all.push(q);
            all.push(a);
        }
        for s in all {
            assert!(!s.contains('\u{2014}'), "{s}");
        }
    }
}
