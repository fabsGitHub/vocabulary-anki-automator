# Architecture & cross-crate contracts

This is the binding contract between the pieces of the workspace. Anything
listed here is consumed by more than one crate or by a frontend, so changing a
name, a serialized shape or a semantic described here means updating every
consumer in the same change. Implementation details that stay inside one
module are deliberately not specified.

IMPLEMENTATION_PLAN.md is the roadmap; this document records how it was built
and every place the build deviated from the plan (§ 12).

## 1. Workspace layout

```
Cargo.toml                 workspace root (Cargo.lock and target/ live here)
crates/vocab-core/         shared core: model, providers, pipeline, store, srs, quiz, ...
crates/vocab-core/data/    bundled reference data (hsk_curriculum.json)
crates/anki-bridge/        AnkiConnect client + Card -> Anki note mapper
src-tauri/                 Automator: desktop clipboard-capture app (package `vocab-automator`)
src/                       Automator frontend (Vue 3 + vue-i18n), served by Vite on :1420
apps/learn/                Learn: review/quiz app (Vue frontend + src-tauri crate `vocab-learn`)
tools/hsk-import/          one-time HSK 3.0 ETL (binary `hsk-import`, not shipped)
docs/                      this file, test-harness strategy, signing-key custody
scripts/                   bootstrap, run-all-checks, git hooks
```

## 2. Conventions (all crates)

- **No network in tests, ever.** Every provider and client takes a base URL
  (`with_base_url`) so tests point it at a `wiremock` server. Unit tests that
  don't need HTTP use in-memory fakes of the provider traits.
- **Hard rule (Milestone A4):** nothing that downloads or installs Anki or
  AnkiConnect runs in tests or at startup. The install paths are reachable only
  from explicit, consent-gated commands (§ 8.4), and their download step goes
  through a `Downloader` trait that tests replace with a fake.
- **Time:** anything that depends on "now" or the local date takes
  `&dyn vocab_core::time::Clock`. Never call `Utc::now()`/`Local::now()` in
  scheduling, session, streak or prompt logic.
