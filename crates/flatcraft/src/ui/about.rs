//! The About dialog: banner, version, release motto, tagline, support
//! links, credits, a maker quote and the third-party licences. It also
//! holds two easter eggs: the maker quote changes on every opening, and
//! five clicks on the banner send a simulated knife along its dotted travel
//! path, cutting the four shape outlines in order before it drives home.

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
pub const QUOTES: [(&str, &str); 7] = [
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
    ("Words are hard.", "Evan & Katelyn"),
    ("Don't glue yourself to the table.", "Joel Telling"),
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

/// Shown on the banner once the knife is home.
pub const END_OF_LINE: &str = "End of line.";
/// The accessible name of the banner.
pub const BANNER_NAME: &str = "FlatCraft banner";
/// Clicks on the banner that start the knife.
pub const BANNER_CLICKS: u32 = 5;
/// How long "End of line." takes to fade in, stays and takes to fade out.
pub const TEXT_FADE_IN: Duration = Duration::from_millis(400);
pub const TEXT_HOLD: Duration = Duration::from_secs(3);
pub const TEXT_FADE_OUT: Duration = Duration::from_millis(600);

/// The banner image (2560 x 640 pixels, drawn on a 1280 x 320 grid).
const BANNER_PNG: &[u8] = include_bytes!("../../../../assets/branding/banner.png");
/// The banner's own coordinate grid (its HTML source).
const BANNER_GRID: Vec2 = Vec2::new(1280.0, 320.0);

// The geometry below is copied from assets/branding/banner.html, in its
// 1280 x 320 space; the test `knife_geometry_is_the_banner_html` keeps
// them in step.

/// Where the dotted teal travel path starts (its first point).
pub const TRAVEL_START: (f64, f64) = (690.0, 300.0);
/// The circle (shape 1): centre and radius. Its cut starts and ends at
/// the rightmost point, where the travel path meets it.
pub const CIRCLE: (f64, f64, f64) = (770.0, 190.0, 44.0);
/// The square (shape 2): x, y, width, height. The cut starts at x, y.
pub const SQUARE: (f64, f64, f64, f64) = (850.0, 140.0, 80.0, 80.0);
/// The star (shape 3), from its top point.
pub const STAR: [(f64, f64); 10] = [
    (1030.0, 128.0),
    (1042.93, 162.2),
    (1079.45, 163.93),
    (1050.92, 186.8),
    (1060.56, 222.07),
    (1030.0, 202.0),
    (999.44, 222.07),
    (1009.08, 186.8),
    (980.55, 163.93),
    (1017.07, 162.2),
];
/// The F (shape 4), from its bottom-left corner.
pub const LETTER_F: [(f64, f64); 10] = [
    (1130.0, 240.0),
    (1130.0, 130.0),
    (1200.0, 130.0),
    (1200.0, 152.0),
    (1152.0, 152.0),
    (1152.0, 176.0),
    (1188.0, 176.0),
    (1188.0, 196.0),
    (1152.0, 196.0),
    (1152.0, 240.0),
];
/// The numbered badges 1 to 4 (centres, radius 12). Badge k lights up
/// when the outline of shape k is cut.
pub const BADGES: [(f32, f32); 4] = [
    (728.0, 136.0),
    (890.0, 104.0),
    (970.0, 104.0),
    (1112.0, 104.0),
];
const BADGE_RADIUS: f32 = 12.0;
/// The text block's left edge (`left: 72px`).
const TEXT_LEFT: f32 = 72.0;

/// The knife's home, at the bottom left of the banner. The way home runs
/// along the bottom edge, under the free space where "End of line." shows.
pub const HOME: (f64, f64) = (24.0, 312.0);
/// The corner the knife turns at on its way home from the F.
const HOME_TURN: (f64, f64) = (1130.0, 312.0);
/// "End of line.": its left edge and vertical centre, in the free space
/// under the tagline and the line under it (which ends at about y 250),
/// left of the travel path.
const TEXT_AT: (f32, f32) = (TEXT_LEFT, 283.0);

/// Banner units per simulated millimetre.
const UNITS_PER_MM: f64 = 10.0;
/// Every this many millimetres of the simulated axis, one trail point.
const TRAIL_STEP_MM: f64 = 0.4;
/// Animation speeds, blade down and up, in mm (of the banner) per second.
const CUT_SPEED: f64 = 70.0;
const TRAVEL_SPEED: f64 = 220.0;
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

/// One point of the knife's way, in banner units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailPoint {
    pub at: Pos2,
    /// Seconds from the start of the animation.
    pub secs: f32,
    /// The blade is down on the move into this point.
    pub down: bool,
    /// The shape (0 to 3) being cut or travelled to; `None` on the way
    /// to the travel path and on the way home.
    pub shape: Option<usize>,
}

