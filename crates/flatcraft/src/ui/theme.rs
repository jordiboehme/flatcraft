//! FlatCraft's own colour and size tokens, and how they map onto egui.

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// The design tokens of one theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    /// Window background.
    pub bg: Color32,
    /// App bar, ribbon and side panels.
    pub panel: Color32,
    /// Cards, buttons and fields on top of a panel.
    pub surface: Color32,
    /// The collapsed side panels' rails, recessed against the panels. The
    /// title bar and the ribbon's tab bar use it too.
    pub rail: Color32,
    /// The thin line around the ribbon panel and its active tab.
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// Teal, the colour of the Prepare workspace.
    pub accent_prepare: Color32,
    /// Amber, the colour of the Cut workspace.
    pub accent_cut: Color32,
    pub danger: Color32,
    /// Weed paths (the weed box and weed lines) in the Cut workspace. A
    /// blue, not the Prepare teal; see `colour::weed_colour` for the mat.
    pub weed_path: Color32,
    /// Kept material on the canvas (the kept and waste overlay): solid
    /// paper.
    pub kept_paper: Color32,
    /// The light hatch over waste on the canvas, with its alpha.
    pub waste_hatch: Color32,
    /// The rail dot of a job that is ready to cut.
    pub ok: Color32,
    /// The rail dot of a job with warnings.
    pub warn: Color32,
    /// Warning text on a panel, a card or a dialog: an amber dark enough
    /// (light) or bright enough (dark) for 4.5:1. `warn` and `accent_cut`
    /// are for dots, icons and lines only.
    pub warn_text: Color32,
    /// The dot beside a value that differs from its source (the material
    /// or plotter preset): orange, for the dot only, never for text.
    pub changed: Color32,
    /// The simulation grade on a panel (not on the mat: the mat picks its
    /// own colours, `colour::deviation_colours`): within the limit,
    /// within twice the limit, beyond.
    pub quality_good: Color32,
    pub quality_warn: Color32,
    pub quality_bad: Color32,
    /// Icons: the paper shapes (the things that get cut) are filled with
    /// `icon_paper` and outlined with `icon_cut`, the part that tells what
    /// happens is `icon_accent`, the rest `icon_ink`.
    pub icon_paper: Color32,
    pub icon_cut: Color32,
    pub icon_accent: Color32,
    pub icon_ink: Color32,
    /// Corner radius of the ribbon's buttons, groups and lists, in points.
    pub ribbon_radius: u8,
    /// Corner radius in points.
    pub radius: u8,
    /// Base spacing in points.
    pub spacing: f32,
}

const fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl Tokens {
    pub const fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(0x15, 0x17, 0x1A),
            panel: Color32::from_rgb(0x1D, 0x20, 0x24),
            surface: Color32::from_rgb(0x27, 0x2B, 0x30),
            rail: Color32::from_rgb(0x17, 0x19, 0x1D),
            border: Color32::from_rgb(0x2E, 0x33, 0x39),
            text: Color32::from_rgb(0xE6, 0xE8, 0xEB),
            text_dim: Color32::from_rgb(0x9A, 0xA1, 0xAB),
            accent_prepare: Color32::from_rgb(0x2B, 0xB3, 0xA3),
            accent_cut: Color32::from_rgb(0xF2, 0xA3, 0x3A),
            danger: Color32::from_rgb(0xE5, 0x48, 0x4D),
            weed_path: rgb(flatcraft_core::colour::WEED_DARK),
            // A very light grey, a touch lighter than the dark default mat,
            // so kept paper stays apart from the mat.
            kept_paper: Color32::from_rgb(0xF6, 0xF7, 0xF9),
            waste_hatch: Color32::from_rgba_unmultiplied_const(0x2A, 0x2F, 0x36, 0x66),
            ok: Color32::from_rgb(0x46, 0xC2, 0x7A),
            warn: Color32::from_rgb(0xE8, 0xB2, 0x3A),
            warn_text: Color32::from_rgb(0xF2, 0xB4, 0x5A),
            changed: Color32::from_rgb(0xF0, 0x8A, 0x3C),
            quality_good: Color32::from_rgb(0x3F, 0xB9, 0x50),
            quality_warn: Color32::from_rgb(0xF2, 0xA3, 0x3A),
            quality_bad: Color32::from_rgb(0xE5, 0x48, 0x4D),
            icon_paper: Color32::from_rgb(0xF3, 0xEC, 0xDD),
            // Darker than the Cut accent, so the line shows on the paper.
            icon_cut: Color32::from_rgb(0xE8, 0x89, 0x2E),
            icon_accent: Color32::from_rgb(0x35, 0xC9, 0xB9),
            icon_ink: Color32::from_rgb(0xD5, 0xDE, 0xDB),
            ribbon_radius: 2,
            radius: 6,
            spacing: 8.0,
        }
    }

    pub const fn light() -> Self {
        Self {
            bg: Color32::from_rgb(0xF4, 0xF5, 0xF7),
            panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            surface: Color32::from_rgb(0xE9, 0xEC, 0xEF),
            rail: Color32::from_rgb(0xEE, 0xF1, 0xF1),
            border: Color32::from_rgb(0xD5, 0xDA, 0xDE),
            text: Color32::from_rgb(0x1B, 0x1E, 0x22),
            text_dim: Color32::from_rgb(0x5E, 0x66, 0x70),
            accent_prepare: Color32::from_rgb(0x2B, 0xB3, 0xA3),
            accent_cut: Color32::from_rgb(0xF2, 0xA3, 0x3A),
            danger: Color32::from_rgb(0xC8, 0x2E, 0x34),
            weed_path: rgb(flatcraft_core::colour::WEED_LIGHT),
            kept_paper: Color32::from_rgb(0xEE, 0xF0, 0xF3),
            waste_hatch: Color32::from_rgba_unmultiplied_const(0x4A, 0x52, 0x5C, 0x80),
            ok: Color32::from_rgb(0x2F, 0x9E, 0x5B),
            warn: Color32::from_rgb(0xD9, 0x9A, 0x1E),
            warn_text: Color32::from_rgb(0x94, 0x52, 0x00),
            changed: Color32::from_rgb(0xC8, 0x5A, 0x0A),
            // Deeper than the dark theme's, so they reach 3:1 on white.
            quality_good: Color32::from_rgb(0x1A, 0x7F, 0x45),
            quality_warn: Color32::from_rgb(0xA8, 0x6A, 0x00),
            quality_bad: Color32::from_rgb(0xC8, 0x2E, 0x34),
            icon_paper: Color32::from_rgb(0xFB, 0xF4, 0xE4),
            icon_cut: Color32::from_rgb(0xD9, 0x78, 0x2A),
            icon_accent: Color32::from_rgb(0x0F, 0x8A, 0x7E),
            icon_ink: Color32::from_rgb(0x33, 0x40, 0x3D),
            ribbon_radius: 2,
            radius: 6,
            spacing: 8.0,
        }
    }

    /// The tokens for a preference value. `system` is the operating
    /// system's appearance, when the window knows it; it only matters for
    /// `Theme::System`, which is dark until the system says otherwise.
    pub const fn for_theme(
        theme: flatcraft_engine::prefs::Theme,
        system: Option<egui::Theme>,
    ) -> Self {
        use flatcraft_engine::prefs::Theme;
        match (theme, system) {
            (Theme::Light, _) | (Theme::System, Some(egui::Theme::Light)) => Self::light(),
            (Theme::Dark, _) | (Theme::System, _) => Self::dark(),
        }
    }

    /// The tokens of the theme `visuals` belong to.
    pub fn of(visuals: &Visuals) -> Self {
        if visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub fn is_dark(&self) -> bool {
        self.bg.r() < 0x80
    }
}

/// Installs the tokens as the egui style of the context. Both themes get
/// their style, so a system appearance change restyles at once. With
/// `follow_system` egui's own preference is System too, so the window
/// frame follows the operating system and reports its changes; otherwise
/// the theme of `t` is forced.
pub fn apply(ctx: &egui::Context, t: &Tokens, follow_system: bool) {
    let other = if t.is_dark() {
        Tokens::light()
    } else {
        Tokens::dark()
    };
    install(ctx, &other);
    let theme = install(ctx, t);
    if follow_system {
        ctx.set_theme(egui::ThemePreference::System);
    } else {
        ctx.set_theme(theme);
    }
}

