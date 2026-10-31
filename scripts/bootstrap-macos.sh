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

# --- tauri-driver / safaridriver: intentionally NOT installed ---------------
# Milestone A1 originally listed `cargo install tauri-driver` and
# `safaridriver --enable` here. tauri-driver only drives WebKitWebDriver (Linux)
# and Microsoft Edge Driver (Windows); there is no WKWebView driver on macOS,
# so neither tool can automate this app on a Mac. The E2E tier (Milestone G)
# therefore belongs on a Linux/Windows CI runner, not in this bootstrap.

# --- just: task runner for the repo's justfile ------------------------------
if command -v just >/dev/null 2>&1; then
  log "just already installed ($(just --version))"
else
  log "installing just"
  brew install just || fail "brew install just failed"
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

# --- frontend test runner: vitest / @vue/test-utils / jsdom -----------------
log "installing npm dependencies from package-lock.json"
(cd "$REPO_ROOT" && npm ci) || fail "npm ci failed"

# --- RTK: token-efficiency wrapper for cargo/npm build output (A3) ----------
if command -v rtk >/dev/null 2>&1; then
  log "rtk already installed"
else
  log "installing rtk"
  brew install rtk || fail "brew install rtk failed"
fi

# --- pre-commit hook: install the tracked script into git's hooks dir -------
# `git rev-parse --git-path hooks` names that dir in a plain clone and in a
# linked worktree, where .git is a file and the hooks live in the common dir.
HOOK_SRC="$REPO_ROOT/scripts/hooks/pre-commit"
if HOOKS_DIR="$(git -C "$REPO_ROOT" rev-parse --path-format=absolute --git-path hooks 2>/dev/null)"; then
  log "installing pre-commit hook into $HOOKS_DIR"
  mkdir -p "$HOOKS_DIR" || fail "failed to create $HOOKS_DIR"
  cp "$HOOK_SRC" "$HOOKS_DIR/pre-commit" || fail "failed to install pre-commit hook"
  chmod +x "$HOOKS_DIR/pre-commit" || fail "failed to chmod pre-commit hook"
else
  log "not a git checkout, skipping pre-commit hook install"
fi

log "bootstrap complete"
