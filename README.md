# vocabulary-anki-automator

Two apps for learning vocabulary, built with Tauri 2 (Rust + Vue 3) on one shared
Rust core:

- **Automator** turns any text you select into a flashcard: press a hotkey, check
  the generated card, and it lands in Anki and in a local history.
- **Learn** teaches the HSK 3.0 word list with FSRS-scheduled reviews, a daily
  goal, exercises, lessons made from your captures, XP and streaks.

Both desktop apps keep their data in the same SQLite file: Learn builds its
lessons from Automator's captures, and a captured HSK word joins the review
schedule when the daily queue reaches it.

## What is in this repository

| Path                 | What it is                                                                                      |
| -------------------- | ----------------------------------------------------------------------------------------------- |
| `src-tauri/`, `src/` | **Automator**: the Tauri crate `vocab-automator` and its Vue frontend (desktop only)            |
| `apps/learn/`        | **Learn**: the Tauri crate `vocab-learn` (`src-tauri/`) and its Vue frontend (`src/`)           |
| `crates/vocab-core`  | Shared core: card model, providers, capture pipeline, SQLite store, FSRS, quizzes, gamification |
| `crates/anki-bridge` | AnkiConnect client and the card-to-note mapping (the only place Anki field names exist)         |
| `tools/hsk-import`   | One-time ETL that builds the bundled HSK curriculum (`crates/vocab-core/data/`)                 |
| `docs/`              | Architecture contract, test-harness strategy, signing-key custody                               |
| `scripts/`           | macOS bootstrap, the shared check script, the pre-commit hook                                   |

## The apps

### Automator (macOS, Windows, Linux)

1. Select a word or phrase in any app and press the hotkey
   (default `CommandOrControl+Shift+C`; hotkey profiles add more shortcuts, each
   with its own language pair and deck).
2. The capture pipeline builds a card: translation with
   [MyMemory](https://mymemory.translated.net/) (free, no key; an optional contact
   e-mail raises the daily quota), a rarity check and synonym (Datamuse, English),
   an example sentence (dictionaryapi.dev for English, the bundled curriculum for
   Chinese) and pronunciation audio (Google Translate TTS). Chinese text gets pinyin
   and its HSK level. A failing provider leaves its field empty and shows a warning;
   it never loses the capture.
3. The approval window shows the card for editing. **Approve** saves it to the
   local history first and then sends it to Anki through the AnkiConnect add-on
   (optionally with a reverse card). If Anki is not running, the card stays in the
   history and **Sync pending to Anki** sends it later.

The settings window covers languages, hotkeys and profiles, deck, note type and
field mapping, and the UI language (English or German). It can also install Anki
(macOS, Windows) and AnkiConnect for you, but only after you agree to it in a
consent dialog; both downloads are pinned and checksum-verified before anything
is installed.

On macOS the hotkey copies the selection with a simulated `Cmd+C`, which needs
the Accessibility permission (System Settings > Privacy & Security >
Accessibility).

### Learn (macOS, Windows, Linux, and the web)

- **Daily goal**: on the first start of a day Learn asks how many new words you
  want to learn. "Not today" means reviews only; the menu reopens the question.
- **Review**: cards that are due come first, then new words: first those you
  captured in the Automator or imported from Anki and have not started yet,
  oldest first, then the HSK 3.0 list in order (captured HSK words keep their
  place in it). Each card is rated Again / Hard / Good / Easy and rescheduled
  with FSRS (`rs-fsrs`); the buttons show when the card would come back.
- **Practice**: multiple choice, sentence builder, and two listening exercises,
  built only from words you have already learned.
- **Lessons**: cloze lessons from the example sentences of your own captures.
- **Stats**: XP, the daily streak and streak freezes.
- **Anki & settings**: the meaning language of new words, and export to and
  import from Anki through AnkiConnect, with the deck, note type, field mapping
  and AnkiConnect address of your choice (desktop only).

The **web build** is the same frontend running in a browser: it stores everything
in IndexedDB and runs a TypeScript port of the core, held to the Rust code by
golden test vectors. It has no text-to-speech and no Anki access, and its data
stays in that browser. iOS and Android builds are not set up yet
(see [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md), Milestone I).

The **live demo** is that web build with ten days of made-up practice already
done, a few captured words among them, and a bar to move its clock a day ahead
or start it over: a way to show Learn to people who do not have it. It builds
to a folder of static files that any web host can serve, and that a web page
can embed ([docs/live-demo.md](docs/live-demo.md)).

### Where the data lives

- Cards, reviews, XP and Learn's settings: `VocabularyAutomator/vocab.sqlite3` in
  the platform data directory (`~/Library/Application Support` on macOS,
  `%APPDATA%` on Windows, `~/.local/share` on Linux). Set `VOCAB_DB_PATH` to use
  another file.