/// Sets the visuals and spacing of the theme that `t` belongs to.
fn install(ctx: &egui::Context, t: &Tokens) -> egui::Theme {
    let mut visuals = if t.is_dark() {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let radius = CornerRadius::same(t.radius);
    visuals.panel_fill = t.panel;
    visuals.window_fill = t.panel;
    visuals.extreme_bg_color = t.bg;
    visuals.faint_bg_color = t.surface;
    visuals.override_text_color = Some(t.text);
    visuals.hyperlink_color = t.accent_prepare;
    visuals.error_fg_color = t.danger;
    visuals.warn_fg_color = t.accent_cut;
    visuals.selection.bg_fill = t.accent_prepare.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, t.accent_prepare);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    visuals.widgets.noninteractive.bg_fill = t.panel;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    visuals.widgets.inactive.bg_fill = t.surface;
    visuals.widgets.inactive.weak_bg_fill = t.surface;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, t.text);
    visuals.widgets.active.fg_stroke = Stroke::new(2.0, t.text);

    let theme = if t.is_dark() {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_visuals_of(theme, visuals);
    ctx.style_mut_of(theme, |style| {
        style.spacing.item_spacing = egui::vec2(t.spacing, t.spacing * 0.75);
        style.spacing.button_padding = egui::vec2(t.spacing, t.spacing * 0.5);
        style.spacing.window_margin = egui::Margin::same(t.spacing as i8);
        style.spacing.menu_margin = egui::Margin::same(t.spacing as i8);
    });
    theme
}

/// The ribbon's tighter look: small corners and less padding, so the
/// groups use their room. Applied to the ribbon and to its lists.
pub fn tight(style: &mut egui::Style) {
    let t = Tokens::of(&style.visuals);
    let radius = CornerRadius::same(t.ribbon_radius);
    let v = &mut style.visuals;
    v.menu_corner_radius = radius;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = radius;
    }
    style.spacing.button_padding = egui::vec2(t.spacing / 2.0, t.spacing / 4.0);
}

