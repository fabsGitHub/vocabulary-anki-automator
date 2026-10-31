//! `anki-bridge` — the AnkiConnect boundary (decision 4).
//!
//! `client` speaks AnkiConnect's JSON protocol; `mapper` is the only place in
//! the workspace that knows Anki field names and converts a neutral
//! `vocab_core::Card` into an `addNote` payload. Implemented in the
//! foundation step; see docs/architecture.md § "anki-bridge".
