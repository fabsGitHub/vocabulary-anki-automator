//! The Automator's commands through the IPC layer of a `tauri::test` mock app
//! (Milestone G): capture, approval with Anki offline, history, settings,
//! consent and setup. Anki is unreachable here (port 1 on loopback); the
//! AnkiConnect conversations themselves are in `tests/ipc_anki.rs`.

mod common;

use std::fs;

use common::{fake_audio, now, Harness, Options};
use serde_json::{json, Value};
use vocab_automator_lib::{
    config::{load_config_from, AppConfig, HotkeyProfile},
    setup::{ankiconnect, AnkiPaths},
};
use vocab_core::{
    store::default_db_path, CaptureOrigin, Card, ScriptMetadata, ScriptVariant, StoredCapture,
};

fn capture(harness: &Harness, text: &str, profile: Option<&str>) {
    let result = harness.call(
        "capture_text",
        json!({ "text": text, "profileId": profile }),
    );
    assert_eq!(result, Value::Null);
}

fn pending(harness: &Harness) -> Value {
    harness.call("get_pending_payload", json!({}))
}

fn pending_card(harness: &Harness) -> Value {
    let pending = pending(harness);
    assert!(!pending.is_null(), "a capture should be pending");
    pending["card"].clone()
}

fn approve(harness: &Harness, card: Value) -> Value {
    harness.call("execute_card_generation", json!({ "card": card }))
}

fn stored(harness: &Harness, id: i64) -> StoredCapture {
    harness
        .open_store()
        .get_capture(id)
        .unwrap()
        .unwrap_or_else(|| panic!("capture {id} should be stored"))
}

fn chinese_profile() -> HotkeyProfile {
    HotkeyProfile {
        id: "zh-de".into(),
        label: "Chinese → German".into(),
        shortcut: "CommandOrControl+Shift+H".into(),
        source_lang: "zh-CN".into(),
        target_lang: "de".into(),
        deck_name: Some("Chinese::HSK".into()),
    }
}

#[test]
fn nothing_is_pending_at_first() {
    let harness = Harness::new();
    assert_eq!(pending(&harness), Value::Null);
    assert_eq!(harness.call("capture_in_progress", json!({})), json!(false));
}

#[test]
fn capture_text_produces_a_pending_capture_and_opens_the_preview() {
    let harness = Harness::new();

    capture(&harness, "  serendipity\n", None);

    let pending = pending(&harness);
    let card: Card = serde_json::from_value(pending["card"].clone()).unwrap();
    let mut expected = Card::new("serendipity", "glücklicher Zufall", "en", "de", now());
    expected.example_source = "An example with serendipity.".into();
    expected.example_target = "[de] An example with serendipity.".into();
    expected.audio_front = Some(fake_audio("en", "serendipity"));
    assert_eq!(card, expected);
    assert_eq!(pending["warnings"], json!([]));
    assert_eq!(pending["profile_id"], Value::Null);
    assert_eq!(pending["card"]["captured_at"], "2026-10-31T12:00:00.123Z");

    assert!(tauri::Manager::get_webview_window(&harness.app, "preview").is_some());
    // A brand-new window fetches the payload itself; the event is for an
    // existing (hidden) one.
    assert!(harness.events("translation-payload").is_empty());
    assert_eq!(harness.call("capture_in_progress", json!({})), json!(true));
}

#[test]
fn a_second_capture_reuses_the_preview_and_sends_it_the_payload() {
    let harness = Harness::new();
    capture(&harness, "serendipity", None);
    capture(&harness, "house", None);

    let events = harness.events("translation-payload");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["card"]["front"], "house");
    assert_eq!(events[0]["card"]["back"], "Haus");
    assert_eq!(pending_card(&harness)["front"], "house");
}

