//! The names an installed Polywort Learn and its data depend on.
//!
//! Learn was called Vocabulary Learn until 2026-11. Only the names people
//! read changed: Tauri derives the app's own folders (its log, and on iOS
//! and Android its database) from the bundle identifier, and on the desktop
//! Learn shares the history with Polywort in a `VocabularyAutomator`
//! folder. Renaming either would leave an updated app without its data, so
//! these tests pin them next to the new name.
//!
//! The log file in the log folder is named after the product, though, so it
//! is now `Polywort Learn.log`. An install from before keeps its old
//! `Vocabulary Learn.log` beside it, and that file is no longer rotated.

use std::path::PathBuf;

use serde_json::{json, Value};

fn tauri_conf() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let text =
        std::fs::read_to_string(path).expect("apps/learn/src-tauri/tauri.conf.json is readable");
    serde_json::from_str(&text).expect("apps/learn/src-tauri/tauri.conf.json is JSON")
}

#[test]
fn the_app_is_called_polywort_learn() {
    let conf = tauri_conf();
    assert_eq!(conf["productName"], "Polywort Learn");
    assert_eq!(
        conf.pointer("/app/windows/0/title"),
        Some(&json!("Polywort Learn"))
    );
    // Without a publisher Tauri takes the identifier's second segment, so
    // Windows would show "ankiautomator" as the exe's company and the
    // installer's publisher.
    assert_eq!(conf.pointer("/bundle/publisher"), Some(&json!("Polywort")));
}

#[test]
fn the_bundle_identifier_keeps_the_app_folders() {
    assert_eq!(tauri_conf()["identifier"], "com.ankiautomator.learn");
}

#[test]
fn the_history_stays_in_the_vocabulary_automator_folder() {
    assert_eq!(
        vocab_core::store::db_path_from(None, Some(PathBuf::from("/data"))),
        PathBuf::from("/data/VocabularyAutomator/vocab.sqlite3")
    );
}
