# Changelog

All notable changes to this project will be documented in this file.
Versions follow a date-based scheme: `vYYYY.MM.DD.<revision>`.

## [2026.08.14.0] - 2026-08-14

### Features
- **First-launch onboarding** — the first time you open Matrix you're walked
  through the screens, keys, commands, and config in one focused welcome
  screen; reopen it anytime with `h` or `:welcome`.
- **Welcome screen redesign** — pixel-art logo, dimmed background scrim,
  high-contrast layout with an aligned key table, wrapped body text, and a
  pinned action bar (`1` scan this machine, `2` add a project manually,
  `3` skip for now).
- **Scrim on every overlay** — the command palette, the project-detection
  modal, and the update card now dim the background behind them, matching
  the welcome screen.
- **Help key** — `h` opens the welcome/help screen from any view; host mode
  moved to `H` (shift+h) in Logs.

### Improvements
- Long lines wrap instead of clipping on narrow terminals.
- Empty project list points at `d` (scan this machine) and
  `:project <id> <abs_path> <command>` (manual add).

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
