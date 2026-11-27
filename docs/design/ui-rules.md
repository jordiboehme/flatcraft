# UI design rules

FlatCraft's screens follow one small set of rules. The values live in one
place, `crates/flatcraft/src/ui/metrics.rs`. Use its constants for layout;
do not type a new number. Drawing geometry (a hatch, a ring, a dash, the
shape of the lock) may keep a local constant next to its drawing.

## Scale

| Token (`metrics::`) | Value | Use |
|---|---|---|
| `GRID` | 4 pt | Every panel size is a multiple of it. |
| `SPACE_2` .. `SPACE_24` | 2, 4, 8, 12, 16, 24 pt | All layout spacing. |
| `ROW` | 28 pt | Pitch of a panel row (a control plus the gap to the next row). |
| `CONTROL` | 24 pt | A field, list or button in a panel row. |
| `ROW_GAP` | 4 pt | Room between the controls of two rows (`ROW - CONTROL`). |
| `ICON_BUTTON` | 22 pt | Compact icon button (panel actions, Objects toggles, collapse). |
| `SECTION_HEADER` / `SUB_HEADER` | 28 / 24 pt | Section header / its Advanced part. |
| `HEADING` | 36 pt | Inspector heading: a title line and one line under it. |
| `PANEL_PAD_X` / `PANEL_PAD_TOP` | 12 / 8 pt | Panel padding left and right / on top. |
| `LABEL_MIN` .. `LABEL_MAX` | 64 .. 112 pt | Label column of a panel section. |
| `DOT_SLOT` | 12 pt | Slot of the change dot. |
| `RIGHT_PANEL_W` / `LEFT_PANEL_W` | 296 / 224 pt | Width a side panel opens with. |
| `ICON_COMPACT` / `ICON_BIG` / `ICON_CHEVRON` | 16 / 24 / 12 pt | Icons in compact places / big ribbon buttons and Cut steps / chevrons. |
| `TEXT_BODY` | 12.5 pt | Body text: labels, values, rows. |
| `TEXT_SECONDARY` | 12 pt | Summaries, notes, a heading's second line; always `text_dim`. |
| `TEXT_SECTION` | 13 pt semibold | Section titles. |
| `TEXT_HEADING` | 14 pt semibold | Panel headings, the inspector title, the workspace tile. |
| Ribbon (`TAB_ROW`, `RIBBON_PAD`, `BIG_HEIGHT`, `GROUP_FOOTER`, ...) | as tuned | The ribbon and the Cut bar; see below. |

The ribbon and the Cut bar keep the values they were tuned to by eye (a 22 pt
tab row, 3 pt padding, 46 pt big buttons, egui's Small style in the
footers). Some are off the 4 pt grid on purpose; they still come only from
`metrics`.

## Rules

Each rule names where it is enforced.

- **Grid and spacing.** Panel sizes are multiples of 4 pt and spacing comes from the scale. Enforced by `metrics::tests::panel_values_sit_on_the_grid`.
- **Row height.** A panel row is 28 pt: a 24 pt control with 2 pt above and below, so two controls are 4 pt apart and nothing else is added between rows. Enforced by `blocks::row`, `blocks::panel_spacing` and `tests/design_rules.rs` `inspector_rows_share_one_height`.
- **Both panels alike.** Objects rows and inspector rows share the row height and the 12 pt padding. Enforced by `panels_share_row_height_and_padding`.
- **Sections.** A 28 pt header with a 1 pt `border` line on top; no extra gap before or after the body. Enforced by `blocks::section` (`metrics::SECTION_HEADER`).
- **Columns.** The label column fits the section's longest label (64 to 112 pt, `blocks::labels`); the value widget fills the rest up to the reset slot. Enforced by `value_widgets_fill_the_value_column`.
- **Type.** Only the four sizes of the scale in panels; secondary text is `text_dim`. Enforced by `metrics::body`, `secondary`, `section_title` and `heading`.
- **Colour.** Theme tokens only (`theme::Tokens`). Colour means state (selection, change dot, warning, grade), never decoration. Enforced in review.
- **Icons.** Lucide; 16 pt in compact places, 24 pt in big ribbon buttons. Enforced by `metrics::ICON_COMPACT` and `ICON_BIG`.
- **Corners.** Fields and cards 6 pt (`Tokens::radius`), the ribbon 2 pt (`Tokens::ribbon_radius`).
- **States.** Hover is a light fill. Something that does not apply stays visible, dimmed, and its hover card says why and what to do. Enforced by `blocks::row_dimmed` and the reachability tests in `tests/inspector.rs`.
- **Hover cards.** Every row, section, action and card has one, in plain words, without an em-dash. Enforced by the card tests in `tests/inspector.rs`.
- **No reserved slots, no jumps.** A note that is usually absent goes into a header line or the hover card, never into an empty line kept free for it; a section's header never moves with the selection. Enforced by `no_reserved_empty_slot_in_cut` and `sections_keep_their_place_whatever_is_selected`.
- **One line, one baseline.** The workspace tile's foot sits on the line of the group footers (Prepare) and of the steps' status lines (Cut). Enforced by `workspace_tile_foot_sits_on_the_footer_line`.

## Why some values are what they are

- The inspector keeps the panel's right padding inside its scroll area: the panel frame has no right margin and a thin floating scroll bar (4 + 4 + 4 pt) sits in the 12 pt padding. A wider bar would have cost about 10 pt of every row.
- `HEADING` is 36 pt, not 40: a 20 pt title line and a 16 pt second line. The second line holds only the group and the linked copies, so it is often empty; it keeps its height so nothing below jumps.
- The panel heading size is 14 pt (the most used semibold size), so the inspector title went from 16 to 14 pt.
