//! Integration tests against a mock AnkiConnect server (Milestone G; the mock
//! stub was chosen over a disposable Anki profile in
//! docs/test-harness-strategy.md).
//!
//! wiremock plays AnkiConnect on a random localhost port. The tests pin the
//! exact JSON the client sends, above all the `addNote` payload produced by
//! the mapper, and how every kind of answer is classified. Nothing here talks
//! to a real Anki or to the network.

use std::time::Duration;

use anki_bridge::{
    card_from_note, deck_query, map_card, push_card, AnkiClient, AnkiError, AnkiTarget, Direction,
    FieldMapping, NoteField, NoteInfo, PushOptions, PushOutcome,
};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use serde_json::{json, Value};
use vocab_core::{Card, ScriptMetadata, ScriptVariant};
use wiremock::{
    matchers::{body_json, method},
    Match, Mock, MockServer, Request, ResponseTemplate,
};

const FWD_AUDIO: &[u8] = &[0xff, 0xfb, 0x90, 0x00]; // base64 "//uQAA=="
const REV_AUDIO: &[u8] = b"ID3\x03\x00"; // base64 "SUQzAwA="

const ENGLISH_DECK: &str = "English::AutomatedVocabulary";
const CHINESE_DECK: &str = "Chinese::HSK";

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

fn error(message: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "result": null, "error": message }))
}

async fn answer(server: &MockServer, action: &'static str, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(Action(action))
        .respond_with(response)
        .mount(server)
        .await;
}

/// Answers only a request whose body is exactly `body`, and fails the test
/// (when the server drops) unless exactly one such request arrived.
async fn answer_exactly(server: &MockServer, body: Value, response: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(body_json(body))
        .respond_with(response)
        .expect(1)
        .mount(server)
        .await;
}

/// Every request body the server received, in order.
async fn sent(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .map(|request| request.body_json().expect("requests are JSON"))
        .collect()
}

/// 2026-10-31T12:00:00.123Z, i.e. 1793448000123 ms since the epoch.
fn captured_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap() + TimeDelta::milliseconds(123)
}

fn english_card() -> Card {
    let mut card = Card::new(
        "serendipity",
        "glücklicher Zufall",
        "en",
        "de",
        captured_at(),
    );
    card.example_source = "It was pure serendipity.".into();
    card.example_target = "Es war reiner Zufall.".into();
    card.notes = "Rare word. More common synonym: luck".into();
    card.tags = vec!["vocab-automator".into()];
    card.audio_front = Some(FWD_AUDIO.to_vec());
    card
}

fn chinese_card() -> Card {
    let mut card = Card::new("学习", "lernen", "zh-CN", "de", captured_at());
    card.example_source = "我们一起学习中文。".into();
    card.example_target = "Wir lernen zusammen Chinesisch.".into();
    card.tags = vec!["vocab-automator".into(), "hsk".into()];
    card.audio_front = Some(FWD_AUDIO.to_vec());
    card.script_metadata = ScriptMetadata::Chinese {
        pinyin: "xué xí".into(),
        hsk_level: Some(1),
        script_variant: ScriptVariant::Simplified,
    };
    card
}

fn english_target() -> AnkiTarget {
    AnkiTarget::new(ENGLISH_DECK, "AppleStyleFinalV3")
}

fn chinese_target() -> AnkiTarget {
    AnkiTarget::new(CHINESE_DECK, "Chinese (HSK)")
}

fn with_reverse() -> PushOptions {
    PushOptions {
        create_reverse: true,
        reverse_audio: Some(REV_AUDIO.to_vec()),
    }
}

fn create_deck_request(deck: &str) -> Value {
    json!({ "action": "createDeck", "version": 6, "params": { "deck": deck } })
}

