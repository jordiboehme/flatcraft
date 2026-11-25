# GPGL protocol spec (Silhouette Cameo 2)

This is FlatCraft's own description of the GPGL dialect spoken by the Silhouette Cameo 2 over USB.
It is the only source the GPGL emitter, the decoder and the USB transport may be written from
(clean-room rule). It is written in our own words from protocol facts. No code from other
projects was read or copied to write it.

Status: provisional. Nothing here has been checked against our own USB captures yet.

## 0. How to read this document

### Confidence marks

- `[M]`: several independent sources agree.
- `[1]`: one source only. Provisional until a capture confirms it.
- `[C]`: confirmed by our own USB capture or test cut. No fact has this mark yet.
- FlatCraft choice (provisional): not a device fact. FlatCraft decides this. It may change after
  captures.
- FlatCraft rule: not a device fact. A hard rule FlatCraft sets for itself; this document states
  each one where it applies. Captures do not change it.

Every fact below carries one of the marks above and a source key. A row with both a mark and
"FlatCraft choice (provisional)" means: the command itself is known (mark), the value or the
way FlatCraft uses it is our choice.

### Source keys

- R: FlatCraft protocol research note, 2026-11-24 (Crystalline
  `projects/flatcraft/flatcraft-cameo-2-usb-protocol-research`). It collects facts from the
  inkscape-silhouette docs (Commands.md, device list) and issues, robocut, py_silhouette, Sillinux
  and the Graphtec GP-GL manual.
- K: FlatCraft drag knife research note, 2026-11-24 (Crystalline
  `projects/flatcraft/flatcraft-drag-knife-research-findings`). Used only for the FC facts.
- U: FlatCraft USB capture methods note, 2026-11-24 (Crystalline
  `projects/flatcraft/flatcraft-usb-capture-methods-for-the-cameo-2`).
- G: Graphtec GP-GL command reference (vendor manual), the base language GPGL is derived from.
  https://github.com/Snow4DV/graphtec-gp-gl-manual. Not read directly for this version and not cited
  in any row yet. Facts from it reach this document only through R.
- #n: issue n in https://github.com/fablabnbg/inkscape-silhouette/issues

### Notation

