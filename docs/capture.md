# Capture guide: recording the vendor software with Wireshark

"I am not locked in here with you. You are locked in here with me." - Rorschach, but for USB. We want to see exactly what Silhouette Studio says to the Cameo 2, so FlatCraft can say the same thing.

This guide is for a MacBook Air. Plan about two hours for the full run list. Read the whole guide once before you start.

## What you need

- The MacBook Air, the Cameo 2 and its USB cable.
- Silhouette Studio installed on the Mac.
- Wireshark (includes `tshark`): `brew install --cask wireshark`.
- Copy paper, a cutting mat, and a shallow blade setting (so the cuts are light).
- A checkout of FlatCraft on the same Mac, to run `cargo xtask capture`.

## Part 1: lower the macOS security (SIP) - READ THIS PART TWICE

> **WARNING: these steps lower the security of your Mac.** macOS hides USB capture behind System Integrity Protection (SIP). We turn it down for the capture session only and turn it back on at the end. Do not forget Part 7.

Apple Silicon Mac (MacBook Air M-series). Steps:

1. Shut the Mac down: Apple menu > Shut Down.
2. Press and hold the power button until you see "Loading startup options".
3. Click Options, then Continue. This is macOS Recovery. Log in if asked.
4. In the menu bar: Utilities > Terminal.
5. Type this and press Return:

   ```
   csrutil enable --without debug
   ```

   (This is the milder setting, sometimes called Permissive. If Wireshark shows no USB interface later, repeat these steps with `csrutil disable` instead. This is the stronger setting.) Both are wrong for daily use, so Part 7 matters.
6. Apple menu > Restart.

Check after the restart. Open Terminal and run:

```
csrutil status
```

It should say that SIP is disabled or that some protections are off. Write it down in your note file.

## Part 2: install the capture permission for Wireshark

1. Open Wireshark once. It asks to install "ChmodBPF". Let it install, enter your password.
2. If it does not ask: in the Wireshark disk image, run "Install ChmodBPF.pkg".
3. Log out and log in again (or restart).

## Part 3: find the USB interface (XHC)

macOS exposes the USB controllers to Wireshark as interfaces called `XHC20`, `XHC0`, `XHC1` and so on.

1. Plug in the Cameo 2 and switch it on.
2. Open Wireshark. On the start screen you see the list of interfaces.
3. Look for names that start with `XHC`. Each one is a USB controller.
4. Double-click one and look at the packet list while you open Silhouette Studio. The right one shows packets with protocol "USB" and the Cameo 2 (vendor ID `0x0b4d`, product ID `0x112b`).
5. If the packet list stays empty, stop the capture and try the next XHC interface.
6. No XHC interface at all, even with SIP lowered? Go to Part 8 (the Cynthion fallback).

Tip: the Cameo 2 may be on a different XHC when you use another USB port. Use the same port for the whole session.

## Part 4: the capture filter and the display filter

- Capture filter: leave it empty. USB packets on macOS come through the XHC interface unfiltered, and a capture filter can hide the useful ones.
- Display filter, to check what you see: `usb.idVendor == 0x0b4d || usb.capdata`
- The tool reads only packets that carry data (`usb.capdata`). You do not need to filter before saving.

## Part 5: one capture per run

For every run in the run list (Part 6):

1. In Silhouette Studio: Page Setup panel, choose the mat or "No mat" for this run. Load the mat with the paper.
2. Load the test pattern sheet and set the one setting that this run changes. Everything else stays at the baseline.
3. In Wireshark: Capture > Start (the blue fin). Wait two seconds.
4. In Silhouette Studio: click Send. Let the Cameo cut until it stops.
5. Wait two more seconds. In Wireshark: Capture > Stop.
6. File > Save As. Format: pcapng. Name: `<pattern>__<setting>.pcapng`.
7. Write the note file (template below).
8. Unload the mat. Check the cut. Go on to the next run.

Change one setting per run. Only one. "Make it so" works for Picard, not for science.

### Naming

`<pattern>__<setting>.pcapng` with two underscores between the parts. Examples:

- `square50__baseline.pcapng`
- `square50__speed3.pcapng`
- `square50__force20.pcapng`
- `square50__passes2.pcapng`
- `star__overcut-on.pcapng`
- `square10__mat12x12.pcapng`
- `square10__nomat.pcapng`

