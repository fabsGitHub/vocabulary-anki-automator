//! The Automator's AnkiConnect conversations through the IPC layer, against a
//! wiremock AnkiConnect on 127.0.0.1 (docs/test-harness-strategy.md): the
//! exact `createDeck`/`addNote` bodies an approval sends, the capture row it
//! leaves behind, duplicates, the retry of unsynced captures into the deck
//! they were approved for, and the status probes. Nothing here talks to a real
//! Anki or leaves the machine.

mod common;

use common::{b64, fake_audio, Harness, Options, UNREACHABLE_ANKI};
use serde_json::{json, Value};
use tauri::async_runtime::block_on;
use vocab_automator_lib::{config::HotkeyProfile, push::deck_key};
use wiremock::{
    matchers::{body_json, method},
    Match, Mock, MockServer, Request, ResponseTemplate,
};

/// 2026-10-31T12:00:00.123Z in milliseconds: the fixed clock's audio stamp.
const STAMP: i64 = 1_793_448_000_123;

/// Matches AnkiConnect requests by their `action`.
struct Action(&'static str);

impl Match for Action {
    fn matches(&self, request: &Request) -> bool {
        request
            .body_json::<Value>()
            .is_ok_and(|body| body["action"] == self.0)
    }
}

fn ok(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "result": result, "error": null }))
}

fn anki_error(message: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "result": null, "error": message }))
}

fn start() -> MockServer {
    block_on(MockServer::start())
}

fn answer(server: &MockServer, action: &'static str, response: ResponseTemplate) {
    block_on(
        Mock::given(method("POST"))
            .and(Action(action))
            .respond_with(response)
            .mount(server),
    );
}

/// Answers only a request whose body is exactly `body`, and fails the test
/// when the server drops unless exactly one such request arrived.
fn answer_exactly(server: &MockServer, body: Value, response: ResponseTemplate) {
    block_on(
        Mock::given(method("POST"))
            .and(body_json(body))
            .respond_with(response)
            .expect(1)
            .mount(server),
    );
}

/// Every request body the server received, in order.
fn sent(server: &MockServer) -> Vec<Value> {
    block_on(server.received_requests())
        .expect("request recording is on")
        .iter()
        .map(|request| request.body_json().expect("requests are JSON"))
        .collect()
}

fn create_deck(deck: &str) -> Value {
    json!({ "action": "createDeck", "version": 6, "params": { "deck": deck } })
}

fn add_note(note: Value) -> Value {
    json!({ "action": "addNote", "version": 6, "params": { "note": note } })
}

fn capture(harness: &Harness, text: &str, profile: Option<&str>) -> Value {
    harness.call(
        "capture_text",
        json!({ "text": text, "profileId": profile }),
    );
    harness.call("get_pending_payload", json!({}))["card"].clone()
}

fn approve(harness: &Harness, card: Value) -> Value {
    harness.call("execute_card_generation", json!({ "card": card }))
}

fn serendipity_forward() -> Value {
    add_note(json!({
        "deckName": "English::AutomatedVocabulary",
        "modelName": "AppleStyleFinalV3",
        "fields": {
            "Front": "serendipity",
            "Back": "glücklicher Zufall",
            "Context Sentence": "An example with serendipity.<br>[de] An example with serendipity.",
            "Notes": ""
        },
        "options": { "allowDuplicate": false, "duplicateScope": "deck" },
        "tags": [],
        "audio": [{
            "data": b64(&fake_audio("en", "serendipity")),
            "filename": format!("vocab_{STAMP}_fwd.mp3"),
            "fields": ["Front Audio"]
        }]
    }))
}

fn serendipity_reverse() -> Value {
    add_note(json!({
        "deckName": "English::AutomatedVocabulary",
        "modelName": "AppleStyleFinalV3",
        "fields": {
            "Front": "glücklicher Zufall",
            "Back": "serendipity",
            "Context Sentence": "[de] An example with serendipity.<br>An example with serendipity.",
            "Notes": ""
        },
        "options": { "allowDuplicate": false, "duplicateScope": "deck" },
        "tags": [],
        "audio": [{
            "data": b64(&fake_audio("de", "glücklicher Zufall")),
            "filename": format!("vocab_{STAMP}_rev.mp3"),
            "fields": ["Front Audio"]
        }]
    }))
}

