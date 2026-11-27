# UI design rules

FlatCraft's screens follow one small set of rules. The values live in one
place, `crates/flatcraft/src/ui/metrics.rs`. Use its constants for layout;
do not type a new number. Drawing geometry (a hatch, a ring, a dash, the
shape of the lock or a caret) may keep a local constant next to its drawing.

## Scale

| Token (`metrics::`) | Value | Use |
|---|---|---|
| `GRID` | 4 pt | Panel sizes are multiples of it (exceptions below). |
| `SPACE_2` .. `SPACE_24` | 2, 4, 8, 12, 16, 24 pt | All layout spacing. |
| `ROW` | 28 pt | Pitch of a panel row (a control plus the gap to the next row). |
| `CONTROL` | 24 pt | A field, list or button in a panel row. |
| `ROW_GAP` | 4 pt | Room between the controls of two rows (`ROW - CONTROL`). |
| `ICON_BUTTON` | 22 pt | Compact icon button (panel actions, Objects toggles, collapse). |
| `SECTION_HEADER` / `SUB_HEADER` | 28 / 24 pt | Section header / its Advanced part. |
| `HEADING` | 28 pt | Inspector heading: one row, the title and a dimmed note. |
| `PANEL_PAD_X` | 12 pt | Panel padding left and right. |
| `PANEL_PAD_TOP` / `PANEL_PAD_BOTTOM` | 8 / 8 pt | Panel padding on top / at the bottom. |
| `CARET_SLOT` | 16 pt | Room for a section's caret before its title. |
| `LABEL_MIN` .. `LABEL_MAX` | 64 .. 112 pt | Label column of a panel section. |
| `DOT_SLOT` | 12 pt | Slot of the change dot. |
| `INDENT` | 16 pt | Indent per group level in Objects. |
| `RIGHT_PANEL_W` / `LEFT_PANEL_W` | 296 / 224 pt | Width a side panel opens with. |
| `ICON_COMPACT` / `ICON_BIG` / `ICON_SMALL` | 16 / 24 / 12 pt | Icons in compact places / big ribbon buttons and Cut steps / chevrons and a card's close X. |
| `TEXT_BODY` (`body()`) | 12.5 pt | Body text: labels, values, rows. |
| `TEXT_SECONDARY` (`secondary()`) | 12 pt | Summaries, notes, the heading's note; always `text_dim`. |
| `TEXT_SECONDARY` semibold (`subsection_title()`) | 12 pt | Titles of Advanced parts. |
| `TEXT_SECTION` (`section_title()`) | 13 pt semibold | Section titles, hint card titles. |
| `TEXT_HEADING` (`heading()`) | 14 pt semibold | Panel headings, the inspector title, the workspace tile, card and toast titles. |
| `TEXT_SMALL` (`small()`, `caption()`) | 11 pt, `caption()` semibold | Badges ("Current", "Open now"), eyebrow labels ("GENERAL", "NEW"), footnotes. |
| `TEXT_TITLE` (`title()`) | 16 pt semibold | Titles of windows, dialogs and Settings pages. |
| `TEXT_DISPLAY` (`display()`) | 24 pt semibold | The one big line of a page: Home's "Start", the About name. |
| `ICON_BAR` (`ui::ICON_SIZE`) | 18 pt | Icons beside a text in the app bar, the ribbon's lists, the Cut steps and the timeline. |
| `WINDOW_PAD` / `CARD_PAD` | 16 / 8 pt | Room inside a window or dialog / inside a card, a toast, a bar over the canvas, a legend. |
| `WINDOW_BAR` | 48 pt | The header and the footer of the Settings window. |
| `BUTTON_MIN_W` / `CLOSE_BUTTON` / `CLOSE_INSET` / `SMALL_DIALOG_W` | 64 / 22 / 12 / 360 pt | A window's main button / a window's close x / its room to the window's edge / a one-field dialog. |
| `MARKER_W`, `line_height(ui, font)` | 8 pt, one text line | A marker dot's slot before a line of text, as tall as that line. |
| `LEGEND_SWATCH` / `HAIRLINE` | 44 x 14 / 1 pt | A legend row's line sample / a divider. |
| `ITEM_SPACING`, `BUTTON_PADDING`, `WINDOW_MARGIN`, `TIGHT_BUTTON_PADDING` | (8, 6), (8, 4), 8, (4, 2) pt | egui's base style (`theme::apply`, `theme::tight`). |
| Ribbon (`TAB_ROW`, `RIBBON_PAD`, `BIG_HEIGHT`, `GROUP_FOOTER`, ...) | as tuned | The ribbon and the Cut bar; see below. |