#[test]
fn a_profile_brings_its_language_pair() {
    let mut options = Options::default();
    options.config.hotkey_profiles.push(chinese_profile());
    let harness = Harness::with(options);

    capture(&harness, "学习", Some("zh-de"));

    let pending = pending(&harness);
    assert_eq!(pending["profile_id"], "zh-de");
    let card: Card = serde_json::from_value(pending["card"].clone()).unwrap();
    assert_eq!(
        (card.source_lang.as_str(), card.target_lang.as_str()),
        ("zh-CN", "de")
    );
    assert_eq!(card.back, "lernen");
    assert_eq!(
        card.script_metadata,
        ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        }
    );
    assert_eq!(card.audio_front, Some(fake_audio("zh-CN", "学习")));
}

/// Milestone H: a phrase is captured and translated; only the single-word
/// rarity check is skipped.
#[test]
fn phrases_are_captured_without_the_rarity_check() {
    let harness = Harness::new();

    capture(&harness, "look forward to", None);

    let card = pending_card(&harness);
    assert_eq!(card["front"], "look forward to");
    assert_eq!(card["back"], "[de] look forward to");
    assert_eq!(card["notes"], "");
    let rarity_calls = |harness: &Harness| {
        harness
            .providers
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("rarity "))
            .collect::<Vec<_>>()
    };
    assert!(rarity_calls(&harness).is_empty());

    capture(&harness, "sesquipedalian", None);
    assert_eq!(
        pending_card(&harness)["notes"],
        "Rare word. More common synonym: long"
    );
    assert_eq!(rarity_calls(&harness), ["rarity sesquipedalian"]);
}

#[test]
fn provider_failures_reach_the_approval_window_as_warnings() {
    let harness = Harness::new();
    capture(&harness, "quota", None);
    let pending = pending(&harness);
    assert_eq!(pending["card"]["back"], "");
    assert_eq!(
        pending["warnings"][0],
        json!({
            "stage": "translate",
            "error": {
                "kind": "quota_exceeded",
                "provider": "fake_translation",
                "message": "come back tomorrow"
            }
        })
    );
}

#[test]
fn refused_captures_are_reported_as_capture_errors() {
    let harness = Harness::new();

    capture(&harness, &"x".repeat(101), None);
    capture(&harness, "   ", None);

    let errors = harness.events("capture-error");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(
        errors[0]["message"],
        "The selection is 101 characters long, but captures are limited to 100 characters. \
         Select less text or raise the limit in the settings."
    );
    assert_eq!(
        errors[1]["message"],
        "Nothing to capture: the selected text is empty."
    );
    assert_eq!(pending(&harness), Value::Null);
    assert_eq!(harness.call("capture_in_progress", json!({})), json!(false));
}

#[test]
fn capture_text_refuses_an_unknown_profile() {
    let harness = Harness::new();
    let err = harness.call_err(
        "capture_text",
        json!({ "text": "house", "profileId": "nope" }),
    );
    assert_eq!(err, "There is no hotkey profile \"nope\".");
}

#[test]
fn an_offline_anki_is_a_result_not_an_error() {
    let harness = Harness::new();
    capture(&harness, "serendipity", None);
    let card = pending_card(&harness);

    let result = approve(&harness, card.clone());

    assert_eq!(result["capture_id"], 1);
    assert_eq!(result["synced_to_anki"], false);
    assert_eq!(result["note_ids"], json!([]));
    let anki_error = result["anki_error"].as_str().unwrap();
    assert!(
        anki_error.starts_with("could not reach AnkiConnect"),
        "{anki_error}"
    );

    let row = stored(&harness, 1);
    assert!(!row.synced_to_anki);
    assert_eq!(row.origin, CaptureOrigin::Capture);
    assert_eq!(row.card, serde_json::from_value::<Card>(card).unwrap());
    assert_eq!(
        pending(&harness),
        Value::Null,
        "an approved card is no longer pending"
    );
    assert_eq!(harness.events("captures-changed").len(), 1);
    // The reverse note's audio was synthesized in the target language.
    assert_eq!(
        harness.providers.tts_calls(),
        ["tts en serendipity", "tts de glücklicher Zufall"]
    );
}

