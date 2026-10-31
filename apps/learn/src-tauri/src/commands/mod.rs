//! The IPC surface: every command of docs/architecture.md § 10, whose
//! TypeScript side is apps/learn/src/platform/types.ts (JS passes arguments
//! in camelCase).
//!
//! Commands stay thin: the scheduling, quiz, gamification and lesson logic is
//! vocab-core's, the AnkiConnect protocol anki-bridge's, and what this crate
//! combines lives in [`crate::session`], [`crate::progress`],
//! [`crate::audio`] and [`crate::anki`]. Every command that touches the store
//! is `async`, so it runs on the async runtime instead of the main thread,
//! and returns [`crate::error::Error`], which reaches the frontend as one
//! readable sentence.

pub mod anki;
pub mod audio;
pub mod daily;
pub mod progress;
pub mod quiz;
pub mod review;
pub mod settings;
