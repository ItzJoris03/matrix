#!/usr/bin/env bash
# Matrix — install / update script
# Clones (or pulls) the repo, ensures the latest release binary is available,
# and falls back to a local `cargo build --release` if binaries aren't published.
#
# Usage:
#   ./install.sh            # install or update to the latest release
#   ./install.sh --debug    # build from source in debug mode (faster, unoptimized)
#   ./install.sh --source   # always build from source (release)
#
# Matrix is installed to ~/.local/bin/matrix (adjust INSTALL_DIR below).

set -euo pipefail

REPO="ItzJoris03/matrix"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
BIN="$INSTALL_DIR/matrix"
BUILD_MODE="release"   # release | debug
FORCE_SOURCE=0

for arg in "$@"; do
  case "$arg" in
    --debug)  BUILD_MODE="debug" ;;
    --source) FORCE_SOURCE=1 ;;
    -h|--help)
      if [ -f "$0" ]; then
        sed -n '2,16p' "$0"
      else
        echo "Matrix — install / update script"
        echo "Usage: curl -fsSL <install-url> | bash"
        echo "Options: --debug, --source"
      fi
      exit 0
      ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

NEED_CARGO=0

if command -v cargo >/dev/null 2>&1; then
  echo "✔ cargo found ($(cargo --version))"
else
  if [ "$FORCE_SOURCE" -eq 1 ]; then
    echo "✘ --source requested but cargo is not installed. Install Rust: https://rustup.rs" >&2
    exit 1
  fi
  NEED_CARGO=1
fi

# ── Try to fetch a prebuilt release binary (unless building from source) ──
fetch_release_binary() {
  if [ "$FORCE_SOURCE" -eq 1 ]; then return 1; fi

  # Token lets the script work against private repos too (public repos need none).
  local auth=()
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    auth=(-H "Authorization: Bearer $GITHUB_TOKEN")
  fi

  local release_json
  release_json="$(curl -fsSL "${auth[@]}" "https://api.github.com/repos/$REPO/releases/latest")" || {
    echo "  could not query GitHub releases, will build from source"
    return 1
  }

  local version
  version="$(printf '%s' "$release_json" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/')"
  if [ -z "$version" ]; then
    echo "  no release found, will build from source"
    return 1
  fi
  echo "  latest release: $version"

  local asset="matrix-$(uname -s)-$(uname -m)"

  # Parse the asset ID + browser URL out of the release JSON. python3 is
  # preferred (robust against field ordering); fall back to the plain URL.
  local asset_id asset_url
  if command -v python3 >/dev/null 2>&1; then
    asset_id="$(printf '%s' "$release_json" | python3 -c "
import json, sys
data = json.load(sys.stdin)
asset = sys.argv[1]
for a in data.get('assets', []):
    if a.get('name') == asset:
        print(a.get('id', ''))
        break
" "$asset")"
    asset_url="$(printf '%s' "$release_json" | python3 -c "
import json, sys
data = json.load(sys.stdin)
asset = sys.argv[1]
for a in data.get('assets', []):
    if a.get('name') == asset:
        print(a.get('browser_download_url', ''))
        break
" "$asset")"
  fi

  # Download. Try the browser URL first (the fast CDN path for public repos);
  # fall back to the API asset endpoint when a token is present (private repos
  # 404 on the browser URL even with a token, the API endpoint works for both).
  local download_ok=0
  if [ -n "$asset_url" ]; then
    echo "  downloading $asset_url"
    if curl -fsSL "$asset_url" -o "$BIN.tmp"; then
      download_ok=1
    fi
  fi
  if [ "$download_ok" -eq 0 ] && [ -n "${GITHUB_TOKEN:-}" ] && [ -n "$asset_id" ]; then
    local api_url="https://api.github.com/repos/$REPO/releases/assets/$asset_id"
    echo "  downloading (api) $api_url"
    rm -f "$BIN.tmp"
    if curl -fsSL "${auth[@]}" -H "Accept: application/octet-stream" "$api_url" -o "$BIN.tmp"; then
      download_ok=1
    fi
  fi

  if [ "$download_ok" -eq 1 ]; then
    chmod +x "$BIN.tmp"
    mv "$BIN.tmp" "$BIN"
    echo "✔ installed $version to $BIN"
    return 0
  fi

  echo "  no prebuilt asset for this platform, building from source"
  rm -f "$BIN.tmp"
  return 1
}

# ── Build from source via cargo ──
build_from_source() {
  if [ "$NEED_CARGO" -eq 1 ]; then
    echo "✘ cargo required to build from source but it is not installed." >&2
    exit 1
  fi
  local workdir
  if [ -d .git ] && [ -f Cargo.toml ] && grep -q 'name = "matrix"' Cargo.toml 2>/dev/null; then
    workdir="$(pwd)"
  else
    workdir="$(mktemp -d)"
    echo "  cloning $REPO into $workdir"
    local clone_url="https://github.com/$REPO.git"
    # Token in the URL only for private repos; public clones don't need it.
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      clone_url="https://x-access-token:${GITHUB_TOKEN}@github.com/$REPO.git"
    fi
    git clone --depth 1 "$clone_url" "$workdir"
  fi

  echo "  building ($BUILD_MODE)…"
  ( cd "$workdir" && cargo build ${BUILD_MODE:+"--$BUILD_MODE"} --bin matrix )
  local built="$workdir/target/${BUILD_MODE}/matrix"

  mkdir -p "$INSTALL_DIR"
  # Copy to a temp name then rename: a running binary can't be overwritten
  # in place (ETXTBSY), but renaming over it works on Linux/Unix.
  cp "$built" "$BIN.tmp"
  chmod +x "$BIN.tmp"
  mv -f "$BIN.tmp" "$BIN"
  echo "✔ built and installed to $BIN"
}

mkdir -p "$INSTALL_DIR"

if ! fetch_release_binary; then
  build_from_source
fi

# ── Ensure on PATH ──
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo
    echo "ℹ $INSTALL_DIR is not on your PATH. Add it to your shell profile:"
    echo "    export PATH=\"\$PATH:$INSTALL_DIR\""
    ;;
esac

echo
echo "Done. Run it with: matrix"
