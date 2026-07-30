# Vocabulary Automator

Tauri 2 desktop app — clipboard-capture vocabulary pipeline with translation,
enrichment, and Anki export. See [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md)
for the full milestone breakdown.

## Git workflow

- Work on a feature branch named after the Paperclip ticket (`pot-N-...`).
- Commit to the branch, never directly to `main`.
- The operator merges into `main` (Tier 2 per SOUL.md).
- Tag releases as `v0.MINOR.PATCH` while pre-1.0; CI triggers off tag pushes.

## Release infrastructure (Milestone N)

- **macOS:** notarized `.dmg` via Apple Developer ID cert (env vars: `APPLE_SIGNING_IDENTITY`,
  `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`).
- **Windows:** OV code-signing cert (`.pfx` or HSM-backed); SmartScreen reputation
  builds over time.
- **Linux:** `.deb`, `.rpm`, and AppImage (the updater target). No OS-level signing.
- **Auto-updater:** `tauri-plugin-updater` with static `latest.json` manifest
  hosted as a GitHub Release asset.

## Signing-key custody (N6)

See [docs/signing-key-custody.md](docs/signing-key-custody.md). The updater private
key and platform signing credentials are the highest-blast-radius secrets in this
project. They must never appear in code, logs, tickets, or agent sessions.
