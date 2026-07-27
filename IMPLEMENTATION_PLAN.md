# Implementation Plan — vocabulary-anki-automator

Status: **planning only** — nothing in this document has been implemented yet. This is the master TODO list for the app and the development workspace, derived from the architecture/vision session on 2026-07-27.

## Guiding Architecture Decisions (locked in)

1. **DeepL is removed entirely** — no `DeepLConfig`, no `api_key` field, no DeepL branding anywhere in the repo.
2. **Default translation provider: MyMemory API** — keyless, officially free, zero setup for anyone who clones the repo.
3. Translation/enrichment logic is built behind **trait-based provider interfaces** so providers are swappable and a future app can reuse the same pipeline.
4. Card data is a **neutral internal struct**, independent of Anki's field names. Anki-specific mapping happens only at the AnkiConnect boundary.
5. Local capture history is persisted in **SQLite from the start** — designed as the seed of a future native app's data store, not throwaway.
6. Long-term: shared Rust **core logic** reusable by both the current automator and a possible future standalone learning app. Anki-format export/import stays available as a portability guarantee even if a native app ships.
7. **Chinese/HSK support extends the neutral `Card` model via a `ScriptMetadata` enum** (a `Chinese { pinyin, hsk_level, script_variant }` variant) rather than bolting hardcoded fields onto the base struct — the same seam accommodates a future script (Japanese furigana/JLPT, Korean romanization, …) without another migration.
8. **Scheduling algorithm: FSRS, not classic SM-2.** Modern Anki itself defaults to FSRS since 23.10, so building on the same algorithm family keeps the Anki-export/import portability guarantee (decision 6) semantically meaningful instead of requiring lossy conversion between two different scheduling models.
9. **HSK curriculum content (hanzi, pinyin, level, example sentences) ships as bundled static reference data**, not a live API call — consistent with the "zero-setup for anyone who clones the repo" principle already applied to the translation provider (decision 2).

---

## Development Approach — Model & Effort Guidance
*For reference when actually implementing each milestone. ("Ultracode" was checked against this machine's config and isn't a real Claude Code tier — this table just records the model/effort assignment agreed on 2026-07-27.)*

Default unless a row below says otherwise: **Sonnet 5 at `high` effort.** Escalate to **Opus 4.8**, and to **`xhigh`/`max`** effort specifically, whenever the work touches one of four things: the provider-trait/`Card`-model boundary (Milestone B), the FSRS scheduler (Milestone L), the vocabulary-isolation guarantee in the quiz engine (Milestone M), or the auto-install/consent security surface (Milestone E).

| Milestone | Model | Effort | Why |
|---|---|---|---|
| A — Workspace & dev tooling | Sonnet 5 | high | mechanical setup |
| B — Core architecture refactor | **Opus 4.8** | **high** | foundational boundary everything else builds on |
| C — Translation provider swap | Sonnet 5 | high | mechanical, well-scoped |
| D — Approval window UI | Sonnet 5 | high | UI wiring |
| E — Consent & supply-chain hardening | **Opus 4.8** | **xhigh/max** | security-sensitive (auto-install, unpinned script download) |
| F — Local history store | Sonnet 5 | high | mechanical CRUD |
| G — Testing infrastructure (general) | Sonnet 5 | high | scaffolding |
| G+ — SRS/quiz leak test specifically | **Opus 4.8** | **xhigh/max** | the one test that has to actually catch a real correctness bug |
| H — Secondary feature polish | Sonnet 5 | high | mechanical |
| I — Native-app foundations | Sonnet 5 | high | not scheduled; revisit when actioned |
| J — Chinese pipeline & curriculum ETL | Sonnet 5 | high | mechanical, well-scoped |
| K — SRS data layer / schema | **Opus 4.8** | high | schema decisions ripple through L/M |
| L — Adaptive daily queue engine | **Opus 4.8** | **xhigh/max** | FSRS math — a subtle bug here is hard to notice and wrong for a long time |
| M — Gamified quiz engine | **Opus 4.8** | **xhigh/max** for the isolation-constraint logic; Sonnet 5 / high for exercise-type UI | isolation guarantee is the one correctness property explicitly required by spec |

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

