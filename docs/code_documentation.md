# Matrix TUI — Developer Notes

Notes for developers working on Matrix: how the pieces fit together, where the
interesting logic lives, and why certain things are the way they are. Not a
tutorial — a map.

## Architecture at a glance

Matrix is a single Rust binary (`tokio` async runtime + `ratatui`). Roughly
three layers:

```
main.rs ── startup, CLI client mode, main event loop
  │
  ├── app/        App state + orchestration: event routing, command mode,
  │               palettes, modals, toasts. Renders via ratatui.
  │
  ├── engine/     ProcessManager: the only place that spawns/kills processes.
  │               Owns the process map, log buffers, port allocation.
  │
  └── features/   One MVC trio per tab (Dashboard, Projects, Logs, Env).
                  Models hold state, views render, controllers handle keys.
```

Everything that touches a running process goes through `ProcessManager`
(`src/engine/mod.rs`). Views never spawn or kill anything themselves — they
ask the manager.

## Layout

```
src/
├── main.rs                 # entry point, CLI client mode, event loop
├── common.rs               # theme tokens, ANSI stripping, toasts, ActiveView
├── socket.rs               # /tmp/matrix-control.sock — external control
├── detect.rs               # scans the filesystem for runnable projects
├── update.rs               # update check + self-update
├── url.rs                  # open_in_browser, per-OS
├── config/
│   ├── mod.rs              # Project, BackendSpec, EnvSpec, Template, Group
│   └── tests.rs
├── engine/
│   ├── mod.rs              # ProcessManager — spawn/kill/deps/cascade
│   ├── env.rs              # env resolution: files, defaults, templates
│   └── tests.rs
├── app/
│   ├── mod.rs              # App state, init, config persistence
│   ├── ui.rs               # ratatui layout
│   ├── events.rs           # input dispatch
│   └── commands.rs         # ':' command parser + suggestions
└── features/               # one MVC trio per tab
    ├── mod.rs
    ├── dashboard/          # resource overview
    ├── env/                # .env editor
    ├── logs/               # live log viewer
    └── projects/           # process list + start/stop
```

