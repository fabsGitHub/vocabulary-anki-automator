//! The Automator's AnkiConnect conversations through the IPC layer, against a
//! wiremock AnkiConnect on 127.0.0.1 (docs/test-harness-strategy.md): the
//! exact `modelFieldNames`/`createDeck`/`addNote` bodies an approval sends,
//! the capture row it leaves behind, duplicates, the retry of unsynced
//! captures into the deck they were approved for, learned languages filed
//! into their own deck and note type, and the status probes. Nothing here
//! talks to a real Anki or leaves the machine.
//!
//! The default config's mapping is matched by field name, so most pushes
//! here start with `modelFieldNames`; the classic note type's fields
//! ([`classic_fields`]) match to the old default mapping.

mod common;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anki_bridge::FieldMapping;
use common::{b64, fake_audio, Harness, Options, UNREACHABLE_ANKI};
use serde_json::{json, Value};
use tauri::async_runtime::block_on;
use vocab_automator_lib::{
    config::{AppConfig, HotkeyProfile, LearnedLanguage},
    push::deck_key,
};
use vocab_core::store::Store;
use wiremock::{
    matchers::{body_json, method},
    Match, Mock, MockServer, Request, Respond, ResponseTemplate,
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

fn model_field_names(model: &str) -> Value {
    json!({ "action": "modelFieldNames", "version": 6, "params": { "modelName": model } })
}

/// The fields of the user's AppleStyleFinalV3 note type.
fn apple_style_final_v3() -> Value {
    json!([
        "Front",
        "Back",
        "English Sentence",
        "German Sentence",
        "Audio"
    ])
}

/// The fields of the classic AppleStyleFinalV3 note type the old default
/// mapping was made for.
fn classic_fields() -> Value {
    json!(["Front", "Back", "Context Sentence", "Notes", "Front Audio"])
}

/// The old default mapping minus its Notes binding: a customized global
/// mapping that does not fit the user's note type.
fn customized_classic() -> FieldMapping {
    let mut mapping = FieldMapping::apple_style_default();
    mapping
        .fields
        .retain(|binding| binding.anki_field != "Notes");
    mapping
}

/// One `addNote` of "serendipity" for the user's AppleStyleFinalV3 note type
/// matched by name: the example halves go to the fields named after their
/// language, on both notes, and the audio to "Audio".
fn apple_v3_note(deck: &str, front: &str, back: &str, audio: Vec<u8>, suffix: &str) -> Value {
    add_note(json!({
        "deckName": deck,
        "modelName": "AppleStyleFinalV3",
        "fields": {
            "Front": front,
            "Back": back,
            "English Sentence": "An example with serendipity.",
            "German Sentence": "[de] An example with serendipity."
        },
        "options": { "allowDuplicate": false, "duplicateScope": "deck" },
        "tags": [],
        "audio": [{
            "data": b64(&audio),
            "filename": format!("vocab_{STAMP}_{suffix}.mp3"),
            "fields": ["Audio"]
        }]
    }))
}

/// Both notes of "serendipity" into `deck` on the user's note type.
fn apple_v3_notes(deck: &str) -> [Value; 2] {
    [
        apple_v3_note(
            deck,
            "serendipity",
            "glücklicher Zufall",
            fake_audio("en", "serendipity"),
            "fwd",
        ),
        apple_v3_note(
            deck,
            "glücklicher Zufall",
            "serendipity",
            fake_audio("de", "glücklicher Zufall"),
            "rev",
        ),
    ]
}

/// A learned language filed into `deck` on AppleStyleFinalV3, its fields
/// matched by name.
fn learned(lang: &str, deck: &str) -> LearnedLanguage {
    LearnedLanguage {
        lang: lang.into(),
        deck_name: deck.into(),
        model_name: "AppleStyleFinalV3".into(),
        field_mapping: None,
        create_reverse: true,
    }
}

fn french_profile(id: &str, deck: Option<&str>) -> HotkeyProfile {
    HotkeyProfile {
        id: id.into(),
        label: id.into(),
        shortcut: format!("Alt+{}", id.len()),
        source_lang: "fr".into(),
        target_lang: "de".into(),
        deck_name: deck.map(str::to_string),
    }
}

/// The deck each request of `requests` names.
fn decks(requests: &[Value]) -> Vec<&str> {
    requests
        .iter()
        .filter_map(|body| match body["action"].as_str().unwrap() {
            "createDeck" => body["params"]["deck"].as_str(),
            "addNote" => body["params"]["note"]["deckName"].as_str(),
            _ => None,
        })
        .collect()
}

fn actions(requests: &[Value]) -> Vec<&str> {
    requests
        .iter()
        .map(|body| body["action"].as_str().unwrap())
        .collect()
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

/// The default config's mapping is matched by field name: on the classic
/// note type that is the old default mapping, so after the one
/// `modelFieldNames` the notes are exactly the ones earlier builds sent.
#[test]
fn approving_sends_the_exact_notes_and_marks_the_capture_synced() {
    let server = start();
    answer_exactly(
        &server,
        model_field_names("AppleStyleFinalV3"),
        ok(classic_fields()),
    );
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
            model_field_names("AppleStyleFinalV3"),
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

/// What the database held for capture 1 when one AnkiConnect request came
/// in: the action, the row's `synced_to_anki` and its remembered deck.
type Seen = (String, Option<bool>, Option<String>);

/// Answers every AnkiConnect request with success (the classic fields for
/// `modelFieldNames`) and records what the shared database holds at that
/// moment, through a second connection as Learn would see it.
#[derive(Clone)]
struct Inspector {
    db: PathBuf,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Respond for Inspector {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = request.body_json().expect("requests are JSON");
        let store = Store::open(&self.db).expect("second connection");
        let synced = store
            .get_capture(1)
            .expect("readable")
            .map(|row| row.synced_to_anki);
        let deck = store.get_state(&deck_key(1)).expect("readable");
        let action = body["action"].as_str().unwrap_or_default().to_string();
        let result = if action == "modelFieldNames" {
            classic_fields()
        } else {
            json!(1)
        };
        self.seen.lock().unwrap().push((action, synced, deck));
        ok(result)
    }
}

/// § 8.2 step 1, and the deck a retry needs: both are committed before
/// AnkiConnect hears about the card, so a crash or quit in the middle of the
/// push, or a `sync_pending_captures` running alongside it, never loses the
/// card or sends it to the default deck.
#[test]
fn the_card_and_its_deck_are_saved_before_anki_is_asked() {
    let server = start();
    let mut options = Options::default();
    options.config.anki.connect_url = server.uri();
    options.config.hotkey_profiles.push(HotkeyProfile {
        id: "zh-de".into(),
        label: "Chinese → German".into(),
        shortcut: "CommandOrControl+Shift+H".into(),
        source_lang: "zh-CN".into(),
        target_lang: "de".into(),
        deck_name: Some("Chinese::HSK".into()),
    });
    let harness = Harness::with(options);
    let inspector = Inspector {
        db: harness.db_path(),
        seen: Arc::default(),
    };
    block_on(
        Mock::given(method("POST"))
            .respond_with(inspector.clone())
            .mount(&server),
    );
    let card = capture(&harness, "学习", Some("zh-de"));

    let result = approve(&harness, card);

    assert_eq!(result["synced_to_anki"], true);
    let seen = inspector.seen.lock().unwrap().clone();
    let actions: Vec<&str> = seen.iter().map(|(action, _, _)| action.as_str()).collect();
    assert_eq!(
        actions,
        ["modelFieldNames", "createDeck", "addNote", "addNote"]
    );
    for (action, synced, deck) in &seen {
        assert_eq!(*synced, Some(false), "{action}: stored, unsynced, first");
        assert_eq!(
            deck.as_deref(),
            Some("Chinese::HSK"),
            "{action}: deck known"
        );
    }
    let store = harness.open_store();
    assert!(store.get_capture(1).unwrap().unwrap().synced_to_anki);
    assert_eq!(
        store.get_state(&deck_key(1)).unwrap(),
        None,
        "forgotten once synced"
    );
}

#[test]
fn a_duplicate_counts_as_synced() {
    let server = start();
    answer(&server, "modelFieldNames", ok(classic_fields()));
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
    answer(&server, "modelFieldNames", ok(classic_fields()));
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

/// Milestones H and J5: a profile's deck and a Chinese note type. A saved
/// global mapping is pushed as saved, without asking for the fields.
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
    options.config.anki.field_mapping = Some(FieldMapping::chinese_default());
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
    answer(&server, "modelFieldNames", ok(classic_fields()));
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
    assert_eq!(
        actions(&requests),
        [
            "modelFieldNames",
            "createDeck",
            "addNote",
            "addNote",
            "createDeck",
            "addNote",
            "addNote"
        ],
        "both cards share the note type, whose fields are read once"
    );
    assert_eq!(
        decks(&requests),
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
        requests[3]["params"]["note"]["audio"][0]["data"],
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

/// "Match by field name" on the user's note type: the default mapping drops
/// its example and audio, the matched one fills every field.
#[test]
fn suggest_field_mapping_matches_the_note_type_s_field_names() {
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
            "English Sentence",
            "German Sentence",
            "Audio"
        ])),
    );
    let harness = Harness::with_anki(&server.uri());

    let matched = harness.call(
        "suggest_field_mapping",
        json!({ "model": "AppleStyleFinalV3", "sourceLang": "en", "targetLang": "de" }),
    );

    assert_eq!(
        matched,
        json!({
            "mapping": {
                "fields": [
                    { "anki_field": "Front", "source": "front" },
                    { "anki_field": "Back", "source": "back" },
                    { "anki_field": "English Sentence", "source": "example_in_source_lang" },
                    { "anki_field": "German Sentence", "source": "example_in_target_lang" }
                ],
                "audio_field": "Audio"
            },
            "guessed": [],
            "unmatched": []
        })
    );
    answer(
        &server,
        "modelFieldNames",
        anki_error("model was not found: Nope"),
    );
    let err = harness.call_err(
        "suggest_field_mapping",
        json!({ "model": "Nope", "sourceLang": "en", "targetLang": "de" }),
    );
    assert_eq!(err, "AnkiConnect error: model was not found: Nope");
}

/// An English learned language without a saved mapping: the note type's
/// fields are read first and matched by name, so the example halves and the
/// audio reach the fields named after them, on both notes.
#[test]
fn a_learned_language_without_a_mapping_is_matched_by_field_name() {
    let server = start();
    answer_exactly(
        &server,
        model_field_names("AppleStyleFinalV3"),
        ok(apple_style_final_v3()),
    );
    answer_exactly(&server, create_deck("english"), ok(json!(1)));
    let [forward, reverse] = apple_v3_notes("english");
    answer_exactly(&server, forward.clone(), ok(json!(101)));
    answer_exactly(&server, reverse.clone(), ok(json!(102)));
    let mut options = Options::default();
    options.config.anki.connect_url = server.uri();
    options.config.learning.languages = vec![learned("en", "english")];
    let harness = Harness::with(options);
    let card = capture(&harness, "serendipity", None);

    let result = approve(&harness, card);

    assert_eq!(result["note_ids"], json!([101, 102]));
    assert_eq!(
        sent(&server),
        [
            model_field_names("AppleStyleFinalV3"),
            create_deck("english"),
            forward,
            reverse
        ]
    );
}

#[test]
fn a_learned_language_with_a_saved_mapping_does_not_ask_for_the_fields() {
    let server = start();
    answer(&server, "createDeck", ok(json!(1)));
    answer(&server, "addNote", ok(json!(7)));
    let mut options = Options::default();
    options.config.anki.connect_url = server.uri();
    options.config.learning.languages = vec![LearnedLanguage {
        field_mapping: Some(FieldMapping::apple_style_default()),
        create_reverse: false,
        ..learned("en", "english")
    }];
    let harness = Harness::with(options);
    let card = capture(&harness, "serendipity", None);

    let result = approve(&harness, card);

    assert_eq!(result["note_ids"], json!([7]));
    let requests = sent(&server);
    assert_eq!(actions(&requests), ["createDeck", "addNote"]);
    assert_eq!(decks(&requests), ["english", "english"]);
    assert_eq!(
        requests[1]["params"]["note"]["fields"]["Context Sentence"],
        "An example with serendipity.<br>[de] An example with serendipity."
    );
}

#[test]
fn a_missing_note_type_keeps_the_capture_for_the_retry_and_names_it() {
    let server = start();
    answer(
        &server,
        "modelFieldNames",
        anki_error("model was not found: AppleStyleFinalV3"),
    );
    let mut options = Options::default();
    options.config.anki.connect_url = server.uri();
    options.config.learning.languages = vec![learned("en", "english")];
    let harness = Harness::with(options);
    let card = capture(&harness, "serendipity", None);

    let result = approve(&harness, card);

    assert_eq!(result["capture_id"], 1);
    assert_eq!(result["synced_to_anki"], false);
    assert_eq!(
        result["anki_error"],
        "AnkiConnect error: model was not found: AppleStyleFinalV3"
    );
    assert_eq!(actions(&sent(&server)), ["modelFieldNames"]);
    assert!(
        !harness
            .open_store()
            .get_capture(1)
            .unwrap()
            .unwrap()
            .synced_to_anki
    );
}

/// The retry files each capture by its language: one `modelFieldNames` for
/// the shared note type, each language's deck, and a profile's own deck
/// where one was remembered.
#[test]
fn the_retry_files_each_capture_by_its_language() {
    let mut options = Options::default();
    options.config.learning.languages = vec![learned("en", "english"), learned("fr", "french")];
    options.config.hotkey_profiles = vec![
        french_profile("fr-de", None),
        french_profile("fr-mined", Some("French::Mined")),
    ];
    let harness = Harness::with(options);
    // Anki is offline while the three cards are approved.
    for (text, profile) in [
        ("house", None),
        ("bonjour", Some("fr-de")),
        ("merci", Some("fr-mined")),
    ] {
        let card = capture(&harness, text, profile);
        assert_eq!(approve(&harness, card)["synced_to_anki"], false, "{text}");
    }
    let store = harness.open_store();
    assert_eq!(
        store.get_state(&deck_key(2)).unwrap(),
        None,
        "the entry's deck"
    );
    assert_eq!(
        store.get_state(&deck_key(3)).unwrap().as_deref(),
        Some("French::Mined")
    );

    // Anki comes back.
    let server = start();
    answer(&server, "modelFieldNames", ok(apple_style_final_v3()));
    answer(&server, "createDeck", ok(json!(1)));
    answer(&server, "addNote", ok(json!(55)));
    let mut settings = harness.call("get_current_settings", json!({}));
    settings["anki"]["connect_url"] = json!(server.uri());
    harness.call("apply_settings", json!({ "newCfg": settings }));

    let report = harness.call("sync_pending_captures", json!({}));

    assert_eq!(report, json!({ "attempted": 3, "synced": 3, "failed": [] }));
    let requests = sent(&server);
    assert_eq!(
        actions(&requests)
            .iter()
            .filter(|action| **action == "modelFieldNames")
            .count(),
        1,
        "the note type's fields are read once"
    );
    assert_eq!(requests[0], model_field_names("AppleStyleFinalV3"));
    assert_eq!(
        decks(&requests),
        [
            "english",
            "english",
            "english",
            "french",
            "french",
            "french",
            "French::Mined",
            "French::Mined",
            "French::Mined"
        ]
    );
    // Each card is matched for its own pair: the French card leaves the
    // English sentence field alone.
    let english_note = &requests[2]["params"]["note"]["fields"];
    assert_eq!(english_note["English Sentence"], "An example with house.");
    let french_note = &requests[5]["params"]["note"]["fields"];
    assert_eq!(french_note["Front"], "bonjour");
    assert_eq!(french_note.get("English Sentence"), None);
    assert!(harness
        .open_store()
        .list_unsynced_captures()
        .unwrap()
        .is_empty());
}

/// The user's report, fixed: their config.json names deck and note type but
/// no field mapping, and their AppleStyleFinalV3 note type has the fields
/// Front, Back, English Sentence, German Sentence and Audio. The approval
/// window warns about nothing, and the example halves and the pronunciation
/// reach the fields named after them, on both notes.
#[test]
fn a_config_without_a_field_mapping_fills_the_user_s_note_type() {
    let server = start();
    answer(&server, "modelFieldNames", ok(apple_style_final_v3()));
    answer_exactly(&server, create_deck("english"), ok(json!(1)));
    let [forward, reverse] = apple_v3_notes("english");
    answer_exactly(&server, forward.clone(), ok(json!(101)));
    answer_exactly(&server, reverse.clone(), ok(json!(102)));
    let config: AppConfig = serde_json::from_value(json!({
        "anki": {
            "connect_url": server.uri(),
            "deck_name": "english",
            "model_name": "AppleStyleFinalV3"
        }
    }))
    .unwrap();
    assert_eq!(config.anki.field_mapping, None);
    assert!(config.learning.languages.is_empty());
    let harness = Harness::with(Options {
        config,
        ..Options::default()
    });
    let card = capture(&harness, "serendipity", None);

    let check = harness.call("check_pending_fields", json!({}));
    let result = approve(&harness, card);

    assert_eq!(
        check,
        json!({
            "status": "checked",
            "model": "AppleStyleFinalV3",
            "auto_matched": true,
            "lost": []
        })
    );
    assert_eq!(result["note_ids"], json!([101, 102]));
    assert_eq!(
        sent(&server),
        [
            model_field_names("AppleStyleFinalV3"),
            model_field_names("AppleStyleFinalV3"),
            create_deck("english"),
            forward,
            reverse
        ]
    );
    assert_eq!(
        harness.call("get_current_settings", json!({}))["anki"]["field_mapping"],
        Value::Null,
        "the match is never written back"
    );
}

/// The approval window's warning for the user's note type: a customized
/// global mapping that does not fit drops the example and the audio, while a
/// global mapping never customized and a learned language, both matched by
/// field name, drop nothing.
#[test]
fn check_pending_fields_reports_what_a_push_would_drop() {
    let server = start();
    answer(&server, "modelFieldNames", ok(apple_style_final_v3()));
    let mut options = Options::default();
    options.config.anki.connect_url = server.uri();
    options.config.anki.field_mapping = Some(customized_classic());
    let harness = Harness::with(options);
    capture(&harness, "serendipity", None);

    let check = harness.call("check_pending_fields", json!({}));

    assert_eq!(
        check,
        json!({
            "status": "checked",
            "model": "AppleStyleFinalV3",
            "auto_matched": false,
            "lost": [
                { "kind": "missing_field", "anki_field": "Context Sentence", "source": "example" },
                { "kind": "audio", "audio_field": "Front Audio" }
            ]
        })
    );
    assert_eq!(sent(&server), [model_field_names("AppleStyleFinalV3")]);

    let matched = json!({
        "status": "checked",
        "model": "AppleStyleFinalV3",
        "auto_matched": true,
        "lost": []
    });
    let harness = Harness::with_anki(&server.uri());
    capture(&harness, "serendipity", None);
    assert_eq!(harness.call("check_pending_fields", json!({})), matched);

    let mut options = Options::default();
    options.config.anki.connect_url = server.uri();
    options.config.learning.languages = vec![learned("en", "english")];
    let harness = Harness::with(options);
    capture(&harness, "serendipity", None);
    assert_eq!(harness.call("check_pending_fields", json!({})), matched);
}

/// The window checks the card as the user edited it: a note typed in that
/// the note type has no field for is reported, an example cleared is not.
#[test]
fn check_pending_fields_checks_the_edited_card() {
    let server = start();
    answer(&server, "modelFieldNames", ok(apple_style_final_v3()));
    let harness = Harness::with_anki(&server.uri());
    let mut draft = capture(&harness, "serendipity", None);
    for (field, value) in [
        ("notes", "Rare word."),
        ("example_source", ""),
        ("example_target", ""),
    ] {
        draft = common::with_field(draft, field, json!(value));
    }

    let check = harness.call("check_pending_fields", json!({ "card": draft }));

    assert_eq!(
        check["lost"],
        json!([{ "kind": "unmapped", "source": "notes" }])
    );
    assert_eq!(
        harness.call("check_pending_fields", json!({ "card": null }))["lost"],
        json!([]),
        "without a draft the pending card is checked"
    );
}

#[test]
fn check_pending_fields_reports_a_missing_note_type() {
    let server = start();
    answer(
        &server,
        "modelFieldNames",
        anki_error("model was not found: AppleStyleFinalV3"),
    );
    let harness = Harness::with_anki(&server.uri());
    capture(&harness, "serendipity", None);

    let check = harness.call("check_pending_fields", json!({}));

    assert_eq!(
        check,
        json!({
            "status": "failed",
            "model": "AppleStyleFinalV3",
            "error": "model was not found: AppleStyleFinalV3"
        })
    );
}