### G+. SRS & quiz-specific tests (added for Milestones J–M, Chinese/HSK feature set)
- [ ] Unit: FSRS interval/state calculations tested against the `rs-fsrs`/`fsrs-rs` crate's own published reference cases, plus edge cases (first review, a lapse/"Again" rating resetting state)
- [ ] Unit: queue-builder tests — given fixture due/new/curriculum data and a quota, assert exact composition and ordering (due-before-new, strict HSK curriculum order preserved, quota=0 → reviews-only with no special-cased code path)
- [ ] **Integration — the leak test (explicitly required):** seed `hsk_curriculum` with more items than exist in the user's `captures`/`srs_state`, generate N instances of every exercise type, assert zero distractor option or sentence-builder tile ever references a hanzi outside `LearnedVocabularyPool` (see Milestone M)
- [ ] Integration: daily-onboarding-prompt-once-per-day behavior across simulated date boundaries (use an injectable clock in tests, not real wall-clock time)
- [ ] Integration: Chinese clipboard capture end-to-end — capture hanzi → language-routed to the Chinese provider set (pinyin generated, HSK level looked up if present, English-only rarity step skipped) → card persisted with `script_metadata` populated

---

## Milestone H — Secondary Feature Polish
- [ ] Multi-word/phrase capture: relax `analyze_rarity`'s space-containing-string bailout; decide fallback behavior for phrases (skip rarity check, still translate)
- [ ] Configurable Anki note-type field mapping instead of the hardcoded `AppleStyleFinalV3` assumption — deck/model selects already exist in the UI, field mapping doesn't
- [ ] i18n: `ui_lang` is persisted but nothing branches on it — introduce `vue-i18n`, extract the currently hardcoded English strings from `App.vue`
- [ ] Per-language hotkey profiles (capture directly into a chosen language pair without changing global settings each time)

---

## Milestone I — Advanced / Native-App Foundations
*Long-term north star. Not scheduled — listed so the shared-core decisions in Milestone B stay intentional rather than accidental.*
*Note: Milestones L and M below pull the SRS-scheduler and gamified-quiz items of this milestone forward, scoped concretely to the Chinese/HSK use case. Treat those two milestones as the first real installment of what this milestone describes, not a separate track.*

- [ ] Extract `src-tauri/src/core/` into an actual Cargo workspace member (`crates/vocab-core`) once a second consumer exists
- [ ] Second Tauri app (`apps/learn/`) consuming `vocab-core` — own review-session UI, FSRS-based scheduler instead of relying on Anki's
- [ ] `ArgosTranslateProvider` — fully offline translation (implements the same `TranslationProvider` trait), likely via bundled ONNX-exported Argos models rather than shelling out to Python
- [ ] FSRS scheduler integration operating over the local SQLite store from Milestone F
- [ ] Duolingo-style mechanics: XP tally, streak + streak-freeze, tied to completed review sessions (not raw captures)
- [ ] Babbel-style contextual lesson cards built from the user's own captured `example_source`/`example_target` pairs, not a generic curriculum
- [ ] Anki export/import (`.apkg` read/write or AnkiConnect bulk sync) so the native app never locks users out of their existing Anki collection
- [ ] Mobile targets via Tauri 2's mobile support (iOS/Android) for the learning app specifically — the automator stays desktop-only since global-hotkey capture is inherently a desktop concept

---

## Milestone J — Chinese (HSK 1–6) Language Pipeline
*Extends Milestones B & C. Unlike L/M below, this slots into the existing clipboard-automator surface — no new app window required.*

### J1. Data model
- [ ] Add `ScriptMetadata` enum to `core/model.rs` per decision 7: `enum ScriptMetadata { None, Chinese { pinyin: String, hsk_level: Option<u8>, script_variant: ScriptVariant } }`, `enum ScriptVariant { Simplified, Traditional }`
- [ ] Add `pub script_metadata: ScriptMetadata` field to `Card`
- [ ] Default `script_variant` to `Simplified` — HSK is a Simplified-based standard; Traditional support is a stretch toggle (via the `opencc-rs`/OpenCC bindings), not MVP

### J2. Bundled HSK curriculum dataset — Official New HSK 3.0 (Levels 1–9)
*Per explicit decision: use the official HSK 3.0 standard (all 9 levels), not the user's own apkg deck (which only covered HSK 1–4 with no explicit level tagging). Verified by fetching the actual data, not just the repo description.*

