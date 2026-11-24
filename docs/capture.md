# Capture guide: recording the vendor software with Wireshark

"I am not locked in here with you. You are locked in here with me." - Rorschach, but for USB. We want to see exactly what Silhouette Studio says to the Cameo 2, so FlatCraft can say the same thing.

This guide is for a MacBook Air. Plan about two hours for the full run list. Read the whole guide once before you start.

## What you need

- The MacBook Air, the Cameo 2 and its USB cable.
- Silhouette Studio installed on the Mac.
- Wireshark with `tshark` (Part 1).
- Copy paper, a cutting mat and a shallow blade setting (so the cuts are light).
- A checkout of FlatCraft on the same Mac, to run `cargo xtask capture`.

## Be honest about the odds

Capturing USB on a Mac is a hack, not a feature. It uses hidden `XHC` interfaces that Apple never promised to keep. A report on the Apple developer forums says it may no longer work on macOS 15.6 and later. **It may fail on macOS 27.** So Part 3 is a go/no-go test. If it fails, we stop and use Plan B (Part 8). Do not spend two hours on the run list before the test says go.

Sources:
- https://wiki.wireshark.org/CaptureSetup/USB
- https://ask.wireshark.org/question/23351/
- https://developer.apple.com/forums/thread/124875

## Part 1: install Wireshark (before the SIP reboot)

```
brew install --cask wireshark-app
```

The cask is called `wireshark-app`. The installer also installs ChmodBPF (the permission helper). That step needs a **reboot**. We do it once, together with the SIP reboot in Part 2, so you reboot into Recovery directly after this install.

Source: https://formulae.brew.sh/api/cask/wireshark-app.json

Also unplug every other USB device now. Use the Cameo's own cable, straight into the Mac. No hub, no dock.

## Part 2: lower the macOS security (SIP) - READ THIS PART TWICE

> **WARNING: these steps lower the security of your Mac.** macOS hides USB capture behind System Integrity Protection (SIP). We turn it off for the capture session only and turn it back on at the end. Do not forget Part 9.

While the Mac runs in this mode, **iOS and iPadOS apps on the Mac stop launching.** That is expected. It comes back when you restore security.

Apple Silicon Mac (MacBook Air M-series):

1. Shut the Mac down: Apple menu > Shut Down.
2. Press and hold the power button until you see "Loading startup options".
3. Click Options, then Continue. This is macOS Recovery. Log in if asked.
4. In the menu bar: Utilities > Startup Security Utility. Select your startup disk, click Security Policy, choose **Reduced Security**, confirm with your admin credentials.
5. In the menu bar: Utilities > Terminal. Run:

   ```
   csrutil disable
   ```

   Answer `y` when asked and give the admin credentials. This is the only SIP command. Nothing milder is needed.
6. Apple menu > Restart.

The resulting boot policy is called "Permissive Security". It is not a milder SIP mode. It is simply what the Mac reports once SIP is off.

After the restart, open Terminal and run:

```
csrutil status
```

It must say that System Integrity Protection is disabled. Write it in your note file.

Sources:
- https://eclecticlight.co/2024/08/21/controlling-system-integrity-protection-using-csrutil-a-reference/
- https://support.apple.com/guide/security/sec7d92dc49f

## Part 3: preflight - go or no-go

Do this before anything else. It takes five minutes.

1. Plug in the Cameo 2 (switch it off for now).
2. List the capture interfaces:

   ```
   ifconfig -l | tr ' ' '\n' | grep XHC
   ```

   or `tshark -D`. You should see names such as `XHC0`, `XHC1`, `XHC20`. These are the USB controllers.
3. Bring **every** XHC interface up, one command each:

   ```
   sudo ifconfig XHC0 up
   sudo ifconfig XHC1 up
   ```

   Repeat for each one in your list. You must do this again after every reboot.
4. Open Wireshark and select the XHC interfaces (or run `tshark -i XHC0 -a duration:10`). Start a 10 second test capture. During those 10 seconds, switch on the Cameo 2 (or unplug and plug in its cable).
5. Look at the packets. You must see packets that carry the Cameo's descriptors: vendor ID `0x0b4d`, product ID `0x112b`.

How to see the Cameo:
- In the Wireshark GUI, type this display filter in the bar and press Return: `usb.idVendor == 0x0b4d`. Packets that remain are the Cameo's descriptors.
- Several XHC interfaces at once: in the start screen, Cmd-click each XHC interface to select more than one, then start the capture.
- On the command line, one call per XHC interface:

  ```
  tshark -i XHC0 -a duration:10 -Y "usb.idVendor == 0x0b4d"
  tshark -i XHC1 -a duration:10 -Y "usb.idVendor == 0x0b4d"
  ```

  Switch the Cameo on during the 10 seconds. Any line of output means GO.

