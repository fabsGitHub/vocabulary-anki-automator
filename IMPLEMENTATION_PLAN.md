# Implementation Plan — vocabulary-anki-automator

Status: **planning only** — nothing in this document has been implemented yet. This is the master TODO list for the app and the development workspace, derived from the architecture/vision session on 2026-07-27.

## Guiding Architecture Decisions (locked in)

1. **DeepL is removed entirely** — no `DeepLConfig`, no `api_key` field, no DeepL branding anywhere in the repo.
2. **Default translation provider: MyMemory API** — keyless, officially free, zero setup for anyone who clones the repo.
3. Translation/enrichment logic is built behind **trait-based provider interfaces** so providers are swappable and a future app can reuse the same pipeline.
4. Card data is a **neutral internal struct**, independent of Anki's field names. Anki-specific mapping happens only at the AnkiConnect boundary.
5. Local capture history is persisted in **SQLite from the start** — designed as the seed of a future native app's data store, not throwaway.
6. Long-term: shared Rust **core logic** reusable by both the current automator and a possible future standalone learning app. Anki-format export/import stays available as a portability guarantee even if a native app ships.

---

## Milestone A — Workspace & Dev Tooling Setup
*Ungated — can start immediately, no app-code dependency.*

### A1. macOS environment (verified against dev machine on 2026-07-27)
- [ ] `cargo install cargo-watch` — not installed; needed for the Rust watch loop
- [ ] `cargo install tauri-driver` — not installed; needed for E2E automation of the compiled app
- [ ] `brew install gh` — not installed; needed for PR/issue CLI workflow
- [ ] `npm install -D vitest @vue/test-utils jsdom` — no frontend test runner exists yet
- [ ] `safaridriver --enable` — one-time enable of the built-in WebDriver backend (binary already present on macOS)
- [ ] Standardize on a single file watcher for the cross-tree (Rust + Vue) watch loop: Watchman is already installed — use it, or install `fswatch` as an alternative. Pick one.

### A2. Repo scaffolding
- [ ] Add `justfile` (or root `package.json` scripts) with: `dev`, `test:unit`, `test:integration`, `test:e2e`, `watch`, `lint`
- [ ] Add `rustfmt.toml`
- [ ] Add `.eslintrc`/`eslint.config.js` + Prettier config for the Vue/TS side (no lint config exists today)
- [ ] Add a pre-commit hook (`husky` or a plain `.git/hooks/pre-commit` script) running: `cargo fmt --check`, `cargo clippy -- -D warnings`, `vue-tsc --noEmit`
- [ ] Create `scripts/bootstrap-macos.sh` — installs everything in A1 idempotently, checks versions, exits non-zero on failure, no interactive prompts
- [ ] Create `scripts/run-all-checks.sh` — chains the unit + integration test tiers; used by both the watch loop and any future CI

