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
    ZoomActual,
    Copies,
    DistributeHorizontal,
    DistributeVertical,
    Linked,
    Heart,
    Move,
    RectangularPattern,
    CircularPattern,
    Pin,
    PinOff,
    ChevronDown,
    AlignLeft,
    AlignCentre,
    AlignRight,
    AlignTop,
    AlignBottom,
    /// "Edit" group.
    Pencil,
    /// "Zoom" group.
    ZoomIn,
    /// "Panels" group.
    Panels,
    /// The Keep aspect switch.
    KeepAspect,
    /// The Each object switch.
    EachObject,
    /// "Auto arrange...", the panel (the one-click one uses Arrange).
    AutoArrangePanel,
    /// The Align directions "to the cutting area": the align icon inside a
    /// small dashed frame, drawn at render time from the Lucide file.
    AlignLeftArea,
    AlignCentreArea,
    AlignRightArea,
    AlignTopArea,
    AlignMiddleArea,
    AlignBottomArea,
}

macro_rules! svg {
    ($name:literal) => {
        include_str!(concat!("../../../../assets/icons/", $name, ".svg"))
    };
}

impl Icon {
    pub const ALL: [Icon; 67] = [
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
        Icon::ZoomActual,
        Icon::Copies,
        Icon::DistributeHorizontal,
        Icon::DistributeVertical,
        Icon::Linked,
        Icon::Heart,
        Icon::Move,
        Icon::RectangularPattern,
        Icon::CircularPattern,
        Icon::Pin,
        Icon::PinOff,
        Icon::ChevronDown,
        Icon::AlignLeft,
        Icon::AlignCentre,
        Icon::AlignRight,
        Icon::AlignTop,
        Icon::AlignBottom,
        Icon::Pencil,
        Icon::ZoomIn,
        Icon::Panels,
        Icon::KeepAspect,
        Icon::EachObject,
        Icon::AutoArrangePanel,
        Icon::AlignLeftArea,
        Icon::AlignCentreArea,
        Icon::AlignRightArea,
        Icon::AlignTopArea,
        Icon::AlignMiddleArea,
        Icon::AlignBottomArea,
    ];

    /// The plain icon a framed "to the cutting area" icon is made from.
    pub fn framed_from(self) -> Option<Icon> {
        Some(match self {
            Icon::AlignLeftArea => Icon::AlignLeft,
            Icon::AlignCentreArea => Icon::AlignCentre,
            Icon::AlignRightArea => Icon::AlignRight,
            Icon::AlignTopArea => Icon::AlignTop,
            Icon::AlignMiddleArea => Icon::Align,
            Icon::AlignBottomArea => Icon::AlignBottom,
            _ => return None,
        })
    }

    /// The vendored Lucide source (of the base icon for a framed one).
    pub fn svg(self) -> &'static str {
        self.framed_from().unwrap_or(self).lucide()
    }

    /// The vendored Lucide SVG source.
    fn lucide(self) -> &'static str {
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
            Icon::ZoomActual => svg!("percent"),
            Icon::Copies => svg!("copy-plus"),
            Icon::DistributeHorizontal => svg!("align-horizontal-distribute-center"),
            Icon::DistributeVertical => svg!("align-vertical-distribute-center"),
            Icon::Linked => svg!("link"),
            Icon::Heart => svg!("heart"),
            Icon::Move => svg!("move"),
            Icon::RectangularPattern => svg!("grid-2x2-plus"),
            Icon::CircularPattern => svg!("circle-dot"),
            Icon::Pin => svg!("pin"),
            Icon::PinOff => svg!("pin-off"),
            Icon::ChevronDown => svg!("chevron-down"),
            Icon::AlignLeft => svg!("align-start-vertical"),
            Icon::AlignCentre => svg!("align-center-vertical"),
            Icon::AlignRight => svg!("align-end-vertical"),
            Icon::AlignTop => svg!("align-start-horizontal"),
            Icon::AlignBottom => svg!("align-end-horizontal"),
            Icon::Pencil => svg!("pencil"),
            Icon::ZoomIn => svg!("zoom-in"),
            Icon::Panels => svg!("panels-top-left"),
            Icon::KeepAspect => svg!("proportions"),
            Icon::EachObject => svg!("ungroup"),
            Icon::AutoArrangePanel => svg!("layout-dashboard"),
            Icon::AlignLeftArea
            | Icon::AlignCentreArea
            | Icon::AlignRightArea
            | Icon::AlignTopArea
            | Icon::AlignMiddleArea
            | Icon::AlignBottomArea => unreachable!("framed icons are built in svg()"),
        }
    }
}