Decision:
- **Packets with 0b4d:112b appear: GO.** Continue with Part 4.
- **Nothing appears, on any XHC: NO-GO.** Stop here. Do not run the list. Go to Plan B (Part 8).

## Part 4: the capture setup

- Capture on the XHC interface that showed the Cameo in Part 3. Tip: the Cameo may sit on a different XHC when you change the port. Use the same port for the whole session.
- Capture filter: leave it empty. A capture filter can hide the useful packets.
- Display filter, to see what you have: `usb.capdata`. In the packet list, bulk packets to the Cameo show as `URB_BULK out`.
- The tool reads only packets that carry data. You do not need to filter before saving.
- Save every capture as pcapng.

## Part 5: one capture per run

Run 0 (the connection) comes first in the session. All other runs follow this recipe:

1. In Silhouette Studio: Page Setup panel, choose the mat or "No mat" for this run. Load the mat with the paper.
2. Load the test pattern sheet and set the one setting that this run changes. Everything else stays at the baseline.
3. In Wireshark: Capture > Start (the blue fin). Wait two seconds.
4. In Silhouette Studio: click Send. Let the Cameo cut until it stops.
5. Wait two more seconds. In Wireshark: Capture > Stop.
6. File > Save As. Format: pcapng. Folder: `~/Desktop/captures/` (create it once). Name: `<pattern>__<setting>.pcapng`.
7. Write the note file (template below).
8. Unload the mat. Check the cut. Go on to the next run.

Change one setting per run. Only one. "Make it so" works for Picard, not for science.

### Naming

`<pattern>__<setting>.pcapng` with two underscores between the parts. The run table below gives the exact name for every run. Lowercase, digits, hyphens. No spaces.

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

The firmware string: the capture contains the `FG` question and its reply (for example `CAMEO V1.10`). After you run the tool (Part 7), look under `# replies` in the `.gpgl.txt` file. If it is not there, Studio did not ask in that run. Copy it from another run.

## Part 6: the run list

The test pattern sheet comes from FlatCraft (command `tools.test_patterns`): 50 mm square, corner star, circles, hole in a square, open line, S-curve, a small "F" and a 100 mm ruler. If FlatCraft cannot make it yet, use a plain 50 mm square drawn in Studio and name the pattern `square50` instead of `sheet`.

Baseline values: pick the ones Studio proposes for "copy paper" and write them in the note file. Low, mid and high are three clearly different values within the Cameo 2 range (speed 1-10, force 1-33).

### 6.0 Run 0: the connection (first in the session)

| # | Name | What | Question it answers |
|---|---|---|---|
| 0 | `connect__studio-start` | Start the capture first. Then plug in or switch on the Cameo. Then start Silhouette Studio and wait until Studio shows the machine. Stop. No cut. | What are the descriptors, endpoints and packet sizes? Which device address does the Cameo get (the tool uses it to ignore other devices)? What is the first thing Studio asks (`ESC 0x05`, `FG`)? How does the status polling look? |

If Studio is already running, quit it first. Run 0 must see the plug-in.

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
| 20 | `sheet__corner-sharpen` | the Studio option that changes how sharp corners are cut (look for it in the cut settings; use the corner star pattern). If Studio has no such option, skip the run and write that in the notes. | What does `FF` really do, and which values does Studio send? Does it differ between the star's sharp and wide corners? |

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

### 6.3 Check run 0 and run 1 right away

After run 0 and again after run 1, do not wait for the end of the session. Save the pcapng in `~/Desktop/captures/` and run the tool at once (Part 7):

```
cargo xtask capture ~/Desktop/captures/connect__studio-start.pcapng
cargo xtask capture ~/Desktop/captures/sheet__baseline.pcapng
```

Then check three things before you run the rest of the list:

- In Wireshark you see `URB_BULK out` packets to the Cameo.
- The tool printed a device address and no "no commands found" warning.
- The `.gpgl.txt` file has commands and none of them appear doubled.

If one check fails, stop and fix the setup first.

## Part 7: turn each capture into fixtures

For each `.pcapng` file, in the FlatCraft checkout:

```
cargo xtask capture ~/Desktop/captures/sheet__baseline.pcapng
```

It works on macOS (Darwin), Linux and Windows (USBPcap) captures.

This writes two files into `tests/fixtures/captures/` of the checkout, wherever you start the command:

- `sheet__baseline.jsonl` - every USB transfer: time in seconds, direction (`out` or `in`), endpoint, device address (`dev`), bytes in hex.
- `sheet__baseline.gpgl.txt` - the OUT bytes decoded, one command per line with its meaning, then the replies under `# replies`.

Options: `--out <folder>` for another folder, `--name <name>` for another file name, `--device <n>` for the Cameo's USB device address.

What the tool tells you:

- It prints the bulk OUT bytes per device address and which address it used. By default it uses the busiest one, which is the Cameo. Check that this fits run 0. If not, pass `--device`.
- If it prints a warning about lines that could not be read, look at the first line it shows. A few bad lines can be fine. Many are not.
- If it says "no USB data found", check in Wireshark that you see URB_BULK out packets to the Cameo before you capture again, and keep the pcapng.

First-run check: open the `.gpgl.txt` file. **Commands must not appear doubled** (the same command twice in a row means the tool read both the submit and the complete packet). If you see doubles, stop and report it with the pcapng.

If it says "tshark not found", install Wireshark (Part 1).

Look at the `.gpgl.txt` file. Commands marked "unknown" are the interesting ones. Do not delete them. Put the note file next to them and commit all three files.

## Part 8: Plan B, if the Mac test says no-go

Ranked. Take the first one you can do.

### Plan B1: a Windows PC (best)

Any x86-64 Windows PC with Silhouette Studio installed. USBPcap works with the plain Windows driver and needs no security changes on the Mac.

1. Install Wireshark on the PC. In the installer, tick **USBPcap** and let it install. Reboot when asked.
2. Plug in the Cameo 2 and switch it on. Use its own cable and unplug other USB devices.
3. Start Wireshark. You see interfaces called `USBPcap1`, `USBPcap2` and so on, one per USB root hub. Pick the one the Cameo is on: open the interface options (gear icon), and in the USBPcap dialog, tick the device with `Silhouette` or vendor ID 0b4d. If unsure, start a capture on one hub, switch the Cameo on, and look for `0b4d:112b`. Try the next hub if nothing shows. In the same dialog, tick "Capture from all devices connected" and "Capture from newly connected devices". Optionally also tick "Inject already connected devices descriptors into capture data", so a capture started after the Cameo was plugged in still has its descriptors. The tool filters by device address anyway. Source: https://ask.wireshark.org/question/37235/capturing-usb-data
4. Use the same run list, the same names and the same note files. Run 0 first.
5. Copy the pcapng files to the Mac and run `cargo xtask capture` as in Part 7. The tool reads USBPcap captures.

### Plan B2: Cynthion and Packetry

The Cynthion (Great Scott Gadgets, about 210 US dollars) is a USB analyser. macOS stays untouched, so SIP stays on (skip Parts 2 and 9).

1. Connect the CONTROL port of the Cynthion to the computer that runs Packetry (the analysis computer).
2. Connect the TARGET C port to the host that runs Silhouette Studio.
3. Connect the TARGET A port to the Cameo 2. **Start the capture in Packetry before you connect TARGET A**, so you see the connection.
4. If the analyser does not respond, reload it: `cynthion run analyzer`.
5. Packetry saves `.pcap` files with LINKTYPE_USB_2_0. **The tool path for these files is untested.** Try `cargo xtask capture` on one. If it finds no data or looks wrong, keep the file and report it, and we adjust the tool.

Sources:
- https://packetry.readthedocs.io/en/latest/quick_start.html
- https://cynthion.readthedocs.io/en/latest/getting_started_packetry.html

### Plan B3: experimental, UTM or QEMU

A virtual machine with Windows or Linux and USB passthrough of the Cameo, with a USB pcap property on the virtual USB controller. This is experimental. We do not know if the vendor software runs well there, and timing may differ. Use it only if B1 and B2 are not possible. Report what you try.

## Part 9: turn SIP back on and clean up - DO NOT SKIP

> **Restore the security of your Mac when the last capture is done.**

1. Bring the capture interfaces down. For every XHC you brought up in Part 3:

   ```
   sudo ifconfig XHC0 down
   ```

   (Same for `XHC1` and the others.) A reboot also resets them.
2. Shut the Mac down. Hold the power button until "Loading startup options".
3. Options > Continue. Log in if asked.
4. Utilities > Terminal. Type:

   ```
   csrutil enable
   ```

5. Utilities > Startup Security Utility. Select your startup disk, Security Policy, choose **Full Security**, confirm.
6. Apple menu > Restart.
7. In Terminal, run `csrutil status`. It must say: `System Integrity Protection status: enabled.` If it does not, repeat the steps.

Wireshark stays installed. To remove it and the ChmodBPF helper, run:

```
brew uninstall --cask wireshark-app
```

The recordings contain only your plotter traffic, but delete the pcapng files after you have the fixtures if they hold anything private.

## Quick checklist for each run

- [ ] Mat or no mat set in Studio, paper loaded
- [ ] Only one setting changed from the baseline
- [ ] Wireshark capturing before Send
- [ ] Cut finished, capture stopped, saved as `<pattern>__<setting>.pcapng`
- [ ] Note file written
- [ ] `cargo xtask capture` run, files in `tests/fixtures/captures/`, no doubled commands

And at the very end: **SIP back on and Full Security** (Part 9). "With great power comes great responsibility."
