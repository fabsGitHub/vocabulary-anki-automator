# Test harness strategy (Milestone A4)

Decision record for how automated tests (unit/integration/E2E, Milestone G)
will exercise Anki/AnkiConnect-facing code without needing a real Anki
install in CI or on a contributor's machine.

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
- What this codebase actually needs to verify is *the shape of the HTTP
  requests it sends to AnkiConnect* (`addNote` payload, deck/model names,
  field mapping produced by `anki/mapper.rs`) — a mock server that records
  and asserts on incoming requests tests exactly that, directly, without a
  GUI dependency.
- It works identically in a contributor's sandbox and in CI — no macOS-only
  GUI automation required for the integration tier (only the true E2E tier,
  which does drive the compiled app via `tauri-driver`, needs a real
  environment at all).

Implementation of the stub itself is Milestone G's job (integration tests);
this document only records the decision so Milestone G doesn't re-litigate
it.

## Hard rule (non-negotiable, carried into every future milestone)

**`download_and_install_anki` / `auto_provision_ankiconnect` must never fire
inside any automated test or watch-loop run.** They are explicit,
user-triggered, consent-gated actions on a developer's real machine (see
Milestone E), not something a test suite or `just watch` loop is ever
allowed to invoke — accidentally installing/launching real Anki from a test
run would be a correctness and safety bug, not just a slow test.

Concretely:

- `scripts/bootstrap-macos.sh` (Milestone A2) never calls into Anki/
  AnkiConnect install logic — it only installs *developer tooling*
  (cargo-watch, tauri-driver, gh, watchman, rtk, npm test deps).
- `scripts/run-all-checks.sh` and the `just test-unit` / `just
  test-integration` recipes only ever talk to the mock AnkiConnect stub
  above, never a real AnkiConnect endpoint.
- Whoever implements Milestone G's integration tests should assert this
  directly (e.g., the mock stub is the only `http://127.0.0.1:8765`-style
  endpoint reachable from test config), not just rely on convention.