- **Local dates and DST:** the UTC offset is a `FixedOffset` sampled when the
  clock is read (`Local::now().offset()`; the web build's
  `Date.getTimezoneOffset()`), and each command reads the clock once (both
  apps snapshot it), so its instant, local date and offset always agree:
  `session_date`, XP dates, streak days and the prompt date are the local
  date of that instant, DST switches included. Only "the end of today"
  (`local_day_bounds(today, offset)`: local midnight at that offset plus 24 h)
  is approximate on the day of a DST switch, and only while the switch is
  still ahead: before a spring-forward switch the day ends an hour late
  (cards due in the next day's first hour count as due today), before a
  fall-back switch an hour early (cards due in the day's last hour wait for
  the switch). The due queue, `reviews_due_count` and `due_today` are
  computed from that bound; nothing stores it.
- **Timestamps** are stored and serialized as RFC 3339 UTC with milliseconds
  (`to_rfc3339_opts(SecondsFormat::Millis, true)`, e.g.
  `2026-10-31T12:00:00.000Z`), which sorts lexicographically. Local calendar
  dates are `YYYY-MM-DD` (`time::format_date`). Serde's default chrono format
  (used in IPC payloads) is also RFC 3339; consumers must accept both.
- **Errors:** library crates return typed errors (`thiserror`); Tauri commands
  return `Result<T, String>` (or a typed error that serializes) at the IPC edge.
  No `unwrap()`/`expect()` on I/O, network or user data outside tests.
- **HTTP clients** are built with a 10 s timeout and a descriptive User-Agent.
- **Formatting/lint gates:** `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `npx vue-tsc --noEmit`,
  `npx eslint .`, `npx prettier --check .`. New code must pass all of them.
- **Sandboxed dev environments:** if `~/.cargo` is not writable, set `CARGO_HOME` to an
  absolute path such as `/tmp/claude-501/cargo-home` (seed it by copying
  `~/.cargo/registry`; `$TMPDIR` resolves differently inside and outside the sandbox),
  and pass `--cache "$TMPDIR/npm-cache"` to npm. The wiremock tests start servers on
  127.0.0.1, so they need local port binding (`sandbox.network.allowLocalBinding: true`)
  or a run outside the sandbox; they never touch anything but loopback.
- **Proxies:** clients that talk to AnkiConnect never use an HTTP proxy (it is always
  local); provider clients honour the system proxy settings.

## 3. Language codes and script detection (`vocab_core::language`)

Canonical codes are `en`, `de`, `fr`, `es`, `it`, `zh-CN` (lowercase, except
the `zh-CN` region). Config, `Card.source_lang/target_lang`, provider calls and
the database all use canonical codes.

```rust
pub const SUPPORTED_LANGS: &[&str] = &["en", "de", "fr", "es", "it", "zh-CN"];
pub fn normalize_lang(code: &str) -> String;   // "EN"->"en", "zh"/"ZH"/"zh-cn"/"zh_CN"/"zh-Hans"->"zh-CN"; unknown -> trimmed lowercase
pub fn is_chinese(code: &str) -> bool;         // normalize_lang(code) == "zh-CN"
pub fn is_cjk_char(c: char) -> bool;           // U+4E00..=U+9FFF (CJK Unified Ideographs) and U+3400..=U+4DBF (Ext. A)
pub fn contains_cjk(text: &str) -> bool;
pub fn cjk_chars(text: &str) -> Vec<char>;     // in order, duplicates kept
pub fn effective_source_lang(configured: &str, text: &str) -> String; // "zh-CN" if text contains CJK, else normalize_lang(configured)
pub fn is_phrase(text: &str) -> bool;          // contains whitespace after trimming (Milestone H)
```

## 4. Providers (`vocab_core::providers`)

Traits (already in `providers/mod.rs`): `TranslationProvider`,
`ExampleProvider`, `RarityProvider`, `TtsProvider`, `ScriptAnnotator`,
`CurriculumLookup`, plus `RarityResult`. All failures are
`vocab_core::ProviderError` (`network`, `quota_exceeded`, `http`, `malformed`,
`not_found`, `unsupported`).

| type (module)                              | trait           | notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ------------------------------------------ | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MyMemoryProvider` (`mymemory`)            | Translation     | `new(contact_email: Option<String>)`, `with_base_url(base, contact_email)`. GET `{base}/get` with `q`, `langpair=SRC%7CTGT` and, when the e-mail is set, `de=EMAIL`; base `https://api.mymemory.translated.net`. Reads `responseData.translatedText`. `quotaFinished == true`, `responseStatus == 429` or a `MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS` text → `QuotaExceeded`; other non-200 `responseStatus` → `Http`; missing fields → `Malformed`; transport → `Network`. Empty `q` → `Unsupported`. |
| `DatamuseProvider` (`datamuse`)            | Rarity          | English only. `{base}/words?sp=WORD&md=f&max=1`, then `rel_syn` for a synonym when `f:` < 5.0 per million. Unknown word → `RarityResult::default()`.                                                                                                                                                                                                                                                                                                                                                                      |
| `DictionaryApiProvider` (`dictionary_api`) | Example         | English only. `{base}/api/v2/entries/en/WORD`; first `meanings[].definitions[].example`; 404 or none → `NotFound`.                                                                                                                                                                                                                                                                                                                                                                                                        |
| `GoogleTtsProvider` (`google_tts`)         | Tts             | `{base}/translate_tts?ie=UTF-8&tl=LANG&client=tw-ob&q=TEXT`; base `https://translate.google.com`; `zh-CN` is passed through verbatim. Non-audio or empty body → `Malformed`.                                                                                                                                                                                                                                                                                                                                              |
| `PinyinAnnotator` (`chinese`)              | ScriptAnnotator | `new(Arc<dyn CurriculumLookup>)`. Curriculum hit → its pinyin + `hsk_level`; miss → generated with the `pinyin` crate (tone marks, syllables joined by single spaces, non-hanzi chars kept as-is), `hsk_level: None`. Always `ScriptVariant::Simplified`.                                                                                                                                                                                                                                                                 |
| `CurriculumExampleProvider` (`chinese`)    | Example         | `new(Arc<dyn CurriculumLookup>)`. Curriculum hit with `example_sentence_source` → that sentence; else `NotFound`.                                                                                                                                                                                                                                                                                                                                                                                                         |

Decisions recorded during implementation (binding for consumers):

- Provider names, which reach the UI as `ProviderError.provider`: `mymemory`,
  `datamuse`, `dictionary_api`, `google_tts`, `pinyin`, `curriculum`.
- HTTP 429 from any provider is `QuotaExceeded`; every other non-2xx is `Http`.
  Transport errors are `Network` and never contain the request URL (it carries the
  captured text and the contact e-mail) — `ProviderError::from_reqwest` strips it too.
- A blank MyMemory `translatedText` is `NotFound`; an unparsable `responseStatus` is
  `Malformed`, so an error text can never end up on a card as a translation.
- Datamuse only trusts an exact (case-insensitive) match of the word, because `sp=`
  is a spelled-like search ("hipopatamus" returns "hippopotamus").
- `PinyinAnnotator` returns `Unsupported` for text without any hanzi.
- Google TTS was verified against the live endpoint (2026-10-31): `+`-encoded spaces
  return the same audio as `%20`, responses are `audio/mpeg`, and `tl=zh-CN` works.

`registry` (Milestone J4):

```rust
pub struct ProviderSet {
    pub translation: Arc<dyn TranslationProvider>,
    pub rarity: Option<Arc<dyn RarityProvider>>,
    pub example: Option<Arc<dyn ExampleProvider>>,
    pub tts: Option<Arc<dyn TtsProvider>>,
    pub annotator: Option<Arc<dyn ScriptAnnotator>>,
}
pub struct ProviderRegistry { /* default set + per-language sets */ }
impl ProviderRegistry {
    pub fn new(default: ProviderSet) -> Self;
    pub fn with_language(self, lang: &str, set: ProviderSet) -> Self;   // lang normalized
    pub fn resolve(&self, source_lang: &str) -> &ProviderSet;           // exact canonical match, else default
    pub fn standard(config: StandardProviders) -> Self;
}
pub struct StandardProviders {
    pub contact_email: Option<String>,
    pub curriculum: Arc<dyn CurriculumLookup>,
}
```

`ProviderRegistry::standard` wiring:

| source lang | translation | rarity                                                  | example           | tts    | annotator       |
| ----------- | ----------- | ------------------------------------------------------- | ----------------- | ------ | --------------- |
| `en`        | MyMemory    | Datamuse                                                | DictionaryApi     | Google | —               |
| `zh-CN`     | MyMemory    | — (no-op by design: HSK level is the difficulty signal) | CurriculumExample | Google | PinyinAnnotator |
| others      | MyMemory    | —                                                       | —                 | Google | —               |

## 5. Capture pipeline (`vocab_core::pipeline`)

```rust
pub struct CaptureRequest {
    pub text: String,
    pub source_lang: String,   // as configured; normalized + CJK-sniffed by the pipeline
    pub target_lang: String,
    pub captured_at: DateTime<Utc>,
    pub tags: Vec<String>,
}
pub struct CaptureOutcome { pub card: Card, pub warnings: Vec<PipelineWarning> }

#[derive(Serialize, Deserialize)] pub struct PipelineWarning { pub stage: PipelineStage, pub error: ProviderError }
#[serde(rename_all = "snake_case")] pub enum PipelineStage { Translate, Rarity, Example, ExampleTranslate, Tts, Annotate }
#[serde(tag = "kind", rename_all = "snake_case")] pub enum PipelineError { Empty, TooLong { len: usize, max: usize } }

pub struct CapturePipeline { /* registry + max_chars */ }
impl CapturePipeline {
    pub fn new(registry: ProviderRegistry, max_chars: usize) -> Self;
    pub async fn run(&self, req: CaptureRequest) -> Result<CaptureOutcome, PipelineError>;
}
```

Semantics, in order:

1. `text = req.text.trim()`. Empty → `Empty`. More than `max_chars`
   **characters** (not bytes — CJK is 3 bytes per char) → `TooLong`.
2. `source = effective_source_lang(req.source_lang, text)`,
   `target = normalize_lang(req.target_lang)`, `set = registry.resolve(source)`.
   When sniffing turns the source into the target (an `en` → `zh-CN` learner copies
   Chinese text), the configured pair is reversed: `target = normalize_lang(req.source_lang)`.
3. Translate `text`. Failure → warning, `back = ""` (the user can type it in
   the approval window). Provider failures are never fatal.
4. Rarity, only when `set.rarity` exists **and** `!is_phrase(text)`
   (Milestone H: phrases skip the rarity check but are still translated).
   Rare → `notes = "Rare word. More common synonym: {syn}"`, or
   `"Rare word."` without a synonym.
5. Example, only when `set.example` exists. `Ok(s)` → `example_source = s`.
   `NotFound` → language fallback template (`en`: `We use the expression '{w}' in everyday context.`;
   `zh-CN`: `我们在日常生活中使用“{w}”这个词。`). Other errors → warning + same fallback.
   Languages without an example provider get no example and no warning; an empty
   `Ok` example counts as `NotFound`.
   A non-empty `example_source` is then translated into `example_target`
   (failure → warning, empty target).
6. TTS of `text` in `source` → `audio_front` (failure → warning, `None`; empty audio → `None`).
   The steps run strictly in sequence (no async runtime dependency in vocab-core), so a
   capture on a network that drops packets can take up to ~10 s per step.
7. Annotator → `script_metadata` (failure → warning, `ScriptMetadata::None`).
8. `card.tags = req.tags`, `card.captured_at = req.captured_at`.

## 6. Store (`vocab_core::store`)

One SQLite file shared by both desktop apps:

```rust
pub fn default_db_path() -> PathBuf; // $VOCAB_DB_PATH, else dirs::data_dir()/VocabularyAutomator/vocab.sqlite3
pub struct Store { /* rusqlite::Connection */ }
#[derive(thiserror::Error)] pub enum StoreError { Sqlite(..), Json(..), Io(..), Invalid(String), NotFound(String) }
pub type StoreResult<T> = Result<T, StoreError>;
```

`Store::open(path)` creates parent dirs, sets `journal_mode=WAL`,
`foreign_keys=ON`, `busy_timeout=5000`, and runs migrations; `open_in_memory()`
for tests; `open_default()`. Migrations are versioned SQL applied in order and
tracked with `PRAGMA user_version`; each runs in a transaction; re-running is a
no-op; a database with a newer `user_version` than the code knows is refused
with `StoreError::Invalid`.

### 6.1 Schema

```sql
-- v1 (Milestone F)
CREATE TABLE captures (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  front           TEXT NOT NULL,
  back            TEXT NOT NULL,
  example_source  TEXT NOT NULL DEFAULT '',
  example_target  TEXT NOT NULL DEFAULT '',
  notes           TEXT NOT NULL DEFAULT '',
  tags            TEXT NOT NULL DEFAULT '[]',          -- JSON array of strings
  source_lang     TEXT NOT NULL,
  target_lang     TEXT NOT NULL,
  captured_at     TEXT NOT NULL,                       -- RFC 3339 UTC, millis
  synced_to_anki  INTEGER NOT NULL DEFAULT 0 CHECK (synced_to_anki IN (0, 1))
);
CREATE INDEX idx_captures_captured_at ON captures(captured_at);

-- v2 (Milestones J, K, L)
ALTER TABLE captures ADD COLUMN pinyin TEXT;
ALTER TABLE captures ADD COLUMN hsk_level INTEGER;
ALTER TABLE captures ADD COLUMN script_kind TEXT NOT NULL DEFAULT 'none';   -- ScriptMetadata::kind()
ALTER TABLE captures ADD COLUMN script_variant TEXT;                        -- 'simplified' | 'traditional' | NULL
ALTER TABLE captures ADD COLUMN origin TEXT NOT NULL DEFAULT 'capture';     -- CaptureOrigin
ALTER TABLE captures ADD COLUMN audio_front BLOB;
CREATE INDEX idx_captures_script_front ON captures(script_kind, front);

CREATE TABLE srs_state (
  card_id           INTEGER PRIMARY KEY REFERENCES captures(id) ON DELETE CASCADE,
  algorithm         TEXT NOT NULL DEFAULT 'fsrs',
  stability         REAL,
  difficulty        REAL,
  interval_days     INTEGER NOT NULL DEFAULT 0,      -- FSRS scheduled_days
  repetitions       INTEGER NOT NULL DEFAULT 0,      -- FSRS reps
  lapses            INTEGER NOT NULL DEFAULT 0,
  fsrs_state        TEXT NOT NULL DEFAULT 'new',     -- new | learning | review | relearning
  elapsed_days      INTEGER NOT NULL DEFAULT 0,
  due_date          TEXT NOT NULL,                   -- RFC 3339 UTC instant (learning steps are sub-day)
  last_reviewed_at  TEXT,
  last_rating       INTEGER CHECK (last_rating BETWEEN 1 AND 4)
);
CREATE INDEX idx_srs_state_due ON srs_state(due_date);

CREATE TABLE hsk_curriculum (
  hanzi                    TEXT PRIMARY KEY,
  traditional              TEXT,
  hsk_level                INTEGER NOT NULL,
  frequency                INTEGER,
  curriculum_order         INTEGER NOT NULL UNIQUE,
  pinyin                   TEXT NOT NULL,
  pos                      TEXT,
  meaning_en               TEXT,
  meaning_de               TEXT,
  cloze_sentence           TEXT,
  example_sentence_source  TEXT,
  example_sentence_pinyin  TEXT,
  example_sentence_target  TEXT,
  vocab_audio_path         TEXT,
  sentence_audio_path      TEXT,
  image_path               TEXT
);

CREATE TABLE daily_sessions (
  session_date          TEXT PRIMARY KEY,            -- local YYYY-MM-DD
  requested_new_count   INTEGER NOT NULL,
  reviews_due_count     INTEGER NOT NULL,
  reviews_completed     INTEGER NOT NULL DEFAULT 0,
  new_items_completed   INTEGER NOT NULL DEFAULT 0,
  completed_at          TEXT
);

CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);

-- v3 (Milestone I)
CREATE TABLE review_log (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  card_id         INTEGER NOT NULL REFERENCES captures(id) ON DELETE CASCADE,
  rating          INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 4),
  reviewed_at     TEXT NOT NULL,
  session_date    TEXT NOT NULL,
  state_before    TEXT NOT NULL,
  scheduled_days  INTEGER NOT NULL,
  elapsed_days    INTEGER NOT NULL
);
CREATE INDEX idx_review_log_card ON review_log(card_id);
CREATE TABLE xp_ledger (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  session_date  TEXT NOT NULL,
  amount        INTEGER NOT NULL,
  reason        TEXT NOT NULL,                       -- review | new_item | session_complete | exercise | lesson
  created_at    TEXT NOT NULL
);
CREATE INDEX idx_xp_ledger_date ON xp_ledger(session_date);
CREATE TABLE streak_freezes (freeze_date TEXT PRIMARY KEY);
```

"Learned" means _has an `srs_state` row_ — never "appears in `hsk_curriculum`".

### 6.2 Store API

Row types (plain data, `Serialize + Deserialize + Clone + Debug + PartialEq`):

```rust
pub struct SrsStateRow {
    pub card_id: i64, pub algorithm: String,
    pub stability: Option<f64>, pub difficulty: Option<f64>,
    pub interval_days: i64, pub repetitions: i64, pub lapses: i64,
    pub fsrs_state: String,                    // new | learning | review | relearning
    pub elapsed_days: i64,
    pub due_date: DateTime<Utc>,
    pub last_reviewed_at: Option<DateTime<Utc>>,
    pub last_rating: Option<u8>,
}
pub struct ReviewLogRow { pub id: i64, pub card_id: i64, pub rating: u8, pub reviewed_at: DateTime<Utc>,
    pub session_date: NaiveDate, pub state_before: String, pub scheduled_days: i64, pub elapsed_days: i64 }
pub struct DailySessionRow { pub session_date: NaiveDate, pub requested_new_count: u32, pub reviews_due_count: u32,
    pub reviews_completed: u32, pub new_items_completed: u32, pub completed_at: Option<DateTime<Utc>> }
```

Methods on `Store` (all `&self` unless noted):

```rust
// captures
fn insert_capture(&self, card: &Card, origin: CaptureOrigin, synced_to_anki: bool) -> StoreResult<i64>;
fn get_capture(&self, id: i64) -> StoreResult<Option<StoredCapture>>;
fn list_recent_captures(&self, limit: u32) -> StoreResult<Vec<StoredCapture>>;   // captured_at DESC, id DESC
fn list_captures(&self, origin: Option<CaptureOrigin>) -> StoreResult<Vec<StoredCapture>>; // captured_at ASC, id ASC
fn list_unsynced_captures(&self) -> StoreResult<Vec<StoredCapture>>;             // synced_to_anki = 0, id ASC
fn set_synced_to_anki(&self, id: i64, synced: bool) -> StoreResult<()>;
fn delete_capture(&self, id: i64) -> StoreResult<bool>;                          // cascades srs_state + review_log
fn find_capture_by_front(&self, front: &str, script_kind: &str) -> StoreResult<Option<StoredCapture>>;
fn chinese_capture_fronts(&self) -> StoreResult<HashSet<String>>;               // fronts with script_kind = 'chinese'
fn learned_chinese_fronts(&self) -> StoreResult<HashSet<String>>;               // ... that also have an srs_state row
fn update_capture_card(&self, id: i64, card: &Card) -> StoreResult<()>;
// curriculum
fn curriculum_count(&self) -> StoreResult<i64>;
fn replace_curriculum(&mut self, items: &[CurriculumItem]) -> StoreResult<()>;   // one transaction
fn ensure_curriculum_seeded(&mut self, items: &[CurriculumItem]) -> StoreResult<bool>; // reseeds when the fingerprint
                                                                                // (app_state 'curriculum_fingerprint') differs; true if it wrote
fn get_curriculum_item(&self, hanzi: &str) -> StoreResult<Option<CurriculumItem>>;
fn curriculum_in_order(&self) -> StoreResult<Vec<CurriculumItem>>;              // curriculum_order ASC
// srs_state
fn get_srs_state(&self, card_id: i64) -> StoreResult<Option<SrsStateRow>>;
fn upsert_srs_state(&self, row: &SrsStateRow) -> StoreResult<()>;
fn list_srs_due_before(&self, before: DateTime<Utc>) -> StoreResult<Vec<SrsStateRow>>; // due_date < before, ORDER BY due_date, card_id
fn count_srs_states(&self) -> StoreResult<i64>;
// review_log
fn insert_review_log(&self, row: &ReviewLogRow) -> StoreResult<i64>;           // row.id ignored
fn review_log_for_card(&self, card_id: i64) -> StoreResult<Vec<ReviewLogRow>>;
// daily_sessions
fn get_daily_session(&self, date: NaiveDate) -> StoreResult<Option<DailySessionRow>>;
fn upsert_daily_session(&self, row: &DailySessionRow) -> StoreResult<()>;
fn list_completed_session_dates(&self) -> StoreResult<Vec<NaiveDate>>;          // completed_at IS NOT NULL, ascending
// app_state
fn get_state(&self, key: &str) -> StoreResult<Option<String>>;
fn set_state(&self, key: &str, value: &str) -> StoreResult<()>;
fn delete_state(&self, key: &str) -> StoreResult<()>;
// xp + streak freezes
fn insert_xp(&self, date: NaiveDate, amount: i64, reason: &str, at: DateTime<Utc>) -> StoreResult<i64>;
fn xp_total(&self) -> StoreResult<i64>;
fn xp_for_date(&self, date: NaiveDate) -> StoreResult<i64>;
fn insert_streak_freeze(&self, date: NaiveDate) -> StoreResult<()>;             // idempotent
fn list_streak_freeze_dates(&self) -> StoreResult<Vec<NaiveDate>>;
// transactions
fn with_transaction<T>(&self, f: impl FnOnce(&Store) -> StoreResult<T>) -> StoreResult<T>; // commit on Ok, rollback on Err
pub(crate) fn conn(&self) -> &rusqlite::Connection;                            // for vocab-core modules only
```

### 6.3 Store behaviour consumers rely on

- **Forward compatibility of enum values:** decoding is strict — an unknown
  `script_kind`, `script_variant` or `origin` is `StoreError::Invalid`. Therefore
  adding a `ScriptMetadata` kind or a `CaptureOrigin` variant **must** come with a new
  migration (`user_version` bump), so an older build refuses the whole file with a
  clear message instead of failing row by row.
- Instants are truncated to milliseconds on write; `list_srs_due_before` rounds a
  sub-millisecond `before` up, so a card made due "now" is included. Callers comparing
  stored rows with in-memory values should use millisecond instants (`FixedClock` with
  whole milliseconds in tests). Years outside 0000–9999 are rejected.
- `with_transaction` is `IMMEDIATE` (takes the write lock up front, so the two apps
  never deadlock on read-then-write) and nests as a SAVEPOINT. **Never hold a
  transaction across a network call** (AnkiConnect, TTS): the other app's writes time
  out after 5 s.
- `set_synced_to_anki` and `update_capture_card` return `NotFound` for unknown ids;
  `find_capture_by_front` and `get_curriculum_item` match exactly (no trimming —
  `CurriculumIndex::lookup` trims); `review_log_for_card` orders by `reviewed_at`, id.
- `ensure_curriculum_seeded` also reseeds when the row count differs from the item
  count; duplicate `hanzi` or `curriculum_order` values make seeding fail, so the ETL
  must guarantee uniqueness.
- `list_recent_captures` returns `audio_front` BLOBs; IPC layers that only need text
  (history lists) should drop them before serializing.

### 6.4 Shared `app_state` keys and curriculum data

| key                                    | owner                | value                                                            |
| -------------------------------------- | -------------------- | ---------------------------------------------------------------- |
| `curriculum_fingerprint`               | store                | FNV-1a 64 of the seeded curriculum                               |
| `last_prompted_date`                   | srs (Learn)          | local date the daily prompt was last answered or dismissed       |
| `streak_freezes_available`             | gamification (Learn) | freezes in stock (0..=2)                                         |
| `learn.settings`                       | Learn                | `LearnSettings` JSON (`apps/learn/src/platform/types.ts`)        |
| `automator.unsynced_deck.<capture id>` | Automator            | the deck a profile capture belongs to, until it syncs            |
| `learn.lesson_completed.<lesson id>`   | Learn                | local date on which that lesson last earned `XP_LESSON_COMPLETE` |

Curriculum data: `crates/vocab-core/data/hsk_curriculum.json` (10,969 items,
HSK 3.0 levels 1–6 plus the 7–9 band), generated by `tools/hsk-import` from two
pinned MIT sources recorded with SHA-256 in `data/hsk_sources.json`; provenance,
licenses, the reading-selection rule and the regeneration steps are in
`crates/vocab-core/data/README.md`. `meaning_de` ships empty (MyMemory's German
was not good enough to ship unreviewed), so German learners see the English
meaning until reviewed German meanings are added to `data/hsk_meaning_de.json`.
`example_sentence_target` holds **English** translations (the apkg reuse pass).

## 7. SRS, quiz, gamification, lessons

### 7.1 `vocab_core::srs` (Milestone L)

```rust
#[serde(into = "u8", try_from = "u8")] pub enum ReviewRating { Again = 1, Hard = 2, Good = 3, Easy = 4 }
pub struct SrsScheduler { /* rs_fsrs::FSRS */ }       // Default = rs_fsrs::Parameters::default()
impl SrsScheduler {
    pub fn new(params: rs_fsrs::Parameters) -> Self;
    pub fn initial_state(&self, card_id: i64, now: DateTime<Utc>) -> SrsStateRow;      // fsrs_state "new", due now
    pub fn review(&self, state: &SrsStateRow, rating: ReviewRating, now: DateTime<Utc>, session_date: NaiveDate) -> ReviewOutcome;
    pub fn preview(&self, state: &SrsStateRow, now: DateTime<Utc>) -> Vec<RatingPreview>;  // due + interval per rating
}
pub struct ReviewOutcome { pub next: SrsStateRow, pub log: ReviewLogRow }
pub struct RatingPreview { pub rating: ReviewRating, pub due_date: DateTime<Utc>, pub interval_days: i64 }

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueItem { Review { card_id: i64, due_date: DateTime<Utc> }, New { hanzi: String, curriculum_order: i64 } }
pub fn build_daily_queue_from(due: &[SrsStateRow], curriculum: &[CurriculumItem], learned_hanzi: &HashSet<String>, new_quota: u32) -> Vec<QueueItem>;
pub fn build_daily_queue(store: &Store, new_quota: u32, clock: &dyn Clock) -> StoreResult<Vec<QueueItem>>;
pub fn introduce_new_item(store: &Store, scheduler: &SrsScheduler, hanzi: &str, target_lang: &str, clock: &dyn Clock) -> StoreResult<i64>;
pub fn submit_review(store: &Store, scheduler: &SrsScheduler, card_id: i64, rating: ReviewRating, clock: &dyn Clock) -> StoreResult<ReviewOutcome>;

// daily onboarding prompt (L2)
pub const LAST_PROMPTED_KEY: &str = "last_prompted_date";
pub fn should_prompt(last_prompted: Option<NaiveDate>, today: NaiveDate) -> bool;
pub struct DailyPromptState { pub needs_prompt: bool, pub today: NaiveDate, pub session: Option<DailySessionRow> }
pub fn daily_prompt_state(store: &Store, clock: &dyn Clock) -> StoreResult<DailyPromptState>;
pub fn submit_daily_quota(store: &Store, new_count: u32, clock: &dyn Clock) -> StoreResult<DailySessionRow>;
pub fn dismiss_daily_prompt(store: &Store, clock: &dyn Clock) -> StoreResult<DailySessionRow>;
pub fn reopen_daily_prompt(store: &Store, clock: &dyn Clock) -> StoreResult<()>;
pub enum SessionProgress { Review, NewItem }
pub fn record_session_progress(store: &Store, kind: SessionProgress, clock: &dyn Clock) -> StoreResult<DailySessionRow>;
```

- Queue: priority 1 = every `srs_state` with `due_date` before the end of the
  local day, ordered by `due_date` then `card_id`; priority 2 = if
  `new_quota > 0`, the first `new_quota` curriculum rows in `curriculum_order`
  whose hanzi is not in `learned_chinese_fronts()` (a Chinese capture **with an
  `srs_state` row**). `new_quota == 0` is the zero-length case of the same code
  path, not a separate branch. A word captured in Automator but never scheduled
  is therefore still offered when its turn comes (otherwise it would be neither
  new nor due, and never reach a review), and `introduce_new_item` adopts the
  existing capture instead of inserting a duplicate.
- `introduce_new_item` inserts a `captures` row (`origin = curriculum`,
  `front = hanzi`, `back = meaning_de` when `target_lang` is `de` and present,
  else `meaning_en`, curriculum example sentence/pinyin/HSK level copied in,
  `script_metadata = Chinese`) plus a `new` `srs_state` row, atomically, and
  returns the existing card id instead if that hanzi was already captured
  (introduced before, or captured in Automator — given a `new` state if it had
  none). The card's `target_lang` is the language `back` is actually in, and
  `example_target` is copied only for English cards: the curriculum's example
  translations are English (§ 6.4).
- `submit_review` loads the state, runs FSRS, writes the new state and a
  `review_log` row in one transaction. `review_log.scheduled_days` is the
  interval this review set. A review timed before the card's last review is
  taken to happen at that instant (a clock set back must not produce negative
  elapsed days or NaN stability); rows FSRS cannot interpret are `Invalid`.
- **Session progress is counted on reviews, not on introductions:** the caller
  records `SessionProgress::NewItem` when a card's review log has
  `state_before == "new"` (its first review), and `Review` otherwise. Introducing
  a word twice (double click, retry) therefore can never count twice.
- Daily prompt: `needs_prompt` is true when `last_prompted_date != today` (or
  unset). Submitting writes/updates today's `daily_sessions` row
  (`reviews_due_count` = queue reviews at that moment whose `fsrs_state` is not
  `new`: a due card that was never rated — a word introduced on an earlier day
  and left unrated — waits for its first review, which counts as a new item, so
  it is no review target) and sets `last_prompted_date = today`. **Dismiss without answering = quota 0
  (reviews only) for today**; `reopen_daily_prompt` clears the flag so the
  menu action can show the prompt again, and a later submit updates
  `requested_new_count` while keeping the progress counters.
- A session is complete (`completed_at` set) once `reviews_completed >=
reviews_due_count`, `new_items_completed >= requested_new_count` **and at
  least one review or new word was done**, whichever write gets there first
  (also `submit_daily_quota` lowering the quota). A day with nothing to do never
  completes: completion feeds the streak and the session XP bonus, which are
  tied to review work. `completed_at` is never cleared. Callers must call
  `gamification::on_session_completed` when a returned session row has just
  gained `completed_at`.

### 7.2 `vocab_core::quiz` (Milestone M)

```rust
pub struct PoolEntry {
    pub card_id: i64, pub front: String, pub back: String,
    pub pinyin: Option<String>, pub hsk_level: Option<u8>,
    pub source_lang: String, pub target_lang: String,
    pub example_source: String, pub example_target: String,
    pub script_kind: String, pub has_audio: bool,
}
pub struct LearnedVocabularyPool { /* entries */ }
impl LearnedVocabularyPool {
    pub const QUERY: &'static str;                       // captures INNER JOIN srs_state ON card_id
    pub fn load(store: &Store) -> StoreResult<Self>;      // THE only constructor outside #[cfg(test)]
    pub fn entries(&self) -> &[PoolEntry];
    pub fn get(&self, card_id: i64) -> Option<&PoolEntry>;
    pub fn learned_hanzi_chars(&self) -> HashSet<char>;   // every CJK char of every entry's front
}
#[serde(rename_all = "snake_case")] pub enum ExerciseKind { MultipleChoice, SentenceBuilder, ListeningChoice, ListeningSentence }
#[serde(tag = "kind", rename_all = "snake_case")] pub enum Exercise { MultipleChoice(..), SentenceBuilder(..), ListeningChoice(..), ListeningSentence(..) }
pub fn generate_exercise(pool: &LearnedVocabularyPool, card_id: i64, kind: ExerciseKind, seed: u32) -> Result<Exercise, QuizError>;
pub fn check_answer(exercise: &Exercise, answer: &ExerciseAnswer) -> bool;
```

Isolation guarantee (M1, the one correctness property the spec requires):
every option, distractor, tile and sentence shown in any exercise is derived
from `LearnedVocabularyPool::load` and nothing else. Concretely: multiple-choice
distractors are other pool entries; sentence-builder sentences are pool
entries' `example_source` values whose every CJK char is in
`learned_hanzi_chars()`; tiles come from segmenting that sentence by
greedy longest match against pool fronts only (falling back to single
characters); extra distractor tiles are other pool fronts. Unlearned curriculum
words are never used, not even as a segmentation dictionary (this supersedes
M2's "boundaries from hsk_curriculum/captures"). Randomness comes only from
`rng::Mulberry32::new(seed)`.

- **Visible text:** `learned_hanzi_chars` covers fronts only, so hanzi in a
  card's translation side are never shown unless they are learned too.
- **Options share one language:** every multiple-choice option is in the
  exercise's `option_lang` (the target's back language for front → back, its
  source language for back → front and pinyin). Distractors prefer the same
  language pair, then the same script; with too few candidates the exercise has
  fewer options, and below two it is `PoolTooSmall`.
- Payload shapes (`MultipleChoiceExercise`, `SentenceBuilderExercise`,
  `ListeningChoiceExercise`, `ListeningSentenceExercise`, `AudioPrompt`, `Tile`,
  `ChoiceOption`, `ExerciseAnswer`) are mirrored field for field in
  `apps/learn/src/platform/types.ts`.

### 7.3 `vocab_core::gamification` and `vocab_core::lessons` (Milestone I)

```rust
pub const XP_REVIEW: i64 = 10; pub const XP_NEW_ITEM: i64 = 15; pub const XP_SESSION_COMPLETE: i64 = 50;
pub const XP_EXERCISE_CORRECT: i64 = 5; pub const XP_LESSON_COMPLETE: i64 = 20;
pub const MAX_STREAK_FREEZES: u32 = 2;   // one earned per 7 consecutive completed days, held in app_state 'streak_freezes_available'
pub fn award_xp(store: &Store, amount: i64, reason: &str, clock: &dyn Clock) -> StoreResult<i64>;
pub fn compute_streak(completed: &BTreeSet<NaiveDate>, frozen: &BTreeSet<NaiveDate>, today: NaiveDate) -> (u32, u32); // (current, longest)
pub fn apply_streak_freezes(store: &Store, clock: &dyn Clock) -> StoreResult<Vec<NaiveDate>>;  // spends freezes on missed days
pub fn on_session_completed(store: &Store, clock: &dyn Clock) -> StoreResult<StreakSummary>;  // XP bonus + freeze earning
pub struct StreakSummary { pub current: u32, pub longest: u32, pub freezes_available: u32, pub completed_today: bool }
pub struct StatsSummary { pub xp_total: i64, pub xp_today: i64, pub streak: StreakSummary, pub learned_count: i64, pub due_today: i64 }
pub fn stats_summary(store: &Store, clock: &dyn Clock) -> StoreResult<StatsSummary>;

pub struct LessonCard { pub card_id: i64, pub word: String, pub translation: String, pub sentence_source: String,
    pub sentence_target: String, pub cloze: String, pub pinyin: Option<String> }
pub struct Lesson { pub id: String, pub title: String, pub source_lang: String, pub target_lang: String, pub cards: Vec<LessonCard> }
pub fn build_lessons(captures: &[StoredCapture], cards_per_lesson: usize) -> Vec<Lesson>;
pub fn lessons_from_store(store: &Store, cards_per_lesson: usize) -> StoreResult<Vec<Lesson>>;
```

A streak counts consecutive local dates ending today (or yesterday, if today
is not completed yet) that have a completed daily session or a freeze. XP and
streaks are tied to review sessions, not to raw captures. Freezes are spent
lazily, whenever the app asks (`apply_streak_freezes`, which `stats_summary` and
`on_session_completed` run first): the missed days after the last completed or
frozen day, up to yesterday, are frozen — all of them or none, when the
inventory does not cover the gap; never today, and never before the first
completed day. A frozen day counts toward the streak length but is no completed
day (`completed_today` stays false and it earns no bonus). One freeze is earned
each time the current streak reaches a multiple of `STREAK_FREEZE_INTERVAL` (7),
capped at `MAX_STREAK_FREEZES`, inventory in `app_state` `STREAK_FREEZES_KEY`. XP reasons
are the `gamification::reason::*` constants. `Lesson.title` is an English
fallback; UIs build a localized title from the languages and the lesson's index.
Lessons use only
`origin = capture` rows whose `example_source` contains the word (so the cloze
has something to blank) and whose `example_target` is not blank, grouped by
language pair in capture order.

## 8. Automator (`src-tauri`, package `vocab-automator`)

### 8.1 Config (`config.json` in the app data dir)

```json
{
  "ui_lang": "en",
  "hotkeys": "CommandOrControl+Shift+C",
  "hotkey_profiles": [
    {
      "id": "zh-de",
      "label": "Chinese → German",
      "shortcut": "CommandOrControl+Shift+H",
      "source_lang": "zh-CN",
      "target_lang": "de",
      "deck_name": null
    }
  ],
  "translation": {
    "source_lang": "en",
    "target_lang": "de",
    "max_chars": 100,
    "provider": "mymemory",
    "contact_email": null
  },
  "anki": {
    "connect_url": "http://127.0.0.1:8765",
    "deck_name": "English::AutomatedVocabulary",
    "model_name": "AppleStyleFinalV3",
    "create_reverse": true,
    "field_mapping": {
      "fields": [{ "anki_field": "Front", "source": "front" }],
      "audio_field": "Front Audio"
    }
  },
  "consent": { "anki_install_granted_at": null, "ankiconnect_provision_granted_at": null }
}
```

- `hotkeys` is the default profile (uses `translation.*` languages and
  `anki.deck_name`); `hotkey_profiles` add more shortcuts with their own pair
  and optional deck (Milestone H). All shortcuts must parse and be distinct.
- Loading a legacy file maps the old `deepl` section onto `translation`
  (`api_key` is dropped), lowercases language codes and fills every new field
  with its default. `load_config_from(path)`/`save_config_to(path, cfg)` are
  the path-taking core; the `AppHandle` wrappers only resolve the path.

### 8.2 Commands (`invoke` names; JS passes args in camelCase)

| command                       | args                                                | returns                                                                                                 |
| ----------------------------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `get_current_settings`        | —                                                   | `AppConfig`                                                                                             |
| `apply_settings`              | `newCfg: AppConfig`                                 | `null` (re-registers shortcuts; validation errors as `string`)                                          |
| `get_pending_payload`         | —                                                   | `PendingCapture \| null`                                                                                |
| `discard_pending_payload`     | —                                                   | `null` (clears state, hides `preview`)                                                                  |
| `execute_card_generation`     | `card: Card`                                        | `SubmitResult`                                                                                          |
| `capture_text`                | `text: string, profileId: string \| null`           | `null` (runs the pipeline as if the hotkey had copied `text`; the internal capture event for tests/E2E) |
| `anki_status`                 | —                                                   | `AnkiStatus`                                                                                            |
| `anki_model_field_names`      | `model: string`                                     | `string[]`                                                                                              |
| `launch_anki_desktop`         | —                                                   | `string`                                                                                                |
| `get_setup_status`            | —                                                   | `SetupStatus`                                                                                           |
| `grant_consent`               | `action: "anki_install" \| "ankiconnect_provision"` | `ConsentState`                                                                                          |
| `download_and_install_anki`   | —                                                   | `string` (error unless consent granted)                                                                 |
| `auto_provision_ankiconnect`  | —                                                   | `string` (error unless consent granted)                                                                 |
| `check_ankiconnect_installed` | —                                                   | `boolean`                                                                                               |
| `list_recent_captures`        | `limit: number`                                     | `StoredCapture[]`                                                                                       |
| `delete_capture`              | `id: number`                                        | `boolean`                                                                                               |
| `sync_pending_captures`       | —                                                   | `SyncReport`                                                                                            |
| `capture_in_progress`         | —                                                   | `boolean`                                                                                               |
| `restart_app`                 | —                                                   | never returns                                                                                           |

Events: `translation-payload` (`PendingCapture`, emitted to the `preview`
window), `capture-error` (`{ message: string }`, app-wide),
`captures-changed` (no payload, app-wide, after a save/delete/sync).

`execute_card_generation(card)`: (1) saves the card to SQLite first, always;
(2) maps it with `anki-bridge` (forward note, plus the reverse note when
`create_reverse`), synthesizing reverse-side audio in `target_lang`;
(3) pushes to AnkiConnect; (4) marks the capture synced on success (an Anki
"duplicate" error also counts as synced). Anki being offline is not an error:
the result says `synced_to_anki: false` with `anki_error`, and
`sync_pending_captures` retries later. If `card.front` differs from the pending
capture's front, TTS is re-run (and a Chinese front re-annotated) for the new text.
On success the pending capture is cleared; hiding the window is left to the
frontend so it can show the result first. `capture_id` is `0` in Anki-only mode
(no store; SQLite never assigns id 0).

Further rules both sides rely on:

- **`apply_settings` ignores `newCfg.consent`**: consent is recorded only by
  `grant_consent`, so no webview script can forge it through a settings save.
- `capture_in_progress` is true from the hotkey until the capture is approved or
  discarded (the approval window counts); closing the preview counts as a discard.
- `max_chars` must be a whole number in `1..=500`; a profile's source and target
  language must differ; all shortcuts must parse and be distinct.
- `sync_pending_captures` pushes `origin = capture` rows only, stops at the first
  "Anki unreachable" error, and uses the deck remembered for profile captures
  (`app_state` `automator.unsynced_deck.<id>`).
- `launch_anki_desktop` waits for `open -a Anki` on macOS and reports a missing
  Anki instead of "Starting Anki…".

### 8.3 Windows and capabilities

`main` (settings, history; label `main`) and `preview` (approval window,
created on demand, `always_on_top`, hidden — not destroyed — after approve or
discard). Both load the same Vue bundle; `App.vue` branches on the window
label. Every webview call the preview makes (event listen, window hide/close)
has a matching permission in `src-tauri/capabilities/` for the `preview` label.

### 8.4 Consent & supply chain (Milestone E)

- Startup only _checks_ (`check_anki_installed`, `check_ankiconnect_installed`);
  nothing is downloaded, installed or launched without a user click.
- Each install action requires a one-time consent recorded in
  `config.consent.*_granted_at` via `grant_consent`; the UI shows a consent
  dialog that explains exactly what will be downloaded, from where, and what
  will be executed or written.
- AnkiConnect is pinned to `FooSoft/anki-connect` commit
  `4064fa142785975255457abd6a496015f5b71f38` (the repository's final, archived
  commit). Files from `plugin/` with their SHA-256:

  | file          | sha256                                                             |
  | ------------- | ------------------------------------------------------------------ |
  | `__init__.py` | `2185b1243b924152cbac7a7668b40c0bf713902619bfc8ede5c42465066d0f05` |
  | `edit.py`     | `4c6659247322be76c3015b8f8944ce20498186782880a3c8ff14a48d2e3f58b2` |
  | `util.py`     | `cc0e12de6f809d57d40b29f3eb5f0db1eadad93e19f35fc63a2cfe250216c3b9` |
  | `web.py`      | `b519b4c6fefa3d2058df926c43ade1cf590d595655ee3b20afdf7ccd2a053a78` |

  All four are downloaded into a temp dir and verified before anything is
  written to `addons21/2055492159/`. (The pre-refactor code fetched
  `master/ankiconnect.py`, which is a 404: it never worked.)

- The `config.json` written for AnkiConnect binds `127.0.0.1:8765` and sets
  `webCorsOriginList` to the app's own webview origin only — `tauri://localhost`
  on macOS/Linux, `http://tauri.localhost` on Windows, plus
  `http://localhost:1420` in debug builds. (Requests from the Rust backend send
  no `Origin` header and are unaffected; the frontend no longer calls
  AnkiConnect directly.)
- Anki is pinned to 24.06.3 with the SHA-256 values from the release's signed
  `anki-24.06.3-checksums.txt`:

  | platform       | file                             | sha256                                                             |
  | -------------- | -------------------------------- | ------------------------------------------------------------------ |
  | macOS arm64    | `anki-24.06.3-mac-apple-qt6.dmg` | `ce2f728c98ab3711450fbc066b8fbe27e983684e5d6195be5f451475d1a48d35` |
  | macOS x86_64   | `anki-24.06.3-mac-intel-qt6.dmg` | `51045da7f1a189119fb0117e995ea108aa4440616ffc8f43f4a4ed735a7befaa` |
  | Windows x86_64 | `anki-24.06.3-windows-qt6.exe`   | `e11ade8b94dbb5374102669ea589bc1072ea07b93897f3b36fa045f5192ef0ba` |

  The installer is verified before it is mounted or executed; a mismatch
  deletes the download and fails. Linux is unsupported (distribution packages).

## 9. `anki-bridge`

```rust
pub struct AnkiClient { /* reqwest + url */ }
impl AnkiClient {
    pub fn new(url: impl Into<String>) -> Self;
    pub async fn version(&self) -> Result<u32, AnkiError>;
    pub async fn deck_names(&self) -> Result<Vec<String>, AnkiError>;
    pub async fn model_names(&self) -> Result<Vec<String>, AnkiError>;
    pub async fn model_field_names(&self, model: &str) -> Result<Vec<String>, AnkiError>;
    pub async fn create_deck(&self, deck: &str) -> Result<i64, AnkiError>;
    pub async fn add_note(&self, note: &NoteSpec) -> Result<i64, AnkiError>;
    pub async fn find_notes(&self, query: &str) -> Result<Vec<i64>, AnkiError>;
    pub async fn notes_info(&self, ids: &[i64]) -> Result<Vec<NoteInfo>, AnkiError>;
}
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnkiError { Unreachable { message: String }, Api { message: String }, Duplicate { message: String }, Malformed { message: String } }

#[serde(rename_all = "snake_case")]
pub enum CardField { Front, Back, ExampleSource, ExampleTarget, Example, Notes, Pinyin, HskLevel }
pub struct FieldBinding { pub anki_field: String, pub source: CardField }
pub struct FieldMapping { pub fields: Vec<FieldBinding>, pub audio_field: Option<String> }
impl FieldMapping {
    pub fn apple_style_default() -> Self; // Front->Front, Back->Back, Example->"Context Sentence", Notes->Notes, audio "Front Audio"
    pub fn chinese_default() -> Self;     // Front->Hanzi, Pinyin->Pinyin, Back->Meaning, Example->Sentence, audio "Audio"
}
impl Default for FieldMapping { /* apple_style_default */ }
pub enum Direction { Forward, Reverse }
pub struct AnkiTarget { pub deck: String, pub model: String }
pub struct NoteSpec { pub deck: String, pub model: String, pub fields: BTreeMap<String, String>,
    pub tags: Vec<String>, pub audio: Option<AudioAttachment> }
pub struct AudioAttachment { pub filename: String, pub data_b64: String, pub fields: Vec<String> }
pub fn map_card(card: &Card, direction: Direction, target: &AnkiTarget, mapping: &FieldMapping, audio: Option<&[u8]>) -> NoteSpec;
pub fn add_note_params(note: &NoteSpec) -> serde_json::Value;     // exactly AnkiConnect's addNote "params"
pub fn card_from_note(note: &NoteInfo, mapping: &FieldMapping, source_lang: &str, target_lang: &str, captured_at: DateTime<Utc>) -> Option<Card>;
```

Shared push helper (both apps use it; never re-implement the sequence):

```rust
#[derive(Default)] pub struct PushOptions { pub create_reverse: bool, pub reverse_audio: Option<Vec<u8>> }
pub struct PushOutcome { pub note_ids: Vec<i64>, pub duplicates: usize }
pub async fn push_card(client: &AnkiClient, card: &Card, target: &AnkiTarget,
    mapping: &FieldMapping, options: &PushOptions) -> Result<PushOutcome, AnkiError>;
pub struct NoteInfo { pub note_id: i64, pub model_name: String, pub tags: Vec<String>,
    pub fields: BTreeMap<String, NoteField>, pub cards: Vec<i64>, pub modified: Option<i64> } // serde camelCase, "mod"
pub struct NoteField { pub value: String, pub order: u32 }
pub fn deck_query(deck: &str) -> String;           // "deck:NAME" with Anki search escaping
pub const DEFAULT_URL: &str = "http://127.0.0.1:8765";  pub const API_VERSION: u32 = 6;
pub const DUPLICATE_NOTE_MESSAGE: &str = "cannot create note because it is a duplicate";
// AnkiClient::with_timeout(Duration), url(), timeout(); AnkiError::message(), is_duplicate(), is_unreachable()
```

Semantics:

- `push_card`: `createDeck` (an "already exists" error is ignored), then the forward
  note with `card.audio_front`, then — only when `create_reverse` is set **and
  `card.back` is not blank** — the reverse note with `reverse_audio`. A `Duplicate`
  on either note is counted, not returned; any other error is returned at once (a
  failed reverse note after a successful forward note is an `Err`, and a later retry
  sees the forward note as a duplicate). `Ok` means the card counts as synced.
- Only AnkiConnect's exact `DUPLICATE_NOTE_MESSAGE` is `Duplicate`; anything else
  (e.g. a missing deck whose user-chosen name contains "duplicate") is `Api`.
- `map_card(Reverse)` swaps front/back and example source/target; the `Example`
  field is the combined example in display order.
- Field values are HTML-escaped (`&`, `<`, `>`) and line breaks are written as
  `<br>` (a raw newline renders as a space in Anki's HTML fields, which put both
  example sentences on one line). `card_from_note` reverses this (tags stripped,
  entities decoded once, `<br>`/`<div>` → newline, `[sound:...]` dropped).
- When several bindings name the same Anki field (compared case-insensitively, as
  AnkiConnect does), the first wins, on export and on import.
- Audio is sent inline (`"audio": [{ "data", "filename", "fields" }]`) only for
  non-empty bytes and a non-blank audio field — no separate `storeMediaFile`, no
  second download. The `addNote` payload uses `allowDuplicate: false,
duplicateScope: "deck"` and the card's tags (Anki splits tags on whitespace).
- `notes_info` sends 500 ids per request, skips AnkiConnect's `{}` entries for deleted
  notes, and sends nothing for an empty slice. The client never uses a proxy.

## 10. Learn (`apps/learn`)

Desktop app (Tauri 2, crate `vocab-learn`, dev server :1430) over the same
SQLite file, plus a standalone web build of the same frontend. Every
Tauri-specific call sits behind `PlatformAdapter`
(`apps/learn/src/platform/types.ts`) with two implementations: `tauri`
(IPC to the commands below) and `web` (IndexedDB + a TypeScript port of the
core logic, verified against golden vectors generated from the Rust core).

| command                  | args                                                   | returns                       |
| ------------------------ | ------------------------------------------------------ | ----------------------------- |
| `get_daily_prompt_state` | —                                                      | `DailyPromptState`            |
| `submit_daily_quota`     | `newCount: number`                                     | `DailySession`                |
| `dismiss_daily_prompt`   | —                                                      | `DailySession`                |
| `reopen_daily_prompt`    | —                                                      | `null`                        |
| `get_daily_queue`        | —                                                      | `QueueItem[]`                 |
| `start_new_item`         | `hanzi: string`                                        | `number` (card id)            |
| `get_card`               | `cardId: number`                                       | `StoredCapture`               |
| `preview_ratings`        | `cardId: number`                                       | `RatingPreview[]`             |
| `submit_review_rating`   | `cardId: number, rating: 1\|2\|3\|4`                   | `ReviewResult`                |
| `list_learned_cards`     | —                                                      | `LearnedCard[]` (the pool)    |
| `next_exercise`          | `cardId: number, kind: ExerciseKind, seed: number`     | `Exercise`                    |
| `check_exercise`         | `exercise: Exercise, answer: ExerciseAnswer`           | `boolean`                     |
| `record_exercise_result` | `cardId: number, kind: ExerciseKind, correct: boolean` | `number` (XP awarded)         |
| `get_stats`              | —                                                      | `StatsSummary`                |
| `list_lessons`           | —                                                      | `Lesson[]`                    |
| `complete_lesson`        | `lessonId: string`                                     | `number` (XP awarded)         |
| `get_card_audio`         | `cardId: number`                                       | `string \| null` (base64 MP3) |
| `synthesize_audio`       | `text: string, lang: string`                           | `string \| null` (base64 MP3) |
| `get_settings`           | —                                                      | `LearnSettings`               |
| `save_settings`          | `settings: LearnSettings`                              | `null`                        |
| `anki_export`            | — (deck/model/mapping from settings)                   | `AnkiSyncReport`              |
| `anki_import`            | `deck: string`                                         | `AnkiSyncReport`              |

Semantics both adapters implement identically (the web adapter's golden `adapter`
scenario pins them against the desktop behaviour):

- `get_daily_queue`: reviews due before the end of the local day, then
  `new_quota = requested_new_count − new_items_completed − (due cards whose fsrs_state
is "new")`, floored at 0, and 0 while today's session does not exist. Introducing a
  word moves it from the new part to the review part instead of pulling in another.
- `submit_review_rating`: one transaction runs `submit_review`, awards `XP_NEW_ITEM`
  for a card's first review (`state_before == "new"`) and `XP_REVIEW` for any other,
  records the matching `SessionProgress`, and calls `on_session_completed` when the
  session has just completed (its bonus is included in `xp_awarded`).
- `start_new_item` trims the hanzi, is idempotent and records no progress;
  `record_exercise_result` rejects cards without an `srs_state` row and pays
  `XP_EXERCISE_CORRECT` for correct answers only; lessons use `CARDS_PER_LESSON = 5`
  (lesson ids depend on it) and pay `XP_LESSON_COMPLETE` once per lesson per local day.
- `anki_export` pushes unsynced rows whose origin is not `capture` (Automator's
  `sync_pending_captures` owns those) with the deck, note type, mapping and reverse
  setting from `LearnSettings`; it stops at the first "unreachable" and counts an
  all-duplicate card as skipped and synced. `anki_import` reads Chinese vocabulary from a
  deck, swaps reverse notes back, skips fronts already stored (any origin), and stores
  rows as `origin = anki_import`, synced.
- `LearnSettings` defaults: `meaning_lang` `en`; Anki `http://127.0.0.1:8765`, deck
  `Chinese::HSK`, note type `Chinese (HSK)`, `FieldMapping::chinese_default()`,
  `create_reverse` false. Loading is lenient (bad stored values fall back to defaults);
  saving validates (http(s) URL with a host, non-blank deck/model, named bindings).

`apps/learn/src/platform/types.ts` is the TypeScript side of this table and holds
every argument and result shape (`ReviewResult`, `LearnedCard`, `LearnSettings`,
`AnkiSyncReport`, `PlatformCapabilities`, ...). The web adapter rejects the TTS and
Anki methods with a clear message and reports `tts: false, anki: false`.

## 11. Serialized shapes (TypeScript)

```ts
type ScriptMetadata =
  | { kind: 'none' }
  | {
      kind: 'chinese'
      pinyin: string
      hsk_level: number | null
      script_variant: 'simplified' | 'traditional'
    }
interface Card {
  front: string
  back: string
  example_source: string
  example_target: string
  notes: string
  tags: string[]
  source_lang: string
  target_lang: string
  captured_at: string
  audio_front: string | null // base64 MP3
  script_metadata: ScriptMetadata
}
type ProviderError =
  | { kind: 'network' | 'quota_exceeded' | 'malformed'; provider: string; message: string }
  | { kind: 'http'; provider: string; status: number; message: string }
  | { kind: 'not_found'; provider: string }
  | { kind: 'unsupported'; provider: string; what: string }
interface PipelineWarning {
  stage: 'translate' | 'rarity' | 'example' | 'example_translate' | 'tts' | 'annotate'
  error: ProviderError
}
interface PendingCapture {
  card: Card
  warnings: PipelineWarning[]
  profile_id: string | null
}
interface StoredCapture {
  id: number
  card: Card
  origin: 'capture' | 'curriculum' | 'anki_import'
  synced_to_anki: boolean
}
interface SubmitResult {
  capture_id: number
  synced_to_anki: boolean
  note_ids: number[]
  anki_error: string | null
}
interface SyncReport {
  attempted: number
  synced: number
  failed: { id: number; error: string }[]
}
interface AnkiStatus {
  online: boolean
  version: number | null
  decks: string[]
  models: string[]
  error: string | null
}
interface ConsentState {
  anki_install_granted_at: string | null
  ankiconnect_provision_granted_at: string | null
}
interface SetupStatus {
  anki_installed: boolean
  ankiconnect_installed: boolean
  platform_supported: boolean
  anki_version: string
  consent: ConsentState
}
type QueueItem =
  | { kind: 'review'; card_id: number; due_date: string }
  | { kind: 'new'; hanzi: string; curriculum_order: number }
```

## 12. Deviations from IMPLEMENTATION_PLAN.md (and why)

1. `anki/mapper.rs` lives in `crates/anki-bridge/src/mapper.rs`, not
   `src-tauri/src/anki/mapper.rs`: Learn's Anki export/import (Milestone I)
   needs the same mapping, and a shared crate keeps it the single place Anki
   field names exist (decision 4) without making `vocab-core` Anki-aware.
2. The `CardStore`/SQLite store lives in `vocab-core::store`, not
   `src-tauri/src/store`: Learn's FSRS scheduler runs over the same store
   (Milestone I, "FSRS scheduler integration operating over the local SQLite
   store from Milestone F"), and both desktop apps open one shared file.
3. `srs_state` adds `lapses`, `fsrs_state` and `elapsed_days`, and `due_date`
   holds a full RFC 3339 instant: FSRS needs all of these to round-trip a card
   (rs-fsrs `Card`), and learning steps are minutes, not days.
4. `captures` adds `script_kind`, `script_variant`, `origin` and `audio_front`:
   L3's pseudo-code filters on `script_kind`, Learn needs to know which rows came
   from the curriculum, and M2's listening exercises reuse the card's TTS audio.
5. The HSK dataset publishes levels 1–6 plus one combined "7–9" band
   (`wordlists/exclusive/new/7.json`), so `hsk_level` is 1–7, with 7 meaning 7–9.
6. The daily prompt and queue run in Learn's main window (decision 10 moved the
   review/quiz surface to Learn); L2's "main-window mount" is Learn's main window.
7. Learn's web build uses a TypeScript port of the scheduling/quiz logic
   (golden-vector parity tests against the Rust core) because compiling the
   core to WASM needed a toolchain change the operator did not approve.
8. HSK data (Milestone J2): the plan's "take the first `forms[]` entry" would
   teach wrong readings (吧 bā "bar" instead of the particle ba), so the ETL picks
   the form matching the official HSK 3.0 reading (`ivankra/hsk30`), then the
   official reading up to neutral tones, then the `pinyin` crate's default, then
   the first undominated form; it also repairs CC-CEDICT leftovers (`u:`, tone 5).
   `meaning_de` is not bundled (see § 6.4).
9. The daily queue skips _learned_ hanzi (Chinese captures with `srs_state`), not
   every Chinese capture as L3's pseudo-code does, so an Automator capture that was
   never scheduled is adopted when its curriculum turn comes instead of being
   stranded (§ 7.1).
