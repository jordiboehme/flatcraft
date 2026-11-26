# Icons

Lucide icons (https://github.com/lucide-icons/lucide), tag 1.79.0.
Licensed under the ISC License, see LICENSE (copied from the same tag).

Each file is icons/<name>.svg from
https://raw.githubusercontent.com/lucide-icons/lucide/1.79.0/icons/<name>.svg, unchanged.

| App icon | Lucide name |
| --- | --- |
| Import | import |
| Trace | spline |
| Duplicate | copy |
| Delete | trash |
| Scale | scaling |
| Rotate | rotate-cw |
| Mirror | square-axis-dashed-vertical (formerly flip-horizontal) |
| Align, Align middle | align-center-horizontal |
| Arrange | layout-grid |
| Undo | undo-2 |
| Redo | redo-2 |
| Save | save |
| Home | house |
| Settings | settings |
| Help | circle-question-mark |
| Plug | plug |
| Play | play |
| Send | send |
| Cut | scissors |
| Prepare | pencil-ruler |
| Search | search |
| Eye | eye |
| EyeOff | eye-off |
| Plus | plus |
| Close | x |
| Broken link | link-2-off |
| Reload | refresh-cw |
| SVG file | file |
| Image file | image |
| Grid | grid-3x3 |
| Rulers | ruler |
| Debug | bug |
| Zoom to fit | maximize |
| Mirror vertical | square-axis-dashed-horizontal |
| Test patterns | flask-conical |
| Objects panel | panel-left |
| Settings panel | panel-right |
| Rotate left | rotate-ccw |
| Zoom 100% | percent |
| Copies | copy-plus |
| Distribute horizontally | align-horizontal-distribute-center |
| Distribute vertically | align-vertical-distribute-center |
| Linked copy | link |
| Heart (About dialog) | heart |
| Move / Copy | move |
| Rectangular pattern | grid-2x2-plus |
| Circular pattern | circle-dot |
| Pin to the ribbon | pin |
| Unpin from the ribbon | pin-off |
| Drop-down | chevron-down |
| Chevron left (collapse or open a side panel) | chevron-left |
| Chevron right (collapse or open a side panel) | chevron-right |
| Align left | align-start-vertical |
| Align centre | align-center-vertical |
| Align right | align-end-vertical |
| Align top | align-start-horizontal |
| Align bottom | align-end-horizontal |
| Edit group | pencil |
| Zoom group | zoom-in |
| Panels group | panels-top-left |
| Each object on (each on its own) | ungroup |
| Each object off (one group) | group |
| Auto arrange... (panel) | layout-dashboard |
| Warning (dialog badge) | triangle-exclamation-point |
| Info (dialog badge) | info |
| Error (dialog badge) | circle-x |
| Appearance (Settings) | palette |
| Agents (Settings) | bot |
| Shortcuts (Settings) | keyboard |
| Plotters (Settings) | printer |
| Materials (Settings) | layers |
| New project (Home) | file-plus |
| Open (Home) | folder-open |
| Thumbs up (How did it go?) | thumbs-up |
| Thumbs down (How did it go?) | thumbs-down |

The Align directions "to the cutting area" use the Align icons above. The app draws
them smaller inside a dashed frame at render time; the files are not changed.

The SVGs draw with currentColor. The files stay unchanged; the app colours them at render
time in the "Paper (crisp)" style (`crates/flatcraft/src/ui/icons.rs`, `styled_svg`):

- Line caps are square and joins are miter, so the icons look crisp.
- Each icon has a colour role entry (`ICON_ROLES` in `crates/flatcraft/src/ui/tools.rs`):
  the paper children (the things that get cut) are filled with the theme's paper colour
  and outlined in the cut colour, one accent part (what happens) uses the accent colour,
  and the rest uses the ink colour. Paper children are drawn first, so no fill covers a line.
- The colours are theme tokens (`icon_paper`, `icon_cut`, `icon_accent`, `icon_ink`) for
  light and dark.
- At 20 px and less the 2-unit stroke is snapped to a whole number of pixels.
- Icons without a role entry (app bar, pin, chevron) are drawn in the ink colour, and a
  tinted icon uses one colour.
