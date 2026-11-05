//! The setup assistant's own commands through the IPC layer:
//! `get_accessibility_status`, `request_accessibility` and
//! `close_setup_assistant`. The Accessibility API is the harness's
//! [`RecordingAccessibility`](common::RecordingAccessibility), so no test
//! reaches the real one; the Anki steps reuse the commands `tests/ipc.rs`
//! covers.

mod common;

use common::{now, Harness, Options};
use serde_json::{json, Value};
use vocab_automator_lib::config::load_config_from;

fn status(harness: &Harness) -> Value {
    harness.call("get_accessibility_status", json!({}))
}

/// Nothing was downloaded, installed or started.
fn assert_nothing_installed(harness: &Harness) {
    assert!(harness.downloader.requests().is_empty());
    assert!(harness.runner.calls().is_empty());
}

#[test]
fn the_accessibility_status_follows_the_switch_and_asks_nothing() {
    let harness = Harness::new();
    assert_eq!(
        status(&harness),
        json!({ "required": true, "trusted": false })
    );

    harness.accessibility.set_trusted(true);
    assert_eq!(
        status(&harness),
        json!({ "required": true, "trusted": true })
    );

    harness.accessibility.set_trusted(false);
    assert_eq!(
        status(&harness),
        json!({ "required": true, "trusted": false }),
        "a switch turned off again is noticed too"
    );
    assert!(
        harness.accessibility.calls().is_empty(),
        "polling never prompts"
    );
    assert_nothing_installed(&harness);
}

#[test]
fn request_accessibility_prompts_and_opens_system_settings() {
    let harness = Harness::new();
    let result = harness.call("request_accessibility", json!({}));
    assert_eq!(result, json!({ "required": true, "trusted": false }));
    assert_eq!(harness.accessibility.calls(), ["prompt", "open_settings"]);

    // The user flips the switch; the next poll sees it.
    harness.accessibility.set_trusted(true);
    assert_eq!(status(&harness)["trusted"], true);
    assert_nothing_installed(&harness);
}

#[test]
fn request_accessibility_does_nothing_once_granted() {
    let harness = Harness::new();
    harness.accessibility.set_trusted(true);
    let result = harness.call("request_accessibility", json!({}));
    assert_eq!(result, json!({ "required": true, "trusted": true }));
    assert!(harness.accessibility.calls().is_empty());
}

#[test]
fn a_platform_without_the_permission_has_no_step() {
    let harness = Harness::with(Options {
        accessibility_required: false,
        ..Options::default()
    });
    let expected = json!({ "required": false, "trusted": true });
    assert_eq!(status(&harness), expected);
    assert_eq!(harness.call("request_accessibility", json!({})), expected);
    assert!(harness.accessibility.calls().is_empty());
}

#[test]
fn the_assistant_is_open_until_closed_and_remembers_how() {
    let harness = Harness::new();
    let settings = harness.call("get_current_settings", json!({}));
    assert_eq!(
        settings["setup_assistant"],
        json!({ "closed_at": null, "finished": false }),
        "a fresh install opens it"
    );

    let dismissed = harness.call("close_setup_assistant", json!({ "finished": false }));
    let at = now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    assert_eq!(dismissed, json!({ "closed_at": at, "finished": false }));
    assert_eq!(
        harness.call("get_current_settings", json!({}))["setup_assistant"],
        dismissed
    );

    let finished = harness.call("close_setup_assistant", json!({ "finished": true }));
    assert_eq!(finished, json!({ "closed_at": at, "finished": true }));
    let saved = load_config_from(harness.state().config_path()).unwrap();
    assert_eq!(
        saved.setup_assistant.closed_at.as_deref(),
        Some(at.as_str())
    );
    assert!(saved.setup_assistant.finished);
    assert_nothing_installed(&harness);
}

#[test]
fn a_settings_save_neither_closes_nor_reopens_the_assistant() {
    let harness = Harness::new();
    let mut settings = harness.call("get_current_settings", json!({}));
    settings["setup_assistant"] =
        json!({ "closed_at": "2026-01-01T00:00:00.000Z", "finished": true });
    harness.call("apply_settings", json!({ "newCfg": settings }));
    assert_eq!(
        harness.call("get_current_settings", json!({}))["setup_assistant"],
        json!({ "closed_at": null, "finished": false }),
        "apply_settings cannot close it"
    );

    let closed = harness.call("close_setup_assistant", json!({ "finished": true }));
    let mut settings = harness.call("get_current_settings", json!({}));
    settings["setup_assistant"] = json!({ "closed_at": null, "finished": false });
    settings["ui_lang"] = json!("de");
    harness.call("apply_settings", json!({ "newCfg": settings }));
    let after = harness.call("get_current_settings", json!({}));
    assert_eq!(after["ui_lang"], "de", "the save went through");
    assert_eq!(after["setup_assistant"], closed, "and did not reopen it");
}