/// The colours an icon is drawn in (see [`super::tools::ICON_ROLES`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Palette {
    pub ink: Color32,
    pub accent: Color32,
    pub paper: Color32,
    pub cut: Color32,
}

impl Palette {
    /// The theme's icon colours.
    pub fn of(t: &super::theme::Tokens) -> Self {
        Self {
            ink: t.icon_ink,
            accent: t.icon_accent,
            paper: t.icon_paper,
            cut: t.icon_cut,
        }
    }

    /// One colour for everything, no paper fill (a tinted icon).
    pub fn mono(c: Color32) -> Self {
        Self {
            ink: c,
            accent: c,
            paper: Color32::TRANSPARENT,
            cut: c,
        }
    }

    fn is_mono(&self) -> bool {
        self.paper == Color32::TRANSPARENT
    }

    fn key(&self) -> [u8; 16] {
        let mut k = [0; 16];
        for (i, c) in [self.ink, self.accent, self.paper, self.cut]
            .iter()
            .enumerate()
        {
            k[i * 4..i * 4 + 4].copy_from_slice(&c.to_array());
        }
        k
    }
}

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// The child elements of a vendored Lucide file, in file order (one per
/// line in the files).
pub fn children(icon: Icon) -> Vec<&'static str> {
    let svg = icon.framed_from().unwrap_or(icon).lucide();
    svg.lines()
        .map(str::trim)
        .filter(|l| l.starts_with('<') && !l.starts_with("<svg") && !l.starts_with("</svg"))
        .collect()
}

/// The icon in the "Paper (crisp)" style: square line ends and sharp
/// joins; paper children filled with paper and outlined in the cut colour,
/// accent children in the accent colour, the rest in ink, paper first so
/// no fill covers a line. A framed icon is drawn at 60 % inside a dashed
/// frame in the cut colour, the cutting area. The Lucide files are not
/// changed, only their contents are wrapped.
pub fn styled_svg(icon: Icon, pal: &Palette) -> String {
    let base = icon.framed_from().unwrap_or(icon);
    let src = base.lucide();
    let open_end = src.find('>').expect("an svg element") + 1;
    let root = src[..open_end]
        .replace("stroke-linecap=\"round\"", "stroke-linecap=\"square\"")
        .replace("stroke-linejoin=\"round\"", "stroke-linejoin=\"miter\"")
        .replace("currentColor", &hex(pal.ink));
    let (paper, accent) = if pal.is_mono() {
        (&[][..], &[][..])
    } else {
        super::tools::icon_roles(base).unwrap_or((&[], &[]))
    };
    let kids = children(base);
    let paint = |i: usize, el: &str| {
        let attrs = if paper.contains(&i) {
            format!(" fill=\"{}\" stroke=\"{}\"", hex(pal.paper), hex(pal.cut))
        } else if accent.contains(&i) {
            format!(" stroke=\"{}\"", hex(pal.accent))
        } else {
            format!(" stroke=\"{}\"", hex(pal.ink))
        };
        let tag_end = el.find([' ', '/', '>']).unwrap_or(el.len());
        format!("{}{attrs}{}", &el[..tag_end], &el[tag_end..])
    };
    let mut body = String::new();
    for (i, el) in kids.iter().enumerate().filter(|(i, _)| paper.contains(i)) {
        body.push_str(&paint(i, el));
    }
    for (i, el) in kids.iter().enumerate().filter(|(i, _)| !paper.contains(i)) {
        body.push_str(&paint(i, el));
    }
    if icon.framed_from().is_some() {
        format!(
            "{root}<rect x=\"1\" y=\"1\" width=\"22\" height=\"22\" stroke=\"{}\" stroke-width=\"1.5\" stroke-dasharray=\"3 2\" /><g transform=\"translate(4.8 4.8) scale(0.6)\">{body}</g></svg>",
            hex(pal.cut)
        )
    } else {
        format!("{root}{body}</svg>")
    }
}