### Off-grid values, on purpose

- `ICON_BUTTON` 22 pt: the ribbon's compact height, so a compact button looks the same everywhere; it sits centred in a 28 pt row.
- `TEXT_BODY` 12.5 pt: egui's Body size, which every widget uses.
- The ribbon and the Cut bar: `TAB_ROW` 22, `RIBBON_PAD` (4, 3), `BIG_HEIGHT` 46, `BIG_PAD_Y` 3, `GROUP_FOOTER` 14, `COMPACT_HEIGHT` 22, `TILE_PAD` 14, `STEP_CHEVRON` 14, `LOCK_COLUMN` 14, and egui's Small style in the footers. They were tuned by eye and still come only from `metrics`.
- The inspector's scroll bar: 6 pt wide, 2 pt from the content, 4 pt from the panel's edge, so it fits the 12 pt padding.
- `ITEM_SPACING` 6 pt rows: egui's base style for popups, menus and windows. Every list outside the panels inherits it; panels and windows set their own spacing.
- `ICON_BAR` 18 pt: the app bar's and the Cut steps' icons, tuned by eye with the app bar.
- The guard's allow-list (`crates/flatcraft/tests/design_rules_allow.txt`), each with its reason there:
  - `switcher.rs` `FontId::proportional(10.5)`: the workspace tile's width is still measured with its old 10.5 pt foot, so the ribbon does not move.
  - `fonts.rs` `FontId::proportional(14.0)`: only a glyph probe, nothing is laid out.
  - `canvas.rs` `medium(9.0)`: the ruler's numbers, drawing that fits the 18 pt ruler (`canvas::RULER`).
  - `cut.rs` `medium(10.0)`: the cut order numbers on the mat, drawing next to a start dot.
  - `inspector/blocks.rs` `2.0 * pad`: a second line of wrapped action buttons keeps the row's padding, so two lines sit a row apart.

### Fixed sizes

These widths and heights stay where they are used, on purpose. Each sizes one surface or one field and is not spacing. This is the whole list; a new one is added here.

- Settings: `settings_window::SIZE` 1040 x 760, `SIDEBAR` 196, `LABEL_W` 200, a control column at most 520, the search field 220.
- Preset pages: `profiles_window::LIST_WIDTH` 176 (on the 8 grid; "Heat transfer vinyl (glitter)" fits on one line), the mat name field 160, the colour hex field 90, the override number field 96.
- Dialogs and windows: `dialog::WIDTH` 440 and `BADGE` 36 (its icon 20); the shortcuts sheet 460; `about::WIDTH` 560, `BADGE_RADIUS` 12, `TEXT_LEFT` 72 and `TEXT_AT` (banner drawing); `import_dialog::PREVIEW_SIZE` 280 and its options column 260.
- Toasts and bars: `toasts::WIDTH` 340, `MIN_WIDTH` 160, `MIN_STACK_HEIGHT` 48, the close ring `X` 20; `keep_pick::BAR_WIDTH` 420; `report::CARD_WIDTH` 360.
- Menus and cards: hover cards at most 280 (`groups::card_ui`); the context menu at least 190, a tab's menu 180, the ribbon's fold menu 220; the calculator hint at most 320.
- Home: `home::MAX_WIDTH` 1080, `NEW_CARD_HEIGHT` 76, `NEW_CARD_MIN` 230, `ROW_HEIGHT` 60, `THUMB_BOX` 44, `SAMPLE_PICTURE` 170, `SAMPLE_CARD_MIN` 260.
- Switcher list: `switcher::LIST_WIDTH` 380, `ROW_HEIGHT` 76.
- Shell: `rails::ARRANGE_W` 270, `RAIL_W` 28; `app_bar::BAR_HEIGHT` 32 and its tab sizes; `canvas::RULER` 18.
- Debug panel: `debug::TIME_W` 70, `DIR_W` 30, `HEX_W` 330.

