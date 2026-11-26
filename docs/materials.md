# Built-in materials

"Measure twice, cut once." FlatCraft ships starting values for the most common materials, so a first cut does not begin with a blank form. They are starting values, not promises: blades wear, batches differ, and a test cut is cheaper than a ruined sheet. The Job card says "Start with a test cut" for every material you have not cut yet.

This page lists every built-in material, the values we picked, the range the public sources gave, and why we picked what we picked. The values are written in our own words. No table was copied, and no brand or product name is used in the presets or on this page; the sources are listed by address only.

## The scale

- Speed: 1 (slowest) to 10.
- Force: 1 to 33. It is a scale from the plotter's lightest to its strongest blade pressure (about 300 grams at the top), not a unit.
- Blade: the number on the manual ratchet blade, 1 to 10. Each step shows about 0.1 mm more blade. FlatCraft never sends it: you turn the blade by hand, and the job settings show the number as a reminder (`blade_depth_hint`).
- Passes: how often the blade runs over each line.
- Mat tack: how sticky the cutting mat should be (light, standard or strong). A recommendation only; it shows in the material card and in the Profiles window.
- Backing sheet: the material sits on a liner, like vinyl. Small loose parts stay on it until transfer tape lifts them. Paper and card have none, so the middle of an O falls out.

Many sources are written for newer machines with an automatic blade. Their force and blade numbers use the same scale, and we use them as a second opinion next to the sources written for manual blades.

## How we pick a value

1. Collect the values for the material from several public sources (below).
2. Where the sources agree, take that value.
3. Where they disagree, take the gentler end of the range most sources share: less blade and less force. A cut that does not go through costs one more pass; a cut that goes too deep scores the mat and dulls the blade. A single value far below or above all the others does not set the start.
4. For speed, take the slower side of the common range: slow cuts tear less at corners and in small details.
5. Overcut stays at FlatCraft's default of 0.5 mm, and track enhancing stays off (it is for long material fed without a mat).

## The presets

None of these values was checked on a real cut yet. Glitter and flock iron-on are the least certain: their sources disagree the most.

| Id | Name | Group | Speed | Force | Passes | Blade | Mat tack | Backing |
|---|---|---|---|---|---|---|---|---|
| `vinyl` | Adhesive vinyl (permanent) | Vinyl | 5 | 8 | 1 | 1 | standard | yes |
| `vinyl-removable` | Adhesive vinyl (removable) | Vinyl | 5 | 8 | 1 | 1 | standard | yes |
| `heat-transfer-vinyl` | Heat transfer vinyl (smooth) | Iron-on | 5 | 4 | 1 | 2 | standard | yes |
| `heat-transfer-vinyl-glitter` | Heat transfer vinyl (glitter) | Iron-on | 4 | 18 | 1 | 3 | standard | yes |
| `heat-transfer-vinyl-flock` | Heat transfer vinyl (flock) | Iron-on | 4 | 14 | 1 | 3 | standard | yes |
| `sticker-paper` | Sticker paper | Paper and card | 4 | 14 | 1 | 2 | light | yes |
| `copy-paper` | Copy paper | Paper and card | 5 | 10 | 1 | 2 | light | no |
| `cardstock` | Cardstock (light, about 160-200 g/m2) | Paper and card | 4 | 14 | 1 | 3 | light | no |
| `cardstock-heavy` | Cardstock (heavy, about 250-300 g/m2) | Paper and card | 2 | 33 | 2 | 7 | strong | no |
| `kraft-paper` | Kraft paper | Paper and card | 3 | 20 | 1 | 3 | light | no |
| `vellum` | Vellum | Paper and card | 3 | 19 | 1 | 1 | light | no |
| `stencil-film` | Stencil and masking film | Film and other | 5 | 7 | 1 | 1 | standard | yes |
| `window-cling` | Window cling | Film and other | 5 | 8 | 1 | 1 | light | yes |
| `magnet-sheet` | Printable magnet sheet (thin) | Film and other | 1 | 24 | 1 | 4 | strong | no |

The ids `vinyl`, `heat-transfer-vinyl`, `copy-paper` and `cardstock` are the four presets of the first milestone. They keep their ids, so projects, the cut log and your own presets made from them keep working; only their names and values changed.