fn english_forward_request() -> Value {
    json!({
        "action": "addNote",
        "version": 6,
        "params": {
            "note": {
                "deckName": "English::AutomatedVocabulary",
                "modelName": "AppleStyleFinalV3",
                "fields": {
                    "Front": "serendipity",
                    "Back": "glücklicher Zufall",
                    "Context Sentence": "It was pure serendipity.\nEs war reiner Zufall.",
                    "Notes": "Rare word. More common synonym: luck"
                },
                "options": { "allowDuplicate": false, "duplicateScope": "deck" },
                "tags": ["vocab-automator"],
                "audio": [{
                    "data": "//uQAA==",
                    "filename": "vocab_1793448000123_fwd.mp3",
                    "fields": ["Front Audio"]
                }]
            }
        }
    })
}

fn english_reverse_request() -> Value {
    json!({
        "action": "addNote",
        "version": 6,
        "params": {
            "note": {
                "deckName": "English::AutomatedVocabulary",
                "modelName": "AppleStyleFinalV3",
                "fields": {
                    "Front": "glücklicher Zufall",
                    "Back": "serendipity",
                    "Context Sentence": "Es war reiner Zufall.\nIt was pure serendipity.",
                    "Notes": "Rare word. More common synonym: luck"
                },
                "options": { "allowDuplicate": false, "duplicateScope": "deck" },
                "tags": ["vocab-automator"],
                "audio": [{
                    "data": "SUQzAwA=",
                    "filename": "vocab_1793448000123_rev.mp3",
                    "fields": ["Front Audio"]
                }]
            }
        }
    })
}

// --- push_card: the exact payloads -----------------------------------------

#[tokio::test]
async fn push_card_sends_the_exact_create_deck_and_add_note_payloads() {
    let server = MockServer::start().await;
    answer_exactly(
        &server,
        create_deck_request(ENGLISH_DECK),
        ok(json!(1_519_323_742_721_i64)),
    )
    .await;
    answer_exactly(
        &server,
        english_forward_request(),
        ok(json!(1_496_198_395_707_i64)),
    )
    .await;
    answer_exactly(
        &server,
        english_reverse_request(),
        ok(json!(1_496_198_395_708_i64)),
    )
    .await;

    let outcome = push_card(
        &AnkiClient::new(server.uri()),
        &english_card(),
        &english_target(),
        &FieldMapping::apple_style_default(),
        &with_reverse(),
    )
    .await;

    assert_eq!(
        sent(&server).await,
        [
            create_deck_request(ENGLISH_DECK),
            english_forward_request(),
            english_reverse_request()
        ]
    );
    assert_eq!(
        outcome,
        Ok(PushOutcome {
            note_ids: vec![1_496_198_395_707, 1_496_198_395_708],
            duplicates: 0,
        })
    );
}

#[tokio::test]
async fn push_card_with_the_chinese_mapping_sends_hanzi_pinyin_meaning_and_audio() {
    let server = MockServer::start().await;
    let forward = json!({
        "action": "addNote",
        "version": 6,
        "params": {
            "note": {
                "deckName": "Chinese::HSK",
                "modelName": "Chinese (HSK)",
                "fields": {
                    "Hanzi": "学习",
                    "Pinyin": "xué xí",
                    "Meaning": "lernen",
                    "Sentence": "我们一起学习中文。\nWir lernen zusammen Chinesisch."
                },
                "options": { "allowDuplicate": false, "duplicateScope": "deck" },
                "tags": ["vocab-automator", "hsk"],
                "audio": [{
                    "data": "//uQAA==",
                    "filename": "vocab_1793448000123_fwd.mp3",
                    "fields": ["Audio"]
                }]
            }
        }
    });
    answer_exactly(&server, create_deck_request(CHINESE_DECK), ok(json!(7))).await;
    answer_exactly(&server, forward.clone(), ok(json!(11))).await;

    // Reverse audio without create_reverse must not produce a second note.
    let options = PushOptions {
        create_reverse: false,
        reverse_audio: Some(REV_AUDIO.to_vec()),
    };
    let outcome = push_card(
        &AnkiClient::new(server.uri()),
        &chinese_card(),
        &chinese_target(),
        &FieldMapping::chinese_default(),
        &options,
    )
    .await;

    assert_eq!(
        sent(&server).await,
        [create_deck_request(CHINESE_DECK), forward]
    );
    assert_eq!(
        outcome,
        Ok(PushOutcome {
            note_ids: vec![11],
            duplicates: 0
        })
    );
}