Lowercase, digits, hyphens. No spaces.

### The note file

Next to each capture, a text file `<same name>.md`:

```
# <pattern>__<setting>

- Date:
- Silhouette Studio version (Help > About):
- Studio edition (Basic, Designer, ...):
- macOS version:
- Plotter: Cameo 2
- Firmware string (the reply to FG, see below):
- Mat selected in Studio:
- Media: copy paper, blade setting:
- Speed:
- Force:
- Passes:
- Overcut / line segment overcut / track enhancing:
- Changed from baseline: <the one setting>
- Notes (anything odd, cut looked wrong, Studio showed a warning):
```

The firmware string: the capture of the first run contains the `FG` question and its reply (for example `CAMEO V1.10`). After you run the tool (Part 6.4), look under `# replies` in the `.gpgl.txt` file. If it is not there, Studio did not ask in that run. Copy it from another run.

## Part 6: the run list

The test pattern sheet comes from FlatCraft (command `tools.test_patterns`): 50 mm square, corner star, circles, hole in a square, open line, S-curve, a small "F" and a 100 mm ruler. If FlatCraft cannot make it yet, use a plain 50 mm square drawn in Studio and name the pattern `square50`.

Baseline values: pick the ones Studio proposes for "copy paper" and write them in the note file. Low, mid and high are three clearly different values within the Cameo 2 range (speed 1-10, force 1-33).

### 6.1 Cutting runs

| # | Name | What changes | Question it answers |
|---|---|---|---|
| 1 | `sheet__baseline` | nothing, Studio defaults for copy paper | How does a normal job start and end (`H`, or `M`+`SO0`)? Does Studio send `FC18`, or `FC0` with its own geometry? Does it send `FG` or `FW`? Does it send `TB50,0`? How many points are in one `D`, and are there decimals? Is there a status polling pattern? |
| 2 | `sheet__speed-low` | speed low | Which command carries speed, what is its value range and unit? |
| 3 | `sheet__speed-mid` | speed mid | Is the speed value linear? |
| 4 | `sheet__speed-high` | speed high | Same, upper end. |
| 5 | `sheet__force-low` | force low | Which command carries force, what is its range? |
| 6 | `sheet__force-mid` | force mid | Is force linear? |
| 7 | `sheet__force-high` | force high | Same, upper end. Is 33 the maximum? |
| 8 | `sheet__passes-2` | passes 2 | Does Studio send the paths twice, or something else? Does the second pass reverse direction? |
| 9 | `sheet__passes-3` | passes 3 | Same, three passes. |
| 10 | `sheet__overcut-on` | overcut on | What bytes does overcut change? Does Studio add geometry or a command? |
| 11 | `sheet__line-segment-overcut-on` | line segment overcut on | Same for the line segment option. How is it different from run 10? |
| 12 | `sheet__track-enhancing-on` | track enhancing on | Which bytes does track enhancing change (confirms `FY0` / `FY1`)? |
| 13 | `sheet__track-enhancing-off` | track enhancing off | Compare with run 12. |
| 14 | `sheet__blade-offset` | change the blade setting in Studio's cut settings, if the option exists | What do `FC` and the offset commands carry? What range does the Cameo 2 take? |
| 15 | `sheet__cancel` | click Send, then stop the job from Studio (or press the Pause/Stop on the machine) in the middle of the cut | How does a cancel look? Which bytes end the job? |

For each run, also write in the note file how the cut looked (corners, overcut, was the paper cut through).

### 6.2 Mat runs (needed for the mat assistant, spec section 9.1)

FlatCraft puts its 0,0 on the printed grid of the mat. To get the offsets right we need to know where the vendor software puts things. Use a small square (for example 20 mm x 20 mm) placed so its top-left corner is at exactly 10 mm, 10 mm on the printed grid. In Studio you set this in the Transform panel, Position. Cut it on copy paper.

For every mat run:

1. In Studio, Page Setup: choose the mat (or "No mat").
2. Place the paper with its corner at the top-left of the printed grid of the mat.
3. Draw the square, set its position to 10 mm, 10 mm, check that the red page border matches.
4. Capture and cut as in Part 5.
5. After the cut, take the mat out and measure with a ruler: distance from the cut square's top-left corner to the top edge and left edge of the grid, and of the mat. Write it in the note file.