Each preset also carries one or two tips (which side goes up, how to weed). The picker shows them when you hover a material, and the Job card shows them under "Material tips". Your own notes on a preset stay separate and are never overwritten.

## Per material

The source numbers (S1 and so on) point to the list at the end.

### `vinyl` - Adhesive vinyl (permanent)

- Found: blade 1 to 2, force 8 to 15, speed 5 (S1, S4, S5, S9, S10). Most sources give blade 1 and force 8 to 11.
- Picked: blade 1, force 8, speed 5. The gentle end of where most agree.
- Mat: sources split between light and standard (S16); standard keeps a slippery liner from shifting.

### `vinyl-removable` - Adhesive vinyl (removable)

- Found: blade 1, force 8 to 10, speed 5 (S1, S4, S5); one source says it wants a little less pressure than permanent vinyl (S10).
- Picked: blade 1, force 8, speed 5. The tip says to go one or two lower if the backing shows deep lines.

### `heat-transfer-vinyl` - Heat transfer vinyl (smooth)

- Found: blade 2 (all sources), force 2 to 6, speed 4 to 8 (S1, S2, S4). One maker's chart gives force 2 for its thinnest film; most give 4 to 6.
- Picked: blade 2, force 4, speed 5.
- Mirror: iron-on is cut from the back with the clear, shiny carrier down, so the design is cut mirrored (S3, S11). The tip says "Mirror the design and cut with the shiny carrier side down", and the Cut stage reminds you, without blocking, when nothing is mirrored.

### `heat-transfer-vinyl-glitter` - Heat transfer vinyl (glitter)

- Found: blade 3 to 5, force 11 to 33, speed 4 to 5 (S1, S2, S4, S10). The manufacturer's guide says force 33 at blade 3; two shop charts say 18 to 21 at blade 4; one automatic-blade row says 11.
- Picked: blade 3, force 18, speed 4. The gentle end of the middle group. Glitter is the toughest iron-on: expect to raise the force after the test cut.
- Least certain: the sources are far apart (force 11 to 33), so this is one of the two least certain presets. Do the test cut.

### `heat-transfer-vinyl-flock` - Heat transfer vinyl (flock)

- Found: blade 2 to 4, force 14 to 33, speed 4 to 5, one or two passes (S1, S2, S4, S10, S11).
- Picked: blade 3, force 14, speed 4, one pass.
- Least certain: the sources are far apart (force 14 to 33, one or two passes), so this is one of the two least certain presets. Do the test cut.

### `sticker-paper` - Sticker paper

- Found for a cut through the sticker layer only (a kiss cut): blade 2 to 3, force 14 to 20, speed 4 to 8 (S7, S8, S15). The manufacturer's value (blade 3, force 33, two passes) is for cutting through the whole sheet, so it does not count here.
- Picked: blade 2, force 14, speed 4.

### `copy-paper` - Copy paper

- Found: blade 2 to 3, force 15 for slightly heavier office paper, speed 5 to 10 (S6, S11); the plotter's maker says thin paper needs a low force (S12).
- Picked: blade 2, force 10, speed 5. Plain 80 g/m2 paper is thinner than the paper in S6, so we start below it.

### `cardstock` - Cardstock (light, about 160-200 g/m2)

- Found: blade 3 to 6, force 10 to 29, speed 3 to 10 (S1, S4, S6, S7, S9, S11). A reader with this plotter model says its software suggests blade 3 for card (S7).
- Picked: blade 3, force 14, speed 4.

### `cardstock-heavy` - Cardstock (heavy, about 250-300 g/m2)

- Found: blade 7, force 33, speed 1 to 3, one or two passes (S6, S13, S14).
- Picked: blade 7, force 33, speed 2, two passes. Every source is at the top of the force scale, so the second pass is what finishes the cut; two of three sources use it.
- Mat: strong, so the stiff sheet does not shift (S16).

### `kraft-paper` - Kraft paper

- Found: blade 3, force 20 to 30, speed 3 (S1, S8).
- Picked: blade 3, force 20, speed 3.

### `vellum` - Vellum

- Found: blade 1, force 19, speed 6 (S1); paper cutters slow down for vellum (S7); paper wants blade 2 to 3 (S11).
- Picked: blade 1, force 19, speed 3. Vellum cracks along fast curves, so the speed is lower than the source.

### `stencil-film` - Stencil and masking film