#[tokio::test]
async fn push_card_omits_the_audio_key_without_an_audio_field() {
    let server = MockServer::start().await;
    let without_audio = |mut request: Value| {
        request["params"]["note"]
            .as_object_mut()
            .unwrap()
            .remove("audio");
        request
    };
    answer(&server, "createDeck", ok(json!(1))).await;
    answer_exactly(
        &server,
        without_audio(english_forward_request()),
        ok(json!(2)),
    )
    .await;
    answer_exactly(
        &server,
        without_audio(english_reverse_request()),
        ok(json!(3)),
    )
    .await;

    let mapping = FieldMapping {
        audio_field: None,
        ..FieldMapping::apple_style_default()
    };
    let outcome = push_card(
        &AnkiClient::new(server.uri()),
        &english_card(),
        &english_target(),
        &mapping,
        &with_reverse(),
    )
    .await;

    let requests = sent(&server).await;
    assert_eq!(requests.len(), 3);
    for note in &requests[1..] {
        assert_eq!(note["action"], "addNote");
        assert!(note["params"]["note"].get("audio").is_none(), "{note}");
    }
    assert_eq!(
        outcome,
        Ok(PushOutcome {
            note_ids: vec![2, 3],
            duplicates: 0
        })
    );
}

// --- push_card: duplicates and errors -----------------------------------------

#[tokio::test]
async fn push_card_counts_duplicates_instead_of_failing() {
    const DUPLICATE: &str = "cannot create note because it is a duplicate";
    let cases = [
        (false, false, vec![100, 200], 0),
        (true, false, vec![200], 1),
        (false, true, vec![100], 1),
        (true, true, vec![], 2),
    ];
    for (forward_duplicate, reverse_duplicate, note_ids, duplicates) in cases {
        let server = MockServer::start().await;
        answer(&server, "createDeck", ok(json!(1))).await;
        let respond = |duplicate: bool, id: i64| {
            if duplicate {
                error(DUPLICATE)
            } else {
                ok(json!(id))
            }
        };
        answer_exactly(
            &server,
            english_forward_request(),
            respond(forward_duplicate, 100),
        )
        .await;
        answer_exactly(
            &server,
            english_reverse_request(),
            respond(reverse_duplicate, 200),
        )
        .await;

        let outcome = push_card(
            &AnkiClient::new(server.uri()),
            &english_card(),
            &english_target(),
            &FieldMapping::apple_style_default(),
            &with_reverse(),
        )
        .await;

        assert_eq!(
            outcome,
            Ok(PushOutcome {
                note_ids,
                duplicates
            }),
            "forward duplicate: {forward_duplicate}, reverse duplicate: {reverse_duplicate}"
        );
    }
}

#[tokio::test]
async fn push_card_tolerates_a_deck_that_already_exists() {
    let server = MockServer::start().await;
    answer(
        &server,
        "createDeck",
        error("Deck 'English::AutomatedVocabulary' already exists"),
    )
    .await;
    answer_exactly(&server, english_forward_request(), ok(json!(5))).await;

    let outcome = push_card(
        &AnkiClient::new(server.uri()),
        &english_card(),
        &english_target(),
        &FieldMapping::apple_style_default(),
        &PushOptions::default(),
    )
    .await;

    assert_eq!(
        outcome,
        Ok(PushOutcome {
            note_ids: vec![5],
            duplicates: 0
        })
    );
}

