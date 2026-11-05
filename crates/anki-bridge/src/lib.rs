//! `anki-bridge` — the AnkiConnect boundary (decision 4).
//!
//! `vocab_core::Card` knows nothing about Anki; this crate is the one place
//! that does. [`client`] speaks AnkiConnect's JSON protocol, [`mapper`] turns a
//! card into an `addNote` payload for the user's note type (and an Anki note
//! back into a card) and defines the note type the app recommends, and
//! [`sync`] runs the deck + notes sequence both apps share. It is a crate of
//! its own, not part of `vocab-core`, so the core stays Anki-agnostic while
//! Automator and Learn still share a single mapping (docs/architecture.md § 9
//! and § 12, deviation 1).
//!
//! Nothing here downloads, installs or launches Anki; that stays behind the
//! Automator's consent-gated commands (§ 8.4).

pub mod client;
pub mod error;
mod html;
pub mod mapper;
pub mod sync;

pub use client::{
    deck_query, AnkiClient, CardTemplate, NoteField, NoteInfo, API_VERSION, DEFAULT_TIMEOUT,
    DEFAULT_URL,
};
pub use error::AnkiError;
pub use mapper::{
    add_note_params, card_from_note, map_card, recommended_note_type, AnkiTarget, AudioAttachment,
    AutoMatch, CardField, Direction, FieldBinding, FieldMapping, LostValue, NoteSpec, NoteTypeSpec,
    RECOMMENDED_NOTE_TYPE,
};
pub use sync::{push_card, PushOptions, PushOutcome};
