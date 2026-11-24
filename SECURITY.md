# Security policy

## Report a vulnerability

Please don't open a public issue for a security problem. Use GitHub's private vulnerability reporting instead:

1. Open the [Security tab](https://github.com/jordiboehme/flatcraft/security) of this repository.
2. Click "Report a vulnerability".
3. Describe the problem and how to reproduce it. A sample file helps a lot.

Only the maintainer can see your report.

## Supported versions

Only the latest release gets security fixes. Please check that the problem still exists there before you report it.

## How FlatCraft treats files and devices

FlatCraft opens files from unknown sources and talks to a machine with a sharp blade, so it is careful on both ends:

- FlatCraft never runs code from an imported file. SVG scripts and external references are ignored, and imports have size limits.
- File Output is the default device. Nothing goes to a USB cutter until you pick it.
- FlatCraft never sends commands that write calibration or firmware settings to a device. A last check refuses every byte it doesn't know.
- The MCP server only listens on localhost and needs a token. It checks where a request comes from, so a web page can't talk to it.
- An agent can prepare and preview a job. A USB cut started by an agent needs your confirmation in the app, unless you turned on "Allow agents to start cuts" yourself.

A way around one of these rules is a security problem. Please report it as described above.
