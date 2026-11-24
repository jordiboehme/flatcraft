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
    pub text: Color32,
    pub text_dim: Color32,
    /// Teal, the colour of the Prepare workspace.
    pub accent_prepare: Color32,
    /// Amber, the colour of the Cut workspace.
    pub accent_cut: Color32,
    pub danger: Color32,
    /// Corner radius in points.
    pub radius: u8,
    /// Base spacing in points.
    pub spacing: f32,
}

impl Tokens {
    pub const fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(0x15, 0x17, 0x1A),
            panel: Color32::from_rgb(0x1D, 0x20, 0x24),
            surface: Color32::from_rgb(0x27, 0x2B, 0x30),
            text: Color32::from_rgb(0xE6, 0xE8, 0xEB),
            text_dim: Color32::from_rgb(0x9A, 0xA1, 0xAB),
            accent_prepare: Color32::from_rgb(0x2B, 0xB3, 0xA3),
            accent_cut: Color32::from_rgb(0xF2, 0xA3, 0x3A),
            danger: Color32::from_rgb(0xE5, 0x48, 0x4D),
            radius: 6,
            spacing: 8.0,
        }
    }

    pub const fn light() -> Self {
        Self {
            bg: Color32::from_rgb(0xF4, 0xF5, 0xF7),
            panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            surface: Color32::from_rgb(0xE9, 0xEC, 0xEF),
            text: Color32::from_rgb(0x1B, 0x1E, 0x22),
            text_dim: Color32::from_rgb(0x5E, 0x66, 0x70),
            accent_prepare: Color32::from_rgb(0x2B, 0xB3, 0xA3),
            accent_cut: Color32::from_rgb(0xF2, 0xA3, 0x3A),
            danger: Color32::from_rgb(0xC8, 0x2E, 0x34),
            radius: 6,
            spacing: 8.0,
        }
    }

    /// The tokens for a preference value.
    pub const fn for_theme(theme: flatcraft_engine::prefs::Theme) -> Self {
        match theme {
            flatcraft_engine::prefs::Theme::Dark => Self::dark(),
            flatcraft_engine::prefs::Theme::Light => Self::light(),
        }
    }

    pub fn is_dark(&self) -> bool {
        self.bg.r() < 0x80
    }
}

/// Installs the tokens as the egui style of the context.
pub fn apply(ctx: &egui::Context, t: &Tokens) {
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
    ctx.set_theme(theme);
    ctx.set_visuals_of(theme, visuals);
    ctx.style_mut_of(theme, |style| {
        style.spacing.item_spacing = egui::vec2(t.spacing, t.spacing * 0.75);
        style.spacing.button_padding = egui::vec2(t.spacing, t.spacing * 0.5);
        style.spacing.window_margin = egui::Margin::same(t.spacing as i8);
        style.spacing.menu_margin = egui::Margin::same(t.spacing as i8);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_values() {
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