/// The banner knife: the simulated blade tip and its timing.
#[derive(Debug, Clone)]
pub struct Knife {
    pub started: Instant,
    pub trail: Vec<TrailPoint>,
    /// When the cut of each shape starts and ends, in seconds.
    pub cut_start: [f32; 4],
    pub cut_end: [f32; 4],
    /// When the knife is home.
    pub run_secs: f32,
}

impl Knife {
    fn secs(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.started).as_secs_f32()
    }

    /// The knife is home.
    pub fn home(&self, now: Instant) -> bool {
        self.secs(now) >= self.run_secs
    }

    /// When everything has faded out.
    pub fn end_secs(&self) -> f32 {
        self.run_secs + (TEXT_FADE_IN + TEXT_HOLD + TEXT_FADE_OUT).as_secs_f32()
    }

    /// Everything has faded out: the banner is back to normal.
    pub fn finished(&self, now: Instant) -> bool {
        self.secs(now) >= self.end_secs()
    }

    /// The opacity of "End of line.": 0 until the knife is home, then it
    /// fades in, stays and fades out.
    pub fn text_alpha(&self, now: Instant) -> f32 {
        let t = self.secs(now) - self.run_secs;
        let fade_in = TEXT_FADE_IN.as_secs_f32();
        let hold = TEXT_HOLD.as_secs_f32();
        let out = TEXT_FADE_OUT.as_secs_f32();
        if t < 0.0 {
            0.0
        } else if t < fade_in {
            t / fade_in
        } else if t < fade_in + hold {
            1.0
        } else {
            (1.0 - (t - fade_in - hold) / out).clamp(0.0, 1.0)
        }
    }

    /// The opacity of the cut lines and the lit badges: full until the
    /// text fades out, then they fade out with it.
    pub fn trail_alpha(&self, now: Instant) -> f32 {
        let t = self.secs(now) - self.run_secs;
        if t < (TEXT_FADE_IN + TEXT_HOLD).as_secs_f32() {
            1.0
        } else {
            self.text_alpha(now)
        }
    }

    /// The trail up to `secs`, ending at the knife's position then.
    pub fn trail_at(&self, secs: f32) -> Vec<TrailPoint> {
        let mut out = Vec::new();
        for (i, p) in self.trail.iter().enumerate() {
            if p.secs <= secs {
                out.push(*p);
                continue;
            }
            if let Some(prev) = i.checked_sub(1).map(|j| self.trail[j]) {
                let f = (secs - prev.secs) / (p.secs - prev.secs).max(1e-6);
                out.push(TrailPoint {
                    at: prev.at.lerp(p.at, f.clamp(0.0, 1.0)),
                    secs,
                    ..*p
                });
            }
            break;
        }
        out
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

fn mm((x, y): (f64, f64)) -> Point {
    Point::new(x / UNITS_PER_MM, y / UNITS_PER_MM)
}

/// The four outlines as closed rings in mm, each starting (and ending)
/// where the travel path meets it.
pub fn outlines() -> [Vec<Point>; 4] {
    let (cx, cy, r) = CIRCLE;
    let circle = (0..=96)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / 96.0;
            mm((cx + r * a.cos(), cy + r * a.sin()))
        })
        .collect();
    let (x, y, w, h) = SQUARE;
    let square = [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)]
        .into_iter()
        .map(mm)
        .collect();
    let ring = |pts: &[(f64, f64)]| {
        pts.iter()
            .chain(pts.first())
            .copied()
            .map(mm)
            .collect::<Vec<_>>()
    };
    [circle, square, ring(&STAR), ring(&LETTER_F)]
}