| # | Name | Mat in Studio | Question it answers |
|---|---|---|---|
| 16 | `square10__mat12x12` | Cameo 12 x 12 inch mat | Which work area command and which start offset does Studio send for this mat? Where does a square at grid 10 mm, 10 mm land on the real mat? |
| 17 | `square10__mat12x24` | 12 x 24 inch mat | Same for the long mat. Do the offsets or the work area differ? |
| 18 | `square10__nomat` | "No mat" | Does the work area or the origin change without a mat? Where does the square land? |
| 19 | `square10__mat12x12-corner` | 12 x 12 mat, square at 0 mm, 0 mm (page corner) | Where is the true origin? Does Studio shift the job, or does the machine? |

Wrong measurements are fine. Write down what you see, even if it looks odd. The odd ones teach most.

### 6.3 Setup run

| # | Name | What | Question it answers |
|---|---|---|---|
| 20 | `connect__studio-start` | Start the capture, then start Silhouette Studio with the plotter on, wait until Studio shows the machine, stop. No cut. | What are the descriptors, endpoints and packet sizes? What is the first thing Studio asks (`ESC 0x05`, `FG`)? How does the status polling work? |

If Studio is already running, quit it first. The setup run captures the connection.

### 6.4 Turn each capture into fixtures

For each `.pcapng` file, in the FlatCraft checkout:

```
cargo xtask capture ~/Desktop/captures/sheet__baseline.pcapng
```

This writes two files into `tests/fixtures/captures/`:

- `sheet__baseline.jsonl` - every USB transfer: time in seconds, direction (`out` or `in`), endpoint, bytes in hex.
- `sheet__baseline.gpgl.txt` - the OUT bytes decoded, one command per line with its meaning, then the replies under `# replies`.

Options: `--out <folder>` for another folder, `--name <name>` for another file name.

If it says "tshark not found", install Wireshark (Part "What you need"). If it says "no USB data found", you picked the wrong XHC interface or the filter hid the data. Capture again.

Look at the `.gpgl.txt` file. Commands marked "unknown" are the interesting ones. Do not delete them. Put the note file next to them and commit all three files.

## Part 7: turn SIP back on - DO NOT SKIP

> **Restore the security of your Mac when the last capture is done.**

1. Shut the Mac down. Hold the power button until "Loading startup options".
2. Options > Continue. Log in if asked.
3. Utilities > Terminal.
4. Type:

   ```
   csrutil enable
   ```

5. Apple menu > Restart.
6. In Terminal, run `csrutil status`. It must say: `System Integrity Protection status: enabled.`

Also: Wireshark's ChmodBPF stays installed. You can remove it: in Wireshark, run "Uninstall ChmodBPF.pkg" from the disk image. It is optional. The recordings contain only your plotter traffic, but delete the pcapng files after you have the fixtures if they hold anything private.

## Part 8: the fallback, a Cynthion analyser

If macOS 27 shows no XHC interface even with SIP lowered, use a hardware USB analyser. The Cynthion (Great Scott Gadgets, about 210 US dollars) sits between the Cameo 2 and the Mac and records the traffic with no changes to macOS. SIP stays on.

1. Wire it as the Cynthion documentation describes for a pass-through capture: the Cameo 2 on the device side, the computer that runs Silhouette Studio on the host side, the Cynthion in between.
2. Install the Cynthion tools (Packetry) as the Cynthion documentation says. Packetry shows the packets and can export a capture.
3. Run the same run list. Export each capture so we get direction, endpoint and bytes. Then convert it: if you can export pcapng, use `cargo xtask capture`. If not, send the export to the project and we write a small reader.
4. SIP is not touched, so Parts 1, 2 and 7 do not apply.

## Quick checklist for each run

- [ ] Mat or no mat set in Studio, paper loaded
- [ ] Only one setting changed from the baseline
- [ ] Wireshark capturing before Send
- [ ] Cut finished, capture stopped, saved as `<pattern>__<setting>.pcapng`
- [ ] Note file written
- [ ] `cargo xtask capture` run, files in `tests/fixtures/captures/`

And at the very end: **SIP back on** (Part 7). "With great power comes great responsibility."
