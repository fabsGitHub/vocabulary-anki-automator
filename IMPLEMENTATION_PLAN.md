# Implementation Plan — vocabulary-anki-automator

Status (2026-10-31): Milestones B–N are implemented and tested on branch `claude/implement-open-todos`, including the Learn app (desktop + web). A final five-part audit (security, IPC contracts, core correctness, plan completeness, release builds) then fixed what it found, with a regression test wherever one can reach the fix; see "Final audit" at the end for that and for what it left open. Unchecked items say why they are open: an operator step (accounts, certificates, keys, global tool installs, a manual test with real Anki), an unapproved dependency, or a platform limit. This is the master TODO list for the app and the development workspace, derived from the architecture/vision session on 2026-07-27; `docs/architecture.md` records how it was built and every deviation.

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
10. **Target platforms, decided 2026-07-28 (resolves Milestone M3's open fork):** the product is two apps sharing one Rust core, not one app trying to be everything.
    - **`Automator`** (this existing app) stays **desktop-only (macOS, Windows, Linux) permanently** — not a phase to grow out of. Global-hotkey capture is an OS-level desktop capability; there is no iOS/Android/Web equivalent, so the automator was never a candidate for those targets regardless of timeline.
    - **`Learn`** (`apps/learn/`, Milestone I's "second Tauri app," moved up from "not scheduled") is the one that reaches **all six targets**: macOS/Windows/Linux/iOS/Android natively via Tauri 2, plus a standalone **web build of the same frontend** for the Web target. The web build requires every Tauri-specific call (`invoke`, `listen`, native storage) to sit behind a platform-adapter interface so a browser build can substitute browser-native equivalents (IndexedDB, etc.) instead of the Rust/SQLite backend — a concrete design task for whoever starts Milestone I's UI work, not fully specified here.
    - `crates/vocab-core` (decision 6) gets extracted as a real Cargo workspace member **as part of Milestone B**, not deferred "once a second consumer exists" — the second consumer (`Learn`) is starting immediately, not hypothetically, now that cross-platform reach is a stated requirement rather than a someday-maybe.
11. **Release/distribution infrastructure (Milestone N, added 2026-07-29) targets `Automator`'s three desktop platforms first; `Learn`'s mobile/web distribution is designed in the same milestone for planning purposes but built as its own later milestone once Learn's UI work actually begins (Milestone I).** This isn't optional infrastructure to defer indefinitely — as of this decision, the pipeline had never defined what happens _after_ an operator's Tier 2 merge approval. Nothing currently gets built, signed, or reaches anyone. Sequenced immediately after Milestone B: B's crate restructuring (`crates/vocab-core`) is what a real build/release pipeline needs to target, so the packaging work waits for B, but the account/certificate/key setup in N has no code dependency and starts in parallel now.

---

## Development Approach — Model & Effort Guidance

_For reference when actually implementing each milestone. ("Ultracode" was checked against this machine's config and isn't a real Claude Code tier — this table just records the model/effort assignment agreed on 2026-07-27.)_

Default unless a row below says otherwise: **Sonnet 5 at `high` effort.** Escalate to **Opus 4.8**, and to **`xhigh`/`max`** effort specifically, whenever the work touches one of four things: the provider-trait/`Card`-model boundary (Milestone B), the FSRS scheduler (Milestone L), the vocabulary-isolation guarantee in the quiz engine (Milestone M), or the auto-install/consent security surface (Milestone E).

| Milestone                               | Model                   | Effort                                                       | Subagent (`.claude/agents/`)                                                   | Why                                                                                                                                                                                                                                          |
| --------------------------------------- | ----------------------- | ------------------------------------------------------------ | ------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A — Workspace & dev tooling             | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | mechanical setup                                                                                                                                                                                                                             |
| B — Core architecture refactor          | **Opus 4.8**            | **high**                                                     | `milestone-opus`                                                               | foundational boundary everything else builds on                                                                                                                                                                                              |
| C — Translation provider swap           | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | mechanical, well-scoped                                                                                                                                                                                                                      |
| D — Approval window UI                  | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | UI wiring                                                                                                                                                                                                                                    |
| E — Consent & supply-chain hardening    | **Opus 4.8**            | **xhigh/max**                                                | `milestone-opus-critical`                                                      | security-sensitive (auto-install, unpinned script download)                                                                                                                                                                                  |
| F — Local history store                 | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | mechanical CRUD                                                                                                                                                                                                                              |
| G — Testing infrastructure (general)    | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | scaffolding                                                                                                                                                                                                                                  |
| G+ — SRS/quiz leak test specifically    | **Opus 4.8**            | **xhigh/max**                                                | `milestone-opus-critical`                                                      | the one test that has to actually catch a real correctness bug                                                                                                                                                                               |
| H — Secondary feature polish            | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | mechanical                                                                                                                                                                                                                                   |
| I — Native-app foundations (`Learn`)    | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | scheduled alongside B per decision 10 (2026-07-28) — no longer deferred                                                                                                                                                                      |
| J — Chinese pipeline & curriculum ETL   | Sonnet 5                | high                                                         | `milestone-sonnet`                                                             | mechanical, well-scoped                                                                                                                                                                                                                      |
| K — SRS data layer / schema             | **Opus 4.8**            | high                                                         | `milestone-opus`                                                               | schema decisions ripple through L/M                                                                                                                                                                                                          |
| L — Adaptive daily queue engine         | **Opus 4.8**            | **xhigh/max**                                                | `milestone-opus-critical`                                                      | FSRS math — a subtle bug here is hard to notice and wrong for a long time                                                                                                                                                                    |
| M — Gamified quiz engine                | **Opus 4.8** / Sonnet 5 | **xhigh/max** for isolation logic; high for exercise-type UI | `milestone-opus-critical` (isolation logic) / `milestone-sonnet` (exercise UI) | isolation guarantee is the one correctness property explicitly required by spec                                                                                                                                                              |
| N — Release, distribution & auto-update | **Opus 4.8**            | **xhigh/max**                                                | `milestone-opus-critical`                                                      | signing-key custody and updater-trust-chain mistakes are the highest-blast-radius category of bug this company can ship — a compromised updater pushes malicious code to every user automatically, worse than any single milestone's own bug |

Subagent definitions live in `.claude/agents/` in this repo. They set **model** (the one lever Claude Code actually exposes per-subagent); the "effort" column above is approximated through each subagent's own instructions (verification rigor, edge-case coverage) rather than a literal API parameter — Claude Code CLI doesn't expose `output_config.effort` as a user-facing dial, so `milestone-opus-critical`'s system prompt spells out concretely what "xhigh/max" should mean in practice for each of its four milestones instead of just naming a setting.

---

## Milestone A — Workspace & Dev Tooling Setup

_Ungated — can start immediately, no app-code dependency._

### A1. macOS environment (verified against dev machine on 2026-07-27)

- [ ] `cargo install cargo-watch` — scripted in `scripts/bootstrap-macos.sh` (idempotent); run it on the actual macOS dev machine — no `cargo`/Homebrew toolchain exists in the Linux sandbox this milestone was implemented in — **Operator step:** run `scripts/bootstrap-macos.sh` on the dev machine (global `cargo install` is Tier 2: the Tier-2 hook blocks a direct `cargo install`, and running the bootstrap, which installs tools globally, is an operator step by rule, not by enforcement).
- [ ] `cargo install tauri-driver` — scripted in `scripts/bootstrap-macos.sh`; same caveat as above — **Won't do — not applicable on macOS (2026-10-31):** tauri-driver has no WKWebView backend, so it cannot drive this app on a Mac; the bootstrap no longer installs it. E2E belongs on a Linux/Windows runner (Milestone G).
- [x] `brew install gh` — scripted in `scripts/bootstrap-macos.sh`; same caveat as above — already present on the dev machine (`/opt/homebrew/bin/gh`, checked 2026-10-31); the bootstrap keeps the idempotent check.
- [x] `npm install -D vitest @vue/test-utils jsdom` — added to `package.json` devDependencies (plus `eslint`/`prettier` tooling for A2); installed and verified in `node_modules`
- [ ] `safaridriver --enable` — scripted in `scripts/bootstrap-macos.sh`; macOS-only, cannot be run from this sandbox — **Won't do — not applicable on macOS:** safaridriver cannot automate WKWebView apps either (see tauri-driver above); removed from the bootstrap.
- [x] Standardize on a single file watcher for the cross-tree (Rust + Vue) watch loop: **decision — Watchman** (see `scripts/bootstrap-macos.sh`); `just watch` otherwise relies on `cargo-watch` for the Rust side and Vite's own HMR for the Vue side

### A2. Repo scaffolding

- [x] Add `justfile` (or root `package.json` scripts) with: `dev`, `test:unit`, `test:integration`, `test:e2e`, `watch`, `lint`
- [x] Add `rustfmt.toml`
- [x] Add `.eslintrc`/`eslint.config.js` + Prettier config for the Vue/TS side (no lint config exists today)
- [x] Add a pre-commit hook (`husky` or a plain `.git/hooks/pre-commit` script) running: `cargo fmt --check`, `cargo clippy -- -D warnings`, `vue-tsc --noEmit` — `scripts/hooks/pre-commit`, which `scripts/bootstrap-macos.sh` copies into git's hooks dir (since the final audit also from a linked worktree, where `.git` is a file). A clone has no hook until the bootstrap runs.
- [x] Create `scripts/bootstrap-macos.sh` — installs everything in A1 idempotently, checks versions, exits non-zero on failure, no interactive prompts
- [x] Create `scripts/run-all-checks.sh` — chains the unit + integration test tiers; used by both the watch loop and any future CI
- **Resolved 2026-10-31:** on the real macOS dev machine `vite build` and `vitest run` work with the plain configs; the read-after-write race was specific to that Linux sandbox. Original note: **Sandbox caveat (self-tested 2026-07-29):** in this Linux sandbox, both `npm run build` (`vite build`) and `npm run test:unit` (`vitest run`) fail with `ENOENT` reading their own just-written temp config bundle (`vite.config.ts.timestamp-*.mjs` / `vitest.config.ts.timestamp-*.mjs`) — a read-after-write race on this sandbox's host-mounted filesystem, reproducible against the pre-existing, untouched `vite.config.ts` too, so it predates this milestone and isn't caused by anything added here. Confirmed workaround for the plain Vite CLI: `vite build --configLoader runner` succeeds. Not applied to `package.json`'s `dev`/`build`/`preview` scripts since editing those is outside this milestone's file list; `vitest@2.x`'s CLI doesn't expose an equivalent flag at all, and bumping vitest's major version to chase a sandbox-only bug is its own can of worms, out of scope here. Expected to be a non-issue on the real macOS dev machine's local disk — re-verify there before relying on this note.
- **Resolved 2026-10-31:** a plain `npm install` on the macOS dev machine regenerated `package-lock.json` (committed), so `npm ci` works; the bootstrap now uses `npm ci`. Original note: **Sandbox caveat (self-tested 2026-07-30):** the same read-after-write race hits plain `npm install` / `npm install --package-lock-only` in this sandbox too — npm truncates `package-lock.json` to 0 bytes while regenerating it, then fails with `ENOENT` reading it back, reproduced twice. Net effect: `package-lock.json` (committed at the first-commit revision) is **not yet in sync** with the `vitest`/`eslint`/`prettier`/`@vue/test-utils`/`jsdom` etc. devDependencies this milestone added to `package.json` — `node_modules` has them (installed successfully before this race was understood) but a fresh `npm ci` against the committed lockfile would not. Left uncommitted/unresolved rather than risking a corrupted lockfile; whoever runs `scripts/bootstrap-macos.sh` on the real macOS dev machine should run a plain `npm install` there first to regenerate `package-lock.json` correctly, then commit it.

### A3. Token-efficiency / assistant-workflow setup

- [ ] Install RTK (`brew install rtk`) and adopt it for wrapping `cargo build` / `cargo tauri build` / `npm run build` output during local dev sessions — scripted in `scripts/bootstrap-macos.sh`; requires Homebrew on the real dev machine — **Operator step:** `brew install rtk` (still scripted in the bootstrap).
- [x] Write `CLAUDE.md` using the tiered-doc convention (only essential context loads by default; deeper notes live in linked files loaded on demand) — do this once real docs exist, not before (still not before: no app-code docs exist yet) — tiered `CLAUDE.md` written 2026-10-31: essentials inline, deeper docs linked (`docs/architecture.md`, this plan, test-harness, signing custody, data README).
- [ ] Deferred (tracked so it isn't forgotten, not actioned yet): CodeGraph, Token Optimizer MCP — revisit once the file count outgrows a single Explore pass — still deferred, as planned.

### A4. Test harness prerequisites

- [x] Design a disposable Anki test profile strategy: either a temp `ANKI_BASE` directory the E2E harness points AnkiConnect at, or a lightweight mock AnkiConnect server (`wiremock`-based Rust binary or small Node stub) that records/asserts `addNote` payloads without needing real Anki — decision recorded in `docs/test-harness-strategy.md` (mock AnkiConnect stub, chosen over a real disposable Anki profile); actual stub implementation is Milestone G's job
- [x] **Hard rule:** `download_and_install_anki` / `auto_provision_ankiconnect` must never fire inside any automated test or watch-loop run — only against a developer's real Anki install after explicit consent (see Milestone E) — encoded in `docs/test-harness-strategy.md` and called out in `scripts/bootstrap-macos.sh`'s header comment

---

## Milestone B — Core Architecture Refactor

_Restructures `src-tauri/src` so Anki-specific assumptions stop leaking into capture/enrichment logic. Prerequisite for C, D, and F._
_Per decision 10: this milestone now also stands up `crates/vocab-core` as a real Cargo workspace member (not just an in-tree `src-tauri/src/core/` module) — `Learn` (Milestone I) consumes it as a second, real workspace member from the start, not "once a second consumer exists."_

### B1. Module layout

- [x] Create `crates/vocab-core/` as a Cargo workspace member; `src-tauri` becomes one consumer of it, not the sole home of this logic — root `Cargo.toml` workspace with `src-tauri`, `apps/learn/src-tauri`, `crates/vocab-core`, `crates/anki-bridge`, `tools/hsk-import`.
- [x] Create `crates/vocab-core/src/model.rs` — the neutral `Card` struct: — `crates/vocab-core/src/model.rs` (plus `script_metadata` per J1 and base64 audio for IPC).
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
- [x] Create `crates/vocab-core/src/providers/mod.rs` with trait definitions: — `crates/vocab-core/src/providers/mod.rs` (object-safe via `async_trait`; plus `ScriptAnnotator`, `CurriculumLookup`).
  - `trait TranslationProvider { async fn translate(&self, text: &str, source: &str, target: &str) -> Result<String, ProviderError>; }`
  - `trait ExampleProvider { async fn fetch_example(&self, word: &str) -> Result<String, ProviderError>; }`
  - `trait RarityProvider { async fn analyze(&self, word: &str) -> Result<RarityResult, ProviderError>; }`
  - `trait TtsProvider { async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError>; }`
- [x] Create `crates/vocab-core/src/providers/mymemory.rs` — implements `TranslationProvider` against MyMemory's REST API — `providers/mymemory.rs`.
- [x] Move existing Datamuse logic out of `src-tauri/src/api_client.rs` into `crates/vocab-core/src/providers/datamuse.rs` implementing `RarityProvider` — `providers/datamuse.rs`.
- [x] Move existing dictionaryapi.dev logic into `crates/vocab-core/src/providers/dictionary_api.rs` implementing `ExampleProvider` — `providers/dictionary_api.rs`.
- [x] Move existing Google-Translate-TTS logic into `crates/vocab-core/src/providers/google_tts.rs` implementing `TtsProvider` (kept only as the TTS source — not as the translation provider) — `providers/google_tts.rs`.
- [x] Delete `src-tauri/src/api_client.rs` once its contents are fully redistributed into the provider modules above — deleted together with `src-tauri/src/anki.rs`.
- [x] Create `crates/vocab-core/src/pipeline.rs` — orchestrates capture → translate → rarity-check → example → tts → `Card`, replacing the inline logic currently in `trigger_clipboard_sequence` (`lib.rs:206-310`). `src-tauri` calls into this; the Anki-specific mapping stays in `src-tauri/src/anki/mapper.rs` below since that boundary is automator-specific, not shared core (decision 4). — `crates/vocab-core/src/pipeline.rs`; the hotkey flow in `src-tauri/src/capture.rs` calls it.
- [x] Create `src-tauri/src/anki/mapper.rs` — the _only_ place that converts a `Card` into Anki's `Front`/`Back`/`Context Sentence`/`Notes` field names; `anki.rs`'s `add_vocab_card` takes a `Card`, not raw strings — lives in `crates/anki-bridge/src/mapper.rs` so Learn's Anki export reuses it (`docs/architecture.md` § 12 item 1).

### B2. Config cleanup

- [x] Remove `DeepLConfig` struct entirely from `config.rs`
- [x] Remove `api_key` field and all DeepL references from `AppConfig`
- [x] Rename the config section (currently `deepl`) to provider-neutral naming, e.g. `translation: TranslationConfig { source_lang, target_lang, max_chars, provider: String, contact_email: Option<String> }` — `translation: TranslationConfig { source_lang, target_lang, max_chars, provider, contact_email }`; legacy `deepl` files still load (`#[serde(alias)]`, tested with a fixture).
- [x] Grep the whole repo for `deepl`/`DeepL` (check `tauri.conf.json` product name, `package.json` `name`, window titles, `Cargo.toml` description) and remove/rename every remaining reference — only the legacy-config alias and its test fixture mention it; crate, lib and npm package renamed to `vocab-automator`.
- [x] Update `Cargo.toml` package `description` (currently references vocabulary automation only — confirm no stray DeepL wording elsewhere) — no provider wording in any `Cargo.toml`.

### B3. Wire `execute_card_generation` through the new pipeline

- [x] Update `execute_card_generation` (`lib.rs:172-204`) to take a `Card` (from the pending-payload state) instead of four raw strings
- [x] Update `PendingPayload` state to hold a typed `Card` instead of `serde_json::Value` — `Option<PendingCapture { card, warnings, profile_id }>`.

---

## Milestone C — Translation Provider Swap (MyMemory)

- [x] Implement `MyMemoryProvider`: GET `https://api.mymemory.translated.net/get?q=...&langpair=SRC|TGT`, parse `responseData.translatedText`
- [x] Handle MyMemory's quota-exceeded response distinctly from network failure or malformed response — current `translate()` swallows all failure modes into an empty string; replace with typed `ProviderError` variants surfaced to the UI — typed `ProviderError` (`quota_exceeded`, `network`, `http`, `malformed`, ...) reaches the approval window as pipeline warnings.
- [x] Add an optional contact-email field to the translation config so users can self-raise their MyMemory daily quota (per MyMemory's documented mechanism) — config field + settings UI; sent as MyMemory's `de=` parameter.
- [x] Update `App.vue`'s config form: confirm no stray DeepL-labeled fields remain; source/target language selects stay as-is since they're provider-agnostic — the settings UI is now `src/views/*` (App.vue is mounted); no DeepL-labelled fields.

---

## Milestone D — MVP Completion: Approval Window

_This is the actual "does the app work end-to-end" gap identified during architecture review — the backend already emits events and exposes commands for this window; the frontend has no UI for it yet._

- [x] Create `src/views/ApprovalWindow.vue` (or a top-level branch in `App.vue` keyed on `windowLabel === 'preview'`) — `src/views/ApprovalWindow.vue`, selected by window label in `App.vue`.
- [x] Listen for the `translation-payload` Tauri event (`listen()` from `@tauri-apps/api/event`) in the preview window
- [x] On mount, also call the existing `get_pending_payload` command to cover the case where the window opens after the event already fired
- [x] Build the approval UI: editable front/back/example/notes fields pre-filled from the payload, an "Approve" button calling `execute_card_generation`, a "Discard" button that closes/hides the window without saving
- [x] On approve success, hide the `preview` window (currently built with `always_on_top(true)` and never explicitly hidden after use)
- [x] Add a capabilities entry for the `preview` window in `src-tauri/capabilities/default.json` (currently scoped only to `"windows": ["main"]` — audit whether the preview window silently lacks a permission it needs) — `src-tauri/capabilities/preview.json`; since the final audit it grants only `core:event:allow-listen`/`-unlisten` and `core:window:allow-hide` (`docs/architecture.md` § 8.3).
- [ ] Manual test: trigger hotkey → confirm payload appears in the preview window → approve → confirm two Anki notes are created with swapped front/back — **Operator step:** needs real Anki and macOS Accessibility permission for the synthetic Cmd+C. The same flow is covered by IPC tests against a mock AnkiConnect (`src-tauri/tests/ipc.rs`, `ipc_anki.rs`) and by Vitest (`ApprovalWindow`).

---

## Milestone E — Consent & Supply-Chain Hardening

- [x] Replace the unconditional auto-run of `download_and_install_anki` + `auto_provision_ankiconnect` in `setup()` (`lib.rs:344-350`) with a gated flow: check-only on startup, explicit user-triggered install (the UI buttons already exist in `App.vue` — `runSetupInstallation`/`bootAnkiApp` — this is mostly about _removing_ the silent auto-run on the Rust side) — startup only checks; installs run from consent-gated commands.
- [x] Pin the AnkiConnect script download (`lib.rs:133`) to a specific commit SHA of `FooSoft/anki-connect`, not `master` — final archived commit `4064fa1`, four files from `plugin/` (the old URL was a 404, so provisioning never worked).
- [x] Add checksum/hash verification after downloading the AnkiConnect script and the Anki installer, before writing/executing either — SHA-256 for every AnkiConnect file and the Anki 24.06.3 installers (from Anki's signed checksums file), verified before anything is written or executed.
- [x] Change the AnkiConnect config this app writes (`lib.rs:127`) from `"webBindAddress"/"webCorsOriginList": ["*"]` to the app's actual origin only — `tauri://localhost` / `http://tauri.localhost` (+ the dev server in debug builds), never `*`. — **Superseded (final audit):** neither frontend calls AnkiConnect any more (only the Rust backend does, without an `Origin` header), so the written config allows no web origin at all: `webCorsOriginList: []` (`docs/architecture.md` § 8.4).
- [x] Add a one-time consent dialog before the Anki DMG/EXE download+install path runs at all — consent dialog in the UI + `grant_consent`; both install commands refuse without it.

---

## Milestone F — Local History Store (secondary tier)

- [x] Add `rusqlite` (or `sqlx` with the `sqlite` feature) to `Cargo.toml` — `rusqlite` 0.40 with bundled SQLite (operator-approved 2026-10-31).
- [x] Create `src-tauri/src/store/mod.rs` with a `CardStore` trait and a SQLite-backed implementation — `crates/vocab-core/src/store` (shared with Learn; § 12 item 2).
- [x] Schema: `captures(id, front, back, example_source, example_target, notes, tags, source_lang, target_lang, captured_at, synced_to_anki BOOLEAN)` — v1 migration; v2/v3 add Milestones J/K/L/I columns and tables.
- [x] Persist every approved card to local SQLite in addition to pushing it to Anki — saved first, then pushed; Anki offline is not an error (`sync_pending_captures` retries).
- [x] New Tauri commands: `list_recent_captures`, `delete_capture` — plus `sync_pending_captures`.
- [x] Frontend: a simple history/review list view in the main window — `src/views/HistoryView.vue`.

---

## Milestone G — Testing Infrastructure

- [x] Unit (Rust): table-driven tests for each provider's response parsing (MyMemory JSON shape, Datamuse frequency-tag parsing) against canned fixtures, no network — pure `parse_*` functions over real + synthetic fixtures, and wiremock HTTP tests (`crates/vocab-core/tests/providers_http.rs`).
- [x] Unit (Rust): `config.rs` save/load round-trip test against a temp directory (requires separating the path-taking core from the `AppHandle`-coupled wrapper) — `load_config_from`/`save_config_to` tested in temp dirs, legacy fixture included.
- [x] Unit (TS): extract `useAnkiConnection` and `useConfig` composables out of `App.vue`; Vitest coverage of the online/offline/plugin-missing state machine — `src/composables/useAnkiConnection.ts`, `useConfig.ts` + Vitest.
- [x] Integration: mock AnkiConnect server asserting the exact `addNote` payload shape produced by `anki/mapper.rs` — `crates/anki-bridge/tests/mock_ankiconnect.rs` (exact request bodies) and `src-tauri/tests/ipc_anki.rs`.
- [x] Integration: `tauri::test::mock_builder` tests invoking `execute_card_generation` / `get_pending_payload` directly — `src-tauri/tests/ipc.rs`.
- [ ] E2E: `tauri-driver` + WebdriverIO script — launch the compiled app, emit the internal capture event (not a real OS keystroke), assert the preview window renders the payload, click approve, assert two notes hit the mocked AnkiConnect stub — **Not done:** needs WebdriverIO (npm) and tauri-driver (cargo install), neither approved, plus a Linux/Windows runner (no macOS backend). The internal capture event it would emit exists: the `capture_text` command.
- [x] Wire `test:unit`/`test:integration` into the pre-commit hook (fast tiers only); keep `test:e2e` at pre-push/nightly — `scripts/hooks/pre-commit` runs lint + `scripts/run-all-checks.sh`; CI runs the same script. No pre-push hook or nightly workflow exists, and `npm run test:e2e` is a placeholder that exits 0 (see the E2E item).

### G+. SRS & quiz-specific tests (added for Milestones J–M, Chinese/HSK feature set)

- [x] Unit: FSRS interval/state calculations tested against the `rs-fsrs`/`fsrs-rs` crate's own published reference cases, plus edge cases (first review, a lapse/"Again" rating resetting state) — `crates/vocab-core/tests/srs_fsrs.rs` replays all five rs-fsrs 1.2.1 reference cases through the store.
- [x] Unit: queue-builder tests — given fixture due/new/curriculum data and a quota, assert exact composition and ordering (due-before-new, strict HSK curriculum order preserved, quota=0 → reviews-only with no special-cased code path) — `tests/srs_queue.rs`.
- [x] **Integration — the leak test (explicitly required):** seed `hsk_curriculum` with more items than exist in the user's `captures`/`srs_state`, generate N instances of every exercise type, assert zero distractor option or sentence-builder tile ever references a hanzi outside `LearnedVocabularyPool` (see Milestone M) — `tests/quiz_isolation.rs`; mutation-checked (it fails when a generator leaks). The final audit found both leak tests (Rust and web) blind to multiple-choice answers that skip the learned-hanzi check, because their only unlearned-hanzi answer was alone in its language pair; a learned word added to both fixtures makes every isolation mutant tried fail them.
- [x] Integration: daily-onboarding-prompt-once-per-day behavior across simulated date boundaries (use an injectable clock in tests, not real wall-clock time) — `tests/srs_daily.rs` with `FixedClock` in non-UTC offsets.
- [x] Integration: Chinese clipboard capture end-to-end — capture hanzi → language-routed to the Chinese provider set (pinyin generated, HSK level looked up if present, English-only rarity step skipped) → card persisted with `script_metadata` populated — `tests/chinese_capture.rs` over the real bundled curriculum.

---

## Milestone H — Secondary Feature Polish

- [x] Multi-word/phrase capture: relax `analyze_rarity`'s space-containing-string bailout; decide fallback behavior for phrases (skip rarity check, still translate) — phrases skip rarity and are still translated.
- [x] Configurable Anki note-type field mapping instead of the hardcoded `AppleStyleFinalV3` assumption — deck/model selects already exist in the UI, field mapping doesn't — `anki.field_mapping` + editor with presets (Apple style, Chinese).
- [x] i18n: `ui_lang` is persisted but nothing branches on it — introduce `vue-i18n`, extract the currently hardcoded English strings from `App.vue` — vue-i18n with en + de.
- [x] Per-language hotkey profiles (capture directly into a chosen language pair without changing global settings each time) — `hotkey_profiles` (own language pair and optional deck).

---

## Milestone I — Advanced / Native-App Foundations

_Per decision 10 (2026-07-28): no longer "not scheduled." The cross-platform requirement means `Learn` starts alongside Milestone B, not as a someday-maybe — this milestone is the first real installment, not a long-term aspiration._
_Note: Milestones L and M below pull the SRS-scheduler and gamified-quiz items of this milestone forward, scoped concretely to the Chinese/HSK use case. Treat those two milestones as the first real installment of what this milestone describes, not a separate track._

- [x] `crates/vocab-core` extraction — moved into Milestone B directly, see B1
- [x] Second Tauri app (`apps/learn/`) consuming `vocab-core` — own review-session UI, FSRS-based scheduler instead of relying on Anki's. Targets macOS/Windows/Linux/iOS/Android via Tauri 2, plus a standalone web build (see decision 10) — this app is where the six-platform requirement actually gets satisfied, not the automator. — `apps/learn`: Tauri crate `vocab-learn` (22 commands over vocab-core, tested through the IPC layer) + Vue UI on a `PlatformAdapter`, and a standalone web build (`npm run learn:build:web`) whose adapter is a TypeScript port of the core over IndexedDB, held to the Rust core by golden vectors; the adapter-level golden scenario is also replayed through the real desktop commands (`apps/learn/src-tauri/tests/golden_adapter.rs`). The browser dev loop is `npm run learn:dev:web` / `just learn-web` (`learn:dev` is the desktop frontend that `tauri dev` loads). Desktop macOS/Windows/Linux and Web are built; iOS/Android see below.
- [ ] `ArgosTranslateProvider` — fully offline translation (implements the same `TranslationProvider` trait), likely via bundled ONNX-exported Argos models rather than shelling out to Python — **Not done:** needs an ONNX runtime crate and Argos models (hundreds of MB): new dependencies and a design of their own.
- [x] FSRS scheduler integration operating over the local SQLite store from Milestone F — `vocab_core::srs` over the shared SQLite store; Learn's review session uses it (`submit_review_rating`).
- [x] Duolingo-style mechanics: XP tally, streak + streak-freeze, tied to completed review sessions (not raw captures) — `vocab_core::gamification` (XP ledger, streak, freezes) + Learn's Stats view.
- [x] Babbel-style contextual lesson cards built from the user's own captured `example_source`/`example_target` pairs, not a generic curriculum — `vocab_core::lessons` + Learn's Lessons view (cloze cards from your own captured sentence pairs).
- [x] Anki export/import (`.apkg` read/write or AnkiConnect bulk sync) so the native app never locks users out of their existing Anki collection — AnkiConnect bulk sync: Learn's `anki_export` / `anki_import` (desktop; the web build has no Anki access). Since the final audit the settings tab also edits the field mapping and the AnkiConnect address (`FieldMappingEditor`); before, only note types whose fields are literally Hanzi/Meaning could be exported or imported.
- [ ] Mobile targets via Tauri 2's mobile support (iOS/Android) for the learning app specifically — the automator stays desktop-only since global-hotkey capture is inherently a desktop concept — **Not done:** iOS/Android need rustup targets and `tauri ios/android init` (toolchain changes the operator did not approve). Learn's crate is mobile-ready (lib crate types, `mobile_entry_point`).

---

## Milestone J — Chinese (HSK 1–6) Language Pipeline

_Extends Milestones B & C. Unlike L/M below, this slots into the existing clipboard-automator surface — no new app window required._

### J1. Data model

- [x] Add `ScriptMetadata` enum to `core/model.rs` per decision 7: `enum ScriptMetadata { None, Chinese { pinyin: String, hsk_level: Option<u8>, script_variant: ScriptVariant } }`, `enum ScriptVariant { Simplified, Traditional }`
- [x] Add `pub script_metadata: ScriptMetadata` field to `Card`
- [x] Default `script_variant` to `Simplified` — HSK is a Simplified-based standard; Traditional support is a stretch toggle (via the `opencc-rs`/OpenCC bindings), not MVP

### J2. Bundled HSK curriculum dataset — Official New HSK 3.0 (Levels 1–9)

_Per explicit decision: use the official HSK 3.0 standard (all 9 levels), not the user's own apkg deck (which only covered HSK 1–4 with no explicit level tagging). Verified by fetching the actual data, not just the repo description._

**Chosen source: [`drkameleon/complete-hsk-vocabulary`](https://github.com/drkameleon/complete-hsk-vocabulary)** — MIT license (confirmed via GitHub API), actively maintained (auto-updated via GitHub Actions).

**License caveat (final audit, 2026-10-31):** the dictionary data (`meaning_en`, `traditional`, `pinyin`, and anything derived from them, such as German meanings machine-translated from `meaning_en`) is CC-CEDICT's (complete-hsk-vocabulary builds its forms from CC-CEDICT entries and credits it), and CC-CEDICT is CC BY-SA 4.0, not MIT: distributing it needs attribution, and the adapted data is share-alike. `crates/vocab-core/data/NOTICE-CC-CEDICT` holds the attribution; showing it in the apps is open (see "Final audit").

- The `level` field tags every entry against both HSK 2.0 (`old-N`) and the official 2021 HSK 3.0 standard (`new-N`) — `new-N` _is_ "Official New HSK 3.0, Level N," so there's no ambiguity to resolve
- Pre-split per-level files already exist: `wordlists/exclusive/new/{1-9}.json` (just that level's new words) and `wordlists/inclusive/new/{1-9}.json` (cumulative through that level) — no manual filtering of the complete list needed
- Per entry: `simplified`, `radical`, `level[]`, `frequency`, `pos[]`, and per-form `traditional`, `transcriptions` (pinyin + numeric + wadegiles + bopomofo + romatzyh), `meanings[]` (English), `classifiers[]`
- Considered and rejected: [`ivankra/hsk30`](https://github.com/ivankra/hsk30) (MIT, but no inline English meanings — would need a separate CC-CEDICT cross-reference step); [`krmanik/HSK-3.0`](https://github.com/krmanik/HSK-3.0) (explicitly separates 2021 vs. 2025 revisions into folders, useful signal, but license is `NOASSERTION` — unclear terms)

**Two gaps this source has, both resolved:**

- [x] **No example sentences** in the official dataset. **Decision: reuse the apkg's ~1,431 sentences for words that overlap; leave the remaining ~9,600 words without an example sentence for now** (a documented gap to carry forward, not a blocker — can be backfilled later from another source or generation step if it matters in practice).
- [x] **Multiple `forms[]` per entry** — a handful of words have more than one pronunciation/meaning form. The flat `hsk_curriculum` schema takes the first form for MVP simplicity, discarding secondary forms. Minor fidelity loss, noted for awareness, not blocking. — resolved better than 'first form': the form matching the official HSK 3.0 reading is chosen (§ 12 item 8).

**Apkg audio/image reuse — confirmed.** The user's `Chinese Vocabulary (HSK 1-4).apkg` has genuinely useful data the official dataset lacks entirely: 100% pre-recorded native audio and 48% images for its 1,431 words. After importing the official HSK 3.0 dataset as the canonical curriculum, do a secondary pass matching apkg `Chinese` values against `hsk_curriculum.hanzi` and attaching the apkg's `vocab_audio_path`/`sentence_audio_path`/`image_path` (and its `Sentence`/`Sentence Pinyin`/`English Sentence` for the example-sentence gap above) where they match. Falls back to Milestone J4's live TTS / blank example for everything else.

### J2b. Import pipeline

- [x] Write a one-time ETL script (Rust or a throwaway script, doesn't need to ship with the app) that downloads/vendors the relevant `wordlists/exclusive/new/{1-9}.json` files, flattens each entry's first `forms[]` element, and writes the result into the `hsk_curriculum` schema below — `tools/hsk-import` (`fetch` with SHA-256-pinned sources, `build`); `crates/vocab-core/data/hsk_curriculum.json` (10,969 items) committed.
- [ ] **No German meaning exists in the source data at all — only English.** `meaning_de` has to be backfilled by running Milestone C's translation provider (English → German) over all ~11,000 entries at import time, not left to live captures alone — **Partly done:** `backfill-de` exists (resumable, quota-aware). A 120-word MyMemory sample came back about one-third wrong or garbage, so no German meanings ship; German learners see the English meaning until reviewed German is added (`crates/vocab-core/data/README.md`). Merging German also needs `build` over the fetched upstream sources, so it needs network access too.
- [x] `curriculum_order` = sort by `level` ascending, then `frequency` ascending within a level (the dataset's own frequency field is a more principled tiebreaker than anything derivable from the apkg's note-creation order) — level, then frequency, then hanzi as the final tiebreak.
- [x] Apkg audio/image/sentence reuse: import as a second pass after the primary HSK 3.0 import, matching on `hanzi` — `hsk-import apkg-enrich` (legacy `.apkg`), tested on synthetic packages. **Operator step:** run it on your own deck (the file was not available here).

### J3. Pinyin generation (for captures outside the bundled curriculum)

- [x] Add a Rust pinyin crate — [`pinyin`](https://crates.io/crates/pinyin) (385k downloads, established) or [`mandarin-to-pinyin`](https://crates.io/crates/mandarin-to-pinyin) (PHF-based lookup, no external files) — evaluate both against a handful of tone-mark edge cases before picking one — `pinyin` 0.11 (the only approved candidate); the evaluation and its heteronym limits are documented in `providers/chinese.rs`.
- [x] Wire pinyin generation into the capture pipeline for any captured Hanzi not already present in `hsk_curriculum`

### J4. Language-aware provider routing (extends Milestone B's provider traits)

- [x] Add a `ProviderRegistry` keyed by language code, resolved once per capture before calling translate/example/rarity — Datamuse (`RarityProvider`) and dictionaryapi.dev (`ExampleProvider`) are **English-only** and must not be called for Chinese captures — `providers/registry.rs`.
- [x] Detect Chinese input either from the active `source_lang` config or by sniffing the captured text for CJK Unified Ideographs (U+4E00–U+9FFF), so routing is correct even if config defaults lag
- [x] Chinese rarity/synonym step: **no-op by design** — HSK level already _is_ the frequency/difficulty signal for curriculum vocabulary, so there's no need to duplicate what `hsk_curriculum.hsk_level` already encodes for in-curriculum captures
- [x] Chinese example sentences: prefer the bundled `hsk_curriculum` example sentence when the captured hanzi matches a curriculum entry; fall back to the existing generic-fallback-string pattern (`fetch_example`'s current behavior) for out-of-curriculum captures
- [x] Confirm the existing Google-Translate-TTS provider accepts `zh-CN` as a language code (it's already parametrized by `source_lang`) — reuse rather than build a new TTS path — verified against the live endpoint on 2026-10-31 (`tl=zh-CN` returns audio/mpeg).

### J5. Anki export mapping

- [x] Extend `anki/mapper.rs` to map `character`/`pinyin`/`hsk_level` into whatever fields the user's chosen Anki note type expects — flag in the UI that a Chinese-compatible note type (Hanzi/Pinyin/Meaning/Audio fields) is a setup requirement, same pattern as the existing deck/model discovery UI — `FieldMapping::chinese_default` + the UI warning when a Chinese setup lacks a pinyin binding.

---

## Milestone K — Local Curriculum & SRS Data Layer

_Extends Milestone F's SQLite store with the schema needed by L and M._

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

- [x] `srs_state` is a separate 1:1 table from `captures`, not inline columns — keeps the scheduling algorithm swappable/migratable without touching card content
- [x] `hsk_curriculum` is reference data (populated once from Milestone J2's dataset), distinct from the user's personal `captures` — "learned" is defined as _has a row in `srs_state`_, never membership in `hsk_curriculum` alone
- [x] Migration tooling: add `rusqlite_migration` (or hand-rolled versioned `.sql` files) since this is now a real multi-table schema, not the single-file config store Milestone F started with — hand-rolled versioned SQL with `PRAGMA user_version` (no extra dependency).

---

## Milestone L — Adaptive Daily Queue Engine

_Builds on Milestone K. This is the first concrete installment of Milestone I's "own SRS" ambition — scoped to Chinese/HSK._

### L1. Scheduling algorithm

- [x] Evaluate [`rs-fsrs`](https://crates.io/crates/rs-fsrs) (lighter, scheduler-only) vs. [`fsrs-rs`](https://github.com/open-spaced-repetition/fsrs-rs) (the official Open Spaced Repetition crate, includes training/simulation via the Burn ML framework — heavier dependency). **Recommendation: start with `rs-fsrs`** for MVP scheduling; revisit `fsrs-rs` only if per-user weight optimization becomes a real feature request — rs-fsrs 1.2.1 chosen, as recommended.
- [x] Implement the review-rating → next-state transition (`Again`/`Hard`/`Good`/`Easy` → new `stability`/`difficulty`/`due_date`) writing into `srs_state` — `vocab_core::srs::submit_review`.

### L2. Daily onboarding prompt

- [x] Persist `last_prompted_date` (new `AppConfig` field or its own small state row) — `app_state` row `last_prompted_date`.
- [x] On main-window mount, compare `last_prompted_date` to today's local date; if different or unset, show a modal — _"How many new characters do you want to learn today?"_ — before the normal UI renders — Learn's main window shows the daily prompt modal on mount (`DailyPromptModal`, `useDailyPrompt`).
- [x] **Decide and document the dismiss-without-answering behavior** (proposed default: treat as quota = 0 / reviews-only for that day, but let the user reopen the prompt later the same day from a menu action) — this is a UX detail that needs an explicit decision, not a silent assumption — **Decided:** dismiss = quota 0 / reviews only for the day; a menu action reopens the prompt and a later answer keeps the progress (`srs/daily.rs`, `docs/architecture.md` § 7.1).
- [x] On submit, write a `daily_sessions` row and update `last_prompted_date` — `reviews_due_count` counts only due cards that were rated before: a word introduced and never rated waits for its first review, which counts as a new item, so it is no review target (`docs/architecture.md` § 7.1; fixed in the final audit, where such a word could keep a day from completing).

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

- [x] Priority 1 (mandatory): all `srs_state` rows with `due_date <= today`, ordered by `due_date` ascending
- [x] Priority 2 (optional): if `new_quota > 0`, append the next N unlearned `hsk_curriculum` rows in strict `curriculum_order` — skipping _learned_ hanzi (§ 12 item 9).
- [x] Low-time mode (`new_quota = 0`) is just the zero-length branch of the same function, not a separate code path — avoid over-engineering a distinct "mode"
- [x] New Tauri commands: `get_daily_queue`, `submit_review_rating(card_id, rating)` — Learn: `get_daily_queue`, `submit_review_rating` (plus `start_new_item`, `preview_ratings`, ...).

---

## Milestone M — Gamified Quiz Engine (Duolingo/Babbel-style)

_Builds on Milestone L's queue. Needs its own UI surface — see the open decision below._

### M1. Vocabulary isolation (the hard constraint)

- [x] Define **one** canonical `LearnedVocabularyPool` query: every `captures` row that has a corresponding `srs_state` row (i.e., has been introduced via the daily queue at least once) — every exercise type below must draw distractors and sentence pools from this single query, not ad hoc per-exercise lookups, so the isolation guarantee is centralized and testable as one thing (see Milestone G+'s leak test) — `LearnedVocabularyPool::QUERY` / `load`.
- [x] `hsk_curriculum` rows with no matching `srs_state` entry must never appear in quiz content, only in the queue builder's "next new items" selection

### M2. Exercise types

- [x] **Multiple Choice**: character ↔ pinyin/meaning (German or English), correct answer plus N-1 distractors drawn from `LearnedVocabularyPool` excluding the correct answer. MVP: uniform-random distractor selection. Stretch: weight distractors toward same-HSK-level or pinyin-similar items for a real difficulty curve — core generator (uniform random, options in one language) + Learn's multiple-choice UI; the stretch weighting is not done.
- [x] **Sentence Builder (word bank)**: tokenize a sentence into tiles — prefer splitting on known-vocabulary word boundaries from `hsk_curriculum`/`captures`, falling back to per-character tiles for anything not recognized as a known unit; shuffle tile order; drag-and-drop or tap-to-place UI; check ordering on submit — pool-only segmentation (§ 12 item 9 / § 7.2) + Learn's tap-to-place tile board.
- [x] **Audio Listening Comprehension**: reuse the TTS audio already generated for the card (Milestone J4); two variants — (a) play audio → multiple-choice identify the character, (b) play audio → sentence-builder using the heard sentence's tiles — both variants in the core + Learn's listening UIs (stored audio, else TTS on desktop).

### M3. UI surface — resolved by decision 10 (2026-07-28)

- [x] Build the quiz/review session as its own view in the `Learn` app (`apps/learn/`, Milestone I), **not** inside the automator. This reverses the milestone's original recommendation (build it as a view in the existing single Tauri app) — that made sense before the six-platform requirement existed, but the automator is permanently desktop-only (global-hotkey capture), so anything that needs to reach iOS/Android/Web cannot live there regardless of how early or validated it is. All scheduling/quiz logic still lives in `crates/vocab-core` (Milestone B) so both apps consume the same implementation — only the UI shell differs per app. — Learn's Review, Practice and Lessons views; all scheduling/quiz logic stays in `crates/vocab-core`.

---

## Milestone N — Release, Distribution & Auto-Update Infrastructure

_Added 2026-07-29. Priority: highest — sequenced immediately after Milestone B (see decision 11). Closes a real, previously-undefined gap: the operator-approval pipeline (Engineer → QA → Test Manager → operator Tier 2 approval) has never had anything after it. Nothing currently builds, signs, publishes, or updates anywhere. Scope here is `Automator` (desktop: macOS/Windows/Linux) — `Learn`'s mobile/web distribution is evaluated in N8 for planning purposes but is its own future milestone, not built here._

### N0. The prerequisite nobody's built yet: a real git workflow

Every engineering session so far has deliberately left changes uncommitted in the working tree ("git safety protocol says never auto-commit without explicit ask" — a real quote from an actual Engineer run this week). That's correct caution, but it means the "Tier 2 merge approval" step in `HOW_THE_COMPANY_WORKS.md` has been a conceptual placeholder, not a working mechanism — there's nothing to merge yet.

- [x] Engineer's own last step becomes: commit to a feature branch named after the Paperclip ticket (e.g. `pot-7-milestone-n`), not just leave the working tree dirty. The existing "don't auto-commit to `main`" caution stays exactly right — a feature branch isn't `main`. — this work is on `claude/implement-open-todos` (no Paperclip ticket number was available).
- [x] Set up a real GitHub remote for this repo now, specifically because N5's CI/CD needs a remote to trigger from (`push` → tag → Actions). Staying fully local blocks the release pipeline entirely, not just makes it less convenient. — `origin` = github.com/fabsGitHub/vocabulary-anki-automator.
- [ ] The operator's Tier 2 approval becomes concrete: review the branch's diff (`git diff main..<branch>` or a GitHub PR if pushed), merge into `main` themselves. This is the existing Tier 2 gate in `SOUL.md` — nothing new is being authorized here, just making the action actually possible. — **Operator step:** review `git diff main..claude/implement-open-todos` and merge.
- [x] Tag convention: `v0.MINOR.PATCH` while pre-1.0 (the app has no real users yet — see N8's App Store versioning note for why this matters later). One tag = one release; N5's pipeline triggers off tag push, not off every merge to `main`. — documented in `CLAUDE.md`; `release.yml` refuses a tag that differs from `tauri.conf.json`'s version.

### N1. macOS: code signing & notarization

- [ ] Apple Developer Program enrollment ($99/year, operator's own account — not something an agent can do, this requires a human with a credit card and an Apple ID) — **Operator step.**
- [ ] Generate a Developer ID Application certificate + Developer ID Installer certificate from that account — **Operator step.**
- [x] Wire signing/notarization into `tauri.conf.json` / build-time env vars: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (an **app-specific password**, generated separately — never the real Apple ID password), `APPLE_TEAM_ID` — `release.yml` passes every `APPLE_*` secret by name, to the macOS jobs only, and the Tauri CLI reads them from the environment. `tauri.conf.json` names no signing identity: its literal `{{env.APPLE_SIGNING_IDENTITY:::}}` was never expanded by Tauri and broke every local macOS bundle, so the final audit removed it (`src-tauri/tests/bundle_config.rs`). `src-tauri/entitlements.mac.plist` is intentionally empty: no hardened-runtime exceptions (`src-tauri/tests/security_config.rs`).
- [ ] Confirm `tauri build` runs `xcrun notarytool submit --wait` and staples the ticket automatically once those env vars are set — Tauri does this natively, don't hand-roll a separate notarization script — **Operator step:** confirm on the first tagged release (tauri-action notarizes when the `APPLE_*` secrets are set).
- [x] Distribution channel: **direct notarized `.dmg` download**, not the Mac App Store, for v1 — no review process, no 30% cut, no sandboxing rework required. Revisit only if App Store discoverability becomes worth the tradeoff later. — `.dmg` via the bundler; no Mac App Store work.

### N2. Windows: code signing

- [ ] Obtain a code-signing certificate — **OV (Organization Validation), not EV.** Verified current as of this writing: Microsoft's 2024 Trusted Root Program change removed EV's instant SmartScreen bypass — EV and OV now both build SmartScreen reputation the same way, via download volume over time. Paying the EV premium buys nothing here. — **Operator step.**
- [ ] Real, current constraint: certificate validity is capped at **458 days** (CA/Browser Forum requirement effective March 2026, enforced across all public CAs) — budget for near-annual renewal regardless of which CA is chosen. — **Operator step** (renewal budgeting).
- [ ] Options: a traditional CA-issued cert (DigiCert, SSL.com, Sectigo) or a cloud-HSM-backed option like Azure Trusted Signing (usually cheaper, no physical hardware token to manage/lose) — **Operator decision.**
- [x] Wire into Tauri's Windows bundle config — plain `.pfx` certs reference directly; HSM-backed certs need `signtool` invoked with the HSM's provider, check whichever option is chosen against Tauri's current Windows signing docs at implementation time — `release.yml` imports a `.pfx` secret and signs by thumbprint; the HSM alternative (`bundle.windows.signCommand`) is noted there.
- [x] **Expect SmartScreen warnings on early releases regardless of cert type** — this is real, unavoidable UX cost from having near-zero download volume yet, not a sign the signing setup is broken. Don't debug this as if it were a bug. — informational; nothing to build.
- [x] Distribution: direct `.msi`/`.exe` download for v1, same reasoning as N1 (Microsoft Store would need MSIX repackaging and a review process for a trust benefit direct signing already covers reasonably well). — `.msi` + NSIS `.exe` via `bundle.targets: "all"`.

### N3. Linux: packaging & distribution

- [x] Build `.deb` and `.rpm` via Tauri's native bundler (`bundle.targets` in `tauri.conf.json` — no extra tooling needed beyond what `tauri build` already does) — `bundle.targets: "all"` (the invalid `bundle.linux.targets` key from Milestone N broke every build and was removed).
- [x] **Also build an AppImage — not optional if N4's auto-updater should cover Linux at all.** Verified: Tauri's updater plugin only supports AppImage as the Linux update artifact. `.deb`/`.rpm` installs would update via their normal package manager instead, which is fine, but decide this explicitly: AppImage is the auto-update-integrated distribution, `.deb`/`.rpm` are secondary convenience downloads without auto-update.
- [x] No code-signing/notarization requirement at the OS level on Linux (unlike macOS/Windows) — meaningfully lower barrier to entry here — informational.
- [ ] Flathub (Flatpak) as a secondary channel is worth real consideration once the app is stable — genuine Linux-user discoverability — but it's its own manifest/review process, not urgent for v1 — deferred, as planned.

### N4. Auto-updater (Tauri, all three desktop platforms)

- [ ] Generate the updater signing keypair: `tauri signer generate`. Public key → `plugins.updater.pubkey` in `tauri.conf.json` (safe to commit). Private key → **only** as a CI-injected secret (`TAURI_SIGNING_PRIVATE_KEY`) — never committed to this repo, never written into company-workspace, never pasted into a Paperclip/Plane ticket or comment. See N7. — **Operator step:** whoever runs `tauri signer generate` sees the private key, so only the operator runs it, writing the key outside the repo (`docs/signing-key-custody.md`); until the public key replaces the placeholder, `release.yml`'s preflight (`scripts/release-preflight.mjs`, which also refuses an empty, malformed or secret key) refuses to release.
- [x] Set `bundle.createUpdaterArtifacts: true` — `tauri build` then produces a `.sig` file alongside every platform artifact automatically — already set. The `.sig` needs `TAURI_SIGNING_PRIVATE_KEY`, which only CI has, so a local `tauri build` stops after bundling: build locally with `npm run tauri build -- --no-sign` or `--no-bundle`.
- [x] Hosting: a static `latest.json` manifest published as a GitHub Release asset (updated by N5's CI on every release), not a dynamic update server — no infrastructure to run or secure, GitHub's own CDN handles availability, and staged/percentage rollout isn't a real need at this app's current scale — `release.yml` (`uploadUpdaterJson: true`).
- [x] App-side: check for updates on startup/idle via the updater plugin's `check()` call, surface a small non-blocking "update available, restart to apply" banner — never a silent forced restart, and never a check mid-capture (would interrupt the actual core workflow this app exists for) — `src/components/UpdateBanner.vue` + `useUpdater`; skipped while `capture_in_progress`; restart only on click.

### N5. CI/CD release pipeline

- [x] GitHub Actions workflow triggered on `v*` tag push, built around `tauri-apps/tauri-action` (Tauri's own official action) — one step builds signed artifacts for all three desktop platforms in a runner matrix, generates the updater manifest, and publishes the GitHub Release — `.github/workflows/release.yml` (tauri-action v1.0.0, pinned by SHA; `ci.yml` runs the checks on PRs). The final audit narrowed its permissions (`contents: write` only on the build job, `persist-credentials: false` on every checkout, `APPLE_*` only on macOS); `ci.yml` also runs `tauri build --debug --no-bundle` for both apps on Linux.
- [x] Runner matrix: `macos-latest` (`.dmg`, notarized per N1), `windows-latest` (`.msi`/`.exe`, signed per N2), `ubuntu-latest` (`.deb`/`.rpm`/AppImage per N3) — macOS arm64 + x86_64, Windows, Ubuntu 22.04 (older glibc baseline for the AppImage).
- [x] All N1/N2/N4 secrets live only in GitHub Actions repo secrets — this is the actual point of N0's remote-repo requirement — referenced by name only.
- [x] **This is the literal, mechanical answer to "how does an approved milestone reach the user":** operator approves (Tier 2, already exists, unchanged) → operator tags a release (`git tag vX.Y.Z && git push --tags` — an explicit, deliberate human action, not something any agent triggers autonomously) → CI builds, signs, and publishes automatically → already-installed apps pick up the update via N4's next startup check. No step in this chain is autonomous end-to-end; the human-triggered tag push is the deliberate release gate, same spirit as the existing Tier 2 merge gate. — wired end to end; the tag push stays the operator's.

### N6. Signing-key custody (why this milestone is opus-critical, not routine)

- [x] The updater private key and both platform signing credentials are collectively the single highest-blast-radius secret this company holds — whoever has them can push a signed, auto-installed "update" to every user's machine. Hold this to at least Milestone E's supply-chain rigor, not routine config-file handling. — `docs/signing-key-custody.md`.
- [x] No agent (Engineer, QA, Test Manager, or PM) ever sees, requests, or handles these credentials directly, under any circumstance — they exist only as GitHub Actions secrets, referenced by name in workflow YAML, never echoed to a log, never pasted into a ticket/comment/chat message by anyone, human or agent. — no credential was seen or handled in this session.
- [x] This is an extension of the exact Tier 2/3 boundary `SOUL.md` already draws, not a new category of rule: publishing/releasing stays a deliberate, human-triggered action, full stop, regardless of how "ready to publish" the label on a ticket says a milestone is.

### N7. Forward-looking: `Learn`'s six-platform distribution (informational — not this milestone's checklist)

_Per decision 11 and the operator's explicit request to think this through now even though it's built later, as its own milestone, once Learn's UI work begins (Milestone I). Nothing below is a checkbox for N — it's context for whoever scopes that future milestone._

**iOS:**

- Public distribution is realistically App-Store-only — no meaningful sideloading path for non-technical end users. Same Apple Developer Program membership as N1 covers this; it's one $99/year account, not a second cost.
- TestFlight is the real, well-supported beta channel — use it before any public release.
- No custom auto-updater is possible or needed here, unlike desktop's from-scratch N4 — the App Store _is_ the update mechanism; users get updates per their OS's own automatic-update setting.
- App Review adds real, variable latency (historically on the order of 1–3 days, and a real rejection is possible) — this shapes release cadence in a way desktop doesn't have; a future milestone should budget for it rather than assume tag-push-to-live speed.
- Verified current as of this research (2026-07-29): Tauri 2 has shipped real, stable iOS support since the 2.0 stable release (October 2024) — same shared Rust core as `Automator`/`Learn`-desktop, genuinely not a separate codebase or rewrite.

**Android:**

- Google Play: one-time $25 developer account (cheaper and less friction than Apple's model), APK/AAB upload, Play Console review (generally faster/less strict than Apple's), and **staged rollout is natively supported** — a real advantage over iOS for catching a bad release before it reaches everyone.
- Unlike iOS, direct APK sideloading is technically viable for end users, which opens two real secondary-channel options: F-Droid (open-source-friendly store, no Google account required for users — a good fit if this project stays open-source) and/or a custom Tauri-updater-based direct-APK channel (verify at implementation time whether Tauri's updater plugin covers the Android target the same way it covers desktop — mobile-target updater support should not be assumed identical to desktop without checking Tauri's docs at that point). Play-Store-installed users update through the Store regardless, same as iOS.
- Same Tauri 2 mobile support as iOS, one shared build configuration (`tauri android build`).

**Web:**

- Not an app store model at all — closer to N1–N3's "direct download," but simpler: static hosting (GitHub Pages, Vercel, Netlify, or S3+CloudFront are all real, standard options).
- "Update" is trivial here — there's no user-installed binary at all; every page load serves whatever was last deployed. This makes Web by far the cheapest platform to iterate on, worth weighing when a future milestone sequences `Learn`'s rollout order.
- Requires the platform-adapter work decision 10 already flagged (every Tauri-specific call needs a browser-native fallback) — that's `Learn`/Milestone I's own scope, not N's. N5's tag-triggered build+publish pattern extends naturally to "also deploy the web build on every release," though — no new CI concept needed, just an added job.

**Windows/Linux for `Learn`:** the same N2/N3 mechanics apply once `Learn`'s desktop build exists — no new research needed when that milestone starts, this section exists purely so the operator has the full six-platform picture now rather than macOS being the only one considered.

---

## Final audit (2026-10-31)

Five independent auditors (security, IPC contracts, core correctness and Rust/TypeScript parity, plan completeness, release builds) reviewed the finished branch and fixed what they found, each fix with a regression test that fails without it unless noted; `docs/architecture.md` records the resulting rules. Beyond the corrections noted at the items above:

- [x] Security: a strict CSP for both apps (it was `null`); capabilities cut to what each window calls (no `core:default`, `opener:default` or `updater:default`); empty macOS entitlements (library validation had been switched off in an Accessibility-privileged app); AnkiConnect allows no web origin; `hsk-import apkg-enrich` caps inflated sizes and no longer writes through symlinks; least-privilege release workflow — `src-tauri/tests/security_config.rs`, `apps/learn/src-tauri/tests/security_config.rs`, `scripts/release-preflight.test.ts`, `tools/hsk-import/tests/apkg.rs`.
- [x] IPC contracts pinned: every shared shape, command name, argument name, result and event payload of both apps is serialized from Rust and checked against the TypeScript types — `src-tauri/tests/ipc_shapes.rs`, `apps/learn/src-tauri/tests/ipc_shapes.rs`, `src/api/ipc_shapes.test.ts`, `apps/learn/src/platform/ipc_shapes.test.ts` (regenerate the fixtures with `UPDATE_IPC_SHAPES=1 cargo test --test ipc_shapes`). The Automator's "Sync pending" count now counts only what that sync pushes.
- [x] Core: a word introduced but never rated no longer counts as a review target (it could keep a day from ever completing); the leak tests fail for every isolation mutant tried; mutation testing (159 Rust and 183 TypeScript mutants) added the missing tests and golden vectors.
- [x] Release builds: the macOS signing-identity template removed (it broke every local bundle); Learn's desktop build no longer ships the web build's 2 MB curriculum; the Automator's dev server no longer watches `target/`; CI runs `tauri build --debug --no-bundle` for both apps; `npm run learn:dev:web` serves the web build for development.
- [x] The Automator keeps running in the tray when its last window closes: before the first capture, closing the settings window ended the app and its hotkeys. `src-tauri/tests/lifecycle.rs` runs the mock event loop on the app `configure` builds (so the wiring is covered too) and pins that Quit still exits. On macOS a click on the Dock icon reopens the settings window: reviewed, not tested (the mock runtime raises no Reopen). Two limits are documented in `docs/architecture.md` § 8.3: where no tray icon is shown (GNOME without an AppIndicator extension) the running app has no visible way back to its settings or to Quit, and with no single-instance guard a second launch starts a second process whose hotkeys collide (`tauri-plugin-single-instance` would fix it; new dependency, operator approval).
- [x] Learn's web build explains that its Lessons tab stays empty (lessons are built from desktop captures) and its settings tab shows only the meaning language (the Anki settings do nothing there); README.md rewritten (it was still the Tauri template); `docs/test-harness-strategy.md` and `docs/signing-key-custody.md` corrected.

Left open, each needing an operator decision, an unapproved dependency or artwork:

- [ ] **Release blocker — data licenses:** the bundled dictionary data (`meaning_en`, `traditional`, `pinyin`) is CC-CEDICT's (CC BY-SA 4.0: attribution, share-alike), not only MIT. `crates/vocab-core/data/NOTICE-CC-CEDICT` records the attribution; showing it and the two MIT notices in the apps and the web build (an About/credits view or bundled notice files), and whether vocab-core's `license = "MIT"` should name CC-BY-SA-4.0 for the embedded data, need an operator/legal decision.
- [x] Learn never scheduled captures or Anki-imported notes that are not HSK curriculum words: they got no `srs_state` row, so captures appeared only in lessons (those with an example translation), and Anki-imported notes (`origin = anki_import`) appeared nowhere in Learn. **Decided 2026-10-31 (operator):** they enter the daily queue automatically, from the same new-item quota as curriculum words, oldest capture first and before the curriculum; captured or imported curriculum words keep their curriculum turn (so an imported HSK deck keeps the curriculum's order); an import starts fresh rather than carrying Anki's review state over. Built on both platforms: `QueueItem::NewCapture`, `Store::list_unscheduled_captures`, `introduce_capture` and Learn's `start_new_capture` (`docs/architecture.md` § 7.1, § 10, § 12 item 10); once started, a card is learned vocabulary for practice, and quiz isolation is unchanged (exercises still come from `LearnedVocabularyPool` alone) — `crates/vocab-core/tests/srs_queue.rs`, `apps/learn/src-tauri/src/session.rs`, `apps/learn/src-tauri/tests/ipc_review.rs` and `ipc_anki.rs`, golden vectors (new `queue` cases, the "captures and imports" scenarios, replayed by `golden.test.ts` and `golden_adapter.rs`), `apps/learn/src/views/ReviewSession.test.ts`. Lessons still use clipboard captures only (§ 7.3).
- [ ] `anki.connect_url` accepts any http(s) host, so a compromised webview could redirect later pushes to another host; limiting it to loopback would reverse a tested behaviour — operator decision (`docs/architecture.md` § 8.3). The unused `tauri-plugin-opener` could also be dropped (no capability grants it any more).
- [ ] After a capture, the clipboard restore writes the previous text back without the pasteboard's concealed/transient markers that password managers set; keeping them needs a new dependency (`arboard`/`objc2`) — operator approval.
- [ ] A single-instance guard for the Automator (`tauri-plugin-single-instance`, with a callback that shows the settings window): a second launch now starts a second process whose hotkeys collide with the first's, and where no tray icon is shown it is the only way back to the settings — new dependency, operator approval.
- [ ] The identifier `com.ankiautomator.app` ends in `.app`, which Tauri warns about; renaming it moves the Automator's app data (config, consent, profiles) and needs a migration — operator decision.
- [ ] Both apps ship Tauri's template icons (the same files) — needs artwork.
- [ ] Smoke-test the built apps (`npm run tauri build -- --no-sign`, and Learn's `npm run learn:tauri build -- --no-sign`) for CSP violations, IPC, audio playback and the update banner: the CSP applies only to built apps, and no GUI could be run where this was implemented — operator step.
- [ ] Known minor limits, left as they are: the end-of-day bound uses the UTC offset sampled at "now", so on a DST-switch day it is an hour off until the switch (§ 2); `is_cjk_char` covers only the two main CJK blocks, so Ext-B and compatibility ideographs count as no hanzi (§ 3); a session opened lazily after midnight can complete one review early; tauri-action finds `src-tauri/` before `apps/learn/src-tauri/` only by glob order.
