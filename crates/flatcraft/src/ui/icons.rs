//! Lucide icons (ISC licence, see `assets/icons/LICENSE`), rasterised with
//! resvg in the theme's text colour and cached per size and colour.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

/// Every icon the app uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Import,
    Trace,
    Duplicate,
    Delete,
    Scale,
    Rotate,
    Mirror,
    Align,
    Arrange,
    Undo,
    Redo,
    Save,
    Home,
    Settings,
    Help,
    Plug,
    Play,
    Send,
    Cut,
    Prepare,
    Search,
    Eye,
    EyeOff,
    Plus,
    Close,
    BrokenLink,
    Reload,
    SvgFile,
    ImageFile,
    Grid,
    Ruler,
    Debug,
    ZoomFit,
    MirrorVertical,
    TestPatterns,
    PanelLeft,
    PanelRight,
    RotateLeft,
}

macro_rules! svg {
    ($name:literal) => {
        include_str!(concat!("../../../../assets/icons/", $name, ".svg"))
    };
}

impl Icon {
    pub const ALL: [Icon; 38] = [
        Icon::Import,
        Icon::Trace,
        Icon::Duplicate,
        Icon::Delete,
        Icon::Scale,
        Icon::Rotate,
        Icon::Mirror,
        Icon::Align,
        Icon::Arrange,
        Icon::Undo,
        Icon::Redo,
        Icon::Save,
        Icon::Home,
        Icon::Settings,
        Icon::Help,
        Icon::Plug,
        Icon::Play,
        Icon::Send,
        Icon::Cut,
        Icon::Prepare,
        Icon::Search,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Plus,
        Icon::Close,
        Icon::BrokenLink,
        Icon::Reload,
        Icon::SvgFile,
        Icon::ImageFile,
        Icon::Grid,
        Icon::Ruler,
        Icon::Debug,
        Icon::ZoomFit,
        Icon::MirrorVertical,
        Icon::TestPatterns,
        Icon::PanelLeft,
        Icon::PanelRight,
        Icon::RotateLeft,
    ];

    /// The vendored Lucide SVG source.
    pub fn svg(self) -> &'static str {
        match self {
            Icon::Import => svg!("import"),
            Icon::Trace => svg!("spline"),
            Icon::Duplicate => svg!("copy"),
            Icon::Delete => svg!("trash"),
            Icon::Scale => svg!("scaling"),
            Icon::Rotate => svg!("rotate-cw"),
            Icon::Mirror => svg!("square-axis-dashed-vertical"),
            Icon::Align => svg!("align-center-horizontal"),
            Icon::Arrange => svg!("layout-grid"),
            Icon::Undo => svg!("undo-2"),
            Icon::Redo => svg!("redo-2"),
            Icon::Save => svg!("save"),
            Icon::Home => svg!("house"),
            Icon::Settings => svg!("settings"),
            Icon::Help => svg!("circle-question-mark"),
            Icon::Plug => svg!("plug"),
            Icon::Play => svg!("play"),
            Icon::Send => svg!("send"),
            Icon::Cut => svg!("scissors"),
            Icon::Prepare => svg!("pencil-ruler"),
            Icon::Search => svg!("search"),
            Icon::Eye => svg!("eye"),
            Icon::EyeOff => svg!("eye-off"),
            Icon::Plus => svg!("plus"),
            Icon::Close => svg!("x"),
            Icon::BrokenLink => svg!("link-2-off"),
            Icon::Reload => svg!("refresh-cw"),
            Icon::SvgFile => svg!("file"),
            Icon::ImageFile => svg!("image"),
            Icon::Grid => svg!("grid-3x3"),
            Icon::Ruler => svg!("ruler"),
            Icon::Debug => svg!("bug"),
            Icon::ZoomFit => svg!("maximize"),
            Icon::MirrorVertical => svg!("square-axis-dashed-horizontal"),
            Icon::TestPatterns => svg!("flask-conical"),
            Icon::PanelLeft => svg!("panel-left"),
            Icon::PanelRight => svg!("panel-right"),
            Icon::RotateLeft => svg!("rotate-ccw"),
        }
    }
}

/// The SVG with `currentColor` replaced by `color`, so resvg does not paint
/// it black.
pub fn tinted_svg(icon: Icon, color: Color32) -> String {
    let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    icon.svg().replace("currentColor", &hex)
}

/// Renders the icon into a square image of `px` pixels.
pub fn rasterize(icon: Icon, px: u32, color: Color32) -> ColorImage {
    let px = px.max(1);
    let svg = tinted_svg(icon, color);
    let tree =
        usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("vendored Lucide SVGs parse");
    let mut pixmap = tiny_skia::Pixmap::new(px, px).expect("icon size is not zero");
    let size = tree.size();
    let transform =
        tiny_skia::Transform::from_scale(px as f32 / size.width(), px as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia stores premultiplied RGBA, as egui expects here.
    ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data())
}

#[derive(Clone, Default)]
struct TextureCache(Arc<std::sync::Mutex<HashMap<CacheKey, TextureHandle>>>);

/// Icon, size in pixels and RGBA colour.
type CacheKey = (Icon, u32, [u8; 4]);

/// The icon as a texture of `size` points in the theme's text colour. Cached
/// per context, size, colour and pixel density.
pub fn texture(ctx: &egui::Context, icon: Icon, size: f32) -> TextureHandle {
    let color = ctx.global_style().visuals.text_color();
    texture_colored(ctx, icon, size, color)
}

/// Like [`texture`], in a given colour.
pub fn texture_colored(
    ctx: &egui::Context,
    icon: Icon,
    size: f32,
    color: Color32,
) -> TextureHandle {
    let px = (size * ctx.pixels_per_point()).round().max(1.0) as u32;
    let key = (icon, px, color.to_array());
    let cache = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<TextureCache>(egui::Id::unique("flatcraft-icons"))
            .clone()
    });
    let mut map = cache
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    map.entry(key)
        .or_insert_with(|| {
            ctx.load_texture(
                format!("icon-{icon:?}-{px}"),
                rasterize(icon, px, color),
                TextureOptions::LINEAR,
            )
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_parses_and_uses_the_given_colour() {
        for icon in Icon::ALL {
            assert!(icon.svg().contains("currentColor"), "{icon:?}");
            let svg = tinted_svg(icon, Color32::from_rgb(0x2B, 0xB3, 0xA3));
            assert!(!svg.contains("currentColor"));
            let img = rasterize(icon, 24, Color32::from_rgb(0x2B, 0xB3, 0xA3));
            let lit: Vec<_> = img.pixels.iter().filter(|p| p.a() == 255).collect();
            assert!(!lit.is_empty(), "{icon:?} rendered nothing");
            assert!(
                lit.iter()
                    .all(|p| (p.r(), p.g(), p.b()) == (0x2B, 0xB3, 0xA3)),
                "{icon:?} is not in the theme colour"
            );
        }
    }
}
