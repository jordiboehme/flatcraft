//! The one source of FlatCraft's layout scale: spacing, heights, panel
//! padding, the label column, icon sizes and the type scale. The rules
//! behind these values are in `docs/design/ui-rules.md`.
//!
//! Layout spacing comes from here. Drawing geometry (a hatch, a ring, a
//! dash, the lock's shape) may keep a local constant next to its drawing.

use egui::{FontId, Vec2, vec2};

use super::fonts;

// ---------------------------------------------------------------- grid

/// The grid: every panel size is a multiple of it.
pub const GRID: f32 = 4.0;

/// The spacing scale, in points. Layout spacing uses only these.
pub const SPACE_2: f32 = 2.0;
pub const SPACE_4: f32 = 4.0;
pub const SPACE_8: f32 = 8.0;
pub const SPACE_12: f32 = 12.0;
pub const SPACE_16: f32 = 16.0;
pub const SPACE_24: f32 = 24.0;

/// The whole scale, smallest first (the guard test checks against it).
pub const SPACING: [f32; 6] = [SPACE_2, SPACE_4, SPACE_8, SPACE_12, SPACE_16, SPACE_24];

// ---------------------------------------------------------------- panels

/// The pitch of a panel row: its control plus the gap to the next row.
pub const ROW: f32 = 28.0;
/// The height of a control in a panel row (a field, a list, a button).
pub const CONTROL: f32 = 24.0;
/// The room between the controls of two rows.
pub const ROW_GAP: f32 = ROW - CONTROL;
/// A compact icon button (a panel's small actions, the Objects toggles).
pub const ICON_BUTTON: f32 = 22.0;
/// A section header in a panel.
pub const SECTION_HEADER: f32 = 28.0;
/// The header of a section's Advanced part.
pub const SUB_HEADER: f32 = 24.0;
/// The heading of the inspector: one row, the title and a dimmed note.
pub const HEADING: f32 = ROW;
/// The room left and right of a panel's content.
pub const PANEL_PAD_X: f32 = SPACE_12;
/// The room above a panel's content.
pub const PANEL_PAD_TOP: f32 = SPACE_8;
/// The room under a panel's content.
pub const PANEL_PAD_BOTTOM: f32 = SPACE_8;
/// The label column of a panel section: as wide as its longest label,
/// within these bounds.
pub const LABEL_MIN: f32 = 64.0;
pub const LABEL_MAX: f32 = 112.0;
/// Room left of a section title for its caret.
pub const CARET_SLOT: f32 = SPACE_16;
/// The slot of the change dot between a label and its value.
pub const DOT_SLOT: f32 = SPACE_12;
/// How far a member's row is indented per group level in Objects.
pub const INDENT: f32 = SPACE_16;
/// The width a side panel opens with.
pub const RIGHT_PANEL_W: f32 = 296.0;
pub const LEFT_PANEL_W: f32 = 224.0;

/// The most of the window's height the fields of a tool card take; more
/// fields scroll inside the card, so its title and buttons stay in sight.
pub const TOOL_CARD_FIELDS_SHARE: f32 = 0.5;

/// The inner margin of a side panel.
pub fn panel_margin() -> egui::Margin {
    egui::Margin {
        left: PANEL_PAD_X as i8,
        right: PANEL_PAD_X as i8,
        top: PANEL_PAD_TOP as i8,
        bottom: PANEL_PAD_BOTTOM as i8,
    }
}

/// The label column for labels whose widest one is `widest` points.
pub fn label_column(widest: f32) -> f32 {
    (widest + SPACE_8).ceil().clamp(LABEL_MIN, LABEL_MAX)
}

// ---------------------------------------------------------------- icons