#[test]
fn an_edited_front_gets_fresh_audio() {
    let harness = Harness::new();
    capture(&harness, "serendipity", None);
    let card = common::with_field(pending_card(&harness), "front", json!(" serendipitous "));

    approve(&harness, card);

    let row = stored(&harness, 1);
    assert_eq!(row.card.front, "serendipitous");
    assert_eq!(
        row.card.audio_front,
        Some(fake_audio("en", "serendipitous"))
    );
    assert_eq!(
        harness.providers.tts_calls(),
        [
            "tts en serendipity",
            "tts en serendipitous",
            "tts de glücklicher Zufall"
        ]
    );
}

#[test]
fn an_unchanged_front_keeps_the_captured_audio_even_if_the_window_dropped_it() {
    let harness = Harness::new();
    capture(&harness, "serendipity", None);
    let card = common::with_field(pending_card(&harness), "audio_front", Value::Null);

    approve(&harness, card);

    assert_eq!(
        stored(&harness, 1).card.audio_front,
        Some(fake_audio("en", "serendipity"))
    );
    assert_eq!(
        harness.providers.tts_calls(),
        ["tts en serendipity", "tts de glücklicher Zufall"],
        "no TTS re-run for an unchanged front"
    );
}

#[test]
fn an_edited_chinese_front_is_annotated_again_unless_the_pinyin_was_edited() {
    let mut options = Options::default();
    options.config.hotkey_profiles.push(chinese_profile());
    let harness = Harness::with(options);

    capture(&harness, "学习", Some("zh-de"));
    let card = common::with_field(pending_card(&harness), "front", json!("学生"));
    approve(&harness, card);
    assert_eq!(
        stored(&harness, 1).card.script_metadata,
        ScriptMetadata::Chinese {
            pinyin: "pinyin of 学生".into(),
            hsk_level: None,
            script_variant: ScriptVariant::Simplified,
        }
    );

    capture(&harness, "学习", Some("zh-de"));
    let mut card = common::with_field(pending_card(&harness), "front", json!("学生"));
    card["script_metadata"]["pinyin"] = json!("xué sheng");
    approve(&harness, card);
    assert_eq!(
        stored(&harness, 2).card.pinyin(),
        Some("xué sheng"),
        "the user's own pinyin wins"
    );
}

#[test]
fn a_card_without_a_front_is_refused_and_stays_pending() {
    let harness = Harness::new();
    capture(&harness, "serendipity", None);
    let card = common::with_field(pending_card(&harness), "front", json!("  "));

    let err = harness.call_err("execute_card_generation", json!({ "card": card }));

    assert_eq!(err, "The card needs a front.");
    assert!(!pending(&harness).is_null());
    assert!(harness
        .open_store()
        .list_recent_captures(10)
        .unwrap()
        .is_empty());
}

#[test]
fn without_a_database_an_offline_anki_means_the_card_stays_pending() {
    let harness = Harness::with(Options {
        with_store: false,
        ..Options::default()
    });
    capture(&harness, "serendipity", None);

    let err = harness.call_err(
        "execute_card_generation",
        json!({ "card": pending_card(&harness) }),
    );

    assert!(err.starts_with("The card was not saved"), "{err}");
    assert!(!pending(&harness).is_null(), "nothing was lost");
    assert!(harness.events("captures-changed").is_empty());
    let history = harness.call_err("list_recent_captures", json!({ "limit": 10 }));
    assert!(
        history.starts_with("The capture history is unavailable"),
        "{history}"
    );
}

#[test]
fn discarding_clears_the_capture_and_hides_the_preview() {
    let harness = Harness::new();
    capture(&harness, "serendipity", None);
    assert_eq!(harness.call("capture_in_progress", json!({})), json!(true));

    assert_eq!(
        harness.call("discard_pending_payload", json!({})),
        Value::Null
    );

    assert_eq!(pending(&harness), Value::Null);
    assert_eq!(harness.call("capture_in_progress", json!({})), json!(false));
    // Hidden, not destroyed: the next capture reuses it. (The mock runtime
    // reports every window as visible, so hiding itself is not observable.)
    assert!(tauri::Manager::get_webview_window(&harness.app, "preview").is_some());
}