**Chosen source: [`drkameleon/complete-hsk-vocabulary`](https://github.com/drkameleon/complete-hsk-vocabulary)** — MIT license (confirmed via GitHub API), actively maintained (auto-updated via GitHub Actions).
- The `level` field tags every entry against both HSK 2.0 (`old-N`) and the official 2021 HSK 3.0 standard (`new-N`) — `new-N` *is* "Official New HSK 3.0, Level N," so there's no ambiguity to resolve
- Pre-split per-level files already exist: `wordlists/exclusive/new/{1-9}.json` (just that level's new words) and `wordlists/inclusive/new/{1-9}.json` (cumulative through that level) — no manual filtering of the complete list needed
- Per entry: `simplified`, `radical`, `level[]`, `frequency`, `pos[]`, and per-form `traditional`, `transcriptions` (pinyin + numeric + wadegiles + bopomofo + romatzyh), `meanings[]` (English), `classifiers[]`
- Considered and rejected: [`ivankra/hsk30`](https://github.com/ivankra/hsk30) (MIT, but no inline English meanings — would need a separate CC-CEDICT cross-reference step); [`krmanik/HSK-3.0`](https://github.com/krmanik/HSK-3.0) (explicitly separates 2021 vs. 2025 revisions into folders, useful signal, but license is `NOASSERTION` — unclear terms)

**Two gaps this source has, both resolved:**
- [x] **No example sentences** in the official dataset. **Decision: reuse the apkg's ~1,431 sentences for words that overlap; leave the remaining ~9,600 words without an example sentence for now** (a documented gap to carry forward, not a blocker — can be backfilled later from another source or generation step if it matters in practice).
- [ ] **Multiple `forms[]` per entry** — a handful of words have more than one pronunciation/meaning form. The flat `hsk_curriculum` schema takes the first form for MVP simplicity, discarding secondary forms. Minor fidelity loss, noted for awareness, not blocking.

**Apkg audio/image reuse — confirmed.** The user's `Chinese Vocabulary (HSK 1-4).apkg` has genuinely useful data the official dataset lacks entirely: 100% pre-recorded native audio and 48% images for its 1,431 words. After importing the official HSK 3.0 dataset as the canonical curriculum, do a secondary pass matching apkg `Chinese` values against `hsk_curriculum.hanzi` and attaching the apkg's `vocab_audio_path`/`sentence_audio_path`/`image_path` (and its `Sentence`/`Sentence Pinyin`/`English Sentence` for the example-sentence gap above) where they match. Falls back to Milestone J4's live TTS / blank example for everything else.

### J2b. Import pipeline
- [ ] Write a one-time ETL script (Rust or a throwaway script, doesn't need to ship with the app) that downloads/vendors the relevant `wordlists/exclusive/new/{1-9}.json` files, flattens each entry's first `forms[]` element, and writes the result into the `hsk_curriculum` schema below
- [ ] **No German meaning exists in the source data at all — only English.** `meaning_de` has to be backfilled by running Milestone C's translation provider (English → German) over all ~11,000 entries at import time, not left to live captures alone
- [ ] `curriculum_order` = sort by `level` ascending, then `frequency` ascending within a level (the dataset's own frequency field is a more principled tiebreaker than anything derivable from the apkg's note-creation order)
- [ ] Apkg audio/image/sentence reuse: import as a second pass after the primary HSK 3.0 import, matching on `hanzi`

### J3. Pinyin generation (for captures outside the bundled curriculum)
- [ ] Add a Rust pinyin crate — [`pinyin`](https://crates.io/crates/pinyin) (385k downloads, established) or [`mandarin-to-pinyin`](https://crates.io/crates/mandarin-to-pinyin) (PHF-based lookup, no external files) — evaluate both against a handful of tone-mark edge cases before picking one
- [ ] Wire pinyin generation into the capture pipeline for any captured Hanzi not already present in `hsk_curriculum`

### J4. Language-aware provider routing (extends Milestone B's provider traits)
- [ ] Add a `ProviderRegistry` keyed by language code, resolved once per capture before calling translate/example/rarity — Datamuse (`RarityProvider`) and dictionaryapi.dev (`ExampleProvider`) are **English-only** and must not be called for Chinese captures
- [ ] Detect Chinese input either from the active `source_lang` config or by sniffing the captured text for CJK Unified Ideographs (U+4E00–U+9FFF), so routing is correct even if config defaults lag
- [ ] Chinese rarity/synonym step: **no-op by design** — HSK level already *is* the frequency/difficulty signal for curriculum vocabulary, so there's no need to duplicate what `hsk_curriculum.hsk_level` already encodes for in-curriculum captures
- [ ] Chinese example sentences: prefer the bundled `hsk_curriculum` example sentence when the captured hanzi matches a curriculum entry; fall back to the existing generic-fallback-string pattern (`fetch_example`'s current behavior) for out-of-curriculum captures
- [ ] Confirm the existing Google-Translate-TTS provider accepts `zh-CN` as a language code (it's already parametrized by `source_lang`) — reuse rather than build a new TTS path

### J5. Anki export mapping
- [ ] Extend `anki/mapper.rs` to map `character`/`pinyin`/`hsk_level` into whatever fields the user's chosen Anki note type expects — flag in the UI that a Chinese-compatible note type (Hanzi/Pinyin/Meaning/Audio fields) is a setup requirement, same pattern as the existing deck/model discovery UI

---

## Milestone K — Local Curriculum & SRS Data Layer
*Extends Milestone F's SQLite store with the schema needed by L and M.*

```sql
-- extends captures(...) from Milestone F with two new nullable columns:
ALTER TABLE captures ADD COLUMN pinyin TEXT;
ALTER TABLE captures ADD COLUMN hsk_level INTEGER;

CREATE TABLE srs_state (
  card_id           INTEGER PRIMARY KEY REFERENCES captures(id) ON DELETE CASCADE,
  algorithm         TEXT NOT NULL DEFAULT 'fsrs',
  stability         REAL,               -- FSRS memory-state parameter
  difficulty        REAL,               -- FSRS memory-state parameter
  interval_days     INTEGER NOT NULL DEFAULT 0,
  repetitions       INTEGER NOT NULL DEFAULT 0,
  due_date          TEXT NOT NULL,
  last_reviewed_at  TEXT,
  last_rating       INTEGER             -- 1=Again, 2=Hard, 3=Good, 4=Easy
);

CREATE TABLE hsk_curriculum (
  hanzi                    TEXT PRIMARY KEY,       -- 'simplified' from the source dataset
  traditional               TEXT,
  hsk_level                INTEGER NOT NULL,       -- parsed from level: "new-N" → N (official HSK 3.0, 1-9)
  frequency                 INTEGER,                -- from the source dataset; used as the within-level curriculum_order tiebreaker
  curriculum_order         INTEGER NOT NULL,       -- derived: hsk_level asc, frequency asc — see J2b
  pinyin                   TEXT NOT NULL,
  pos                        TEXT,                   -- part of speech, from source
  meaning_en                TEXT,
  meaning_de                TEXT,                    -- backfilled via Milestone C's provider at import time, not present in source data
  cloze_sentence            TEXT,                    -- NULL unless backfilled — see J2's example-sentence gap
  example_sentence_source  TEXT,
  example_sentence_pinyin   TEXT,
  example_sentence_target  TEXT,                    -- German backfilled same as meaning_de, once a sentence source is chosen
  vocab_audio_path         TEXT,                    -- NULL unless matched via the optional apkg-reuse enrichment; falls back to live TTS otherwise
  sentence_audio_path      TEXT,
  image_path                TEXT                     -- NULL unless matched via the optional apkg-reuse enrichment
);

CREATE TABLE daily_sessions (
  session_date          TEXT PRIMARY KEY,   -- local date, YYYY-MM-DD
  requested_new_count   INTEGER NOT NULL,
  reviews_due_count     INTEGER NOT NULL,
  reviews_completed     INTEGER NOT NULL DEFAULT 0,
  new_items_completed   INTEGER NOT NULL DEFAULT 0,
  completed_at          TEXT
);
```

- [ ] `srs_state` is a separate 1:1 table from `captures`, not inline columns — keeps the scheduling algorithm swappable/migratable without touching card content
- [ ] `hsk_curriculum` is reference data (populated once from Milestone J2's dataset), distinct from the user's personal `captures` — "learned" is defined as *has a row in `srs_state`*, never membership in `hsk_curriculum` alone
- [ ] Migration tooling: add `rusqlite_migration` (or hand-rolled versioned `.sql` files) since this is now a real multi-table schema, not the single-file config store Milestone F started with

---

## Milestone L — Adaptive Daily Queue Engine
*Builds on Milestone K. This is the first concrete installment of Milestone I's "own SRS" ambition — scoped to Chinese/HSK.*

### L1. Scheduling algorithm
- [ ] Evaluate [`rs-fsrs`](https://crates.io/crates/rs-fsrs) (lighter, scheduler-only) vs. [`fsrs-rs`](https://github.com/open-spaced-repetition/fsrs-rs) (the official Open Spaced Repetition crate, includes training/simulation via the Burn ML framework — heavier dependency). **Recommendation: start with `rs-fsrs`** for MVP scheduling; revisit `fsrs-rs` only if per-user weight optimization becomes a real feature request
- [ ] Implement the review-rating → next-state transition (`Again`/`Hard`/`Good`/`Easy` → new `stability`/`difficulty`/`due_date`) writing into `srs_state`

### L2. Daily onboarding prompt
- [ ] Persist `last_prompted_date` (new `AppConfig` field or its own small state row)
- [ ] On main-window mount, compare `last_prompted_date` to today's local date; if different or unset, show a modal — *"How many new characters do you want to learn today?"* — before the normal UI renders
- [ ] **Decide and document the dismiss-without-answering behavior** (proposed default: treat as quota = 0 / reviews-only for that day, but let the user reopen the prompt later the same day from a menu action) — this is a UX detail that needs an explicit decision, not a silent assumption
- [ ] On submit, write a `daily_sessions` row and update `last_prompted_date`

### L3. Dynamic queue builder
```rust
fn build_daily_queue(new_quota: u32) -> Vec<QueueItem> {
    let mut queue: Vec<QueueItem> = srs_state
        .where(due_date <= today)
        .order_by(due_date, Asc)
        .map(QueueItem::Review)
        .collect();

    if new_quota > 0 {
        let learned_hanzi = captures.where(script_kind = "chinese").select(front);
        queue.extend(
            hsk_curriculum
                .where(hanzi NOT IN learned_hanzi)
                .order_by(curriculum_order, Asc)
                .limit(new_quota)
                .map(QueueItem::New),
        );
    }
    queue // new_quota == 0 falls out of this naturally as "reviews only" — no separate low-time-mode branch needed
}
```
- [ ] Priority 1 (mandatory): all `srs_state` rows with `due_date <= today`, ordered by `due_date` ascending
- [ ] Priority 2 (optional): if `new_quota > 0`, append the next N unlearned `hsk_curriculum` rows in strict `curriculum_order`
- [ ] Low-time mode (`new_quota = 0`) is just the zero-length branch of the same function, not a separate code path — avoid over-engineering a distinct "mode"
- [ ] New Tauri commands: `get_daily_queue`, `submit_review_rating(card_id, rating)`

---

## Milestone M — Gamified Quiz Engine (Duolingo/Babbel-style)
*Builds on Milestone L's queue. Needs its own UI surface — see the open decision below.*

### M1. Vocabulary isolation (the hard constraint)
- [ ] Define **one** canonical `LearnedVocabularyPool` query: every `captures` row that has a corresponding `srs_state` row (i.e., has been introduced via the daily queue at least once) — every exercise type below must draw distractors and sentence pools from this single query, not ad hoc per-exercise lookups, so the isolation guarantee is centralized and testable as one thing (see Milestone G+'s leak test)
- [ ] `hsk_curriculum` rows with no matching `srs_state` entry must never appear in quiz content, only in the queue builder's "next new items" selection

### M2. Exercise types
- [ ] **Multiple Choice**: character ↔ pinyin/meaning (German or English), correct answer plus N-1 distractors drawn from `LearnedVocabularyPool` excluding the correct answer. MVP: uniform-random distractor selection. Stretch: weight distractors toward same-HSK-level or pinyin-similar items for a real difficulty curve
- [ ] **Sentence Builder (word bank)**: tokenize a sentence into tiles — prefer splitting on known-vocabulary word boundaries from `hsk_curriculum`/`captures`, falling back to per-character tiles for anything not recognized as a known unit; shuffle tile order; drag-and-drop or tap-to-place UI; check ordering on submit
- [ ] **Audio Listening Comprehension**: reuse the TTS audio already generated for the card (Milestone J4); two variants — (a) play audio → multiple-choice identify the character, (b) play audio → sentence-builder using the heard sentence's tiles

### M3. UI surface — open decision
- [ ] **Recommendation:** build the quiz/review session as a new view/window (`src/views/PracticeSession.vue`) inside the *existing* Tauri app, not as Milestone I's separate second app. Keep all scheduling/quiz logic in the shared `core` module (per Milestone B) so extracting it into a standalone app later is a packaging change, not a rewrite. This avoids standing up a second app+workspace before there's any usage validating the feature — **flagging for confirmation before building, since it's a real fork in strategy** (single-app-for-now vs. start the second app now)

---