`build.rs` embeds the exact git tag into the binary — see
[Versioning & release](#versioning--release).

## Startup flow (src/main.rs)

1. **CLI client mode** — if args are given, Matrix acts as a client to the
   running instance: `matrix update` self-updates, `matrix uninstall` removes
   the binary (with a config-removal prompt), anything else is sent verbatim
   over the control socket and the response is printed. No TUI. `update` and
   `uninstall` work without a running instance (see src/uninstall.rs).
2. **Single-instance guard** — `socket::is_socket_owned()` refuses a second
   TUI if a live instance already owns the socket.
3. **Terminal setup** — raw mode, alternate screen, mouse capture.
4. **Config load** — `~/.matrix/matrix.json`, then
   `config.normalize_paths()` makes relative paths absolute against the
   config's directory (so the same file works from any launch location).
5. **ProcessManager** is constructed from the config and wrapped in `Arc` —
   it's shared by the app, the socket server, and background threads.
6. **Toast channel** — an unbounded `mpsc`; the TUI, socket commands, and
   background threads all push `ToastEvent`s through it so external actions
   show up live in the UI.
7. **Update check** — a plain `std::thread` queries GitHub once (non-blocking);
   the result arrives on a channel and pops the bottom-right update card.
8. **Socket server** — `tokio::spawn` runs `socket::run_socket_server`.
9. **Event loop** — drain toasts → drain update result → `terminal.draw`
   → poll input (100 ms tick) → `app.update_system()` (CPU/mem refresh every
   2 s) → every 30 s, `manager.enforce_log_cap()` trims log buffers.
10. **Shutdown** — `manager.shutdown()` (kills remaining children), restore
    the terminal.

## The app layer (src/app/)

`App` (app/mod.rs) is the single struct holding all global UI state: active
view, sidebar, command mode, the detect modal, per-feature models, the toast
stack, and the update card. Feature-specific state lives in each feature's
model, not in `App`.

### Event routing (app/events.rs)

`handle_event` dispatches in priority order:

1. **Update card** — `u` opens the release page (never dismisses the card);
   `Esc`/`Enter` dismisses. Enter deliberately does nothing else.
2. **Command mode** — keys go to `handle_command_key`.
3. **Detect modal** — `o` toggles sort order (needs App state), everything
   else goes to `DetectController::handle_key` (navigate/add/close).
4. **Normal mode** — routed to the active view's controller, with global
   shortcuts (view switching, `:` command mode, `q` quit, `d` detect).

While editing (port/category in Projects, or the env editor) global
shortcuts are disabled so typing doesn't trigger tab switches or quit.

### Command mode (app/commands.rs)

`COMMAND_TEMPLATES` is the single registry: command name + usage hint array.
It drives the palette filter (`compute_command_matches`, prefix match) and
the ghost-template hints.

- `split_flags` separates `-flag` tokens (standalone booleans) from
  positionals, so `template -a tpl proj` parses as
  `positionals = ["tpl", "proj"]`, `flags = {"a"}`.
- `Tab` completes the highlighted command or path suggestion; `Enter`
  executes (or applies a suggestion); `Esc` clears.
- Path suggestions run in `tokio::task::spawn_blocking` — a slow disk must
  never freeze the key handler.

### Modals and toasts

The command palette and detect modal are rendered as centered overlays with
fixed geometry per terminal size (they must not jump/resize as you type).
Toasts are capped at 4 visible, pruned by TTL once per frame.

## The engine (src/engine/)

`ProcessManager` owns the `processes: HashMap<String, ProcessHandle>` map.
Everything that touches a process goes through this map; the `is_running`
closure used by env resolution reads the same map.

### Starting a project

1. Resolve dependencies: `project.deps` plus an implicit `engine:<id>`
   dependency when the project has a `backend`. Dependencies start first,
   recursively, with a cycle guard (visited chain in `start_core`).
2. Resolve the port. A `backend.port` is fixed; otherwise the engine port
   is `parent_port + 1` via `find_available_port`. Standalone projects
   (no group, no virtual) start at 5173; grouped ones at 3000.
3. Resolve env from `EnvSpec`s — see env.rs below.
4. Spawn in its own process group (`setpgid(0, 0)` in `pre_exec`) so the
   whole tree can be killed together.
5. Pipe stdout/stderr into tokio reader tasks; log lines land in the
   handle's `logs: Arc<Mutex<VecDeque<String>>>` buffer.

### Stopping a project

SIGINT to the process group, wait ~1s, SIGKILL if still alive. Cascades to
the backend first. The child is reaped (an async `child.wait()` task marks
the handle `Stopped`/`Crashed` when the process exits on its own — no
zombies). A crashed process can be restarted; its status resets to `Running`
on the next start.

### Port availability

`is_port_available` binds `127.0.0.1:port` and `[::1]:port` (IPv6
`AddrInUse` counts as taken — otherwise a service on `0.0.0.0`/`::1` makes
the check falsely succeed). `find_available_port` walks up from the start
port, skipping an explicit skip-list. The check-to-spawn race still exists
in principle; it's inherent to this pattern and low-risk for a dev tool.

### Host mode

`h` in the Logs tab toggles host mode for the viewed engine process: Matrix
restarts it with Vite's `--host` flag. The firewall rule alone can't
redirect traffic to a 127.0.0.1-only socket, so the process itself must
listen on all interfaces.

## Env resolution (src/engine/env.rs)

An `EnvSpec` is `{ key, value?, file[], default?, if_running?, else_value? }`.
Resolution order per key:

1. explicit `value` (or `else_value` when `if_running`'s project is not running)
2. first match in `file` list, read in order (.env format)
3. templated `default`

`PORT` and `ENGINE_PORT` are always injected in addition. Templates use
`{{...}}`: `{{id}}`, `{{path}}`, `{{port}}`, `{{parent_port|3000}}`
(pipe = fallback), `{{backend_port}}`, `{{dbname}}`, `{{dbname_upper}}`,
`{{env:KEY}}` (an earlier entry in the same list), `{{port+5}}`.
Unknown placeholders stay literal. Backend env specs resolve against the
parent project (its `{{id}}`/`{{path}}`/`{{parent_port}}`; `{{port}}` is the
backend's own).

## The log pipeline (src/features/logs/)

This is the most performance-sensitive code in the app. Log lines stream in
continuously; word wrapping and URL detection are expensive, so they never
run on the render path.

### The processor thread (processor.rs)

- A `SharedLogCache` holds the processed output, keyed by
  `(project_id, width, rev)` — rev is the engine's log revision at capture
  time, so a cache entry is only valid while nothing new arrived.
- The processor thread receives `Process` commands (project, lines, rev,
  width). It holds the lock only while *reading* the raw lines; all heavy
  work (ANSI strip, wrap, URL detect) happens outside the lock.
- **Burst coalescing**: before processing, it drains queued `Process`
  commands and keeps only the newest. A burst of N new lines becomes one
  recompute instead of N — this is what stops the worker falling behind
  under heavy output.

### Rendering without flicker (view.rs)

The render path never blocks on the processor:

- If the cache is valid for `(project, width, rev)`, render it.
- Otherwise enqueue a non-blocking re-process *only if the request changed*
  (a per-frame resend would flood the channel — the worker also coalesces),
  and fall back to the stale inline cache. This is the flicker fix: while
  the worker recomputes, the previous frame's wrapped lines are still shown.
  Gating on `rev` here was the root cause of the "Processing logs…" flash —
  a new line bumps `rev` but the old lines are still valid to display.
- First frame ever for a project renders the raw lines inline (cheap, no
  wrap pass) so there's never a flash even on first view.

The controller handles selection, click-to-open URLs (checking `cache_urls`
first, then falling back to scanning the raw line), and clipboard copy.

## Detection (src/detect.rs)

`default_roots()` scans `$HOME` itself (so projects anywhere in user space
are found), pruning noise via `is_skipped`: `node_modules`, hidden dirs,
SDK roots (GOPATH, cargo home, etc.), library leaves (`packages/`,
`plugin-*`, `ui`, `shared`), and old version snapshots (keeps the newest
`vN` sibling). Scan depth is capped (depth 8) so a deep monorepo can't
drag the scan out. Workspace manifests (`package.json` workspaces,
`pnpm-workspace.yaml` globs like `'apps/*' -> "apps"`) are expanded so
monorepo apps surface as individual candidates.

`App::open_detect_modal` scans, then drops any candidate whose canonical
path or id already exists in Matrix. Candidates sort by language (category)
by default; `o` toggles to by-name. `add_detected_project` creates a
`Project` from the candidate and saves config.

The detector has a fixture-based test, not a real-tree dependency.

## Control socket (src/socket.rs)

Listens on `/tmp/matrix-control.sock`. Commands are newline-terminated
(`start <id>`, `stop <id>`, `restart <id>`, `status`, …); the response ends
with a `--END--` marker so clients can stop reading without waiting for
EOF — the connection stays open to accept further commands. `is_socket_owned()`
guards against a stale socket from a dead instance.

The CLI client (`matrix <command>`) is a thin wrapper: it connects, writes
the command, reads until `--END--` or timeout, and prints the response.

## Config persistence

- **One config per device**: `~/.matrix/matrix.json`, regardless of launch
  directory. `default_config_path()` resolves it from `$HOME`.
- `MatrixConfig { projects, templates, groups }` — plain serde_json,
  pretty-printed on save.
- `save_config()` (app/mod.rs) creates the config directory if needed and
  writes the manager's current projects/templates/groups. Called after any
  mutation (add/delete/edit project, template/group changes).
- Relative paths are normalized to absolute against the config directory at
  load time (`normalize_paths`), so a relative path in the file is never
  interpreted against wherever `matrix` was launched from.

## Update flow (src/update.rs)

- **Version scheme**: date-based `vYYYY.MM.DD.<revision>` tags, compared
  tuple-wise by `parse_version`/`is_newer`. Anything unparseable is never
  "newer" (fail closed).
- **Check**: at startup, a background thread hits
  `api.github.com/repos/ItzJoris03/matrix/releases/latest` (6 s timeout).
  Any failure returns `None` silently — the TUI must never stall over a
  network check. The repo is public, so no token is needed; the
  `GITHUB_TOKEN` path remains for private forks. A newer release pops the
  bottom-right card; `u` opens the release page.
- **Self-update**: `perform_update` downloads the platform asset (named
  `matrix-<OS>-<arch>`, matching install.sh), writes to
  `<exe>.update.tmp`, chmods +x, then atomically renames over the running
  binary. It never exits the process itself — the TUI runs it on a
  background thread and surfaces the result as a toast; the CLI wrapper
  (`matrix update`) exits nonzero on failure.

## Versioning & release

`Cargo.toml` can only carry a 3-part semver (`2026.8.13`), but releases are
4-part (`v2026.08.13.5`). `build.rs` therefore runs
`git describe --tags --exact-match` at compile time and bakes the full tag
into `MATRIX_BUILD_VERSION`. When built from a non-tagged checkout the
binary falls back to the Cargo version formatted as `2026.08.12.0`
(see `format_version` in app/mod.rs). Consequences:

- **A release binary must be built after tagging**, or the embedded version
  won't match the release.
- The binary reports the exact tag in the header and the update check
  compares against it.

Release flow: tag → `cargo build --release` → `gh release create` with
`install.sh` + the binary as assets.

## Testing & CI

- `src/engine/tests.rs` — process manager unit tests (spawn/stop/port).
- `src/config/tests.rs` — config model tests.
- `src/detect.rs` — fixture-based detection tests.
- `src/update.rs` — version parse/compare tests (fail-closed behavior).
- `src/app/mod.rs` — version-formatting tests.

CI (`.github/workflows/ci.yml`, on push to main + PRs) runs:
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`. Keep it green — it's the only gate before release.

## Design decisions worth knowing

- **Process groups** (`setpgid` + kill by group): the whole child tree dies
  together, and backends cascade before their parent.
- **Graceful stop**: SIGINT first, 1 s grace, SIGKILL only if still alive —
  children get their cleanup hooks (no orphaned databases, no stale locks).
- **Both-stack port binding**: a port is only free if bindable on
  `127.0.0.1` *and* `[::1]`.
- **Per-device config, not per-folder**: Matrix is an orchestrator, not a
  project-local tool; one config follows you everywhere.
- **The log pipeline never blocks render**: processor thread + stale-cache
  fallback + burst coalescing is the whole story of why logs stay smooth
  under heavy output. Don't "simplify" it by moving work back into render.
- **Unknown `{{placeholders}}` stay literal**: configs are forward-compatible
  with older binaries; a typo never silently becomes an empty string.
- **License**: GPL-3.0-or-later — forks must stay open. Contributions are
  welcome; they lock the license in, which is intended.
