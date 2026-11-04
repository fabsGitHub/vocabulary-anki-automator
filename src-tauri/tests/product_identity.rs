//! The names an installed Polywort and its data depend on.
//!
//! The app was called Vocabulary Automator until 2026-11. Only the names
//! people read changed: Tauri derives the settings folder
//! (`~/Library/Application Support/com.ankiautomator.app/config.json`) and
//! the log folder (`~/Library/Logs/com.ankiautomator.app`) from the bundle
//! identifier, both apps share the history in a `VocabularyAutomator` folder,
//! and installed copies ask the updater endpoint for their updates. Renaming
//! any of them would leave an updated app without its settings, history or
//! updates, so these tests pin them next to the new name.

use std::path::PathBuf;

use serde_json::{json, Value};

fn tauri_conf() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let text = std::fs::read_to_string(path).expect("src-tauri/tauri.conf.json is readable");
    serde_json::from_str(&text).expect("src-tauri/tauri.conf.json is JSON")
}

#[test]
fn the_app_is_called_polywort() {
    // The bundle (Polywort.app), the disk image (Polywort_<version>_universal.dmg,
    // which site/build.mjs requires), the menu bar and the window title.
    let conf = tauri_conf();
    assert_eq!(conf["productName"], "Polywort");
    assert_eq!(
        conf.pointer("/app/windows/0/title"),
        Some(&json!("Polywort"))
    );
}

#[test]
fn the_bundle_identifier_keeps_the_settings_and_log_folders() {
    assert_eq!(tauri_conf()["identifier"], "com.ankiautomator.app");
}

#[test]
fn the_updater_asks_the_same_repository() {
    assert_eq!(
        tauri_conf().pointer("/plugins/updater/endpoints"),
        Some(&json!([
            "https://github.com/fabsGitHub/vocabulary-anki-automator/releases/latest/download/latest.json"
        ]))
    );
}

#[test]
fn the_history_stays_in_the_vocabulary_automator_folder() {
    assert_eq!(
        vocab_core::store::db_path_from(None, Some(PathBuf::from("/data"))),
        PathBuf::from("/data/VocabularyAutomator/vocab.sqlite3")
    );
}
