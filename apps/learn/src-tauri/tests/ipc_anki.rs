//! Learn's Anki export and import through the IPC layer, against a wiremock
//! AnkiConnect on 127.0.0.1 (docs/test-harness-strategy.md) and against an
//! unreachable one: the exact `createDeck`/`addNote` bodies an export sends,
//! which cards it leaves to Automator, duplicates, and an import that skips
//! the fronts the store already has. Nothing here talks to a real Anki or
//! leaves the machine.

mod common;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use common::{
    b64, capture, chinese_capture, fake_audio, start_time, Harness, Options, UNREACHABLE_ANKI,
};
use serde_json::{json, Value};
use tauri::async_runtime::block_on;
use vocab_core::{store::Store, CaptureOrigin, Card, ScriptMetadata, ScriptVariant};
use wiremock::{
    matchers::{body_json, method},
    Match, Mock, MockServer, Request, Respond, ResponseTemplate,
};

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

/// The forward note of curriculum word 学习 in the default Chinese note type.
fn xuexi_note(audio: Option<Value>) -> Value {
    let mut note = json!({
        "deckName": "Chinese::HSK",
        "modelName": "Chinese (HSK)",
        "fields": {
            "Hanzi": "学习",
            "Pinyin": "xué xí",
            "Meaning": "to study",
            "Sentence": "我们一起学习中文。<br>We study Chinese together."
        },
        "options": { "allowDuplicate": false, "duplicateScope": "deck" },
        "tags": []
    });
    if let Some(audio) = audio {
        note["audio"] = audio;
    }
    note
}

fn audio(lang: &str, text: &str, direction: &str) -> Value {
    json!([{
        "data": b64(&fake_audio(lang, text)),
        "filename": format!("vocab_{}_{direction}.mp3", start_time().timestamp_millis()),
        "fields": ["Audio"]
    }])
}

/// An unsynced Automator capture and an imported card, next to whatever the
/// test learns.
fn with_cards_learn_does_not_export() -> Options {
    Options {
        before_start: Some(Box::new(|store| {
            store
                .insert_capture(
                    &Card::new("Haus", "house", "de", "en", start_time()),
                    CaptureOrigin::Capture,
                    false,
                )
                .unwrap();
            store
                .insert_capture(
                    &chinese_capture("银行", "Bank", "yín háng", ("", "")),
                    CaptureOrigin::AnkiImport,
                    true,
                )
                .unwrap();
        })),
        ..Options::default()
    }
}

#[test]
fn export_sends_the_exact_notes_and_marks_learns_cards_synced() {
    let server = start();
    answer_exactly(&server, create_deck("Chinese::HSK"), ok(json!(1)));
    answer_exactly(&server, add_note(xuexi_note(None)), ok(json!(101)));
    let harness = Harness::with(with_cards_learn_does_not_export());
    harness.use_anki(&server.uri());
    let xuexi = harness.call("start_new_item", json!({ "hanzi": "学习" }));

    let report = harness.call("anki_export", json!({}));

    assert_eq!(
        report,
        json!({ "attempted": 1, "succeeded": 1, "skipped": 0, "failed": [] })
    );
    assert_eq!(
        sent(&server),
        [create_deck("Chinese::HSK"), add_note(xuexi_note(None))]
    );
    let store = harness.open_store();
    let unsynced: Vec<i64> = store
        .list_unsynced_captures()
        .unwrap()
        .iter()
        .map(|capture| capture.id)
        .collect();
    assert_eq!(unsynced, [1], "Automator's capture is left to Automator");
    assert!(
        store
            .get_capture(xuexi.as_i64().unwrap())
            .unwrap()
            .unwrap()
            .synced_to_anki
    );

    let again = harness.call("anki_export", json!({}));
    assert_eq!(
        again,
        json!({ "attempted": 0, "succeeded": 0, "skipped": 0, "failed": [] })
    );
    assert_eq!(sent(&server).len(), 2);
}

