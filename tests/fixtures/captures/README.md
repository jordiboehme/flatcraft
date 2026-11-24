# Captured USB traffic

Fixtures made from Wireshark recordings of the vendor software talking to a Cameo 2. They are the ground truth for the GPGL emitter tests.

Each capture gives two files plus a note:

- `<name>.jsonl` - one JSON object per USB transfer: `{"t": seconds, "dir": "out"|"in", "ep": "0x01", "hex": "..."}`. Same format as the Debug panel byte log.
- `<name>.gpgl.txt` - the OUT bytes decoded, one command per line with its meaning, then the replies under `# replies`.
- `<name>.md` - the note file (Studio version, firmware string, settings). Template in `docs/capture.md`.

Make them with `cargo xtask capture <file.pcapng>`. The full guide is `docs/capture.md`.

Naming: `<pattern>__<setting>`, for example `square50__speed3`.