### A3. Token-efficiency / assistant-workflow setup
- [ ] Install RTK (`brew install rtk`) and adopt it for wrapping `cargo build` / `cargo tauri build` / `npm run build` output during local dev sessions
- [ ] Write `CLAUDE.md` using the tiered-doc convention (only essential context loads by default; deeper notes live in linked files loaded on demand) — do this once real docs exist, not before
- [ ] Deferred (tracked so it isn't forgotten, not actioned yet): CodeGraph, Token Optimizer MCP — revisit once the file count outgrows a single Explore pass

### A4. Test harness prerequisites
- [ ] Design a disposable Anki test profile strategy: either a temp `ANKI_BASE` directory the E2E harness points AnkiConnect at, or a lightweight mock AnkiConnect server (`wiremock`-based Rust binary or small Node stub) that records/asserts `addNote` payloads without needing real Anki
- [ ] **Hard rule:** `download_and_install_anki` / `auto_provision_ankiconnect` must never fire inside any automated test or watch-loop run — only against a developer's real Anki install after explicit consent (see Milestone E)

---

## Milestone B — Core Architecture Refactor
*Restructures `src-tauri/src` so Anki-specific assumptions stop leaking into capture/enrichment logic. Prerequisite for C, D, and F.*

### B1. Module layout
- [ ] Create `src-tauri/src/core/mod.rs` — new home for platform-agnostic logic
- [ ] Create `src-tauri/src/core/model.rs` — the neutral `Card` struct:
  ```rust
  pub struct Card {
      pub front: String,
      pub back: String,
      pub example_source: String,
      pub example_target: String,
      pub notes: String,
      pub tags: Vec<String>,
      pub source_lang: String,
      pub target_lang: String,
      pub captured_at: chrono::DateTime<chrono::Utc>,
      pub audio_front: Option<Vec<u8>>,
  }
  ```
- [ ] Create `src-tauri/src/core/providers/mod.rs` with trait definitions:
  - `trait TranslationProvider { async fn translate(&self, text: &str, source: &str, target: &str) -> Result<String, ProviderError>; }`
  - `trait ExampleProvider { async fn fetch_example(&self, word: &str) -> Result<String, ProviderError>; }`
  - `trait RarityProvider { async fn analyze(&self, word: &str) -> Result<RarityResult, ProviderError>; }`
  - `trait TtsProvider { async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError>; }`
- [ ] Create `src-tauri/src/core/providers/mymemory.rs` — implements `TranslationProvider` against MyMemory's REST API
- [ ] Move existing Datamuse logic out of `api_client.rs` into `src-tauri/src/core/providers/datamuse.rs` implementing `RarityProvider`
- [ ] Move existing dictionaryapi.dev logic into `src-tauri/src/core/providers/dictionary_api.rs` implementing `ExampleProvider`
- [ ] Move existing Google-Translate-TTS logic into `src-tauri/src/core/providers/google_tts.rs` implementing `TtsProvider` (kept only as the TTS source — not as the translation provider)
- [ ] Delete `src-tauri/src/api_client.rs` once its contents are fully redistributed into the provider modules above
- [ ] Create `src-tauri/src/core/pipeline.rs` — orchestrates capture → translate → rarity-check → example → tts → `Card`, replacing the inline logic currently in `trigger_clipboard_sequence` (`lib.rs:206-310`)
- [ ] Create `src-tauri/src/anki/mapper.rs` — the *only* place that converts a `Card` into Anki's `Front`/`Back`/`Context Sentence`/`Notes` field names; `anki.rs`'s `add_vocab_card` takes a `Card`, not raw strings

### B2. Config cleanup
- [ ] Remove `DeepLConfig` struct entirely from `config.rs`
- [ ] Remove `api_key` field and all DeepL references from `AppConfig`
- [ ] Rename the config section (currently `deepl`) to provider-neutral naming, e.g. `translation: TranslationConfig { source_lang, target_lang, max_chars, provider: String, contact_email: Option<String> }`
- [ ] Grep the whole repo for `deepl`/`DeepL` (check `tauri.conf.json` product name, `package.json` `name`, window titles, `Cargo.toml` description) and remove/rename every remaining reference
- [ ] Update `Cargo.toml` package `description` (currently references vocabulary automation only — confirm no stray DeepL wording elsewhere)

### B3. Wire `execute_card_generation` through the new pipeline
- [ ] Update `execute_card_generation` (`lib.rs:172-204`) to take a `Card` (from the pending-payload state) instead of four raw strings
- [ ] Update `PendingPayload` state to hold a typed `Card` instead of `serde_json::Value`

---

## Milestone C — Translation Provider Swap (MyMemory)
- [ ] Implement `MyMemoryProvider`: GET `https://api.mymemory.translated.net/get?q=...&langpair=SRC|TGT`, parse `responseData.translatedText`
- [ ] Handle MyMemory's quota-exceeded response distinctly from network failure or malformed response — current `translate()` swallows all failure modes into an empty string; replace with typed `ProviderError` variants surfaced to the UI
- [ ] Add an optional contact-email field to the translation config so users can self-raise their MyMemory daily quota (per MyMemory's documented mechanism)
- [ ] Update `App.vue`'s config form: confirm no stray DeepL-labeled fields remain; source/target language selects stay as-is since they're provider-agnostic

---

## Milestone D — MVP Completion: Approval Window
*This is the actual "does the app work end-to-end" gap identified during architecture review — the backend already emits events and exposes commands for this window; the frontend has no UI for it yet.*

- [ ] Create `src/views/ApprovalWindow.vue` (or a top-level branch in `App.vue` keyed on `windowLabel === 'preview'`)
- [ ] Listen for the `translation-payload` Tauri event (`listen()` from `@tauri-apps/api/event`) in the preview window
- [ ] On mount, also call the existing `get_pending_payload` command to cover the case where the window opens after the event already fired
- [ ] Build the approval UI: editable front/back/example/notes fields pre-filled from the payload, an "Approve" button calling `execute_card_generation`, a "Discard" button that closes/hides the window without saving
- [ ] On approve success, hide the `preview` window (currently built with `always_on_top(true)` and never explicitly hidden after use)
- [ ] Add a capabilities entry for the `preview` window in `src-tauri/capabilities/default.json` (currently scoped only to `"windows": ["main"]` — audit whether the preview window silently lacks a permission it needs)
- [ ] Manual test: trigger hotkey → confirm payload appears in the preview window → approve → confirm two Anki notes are created with swapped front/back

---

## Milestone E — Consent & Supply-Chain Hardening
- [ ] Replace the unconditional auto-run of `download_and_install_anki` + `auto_provision_ankiconnect` in `setup()` (`lib.rs:344-350`) with a gated flow: check-only on startup, explicit user-triggered install (the UI buttons already exist in `App.vue` — `runSetupInstallation`/`bootAnkiApp` — this is mostly about *removing* the silent auto-run on the Rust side)
- [ ] Pin the AnkiConnect script download (`lib.rs:133`) to a specific commit SHA of `FooSoft/anki-connect`, not `master`
- [ ] Add checksum/hash verification after downloading the AnkiConnect script and the Anki installer, before writing/executing either
- [ ] Change the AnkiConnect config this app writes (`lib.rs:127`) from `"webBindAddress"/"webCorsOriginList": ["*"]` to the app's actual origin only
- [ ] Add a one-time consent dialog before the Anki DMG/EXE download+install path runs at all

---

## Milestone F — Local History Store (secondary tier)
- [ ] Add `rusqlite` (or `sqlx` with the `sqlite` feature) to `Cargo.toml`
- [ ] Create `src-tauri/src/store/mod.rs` with a `CardStore` trait and a SQLite-backed implementation
- [ ] Schema: `captures(id, front, back, example_source, example_target, notes, tags, source_lang, target_lang, captured_at, synced_to_anki BOOLEAN)`
- [ ] Persist every approved card to local SQLite in addition to pushing it to Anki
- [ ] New Tauri commands: `list_recent_captures`, `delete_capture`
- [ ] Frontend: a simple history/review list view in the main window

---

## Milestone G — Testing Infrastructure
- [ ] Unit (Rust): table-driven tests for each provider's response parsing (MyMemory JSON shape, Datamuse frequency-tag parsing) against canned fixtures, no network
- [ ] Unit (Rust): `config.rs` save/load round-trip test against a temp directory (requires separating the path-taking core from the `AppHandle`-coupled wrapper)
- [ ] Unit (TS): extract `useAnkiConnection` and `useConfig` composables out of `App.vue`; Vitest coverage of the online/offline/plugin-missing state machine
- [ ] Integration: mock AnkiConnect server asserting the exact `addNote` payload shape produced by `anki/mapper.rs`
- [ ] Integration: `tauri::test::mock_builder` tests invoking `execute_card_generation` / `get_pending_payload` directly
- [ ] E2E: `tauri-driver` + WebdriverIO script — launch the compiled app, emit the internal capture event (not a real OS keystroke), assert the preview window renders the payload, click approve, assert two notes hit the mocked AnkiConnect stub
- [ ] Wire `test:unit`/`test:integration` into the pre-commit hook (fast tiers only); keep `test:e2e` at pre-push/nightly

---

## Milestone H — Secondary Feature Polish
- [ ] Multi-word/phrase capture: relax `analyze_rarity`'s space-containing-string bailout; decide fallback behavior for phrases (skip rarity check, still translate)
- [ ] Configurable Anki note-type field mapping instead of the hardcoded `AppleStyleFinalV3` assumption — deck/model selects already exist in the UI, field mapping doesn't
- [ ] i18n: `ui_lang` is persisted but nothing branches on it — introduce `vue-i18n`, extract the currently hardcoded English strings from `App.vue`
- [ ] Per-language hotkey profiles (capture directly into a chosen language pair without changing global settings each time)

---

## Milestone I — Advanced / Native-App Foundations
*Long-term north star. Not scheduled — listed so the shared-core decisions in Milestone B stay intentional rather than accidental.*

- [ ] Extract `src-tauri/src/core/` into an actual Cargo workspace member (`crates/vocab-core`) once a second consumer exists
- [ ] Second Tauri app (`apps/learn/`) consuming `vocab-core` — own review-session UI, FSRS-based scheduler instead of relying on Anki's
- [ ] `ArgosTranslateProvider` — fully offline translation (implements the same `TranslationProvider` trait), likely via bundled ONNX-exported Argos models rather than shelling out to Python
- [ ] FSRS scheduler integration operating over the local SQLite store from Milestone F
- [ ] Duolingo-style mechanics: XP tally, streak + streak-freeze, tied to completed review sessions (not raw captures)
- [ ] Babbel-style contextual lesson cards built from the user's own captured `example_source`/`example_target` pairs, not a generic curriculum
- [ ] Anki export/import (`.apkg` read/write or AnkiConnect bulk sync) so the native app never locks users out of their existing Anki collection
- [ ] Mobile targets via Tauri 2's mobile support (iOS/Android) for the learning app specifically — the automator stays desktop-only since global-hotkey capture is inherently a desktop concept