/// Icons in compact places: rows, toggles, small buttons.
pub const ICON_COMPACT: f32 = 16.0;
/// Icons in the ribbon's big buttons and the Cut steps.
pub const ICON_BIG: f32 = 24.0;
/// Small icons inside a control: chevrons, a card's close X.
pub const ICON_SMALL: f32 = 12.0;
/// Icons beside a text in the app bar, the ribbon's lists, the Cut steps
/// and the timeline (`ui::ICON_SIZE`). Off the scale on purpose: tuned by
/// eye with the app bar (see docs/design/ui-rules.md).
pub const ICON_BAR: f32 = 18.0;

// ---------------------------------------------------------------- type

/// Body text: values, labels, rows (egui's Body style).
pub const TEXT_BODY: f32 = 12.5;
/// Secondary text: summaries, notes, a heading's second line. Always in
/// `text_dim`.
pub const TEXT_SECONDARY: f32 = 12.0;
/// A section title (semibold).
pub const TEXT_SECTION: f32 = 13.0;
/// A panel heading (semibold): "Objects", the inspector's title, the
/// workspace tile's name.
pub const TEXT_HEADING: f32 = 14.0;

pub fn body() -> FontId {
    FontId::proportional(TEXT_BODY)
}

pub fn secondary() -> FontId {
    FontId::proportional(TEXT_SECONDARY)
}

pub fn section_title() -> FontId {
    fonts::semibold(TEXT_SECTION)
}

/// A sub-section title (an Advanced part): secondary size, semibold.
pub fn subsection_title() -> FontId {
    fonts::semibold(TEXT_SECONDARY)
}

pub fn heading() -> FontId {
    fonts::semibold(TEXT_HEADING)
}

/// Small text: badges ("Current", "Open now") and the eyebrow labels
/// over a group of items ("GENERAL", "WORKSPACE").
pub const TEXT_SMALL: f32 = 11.0;
/// The title of a window, a dialog or a page (semibold).
pub const TEXT_TITLE: f32 = 16.0;
/// The one big line of a page: Home's greeting, the About name (semibold).
pub const TEXT_DISPLAY: f32 = 24.0;

pub fn small() -> FontId {
    FontId::proportional(TEXT_SMALL)
}

/// A badge or an eyebrow label: small, semibold.
pub fn caption() -> FontId {
    fonts::semibold(TEXT_SMALL)
}

pub fn title() -> FontId {
    fonts::semibold(TEXT_TITLE)
}

pub fn display() -> FontId {
    fonts::semibold(TEXT_DISPLAY)
}

/// The height of one line of `font`: a marker dot sits centred on it.
pub fn line_height(ui: &egui::Ui, font: &FontId) -> f32 {
    ui.fonts_mut(|f| f.row_height(font))
}

// ---------------------------------------------------------------- windows

/// The room around the content of a window or a dialog.
pub const WINDOW_PAD: f32 = SPACE_16;
/// The room around the content of a card: a hint card, a toast, a bar
/// over the canvas, a legend.
pub const CARD_PAD: f32 = SPACE_8;
/// The header and the footer bar of the Settings window.
pub const WINDOW_BAR: f32 = 48.0;
/// The narrowest a window's main button gets (OK, Done).
pub const BUTTON_MIN_W: f32 = 64.0;
/// The width of a small dialog with one field (Save as, Delete).
pub const SMALL_DIALOG_W: f32 = 360.0;
/// The slot of a marker dot before a line of text (a severity, a grade).
pub const MARKER_W: f32 = SPACE_8;
/// The swatch of a legend row: a line sample on the mat colour.
pub const LEGEND_SWATCH: Vec2 = vec2(44.0, 14.0);
/// A hairline divider.
pub const HAIRLINE: f32 = 1.0;
/// A window's close button (the x at its top right).
pub const CLOSE_BUTTON: f32 = ICON_BUTTON;
/// Room between a window's or a dialog's edge and its close button.
pub const CLOSE_INSET: f32 = SPACE_12;

// ---------------------------------------------------------------- base style

