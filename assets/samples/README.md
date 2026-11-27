# Samples

The sample projects on Home's Start page. They are original designs made for
FlatCraft from simple geometry (polygons, circles and waves). The lettering is
IBM Plex Sans SemiBold (OFL-1.1, shipped in `assets/fonts`), turned into outlines
when the samples are made, so a sample needs no font. No third-party art or
logos. They are part of FlatCraft and licensed under MPL-2.0, like the
rest of the code.

| File | What it shows |
| --- | --- |
| pantry-labels.flatcraft | Linked copies in a rectangular pattern: 12 pantry labels with a rim, a groove and a blank panel, on vinyl, cut with the Outlines last strategy. |
| iron-on-shirt.flatcraft | A striped sunset over "BORN TO MAKE" on heat transfer vinyl, stored mirrored because iron-on is cut from the back. |
| stencil.flatcraft | The letters C, U and T cut out of the sheet on the mat, with keep mode Stencil: the sheet is kept and the letters fall out. The letters have no closed counters, so only the letters fall out. |
| gift-tags.flatcraft | A circular pattern of 6 linked tags, each with a hole. |
| window-star.flatcraft | Paper cut work: an eight-point star with 25 inner shapes, cut from the inside out. |
| maker-quotes.flatcraft | Seven vinyl stickers: the six short maker quotes of the About dialog and "Eat. Sleep. Make. Repeat.", in solid capitals inside a 3 mm frame band, each author's name a separate object. |

Every cut stroke and gap in the samples is at least 1 mm wide, so they weed on
adhesive vinyl and iron-on; the test `every_cut_stroke_and_gap_is_at_least_1_mm`
checks it. The small notches IBM Plex cuts into some joins for print (ink traps)
are closed when the lettering is outlined, since vinyl has no ink to trap.

Each file is a FlatCraft project: a zip archive with `project.json`, the sources
and a `thumbnail.png` that the Start page shows. The files are written by the
test `regenerate_samples` in `crates/flatcraft/tests/samples.rs`, which holds the
designs:

    cargo nextest run -p flatcraft --test samples --run-ignored only

The test `the_bundled_samples_are_the_designs_of_this_file` fails when the
`project.json` of a file and its design no longer agree. A sample always opens as a new untitled copy; the
bundled file is never written.