#[tokio::test]
async fn push_card_stops_at_the_first_real_error() {
    // (failing action, error, requests sent before giving up)
    let cases = [
        ("createDeck", "collection is not available", 1),
        ("forward", "model was not found: AppleStyleFinalV3", 2),
        ("reverse", "cannot create note because it is empty", 3),
    ];
    for (failing, message, requests) in cases {
        let server = MockServer::start().await;
        let respond = |step: &str, id: i64| {
            if step == failing {
                error(message)
            } else {
                ok(json!(id))
            }
        };
        answer(&server, "createDeck", respond("createDeck", 1)).await;
        // Only the notes that should still be attempted get a (counted) mock.
        if requests >= 2 {
            answer_exactly(&server, english_forward_request(), respond("forward", 2)).await;
        }
        if requests == 3 {
            answer_exactly(&server, english_reverse_request(), respond("reverse", 3)).await;
        }

        let outcome = push_card(
            &AnkiClient::new(server.uri()),
            &english_card(),
            &english_target(),
            &FieldMapping::apple_style_default(),
            &with_reverse(),
        )
        .await;

        assert_eq!(
            outcome,
            Err(AnkiError::Api {
                message: message.into()
            }),
            "{failing}"
        );
        assert_eq!(sent(&server).await.len(), requests, "{failing}");
    }
}

#[tokio::test]
async fn push_card_reports_an_unreachable_anki() {
    let outcome = push_card(
        &AnkiClient::new(closed_port_url()),
        &english_card(),
        &english_target(),
        &FieldMapping::apple_style_default(),
        &with_reverse(),
    )
    .await;
    assert!(
        matches!(outcome, Err(AnkiError::Unreachable { .. })),
        "{outcome:?}"
    );
}

// --- the client: requests and result parsing ------------------------------------

#[tokio::test]
async fn parameterless_actions_send_no_params_and_parse_their_results() {
    let server = MockServer::start().await;
    answer(&server, "version", ok(json!(6))).await;
    answer(&server, "deckNames", ok(json!(["Default", ENGLISH_DECK]))).await;
    answer(
        &server,
        "modelNames",
        ok(json!(["Basic", "AppleStyleFinalV3"])),
    )
    .await;
    let client = AnkiClient::new(server.uri());

    assert_eq!(client.version().await, Ok(6));
    assert_eq!(
        client.deck_names().await,
        Ok(vec!["Default".to_string(), ENGLISH_DECK.to_string()])
    );
    assert_eq!(
        client.model_names().await,
        Ok(vec!["Basic".to_string(), "AppleStyleFinalV3".to_string()])
    );
    assert_eq!(
        sent(&server).await,
        [
            json!({ "action": "version", "version": 6 }),
            json!({ "action": "deckNames", "version": 6 }),
            json!({ "action": "modelNames", "version": 6 }),
        ]
    );
}

#[tokio::test]
async fn model_field_names_and_create_deck_send_their_parameters() {
    let server = MockServer::start().await;
    answer(
        &server,
        "modelFieldNames",
        ok(json!([
            "Front",
            "Back",
            "Context Sentence",
            "Notes",
            "Front Audio"
        ])),
    )
    .await;
    answer(&server, "createDeck", ok(json!(1_519_323_742_721_i64))).await;
    let client = AnkiClient::new(server.uri());

    assert_eq!(
        client.model_field_names("AppleStyleFinalV3").await,
        Ok(
            ["Front", "Back", "Context Sentence", "Notes", "Front Audio"]
                .map(String::from)
                .to_vec()
        )
    );
    assert_eq!(client.create_deck("A::B").await, Ok(1_519_323_742_721));
    assert_eq!(
        sent(&server).await,
        [
            json!({
                "action": "modelFieldNames",
                "version": 6,
                "params": { "modelName": "AppleStyleFinalV3" }
            }),
            create_deck_request("A::B"),
        ]
    );
}