- Found for adhesive stencil film on a liner: blade 1 to 2, force 4 to 10, speed 5 to 8 (S1, S4, S5, S9).
- Picked: blade 1, force 7, speed 5.
- Thick reusable stencil sheets without a liner are a different material: sources give blade 3 to 6, force 10 to 33, slow (S1, S9). Make your own preset from this one for those.

### `window-cling` - Window cling

- Found: blade 1 to 2, force 8 to 14, speed 5 to 8, low pressure because it stretches (S1, S10).
- Picked: blade 1, force 8, speed 5.

### `magnet-sheet` - Printable magnet sheet (thin)

- Can this plotter cut it? Yes, for thin printable sheets of about 0.3 mm: the sources give blade 4 to 6 and force 15 to 33 (S1, S4), inside the blade's 10 steps and the force scale. Thicker sheets (0.5 mm and more) need more than the manual blade and the top force give, so they are left out.
- Found: blade 4, force 15 to 30, speed 1 to 5 (S1, S4); an adhesive-backed magnet sheet needs blade 6, force 33 and two passes (S1).
- Picked: blade 4, force 24, speed 1, one pass. Below the maker's 30, above the single 15 for a newer machine. The tip says to add a pass instead of more force.
- Backing: none. A printable magnet sheet is one layer, so a loose middle falls out like with card.

## Mat tack

- Light: copy paper, vellum, light card, sticker paper, kraft paper, window cling. Thin paper tears when a strong mat holds it (S16).
- Standard: vinyl, iron-on and stencil film. One source puts vinyl on a light mat, another on a standard one (S16); standard holds a slippery liner better.
- Strong: heavy card and magnet sheets, which would shift on a weaker mat (S16).

## Sources

- S1: the plotter maker's materials guide, with suggested blade, speed, force and passes per material. https://www.silhouette101.com/archives/materials-guide
- S2: an iron-on maker's cutter chart for several craft cutters. https://assets.stahls.com/stahls/content/pdf/education/craft-cutter-settings-stahls-htv.pdf
- S3: an iron-on maker's test cut guide: carrier side down, always mirror. https://siserstore.com/pages/how-to-test-cut-siser-htv-silhouette-cameo
- S4: a vinyl shop's settings for a newer machine with an automatic blade. https://www.gmcrafts.co.uk/silhouette-cameo-4-cut-settings/
- S5: the same shop's settings for a smaller machine. https://www.gmcrafts.co.uk/silhouette-portrait-3-cut-settings-2/
- S6: a maker blog on paper and card weights. https://www.silhouetteschoolblog.com/2018/08/best-silhouette-cut-settings-for-paper.html
- S7: a maker blog on cutting card, with readers' settings. https://www.silhouetteschoolblog.com/2016/09/cutting-cardstock-with-silhouette-cameo.html
- S8: a maker blog on kiss cutting sticker sheets. https://www.silhouetteschoolblog.com/2020/06/kiss-cut-stickers-on-silhouette-cameo-3.html
- S9: a stencil guide with settings for vinyl, stencil film and card. https://www.stenciliq.app/guides/cricut-silhouette-stencil-guide
- S10: a blade settings guide (pressure in grams; we use only its blade depths and its comparisons). https://www.printablepress.com/vinyl-cutting-blade-settings-guide/
- S11: a public library's guide: blade depth bands, carrier side down, mirror for iron-on. https://www.sandiego.gov/sites/default/files/silhouette_guide.pdf
- S12: the plotter maker's help on blade, force and speed: find the lowest blade and force that cut. https://silhouetteamerica.freshdesk.com/support/solutions/articles/35000206256
- S13: a forum thread on very thick card. https://www.reddit.com/r/silhouettecutters/comments/pzikx9/blade_and_cut_settings_for_very_thick_cardstock/
- S14: a forum thread on 300 g/m2 paper. https://www.reddit.com/r/silhouettecutters/comments/1jd6s3c/tips_for_cutting_large_500x700mm_300g_paper/
- S15: a forum thread on kiss cutting sticker sheets. https://www.reddit.com/r/stickers/comments/1o16kds/what_are_the_correct_silhouette_settings_for_kiss/
- S16: two mat guides from cutter makers (which grip for which material). https://help.singer.com/en-US/which-cutting-mat-should-i-use-405990 and https://help.cricut.com/hc/en-us/articles/360009554153
