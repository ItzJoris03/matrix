# Contributing

Matrix is a small project with a clear scope: a fast, logic-driven dev
engine and TUI for managing local projects. If that sounds useful to you,
contributions are welcome.

## Ground rules

- Matrix is GPL-3.0-or-later. By submitting a pull request you agree to
  license your contribution under that license (Developer Certificate of
  Origin, https://developercertificate.org/). That's non-negotiable —
  it's what keeps Matrix free.
- Keep the scope tight. A PR that adds a whole new feature tab will get
  more scrutiny than a focused fix. If you want to build something big,
  open an issue first and talk it through.
- No AI-generated drive-by PRs. We all use AI tools — but run the output
  through your own judgment, match the existing style, and don't send
  twenty near-identical "fix typo" PRs.

## Setting up

```sh
git clone https://github.com/ItzJoris03/matrix.git
cd matrix
cargo build
```

The config lives in `matrix.json` in the directory you run Matrix from.
See the README for the schema — `deps`, `backend`, and `env` are the
interesting fields.

## Running the tests

```sh
cargo test
```

All tests must pass before a PR is merged. The detector test
(`scan_fixture_tree_filters_snapshots_and_libs`) is fixture-based, so it
doesn't depend on your local filesystem layout — keep it that way.

## Style

- `cargo fmt` before committing.
- Comments when they clarify, not when they restate. Prefer readable code
  over explanatory comments.
- No personal strings in the source. Matrix is generic by design — project
  specifics belong in `matrix.json`, not in code.

## What's a good first PR?

- Start with the developer notes (`docs/code_documentation.md`) — it's the map
  of the codebase and where the interesting logic lives.
- Port conflicts, log rendering edge cases, and env-resolution corner
  cases are all good hunting grounds.
- Look for `TODO` and `FIXME` comments in the source.

## Reporting bugs

Open an issue with: what you did, what you expected, what happened, and —
if it's a crash — the output of `RUST_BACKTRACE=1 matrix`. If a project
fails to start, include the relevant part of `matrix.json` (redact any
secrets).
