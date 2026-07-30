#!/bin/bash
# scripts/bootstrap-macos.sh
#
# Idempotently installs everything Milestone A (IMPLEMENTATION_PLAN.md, "A1.
# macOS environment") requires on a developer's macOS machine. Safe to re-run:
# every step checks whether its tool is already present/enabled before acting.
# No interactive prompts. Exits non-zero on the first failure so CI/dev
# scripts can tell success from failure.
#
# Hard rule (Milestone A4): this script never touches Anki or AnkiConnect
# itself — no `download_and_install_anki` / `auto_provision_ankiconnect`
# equivalent lives here. Those are explicit, consent-gated, user-triggered
# actions inside the app (see Milestone E), never part of dev-environment
# bootstrap.

set -euo pipefail

log() { printf '[bootstrap] %s\n' "$1"; }
fail() { printf '[bootstrap] ERROR: %s\n' "$1" >&2; exit 1; }

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "this script targets macOS only (uname reported $(uname -s)); see IMPLEMENTATION_PLAN.md Milestone A1"
fi

command -v brew >/dev/null 2>&1 || fail "Homebrew is required but not installed (https://brew.sh)"
command -v cargo >/dev/null 2>&1 || fail "Rust/cargo is required but not installed (https://rustup.rs)"
command -v npm >/dev/null 2>&1 || fail "Node/npm is required but not installed"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# --- cargo-watch: Rust watch loop -------------------------------------------
if cargo watch --version >/dev/null 2>&1; then
  log "cargo-watch already installed ($(cargo watch --version))"
else
  log "installing cargo-watch"
  cargo install cargo-watch || fail "cargo install cargo-watch failed"
fi

# --- tauri-driver: E2E automation of the compiled app -----------------------
if command -v tauri-driver >/dev/null 2>&1; then
  log "tauri-driver already installed ($(tauri-driver --version 2>&1 || true))"
else
  log "installing tauri-driver"
  cargo install tauri-driver || fail "cargo install tauri-driver failed"
fi

# --- gh: PR/issue CLI workflow -----------------------------------------------
if command -v gh >/dev/null 2>&1; then
  log "gh already installed ($(gh --version | head -1))"
else
  log "installing gh"
  brew install gh || fail "brew install gh failed"
fi

# --- watchman: single cross-tree (Rust + Vue) file watcher ------------------
# Decision (A1): standardize on Watchman rather than fswatch.
if command -v watchman >/dev/null 2>&1; then
  log "watchman already installed ($(watchman --version))"
else
  log "installing watchman"
  brew install watchman || fail "brew install watchman failed"
fi

# --- safaridriver: enable the built-in WebDriver backend used by tauri-driver
if safaridriver --enable >/dev/null 2>&1; then
  log "safaridriver enabled"
else
  # safaridriver --enable exits non-zero if already enabled on some macOS
  # versions; only treat this as fatal if it's not already enabled.
  if safaridriver --enable 2>&1 | grep -qi "already enabled"; then
    log "safaridriver already enabled"
  else
    fail "safaridriver --enable failed (may require: sudo safaridriver --enable)"
  fi
fi

# --- frontend test runner: vitest / @vue/test-utils / jsdom -----------------
log "syncing npm devDependencies (vitest, @vue/test-utils, jsdom, eslint, prettier)"
(cd "$REPO_ROOT" && npm install) || fail "npm install failed"

# --- RTK: token-efficiency wrapper for cargo/npm build output (A3) ----------
if command -v rtk >/dev/null 2>&1; then
  log "rtk already installed"
else
  log "installing rtk"
  brew install rtk || fail "brew install rtk failed"
fi

# --- pre-commit hook: install the tracked script into .git/hooks -----------
HOOK_SRC="$REPO_ROOT/scripts/hooks/pre-commit"
HOOK_DEST="$REPO_ROOT/.git/hooks/pre-commit"
if [[ -d "$REPO_ROOT/.git" ]]; then
  log "installing pre-commit hook"
  cp "$HOOK_SRC" "$HOOK_DEST" || fail "failed to install pre-commit hook"
  chmod +x "$HOOK_DEST" || fail "failed to chmod pre-commit hook"
else
  log "no .git directory found, skipping pre-commit hook install"
fi

log "bootstrap complete"
