# Quality benchmarks

"Measure twice, cut once." FlatCraft measures every cut before the blade
touches the material: the knife simulator follows the blade tip along the
planned path and checks how far it strays. This page holds those numbers for
a fixed set of jobs, so a change that makes the cuts worse shows up before
it ships.

This page is written by `cargo xtask bench-quality --write`. Do not edit it
by hand.

## The jobs

Every job is cut by a Silhouette Cameo 2 (File Output) with a 0.3 mm blade
and host compensation (FlatCraft steers the blade around every corner), on
the 12 x 12 inch mat, and simulated on the plotter's own grid of 20 steps
per mm, so the numbers include the rounding the plotter really gets.

- **test-sheet**: the built-in test patterns (Test patterns in the app).
- **small-text**: block letters 4.8 mm high, drawn as filled shapes.
- **stickers-with-holes**: six stickers with windows; the donut has an
  island in its hole.
- **tiny-dots**: 96 dots with a radius of 0.1 to 0.6 mm. Most of them are
  smaller than the blade can turn around, so their numbers are a report, not
  a limit.
- **long-thin**: strips, a wave, a frame and a needle, up to 250 mm long.
- **round-holes**: a plate with 27 round holes of 1 to 10 mm, and three
  washers.
- **dense-trace**: a picture of 400 small shapes, some with holes, traced
  into cut lines with the default trace settings.

The files are in `tests/fixtures/bench`. They were drawn for FlatCraft.

## The variants

- **quality**, **fast**, **outlines_last**: the three cut strategies, as you
  pick them in the job settings.
- **baseline**: the cut order without the newer tricks (nearest neighbour
  plus 2-opt only). It shows what the better order buys.
- **passes_chained** and **passes_separate**: two passes, once as one
  ring that goes round twice without lifting the blade (the default), once
  as two separate laps with a lift between them.

## What the columns mean

- **score** and **grade**: the quality report's summary (100 is perfect).
- **deviation**: how far the blade tip strays from the planned line at its
  worst, mm.
- **start**: the worst stray where a cut begins, mm.
- **closure gap**: the worst gap where a ring should meet its start, mm.
- **into kept**: how deep a waste move cuts into material you keep, mm.
- **uncut**: line length the blade never cuts through, mm.
- **pushed**: how often the blade is pushed from behind.
- **travel mm** and **crossings**: blade-up moves, and how often they cross
  a line that is cut already.
- **segments** and **lifts**: straight moves with the blade down, and
  how often it goes up.
- **time s**: the time estimate in seconds.

## The rule

Every strategy row must stay as good as its row in
`tests/fixtures/bench/baseline.json`:

- the score drops by at most 1,
- each mm number grows by at most 5 % plus 0.002 mm,
- travel grows by at most 5 %,
- segments grow by at most 10 %,
- pushes and the cut into kept material never grow.

The strategy rows (except tiny-dots) also stay within the device limit of
0.075 mm. The baseline and passes rows are for comparison only.
A change that makes the numbers better writes the new baseline in the same
commit, with the difference in the commit message.

The test `cargo test -p flatcraft-engine --test benchmarks` checks all of
this in CI. `cargo xtask bench-quality` prints the tables with the time each
run took on your machine.

## Results

### test-sheet

| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | segments | lifts | time s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quality | 100.0 | good | 0.0359 | 0.0278 | 0.0176 | 0.0000 | 0.0000 | 0 | 1115.3 | 2 | 1455 | 44 | 59.2 |
| fast | 100.0 | good | 0.0359 | 0.0278 | 0.0206 | 0.0000 | 0.0000 | 0 | 1074.0 | 5 | 1521 | 44 | 59.3 |
| outlines_last | 100.0 | good | 0.0359 | 0.0278 | 0.0127 | 0.0000 | 0.0000 | 0 | 1181.3 | 1 | 1463 | 44 | 61.0 |
| baseline | 100.0 | good | 0.0359 | 0.0278 | 0.0164 | 0.0000 | 0.0000 | 0 | 1113.7 | 12 | 1465 | 44 | 60.6 |
| passes_chained | 100.0 | good | 0.0359 | 0.0278 | 0.0176 | 0.0000 | 0.0000 | 0 | 1541.5 | 4 | 2704 | 70 | 104.8 |
| passes_separate | 100.0 | good | 0.0359 | 0.0278 | 0.0179 | 0.0000 | 0.0000 | 0 | 1553.8 | 5 | 2739 | 88 | 108.0 |