#[test]
fn the_history_lists_newest_first_without_audio() {
    let harness = Harness::new();
    for word in ["serendipity", "house", "quota"] {
        capture(&harness, word, None);
        approve(&harness, pending_card(&harness));
    }
    assert!(
        harness
            .open_store()
            .get_capture(1)
            .unwrap()
            .unwrap()
            .card
            .audio_front
            .is_some(),
        "precondition: the store holds audio"
    );

    let all = harness.call("list_recent_captures", json!({ "limit": 10 }));
    let fronts: Vec<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["card"]["front"].as_str().unwrap())
        .collect();
    // Equal timestamps (fixed clock): the newest row comes first.
    assert_eq!(fronts, ["quota", "house", "serendipity"]);
    assert!(all
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["card"]["audio_front"].is_null()));
    assert_eq!(all[0]["synced_to_anki"], false);
    assert_eq!(all[0]["origin"], "capture");

    let clamped_low = harness.call("list_recent_captures", json!({ "limit": 0 }));
    assert_eq!(clamped_low.as_array().unwrap().len(), 1);
    let negative = harness.call("list_recent_captures", json!({ "limit": -5 }));
    assert_eq!(negative.as_array().unwrap().len(), 1);
    let clamped_high = harness.call("list_recent_captures", json!({ "limit": 1_000_000 }));
    assert_eq!(clamped_high.as_array().unwrap().len(), 3);
}

#[test]
fn deleting_a_capture() {
    let harness = Harness::new();
    capture(&harness, "serendipity", None);
    approve(&harness, pending_card(&harness));
    let changes_before = harness.events("captures-changed").len();

    assert_eq!(
        harness.call("delete_capture", json!({ "id": 1 })),
        json!(true)
    );
    assert_eq!(
        harness.call("delete_capture", json!({ "id": 1 })),
        json!(false)
    );

    assert!(harness.open_store().get_capture(1).unwrap().is_none());
    assert_eq!(harness.events("captures-changed").len(), changes_before + 1);
}

#[test]
fn sync_stops_at_the_first_unreachable_anki() {
    let harness = Harness::new();
    for word in ["serendipity", "house"] {
        capture(&harness, word, None);
        approve(&harness, pending_card(&harness));
    }

    let report = harness.call("sync_pending_captures", json!({}));

    assert_eq!(report["attempted"], 1);
    assert_eq!(report["synced"], 0);
    assert_eq!(report["failed"][0]["id"], 1);
    let error = report["failed"][0]["error"].as_str().unwrap();
    assert!(error.starts_with("could not reach AnkiConnect"), "{error}");
    assert_eq!(
        harness.open_store().list_unsynced_captures().unwrap().len(),
        2
    );
}

#[test]
fn settings_round_trip_through_the_file() {
    let harness = Harness::new();
    let mut settings = harness.call("get_current_settings", json!({}));
    assert_eq!(settings["translation"]["provider"], "mymemory");

    settings["ui_lang"] = json!("de");
    settings["translation"]["source_lang"] = json!("FR");
    settings["translation"]["contact_email"] = json!("  ");
    settings["hotkey_profiles"] = json!([{
        "id": "zh-de",
        "label": "Chinese → German",
        "shortcut": "CommandOrControl+Shift+H",
        "source_lang": "zh",
        "target_lang": "de",
        "deck_name": "Chinese::HSK"
    }]);
    assert_eq!(
        harness.call("apply_settings", json!({ "newCfg": settings })),
        Value::Null
    );

    let saved = load_config_from(harness.state().config_path()).unwrap();
    assert_eq!(saved.ui_lang, "de");
    assert_eq!(saved.translation.source_lang, "fr");
    assert_eq!(saved.translation.contact_email, None);
    assert_eq!(saved.hotkey_profiles[0].source_lang, "zh-CN");
    let current: AppConfig =
        serde_json::from_value(harness.call("get_current_settings", json!({}))).unwrap();
    assert_eq!(current, saved);
}