#[test]
fn approving_sends_the_exact_notes_and_marks_the_capture_synced() {
    let server = start();
    answer_exactly(
        &server,
        create_deck("English::AutomatedVocabulary"),
        ok(json!(1_700_000_000_000_i64)),
    );
    answer_exactly(&server, serendipity_forward(), ok(json!(101)));
    answer_exactly(&server, serendipity_reverse(), ok(json!(102)));
    let harness = Harness::with_anki(&server.uri());
    let card = capture(&harness, "serendipity", None);

    let result = approve(&harness, card);

    assert_eq!(
        result,
        json!({ "capture_id": 1, "synced_to_anki": true, "note_ids": [101, 102], "anki_error": null })
    );
    assert_eq!(
        sent(&server),
        [
            create_deck("English::AutomatedVocabulary"),
            serendipity_forward(),
            serendipity_reverse()
        ]
    );
    let row = harness
        .open_store()
        .get_capture(1)
        .unwrap()
        .expect("persisted");
    assert!(row.synced_to_anki);
    assert_eq!(row.card.front, "serendipity");
    assert_eq!(row.card.audio_front, Some(fake_audio("en", "serendipity")));
    assert_eq!(harness.call("get_pending_payload", json!({})), Value::Null);
    assert_eq!(harness.events("captures-changed").len(), 1);
}

#[test]
fn a_duplicate_counts_as_synced() {
    let server = start();
    answer(&server, "createDeck", ok(json!(1)));
    answer(
        &server,
        "addNote",
        anki_error("cannot create note because it is a duplicate"),
    );
    let harness = Harness::with_anki(&server.uri());
    let card = capture(&harness, "serendipity", None);

    let result = approve(&harness, card);

    assert_eq!(result["synced_to_anki"], true);
    assert_eq!(result["note_ids"], json!([]));
    assert_eq!(result["anki_error"], Value::Null);
    assert!(
        harness
            .open_store()
            .get_capture(1)
            .unwrap()
            .unwrap()
            .synced_to_anki
    );
}

#[test]
fn an_anki_error_keeps_the_capture_for_the_retry() {
    let server = start();
    answer(&server, "createDeck", ok(json!(1)));
    answer(
        &server,
        "addNote",
        anki_error("model was not found: AppleStyleFinalV3"),
    );
    let harness = Harness::with_anki(&server.uri());
    let card = capture(&harness, "serendipity", None);

    let result = approve(&harness, card);

    assert_eq!(result["synced_to_anki"], false);
    assert_eq!(
        result["anki_error"],
        "AnkiConnect error: model was not found: AppleStyleFinalV3"
    );
    let row = harness.open_store().get_capture(1).unwrap().unwrap();
    assert!(!row.synced_to_anki);
}

/// Milestones H and J5: a profile's deck and a Chinese note type.
#[test]
fn a_chinese_profile_capture_fills_the_chinese_note_type_in_its_deck() {
    let server = start();
    answer_exactly(&server, create_deck("Chinese::HSK"), ok(json!(1)));
    let note = add_note(json!({
        "deckName": "Chinese::HSK",
        "modelName": "Chinese (HSK)",
        "fields": { "Hanzi": "学习", "Pinyin": "xué xí", "Meaning": "lernen", "Sentence": "" },
        "options": { "allowDuplicate": false, "duplicateScope": "deck" },
        "tags": [],
        "audio": [{
            "data": b64(&fake_audio("zh-CN", "学习")),
            "filename": format!("vocab_{STAMP}_fwd.mp3"),
            "fields": ["Audio"]
        }]
    }));
    answer_exactly(&server, note.clone(), ok(json!(7)));

    let mut options = Options::default();
    options.config.anki.connect_url = server.uri();
    options.config.anki.model_name = "Chinese (HSK)".into();
    options.config.anki.create_reverse = false;
    options.config.anki.field_mapping = anki_bridge::FieldMapping::chinese_default();
    options.config.hotkey_profiles.push(HotkeyProfile {
        id: "zh-de".into(),
        label: "Chinese → German".into(),
        shortcut: "CommandOrControl+Shift+H".into(),
        source_lang: "zh-CN".into(),
        target_lang: "de".into(),
        deck_name: Some("Chinese::HSK".into()),
    });
    let harness = Harness::with(options);
    let card = capture(&harness, "学习", Some("zh-de"));

    let result = approve(&harness, card);

    assert_eq!(result["note_ids"], json!([7]));
    assert_eq!(sent(&server), [create_deck("Chinese::HSK"), note]);
}

