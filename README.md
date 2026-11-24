# FlatCraft

**Free your cutting plotter. Keep the Cameo out of landfill.**

[![License: MPL-2.0](https://img.shields.io/github/license/jordiboehme/flatcraft)](LICENSE)
[![Status: M1](https://img.shields.io/badge/status-M1%20slicer-orange)](#status)

Open-source desktop software for cutting plotters like the Silhouette Cameo 2. One Rust app, no account, no cloud, no subscription. It also speaks MCP, so an agent can prepare a cut for you.

## Why

Plenty of perfectly good cutting plotters are one dead app away from the bin. The hardware is fine. The software was tied to a vendor, a login or an old operating system. That is a sad end for a machine that can cut a Millennium Falcon out of cardstock.

FlatCraft is the fix. It takes your artwork, turns it into clean cut paths and writes a file the plotter understands. No landfill. No "your device is no longer supported".

> "Failure is always an option." - Adam Savage

A plotter that gets thrown away never gets the chance to fail, learn and cut again. We say: keep it running. Like Def Leppard put it, we want to pour some sugar on it - and by sugar we mean fresh cardstock.

Think of it as the Mr. Fusion for old plotters: you feed it what you have, and it keeps the flux capacitor of your workshop running. We're gonna need a bigger mat.

## Status

M1: slicer with File Output. FlatCraft does not talk to a plotter over USB yet. It prepares the job and writes it to a file. Direct USB cutting comes in a later milestone, after the file output has proven itself on real material.

What works in M1:

- Import SVG and common raster images (PNG, JPEG, WebP, GIF)
- Turn pictures into cut paths, with your own tracing (no Potrace)
- Flatten curves, join paths and add overcut and blade offset
- Order paths so the blade does not take the scenic route
- Device profiles with a media size and margins for the Cameo 2
- File Output as the default device
- An MCP server so an agent can open, check and preview a job

Like Van Halen, we check the bowl for brown M&Ms: the details matter. Think of M1 as the first Back to the Future film: the DeLorean gets to 88 mph, and the road trip comes later.

## Build from source

You need Rust. rustup picks the right version from `rust-toolchain.toml`.

```sh
git clone https://github.com/jordiboehme/flatcraft.git
cd flatcraft
cargo build --release
cargo run --release -p flatcraft
```

On Linux you also need the development packages for the window system:

```sh
sudo apt-get install libxkbcommon-dev libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libwayland-dev
```

Run the tests with `cargo test --workspace`.

## Safety first

A cutting plotter has a blade, and a blade does not have an undo button. So:

- File Output is the default device. Nothing moves until you choose a real device.
- FlatCraft has no commands that write calibration settings to a machine.
- Every job that is sent in full ends with a clean "home" command.
- If a write fails halfway, FlatCraft stops and sends nothing more. No heroics.
- An agent can prepare a cut, but it cannot confirm its own cut over USB. A human clicks the button.

As Spider-Man's uncle said: with great power comes great responsibility. As the Cameo 2 says: with great force comes a ruined mat.

## Principles

- **Local first.** No cloud services, no telemetry.
- **Clean licenses.** FlatCraft links only libraries with permissive or MPL licenses. No GPL code was read or copied.
- **Honest output.** When something can't be done, FlatCraft says so.

## Support

FlatCraft is free and open source. If it saved your plotter, you can support the work here:

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/V7V31T6CL9)
[![PayPal](https://img.shields.io/badge/PayPal-sponsor-blue?logo=paypal)](https://paypal.me/jordiboehmelopez)

## License

[MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/)

*Cut it out. Keep it rolling.*
