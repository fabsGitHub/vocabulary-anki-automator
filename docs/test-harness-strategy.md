# Test harness strategy (Milestone A4)

Decision record for how automated tests (unit/integration/E2E, Milestone G)
exercise Anki/AnkiConnect-facing code without needing a real Anki install in
CI or on a contributor's machine, and where that decision now lives in the
code.

## Chosen approach: mock AnkiConnect server, not a disposable Anki profile

Two options were on the table:

1. A temp `ANKI_BASE` directory the E2E harness points a real, disposable
   Anki instance at.
2. A lightweight mock AnkiConnect server (`wiremock`-based Rust binary, or a
   small Node/`http` stub) that records/asserts `addNote` payloads without
   needing real Anki at all.

**Decision: option 2, a mock AnkiConnect stub.** Reasons:

- Anki itself is a GUI desktop app with no documented headless/CI mode;
  scripting a temp `ANKI_BASE` still means launching and driving the real
  GUI, which is slow and brittle for anything faster than the nightly E2E
  tier.
- What this codebase actually needs to verify is _the shape of the HTTP
  requests it sends to AnkiConnect_ (`addNote` payload, deck/model names,
  field mapping produced by `crates/anki-bridge/src/mapper.rs`) — a mock
  server that records and asserts on incoming requests tests exactly that,
  directly, without a GUI dependency.
- It works identically in a contributor's sandbox and in CI — no macOS-only
  GUI automation required for the integration tier.

### Where it is implemented (Milestone G)

- `crates/anki-bridge/tests/mock_ankiconnect.rs` asserts the exact JSON
  bodies the client sends (`addNote`, `createDeck`, `findNotes`,
  `notesInfo`, ...) to a `wiremock` AnkiConnect on `127.0.0.1`;
  `tests/no_proxy.rs` checks that AnkiConnect is never reached through a proxy.
- `src-tauri/tests/ipc_anki.rs` (Automator) and
  `apps/learn/src-tauri/tests/ipc_anki.rs` (Learn) drive the apps' own IPC
  commands through `tauri::test` against the same kind of mock.
- Provider HTTP (MyMemory, Datamuse, dictionaryapi.dev, Google TTS) is mocked
  the same way in `crates/vocab-core/tests/providers_http.rs`.

The `wiremock` suites start servers on `127.0.0.1`, so they need local port
binding (docs/architecture.md § 2); they never contact anything else.

The E2E tier (`tauri-driver` + WebdriverIO against the compiled app) is not
built: it needs dependencies that have not been approved, and `tauri-driver`
has no WKWebView backend, so it can only run on a Linux or Windows runner
(IMPLEMENTATION_PLAN.md, Milestone G). Its entry point would be the
`capture_text` command, which runs the capture pipeline as if the hotkey had
copied the text.

## Hard rule (non-negotiable, carried into every future milestone)

**No automated test or watch-loop run may download or install Anki or
AnkiConnect, touch a real Anki install, or reach the network.**
`download_and_install_anki` / `auto_provision_ankiconnect` are explicit,
user-triggered, consent-gated actions (Milestone E); accidentally installing
or launching real Anki from a test run would be a correctness and safety
bug, not just a slow test.

Tests do call those two commands, because the consent gate and the checksum
verification are exactly what must be tested, but only with everything that
reaches the outside world replaced: the `Downloader` trait by a recording
fake, the Anki folders (`AnkiPaths`) by temp directories, and the process
runner (`CommandRunner`) by a recording one. Concretely:

- `scripts/bootstrap-macos.sh` (Milestone A2) never calls into Anki/
  AnkiConnect install logic — it only installs _developer tooling_
  (cargo-watch, just, gh, watchman, rtk, the npm dependencies) and the
  pre-commit hook.
- `scripts/run-all-checks.sh` and the `just test-unit` / `just
test-integration` recipes only ever talk to mock servers on `127.0.0.1`,
  never a real AnkiConnect endpoint.
- `the_harness_never_points_at_real_locations` in `src-tauri/tests/ipc.rs`
  asserts it for Automator: config, database and every Anki folder are
  inside the test's temp dir and differ from the real ones, and an install
  attempt reaches the recording fake downloader (and fails on its 404). The
  test of the same name in `apps/learn/src-tauri/tests/ipc_review.rs` does
  the same for Learn: a temp database, an unreachable AnkiConnect address and
  a fake text-to-speech provider.