/// [`tight`] on top of egui's menu style, for the ribbon's lists.
pub fn tight_menu(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    tight(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance.
    fn luminance(c: Color32) -> f64 {
        let ch = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn icon_colours_keep_contrast_in_both_themes() {
        for t in [Tokens::dark(), Tokens::light()] {
            // Lines against the panel the icons sit on: 3:1 for graphics.
            for c in [t.icon_ink, t.icon_accent, t.icon_cut] {
                assert!(contrast(c, t.panel) >= 3.0, "{c:?} on {:?}", t.panel);
            }
            // The cut line must show against the paper it outlines.
            assert!(contrast(t.icon_cut, t.icon_paper) >= 2.0);
        }
    }

    /// The change dot reads at 3:1 (graphics) on the panel and the
    /// surface in both themes.
    #[test]
    fn the_change_dot_reads_on_the_panel() {
        for t in [Tokens::dark(), Tokens::light()] {
            for bg in [t.panel, t.surface] {
                let c = contrast(t.changed, bg);
                assert!(c >= 3.0, "{:?} on {bg:?}: {c:.2}", t.changed);
            }
        }
    }

    /// Warning text reads at 4.5:1 (WCAG AA for text) on the window, the
    /// panels and the cards in both themes.
    #[test]
    fn warning_text_reads_on_every_surface() {
        for t in [Tokens::dark(), Tokens::light()] {
            for bg in [t.bg, t.panel, t.surface] {
                let c = contrast(t.warn_text, bg);
                assert!(c >= 4.5, "{:?} on {bg:?}: {c:.2}", t.warn_text);
            }
        }
    }

    /// The grade dots read at 3:1 (graphics) on the panel and on the
    /// card fill of the compare row in use, in both themes. Grade text is
    /// drawn in the text colour.
    #[test]
    fn quality_colours_read_on_the_panel() {
        for t in [Tokens::dark(), Tokens::light()] {
            for c in [t.quality_good, t.quality_warn, t.quality_bad] {
                for bg in [t.panel, t.surface] {
                    assert!(contrast(c, bg) >= 3.0, "{c:?} on {bg:?}");
                }
            }
        }
    }

    /// Weed paths read at 3:1 or better on the paper mat, a dark vinyl
    /// mat and the canvas of their theme, in the colour the canvas picks
    /// (`weed_colour`), and they are blue: neither the Prepare teal nor
    /// the cut orange.
    #[test]
    fn weed_paths_read_on_the_mats_and_the_canvas() {
        use flatcraft_core::colour::{cut_colour, weed_colour};
        let paper = flatcraft_device::DEFAULT_MAT_COLOR_LIGHT;
        // A dark vinyl backing used as a mat colour (no preset has one).
        let vinyl = [0x33, 0x37, 0x3C];
        let raw = |c: Color32| [c.r(), c.g(), c.b()];
        for t in [Tokens::dark(), Tokens::light()] {
            let own = raw(t.weed_path);
            for mat in [
                paper,
                flatcraft_device::DEFAULT_MAT_COLOR_LIGHT,
                flatcraft_device::DEFAULT_MAT_COLOR_DARK,
                [0xFB, 0xF4, 0xE4],
                vinyl,
            ] {
                let c = super::rgb(weed_colour(mat, own));
                assert!(contrast(c, super::rgb(mat)) >= 3.0, "{c:?} on {mat:?}");
                assert_ne!(c, super::rgb(cut_colour(mat)));
            }
            // The theme's own blue on the paper mat and on its canvas.
            assert_eq!(weed_colour(paper, own), own);
            assert!(
                contrast(t.weed_path, t.bg) >= 3.0,
                "{:?} on {:?}",
                t.weed_path,
                t.bg
            );
            // Blue: blue leads.
            assert!(t.weed_path.b() > t.weed_path.g() && t.weed_path.b() > t.weed_path.r());
            // Hue in degrees: the teal sits near 173, the blues past 210.
            let hue = |c: Color32| {
                let (r, g, b) = (f64::from(c.r()), f64::from(c.g()), f64::from(c.b()));
                let (max, min) = (r.max(g).max(b), r.min(g).min(b));
                let h = if max == b {
                    4.0 + (r - g) / (max - min)
                } else if max == g {
                    2.0 + (b - r) / (max - min)
                } else {
                    ((g - b) / (max - min)).rem_euclid(6.0)
                };
                h * 60.0
            };
            let apart = (hue(t.weed_path) - hue(t.accent_prepare)).abs();
            assert!(apart >= 30.0, "only {apart} degrees from the Prepare teal");
        }
    }

    /// The icons keep their cream paper; the mat is white (light) or a dimmed
    /// grey (dark) by default, and kept paper stays apart from either.
    #[test]
    fn kept_paper_differs_from_white_mat() {
        use flatcraft_device::{DEFAULT_MAT_COLOR_DARK, DEFAULT_MAT_COLOR_LIGHT};
        let raw = |c: Color32| [c.r(), c.g(), c.b()];
        assert_ne!(DEFAULT_MAT_COLOR_LIGHT, [0xFB, 0xF4, 0xE4]);
        for (t, mat) in [
            (Tokens::light(), DEFAULT_MAT_COLOR_LIGHT),
            (Tokens::dark(), DEFAULT_MAT_COLOR_DARK),
        ] {
            let kept = raw(t.kept_paper);
            assert_ne!(kept, mat);
            assert!(contrast(t.kept_paper, super::rgb(mat)) >= 1.05);
        }
        assert_eq!(
            Tokens::light().icon_paper,
            Color32::from_rgb(0xFB, 0xF4, 0xE4)
        );
    }

    #[test]
    fn token_values() {
        let d = Tokens::dark();
        assert_eq!(d.bg, Color32::from_rgb(0x15, 0x17, 0x1A));
        assert_eq!(d.accent_prepare, Color32::from_rgb(0x2B, 0xB3, 0xA3));
        assert_eq!(d.accent_cut, Color32::from_rgb(0xF2, 0xA3, 0x3A));
        assert_eq!(d.radius, 6);
        assert_eq!(d.spacing, 8.0);
        assert!(d.is_dark());
        assert!(!Tokens::light().is_dark());
    }
}
