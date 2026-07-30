#!/bin/bash
# scripts/run-all-checks.sh
#
# Chains the unit + integration test tiers. Shared entry point used by both
# the `just watch` loop (for fast local feedback) and any future CI — one
# script, not duplicated logic in two places.
#
# Deliberately excludes the e2e tier (tauri-driver + WebdriverIO, Milestone G):
# that tier is slow and launches the compiled app, so it belongs at
# pre-push/nightly, not in the tight inner loop this script serves.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Rust unit tests"
(cd src-tauri && cargo test --lib)

echo "==> Rust integration tests"
if (cd src-tauri && cargo test --test '*' --no-run >/dev/null 2>&1); then
  (cd src-tauri && cargo test --test '*')
else
  echo "(no integration tests yet — Milestone G)"
fi

echo "==> Frontend unit tests (Vitest)"
npm run test:unit

echo "==> all checks passed"