Drawing inside a hand-painted row or card (an icon's offset, a badge's padding, a dot's radius) stays next to its drawing.

## Rules

Each rule names where it is enforced.

- **Grid and spacing.** Panel sizes are multiples of 4 pt and spacing comes from the scale. Enforced by `metrics::tests::panel_values_sit_on_the_grid`.
- **Row height.** A panel row is 28 pt: a 24 pt control with 2 pt above and below, so two controls are 4 pt apart and nothing else is added between rows. Enforced by `blocks::row`, `blocks::panel_spacing` and `tests/design_rules.rs` `inspector_rows_share_one_height`.
- **Both panels alike.** Objects rows and inspector rows share the row height and the 12 pt padding. Enforced by `panels_share_row_height_and_padding`.
- **Sections.** A 28 pt header with a 1 pt `border` line on top; no extra gap before or after the body. Enforced by `blocks::section` and `inspector_rows_share_one_height`.
- **Columns.** The label column fits the section's longest label (64 to 112 pt, `blocks::labels`); the value widget fills the rest up to the reset slot. Enforced by `value_widgets_fill_the_value_column`.
- **Layout numbers come from metrics.** Spacing, margins, grid gaps, button sizes and text sizes in `ui/` are `metrics` constants. The guard fails on a number typed into:
  - `add_space`, `inner_margin`, `outer_margin`, `Margin::same/symmetric`, a `Margin { .. }` literal;
  - `item_spacing`, `button_padding`, `interact_size` (`=`, `+=`, `-=`), a grid's or a layout's `.spacing(..)`, `min_row_height`;
  - `min_size`, `set_height`, `set_min_height`, the `vec2`/array size of `allocate_exact_size` and `add_sized`;
  - `FontId::proportional/monospace/new`, `fonts::semibold/medium`, `RichText::size`.

  A number is fine when it is 0, the `/ 2.0` of centring, or an allow-list entry with its reason. Block and line comments are skipped. Enforced by `tests/design_rules.rs` `ui_layout_numbers_come_from_metrics`; `the_guard_finds_layout_literals` checks every pattern.
- **No layout number in a `let`.** `let gap = 6.0; ui.add_space(gap);` slips past the guard. Name the value in `metrics` instead. Checked in review only.
- **Type.** The whole UI uses only the sizes of the scale, through `metrics::body`, `secondary`, `small`, `caption`, `subsection_title`, `section_title`, `heading`, `title` and `display`; secondary text is `text_dim`. The sizes are enforced by the guard above; their use is checked in review.
- **Windows and dialogs.** Content sits `WINDOW_PAD` from the edge; a title is `title()`; the close x is `CLOSE_BUTTON` with a 16 pt icon at the top right. Settings rows follow the panel row: a 24 pt control, 28 pt apart, and the lines inside one control (a choice and its explanation) 2 pt apart. Preset forms use the same 28 pt row. Checked in review and by `settings_window` tests.
- **Data lines.** A label and a value without a control (Cut stats, simulation metrics) stay one text line high, `ROW_GAP` apart. Checked in review.
- **Colour.** Theme tokens only (`theme::Tokens`); colour means state (selection, change dot, warning, grade), never decoration. The tokens' contrast is enforced by `theme::tests` (`the_change_dot_reads_on_the_panel`, `warning_text_reads_on_every_surface`, `quality_colours_read_on_the_panel`); their use is checked in review.
- **Icons.** Lucide; 16 pt in compact places, 24 pt in big ribbon buttons (`metrics::ICON_COMPACT`, `ICON_BIG`). Checked in review.
- **Corners.** Fields and cards 6 pt (`Tokens::radius`), the ribbon 2 pt (`Tokens::ribbon_radius`). Enforced by `theme::tests::corner_radii`.
- **States.** Hover is a light fill. Something that does not apply stays visible, dimmed, and its hover card says why and what to do. Enforced by `blocks::row_dimmed` and the reachability tests in `tests/inspector.rs`.
- **Hover cards.** Every row, section, action and card has one, in plain words, without an em-dash. Enforced by the card tests in `tests/inspector.rs`.
- **No reserved slots, no jumps.** A note that is usually absent goes into a header line or the hover card, never into an empty line kept free for it; a state that comes and goes ("checking...") sits in its section's header line; a section's header never moves with the selection. Enforced by `no_reserved_empty_slot_in_cut`, `sections_keep_their_place_whatever_is_selected` and `hints_are_checked_off_the_ui_thread_and_the_last_check_shows_faded`.
- **One line, one baseline.** The workspace tile's foot sits on the line of the group footers (Prepare) and of the steps' status lines (Cut), in the footers' size and colour. Enforced by `workspace_tile_aligns_with_group_footers`.

## Why some values are what they are

- The inspector keeps the panel's right padding inside its scroll area: the panel frame has no right margin and the scroll bar sits in the 12 pt padding. A bar beside the padding would have cost about 10 pt of every row.
- `HEADING` is one row. The note after the title holds only the group and the linked copies, so it is often empty; one line that is always there cannot jump.
- The panel heading size is 14 pt (the most used semibold size), so the inspector title went from 16 to 14 pt.
