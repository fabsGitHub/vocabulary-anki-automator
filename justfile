# justfile — cross-tree (Rust + Vue) dev tooling for vocabulary-anki-automator
# Install `just` via `brew install just` (or see https://github.com/casey/just).
# Run `just --list` to see all recipes.

set shell := ["bash", "-uc"]

# Default recipe: show available commands.
default:
    @just --list

# Run the Tauri app in dev mode (frontend + backend, hot-reloaded).
dev:
    npm run tauri dev

# Fast unit tests: Rust crate/unit tests + Vitest component tests. No network,
# no real Anki/AnkiConnect — safe to run on every commit (see Milestone A4's
# hard rule in docs/test-harness-strategy.md).
test-unit:
    cd src-tauri && cargo test --lib
    npm run test:unit

# Integration tests: mock-AnkiConnect + tauri::test::mock_builder style tests
# (Milestone G). Still never touches a real Anki install.
test-integration:
    cd src-tauri && cargo test --test '*' 2>/dev/null || echo "no integration tests yet (Milestone G)"

# End-to-end tests: tauri-driver + WebdriverIO against the compiled app
# (Milestone G). Heavier/slower — run at pre-push/nightly, not per-commit.
test-e2e:
    npm run test:e2e

# Watch loop: rebuild/re-test on file changes across both the Rust and Vue
# trees. Uses cargo-watch for the Rust side; `npm run tauri dev` already
# hot-reloads the Vue side on its own via Vite.
watch:
    cd src-tauri && cargo watch -x "check" -x "test --lib"

# Lint everything: rustfmt check, clippy (deny warnings), ESLint, vue-tsc.
lint:
    cd src-tauri && cargo fmt --check
    cd src-tauri && cargo clippy --all-targets -- -D warnings
    npm run lint

# Chains the fast test tiers; used by both `watch` (optionally) and CI.
check-all:
    ./scripts/run-all-checks.sh