#[tokio::test]
async fn find_notes_and_notes_info_parse_into_note_info() {
    let server = MockServer::start().await;
    answer(
        &server,
        "findNotes",
        ok(json!([1502298033753_i64, 1502298033754_i64, 99])),
    )
    .await;
    // Canned notesInfo answer; AnkiConnect reports a deleted note as `{}`.
    answer(
        &server,
        "notesInfo",
        ok(json!([
            {
                "noteId": 1502298033753_i64,
                "profile": "User 1",
                "modelName": "AppleStyleFinalV3",
                "tags": ["vocab-automator", "en-de"],
                "fields": {
                    "Front": { "value": "serendipity", "order": 0 },
                    "Back": { "value": "glücklicher Zufall", "order": 1 },
                    "Context Sentence": { "value": "It was pure serendipity.<br>Es war reiner Zufall.", "order": 2 },
                    "Notes": { "value": "", "order": 3 },
                    "Front Audio": { "value": "[sound:vocab_1793448000123_fwd.mp3]", "order": 4 }
                },
                "mod": 1793448001,
                "cards": [1502298033760_i64]
            },
            {
                "noteId": 1502298033754_i64,
                "modelName": "Basic",
                "tags": [],
                "fields": { "Front": { "value": "Haus", "order": 0 }, "Back": { "value": "house", "order": 1 } }
            },
            {}
        ])),
    )
    .await;
    let client = AnkiClient::new(server.uri());

    let ids = client.find_notes(&deck_query(ENGLISH_DECK)).await.unwrap();
    assert_eq!(ids, [1_502_298_033_753, 1_502_298_033_754, 99]);
    let notes = client.notes_info(&ids).await.unwrap();

    assert_eq!(
        sent(&server).await,
        [
            json!({
                "action": "findNotes",
                "version": 6,
                "params": { "query": "\"deck:English::AutomatedVocabulary\"" }
            }),
            json!({
                "action": "notesInfo",
                "version": 6,
                "params": { "notes": [1502298033753_i64, 1502298033754_i64, 99] }
            }),
        ]
    );
    let field = |value: &str, order: u32| NoteField {
        value: value.into(),
        order,
    };
    assert_eq!(
        notes,
        [
            NoteInfo {
                note_id: 1_502_298_033_753,
                model_name: "AppleStyleFinalV3".into(),
                tags: vec!["vocab-automator".into(), "en-de".into()],
                fields: [
                    ("Front", field("serendipity", 0)),
                    ("Back", field("glücklicher Zufall", 1)),
                    (
                        "Context Sentence",
                        field("It was pure serendipity.<br>Es war reiner Zufall.", 2)
                    ),
                    ("Notes", field("", 3)),
                    (
                        "Front Audio",
                        field("[sound:vocab_1793448000123_fwd.mp3]", 4)
                    ),
                ]
                .into_iter()
                .map(|(name, field)| (name.to_string(), field))
                .collect(),
                cards: vec![1_502_298_033_760],
                modified: Some(1_793_448_001),
            },
            NoteInfo {
                note_id: 1_502_298_033_754,
                model_name: "Basic".into(),
                tags: vec![],
                fields: [("Front", field("Haus", 0)), ("Back", field("house", 1))]
                    .into_iter()
                    .map(|(name, field)| (name.to_string(), field))
                    .collect(),
                cards: vec![],
                modified: None,
            },
        ]
    );

    let imported = card_from_note(
        &notes[0],
        &FieldMapping::apple_style_default(),
        "en",
        "de",
        captured_at(),
    )
    .unwrap();
    let mut expected = english_card();
    expected.tags = vec!["vocab-automator".into(), "en-de".into()];
    expected.notes.clear();
    expected.audio_front = None;
    assert_eq!(imported, expected);
}

