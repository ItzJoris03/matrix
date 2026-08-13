# Changelog

All notable changes to this project will be documented in this file.
Versions follow a date-based scheme: `vYYYY.MM.DD.<revision>`.

## [2026.08.13.0] - 2026-08-13

Initial public release of Matrix.

### Features
- **Process management** — start, stop, restart projects and groups with
  keyboard-only control; dependency ordering, port conflict detection, and
  automatic fallback ports.
- **Project detection** — scan curated roots on disk for runnable projects
  and add them in one keystroke, sorted by language or name.
- **Live logs** — background processor thread keeps log views smooth under
  heavy output, with word wrapping, URL detection, ANSI cleaning, mouse
  selection, clipboard copy, and host-mode toggling.
- **Command mode** — `:` palette with autocompletion, path suggestions, and
  flag-based commands.
- **Environment editor** — interactive `.env` editing inside the TUI.
- **Dashboard** — CPU/RAM meters, running-service counts, per-service status.
- **External control** — Unix control socket (`/tmp/matrix-control.sock`) so
  other tooling can drive Matrix headlessly; `matrix <command>` CLI client.
- **Self-update** — `matrix update` swaps the binary in place from GitHub
  releases; `matrix uninstall` removes Matrix (binary only, or everything
  including `~/.matrix/` config, your choice).