#[test]
fn export_sends_the_stored_recording_and_a_spoken_reverse_note() {
    let server = start();
    answer_exactly(&server, create_deck("Chinese::HSK"), ok(json!(1)));
    let forward = add_note(xuexi_note(Some(audio("zh-CN", "学习", "fwd"))));
    let reverse = add_note(json!({
        "deckName": "Chinese::HSK",
        "modelName": "Chinese (HSK)",
        "fields": {
            "Hanzi": "to study",
            "Pinyin": "xué xí",
            "Meaning": "学习",
            "Sentence": "We study Chinese together.<br>我们一起学习中文。"
        },
        "options": { "allowDuplicate": false, "duplicateScope": "deck" },
        "tags": [],
        "audio": audio("en", "to study", "rev")
    }));
    answer_exactly(&server, forward.clone(), ok(json!(101)));
    answer_exactly(&server, reverse.clone(), ok(json!(102)));
    let harness = Harness::new();
    let mut settings = harness.call("get_settings", json!({}));
    settings["anki"]["connect_url"] = json!(server.uri());
    settings["anki"]["create_reverse"] = json!(true);
    harness.call("save_settings", json!({ "settings": settings }));
    let xuexi = harness.call("start_new_item", json!({ "hanzi": "学习" }));
    harness.call("get_card_audio", json!({ "cardId": xuexi }));

    let report = harness.call("anki_export", json!({}));

    assert_eq!(report["succeeded"], 1);
    assert_eq!(
        sent(&server),
        [create_deck("Chinese::HSK"), forward, reverse]
    );
    assert_eq!(harness.tts.calls(), ["zh-CN 学习", "en to study"]);
}

#[test]
fn notes_anki_already_has_count_as_skipped_and_synced() {
    let server = start();
    answer(&server, "createDeck", ok(json!(1)));
    answer(
        &server,
        "addNote",
        anki_error("cannot create note because it is a duplicate"),
    );
    let harness = Harness::new();
    harness.use_anki(&server.uri());
    let wo = harness.call("start_new_item", json!({ "hanzi": "我" }));

    let report = harness.call("anki_export", json!({}));

    assert_eq!(
        report,
        json!({ "attempted": 1, "succeeded": 0, "skipped": 1, "failed": [] })
    );
    let store = harness.open_store();
    assert!(
        store
            .get_capture(wo.as_i64().unwrap())
            .unwrap()
            .unwrap()
            .synced_to_anki
    );
}

#[test]
fn an_anki_error_is_reported_and_the_card_stays_unsynced() {
    let server = start();
    answer(&server, "createDeck", ok(json!(1)));
    answer(
        &server,
        "addNote",
        anki_error("model was not found: Chinese (HSK)"),
    );
    let harness = Harness::new();
    harness.use_anki(&server.uri());
    let wo = harness
        .call("start_new_item", json!({ "hanzi": "我" }))
        .as_i64()
        .unwrap();
    let ni = harness
        .call("start_new_item", json!({ "hanzi": "你" }))
        .as_i64()
        .unwrap();

    let report = harness.call("anki_export", json!({}));

    let error = "AnkiConnect error: model was not found: Chinese (HSK)";
    assert_eq!(
        report,
        json!({
            "attempted": 2,
            "succeeded": 0,
            "skipped": 0,
            "failed": [
                { "card_id": wo, "front": "我", "error": error },
                { "card_id": ni, "front": "你", "error": error }
            ]
        }),
        "an API error does not stop the export"
    );
    assert_eq!(
        harness.open_store().list_unsynced_captures().unwrap().len(),
        2
    );
}