- Automator's settings: `config.json` in its app data directory
  (`com.ankiautomator.app`).

## Getting started

Requirements: Rust (stable), Node.js 22 with npm, and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your
platform (on Linux the packages `.github/workflows/ci.yml` installs). The Anki
features need [Anki](https://apps.ankiweb.net/) with the AnkiConnect add-on,
running on the same machine.

```bash
npm ci                        # frontend dependencies (Vite, Vue, Vitest, ESLint, Prettier, Tauri CLI)
./scripts/bootstrap-macos.sh  # macOS dev machine: cargo-watch, just, gh, watchman, rtk, pre-commit hook
```

### Run in development

```bash
npm run tauri dev          # Automator (frontend on http://localhost:1420)
npm run learn:tauri dev    # Learn, desktop app (frontend on http://localhost:1430)
npm run learn:dev:web      # Learn, web build in your browser at http://localhost:1430
npm run learn:dev:demo     # Learn, the live demo in your browser at http://localhost:1430
```

With [just](https://github.com/casey/just): `just dev`, `just learn-dev`,
`just learn-web`, `just learn-demo`. (`npm run learn:dev` alone serves the desktop frontend that
`tauri dev` loads; opened in a plain browser, its calls to the Tauri backend fail.)

### Build

```bash
npm run learn:build:web                # Learn web build -> apps/learn/dist-web (static files, host anywhere)
npm run learn:build:demo               # Learn live demo -> apps/learn/dist-demo (static files, host anywhere)
npm run learn:tauri build              # Learn desktop bundles -> target/release/bundle
npm run tauri build -- --no-sign      # Automator bundles for local testing, unsigned -> target/release/bundle
```

Automator's real releases are built, signed and published by
`.github/workflows/release.yml` when the operator pushes a `v*` tag that matches
`src-tauri/tauri.conf.json`'s version. Because Automator ships with the updater,
a plain `npm run tauri build` also signs updater artifacts and needs the updater
key (`TAURI_SIGNING_PRIVATE_KEY`), which only CI holds
([docs/signing-key-custody.md](docs/signing-key-custody.md)); `--no-sign` above
skips all signing (codesign and updater signatures) for a local build, and
`-- --no-bundle` only compiles the app.

### Test and lint

```bash
just lint        # cargo fmt --check, clippy -D warnings, vue-tsc (both apps), ESLint, Prettier
just check-all   # scripts/run-all-checks.sh: Rust unit, integration and doc tests,
                 # the Rust/TypeScript golden-vector check, Vitest for both frontends
```

Or piece by piece:

```bash
cargo test --workspace
npx vitest run                                      # Automator frontend
npx vitest run --config apps/learn/vitest.config.ts # Learn frontend, including the web core
```

No test reaches the network or a real Anki install: HTTP providers and
AnkiConnect are replaced by `wiremock` servers on `127.0.0.1` (so the tests need
local port binding), and the install commands run against fakes
([docs/test-harness-strategy.md](docs/test-harness-strategy.md)). CI
(`.github/workflows/ci.yml`) runs the lint on Linux, and the tests and the
production frontend builds on Linux, macOS and Windows.

### The HSK curriculum

`crates/vocab-core/data/hsk_curriculum.json` (10,969 words, HSK 3.0 levels 1–6
plus the 7–9 band) is generated from two pinned, MIT-licensed upstream lists by
`tools/hsk-import` (`just hsk-import --help`). Its dictionary data (English
meanings, traditional forms, pinyin) comes from CC-CEDICT, which is licensed
CC BY-SA 4.0, not MIT ([NOTICE-CC-CEDICT](crates/vocab-core/data/NOTICE-CC-CEDICT)).
Provenance, licenses, known gaps (no German meanings or example sentences yet)
and how to regenerate it are in
[crates/vocab-core/data/README.md](crates/vocab-core/data/README.md).

## Documentation

| Read                                                                 | For                                                                 |
| -------------------------------------------------------------------- | ------------------------------------------------------------------- |
| [docs/architecture.md](docs/architecture.md)                         | Module map, cross-crate APIs, database schema, IPC commands, shapes |
| [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md)                     | Roadmap and milestone status, including what is still open and why  |
| [docs/test-harness-strategy.md](docs/test-harness-strategy.md)       | How tests avoid real Anki and the network                           |
| [docs/signing-key-custody.md](docs/signing-key-custody.md)           | Release secrets and the updater signing key                         |
| [docs/live-demo.md](docs/live-demo.md)                               | The public live demo: how it works, hosting it, embedding it        |
| [crates/vocab-core/data/README.md](crates/vocab-core/data/README.md) | HSK data provenance, licenses and regeneration                      |
| [CLAUDE.md](CLAUDE.md)                                               | Working rules for coding agents in this repository                  |