/// Simulates a 0.25 mm drag knife: from home to the start of the dotted
/// travel path, then along it to each shape in turn (flatcraft_sim does
/// the cuts and the travel between them), then home along the bottom.
pub fn knife(now: Instant) -> Knife {
    let paths: Vec<Toolpath> = outlines()
        .into_iter()
        .enumerate()
        .map(|(i, points)| {
            Toolpath::new(
                points,
                true,
                PathRef {
                    object: ObjectId(i as u64),
                    path_index: 0,
                },
            )
        })
        .collect();
    let start = mm(TRAVEL_START);
    let trace = simulate(&paths, start, &SimSettings::float(0.25));
    let samples: Vec<_> = trace.iter().flat_map(|t| &t.samples).collect();

    let to_banner = |p: Point| Pos2::new((p.x * UNITS_PER_MM) as f32, (p.y * UNITS_PER_MM) as f32);
    let mut trail = vec![TrailPoint {
        at: to_banner(mm(HOME)),
        secs: 0.0,
        down: false,
        shape: None,
    }];
    let mut secs = 0.0f64;
    let mut last = mm(HOME);
    let travel_to = |trail: &mut Vec<TrailPoint>, secs: &mut f64, last: &mut Point, p: Point| {
        *secs += last.distance(p) / TRAVEL_SPEED;
        *last = p;
        trail.push(TrailPoint {
            at: to_banner(p),
            secs: *secs as f32,
            down: false,
            shape: None,
        });
    };
    travel_to(&mut trail, &mut secs, &mut last, start);
    // Along the simulation: time by the axis travel, blade down slower.
    let mut prev_t = samples.first().map_or(0.0, |s| s.t_mm);
    let mut kept_t = f64::NEG_INFINITY;
    for (i, s) in samples.iter().enumerate() {
        let speed = if s.down { CUT_SPEED } else { TRAVEL_SPEED };
        secs += (s.t_mm - prev_t) / speed;
        prev_t = s.t_mm;
        let prev = trail.last().copied();
        let changed = prev.is_some_and(|p| p.down != s.down || p.shape != Some(s.toolpath));
        // The last sample before the blade goes up or down (or the next
        // shape starts) stays too, so a cut ends where it really ends.
        let end = samples
            .get(i + 1)
            .is_none_or(|n| n.down != s.down || n.toolpath != s.toolpath);
        if s.t_mm - kept_t < TRAIL_STEP_MM && !changed && !end {
            continue;
        }
        kept_t = s.t_mm;
        trail.push(TrailPoint {
            at: to_banner(s.tip),
            secs: secs as f32,
            down: s.down,
            shape: Some(s.toolpath),
        });
    }
    last = samples.last().map_or(start, |s| s.axis);
    travel_to(&mut trail, &mut secs, &mut last, mm(HOME_TURN));
    travel_to(&mut trail, &mut secs, &mut last, mm(HOME));

    let mut cut_start = [f32::INFINITY; 4];
    let mut cut_end = [0.0f32; 4];
    for p in trail.iter().filter(|p| p.down) {
        if let Some(k) = p.shape.filter(|k| *k < 4) {
            cut_start[k] = cut_start[k].min(p.secs);
            cut_end[k] = cut_end[k].max(p.secs);
        }
    }
    Knife {
        started: now,
        trail,
        cut_start,
        cut_end,
        run_secs: secs as f32,
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
    if st.knife.as_ref().is_some_and(|k| k.finished(now)) {
        // Faded out: the banner is back to normal and asks for no frames.
        st.knife = None;
    }
    let Some(knife) = &st.knife else {
        return;
    };
    // Frames only while something moves or fades.
    ui.ctx().request_repaint();
    let rect = resp.rect;
    let scale = rect.width() / BANNER_GRID.x;
    let to_screen =
        |p: Pos2| rect.min + Vec2::new(p.x / BANNER_GRID.x, p.y / BANNER_GRID.y) * rect.size();
    let secs = now.saturating_duration_since(knife.started).as_secs_f32();
    let fade = knife.trail_alpha(now);
    let painter = ui.painter_at(rect);
    let cut = egui::Stroke::new(2.0, t.accent_cut.gamma_multiply(fade));
    let travel = egui::Stroke::new(1.0, t.accent_cut.gamma_multiply(0.35 * fade));
    let trail = knife.trail_at(secs);
    // Runs of the same kind (cut or travel) become one line each.
    let mut i = 1;
    while i < trail.len() {
        let down = trail[i].down;
        let mut line = vec![to_screen(trail[i - 1].at)];
        while i < trail.len() && trail[i].down == down {
            line.push(to_screen(trail[i].at));
            i += 1;
        }
        painter.add(egui::Shape::line(line, if down { cut } else { travel }));
    }
    for (k, &(x, y)) in BADGES.iter().enumerate() {
        if knife.cut_end[k] <= secs {
            let c = to_screen(Pos2::new(x, y));
            painter.circle_filled(
                c,
                BADGE_RADIUS * scale,
                t.accent_cut.gamma_multiply(0.45 * fade),
            );
            painter.circle_stroke(
                c,
                BADGE_RADIUS * scale + 1.0,
                egui::Stroke::new(2.0, t.accent_cut.gamma_multiply(fade)),
            );
        }
    }
    if !knife.home(now)
        && let Some(tip) = trail.last()
    {
        let p = to_screen(tip.at);
        painter.circle_filled(p, 4.0, t.accent_cut);
        painter.circle_stroke(p, 6.0, egui::Stroke::new(1.0, Color32::WHITE));
    }
    let alpha = knife.text_alpha(now);
    if alpha > 0.0 {
        // Left-aligned and vertically centred on TEXT_AT, in a band that
        // ends before the travel path starts (x 690).
        let left_mid = to_screen(Pos2::new(TEXT_AT.0, TEXT_AT.1));
        let right = to_screen(Pos2::new(TRAVEL_START.0 as f32, TEXT_AT.1)).x;
        let band = egui::Rect::from_min_max(
            left_mid - Vec2::new(0.0, 20.0),
            Pos2::new(right, left_mid.y + 20.0),
        );
        let builder = egui::UiBuilder::new()
            .max_rect(band)
            .layout(egui::Layout::left_to_right(egui::Align::Center));
        ui.scope_builder(builder, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(END_OF_LINE)
                        .font(fonts::semibold(20.0))
                        .color(t.accent_cut.gamma_multiply(alpha)),
                )
                .selectable(false),
            );
        });
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
    fn knife_cuts_the_shapes_in_order_and_ends_home() {
        let k = knife(Instant::now());
        // Shape k is cut completely before shape k + 1 starts.
        for w in 0..3 {
            assert!(k.cut_start[w] < k.cut_end[w], "{k:?}");
            assert!(k.cut_end[w] < k.cut_start[w + 1], "shape {w}");
        }
        // The F is cut: blade-down points run around its whole outline.
        let f: Vec<Pos2> = k
            .trail
            .iter()
            .filter(|p| p.down && p.shape == Some(3))
            .map(|p| p.at)
            .collect();
        for &(x, y) in &LETTER_F {
            let corner = Pos2::new(x as f32, y as f32);
            let near = f
                .iter()
                .map(|p| p.distance(corner))
                .fold(f32::MAX, f32::min);
            // The tip trails the axis by 0.25 mm (2.5 banner units).
            assert!(near < 6.0, "F corner {corner:?} not cut ({near})");
        }
        // Between the shapes the blade is up, along the dotted path.
        let start = k.trail.iter().position(|p| p.shape.is_some()).unwrap();
        assert!(k.trail[start].at.distance(Pos2::new(690.0, 300.0)) < 3.0);
        // Home at the end, the blade up.
        let end = k.trail.last().unwrap();
        assert_eq!(end.at, Pos2::new(HOME.0 as f32, HOME.1 as f32));
        assert!(!end.down);
        assert!((end.secs - k.run_secs).abs() < 1e-3);
        assert!(k.trail.windows(2).all(|w| w[0].secs <= w[1].secs));
    }

    #[test]
    fn every_cut_ends_as_a_cut() {
        let k = knife(Instant::now());
        // The simulation's last blade-down point of each shape, unthinned.
        let paths: Vec<Toolpath> = outlines()
            .into_iter()
            .map(|p| {
                Toolpath::new(
                    p,
                    true,
                    PathRef {
                        object: ObjectId(0),
                        path_index: 0,
                    },
                )
            })
            .collect();
        let trace = simulate(&paths, mm(TRAVEL_START), &SimSettings::float(0.25)).unwrap();
        for shape in 0..4 {
            let last = trace
                .samples
                .iter()
                .rfind(|s| s.down && s.toolpath == shape)
                .unwrap();
            let want = Pos2::new(
                (last.tip.x * UNITS_PER_MM) as f32,
                (last.tip.y * UNITS_PER_MM) as f32,
            );
            let drawn = k
                .trail
                .iter()
                .rfind(|p| p.down && p.shape == Some(shape))
                .unwrap();
            assert_eq!(drawn.at, want, "shape {shape}");
            // The next point is travel: the cut is not drawn on into it.
            let i = k.trail.iter().position(|p| p == drawn).unwrap();
            assert!(!k.trail[i + 1].down, "shape {shape}");
        }
    }

    #[test]
    fn end_of_line_fades_in_and_out() {
        let t0 = Instant::now();
        let k = knife(t0);
        let at = |s: f32| t0 + Duration::from_secs_f32(s);
        assert_eq!(k.text_alpha(at(k.run_secs - 0.1)), 0.0);
        assert!(k.text_alpha(at(k.run_secs + 0.2)) > 0.0);
        assert!(k.text_alpha(at(k.run_secs + 0.2)) < 1.0);
        assert_eq!(k.text_alpha(at(k.run_secs + 2.0)), 1.0);
        assert_eq!(k.trail_alpha(at(k.run_secs + 2.0)), 1.0);
        assert!(k.text_alpha(at(k.end_secs() - 0.1)) < 0.5);
        assert!(!k.finished(at(k.end_secs() - 0.1)));
        assert_eq!(k.text_alpha(at(k.end_secs() + 0.01)), 0.0);
        assert!(k.finished(at(k.end_secs() + 0.01)));
        // About 3 s of text, after a run of a few seconds.
        assert!((2.0..8.0).contains(&k.run_secs), "{}", k.run_secs);
    }

    /// The constants are the banner's own geometry.
    #[test]
    fn knife_geometry_is_the_banner_html() {
        let html = include_str!("../../../../assets/branding/banner.html");
        let group = |open: &str| {
            let i = html.find(open).unwrap_or_else(|| panic!("no {open}"));
            let j = html[i..].find("</g>").unwrap() + i;
            &html[i..j]
        };
        let n = |v: f64| format!("{v}");
        let travel = group(r##"<g fill="none" stroke="#7FE0D2" stroke-opacity="0.6""##);
        let pts = [
            TRAVEL_START,
            (CIRCLE.0 + CIRCLE.2, CIRCLE.1),
            (SQUARE.0, SQUARE.1),
            STAR[0],
            LETTER_F[0],
        ];
        for w in pts.windows(2) {
            let d = format!(
                r#"<path d="M{} {} L{} {}">"#,
                n(w[0].0),
                n(w[0].1),
                n(w[1].0),
                n(w[1].1)
            );
            assert!(travel.contains(&d), "{d}");
        }
        let outlines = group(r##"<g fill="none" stroke="#F2A33A" stroke-opacity="0.6""##);
        let (cx, cy, r) = CIRCLE;
        let (x, y, w, h) = SQUARE;
        let star: Vec<String> = STAR
            .iter()
            .map(|p| format!("{},{}", n(p.0), n(p.1)))
            .collect();
        let f: Vec<String> = LETTER_F
            .iter()
            .map(|p| format!("{} {}", n(p.0), n(p.1)))
            .collect();
        for want in [
            format!(r#"<circle cx="{}" cy="{}" r="{}">"#, n(cx), n(cy), n(r)),
            format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}">"#,
                n(x),
                n(y),
                n(w),
                n(h)
            ),
            format!(r#"<polygon points="{}">"#, star.join(" ")),
            format!(r#"<path d="M{} Z">"#, f.join(" L")),
        ] {
            assert!(outlines.contains(&want), "{want}");
        }
        for (x, y) in BADGES {
            let want = format!(r#"<circle cx="{x}" cy="{y}" r="{BADGE_RADIUS}""#);
            assert!(html.contains(&want), "{want}");
        }
        assert!(html.contains(&format!("left: {TEXT_LEFT}px")));
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