#[test]
fn export_stops_at_an_unreachable_anki() {
    let harness = Harness::new();
    let wo = harness
        .call("start_new_item", json!({ "hanzi": "我" }))
        .as_i64()
        .unwrap();
    harness.call("start_new_item", json!({ "hanzi": "你" }));

    let report = harness.call("anki_export", json!({}));

    assert_eq!(report["attempted"], 1, "the second card is not tried");
    assert_eq!(report["succeeded"], 0);
    let failed = report["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["card_id"], wo);
    assert_eq!(failed[0]["front"], "我");
    let error = failed[0]["error"].as_str().unwrap();
    assert!(error.starts_with("could not reach AnkiConnect"), "{error}");
    assert_eq!(
        harness.open_store().list_unsynced_captures().unwrap().len(),
        2
    );
}

/// A note as AnkiConnect's `notesInfo` reports it, with the default Chinese
/// note type's fields.
fn chinese_note(note_id: i64, hanzi: &str, pinyin: &str, meaning: &str, sentence: &str) -> Value {
    json!({
        "noteId": note_id,
        "modelName": "Chinese (HSK)",
        "tags": ["hsk"],
        "fields": {
            "Hanzi": { "value": hanzi, "order": 0 },
            "Pinyin": { "value": pinyin, "order": 1 },
            "Meaning": { "value": meaning, "order": 2 },
            "Sentence": { "value": sentence, "order": 3 },
            "Audio": { "value": "[sound:x.mp3]", "order": 4 }
        },
        "cards": [note_id + 100],
        "mod": 1793448001
    })
}

#[test]
fn import_adds_new_chinese_words_and_skips_known_fronts() {
    let server = start();
    answer_exactly(
        &server,
        json!({
            "action": "findNotes",
            "version": 6,
            "params": { "query": "\"deck:Chinese::HSK\"" }
        }),
        ok(json!([11, 12, 13, 14, 15, 16, 17])),
    );
    answer_exactly(
        &server,
        json!({
            "action": "notesInfo",
            "version": 6,
            "params": { "notes": [11, 12, 13, 14, 15, 16, 17] }
        }),
        ok(json!([
            chinese_note(11, "学习", "xué xí", "lernen", ""),
            chinese_note(
                12,
                "<b>银行</b>",
                "",
                "Bank",
                "我去银行。<br>Ich gehe zur Bank."
            ),
            // The reverse note Learn and Automator write next to a forward one.
            chinese_note(
                13,
                "Freund",
                "péng you",
                "朋友",
                "Wir sind Freunde.<br>我们是朋友。"
            ),
            chinese_note(
                14,
                "朋友",
                "péng you",
                "Freund",
                "我们是朋友。<br>Wir sind Freunde."
            ),
            chinese_note(15, "Haus", "", "house", ""),
            chinese_note(16, "空", "kōng", "", ""),
            {}
        ])),
    );
    let harness = Harness::with(Options {
        before_start: Some(Box::new(|store| {
            capture(
                store,
                &chinese_capture("学习", "lernen", "xué xí", ("", "")),
            );
        })),
        ..Options::default()
    });
    let mut settings = harness.call("get_settings", json!({}));
    settings["meaning_lang"] = json!("de");
    settings["anki"]["connect_url"] = json!(server.uri());
    harness.call("save_settings", json!({ "settings": settings }));

    let report = harness.call("anki_import", json!({ "deck": " Chinese::HSK " }));

    assert_eq!(
        report,
        json!({
            "attempted": 6,
            "succeeded": 2,
            "skipped": 2,
            "failed": [
                {
                    "card_id": null,
                    "front": "Haus",
                    "error": "No hanzi on either side: Learn imports Chinese vocabulary."
                },
                {
                    "card_id": null,
                    "front": "",
                    "error": "Anki note 16 has no word or no meaning in the fields the mapping reads."
                }
            ]
        })
    );
    let store = harness.open_store();
    let imported = store
        .list_captures(Some(CaptureOrigin::AnkiImport))
        .unwrap();
    assert_eq!(imported.len(), 2);
    let bank = &imported[0];
    assert!(bank.synced_to_anki, "it came from Anki");
    let mut expected = Card::new("银行", "Bank", "zh-CN", "de", start_time());
    expected.example_source = "我去银行。".into();
    expected.example_target = "Ich gehe zur Bank.".into();
    expected.tags = vec!["hsk".into()];
    expected.script_metadata = ScriptMetadata::Chinese {
        pinyin: "yín háng".into(),
        hsk_level: Some(2),
        script_variant: ScriptVariant::Simplified,
    };
    assert_eq!(bank.card, expected, "annotated from the curriculum");
    let friend = &imported[1];
    assert_eq!(
        (friend.card.front.as_str(), friend.card.back.as_str()),
        ("朋友", "Freund"),
        "the reverse note, the right way round"
    );
    assert_eq!(friend.card.example_source, "我们是朋友。");
    assert_eq!(store.count_srs_states().unwrap(), 0, "nothing is scheduled");

    // When the curriculum reaches an imported word, the queue adopts it.
    assert_eq!(
        harness.call("start_new_item", json!({ "hanzi": "银行" })),
        bank.id
    );
}

#[test]
fn import_needs_a_deck_and_a_reachable_anki() {
    let harness = Harness::new();

    assert_eq!(
        harness.call_err("anki_import", json!({ "deck": "  " })),
        "Choose the Anki deck to import."
    );
    let err = harness.call_err("anki_import", json!({ "deck": "Chinese::HSK" }));
    assert!(err.starts_with("could not reach AnkiConnect"), "{err}");
    assert_eq!(
        harness.call("get_settings", json!({}))["anki"]["connect_url"],
        UNREACHABLE_ANKI
    );
    assert!(harness.open_store().list_captures(None).unwrap().is_empty());
}

/// What the shared database allowed while one AnkiConnect request was being
/// answered: whether a second connection could write.
#[derive(Clone)]
struct Inspector {
    db: PathBuf,
    writable: Arc<Mutex<Vec<(String, bool)>>>,
    answers: Arc<Mutex<Vec<(&'static str, Value)>>>,
}

impl Respond for Inspector {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = request.body_json().expect("requests are JSON");
        let action = body["action"].as_str().unwrap_or_default().to_string();
        let written = Store::open(&self.db)
            .and_then(|store| store.set_state("automator.probe", &action))
            .is_ok();
        self.writable
            .lock()
            .unwrap()
            .push((action.clone(), written));
        let result = self
            .answers
            .lock()
            .unwrap()
            .iter()
            .find(|(name, _)| *name == action)
            .map(|(_, result)| result.clone())
            .unwrap_or(json!(1));
        ok(result)
    }
}

/// § 6.3: no transaction is open while AnkiConnect is asked, so Automator
/// can keep writing (a held transaction would make its write wait out the
/// five-second busy timeout and fail).
#[test]
fn the_database_stays_writable_while_anki_is_asked() {
    let server = start();
    let harness = Harness::new();
    let inspector = Inspector {
        db: harness.db_path(),
        writable: Arc::default(),
        answers: Arc::new(Mutex::new(vec![
            ("findNotes", json!([21])),
            (
                "notesInfo",
                json!([chinese_note(21, "银行", "", "bank", "")]),
            ),
        ])),
    };
    block_on(
        Mock::given(method("POST"))
            .respond_with(inspector.clone())
            .mount(&server),
    );
    harness.use_anki(&server.uri());
    harness.call("start_new_item", json!({ "hanzi": "我" }));

    assert_eq!(harness.call("anki_export", json!({}))["succeeded"], 1);
    assert_eq!(
        harness.call("anki_import", json!({ "deck": "Chinese::HSK" }))["succeeded"],
        1
    );

    let seen = inspector.writable.lock().unwrap().clone();
    let actions: Vec<&str> = seen.iter().map(|(action, _)| action.as_str()).collect();
    assert_eq!(actions, ["createDeck", "addNote", "findNotes", "notesInfo"]);
    for (action, written) in &seen {
        assert!(written, "{action}: the other app could not write");
    }
}