#[test]
fn the_retry_pushes_each_capture_into_the_deck_it_was_approved_for() {
    let mut options = Options::default();
    options.config.hotkey_profiles.push(HotkeyProfile {
        id: "zh-de".into(),
        label: "Chinese → German".into(),
        shortcut: "CommandOrControl+Shift+H".into(),
        source_lang: "zh-CN".into(),
        target_lang: "de".into(),
        deck_name: Some("Chinese::HSK".into()),
    });
    let harness = Harness::with(options);
    // Anki is offline while both cards are approved.
    let chinese = capture(&harness, "学习", Some("zh-de"));
    assert_eq!(approve(&harness, chinese)["synced_to_anki"], false);
    let english = capture(&harness, "house", None);
    assert_eq!(approve(&harness, english)["synced_to_anki"], false);
    assert_eq!(
        harness
            .open_store()
            .get_state(&deck_key(1))
            .unwrap()
            .as_deref(),
        Some("Chinese::HSK")
    );

    // Anki comes back.
    let server = start();
    answer(&server, "createDeck", ok(json!(1)));
    answer(&server, "addNote", ok(json!(55)));
    let mut settings = harness.call("get_current_settings", json!({}));
    assert_eq!(settings["anki"]["connect_url"], UNREACHABLE_ANKI);
    settings["anki"]["connect_url"] = json!(server.uri());
    harness.call("apply_settings", json!({ "newCfg": settings }));
    let changes_before = harness.events("captures-changed").len();

    let report = harness.call("sync_pending_captures", json!({}));

    assert_eq!(report, json!({ "attempted": 2, "synced": 2, "failed": [] }));
    let requests = sent(&server);
    let decks: Vec<&str> = requests
        .iter()
        .map(|body| match body["action"].as_str().unwrap() {
            "createDeck" => body["params"]["deck"].as_str().unwrap(),
            _ => body["params"]["note"]["deckName"].as_str().unwrap(),
        })
        .collect();
    assert_eq!(
        decks,
        [
            "Chinese::HSK",
            "Chinese::HSK",
            "Chinese::HSK",
            "English::AutomatedVocabulary",
            "English::AutomatedVocabulary",
            "English::AutomatedVocabulary"
        ]
    );
    // The reverse note of the retried card has audio in the target language.
    assert_eq!(
        requests[2]["params"]["note"]["audio"][0]["data"],
        b64(&fake_audio("de", "lernen"))
    );
    let store = harness.open_store();
    assert!(store.list_unsynced_captures().unwrap().is_empty());
    assert_eq!(
        store.get_state(&deck_key(1)).unwrap(),
        None,
        "the deck note is cleaned up"
    );
    assert_eq!(harness.events("captures-changed").len(), changes_before + 1);

    let again = harness.call("sync_pending_captures", json!({}));
    assert_eq!(again, json!({ "attempted": 0, "synced": 0, "failed": [] }));
}

#[test]
fn the_retry_leaves_rows_learn_created_alone() {
    let server = start();
    answer(&server, "createDeck", ok(json!(1)));
    answer(&server, "addNote", ok(json!(1)));
    let harness = Harness::with_anki(&server.uri());
    let curriculum_card = vocab_core::Card::new("你好", "hallo", "zh-CN", "de", common::now());
    harness
        .open_store()
        .insert_capture(
            &curriculum_card,
            vocab_core::CaptureOrigin::Curriculum,
            false,
        )
        .unwrap();

    let report = harness.call("sync_pending_captures", json!({}));

    assert_eq!(report["attempted"], 0);
    assert!(sent(&server).is_empty());
}

#[test]
fn anki_status_lists_decks_and_note_types() {
    let server = start();
    answer(&server, "version", ok(json!(6)));
    answer(
        &server,
        "deckNames",
        ok(json!(["Default", "English::AutomatedVocabulary"])),
    );
    answer(
        &server,
        "modelNames",
        ok(json!(["Basic", "AppleStyleFinalV3"])),
    );
    let harness = Harness::with_anki(&server.uri());

    let status = harness.call("anki_status", json!({}));

    assert_eq!(
        status,
        json!({
            "online": true,
            "version": 6,
            "decks": ["Default", "English::AutomatedVocabulary"],
            "models": ["Basic", "AppleStyleFinalV3"],
            "error": null
        })
    );
}

#[test]
fn model_field_names_come_from_anki_connect() {
    let server = start();
    answer_exactly(
        &server,
        json!({
            "action": "modelFieldNames",
            "version": 6,
            "params": { "modelName": "AppleStyleFinalV3" }
        }),
        ok(json!([
            "Front",
            "Back",
            "Context Sentence",
            "Notes",
            "Front Audio"
        ])),
    );
    let harness = Harness::with_anki(&server.uri());

    let fields = harness.call(
        "anki_model_field_names",
        json!({ "model": "AppleStyleFinalV3" }),
    );

    assert_eq!(
        fields,
        json!(["Front", "Back", "Context Sentence", "Notes", "Front Audio"])
    );
    answer(
        &server,
        "modelFieldNames",
        anki_error("model was not found: Nope"),
    );
    let err = harness.call_err("anki_model_field_names", json!({ "model": "Nope" }));
    assert_eq!(err, "AnkiConnect error: model was not found: Nope");
}
