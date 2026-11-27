# Knife compensation: the numbers behind the knobs

A drag knife is a tiny caster wheel with a blade on it. The tip trails the
holder axis by the blade offset `d` (about 0.25 to 0.3 mm), so a plotter that
drives the axis along your drawing cuts a slightly different shape: corners
get rounded, starts get a hook, and a closed shape may not quite close. Host
compensation moves the axis so that the trailing tip lands on the line.

This page explains the model FlatCraft uses, lists every measurement that
picked a constant, and says why each value won. Measured, not guessed: "In
God we trust, all others must bring data." Every number below comes from
`crates/flatcraft-sim/tests/tuning.rs`; run it with
`cargo test -p flatcraft-sim --test tuning -- --include-ignored --nocapture`
to print the tables again.

## The model

- The tip trails the axis at the distance `d` and only ever moves along the
  line from the tip to the axis (a pursuit model). Along a straight axis move
  the tip follows a tractrix, which has a closed form, so the simulator is
  exact for the model at any step size.
- Host compensation is the exact inverse: for every segment the axis goes to
  the segment end plus `d` along the segment, and at every corner the axis
  runs an arc of radius `d` round the corner point, so the blade turns on the
  spot while the tip stays on the corner (research item R2).
- Every blade-down start is aligned (R1): the blade turns on the spot, or
  comes in along a short straight lead-in on the waste side, or along a
  quarter-circle lead-in inside a hole. Every waste move is checked against
  the material you keep.
- A move that drives the axis back at the tip pushes the blade from behind
  (push angle above 150 deg). A real blade flips or digs in there, so it
  must not happen.
- The simulator runs the job twice (R3): in float mode with exact
  coordinates, and in device mode where every point is rounded to the
  plotter's units (20 per mm on the Cameo 2), as the plotter really gets it.

## Quality limits (R7)

| Limit | Value | Applies to |
|---|---|---|
| `ALGORITHM_LIMIT_MM` | 0.005 mm | float simulation, assumed d = real d, against the planned path |
| `DEVICE_LIMIT_MM` | 0.075 mm | the rounded device output, against the planned path |
| `END_TO_END_LIMIT_MM` | 0.06 mm | the cut against the source shape, at every grid position |

The end-to-end limit is the guarantee that counts: it is what you see on
the mat. It measures the simulated cut against your original shape, after
flattening, the knife stage and the rounding to device units, both ways:
no cut may stray from the shape (an over-cut), and no part of the shape may
be missed by the cut (an under-cut, such as a corner the tip never
reaches). It must hold wherever the mat's grid lies under the plotter's
units.

Why these values:

- Rounding a point to whole 0.05 mm units moves it up to
  `0.025 * (|sin a| + |cos a|)` off a line at angle `a`: 0.0354 mm at
  45 deg. No planner can beat that floor. The spec's first target of
  0.03 mm for the device output was below it.
- On a corner the blade turns on a pivot arc of radius `d`, only five or six
  device units across. Rounding its points and the trailing tip together
  measured up to 0.0569 mm against the planned path. So the device limit
  is 1.5 units (0.075 mm), a bound on each point, not the promise.
- End to end, the limit is one device unit plus the flatten tolerance
  (0.05 + 0.01 = 0.06 mm). The spec's first target of 0.05 mm cannot be
  held with whole units: the measured worst over 32 grid positions is
  0.0521 mm (table below), and 0.0569 mm on an odd calibrated grid.
- Waste moves (lead-ins, overshoots, the alignment stroke) must never cut
  into the material you keep. In device mode the simulator lets a waste tip
  come only one device unit (0.05 mm) near a cut before it counts as cutting
  in: that is what rounding alone can do. This tolerance has its own
  constant (`DEVICE_WASTE_TOLERANCE_MM`) and does not grow with the device
  limit. Every sweep and every grid position reports 0 mm into kept
  material.
- Whether the Cameo 2 takes decimals or half units in `D` is an open
  capture question (`docs/protocol/gpgl.md`, open question 14). If it does,
  the limits can come down again:

| units per mm | d | worst device deviation on the sheet and probes | worst case | pushed |
|---|---|---|---|---|
| 20 | 0.25 | 0.0406 | circle-r25 | 0 |
| 20 | 0.3 | 0.0417 | circle-r100 | 0 |
| 40 | 0.25 | 0.0195 | star-90 | 0 |
| 40 | 0.3 | 0.0198 | circle-r100 | 0 |
| 100 | 0.25 | 0.0079 | star-120 | 0 |
| 100 | 0.3 | 0.0083 | star-170 | 0 |
| 200 | 0.25 | 0.0047 | turns | 0 |
| 200 | 0.3 | 0.0054 | star-150 | 0 |

## The shapes

The test sheet's shapes, each at its sheet position on an uncalibrated
12x12 mat (the grid, inset by 5 mm): circles of radius 0.5 to 25 mm, star
tips of 15 to 170 deg, the 50 mm square, the square with a hole, a 10 mm
round hole in a 40 mm piece, and the S-curve. Some probes join them where
the sheet cannot tell candidates apart:

- `needle-2`: a closed tip of 2 deg, so its point turns 178 deg.
- `hairpin-180`: an open line that goes out and straight back.
- `turns`: open lines with one turn of just under 0.5, 1, 2, 5 or 10 deg, then
  150 mm straight on. That is the largest turn each threshold skips, with a
  long segment after it, the worst case for a skipped turn.
- `circle-r100` and `arc-r400`: gentle curves that turn about 1.6 and 0.8 deg
  per flattened vertex.

Every shape is planned with d = 0.25 and d = 0.3 mm and simulated in float
and device mode.

## Rounding-aware device runs

Plain rounding of the exact axis runs pushed the blade: a pivot arc of
radius 0.25 mm is only five device units across, and a rounded arc point can
step back toward the tip. The 50 mm square at d = 0.25 got 2 pushes, the
whole sheet 26. Now, when a plan is made for a known plotter, the knife
stage rounds the runs itself (`flatcraft_path::quantize::round_runs`),
following the tip with the same model:

- Each point goes to its nearest device unit. Where that move would push
  the blade, it goes to the best of the other three unit points round it
  instead (up to 1.4 units, 0.071 mm, from where it was planned; the device
  deviation of the result is measured, and stays within 0.075 mm), or it is
  dropped.
- A dropped point never lies more than 0.05 mm (one unit on the Cameo 2; a
  bound in millimetres, not in units) from the rounded run that replaces
  it. Where a push cannot be avoided within that bound,
  the point stays and the push is counted; the simulator reports it.
- The end of a run always stays: when the step onto it would push, the
  points before it go instead.

The plotter's units, grid origin, mirror and calibration scale come from
one place (`flatcraft_core::device_grid::DeviceGrid`). The emitter, the
knife stage and the simulator all use it, so the points the knife stage
picks reach the plotter unchanged. The float plan does not change at all.

| case | d | device dev, plain rounding | pushed, plain | device dev, rounding-aware | pushed, aware |
|---|---|---|---|---|---|
| square-50 | 0.25 | 0.0245 | 2 | 0.0245 | 0 |
| hole-in-square | 0.25 | 0.0245 | 4 | 0.0245 | 0 |
| circle-r25 | 0.3 | 0.0332 | 4 | 0.0356 | 0 |
| circle-r25 | 0.25 | 0.0406 | 4 | 0.0406 | 0 |
| star-30 | 0.3 | 0.0425 | 2 | 0.0392 | 0 |
| star-60 | 0.25 | 0.0449 | 1 | 0.0398 | 0 |
| circle-r100 | 0.3 | 0.0417 | 2 | 0.0417 | 0 |
| circle-r100 | 0.25 | 0.0352 | 3 | 0.0404 | 0 |
| whole sheet | 0.25 | 0.0449 | 26 | 0.0406 | 0 |
| whole sheet | 0.3 | 0.0425 | 11 | 0.0392 | 0 |

Every sheet shape and probe stays within 0.0024 mm in float mode, with no
pushed blade.

## Every grid position