/// egui's spacing outside the panels: popups, menus, windows. The 6 pt
/// row gap is egui's own look and is off the scale on purpose (see
/// docs/design/ui-rules.md).
pub const ITEM_SPACING: Vec2 = vec2(SPACE_8, 6.0);
/// The padding of a button outside the ribbon.
pub const BUTTON_PADDING: Vec2 = vec2(SPACE_8, SPACE_4);
/// The inner margin of a window and of a menu.
pub const WINDOW_MARGIN: f32 = SPACE_8;
/// The ribbon's tighter button padding (`theme::tight`).
pub const TIGHT_BUTTON_PADDING: Vec2 = vec2(SPACE_4, SPACE_2);

// ---------------------------------------------------------------- ribbon

// The ribbon and the Cut bar keep the values they were tuned to by eye; some
// are off the 4 pt grid on purpose (see docs/design/ui-rules.md).

/// The tab row above the ribbon panel.
pub const TAB_ROW: f32 = 22.0;
/// The ribbon panel's padding (x, y).
pub const RIBBON_PAD: Vec2 = vec2(4.0, 3.0);
/// The row of pinned buttons, as tall as a big button. Two compact
/// switches ([`COMPACT_HEIGHT`]) stack in it.
pub const BIG_HEIGHT: f32 = 46.0;
/// The narrowest a big button, a tile or a Cut step gets.
pub const BIG_MIN_WIDTH: f32 = 40.0;
/// Room left and right of a big button's label, and of a tile's title.
pub const BIG_PAD_X: f32 = 4.0;
/// Room above a big button's icon and under its label.
pub const BIG_PAD_Y: f32 = 3.0;
/// The group button (footer) under the pinned buttons.
pub const GROUP_FOOTER: f32 = 14.0;
/// Room between the pinned buttons and the footer: none.
pub const GROUP_GAP: f32 = 0.0;
/// Room between two buttons of the ribbon and of the Cut bar.
pub const BUTTON_GAP: f32 = SPACE_4;
/// The ribbon's tools row on every tab, and the Cut stage bar.
pub const TOOLS_ROW: f32 = BIG_HEIGHT + GROUP_GAP + GROUP_FOOTER;
/// A compact button: two of them stack inside [`BIG_HEIGHT`].
pub const COMPACT_HEIGHT: f32 = 22.0;
/// The padding of a small ribbon button around its icon.
pub const SMALL_PADDING: Vec2 = vec2(2.0, 2.0);
/// The gap of the Position and size grid on the ribbon.
pub const SIZE_GRID_GAP: Vec2 = vec2(4.0, 4.0);
/// The inner margin of a ribbon number field.
pub const FIELD_MARGIN: Vec2 = vec2(4.0, 2.0);
/// The ribbon's lock column (lock and bracket arms).
pub const LOCK_COLUMN: f32 = 14.0;
/// The search field's width on the ribbon.
pub const SEARCH_WIDTH: f32 = 200.0;
/// The chevron between two Cut steps.
pub const STEP_CHEVRON: f32 = 14.0;
/// The workspace tile: the tab row and the ribbon panel.
pub const TILE_HEIGHT: f32 = TAB_ROW + RIBBON_PAD.y + TOOLS_ROW + RIBBON_PAD.y;
/// Room between the workspace tile's edge and its text.
pub const TILE_PAD: f32 = 14.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_values_sit_on_the_grid() {
        for v in [
            ROW,
            CONTROL,
            ROW_GAP,
            SECTION_HEADER,
            SUB_HEADER,
            HEADING,
            PANEL_PAD_X,
            PANEL_PAD_TOP,
            PANEL_PAD_BOTTOM,
            LABEL_MIN,
            LABEL_MAX,
            DOT_SLOT,
            INDENT,
            RIGHT_PANEL_W,
            LEFT_PANEL_W,
        ] {
            assert_eq!(v % GRID, 0.0, "{v}");
        }
        assert_eq!(ROW_GAP, SPACE_4);
    }

    #[test]
    fn the_label_column_stays_in_its_bounds() {
        assert_eq!(label_column(0.0), LABEL_MIN);
        assert_eq!(label_column(500.0), LABEL_MAX);
        assert_eq!(label_column(80.0), 88.0);
    }
}
