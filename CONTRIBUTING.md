# Contributing to FlatCraft

Thanks for helping. FlatCraft keeps old cutting plotters running, and every report from a real machine makes it better.

## Build and test

You need Rust. rustup picks the right version from `rust-toolchain.toml`.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

CI runs the same checks on macOS, Linux and Windows. A pull request needs a green `ci-passed` check.

## The clean-room rule

Most open-source plotter drivers are GPL or LGPL. FlatCraft is MPL-2.0, so it can't take code from them.

- Protocol facts are fine from any source: command names, argument order, units, USB IDs, status codes, captured bytes.
- Facts go into [docs/protocol/gpgl.md](docs/protocol/gpgl.md) first, in your own words, with their source and a confidence mark.
- Protocol code is written only from that file. Don't open the source code of GPL or LGPL plotter drivers while you work on FlatCraft code.
- Dependencies must have a permissive or MPL license. `cargo deny check` enforces it.

## Safety rules

A plotter has a blade. These rules are not up for change in a pull request:

- Never send commands that write calibration or firmware settings.
- After a failed or partial write, send nothing more.
- A USB cut started by an agent needs a person's confirmation or their explicit setting.
- The blade offset is corrected in one place only: on the plotter or in FlatCraft.

## Report a plotter

Have a plotter FlatCraft doesn't support yet? Open an issue with the model, the USB vendor and product ID (on a Mac: `system_profiler SPUSBHostDataType`) and, if you have one, a USB capture of a simple cut.

## Commits

Plain conventional commits (`feat:`, `fix:`, `docs:` and so on). Keep a pull request to one topic.
