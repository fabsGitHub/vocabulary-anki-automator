//! The IPC surface: every command of docs/architecture.md § 8.2, which is
//! binding for the frontend (JS passes arguments in camelCase).
//!
//! Commands are generic over the Tauri runtime so the `tauri::test` mock app
//! invokes exactly the code the real app does, and they stay thin: the logic
//! lives in [`crate::push`], [`crate::capture`], [`crate::setup`] and
//! [`crate::config`]. Errors cross the IPC edge as plain sentences.

pub mod anki;
pub mod app;
pub mod capture;
pub mod history;
pub mod settings;
pub mod setup;