Where the mat's grid lies under the plotter's units changes how each point
rounds. The sweep moves the grid origin in quarter units (16 positions),
on a plain plotter and on a calibrated one (scale 1.002 across and 0.998
along, mirrored across the carriage). Each position is planned for its own
grid, the sheet and the probes at d = 0.25 and 0.3 mm. "Device" is the
rounded cut against the planned path, "end to end" against your original
shapes, both ways (the larger of the cut's distance from the shape and the
shape's distance from the cut, the shape sampled every 0.01 mm).

| scale | mirror x | grid origin | device max dev | worst case | end to end max | worst case | pushed | into kept |
|---|---|---|---|---|---|---|---|---|
| 1, 1 | false | 0, 0 | 0.0417 | circle-r100 0.3 | 0.0396 | circle-r100 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0, 0.0125 | 0.0437 | circle-r100 0.25 | 0.0410 | s-curve 0.25 | 0 | 0.0000 |
| 1, 1 | false | 0, 0.025 | 0.0494 | star-30 0.3 | 0.0494 | star-30 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0, 0.0375 | 0.0428 | circle-r100 0.3 | 0.0493 | s-curve 0.25 | 0 | 0.0000 |
| 1, 1 | false | 0.0125, 0 | 0.0490 | star-90 0.25 | 0.0464 | star-90 0.25 | 0 | 0.0000 |
| 1, 1 | false | 0.0125, 0.0125 | 0.0460 | star-45 0.25 | 0.0482 | circle-r100 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0.0125, 0.025 | 0.0399 | star-60 0.25 | 0.0448 | circle-r25 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0.0125, 0.0375 | 0.0428 | circle-r100 0.25 | 0.0453 | circle-r100 0.25 | 0 | 0.0000 |
| 1, 1 | false | 0.025, 0 | 0.0482 | star-60 0.3 | 0.0485 | circle-r100 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0.025, 0.0125 | 0.0432 | circle-r100 0.25 | 0.0461 | circle-r100 0.25 | 0 | 0.0000 |
| 1, 1 | false | 0.025, 0.025 | 0.0500 | turns 0.3 | 0.0500 | turns 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0.025, 0.0375 | 0.0508 | star-45 0.25 | 0.0469 | circle-r25 0.25 | 0 | 0.0000 |
| 1, 1 | false | 0.0375, 0 | 0.0438 | star-120 0.25 | 0.0475 | circle-r100 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0.0375, 0.0125 | 0.0431 | star-45 0.3 | 0.0453 | circle-r100 0.25 | 0 | 0.0000 |
| 1, 1 | false | 0.0375, 0.025 | 0.0424 | circle-r100 0.3 | 0.0499 | circle-r100 0.3 | 0 | 0.0000 |
| 1, 1 | false | 0.0375, 0.0375 | 0.0453 | turns 0.25 | 0.0471 | circle-r100 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0, 0 | 0.0434 | star-170 0.25 | 0.0434 | star-170 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0, 0.0125 | 0.0445 | star-45 0.25 | 0.0472 | circle-r100 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0, 0.025 | 0.0516 | star-45 0.25 | 0.0484 | circle-r100 0.3 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0, 0.0375 | 0.0426 | circle-r100 0.25 | 0.0490 | circle-r100 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0125, 0 | 0.0429 | circle-r100 0.25 | 0.0465 | circle-r100 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0125, 0.0125 | 0.0521 | star-170 0.3 | 0.0521 | star-170 0.3 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0125, 0.025 | 0.0480 | star-120 0.3 | 0.0480 | star-120 0.3 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0125, 0.0375 | 0.0441 | circle-r100 0.3 | 0.0459 | circle-r100 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.025, 0 | 0.0416 | star-15 0.3 | 0.0470 | circle-r100 0.3 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.025, 0.0125 | 0.0441 | star-15 0.25 | 0.0478 | circle-r100 0.3 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.025, 0.025 | 0.0493 | star-120 0.3 | 0.0480 | star-120 0.3 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.025, 0.0375 | 0.0452 | star-150 0.3 | 0.0504 | circle-r100 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0375, 0 | 0.0412 | circle-r100 0.25 | 0.0454 | circle-r100 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0375, 0.0125 | 0.0431 | circle-r100 0.3 | 0.0380 | star-150 0.3 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0375, 0.025 | 0.0497 | star-150 0.25 | 0.0497 | star-150 0.25 | 0 | 0.0000 |
| 1.002, 0.998 | true | 0.0375, 0.0375 | 0.0468 | star-150 0.25 | 0.0468 | star-150 0.25 | 0 | 0.0000 |

Nothing is pushed at any position, every position stays within the
end-to-end limit of 0.06 mm (worst 0.0521 mm; the shape-to-cut direction
alone is at most 0.0504 mm), and the device output stays within its
0.075 mm bound. An odd calibrated grid at origin (0.37, 0.013) is the worst
seen so far: 0.0569 mm on star-170 at d = 0.3, device and end to end, still
within 0.06 mm. The CI test checks it on every run, with
origin 0 and the two worst positions of the table. A plain grid at origin 0 happens to be one of the
kinder positions (0.0417 mm), which is why a single position is not enough
to judge.

## The chosen values

### `ARC_STEP_DEG` = 10

The angle between two points of a pivot arc. Rule: the largest of 30, 20,
15, 10 and 5 deg whose corner deviation (the largest error within `2 d` of a
corner) and pivot drift (how far the tip leaves the corner while the blade
turns) stay within 0.005 mm on every star tip and on 178 and 180 deg turns.

Cells: corner deviation / pivot drift (axis segments).

| case | d | 30 | 20 | 15 | 10 | 5 |
|---|---|---|---|---|---|---|
| star-15 | 0.3 | 0.0178 / 0.0178 (20) | 0.0080 / 0.0080 (25) | 0.0054 / 0.0054 (31) | 0.0024 / 0.0023 (43) | 0.0024 / 0.0006 (80) |
| star-150 | 0.3 | 0.0178 / 0.0178 (18) | 0.0080 / 0.0080 (24) | 0.0054 / 0.0054 (29) | 0.0023 / 0.0023 (42) | 0.0006 / 0.0006 (77) |
| star-170 | 0.3 | 0.0211 / 0.0211 (17) | 0.0095 / 0.0095 (23) | 0.0053 / 0.0053 (29) | 0.0024 / 0.0024 (41) | 0.0006 / 0.0006 (78) |
| needle-2 | 0.3 | 0.0222 / 0.0222 (18) | 0.0099 / 0.0099 (23) | 0.0056 / 0.0056 (30) | 0.0025 / 0.0025 (42) | 0.0006 / 0.0006 (78) |
| hairpin-180 | 0.25 | 0.0191 / 0.0191 (8) | 0.0086 / 0.0086 (11) | 0.0048 / 0.0048 (14) | 0.0022 / 0.0022 (20) | 0.0005 / 0.0005 (38) |
| hairpin-180 | 0.3 | 0.0229 / 0.0229 (8) | 0.0103 / 0.0103 (11) | 0.0058 / 0.0058 (14) | 0.0026 / 0.0026 (20) | 0.0007 / 0.0007 (38) |
| square-50 | 0.3 | 0.0101 / 0.0137 (20) | 0.0037 / 0.0051 (30) | 0.0026 / 0.0036 (35) | 0.0011 / 0.0016 (50) | 0.0003 / 0.0004 (95) |
| worst of all | | 0.0229 | 0.0103 | 0.0058 | 0.0026 | 0.0024 |

15 deg misses on the sharpest turns, 10 deg holds with room to spare.

### Pivot threshold: Quality 0 deg, Fast 0.5 deg

A turn at or below the threshold gets no pivot arc, which saves axis
segments but lets the tip cut a little inside the corner (about
`d * sin(turn)`). Candidates: 0, 0.5, 1, 2, 5 and 10 deg.

| threshold | skipped pivots | segments | float max dev | device max dev |
|---|---|---|---|---|
| 0 | 0 | 4804 | 0.0024 | 0.0417 |
| 0.5 | 4 | 4800 | 0.0026 | 0.0417 |
| 1 | 30 | 4774 | 0.0052 | 0.0417 |
| 2 | 542 | 4262 | 0.0103 | 0.0406 |
| 5 | 1116 | 3688 | 0.0258 | 0.0456 |
| 10 | 1384 | 3420 | 0.0514 | 0.0723 |

- Quality and Outlines last: 0 deg, an arc at every corner. 0.5 deg would
  stay within the float limit, but it saves only 4 of 4804 segments. That
  is not worth leaving the simpler rule, so a threshold above 0 must save at
  least 1 % of the segments to become the default. 1 deg already misses the
  float limit (0.0052 mm).
- Fast: 0.5 deg, the largest candidate whose rounded output stays within
  the device limit and whose float output stays within the algorithm
  limit. In device mode every candidate up to 5 deg is within the device
  limit, because the rounding is bigger than the skipped turns; the float
  limit is what decides. 0.5 deg saves only a little, so Fast is fast
  mostly through its cut order.

### `HOLE_LEAD_IN_RADIUS_MM` = 0.5

The radius of the quarter-circle lead-in inside a hole. Rule: the smallest of
0.5, 1, 2 and 3 mm whose starts stay within 0.005 mm, push nothing, touch no
kept material and take the arc lead-in on every start (round holes of radius
1, 2, 5 and 10 mm, 12 headings, both ways round).

| radius | holes where every start gets the arc | start dev (max) | pushed | into kept | waste per start |
|---|---|---|---|---|---|
| 0.5 | 1, 2, 5, 10 | 0.0010 | 0 | 0 | 0.82 |
| 1 | 2, 5, 10 (not 1) | 0.0008 | 0 | 0 | 1.61 |
| 2 | 5, 10 (not 1, 2) | 0.0008 | 0 | 0 | 3.18 |
| 3 | 5, 10 (not 1, 2) | 0.0008 | 0 | 0 | 4.75 |

Every radius meets the deviation, push and kept-material limits, but only
0.5 mm takes the arc on every hole: 1 mm and larger do not fit the 1 and
2 mm holes. It also wastes the least. A hole too small even for it gets a swivel on
the spot instead (a larger radius used to halve down to 0.5 mm; with 0.5 as
the radius that fallback had nothing left to do, so it is gone).

### `SAMPLE_STEP_FACTOR` = 20

The simulator samples every `d / factor` mm of axis travel. The model is
exact per step, so the step only decides how finely the trace is read.
Rule: the smallest of 10, 20, 40 and 80 whose float metrics (Hausdorff,
correspondence, corner, start and closure gap) stay within 0.0005 mm of the
values at 80 on the jobs as planned, and within 0.005 mm when the first
start is entered with the blade 90 deg off the assumed heading. The quality
report shows such misaligned first starts too, and the tip swings fastest
there, so they are the values most sensitive to the step. Their errors are
0.1 to 0.3 mm, so 0.005 mm is below 5 %.

| factor | real step at d 0.25 / 0.3 | largest change, as planned | largest change, first start 90 deg off |
|---|---|---|---|
| 10 | 0.025 / 0.03 | 0.0002 | 0.0123 |
| 20 | 0.0125 / 0.015 | 0.0001 | 0.00495 |
| 40 | 0.0063 / 0.0075 | 0.00002 | 0.0012 |
| 80 | 0.005 / 0.005 | reference | reference |

10 is plenty for planned jobs but reads a misaligned first start up to
0.012 mm low (the 50 mm square). 20 just makes it (0.00495 against 0.005,
a thin margin: a small change to the planner or the metrics can tip the
rule to 40), at twice the cost of 10.
Device mode is left out of the rule: it has its own rounding floor, and its
rounded moves make the trace jump between steps. The smallest step is
clamped to 0.005 mm (`MIN_SAMPLE_STEP_MM`), so "80" is really 0.005 mm at
both offsets; the clamp only touches factors above 50 to 60, never the
chosen one.

## Starts and the blade heading (R1)

The alignment check turns the blade 0 to 179 deg (both ways) before the
first segment of a piece (entered at a corner and mid-edge), a stencil, a
"both" outline and a round hole. Every start stays within 0.0024 mm, pushes
nothing and touches no kept material.

| layout | starts |
|---|---|
| piece, corner | swivel up to 90 deg, straight lead-in above (1.4 to 2.1 mm) |
| piece, mid-edge | arc lead-in (its backward extension is the cut itself) |
| stencil, corner | swivel |
| stencil, mid-edge | arc lead-in |
| both, mid-edge | swivel (no waste side) |
| round hole | arc lead-in |

How far the blade may swivel on the spot (`SWIVEL_MAX_DEG`) is a matter of
how the material takes a turning blade, which the model cannot rate. 90 deg
stays provisional until the M3 test cuts.

| swivel max | d | swivels | lead-ins | waste (sum, mm) | twist (max deg) |
|---|---|---|---|---|---|
| 45 | 0.3 | 6 | 18 | 24.30 | 134.7 |
| 60 | 0.3 | 8 | 16 | 22.58 | 149.7 |
| 90 | 0.3 | 12 | 12 | 17.49 | 179.6 |
| 120 | 0.3 | 16 | 8 | 12.13 | 209.5 |

## The first path and the unknown start heading

Before a job the blade heading is unknown; the planner assumes `+x`. The
whole sheet was simulated with the blade really pointing at 8 other
headings. Only the first path suffers; every later path stays within
0.0024 mm. The first path's error grows up to `d`:

| real heading | 22.5 | 67.5 | 112.5 | 157.5 | 202.5 | 247.5 | 292.5 | 337.5 |
|---|---|---|---|---|---|---|---|---|
| first path, d 0.25 | 0.070 | 0.141 | 0.180 | 0.208 | 0.234 | 0.250 | 0.250 | 0.129 |
| first path, d 0.3 | 0.084 | 0.169 | 0.216 | 0.250 | 0.281 | 0.300 | 0.300 | 0.155 |

With an alignment stroke (a short cut in waste before the job) every cut
stays within 0.0024 mm. The stroke itself is pushed once at 157.5 and
202.5 deg, which is its job, and it lies entirely in waste.

## A wrong offset (R4)

Planned for `d`, cut by a blade of 0.9 `d` or 1.1 `d`:

| case | planned d | real d | max dev | corner dev | into kept |
|---|---|---|---|---|---|
| square-50 | 0.3 | 0.27 | 0.0549 | 0.0549 | 0.0335 |
| square-50 | 0.3 | 0.33 | 0.0486 | 0.0486 | 0 |
| star-15 | 0.3 | 0.27 | 0.0794 | 0.0794 | 0 |
| star-15 | 0.3 | 0.33 | 0.0992 | 0.0992 | 0 |
| s-curve | 0.3 | 0.27 | 0.0486 | 0 | 0 |
| s-curve | 0.3 | 0.33 | 0.0396 | 0 | 0 |

A 10 % wrong offset costs ten times the algorithm limit, and a blade with a
smaller real offset cuts into the kept piece at corners. That is why the
offset calibration series exists: know your blade. (This table is the
single shapes of the Task 10 sweep in float; the next section is the whole
test sheet on the device grid, where it is the larger blade that swings
into a kept piece.)

## If the blade offset is wrong

FlatCraft steers the blade with the offset in your plotter preset. If the
real blade trails by a different amount, every corner and every start is a
little off. The test sheet below was planned for 0.3 mm and cut (in the
simulator, on the Cameo 2 device grid) by blades of 0.27, 0.3 and 0.33 mm:
10 % too small, right, and 10 % too large. The numbers are the largest
distance of the cut from the drawn line, in mm, per pattern of the sheet
(`crates/flatcraft-engine/tests/quality_golden.rs`,
`offset_sensitivity_table`).

| pattern | blade 0.27 | blade 0.30 | blade 0.33 |
|---|---|---|---|
| square-50 | 0.0655 | 0.0214 | 0.0386 |
| hole-in-square | 0.0655 | 0.0214 | 0.0386 |
| f-glyph | 0.0655 | 0.0214 | 0.0386 |
| circles | 0.0640 | 0.0352 | 0.0482 |
| s-curve | 0.0631 | 0.0335 | 0.0409 |
| open-line | 0.0586 | 0.0085 | 0.0307 |
| ruler-x | 0.0655 | 0.0125 | 0.0385 |
| ruler-y | 0.0655 | 0.0104 | 0.0425 |
| star-15 | 0.0748 | 0.0275 | 0.1070 |
| star-30 | 0.0638 | 0.0300 | 0.1059 |
| star-45 | 0.0935 | 0.0339 | 0.0693 |
| star-60 | 0.0830 | 0.0282 | 0.0867 |
| star-90 | 0.0915 | 0.0359 | 0.0777 |
| star-120 | 0.0987 | 0.0310 | 0.0730 |
| star-150 | 0.0871 | 0.0341 | 0.0968 |
| star-170 | 0.0889 | 0.0318 | 0.1177 |

In plain words:

- With the right offset every pattern stays within 0.036 mm.
- 10 % off, squares, curves and lines are two to seven times as far off,
  and sharp star tips up to four times: up to 0.12 mm.
- 10 % off, the blade can also get pushed from behind once in a while
  (star-30 at 0.27, star-170 at 0.33), which can tear thin material.
- A blade that trails more than you told FlatCraft (0.33 here) can swing
  into the piece you keep at a start: 0.06 mm on this sheet.

So the offset is not a number to guess. Cut the offset calibration series
once per blade and set the winner in the plotter preset.

## Overcut and the closure gap

The overcut stays 0.5 mm. In firmware mode FlatCraft sends the drawing as it
is, and the simulator cannot model what the firmware's own compensation
(`FC18`) does. The closest it can show is an uncompensated ring:

| case | d | overcut | closure gap | gap with the blade aligned at the start | tractrix `d * sin(2 * atan(exp(-o / d)))` |
|---|---|---|---|---|---|
| square-50 | 0.3 | 0 | 0.4630 | 0.3000 | 0.3000 |
| square-50 | 0.3 | 0.5 | 0.2069 | 0.1094 | 0.1094 |
| square-50 | 0.3 | 1 | 0.0426 | 0.0214 | 0.0214 |
| star-90 | 0.3 | 0.5 | 0.3146 | 0.1094 | 0.1094 |
| square-50 | 0.25 | 0.5 | 0.1287 | 0.0665 | 0.0665 |

The measured gap is about twice the tractrix value, and the whole extra
comes from the start: the blade starts at the wrong heading and the first
millimetre is cut off the line. With the blade already along the first
segment the gap is exactly the tractrix value. So a longer overcut is not
the fix for this; a known start heading is. Captures and test cuts in M3
decide what `FC18` really does.

## The quality report and firmware mode

`job.simulate` grades a job with this simulator, and `job.compare_strategies`
runs it once per cut strategy. What it simulates follows the plotter preset:

- Host compensation: simulated on the device grid, as the plotter gets it.
- No compensation ("None", a pen): simulated with the preset's blade
  offset, so a knife set to "None" by mistake shows its real corners. A pen
  sets the blade offset to 0.
- Firmware compensation (`FC18`): not simulated. The report says so and
  gives only the commanded path, the travel, the segment count and the
  largest heading change per vertex on curves.

Task 51 saw a plain design without weed lines reach `max_into_kept_mm` of
about 0.3 mm when it was simulated without compensation. Measured on a
50 mm square with a 20 mm hole at d 0.3 (no compensation):

| spot | depth into kept material | where |
|---|---|---|
| the hole's drop point | 0.300 mm | 0.3 mm before the hole's start, along the travel |
| each outer corner | 0.143 mm | the corner the trailing tip rounds into the piece |

At d 0 (a pen) both are 0. So this is not an artefact of the simulator:
the blade comes down with its tip d behind the head along the travel
direction, and the travel to a hole crosses the kept ring, so the tip
lands in kept material and is dragged to the line from there. An
uncompensated knife really does that. What the firmware does at a drop
and at a corner is not known, so in firmware mode this is no longer
reported as a nick: firmware mode is not simulated until the M3 captures
and test cuts show what `FC18` does.

## Sources

- R1 start alignment, R2 arc at every corner, R3 simulation of the rounded
  output, R4 offset calibration, R5 corner handling by side, R6 passes and
  R7 metrics: the ranked items of FlatCraft's research report on cutting
  path algorithms and drag knife quality (M2 plan).
- The drag knife model and its tractrix are plain geometry, written from the
  research and the plan only.