#[tokio::test]
async fn notes_info_is_sent_in_batches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(Action("notesInfo"))
        .respond_with(|request: &Request| {
            let body: Value = request.body_json().unwrap();
            let notes: Vec<Value> = body["params"]["notes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| json!({ "noteId": id, "fields": { "Front": { "value": id.to_string(), "order": 0 } } }))
                .collect();
            ok(Value::Array(notes))
        })
        .mount(&server)
        .await;
    let ids: Vec<i64> = (1..=1201).collect();

    let notes = AnkiClient::new(server.uri())
        .notes_info(&ids)
        .await
        .unwrap();

    let batch_sizes: Vec<usize> = sent(&server)
        .await
        .iter()
        .map(|body| body["params"]["notes"].as_array().unwrap().len())
        .collect();
    assert_eq!(batch_sizes, [500, 500, 201]);
    let returned: Vec<i64> = notes.iter().map(|note| note.note_id).collect();
    assert_eq!(returned, ids);
    assert_eq!(notes[1200].field("Front"), Some("1201"));
}

#[tokio::test]
async fn notes_info_without_ids_sends_no_request() {
    let server = MockServer::start().await;
    let notes = AnkiClient::new(server.uri()).notes_info(&[]).await;
    assert_eq!(notes, Ok(vec![]));
    assert!(sent(&server).await.is_empty());
}

#[tokio::test]
async fn requests_identify_the_client_and_are_json() {
    let server = MockServer::start().await;
    answer(&server, "version", ok(json!(6))).await;
    AnkiClient::new(server.uri()).version().await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let header = |name: &str| {
        requests[0]
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    assert_eq!(header("content-type").as_deref(), Some("application/json"));
    let user_agent = header("user-agent").unwrap_or_default();
    assert!(
        user_agent.starts_with("vocabulary-anki-automator anki-bridge/"),
        "{user_agent}"
    );
}

// --- the client: error classification over the wire -----------------------------

#[tokio::test]
async fn ankiconnect_errors_are_classified() {
    let note = map_card(
        &english_card(),
        Direction::Forward,
        &english_target(),
        &FieldMapping::apple_style_default(),
        None,
    );
    let cases = [
        (
            "cannot create note because it is a duplicate",
            AnkiError::Duplicate {
                message: "cannot create note because it is a duplicate".into(),
            },
        ),
        (
            "Duplicate note in deck",
            AnkiError::Duplicate {
                message: "Duplicate note in deck".into(),
            },
        ),
        (
            "deck was not found: English::AutomatedVocabulary",
            AnkiError::Api {
                message: "deck was not found: English::AutomatedVocabulary".into(),
            },
        ),
    ];
    for (message, expected) in cases {
        let server = MockServer::start().await;
        answer(&server, "addNote", error(message)).await;
        let result = AnkiClient::new(server.uri()).add_note(&note).await;
        assert_eq!(result, Err(expected), "{message}");
    }
}

#[tokio::test]
async fn bodies_that_are_not_ankiconnect_answers_are_malformed() {
    let cases = [
        (
            "html page",
            ResponseTemplate::new(200).set_body_string("<html>It works!</html>"),
        ),
        ("wrong result type", ok(json!({ "decks": ["Default"] }))),
        (
            "not an envelope",
            ResponseTemplate::new(200).set_body_json(json!(["Default"])),
        ),
        ("http error", ResponseTemplate::new(500)),
    ];
    for (name, response) in cases {
        let server = MockServer::start().await;
        answer(&server, "deckNames", response).await;
        let result = AnkiClient::new(server.uri()).deck_names().await;
        assert!(
            matches!(&result, Err(AnkiError::Malformed { message }) if message.starts_with("deckNames: ")),
            "{name}: {result:?}"
        );
    }
}

/// A localhost URL nothing listens on: an ephemeral port that was just free,
/// or the discard port where the environment does not allow binding (the
/// connection then fails before reaching anything, which is the same case).
fn closed_port_url() -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_or(9, |addr| addr.port());
    format!("http://127.0.0.1:{port}")
}

#[tokio::test]
async fn a_closed_port_is_unreachable() {
    let url = closed_port_url();
    let result = AnkiClient::new(url.clone()).version().await;
    let Err(AnkiError::Unreachable { message }) = result else {
        panic!("expected Unreachable, got {result:?}");
    };
    assert!(message.contains("127.0.0.1"), "{message}");
}

