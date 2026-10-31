//! `Learn` — FSRS review sessions over the HSK daily queue, quizzes,
//! lessons, XP and streaks, over the SQLite store Automator captures into
//! (Milestone I, decision 10). The desktop build talks to these commands; the
//! web build of the same frontend replaces them with a TypeScript port of the
//! core (docs/architecture.md § 10, § 12 item 7), and the iOS and Android
//! builds reuse this crate through [`run`]'s mobile entry point.
//!
//! Module map (contracts: docs/architecture.md § 10 and
//! apps/learn/src/platform/types.ts):
//!
//! | module     | milestone | contents                                               |
//! |------------|-----------|--------------------------------------------------------|
//! | `state`    | I         | runtime state, the injected services, startup          |
//! | `settings` | I         | `LearnSettings` in the store's `app_state`             |
//! | `session`  | L2, L3    | today's queue; ratings with progress, XP and streaks   |
//! | `progress` | I, M      | XP for exercises and lessons                           |
//! | `audio`    | M2        | stored recordings or TTS, kept with the card           |
//! | `anki`     | I         | AnkiConnect export and import                          |
//! | `error`    | all       | the error every command returns, as a sentence         |
//! | `commands` | all       | the IPC surface (§ 10)                                 |
//!
//! This crate decides only what a command combines and in which transaction:
//! scheduling, quizzes, gamification and lessons are `vocab_core`'s, and the
//! AnkiConnect protocol and note mapping are `anki_bridge`'s.

pub mod anki;
pub mod audio;
pub mod commands;
pub mod error;
pub mod progress;
pub mod session;
pub mod settings;
pub mod state;

use std::path::PathBuf;

use tauri::{Builder, Manager, Runtime};
use vocab_core::store::{default_db_path, DB_PATH_ENV};

use crate::state::{AppServices, AppState};

/// What the real app and the `tauri::test` mock app share: the invoke handler
/// with every command of § 10. [`run`] adds the plugins and the
/// [`AppState`]; tests manage their own state.
pub fn configure<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::daily::get_daily_prompt_state,
        commands::daily::submit_daily_quota,
        commands::daily::dismiss_daily_prompt,
        commands::daily::reopen_daily_prompt,
        commands::daily::get_daily_queue,
        commands::daily::start_new_item,
        commands::review::get_card,
        commands::review::preview_ratings,
        commands::review::submit_review_rating,
        commands::quiz::list_learned_cards,
        commands::quiz::next_exercise,
        commands::quiz::check_exercise,
        commands::quiz::record_exercise_result,
        commands::progress::get_stats,
        commands::progress::list_lessons,
        commands::progress::complete_lesson,
        commands::audio::get_card_audio,
        commands::audio::synthesize_audio,
        commands::settings::get_settings,
        commands::settings::save_settings,
        commands::anki::anki_export,
        commands::anki::anki_import,
    ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = configure(tauri::Builder::default())
        .plugin(tauri_plugin_log::Builder::new().build())
        .setup(|app| {
            log::info!("Vocabulary Learn is starting.");
            let db_path = database_path(app);
            app.manage(AppState::new(AppServices::production(&db_path)));
            log::info!("Vocabulary Learn is ready.");
            Ok(())
        })
        .run(tauri::generate_context!());
    if let Err(err) = result {
        log::error!("Vocabulary Learn stopped with an error: {err}");
        eprintln!("Vocabulary Learn stopped with an error: {err}");
        std::process::exit(1);
    }
}

/// The database file: on the desktop the one Automator shares
/// ([`default_db_path`]). iOS and Android have no Automator and no data
/// directory the `dirs` crate can find, so there Learn keeps its database in
/// its own app data directory, unless `VOCAB_DB_PATH` says otherwise.
fn database_path(app: &tauri::App) -> PathBuf {
    if cfg!(mobile) && std::env::var_os(DB_PATH_ENV).is_none() {
        match app.path().app_data_dir() {
            Ok(dir) => return dir.join("vocab.sqlite3"),
            Err(err) => log::error!("No app data directory: {err}"),
        }
    }
    default_db_path()
}
