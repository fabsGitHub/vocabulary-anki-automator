# justfile — cross-tree (Rust + Vue) dev tooling for vocabulary-anki-automator
# Install `just` via `brew install just` (scripts/bootstrap-macos.sh does it).
# Run `just --list` to see all recipes. Rust commands run from the repo root
# and cover every Cargo workspace member.

set shell := ["bash", "-uc"]

# Default recipe: show available commands.
default:
    @just --list

# Run Automator in dev mode (frontend + backend, hot-reloaded).
dev:
    npm run tauri dev

# Fast unit tests: Rust lib/bin tests + Vitest. No network, no real
# Anki/AnkiConnect (Milestone A4 hard rule, docs/test-harness-strategy.md).
test-unit:
    cargo test --workspace --lib --bins
    npm run test:unit

# Integration tests: mock AnkiConnect / HTTP providers via wiremock, SQLite in
# temp dirs, tauri::test mock runtime. Still never touches a real Anki install.
test-integration:
    cargo test --workspace --test '*'

# End-to-end tests (tauri-driver + WebdriverIO) are not wired up yet: they need
# a Linux/Windows runner and dependencies that have not been approved.
test-e2e:
    npm run test:e2e

# Watch loop: re-check and re-test the Rust workspace on change. Vite already
# hot-reloads the Vue side inside `just dev`.
watch:
    cargo watch -x "check --workspace --all-targets" -x "test --workspace --lib"

# Lint everything: rustfmt, clippy (deny warnings), vue-tsc, ESLint, Prettier.
lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    npx vue-tsc --noEmit
    npx vue-tsc --noEmit -p apps/learn
    npm run lint

# Learn: desktop dev (Tauri) and the standalone web build.
learn-dev:
    npm run learn:tauri dev

learn-web:
    npm run learn:dev

# Chains the fast test tiers; the same script CI and the pre-commit hook run.
check-all:
    ./scripts/run-all-checks.sh

# HSK 3.0 curriculum ETL (Milestone J2b), e.g. `just hsk-import build --help`.
hsk-import *ARGS:
    cargo run -p hsk-import -- {{ARGS}}