#[tokio::test]
async fn an_invalid_url_is_unreachable() {
    let result = AnkiClient::new("not a url").deck_names().await;
    assert!(
        matches!(result, Err(AnkiError::Unreachable { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_slow_answer_times_out_as_unreachable() {
    let server = MockServer::start().await;
    answer(
        &server,
        "version",
        ok(json!(6)).set_delay(Duration::from_secs(5)),
    )
    .await;
    let client = AnkiClient::new(server.uri()).with_timeout(Duration::from_millis(200));

    let started = std::time::Instant::now();
    let result = client.version().await;

    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    let Err(AnkiError::Unreachable { message }) = result else {
        panic!("expected Unreachable, got {result:?}");
    };
    assert!(message.contains("within 200ms"), "{message}");
}

// --- export -> import round trip --------------------------------------------

/// What Anki would report for the note `add_note_request` created: the same
/// fields, with each attached file referenced from its audio field.
fn stored_note(note_id: i64, add_note_request: &Value) -> Value {
    let note = &add_note_request["params"]["note"];
    let mut fields = note["fields"].as_object().unwrap().clone();
    for attachment in note["audio"].as_array().into_iter().flatten() {
        for field in attachment["fields"].as_array().unwrap() {
            let name = field.as_str().unwrap();
            let current = fields.get(name).and_then(Value::as_str).unwrap_or("");
            let referenced = format!(
                "{current}[sound:{}]",
                attachment["filename"].as_str().unwrap()
            );
            fields.insert(name.to_string(), json!(referenced));
        }
    }
    let fields: serde_json::Map<String, Value> = fields
        .into_iter()
        .enumerate()
        .map(|(order, (name, value))| (name, json!({ "value": value, "order": order })))
        .collect();
    json!({
        "noteId": note_id,
        "modelName": note["modelName"],
        "tags": note["tags"],
        "fields": fields,
        "mod": 1793448001,
        "cards": [note_id + 1]
    })
}

#[tokio::test]
async fn exported_notes_import_back_as_the_same_card() {
    let mut chinese_imported = chinese_card();
    // chinese_default has no HskLevel binding, so the level does not survive.
    chinese_imported.script_metadata = ScriptMetadata::Chinese {
        pinyin: "xué xí".into(),
        hsk_level: None,
        script_variant: ScriptVariant::Simplified,
    };
    let cases = [
        (
            english_card(),
            english_target(),
            FieldMapping::apple_style_default(),
            english_card(),
        ),
        (
            chinese_card(),
            chinese_target(),
            FieldMapping::chinese_default(),
            chinese_imported,
        ),
    ];
    for (card, target, mapping, mut expected) in cases {
        let export = MockServer::start().await;
        answer(&export, "createDeck", ok(json!(1))).await;
        answer(&export, "addNote", ok(json!(4242))).await;
        let pushed = push_card(
            &AnkiClient::new(export.uri()),
            &card,
            &target,
            &mapping,
            &PushOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(pushed.note_ids, [4242]);
        let add_note_request = sent(&export).await.pop().unwrap();

        let import = MockServer::start().await;
        answer(&import, "findNotes", ok(json!([4242]))).await;
        answer(
            &import,
            "notesInfo",
            ok(json!([stored_note(4242, &add_note_request)])),
        )
        .await;
        let client = AnkiClient::new(import.uri());
        let ids = client.find_notes(&deck_query(&target.deck)).await.unwrap();
        let notes = client.notes_info(&ids).await.unwrap();
        assert_eq!(notes.len(), 1);

        let imported = card_from_note(
            &notes[0],
            &mapping,
            &card.source_lang,
            &card.target_lang,
            card.captured_at,
        );
        expected.audio_front = None;
        assert_eq!(imported, Some(expected), "{}", target.deck);
    }
}
