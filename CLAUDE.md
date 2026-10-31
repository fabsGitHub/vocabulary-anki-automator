# Vocabulary Automator

Two Tauri 2 desktop apps sharing one Rust core:

- **Automator** (`src-tauri/` + Vue frontend in `src/`): select text anywhere, press the
  hotkey, approve the generated card, and it lands in Anki and in the local SQLite history.
  Desktop-only by design (global hotkeys).
- **Learn** (`apps/learn/`): FSRS review sessions, the HSK daily queue and gamified quizzes over
  the same SQLite store; also builds as a standalone web app (`npm run learn:build:web`).
- **Core** (`crates/vocab-core`), **Anki boundary** (`crates/anki-bridge`), **HSK ETL**
  (`tools/hsk-import`).

## Read on demand (tiered docs)

| Need                                                       | Read                                                                 |
| ---------------------------------------------------------- | -------------------------------------------------------------------- |
| Module map, cross-crate APIs, schema, IPC commands, shapes | [docs/architecture.md](docs/architecture.md) — binding contract      |
| Roadmap, milestone status, what is blocked and why         | [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md)                     |
| How tests avoid real Anki / network                        | [docs/test-harness-strategy.md](docs/test-harness-strategy.md)       |
| Release pipeline, signing secrets                          | [docs/signing-key-custody.md](docs/signing-key-custody.md)           |
| HSK data provenance, licenses, regeneration                | [crates/vocab-core/data/README.md](crates/vocab-core/data/README.md) |

## Commands

```bash
just lint          # rustfmt, clippy -D warnings, vue-tsc, ESLint, Prettier
just check-all     # unit + integration tiers (scripts/run-all-checks.sh; CI runs the same)
npm run tauri dev  # Automator
npm run learn:tauri dev   # Learn (desktop); npm run learn:dev:web for the browser build
```

In a sandbox where `~/.cargo` is read-only, prefix cargo with an absolute `CARGO_HOME` such as
`/tmp/claude-501/cargo-home` (`$TMPDIR` differs inside and outside the sandbox, see
docs/architecture.md § 2); give npm `--cache "$TMPDIR/npm-cache"`. HTTP-mock tests (wiremock)
need local port binding.

## Rules that are easy to break

- **Git:** work on a feature branch (`pot-N-...` for Paperclip tickets), commit there, never
  directly to `main`; the operator merges (Tier 2 per SOUL.md). Never push. Tag releases
  `v0.MINOR.PATCH`; CI releases on tag push, and only the operator tags.
- **Dependencies:** adding any crate or npm package needs operator approval (Tier 2). Tauri crates
  and `@tauri-apps/*` packages must stay on the same major.minor (the CLI refuses to build
  otherwise) — they are pinned together in `Cargo.toml` and `package.json`.
- **Tests:** never touch the real network or a real Anki install. `download_and_install_anki` /
  `auto_provision_ankiconnect` run only from consent-gated UI actions, never at startup or in tests.
- **Anki field names** exist only in `crates/anki-bridge/src/mapper.rs`; the core `Card` is
  Anki-agnostic.
- **Quiz isolation:** every exercise draws from `LearnedVocabularyPool::load` and nothing else; the
  leak test in `crates/vocab-core/tests/quiz_isolation.rs` must keep failing when that is violated.
- **Signing keys** (updater private key, Apple/Windows certificates) never appear in code, logs,
  tickets or agent sessions — they exist only as GitHub Actions secrets.
