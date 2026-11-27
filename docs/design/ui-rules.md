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
| `TEXT_HEADING` (`heading()`) | 14 pt semibold | Panel headings, the inspector title, the workspace tile. |
| Ribbon (`TAB_ROW`, `RIBBON_PAD`, `BIG_HEIGHT`, `GROUP_FOOTER`, ...) | as tuned | The ribbon and the Cut bar; see below. |

### Off-grid values, on purpose

- `ICON_BUTTON` 22 pt: the ribbon's compact height, so a compact button looks the same everywhere; it sits centred in a 28 pt row.
- `TEXT_BODY` 12.5 pt: egui's Body size, which every widget uses.
- The ribbon and the Cut bar: `TAB_ROW` 22, `RIBBON_PAD` (4, 3), `BIG_HEIGHT` 46, `BIG_PAD_Y` 3, `GROUP_FOOTER` 14, `COMPACT_HEIGHT` 22, `TILE_PAD` 14, `STEP_CHEVRON` 14, `LOCK_COLUMN` 14, and egui's Small style in the footers. They were tuned by eye and still come only from `metrics`.
- The inspector's scroll bar: 6 pt wide, 2 pt from the content, 4 pt from the panel's edge, so it fits the 12 pt padding.

## Rules

Each rule names where it is enforced.

- **Grid and spacing.** Panel sizes are multiples of 4 pt and spacing comes from the scale. Enforced by `metrics::tests::panel_values_sit_on_the_grid`.
- **Row height.** A panel row is 28 pt: a 24 pt control with 2 pt above and below, so two controls are 4 pt apart and nothing else is added between rows. Enforced by `blocks::row`, `blocks::panel_spacing` and `tests/design_rules.rs` `inspector_rows_share_one_height`.
- **Both panels alike.** Objects rows and inspector rows share the row height and the 12 pt padding. Enforced by `panels_share_row_height_and_padding`.
- **Sections.** A 28 pt header with a 1 pt `border` line on top; no extra gap before or after the body. Enforced by `blocks::section` and `inspector_rows_share_one_height`.
- **Columns.** The label column fits the section's longest label (64 to 112 pt, `blocks::labels`); the value widget fills the rest up to the reset slot. Enforced by `value_widgets_fill_the_value_column`.
- **Type.** Panels use only the sizes of the scale, through `metrics::body`, `secondary`, `subsection_title`, `section_title` and `heading`; secondary text is `text_dim`. Checked in review.
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