#[test]
fn invalid_settings_are_refused_and_nothing_is_saved() {
    let harness = Harness::new();
    let before = fs::read_to_string(harness.state().config_path()).unwrap();
    let mut settings = harness.call("get_current_settings", json!({}));
    settings["translation"]["max_chars"] = json!(0);
    settings["translation"]["target_lang"] = json!("pt");

    let err = harness.call_err("apply_settings", json!({ "newCfg": settings }));

    assert_eq!(
        err,
        "Translation: the target language \"pt\" is not supported (supported: en, de, fr, es, \
         it, zh-CN).\nThe capture limit must be between 1 and 500 characters, not 0."
    );
    assert_eq!(
        fs::read_to_string(harness.state().config_path()).unwrap(),
        before
    );
}

#[test]
fn consent_is_only_recorded_by_grant_consent() {
    let harness = Harness::new();
    let mut settings = harness.call("get_current_settings", json!({}));
    settings["consent"]["anki_install_granted_at"] = json!("2026-01-01T00:00:00.000Z");
    harness.call("apply_settings", json!({ "newCfg": settings }));
    assert_eq!(
        harness.call("get_current_settings", json!({}))["consent"],
        json!({ "anki_install_granted_at": null, "ankiconnect_provision_granted_at": null }),
        "apply_settings cannot forge consent"
    );

    let consent = harness.call("grant_consent", json!({ "action": "anki_install" }));

    assert_eq!(
        consent,
        json!({
            "anki_install_granted_at": "2026-10-31T12:00:00.123Z",
            "ankiconnect_provision_granted_at": null
        })
    );
    let saved = load_config_from(harness.state().config_path()).unwrap();
    assert_eq!(
        saved.consent.anki_install_granted_at.as_deref(),
        Some("2026-10-31T12:00:00.123Z")
    );
    let unknown = harness.invoke("grant_consent", json!({ "action": "format_disk" }));
    assert!(unknown.is_err());
}

#[test]
fn both_install_commands_refuse_without_consent() {
    let harness = Harness::new();

    let anki = harness.call_err("download_and_install_anki", json!({}));
    let ankiconnect = harness.call_err("auto_provision_ankiconnect", json!({}));

    assert_eq!(
        anki,
        "Installing Anki needs your consent first. Confirm it in the setup dialog, then try again."
    );
    assert_eq!(
        ankiconnect,
        "Installing AnkiConnect needs your consent first. Confirm it in the setup dialog, then \
         try again."
    );
    assert!(
        harness.downloader.requests().is_empty(),
        "nothing was downloaded"
    );
    assert!(harness.runner.calls().is_empty(), "nothing was run");
    assert!(!harness.anki_paths().anki_base.exists());
}

#[test]
fn a_tampered_ankiconnect_download_installs_nothing() {
    let harness = Harness::new();
    harness.call(
        "grant_consent",
        json!({ "action": "ankiconnect_provision" }),
    );
    for file in ankiconnect::FILES {
        harness
            .downloader
            .serve(ankiconnect::file_url(file.name), "print('not AnkiConnect')");
    }

    let err = harness.call_err("auto_provision_ankiconnect", json!({}));

    assert!(err.contains("does not match its pinned SHA-256"), "{err}");
    assert_eq!(
        harness.downloader.requests(),
        [ankiconnect::file_url("__init__.py")],
        "the first bad file stops everything"
    );
    assert!(!harness.anki_paths().ankiconnect_dir().exists());
    assert_eq!(
        harness.call("check_ankiconnect_installed", json!({})),
        json!(false)
    );
}

#[test]
fn a_tampered_anki_installer_is_never_run() {
    let harness = Harness::new();
    harness.call("grant_consent", json!({ "action": "anki_install" }));
    let pin = vocab_automator_lib::setup::anki_install::installer_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    if let Some(pin) = pin {
        harness.downloader.serve(
            vocab_automator_lib::setup::anki_install::installer_url(pin.file),
            "MZ not the real installer",
        );
    }

    let err = harness.call_err("download_and_install_anki", json!({}));

    match pin {
        Some(_) => assert!(err.contains("does not match its pinned SHA-256"), "{err}"),
        None => assert!(
            err.starts_with("Automatic Anki installation is not available"),
            "{err}"
        ),
    }
    assert!(
        harness.runner.calls().is_empty(),
        "no installer, hdiutil or ditto ran"
    );
    assert!(!harness
        .anki_paths()
        .applications_dir
        .join("Anki.app")
        .exists());
}