/// Renders the icon into a square image of `px` pixels.
pub fn rasterize(icon: Icon, px: u32, pal: &Palette) -> ColorImage {
    let px = px.max(1);
    let svg = snap_strokes(&styled_svg(icon, pal), px);
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

/// At small sizes (20 px and less) the 2-unit Lucide stroke becomes a
/// whole number of pixels (16 px: 1, 20 px: 2), so lines are not blurred
/// across two pixel rows. The Lucide geometry sits on a 24-unit grid, so
/// line positions cannot all land on pixel edges at 16 or 20 px; only the
/// widths are snapped.
pub fn snap_strokes(svg: &str, px: u32) -> String {
    if px > 20 {
        return svg.to_string();
    }
    let pixels = (2.0 * px as f32 / 24.0).round().max(1.0);
    let units = pixels * 24.0 / px as f32;
    svg.replacen(
        "stroke-width=\"2\"",
        &format!("stroke-width=\"{units}\""),
        1,
    )
}

#[derive(Clone, Default)]
struct TextureCache(Arc<std::sync::Mutex<HashMap<CacheKey, TextureHandle>>>);

/// Icon, size in pixels and the palette's colours.
type CacheKey = (Icon, u32, [u8; 16]);

/// The icon as a texture of `size` points in the theme's icon colours.
/// Cached per context, size, palette and pixel density.
pub fn texture(ctx: &egui::Context, icon: Icon, size: f32) -> TextureHandle {
    let t = if ctx.global_style().visuals.dark_mode {
        super::theme::Tokens::dark()
    } else {
        super::theme::Tokens::light()
    };
    texture_with(ctx, icon, size, &Palette::of(&t))
}

/// Like [`texture`], in one given colour.
pub fn texture_colored(
    ctx: &egui::Context,
    icon: Icon,
    size: f32,
    color: Color32,
) -> TextureHandle {
    texture_with(ctx, icon, size, &Palette::mono(color))
}

fn texture_with(ctx: &egui::Context, icon: Icon, size: f32, pal: &Palette) -> TextureHandle {
    let px = (size * ctx.pixels_per_point()).round().max(1.0) as u32;
    let key = (icon, px, pal.key());
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
                rasterize(icon, px, pal),
                TextureOptions::LINEAR,
            )
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAL: Color32 = Color32::from_rgb(0x2B, 0xB3, 0xA3);

    fn lit(img: &ColorImage) -> Vec<(u8, u8, u8)> {
        img.pixels
            .iter()
            .filter(|p| p.a() == 255)
            .map(|p| (p.r(), p.g(), p.b()))
            .collect()
    }

    #[test]
    fn every_icon_parses_and_a_tinted_one_uses_the_given_colour() {
        for icon in Icon::ALL {
            assert!(icon.svg().contains("currentColor"), "{icon:?}");
            assert!(!children(icon).is_empty(), "{icon:?}");
            let img = rasterize(icon, 24, &Palette::mono(TEAL));
            let lit = lit(&img);
            assert!(!lit.is_empty(), "{icon:?} rendered nothing");
            assert!(
                lit.iter().all(|p| *p == (0x2B, 0xB3, 0xA3)),
                "{icon:?} is not in the tint"
            );
        }
    }

    #[test]
    fn paper_icons_are_crisp_and_use_the_theme_roles() {
        for t in [
            super::super::theme::Tokens::dark(),
            super::super::theme::Tokens::light(),
        ] {
            let pal = Palette::of(&t);
            for icon in Icon::ALL {
                let svg = styled_svg(icon, &pal);
                assert!(svg.contains("stroke-linecap=\"square\""), "{icon:?}");
                assert!(svg.contains("stroke-linejoin=\"miter\""), "{icon:?}");
                assert!(usvg::Tree::from_str(&svg, &usvg::Options::default()).is_ok());
                let Some((paper, accent)) = super::super::tools::icon_roles(icon) else {
                    continue;
                };
                let img = rasterize(icon, 48, &pal);
                let px = lit(&img);
                let has = |c: Color32| px.contains(&(c.r(), c.g(), c.b()));
                assert!(has(pal.accent), "{icon:?}: no accent pixels");
                if !paper.is_empty() {
                    assert!(has(pal.paper), "{icon:?}: no paper fill");
                    assert!(has(pal.cut), "{icon:?}: no cut line");
                }
                assert!(!accent.is_empty());
            }
        }
    }

    #[test]
    fn small_strokes_are_whole_pixels() {
        let svg = Icon::Move.svg();
        assert!(
            snap_strokes(svg, 16).contains("stroke-width=\"1.5\""),
            "1 px at 16"
        );
        assert!(
            snap_strokes(svg, 20).contains("stroke-width=\"2.4\""),
            "2 px at 20"
        );
        assert_eq!(snap_strokes(svg, 36), svg, "large sizes stay");
    }

    #[test]
    fn rotate_arrows_are_teal() {
        let pal = Palette::of(&super::super::theme::Tokens::light());
        let teal = hex(pal.accent);
        for (icon, head) in [(Icon::Rotate, "M21 3v5h-5"), (Icon::RotateLeft, "M3 3v5h5")] {
            let svg = styled_svg(icon, &pal);
            let want = format!("<path stroke=\"{teal}\" d=\"{head}\"");
            assert!(svg.contains(&want), "{icon:?}: {svg}");
        }
    }
}
