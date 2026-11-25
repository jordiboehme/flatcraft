# Samples

The sample projects on Home's Start page. They are original designs made for
FlatCraft from simple geometry (polygons, circles and waves). No third-party art,
fonts or logos. They are part of FlatCraft and licensed under MPL-2.0, like the
rest of the code.

| File | What it shows |
| --- | --- |
| sticker-sheet.flatcraft | Linked copies in a rectangular pattern: 12 daisies. |
| stencil.flatcraft | The letters C, U and T cut out of a sheet. The letters have no closed counters, so only the letters fall out. |
| gift-tags.flatcraft | A circular pattern of 6 linked tags, each with a hole. |
| window-star.flatcraft | Paper cut work: an eight-point star with 25 inner shapes, cut from the inside out. |

Each file is a FlatCraft project: a zip archive with `project.json`, the sources
and a `thumbnail.png` that the Start page shows. The files are written by the
test `regenerate_samples` in `crates/flatcraft/tests/samples.rs`, which holds the
designs:

    cargo nextest run -p flatcraft --test samples --run-ignored only

The test `the_bundled_samples_are_the_designs_of_this_file` fails when the
`project.json` of a file and its design no longer agree. A sample always opens as a new untitled copy; the
bundled file is never written.
