<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/icon-dark-256.png">
    <img src="assets/branding/icon-light-256.png" width="112" height="112" alt="FlatCraft">
  </picture>
</p>

<h1 align="center">FlatCraft</h1>

<p align="center">
  <strong>Your cutting plotter, back in the game. Agents welcome.</strong><br>
  An open-source desktop app for cutting plotters like the Silhouette Cameo 2. Bring in a design,
  place it on the mat and cut it. Or let your AI agent prepare the job through the built-in MCP server.
</p>

<p align="center">
  <a href="#status">Status</a> ·
  <a href="#what-it-will-do">Features</a> ·
  <a href="#ai-agents-mcp">MCP</a> ·
  <a href="#safety-first">Safety</a> ·
  <a href="docs/protocol/gpgl.md">Protocol notes</a> ·
  <a href="CONTRIBUTING.md">Contributing</a> ·
  <a href="#sponsors">Sponsor</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/github/license/jordiboehme/flatcraft" alt="License: MPL-2.0"></a>
  <a href="#status"><img src="https://img.shields.io/badge/status-early-orange" alt="Status: early"></a>
  <a href="https://github.com/jordiboehme/crystalline"><img src=".github/badges/works-best-with-crystalline.svg" alt="Works best with Crystalline"></a>
  <a href="https://github.com/jordiboehme/flatcraft/stargazers"><img src="https://img.shields.io/github/stars/jordiboehme/flatcraft?style=flat" alt="Stars"></a>
  <a href="https://ko-fi.com/V7V31T6CL9"><img src="https://img.shields.io/badge/Ko--fi-support-FF5E5B?logo=kofi&logoColor=white" alt="Support on Ko-fi"></a>
</p>

![FlatCraft: a teal cutting mat with a ruler, paper pieces lifted off the mat in their cut order, and the dotted travel path between them](assets/branding/banner.png)

You have a cutting plotter. It works fine. The software that drives it is about to stop working on your computer. That's how good machines end up in landfill: not broken, just abandoned.

FlatCraft is the way out. It works like a slicer for 3D printers: one profile per plotter, one per material, your own presets on top. Josef Prusa showed how good that feels for printers. FlatCraft brings it to flat things.

One Rust app. No account, no cloud, no subscription. Where we're going, we don't need logins.

## Status

Early. Nothing to download yet.

- Now: M1, the slicer app with File Output. It's built in the open, one reviewed task at a time ([PR #1](https://github.com/jordiboehme/flatcraft/pull/1)).
- Next: M2, a knife simulator that measures cut quality, and the final path algorithms.
- Then: M3, USB for a real Cameo 2, with ruler tests and a calibration wizard.
- Later: M4, a signed app, Homebrew and self-update.

Star the repo if you want to see it cut. Don't stop believin'.

## What it will do

- **Import in true size:** SVG in px, mm or inches comes in at the right size. PNG, JPEG, WebP and GIF are traced into cut outlines, with a live preview.
- **Arrange like a slicer:** move, scale, rotate, mirror, align and auto-arrange on the mat. No vector editor, no node fiddling: bring it in, place it, cut it.
- **Profiles, PrusaSlicer style:** plotter and material presets with inheritance. Speed, force and passes per material, and a quick override per job.
- **Cut quality first:** holes before outlines, so a piece never moves before it's done. Start points on corners. An overcut that closes every shape. Passes that stay on one shape until it's finished.
- **File Output:** the exact bytes the plotter would get, plus a readable listing of every command. Think G-code for your plotter. It's the default device, so nothing moves by accident.
- **Two workspaces:** Prepare (ribbon, objects, settings) and Cut (device and material, path preview, simulate, send).

## Build from source

You need Rust (rustup picks the right version from `rust-toolchain.toml`). M1 lives on the `m1-slicer` branch until it's done:

```sh
git clone -b m1-slicer https://github.com/jordiboehme/flatcraft.git
cd flatcraft
cargo test --workspace
```

On Linux you also need the window system headers:

```sh
sudo apt-get install libxkbcommon-dev libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libwayland-dev
```

## AI agents (MCP)

FlatCraft speaks [MCP](https://modelcontextprotocol.io) over stdio. Your agent's client starts it, so there's no server to run, no port and no token. For Claude Code:

```sh
claude mcp add flatcraft -- flatcraft mcp --window
```

With `--window` the app opens and you watch the agent work. Without it FlatCraft runs headless, which is fine for File Output.

Everything a person can do, an agent can do too, from the same command list: import, trace, arrange, pick plotter and material, preview and send to File Output. Units are millimetres, the origin is the top-left corner of the mat.

The blade stays your call. A USB cut started by an agent needs your click in the window, or a setting you turned on yourself.

## Safety first

A cutting plotter has a blade, and a blade has no undo button. FlatCraft follows a few hard rules:

- **No calibration writes:** FlatCraft can't send the commands that change a plotter's factory calibration. A last check on every byte refuses anything it doesn't know.
- **No half jobs:** a job that was sent completely ends with "home". If a write fails halfway, FlatCraft sends nothing more. Don't panic: use the stop button on the machine.
- **People confirm real cuts:** an agent can prepare a cut, but a USB cut needs your click or a setting you turned on yourself.
- **No double knife correction:** the blade offset is corrected in exactly one place, on the plotter or in FlatCraft, never both.

With great power comes great responsibility. With great force comes a cut mat.

## Why

There's a Cameo 2 on my desk. It's years old and it still cuts fine. The vendor software will soon drop support for my system, and then it's a very precise paperweight.

Adam Savage likes to say that failure is always an option. Giving up on a working machine shouldn't be.

So FlatCraft is long-life software for the plotters we already own. It's in the spirit of the makers who share how they work: Adam Savage, Josef Prusa, Laura Kampf and Felix Schelhasse. Build it, fix it, keep it running.

## Principles

- **Local only:** no cloud services, no telemetry. Your designs stay on your machine.
- **Measured, not guessed:** cut quality is tested with a simulator and with a ruler on real paper.
- **Clean licenses:** FlatCraft uses only libraries with permissive or MPL licenses.
- **Honest output:** when something can't be cut, FlatCraft says what and why.

## Contributing

Help is welcome, most of all from people with a plotter on their desk. [CONTRIBUTING.md](CONTRIBUTING.md) covers how to build and test, the clean-room rule for protocol work and how to report a plotter that isn't supported yet. Open work is in [GitHub issues](https://github.com/jordiboehme/flatcraft/issues). For a security problem, read [SECURITY.md](SECURITY.md).

## Sponsors

FlatCraft is free and open source. If it saved your plotter from the bin, you can support the work here:

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/V7V31T6CL9)
[![PayPal](https://img.shields.io/badge/PayPal-sponsor-blue?logo=paypal)](https://paypal.me/jordiboehmelopez)

Want to sponsor support for your plotter? [Open an issue](https://github.com/jordiboehme/flatcraft/issues) and let's talk.

## Disclaimer

FlatCraft drives a machine with a blade. Use it at your own risk: start with paper and a shallow blade, and keep a hand near the stop button. The software comes as is, without warranty (see the [license](LICENSE)).

## License

[MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/). FlatCraft was written from public protocol facts and its own notes, without copying code from other plotter drivers.

FlatCraft is independent. It is not affiliated with, sponsored by or endorsed by Silhouette America or Graphtec. Silhouette and Cameo are trademarks of their owners, named only to describe compatibility.

*Keep cutting.*
