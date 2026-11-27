//! The names and hints of the cut options (cut mode, keep and strokes)
//! the object inspector (`inspector::object`) shows, and the texts and
//! helpers the Job panel (`inspector::job`) shares with the rest of the
//! window.

use egui::{Response, Ui};
use flatcraft_core::document::{CutMode, KeepMode, StrokeMode, WeedLines};

use super::prepare::Snapshot;

/// The speed hint after a typed speed above the plotter's maximum.
pub const ELEVEN: &str = "These go to eleven.";

/// Sets the accessible name of a widget, replacing the one it gave itself.
pub fn set_name(ui: &Ui, resp: &Response, name: &str) {
    ui.ctx().accesskit_node_builder(resp.id, |node| {
        node.clear_labelled_by();
        node.set_label(name);
    });
}

/// A row label that is only painted, so the widget it names (which gets
/// the name through [`set_name`]) is the only accessible node with it.
pub fn painted_label(ui: &mut Ui, text: &str) {
    let color = ui.visuals().text_color();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.left_top(), galley, color);
}

/// The name of a cut mode in the Cut mode combo.
pub fn mode_name(m: CutMode) -> &'static str {
    match m {
        CutMode::AllLines => "All lines",
        CutMode::ObjectOutlines => "Object outlines",
        CutMode::MergedOutline => "Merged outline",
        CutMode::VisibleOutline => "Visible outline",
    }
}

/// The cut modes in the order the Cut mode combo lists them.
pub const CUT_MODES: [CutMode; 4] = [
    CutMode::ObjectOutlines,
    CutMode::AllLines,
    CutMode::MergedOutline,
    CutMode::VisibleOutline,
];

/// The short hint of a cut mode, on its entry in the Cut mode combo: what
/// it cuts and when to pick it.
pub fn mode_hint(m: CutMode) -> &'static str {
    match m {
        CutMode::ObjectOutlines => {
            "Cuts around every shape you can see, holes included, like the middle of an O. Shapes that overlap are each cut in full. The right pick for most designs."
        }
        CutMode::AllLines => {
            "Cuts every line exactly as drawn, along its middle, whatever its fill or thickness. Use it for line art, score lines or a drawing made of single strokes."
        }
        CutMode::MergedOutline => {
            "Welds overlapping shapes into one piece: one cut around all of them, none where they overlap. Every overlapping object must use this mode and the same Keep choice. The weld belongs to the object furthest back. Use it for joined script letters or a badge made of several shapes."
        }
        CutMode::VisibleOutline => {
            "Cuts only what you can see: where an object in front covers this one, that part is not cut. An edge two shapes share is cut once. Objects lower in the Objects list lie in front (Bring to front and Send to back in Arrange change that), and a skipped object still covers. Use it for layered stickers and knockouts."
        }
    }
}

/// The name of a keep mode in the Keep combo.
pub fn keep_name(k: KeepMode) -> &'static str {
    match k {
        KeepMode::Pieces => "Pieces",
        KeepMode::Stencil => "Stencil",
        KeepMode::Both => "Both",
    }
}

/// The short hint of a keep mode, on its entry in the Keep combo.
pub fn keep_hint(k: KeepMode) -> &'static str {
    match k {
        KeepMode::Pieces => {
            "Keep the shapes, like stickers. The sheet around them and their holes are waste."
        }
        KeepMode::Stencil => "Keep the sheet, like a stencil. The shapes fall out and are waste.",
        KeepMode::Both => {
            "Keep both sides of every cut: the shapes and the sheet around them. The blade never starts or overshoots there; it only turns on the spot."
        }
    }
}

/// The name of a stroke mode in the Strokes combo.
pub fn strokes_name(m: StrokeMode) -> &'static str {
    match m {
        StrokeMode::Auto => "Auto",
        StrokeMode::CentreLine => "Centre line",
        StrokeMode::Outline => "Outline",
    }
}

/// The name of a weed lines choice.
pub fn weed_lines_name(l: WeedLines) -> &'static str {
    match l {
        WeedLines::Off => "Off",
        WeedLines::Horizontal => "Horizontal",
        WeedLines::Vertical => "Vertical",
        WeedLines::Both => "Both",
    }
}

/// Why per object is dimmed.
pub const WEED_BOX_OFF: &str = "Turn on Weed box first.";
/// Why the box margin is dimmed.
pub const WEEDING_OFF: &str = "Turn on Weed box or Weed lines first.";
/// Why the weed line spacing is dimmed.
pub const WEED_LINES_OFF: &str = "Pick a direction for Weed lines first.";

/// Why weeding cuts nothing while an object keeps the shared sheet: the
/// Weeding section, the Path legend's weed row and its weed path count.
pub const SHEET_KEPT_NO_WEEDING: &str =
    "An object set to Stencil or Both keeps the sheet on this mat, so there is nothing to weed.";

/// The guess of [`super::cut::sheet_kept_now`] without a current plan:
/// an object that is cut keeps the shared sheet (Keep is Stencil or
/// Both). The plan counts only objects with a closed cut.
pub fn sheet_kept(snap: &Snapshot) -> bool {
    snap.objects
        .iter()
        .any(|o| o.visible && o.cut_enabled && o.keep != KeepMode::Pieces)
}

/// The card of the Weeding heading.
pub const WEEDING_HINT: &str = "Weeding means peeling away the vinyl you do not need after the cut, so only your design stays on the backing. A weed box and weed lines are extra cuts through that waste only: it comes off in neat pieces instead of tearing. They are cut after the design and show dashed in the Path view.";

/// The title of the Job panel's Material tips section.
pub const TIPS_HEADING: &str = "Material tips";