- Bytes are written as hex-escaped ASCII: printable characters as is, other bytes as `\xNN`.
- `ETX` is the byte `0x03`. `ESC` is the byte `0x1b`.
- A backslash that is part of a command is written `\\` inside byte strings and `\` in prose.

## 1. Transport facts

| Fact | Value | Conf. | Source |
|---|---|---|---|
| USB vendor ID | `0x0b4d` (Graphtec America) | [M] | R |
| Cameo 2 product ID | `0x112b` | [1] | R, U (inkscape-silhouette device list only) |
| Related product IDs | Cameo `0x1121`, Cameo 3 `0x112f`, Portrait `0x1123`, Portrait 2 `0x1132` | [1] (some IDs have several sources, but not each one) | R |
| USB interface class | 7 (Printer) | [M] | R |
| Interface number | 0 | [M] | R |
| Bulk OUT endpoint | `0x01` | [M] (no Cameo 2 descriptor dump seen) | R, U |
| Bulk IN endpoint | `0x82` | [M] (no Cameo 2 descriptor dump seen) | R, U |
| wMaxPacketSize | Value unknown. FlatCraft reads it from the descriptors at runtime. | FlatCraft choice (provisional) | R |
| Command framing | Every text command ends with `ETX` (`0x03`). | [M] | R, U |
| ESC commands | `ESC 0x04` and `ESC 0x05` are two bytes. They have no `ETX`. | [M] | R, U |
| Reply framing | Replies are ASCII text ended by `ETX`. Buffer the input and split at `ETX`. | [M] | R, U |
| Flow control | The device NAKs bulk OUT while its buffer is full. A bulk write then blocks. Use long timeouts (about 30 s). | [M] | R |
| Write size | Plain blocking bulk writes are enough. Writing 1 KB and then polling for ready caused stalls (#216). 4096-byte chunks are used by one project, not proven needed. | [1] | R, U, #216 |
| Linux | `usblp` binds the device. Detach it, claim interface 0. A udev rule for VID `0b4d` gives access. | [M] | R |
| macOS | Interface 0 could be claimed without a detach in one report, so likely no kernel driver holds it. | [1] (inference, untested) | R |
| Windows | The device binds to `usbprint.sys`. A native USBPRINT path worked on a Cameo 4 without driver swaps. Untested on a Cameo 2. | [1] | R |

## 2. Units and axes

| Fact | Value | Conf. | Source |
|---|---|---|---|
| Unit size | 1 device unit = 0.05 mm, so 20 units per mm | [M] | R, K |
| Coordinate order | Every coordinate pair is sent as `y,x` | [M] | R |
| y axis | y is the media feed direction (down the mat) | [M] | R |
| x axis | x runs across the carriage | [M] | R |
| Coordinates | `M` and `D` take absolute coordinates | [M] | R |
| Decimals | Decimal coordinates have been seen in other senders | [1] | R |
| FlatCraft number format | Integers only, no sign for zero, no leading zeros, no spaces | FlatCraft choice (provisional) | - |
| FlatCraft rounding | Convert every absolute point from mm with `round(mm * 20.0)`. Never add up rounded deltas, so errors do not build up. | FlatCraft choice (provisional) | - |
| FlatCraft rounding ties | A tie (`.5`) rounds half away from zero, as Rust `f64::round` does. Example: 10.025 mm * 20.0 = 200.5, sent as `201`. | FlatCraft choice (provisional) | - |
| FlatCraft grid origin | Device coordinates = app coordinates + the grid origin of the mat (x and y in mm, measured from the mat's top-left corner in the unmirrored physical frame, before any mirror). This is the first step, before mirror, scale, units and rounding. App 0,0 is the 0,0 of the grid printed on the mat. | FlatCraft rule | - |
| FlatCraft coordinate range | After grid origin, mirror, scale and rounding, every coordinate must lie in 0..media height (y) and 0..media width (x) of the physical mat, in units, both ends included. A point outside is an error before anything is emitted, like an out-of-range speed, force or FC. Negative numbers are never emitted. | FlatCraft rule | - |
| FlatCraft origin | Physical media top-left = device `0,0`, because the job sets `\0,0` as the work area upper-left. The grid origin of the mat shifts app coordinates inside it (see above). | FlatCraft choice (provisional) | - |

Example: the mat point x = 10.5 mm, y = 10 mm is sent as `200,210`.

## 3. Command table

Dialects: **Cameo12** is the form for the Cameo 1 and Cameo 2. **Cameo3+** is the form for the
Cameo 3 and later. FlatCraft M1 emits Cameo12 only.

| Command | Cameo12 syntax | Cameo3+ syntax | Meaning | Valid range (Cameo 2) | Conf. | Source |
|---|---|---|---|---|---|---|
| Init | `ESC 0x04` | same | Initialize the device | - | [M] | R |
| Status query | `ESC 0x05` | same | Ask for the status. Reply see section 4. | - | [M] | R, U |
| Tool setup | not on Cameo 2 | `ESC 0x15` | Tool types | - | [1] | R |
| Firmware version | `FG` | same | Reply is a version string, for example `CAMEO V1.10` | - | [M] | R |
| Query upper-left limit | `[` | same | Reply is the upper-left limit | - | [M] | R |
| Query lower-right limit | `U` | same | Reply is the lower-right limit | - | [M] | R |
| Media preset | `FW<n>` | not used | Selects a media preset | 100-300 | [1] | R |
| Speed | `!<s>` | `!<s>,<tool>` | Cutting speed | 1-10 | [M] | R |
| Force | `FX<f>` | `FX<f>,<tool>` | Blade force | 1-33 | [M] | R |
| Blade offset | `FC<p>` | `FC<p>,<q>,<tool>` | Firmware blade offset compensation. `FC0` = no compensation (pen, or host does it). `FC18` is the common knife value. | unknown. FlatCraft limits FC to 0-100 until captures say more. | [M] | R, K |
| Lift | `FE0,0` | not recorded | Lift setting between paths and at corners | FlatCraft sends `FE0,0` only | [M] | R, K |
| Corner sharpening | `FF<s>,<e>,<tool>` | same | Sharpen corners, start and end values | FlatCraft sends `FF0,0,0` only | [M] | R, K |
| Track enhancing | `FY0` = on, `FY1` = off | same | Feed roller track enhancing. Said to need force 19 or more. Meaning disputed. | 0 or 1 | [1] | R |
| Orientation | `FN0` | same | Portrait orientation | FlatCraft sends `FN0` only | [M] | R |
| Orientation (TB50) | `TB50,0` / `TB50,1` | same | Portrait / landscape | 0 or 1 | [M] | R |
| Work area upper-left | `\<y>,<x>` | same | Sets the upper-left corner of the work area | units | [M] | R |
| Work area lower-right | `Z<y>,<x>` | same | Sets the lower-right corner of the work area | units | [M] | R |
| Line type | `L0` | same | Solid line | FlatCraft sends `L0` only | [M] | R |
| Tool select | not on Cameo 2 | `J<n>` | Selects tool holder | - | [M] | R |
| Auto blade depth | not on Cameo 2 (manual ratchet blade) | `TF<d>,1` | Sets blade depth | - | [M] | R |
| Mat type | not on Cameo 2 | `TG<n>` | 0 none, 1 12x12, 2 12x24, 8 15x15, 9 24x24 | - | [M] | R |
| Move | `M<y>,<x>` | same | Blade up, move to the absolute point | units | [M] | R |
| Draw | `D<y>,<x>[,<y>,<x>...]` | same | Blade down, cut through the absolute points in order | units | [M] | R |
| Binary draw | not on Cameo 2 | `BE1` / `BE2` / `BE3` | Binary encoded draw (Cameo 4, Portrait 3) | - | [M] | R |
| Curves | `BZ`, `Y`, `W`, `)` | same | GP-GL curve commands. Not known to work on Silhouette devices. FlatCraft flattens curves to `D` and never sends these. | - | [1] (inference, untested), FlatCraft choice (provisional) | R |
| Home | `H` | same | Move the head to the home position | - | [M] | R |
| Set origin after feed | `M<y>,0` then `SO0` | same | Feed past the cut and make that the new origin | - | [M] | R |
| Calibration read | `TB71`, `FA` | same | Read calibration values | - | [M] | R |
| Calibration write | `FB...`, `TB72...` | same | Writes calibration permanently. **Forbidden**, see section 6. | never send | [M] | R |
| Regmark setup | `TB99`, `TB52,2`, `TB51,<n>`, `TB53,<n>`, `TB55,1` | same | Registration mark type and geometry. TB51/TB53 meaning disputed. Not used in M1. | - | [M] | R, K |
| Regmark search | `TB123,<h>,<w>,<top>,<left>` (auto) or `TB23,<h>,<w>` (manual) | same | Device searches the marks. Reply see section 4. Not used in M1. | - | [M] | R, K |

Notes on FC:

- The firmware does the blade offset compensation when FC is non-zero. The host must then not
  also correct the geometry. When the host corrects, it sends `FC0`. [M] R, K
- One source calls the FC value a "blade diameter": 18 units = 0.9 mm. How this relates to the
  physical blade offset of about 0.3 mm is not settled. [1] K
- FlatCraft default is `FC18`. FlatCraft refuses FC values outside 0-100 (FlatCraft choice
  (provisional)).

## 4. Status replies

All replies end with `ETX`. Read until `ETX`, then strip it.

| Reply (before ETX) | Meaning | Ready? | Conf. | Source |
|---|---|---|---|---|
| `0` | Ready | yes | [M] | R, U |
| `1` | Moving (busy) | no | [M] | R, U |
| `2` | No media loaded | no | [M] | R, U |
| `3` | Paused | no | [M] | R |
| `4` | State after a cancel | no | [1] | R |
| `    0` (four spaces, then `0`) | Reply to a registration mark search: marks found. This is NOT the ready reply. Compare the exact string, do not trim spaces. | not a status | [M] | R |
| any other text | Unknown. Treat as not ready. | no | FlatCraft choice (provisional) | - |

- Only `0` (one character, no spaces) means ready. [M] R
- A registration mark search takes 20 to 40 s before the reply comes. [M] R

### Pre-send check (FlatCraft rule)

Used by the USB transport (M3). File Output has no device and does no check.

| Step / reply | Action | Conf. | Source |
|---|---|---|---|
| 1 | Send `ESC 0x05` (`\x1b\x05`). Read until `ETX`. | FlatCraft rule; command [M] | R, U |
| `0` exactly | Ready. Start the job. | FlatCraft rule; meaning [M] | R, U |
| `2` | No media. Send nothing. Tell the user to load media. | FlatCraft rule; meaning [M] | R, U |
| `1`, `3`, `4` or anything else | Not ready. Send nothing. Tell the user the device is not ready and to check that the head is parked on the left. | FlatCraft rule; meanings [M] / [1] | R |

There is no command to check the head position. The not-ready message covers it (section 7). [1] R

## 5. Typical job sequence

The job shape other senders use for a Cameo 2, in order: [M] R

| Step | Command | Purpose | Conf. | Source |
|---|---|---|---|---|
| 1 | `ESC 0x04` | Init | [M] | R |
| 2 | `FG` | Firmware query (reads a reply) | [M] | R |
| 3 | `FW<n>` | Media preset, optional | [1] | R |
| 4 | `!<s>` | Speed | [M] | R |
| 5 | `FX<f>` | Force | [M] | R |
| 6 | `FC18` | Blade offset | [M] | R |
| 7 | `FY1` | Track enhancing off | [M] as a step, [1] for the on/off meaning | R |
| 8 | `FN0` | Portrait | [M] | R |
| 9 | `TB50,0` | Portrait | [M] | R |
| 10 | `\0,0` | Work area upper-left | [M] | R |
| 11 | `Z<h>,<w>` | Work area lower-right | [M] | R |
| 12 | `L0` | Solid line | [M] | R |
| 13 | `FE0,0` | Lift setting | [M] | R |
| 14 | `FF0,0,0` | Corner setting | [M] | R |
| 15 | `M` / `D` lines | The cut | [M] | R |
| 16 | `H`, or `M<end>,0` then `SO0` | End the job | [M] | R |

The typical Cameo 2 job in the research includes both `FN0` and `TB50,0`. [M] R
Whether Silhouette Studio itself sends them is open question 5.

FlatCraft M1 uses this shape with three differences, all FlatCraft choice (provisional):
no `FG` inside the job bytes (a firmware query belongs to the connection, not to the job file),
no `FW`, and `H` at the end of every completely sent job (FlatCraft rule). Section 9 has the exact bytes.

## 6. Forbidden commands

| Command | Why | Conf. | Source |
|---|---|---|---|
| `FB...` | Changes the device calibration permanently. | [M] | R |
| `TB72...` | Writes calibration permanently, like `FB`. | [M] | R |
| `FB...` and `TB72...` in FlatCraft | Never built, never sent. The rules below say how. | FlatCraft rule | - |

Rules for FlatCraft (FlatCraft rule, not provisional):

- The emitter has no way to build `FB` or `TB72`.
- The safety check rejects any job that contains a command starting with `FB` or `TB72`.
- Scale errors are corrected in software (a scale factor in the plotter profile), never with `FB`.
  Scale errors of about 0.5 mm were reported on a Cameo 1 (#388). [1] R, #388

## 7. Quirks

| Quirk | What to do | Conf. | Source |
|---|---|---|---|
| One command of look-ahead | The device holds back the last command until the next one arrives (it needs it for the blade offset). The last `D` of a job is not cut until another command follows. Every job that was sent completely ends with `H` (FlatCraft rule). | [M] | R |
| Never resend | After a timeout or a partial write, part of the data may already be in the device. Sending it again cuts twice. | [M] | R |
| Failure stop | After a write timeout, a partial write or a user cancel, FlatCraft sends nothing more on that connection: no `H`, no retry, no status poll (`ESC 0x05`). This lasts until the user has reset the machine with its own button and reconnects (a new transport session). Reason: a half-sent command plus any new bytes can merge into a different command. | FlatCraft rule, based on "Never resend" above [M] | R |
| Head parked left | A Cameo 2 does not start a job unless the head is parked on the left. There is no command to check it. When the pre-send check (section 4) says not ready, the message tells the user to check that the head is parked left. | [1] | R |
| Never USB-reset | Do not reset the USB device by default. It broke usbip setups (#10). | [M] | R, #10 |
| Status 3 and 4 | Paused and after-cancel are not ready. Do not send a job then. | [M] | R |
| Regmark area offset | The regmark area is about 10 mm off to the left on a Cameo 2 (#205). Not used in M1. | [1] | R, #205 |
| No official docs | Silhouette America declined to publish an SDK (#72). | [M] | R, #72 |

## 8. Open questions for the capture campaign

Captures: Silhouette Studio talking to Jordi's Cameo 2, recorded on the Mac (Wireshark on an XHC
interface with SIP off for a short time) or with a USB analyzer such as Cynthion. Every capture is
normalized to (direction, endpoint, hex) lines for replay tests. [1] U

| # | Question | Why it matters | Conf. | Source |
|---|---|---|---|---|
| 1 | Is the PID `0x112b`? What are the descriptors, endpoints, wMaxPacketSize and the `FG` string? | Transport setup | [1] | R, U |
| 2 | Does CUPS or a Silhouette helper hold interface 0 on macOS? | Claiming the device | [1] | R |
| 3 | Does Studio send `FC18`, or `FC0` with its own geometry correction? | Knife compensation mode | [M] (FC18 seen in other senders) | R, K |
| 4 | What does FC18 mean physically? What range does the Cameo 2 accept? | FC limit (now 0-100) | [1] | R, K |
| 5 | Does Studio send `TB50,0` on a Cameo 2, and is it needed? | Section 9 keeps it | [M] | R |
| 6 | How does Studio start and end a job (`H`, or `M<end>,0` + `SO0`)? Does it send `FG` or `FW`? | Job start and end bytes | [M] | R |
| 7 | How many points does Studio put in one `D`? Does it use decimals? | Draw chunking (FlatCraft uses 16) | [1] | R |
| 8 | Do `BZ`, `Y` or other curve commands work on a Cameo 2? | Curves are flattened today | [1] | R |
| 9 | What do `FY`, `FM`, `TB55`, `TB51` and `TB53` mean exactly? | Decoder text, regmarks | [1] | R, K |
| 10 | Which byte sequence does a track enhancing on / off change in Studio? | Confirms `FY0` / `FY1` | [1] | R |
| 11 | Do bidirectional reads work through `usbprint.sys` on a Cameo 2? | Windows transport | [1] | R |
| 12 | Is the status `4` real on a Cameo 2? | Status decoding | [1] | R |
| 13 | What does `FE` mean exactly? The research says lift between paths, the knife note says lift at sharp corners. | Decoder text, cut quality | [1] | R, K |

## 9. M1 job sequence, exact bytes

This section is the reference for the emitter's golden test (`square_job_golden`). If code and this section
disagree, this section wins: the implementer stops and reports.

### The job

Test input (FlatCraft choice (provisional), chosen for the golden test):

- Device: Cameo 2, dialect Cameo12
- Media: 304.8 x 304.8 mm (12 x 12 inch mat) = 6096 x 6096 units
- Speed 5, force 10
- Blade offset: firmware, `FC18`
- Track enhancing: off
- Path (x, y in mm): (10,10), (20,10), (20,20), (10,20), (10,10), then the overcut point (10.5,10)

The path is a 10 mm square with its upper-left corner at mat position x = 10 mm, y = 10 mm, with
0.5 mm overcut along the first edge.

Points in device units (`y,x`, `round(mm * 20)`):

| Point (x, y mm) | Device `y,x` | Conf. |
|---|---|---|
| (10, 10) | `200,200` | [M] (unit and order rules, section 2) |
| (20, 10) | `200,400` | [M] |
| (20, 20) | `400,400` | [M] |
| (10, 20) | `400,200` | [M] |
| (10, 10) | `200,200` | [M] |
| (10.5, 10) | `200,210` | [M] |

### Exact bytes, one command per row

| # | Bytes (hex-escaped ASCII) | Meaning | Conf. | Source |
|---|---|---|---|---|
| 1 | `\x1b\x04` | Init (ESC 0x04, no ETX) | [M] | R |
| 2 | `!5\x03` | Speed 5 | [M] | R |
| 3 | `FX10\x03` | Force 10 | [M] | R |
| 4 | `FC18\x03` | Firmware blade offset 18 | [M] | R, K |
| 5 | `FY1\x03` | Track enhancing off | [1] | R |
| 6 | `FN0\x03` | Portrait | [M] | R |
| 7 | `TB50,0\x03` | Portrait (TB50 form) | [M] | R |
| 8 | `\\0,0\x03` | Work area upper-left = `0,0`. FlatCraft choice (provisional). | [M] | R |
| 9 | `Z6096,6096\x03` | Work area lower-right = media height, width. FlatCraft choice (provisional). | [M] | R |
| 10 | `L0\x03` | Solid line. FlatCraft choice (provisional). | [M] | R |
| 11 | `FE0,0\x03` | Lift setting 0. FlatCraft choice (provisional). | [M] | R |
| 12 | `FF0,0,0\x03` | Corner setting 0. FlatCraft choice (provisional). | [M] | R |
| 13 | `M200,200\x03` | Move to the first point | [M] | R |
| 14 | `D200,400,400,400,400,200,200,200,200,210\x03` | Cut the other 5 points. At most 16 points per `D`: FlatCraft choice (provisional). | [M] | R |
| 15 | `H\x03` | Home. Also flushes the look-ahead (section 7). Trailing `H` on a completely sent job: FlatCraft rule. | [M] | R |

The complete job as one byte string:

```text
\x1b\x04!5\x03FX10\x03FC18\x03FY1\x03FN0\x03TB50,0\x03\\0,0\x03Z6096,6096\x03L0\x03FE0,0\x03FF0,0,0\x03M200,200\x03D200,400,400,400,400,200,200,200,200,210\x03H\x03
```

Command 8 (`\\0,0`) is a single backslash `0x5c` followed by `0,0`. The job has no `FG`, no `FW` and no
`ESC 0x05`.

### Rules behind these bytes

| Rule | Conf. | Source |
|---|---|---|
| `TB50,0` is included. Rule from the brief: include it only if the sources support it as [M]. The research lists both `FN0` and `TB50,0` in the typical Cameo 2 job, marked [M]. So it stays. Whether Studio sends it is open question 5. | [M] | R |
| No `FG` inside the job bytes. The job file is the same bytes a File Output writes, and it has no reply channel. | FlatCraft choice (provisional) | - |
| No `FW` media preset. FlatCraft sets the work area with `\` and `Z` instead. | FlatCraft choice (provisional) | - |
| Work area is `\0,0` and `Z<media height units>,<media width units>`, so media coordinates go to the device without an offset. | FlatCraft choice (provisional) | - |
| `L0`, `FE0,0`, `FF0,0,0` are sent with these fixed values. | FlatCraft choice (provisional) | - |
| `FY1` is sent when track enhancing is off, `FY0` when it is on. It is always sent. `FN0` and `TB50,0` are always sent. The FY on/off meaning is from one source. | FlatCraft choice (provisional); FY meaning [1] | R |
| A path with fewer than 2 points is skipped: nothing is emitted for it. A job with no paths left is an error, never an empty job. | FlatCraft rule | - |
| Each path is one `M` to its first point, then `D` commands with the remaining points, at most 16 points per `D`. A 40-point path gives `M` plus `D` commands with 16, 16 and 7 points. | FlatCraft choice (provisional) | - |
| Every job that is sent completely ends with `H`. After a failed or partial write or a cancel, nothing more is sent, not even `H` (section 7, failure stop). | FlatCraft rule, based on the look-ahead quirk [M] | R |
| Valid ranges checked before emitting: speed 1-10, force 1-33, FC 0-100, coordinates 0..media (section 2). Out of range is an error, not a clamp. | speed, force [M]; FC: FlatCraft choice (provisional) | R |
| Firmware compensation sends `FC<fc>`. Host compensation and "none" send `FC0`. | [M] | R, K |
