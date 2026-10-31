#!/bin/bash
# scripts/run-all-checks.sh
#
# Chains the unit + integration test tiers. Shared entry point used by the
# `just watch`/`just check-all` loop, the pre-commit hook and CI
# (.github/workflows/ci.yml) — one script, not duplicated logic.
#
# Deliberately excludes the E2E tier (tauri-driver + WebdriverIO): it is slow,
# launches the compiled app, and tauri-driver has no macOS backend, so it
# belongs in Linux/Windows CI at pre-push/nightly, not in the inner loop.
#
# Nothing here may talk to a real Anki/AnkiConnect or any external API
# (Milestone A4 hard rule): HTTP-facing tests run against wiremock servers on
# 127.0.0.1, which needs local port binding to be allowed.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "==> Rust unit tests (all workspace members)"
cargo test --workspace --lib --bins

echo "==> Rust integration tests"
cargo test --workspace --test '*'

echo "==> Rust doc tests"
cargo test --workspace --doc

echo "==> Rust/TypeScript parity vectors (Learn web build)"
cargo run -q -p vocab-core --example golden_vectors -- --check

echo "==> Frontend unit tests (Vitest: Automator, then Learn incl. the web core)"
npm run test:unit

echo "==> all checks passed"