#[test]
fn setup_status_and_the_ankiconnect_check_read_the_injected_folders() {
    let harness = Harness::new();
    let status = harness.call("get_setup_status", json!({}));
    assert_eq!(status["anki_installed"], false);
    assert_eq!(status["ankiconnect_installed"], false);
    assert_eq!(status["anki_version"], "24.06.3");
    assert_eq!(
        status["platform_supported"],
        cfg!(any(
            all(
                target_os = "macos",
                any(target_arch = "aarch64", target_arch = "x86_64")
            ),
            all(target_os = "windows", target_arch = "x86_64")
        ))
    );
    assert_eq!(status["consent"]["anki_install_granted_at"], Value::Null);

    let addon = harness.anki_paths().ankiconnect_dir();
    fs::create_dir_all(&addon).unwrap();
    fs::write(addon.join("__init__.py"), "# AnkiConnect\n").unwrap();
    assert_eq!(
        harness.call("check_ankiconnect_installed", json!({})),
        json!(true)
    );
    assert_eq!(
        harness.call("get_setup_status", json!({}))["ankiconnect_installed"],
        true
    );
}

#[test]
fn launching_anki_goes_through_the_injected_runner() {
    let harness = Harness::new();
    let result = harness.invoke("launch_anki_desktop", json!({}));
    if cfg!(target_os = "windows") {
        // Not "installed" in the temp Program Files, so nothing starts.
        assert!(result.is_err());
        assert!(harness.runner.calls().is_empty());
    } else {
        assert_eq!(result, Ok(json!("Starting Anki…")));
        let calls = harness.runner.calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        if cfg!(target_os = "macos") {
            assert_eq!(
                calls[0],
                (
                    "/usr/bin/open".to_string(),
                    vec!["-a".into(), "Anki".into()]
                )
            );
        }
    }
}

#[test]
fn anki_status_reports_an_offline_anki() {
    let harness = Harness::new();
    let status = harness.call("anki_status", json!({}));
    assert_eq!(status["online"], false);
    assert_eq!(status["version"], Value::Null);
    assert_eq!(status["decks"], json!([]));
    assert!(status["error"]
        .as_str()
        .unwrap()
        .starts_with("could not reach AnkiConnect"));
}

/// Milestone A4: no test may reach a real Anki folder, the real config, the
/// real database or the network. This fails as soon as the harness is wired
/// to any default location or the real downloader.
#[test]
fn the_harness_never_points_at_real_locations() {
    let harness = Harness::new();
    let root = harness.dir.path().to_path_buf();
    let state = harness.state();
    let paths = state.setup().paths().unwrap().clone();

    for path in [&paths.anki_base, &paths.applications_dir, &paths.temp_dir] {
        assert!(
            path.starts_with(&root),
            "{} is outside the temp dir",
            path.display()
        );
    }
    if let Ok(real) = AnkiPaths::detect() {
        assert_ne!(paths.anki_base, real.anki_base);
        assert_ne!(paths.applications_dir, real.applications_dir);
        assert_ne!(paths.temp_dir, real.temp_dir);
    }
    assert!(state.config_path().starts_with(&root));
    assert_ne!(harness.db_path(), default_db_path());
    assert!(harness.db_path().starts_with(&root));

    // The downloader is the recording fake: an install attempt reaches it
    // (and fails on its 404), never the internet.
    harness.call(
        "grant_consent",
        json!({ "action": "ankiconnect_provision" }),
    );
    let err = harness.call_err("auto_provision_ankiconnect", json!({}));
    assert!(err.contains("HTTP 404 Not Found"), "{err}");
    assert_eq!(
        harness.downloader.requests(),
        [ankiconnect::file_url("__init__.py")]
    );
}