### small-text

| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | segments | lifts | time s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quality | 100.0 | good | 0.0350 | 0.0350 | 0.0190 | 0.0000 | 0.0000 | 0 | 199.6 | 16 | 3803 | 36 | 23.7 |
| fast | 100.0 | good | 0.0456 | 0.0456 | 0.0148 | 0.0000 | 0.0000 | 0 | 202.3 | 13 | 4005 | 36 | 24.0 |
| outlines_last | 100.0 | good | 0.0351 | 0.0351 | 0.0187 | 0.0000 | 0.0000 | 0 | 319.8 | 18 | 3870 | 36 | 26.1 |
| baseline | 100.0 | good | 0.0342 | 0.0342 | 0.0174 | 0.0000 | 0.0000 | 0 | 216.8 | 24 | 3992 | 36 | 24.1 |
| passes_chained | 100.0 | good | 0.0350 | 0.0350 | 0.0190 | 0.0000 | 0.0000 | 0 | 199.6 | 16 | 7477 | 36 | 38.0 |
| passes_separate | 100.0 | good | 0.0350 | 0.0350 | 0.0190 | 0.0000 | 0.0000 | 0 | 218.7 | 16 | 7522 | 72 | 44.1 |

### stickers-with-holes

| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | segments | lifts | time s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quality | 100.0 | good | 0.0402 | 0.0361 | 0.0167 | 0.0000 | 0.0000 | 0 | 299.6 | 0 | 2014 | 15 | 27.2 |
| fast | 100.0 | good | 0.0402 | 0.0325 | 0.0218 | 0.0000 | 0.0000 | 0 | 298.0 | 2 | 2045 | 15 | 27.2 |
| outlines_last | 100.0 | good | 0.0402 | 0.0340 | 0.0220 | 0.0000 | 0.0000 | 0 | 421.8 | 6 | 2006 | 15 | 30.0 |
| baseline | 100.0 | good | 0.0402 | 0.0300 | 0.0186 | 0.0000 | 0.0000 | 0 | 350.2 | 6 | 1986 | 15 | 28.4 |
| passes_chained | 100.0 | good | 0.0402 | 0.0361 | 0.0167 | 0.0000 | 0.0000 | 0 | 299.6 | 0 | 3774 | 15 | 46.9 |
| passes_separate | 100.0 | good | 0.0402 | 0.0361 | 0.0224 | 0.0000 | 0.0000 | 0 | 310.8 | 0 | 3926 | 30 | 49.6 |

### tiny-dots

| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | segments | lifts | time s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quality | 100.0 | good | 0.0426 | 0.0426 | 0.2248 | 0.0000 | 0.0000 | 0 | 328.0 | 10 | 6465 | 86 | 28.1 |
| fast | 100.0 | good | 0.0346 | 0.0346 | 0.2086 | 0.0000 | 0.0000 | 0 | 201.2 | 1 | 6738 | 86 | 27.6 |
| outlines_last | 100.0 | good | 0.0426 | 0.0426 | 0.2248 | 0.0000 | 0.0000 | 0 | 328.0 | 10 | 6465 | 86 | 28.1 |
| baseline | 100.0 | good | 0.0347 | 0.0347 | 0.2228 | 0.0000 | 0.0000 | 0 | 281.3 | 25 | 6631 | 86 | 28.8 |
| passes_chained | 100.0 | good | 0.0426 | 0.0426 | 0.2247 | 0.0000 | 0.0000 | 0 | 328.0 | 10 | 11701 | 86 | 35.2 |
| passes_separate | 100.0 | good | 0.0425 | 0.0425 | 0.2248 | 0.0000 | 0.0000 | 0 | 478.3 | 66 | 13064 | 172 | 54.7 |

### long-thin

| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | segments | lifts | time s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quality | 100.0 | good | 0.0310 | 0.0204 | 0.0053 | 0.0000 | 0.0000 | 0 | 88.1 | 1 | 1197 | 9 | 98.0 |
| fast | 100.0 | good | 0.0310 | 0.0214 | 0.0048 | 0.0000 | 0.0000 | 0 | 85.9 | 6 | 1197 | 9 | 98.0 |
| outlines_last | 100.0 | good | 0.0310 | 0.0204 | 0.0027 | 0.0000 | 0.0000 | 0 | 133.1 | 6 | 1206 | 9 | 99.0 |
| baseline | 100.0 | good | 0.0310 | 0.0214 | 0.0048 | 0.0000 | 0.0000 | 0 | 85.9 | 6 | 1197 | 9 | 98.0 |
| passes_chained | 100.0 | good | 0.0310 | 0.0204 | 0.0053 | 0.0000 | 0.0000 | 0 | 88.1 | 1 | 2376 | 9 | 192.9 |
| passes_separate | 100.0 | good | 0.0310 | 0.0204 | 0.0053 | 0.0000 | 0.0000 | 0 | 92.6 | 1 | 2385 | 18 | 194.4 |

### round-holes

| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | segments | lifts | time s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quality | 100.0 | good | 0.0419 | 0.0419 | 0.0136 | 0.0000 | 0.0000 | 0 | 371.1 | 1 | 3919 | 34 | 33.0 |
| fast | 100.0 | good | 0.0328 | 0.0328 | 0.0047 | 0.0000 | 0.0000 | 0 | 368.6 | 3 | 4022 | 34 | 33.0 |
| outlines_last | 100.0 | good | 0.0392 | 0.0392 | 0.0070 | 0.0000 | 0.0000 | 0 | 435.8 | 6 | 3910 | 34 | 34.3 |
| baseline | 100.0 | good | 0.0326 | 0.0325 | 0.0081 | 0.0000 | 0.0000 | 0 | 416.7 | 26 | 3968 | 34 | 33.8 |
| passes_chained | 100.0 | good | 0.0419 | 0.0419 | 0.0136 | 0.0000 | 0.0000 | 0 | 371.1 | 1 | 7187 | 34 | 52.9 |
| passes_separate | 100.0 | good | 0.0419 | 0.0419 | 0.0151 | 0.0000 | 0.0000 | 0 | 400.0 | 1 | 7778 | 68 | 59.5 |

### dense-trace

| variant | score | grade | deviation | start | closure gap | into kept | uncut | pushed | travel mm | crossings | segments | lifts | time s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| quality | 100.0 | good | 0.0448 | 0.0448 | 0.0375 | 0.0000 | 0.0000 | 0 | 1987.4 | 133 | 33623 | 507 | 236.8 |
| fast | 100.0 | good | 0.0442 | 0.0399 | 0.0337 | 0.0000 | 0.0000 | 0 | 1431.7 | 129 | 36298 | 507 | 232.3 |
| outlines_last | 100.0 | good | 0.0448 | 0.0448 | 0.0211 | 0.0000 | 0.0000 | 0 | 2538.4 | 183 | 33656 | 507 | 246.8 |
| baseline | 100.0 | good | 0.0525 | 0.0525 | 0.0347 | 0.0000 | 0.0000 | 0 | 1628.0 | 230 | 35571 | 507 | 233.6 |
| passes_chained | 100.0 | good | 0.0448 | 0.0448 | 0.0375 | 0.0000 | 0.0000 | 0 | 1987.4 | 133 | 65375 | 507 | 354.2 |
| passes_separate | 100.0 | good | 0.0539 | 0.0539 | 0.0393 | 0.0000 | 0.0000 | 0 | 2339.8 | 135 | 66526 | 1014 | 443.6 |

## Travel crossings of the order improvement

A blade-up move that crosses a finished cut can snag a loose piece. The
order improvement (2-opt, Or-opt and the entry search) avoids crossings
but does not count them exactly. This sweep plans 118 small jobs (40 or 150
squares of 0.5 to 4.5 mm in 60 x 60 mm, seeds 1 to 59) with the improvement
and with the first nearest-neighbour order alone, and counts the crossings
in the simulation. The rule: the number of jobs the improvement makes
worse never grows, and the number it makes better never drops.

| jobs | fewer crossings | more | same | crossings, construction alone | crossings, improved |
|---|---|---|---|---|---|
| 118 | 87 | 18 | 13 | 4089 | 3713 |
