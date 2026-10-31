//! The Automator's IPC surface, pinned for the frontend
//! (docs/architecture.md § 8.2 and § 11).
//!
//! The commands and their TypeScript side (`src/api/types.ts`,
//! `commands.ts`, `events.ts`) are written separately, so this test writes
//! what the Rust side actually does into a fixture that the Vitest suite
//! `src/api/ipc_shapes.test.ts` checks the TypeScript side against:
//!
//! - `shapes`: serde's JSON of representative values of every Rust type
//!   behind a type of `types.ts` (keyed by the TypeScript name), with every
//!   enum variant and both sides of every `Option` among them. Vitest checks
//!   each value against a schema that is compile-time equal to the
//!   TypeScript type, exact keys included, and through the settings form's
//!   own parsing.
//! - `commands`: every command of § 8.2 with the `src/api` wrapper that
//!   calls it, its camelCase argument names and its result type, as the
//!   TypeScript side spells them. All of it is checked here against the real
//!   invoke handler: each command runs through the IPC layer with exactly
//!   these arguments, each argument is shown to be read under its name (a
//!   value of the wrong type is refused naming it), and each result is
//!   exactly the serialization of the Rust type the TypeScript name stands
//!   for. `restart_app` is the one command that is not run, since it would
//!   restart the test process.
//! - `events`: every event the frontend listens to, with its payload type;
//!   the payloads the backend emits during the session are checked the same
//!   way.
//!
//! The fixture is checked by default and rewritten with
//! `UPDATE_IPC_SHAPES=1 cargo test -p vocab-automator --test ipc_shapes`.

mod common;

use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

use anki_bridge::{CardField, FieldBinding, FieldMapping};
use chrono::{DateTime, Utc};
use common::Harness;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Map, Value};
use vocab_automator_lib::{
    capture::{CaptureErrorPayload, PendingCapture},
    commands::anki::AnkiStatus,
    config::{
        AnkiConfig, AppConfig, ConsentAction, ConsentState, HotkeyProfile, TranslationConfig,
    },
    push::{SubmitResult, SyncFailure, SyncReport},
    setup::SetupStatus,
};
use vocab_core::{
    curriculum::CurriculumIndex,
    language::SUPPORTED_LANGS,
    pipeline::{PipelineStage, PipelineWarning},
    providers::{ProviderRegistry, StandardProviders},
    srs::QueueItem,
    CaptureOrigin, Card, ProviderError, ScriptMetadata, ScriptVariant, StoredCapture,
};

/// The fixture, relative to this crate.
const FIXTURE: &str = "../src/api/__fixtures__/ipc_shapes.json";
/// Set to rewrite the fixture instead of checking it.
const UPDATE_ENV: &str = "UPDATE_IPC_SHAPES";

/// Every command of § 8.2: the `src/api/commands.ts` wrapper that calls it,
/// the command, the camelCase argument names in parameter order, and what it
/// resolves to in the TypeScript spelling.
const COMMANDS: &[(&str, &str, &[&str], &str)] = &[
    (
        "getCurrentSettings",
        "get_current_settings",
        &[],
        "AppConfig",
    ),
    ("applySettings", "apply_settings", &["newCfg"], "null"),
    (
        "getPendingPayload",
        "get_pending_payload",
        &[],
        "PendingCapture | null",
    ),
    (
        "discardPendingPayload",
        "discard_pending_payload",
        &[],
        "null",
    ),
    (
        "executeCardGeneration",
        "execute_card_generation",
        &["card"],
        "SubmitResult",
    ),
    (
        "captureText",
        "capture_text",
        &["text", "profileId"],
        "null",
    ),
    ("ankiStatus", "anki_status", &[], "AnkiStatus"),
    (
        "ankiModelFieldNames",
        "anki_model_field_names",
        &["model"],
        "string[]",
    ),
    ("launchAnkiDesktop", "launch_anki_desktop", &[], "string"),
    ("getSetupStatus", "get_setup_status", &[], "SetupStatus"),
    ("grantConsent", "grant_consent", &["action"], "ConsentState"),
    (
        "downloadAndInstallAnki",
        "download_and_install_anki",
        &[],
        "string",
    ),
    (
        "autoProvisionAnkiconnect",
        "auto_provision_ankiconnect",
        &[],
        "string",
    ),
    (
        "checkAnkiconnectInstalled",
        "check_ankiconnect_installed",
        &[],
        "boolean",
    ),
    (
        "listRecentCaptures",
        "list_recent_captures",
        &["limit"],
        "StoredCapture[]",
    ),
    ("deleteCapture", "delete_capture", &["id"], "boolean"),
    (
        "syncPendingCaptures",
        "sync_pending_captures",
        &[],
        "SyncReport",
    ),
    ("captureInProgress", "capture_in_progress", &[], "boolean"),
    ("restartApp", "restart_app", &[], "never"),
];

/// The command that is never run: it restarts the process.
const NOT_RUN: &str = "restart_app";

/// Commands that fail in the harness as they would for a user without Anki
/// (or, for `launch_anki_desktop`, on Windows without Anki in its default
/// folder) or without consent: their own error, after Tauri took the
/// arguments.
const MAY_FAIL: &[&str] = &[
    "anki_model_field_names",
    "launch_anki_desktop",
    "download_and_install_anki",
    "auto_provision_ankiconnect",
];

/// Every event the frontend listens to (`src/api/events.ts`), with its
/// payload type in the TypeScript spelling.
const EVENTS: &[(&str, &str)] = &[
    ("translation-payload", "PendingCapture"),
    ("capture-error", "CaptureErrorPayload"),
    ("captures-changed", "null"),
];

fn json_of<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).expect("serializable")
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant")
        .with_timezone(&Utc)
}

/// The path of the first difference between `expected` and `actual`.
fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|key| {
                let path = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => first_difference(x, y, &path),
                    _ => Some(path),
                }
            })
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (x, y))| first_difference(x, y, &format!("{path}[{index}]"))),
        _ => (expected != actual).then(|| path.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Representative values

/// An English card with everything set, captured at an instant with
/// milliseconds.
fn english_card() -> Card {
    Card {
        front: "serendipity".into(),
        back: "glücklicher Zufall".into(),
        example_source: "It was pure serendipity.".into(),
        example_target: "Es war reiner Zufall.".into(),
        notes: "Rare word. More common synonym: luck".into(),
        tags: vec!["vocab-automator".into(), "en-de".into()],
        source_lang: "en".into(),
        target_lang: "de".into(),
        captured_at: instant("2026-10-31T12:00:00.123Z"),
        audio_front: Some(b"ID3 mp3 bytes".to_vec()),
        script_metadata: ScriptMetadata::None,
    }
}

/// A Chinese card without audio, captured on a whole second (serde then
/// writes no fraction).
fn chinese_card() -> Card {
    Card {
        front: "学习".into(),
        back: "lernen".into(),
        example_source: "我们在日常生活中使用“学习”这个词。".into(),
        example_target: String::new(),
        notes: String::new(),
        tags: Vec::new(),
        source_lang: "zh-CN".into(),
        target_lang: "de".into(),
        captured_at: instant("2026-10-31T12:00:00Z"),
        audio_front: None,
        script_metadata: ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        },
    }
}

fn provider_errors() -> Vec<ProviderError> {
    vec![
        ProviderError::Network {
            provider: "mymemory".into(),
            message: "error sending request: connection refused".into(),
        },
        ProviderError::QuotaExceeded {
            provider: "mymemory".into(),
            message: "MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY".into(),
        },
        ProviderError::Http {
            provider: "dictionary_api".into(),
            status: 503,
            message: "Service Unavailable".into(),
        },
        ProviderError::Malformed {
            provider: "google_tts".into(),
            message: "the response is not audio".into(),
        },
        ProviderError::NotFound {
            provider: "datamuse".into(),
        },
        ProviderError::Unsupported {
            provider: "pinyin".into(),
            what: "text without hanzi".into(),
        },
    ]
}

fn stages() -> [PipelineStage; 6] {
    [
        PipelineStage::Translate,
        PipelineStage::Rarity,
        PipelineStage::Example,
        PipelineStage::ExampleTranslate,
        PipelineStage::Tts,
        PipelineStage::Annotate,
    ]
}

fn warnings() -> Vec<PipelineWarning> {
    stages()
        .into_iter()
        .zip(provider_errors())
        .map(|(stage, error)| PipelineWarning { stage, error })
        .collect()
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

fn french_profile() -> HotkeyProfile {
    HotkeyProfile {
        id: "fr-de".into(),
        label: String::new(),
        shortcut: "Alt+F".into(),
        source_lang: "fr".into(),
        target_lang: "de".into(),
        deck_name: None,
    }
}

fn granted_consent() -> ConsentState {
    ConsentState {
        anki_install_granted_at: Some("2026-10-31T12:00:00.123Z".into()),
        ankiconnect_provision_granted_at: Some("2026-10-31T12:05:00.000Z".into()),
    }
}

fn custom_mapping() -> FieldMapping {
    FieldMapping {
        fields: vec![
            FieldBinding::new("Front", CardField::Front),
            FieldBinding::new("Back", CardField::Back),
            FieldBinding::new("Level", CardField::HskLevel),
        ],
        audio_field: None,
    }
}

fn custom_config() -> AppConfig {
    AppConfig {
        ui_lang: "de".into(),
        hotkeys: "CommandOrControl+Alt+V".into(),
        hotkey_profiles: vec![chinese_profile(), french_profile()],
        translation: TranslationConfig {
            source_lang: "en".into(),
            target_lang: "fr".into(),
            max_chars: 250,
            provider: "mymemory".into(),
            contact_email: Some("learner@example.com".into()),
        },
        anki: AnkiConfig {
            connect_url: "http://localhost:8765".into(),
            deck_name: "Vocabulary::Mined".into(),
            model_name: "Basic".into(),
            create_reverse: false,
            field_mapping: custom_mapping(),
        },
        consent: granted_consent(),
    }
}

/// `shapes`: the TypeScript type name and its samples.
fn shapes() -> Map<String, Value> {
    let stored = |id, card, origin, synced_to_anki| StoredCapture {
        id,
        card,
        origin,
        synced_to_anki,
    };
    let entries: Vec<(&str, Vec<Value>)> = vec![
        (
            "ScriptMetadata",
            vec![
                json_of(ScriptMetadata::None),
                json_of(chinese_card().script_metadata),
                json_of(ScriptMetadata::Chinese {
                    pinyin: "xué xí".into(),
                    hsk_level: None,
                    script_variant: ScriptVariant::Traditional,
                }),
            ],
        ),
        (
            "Card",
            vec![json_of(english_card()), json_of(chinese_card())],
        ),
        (
            "ProviderError",
            provider_errors().iter().map(json_of).collect(),
        ),
        ("PipelineStage", stages().iter().map(json_of).collect()),
        ("PipelineWarning", warnings().iter().map(json_of).collect()),
        (
            "PendingCapture",
            vec![
                json_of(PendingCapture {
                    card: english_card(),
                    warnings: warnings(),
                    profile_id: None,
                }),
                json_of(PendingCapture {
                    card: chinese_card(),
                    warnings: Vec::new(),
                    profile_id: Some("zh-de".into()),
                }),
            ],
        ),
        (
            "CaptureOrigin",
            [
                CaptureOrigin::Capture,
                CaptureOrigin::Curriculum,
                CaptureOrigin::AnkiImport,
            ]
            .iter()
            .map(json_of)
            .collect(),
        ),
        (
            "StoredCapture",
            vec![
                json_of(stored(1, english_card(), CaptureOrigin::Capture, true)),
                json_of(stored(2, chinese_card(), CaptureOrigin::Curriculum, false)),
                json_of(stored(3, chinese_card(), CaptureOrigin::AnkiImport, true)),
            ],
        ),
        (
            "SubmitResult",
            vec![
                json_of(SubmitResult {
                    capture_id: 7,
                    synced_to_anki: true,
                    note_ids: vec![1_761_912_000_001, 1_761_912_000_002],
                    anki_error: None,
                }),
                json_of(SubmitResult {
                    capture_id: 8,
                    synced_to_anki: false,
                    note_ids: Vec::new(),
                    anki_error: Some("could not reach AnkiConnect: connection refused".into()),
                }),
            ],
        ),
        (
            "SyncFailure",
            vec![json_of(SyncFailure {
                id: 8,
                error: "could not reach AnkiConnect: connection refused".into(),
            })],
        ),
        (
            "SyncReport",
            vec![
                json_of(SyncReport::default()),
                json_of(SyncReport {
                    attempted: 3,
                    synced: 2,
                    failed: vec![SyncFailure {
                        id: 8,
                        error: "AnkiConnect refused the note: deck was not found".into(),
                    }],
                }),
            ],
        ),
        (
            "AnkiStatus",
            vec![
                json_of(AnkiStatus {
                    online: true,
                    version: Some(6),
                    decks: vec!["Default".into(), "English::AutomatedVocabulary".into()],
                    models: vec!["Basic".into(), "AppleStyleFinalV3".into()],
                    error: None,
                }),
                json_of(AnkiStatus {
                    online: false,
                    version: None,
                    decks: Vec::new(),
                    models: Vec::new(),
                    error: Some("could not reach AnkiConnect: connection refused".into()),
                }),
            ],
        ),
        (
            "ConsentState",
            vec![json_of(ConsentState::default()), json_of(granted_consent())],
        ),
        (
            "ConsentAction",
            [
                ConsentAction::AnkiInstall,
                ConsentAction::AnkiconnectProvision,
            ]
            .iter()
            .map(json_of)
            .collect(),
        ),
        (
            "SetupStatus",
            vec![
                json_of(SetupStatus {
                    anki_installed: true,
                    ankiconnect_installed: false,
                    platform_supported: true,
                    anki_version: "24.06.3".into(),
                    consent: granted_consent(),
                }),
                json_of(SetupStatus {
                    anki_installed: false,
                    ankiconnect_installed: false,
                    platform_supported: false,
                    anki_version: "24.06.3".into(),
                    consent: ConsentState::default(),
                }),
            ],
        ),
        (
            "QueueItem",
            vec![
                json_of(QueueItem::Review {
                    card_id: 3,
                    due_date: instant("2026-10-31T12:10:00.500Z"),
                }),
                json_of(QueueItem::New {
                    hanzi: "学习".into(),
                    curriculum_order: 4,
                }),
            ],
        ),
        (
            "CardField",
            [
                CardField::Front,
                CardField::Back,
                CardField::ExampleSource,
                CardField::ExampleTarget,
                CardField::Example,
                CardField::Notes,
                CardField::Pinyin,
                CardField::HskLevel,
            ]
            .iter()
            .map(json_of)
            .collect(),
        ),
        (
            "FieldBinding",
            vec![json_of(FieldBinding::new(
                "Context Sentence",
                CardField::Example,
            ))],
        ),
        (
            "FieldMapping",
            vec![
                json_of(FieldMapping::apple_style_default()),
                json_of(FieldMapping::chinese_default()),
                json_of(custom_mapping()),
            ],
        ),
        (
            "HotkeyProfile",
            vec![json_of(chinese_profile()), json_of(french_profile())],
        ),
        (
            "TranslationConfig",
            vec![
                json_of(TranslationConfig::default()),
                json_of(custom_config().translation),
            ],
        ),
        (
            "AnkiConfig",
            vec![
                json_of(AnkiConfig::default()),
                json_of(custom_config().anki),
            ],
        ),
        (
            "AppConfig",
            vec![json_of(AppConfig::default()), json_of(custom_config())],
        ),
        (
            "CaptureErrorPayload",
            vec![json_of(CaptureErrorPayload {
                message: "Nothing to capture: the selected text is empty.".into(),
            })],
        ),
    ];
    entries
        .into_iter()
        .map(|(name, samples)| (name.to_string(), Value::Array(samples)))
        .collect()
}

// ---------------------------------------------------------------------------
// The commands and events, through the real invoke handler

/// `Ok` when `value` is exactly serde's JSON of a value of the Rust type
/// behind the TypeScript `returns`: it deserializes, and serializing that
/// again gives `value` back (no key missing, none extra, nothing defaulted).
fn conforms(returns: &str, value: &Value) -> Result<(), String> {
    fn exactly<T: Serialize + DeserializeOwned>(value: &Value) -> Result<(), String> {
        let parsed: T = serde_json::from_value(value.clone()).map_err(|err| err.to_string())?;
        match first_difference(value, &json_of(parsed), "$") {
            None => Ok(()),
            Some(at) => Err(format!("not a serialized value of the type, at {at}")),
        }
    }
    if let Some(element) = returns.strip_suffix("[]") {
        let items = value.as_array().ok_or("not an array")?;
        return items.iter().try_for_each(|item| conforms(element, item));
    }
    if let Some(inner) = returns.strip_suffix(" | null") {
        return if value.is_null() {
            Ok(())
        } else {
            conforms(inner, value)
        };
    }
    let kind = |ok: bool| {
        ok.then_some(())
            .ok_or_else(|| "a value of another kind".to_string())
    };
    match returns {
        "null" => kind(value.is_null()),
        "boolean" => kind(value.is_boolean()),
        "string" => kind(value.is_string()),
        "AppConfig" => exactly::<AppConfig>(value),
        "PendingCapture" => exactly::<PendingCapture>(value),
        "SubmitResult" => exactly::<SubmitResult>(value),
        "AnkiStatus" => exactly::<AnkiStatus>(value),
        "SetupStatus" => exactly::<SetupStatus>(value),
        "ConsentState" => exactly::<ConsentState>(value),
        "StoredCapture" => exactly::<StoredCapture>(value),
        "SyncReport" => exactly::<SyncReport>(value),
        "CaptureErrorPayload" => exactly::<CaptureErrorPayload>(value),
        other => Err(format!("no Rust type is known for {other}")),
    }
}

/// A value of another JSON type than `sample`, which no argument accepts.
fn wrong_value(sample: &Value) -> Value {
    if sample.is_boolean() {
        json!("not a boolean")
    } else {
        json!(true)
    }
}

type Results = BTreeMap<&'static str, (Value, Result<Value, Value>)>;

/// Runs a session that calls every command but `restart_app` once, each
/// with arguments as the frontend sends them, and checks every argument
/// name; returns the arguments and results, and the harness for its events.
fn run_every_command() -> (Results, Harness) {
    let harness = Harness::new();
    let mut results = Results::new();
    let mut call = |command: &'static str, args: Value| {
        let result = harness.invoke(command, args.clone());
        results.insert(command, (args, result.clone()));
        result.unwrap_or(Value::Null)
    };
    let settings = call("get_current_settings", json!({}));
    call("apply_settings", json!({ "newCfg": settings }));
    call("get_pending_payload", json!({}));
    // The first capture opens the approval window; the second reaches it as
    // `translation-payload`, and an empty one is a `capture-error`.
    harness.call(
        "capture_text",
        json!({ "text": "house", "profileId": null }),
    );
    harness.call("capture_text", json!({ "text": "   ", "profileId": null }));
    call(
        "capture_text",
        json!({ "text": "serendipity", "profileId": null }),
    );
    let pending = call("get_pending_payload", json!({}));
    let submitted = call(
        "execute_card_generation",
        json!({ "card": pending["card"] }),
    );
    call("discard_pending_payload", json!({}));
    call("anki_status", json!({}));
    call("anki_model_field_names", json!({ "model": "Basic" }));
    call("launch_anki_desktop", json!({}));
    call("get_setup_status", json!({}));
    // Before any consent: both install commands refuse at once (the hard
    // rule of docs/test-harness-strategy.md), which the assertions after the
    // session confirm.
    call("download_and_install_anki", json!({}));
    call("auto_provision_ankiconnect", json!({}));
    call("grant_consent", json!({ "action": "anki_install" }));
    call("check_ankiconnect_installed", json!({}));
    call("list_recent_captures", json!({ "limit": 10 }));
    call("sync_pending_captures", json!({}));
    call("delete_capture", json!({ "id": submitted["capture_id"] }));
    call("capture_in_progress", json!({}));

    // Each argument is read under its name: a value of the wrong type is
    // refused before the command runs, naming it.
    for &(_, command, names, _) in COMMANDS {
        for name in names {
            let (args, _) = &results[command];
            let mut wrong = args.clone();
            wrong[*name] = wrong_value(&args[*name]);
            let err = harness
                .invoke(command, wrong)
                .expect_err("a wrongly typed argument is refused");
            let expected = format!("invalid args `{name}` for command `{command}`");
            assert!(
                err.as_str().is_some_and(|err| err.starts_with(&expected)),
                "{command}: {err}"
            );
        }
    }
    let unknown = harness.invoke("no_such_command", json!({}));
    assert_eq!(
        unknown,
        Err(json!("Command no_such_command not found")),
        "the absence of this error is what proves a command exists"
    );
    assert!(
        harness.downloader.requests().is_empty(),
        "nothing was downloaded"
    );
    assert!(
        harness.runner.calls().len() <= 1,
        "nothing but launch_anki_desktop ran a program: {:?}",
        harness.runner.calls()
    );
    (results, harness)
}

#[test]
fn every_command_takes_its_arguments_and_returns_its_type() {
    let (results, harness) = run_every_command();
    for &(_, command, names, returns) in COMMANDS {
        if command == NOT_RUN {
            assert!(!results.contains_key(command));
            continue;
        }
        let (args, result) = &results[command];
        let mut sent: Vec<&str> = args
            .as_object()
            .expect("arguments are an object")
            .keys()
            .map(String::as_str)
            .collect();
        sent.sort_unstable();
        let mut expected: Vec<&str> = names.to_vec();
        expected.sort_unstable();
        assert_eq!(sent, expected, "{command} was called with its arguments");
        match result {
            Ok(value) => {
                if let Err(why) = conforms(returns, value) {
                    panic!("{command} returned {value}: {why}");
                }
            }
            Err(Value::String(err)) if MAY_FAIL.contains(&command) => {
                assert!(!err.contains("invalid args"), "{command}: {err}");
                assert!(!err.contains("missing required key"), "{command}: {err}");
            }
            Err(err) => panic!("{command} failed: {err}"),
        }
    }
    assert_eq!(results.len(), COMMANDS.len() - 1, "every other command ran");

    for &(event, payload) in EVENTS {
        let emitted = harness.events(event);
        assert!(!emitted.is_empty(), "the session emitted {event}");
        for value in &emitted {
            if let Err(why) = conforms(payload, value) {
                panic!("{event} carried {value}: {why}");
            }
        }
    }
}

/// Every provider name a `ProviderError` can carry to the approval window:
/// those of the standard registry the app captures with, for every
/// supported language.
fn provider_names() -> Vec<&'static str> {
    let registry = ProviderRegistry::standard(StandardProviders {
        contact_email: None,
        curriculum: Arc::new(CurriculumIndex::default()),
    });
    let mut names = Vec::new();
    for lang in SUPPORTED_LANGS {
        let set = registry.resolve(lang);
        names.push(set.translation.name());
        names.extend(set.rarity.as_ref().map(|provider| provider.name()));
        names.extend(set.example.as_ref().map(|provider| provider.name()));
        names.extend(set.tts.as_ref().map(|provider| provider.name()));
        names.extend(set.annotator.as_ref().map(|provider| provider.name()));
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// The parsed fixture this crate writes.
fn fixture() -> Value {
    let commands: Vec<Value> = COMMANDS
        .iter()
        .map(|&(function, command, args, returns)| {
            json!({ "function": function, "command": command, "args": args, "returns": returns })
        })
        .collect();
    let events: Vec<Value> = EVENTS
        .iter()
        .map(|&(name, payload)| json!({ "name": name, "payload": payload }))
        .collect();
    json!({
        "about": "Generated by src-tauri/tests/ipc_shapes.rs from the Rust types and the real \
                  invoke handler; regenerate with `UPDATE_IPC_SHAPES=1 cargo test -p \
                  vocab-automator --test ipc_shapes`, never edit by hand.",
        "commands": commands,
        "events": events,
        "providers": provider_names(),
        "shapes": shapes(),
    })
}

#[test]
fn the_fixture_is_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
    let expected = fixture();
    if std::env::var_os(UPDATE_ENV).is_some() {
        let text = serde_json::to_string_pretty(&expected).expect("JSON");
        fs::create_dir_all(path.parent().expect("a parent")).expect("fixture folder");
        fs::write(&path, format!("{text}\n")).expect("fixture written");
        return;
    }
    let committed = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    let committed: Value = serde_json::from_str(&committed).expect("the fixture parses");
    if let Some(at) = first_difference(&committed, &expected, "$") {
        panic!(
            "{} is stale (first difference at {at}); regenerate it with \
             `{UPDATE_ENV}=1 cargo test -p vocab-automator --test ipc_shapes`",
            path.display()
        );
    }
}

#[test]
fn every_result_and_payload_type_has_conforming_samples() {
    let shapes = shapes();
    let types = COMMANDS
        .iter()
        .map(|&(_, _, _, returns)| returns)
        .chain(EVENTS.iter().map(|&(_, payload)| payload));
    for returns in types {
        let name = returns.trim_end_matches("[]").trim_end_matches(" | null");
        if matches!(name, "null" | "boolean" | "string" | "never") {
            continue;
        }
        let samples = shapes[name].as_array().expect("samples");
        assert!(!samples.is_empty(), "no samples of {name}");
        for sample in samples {
            if let Err(why) = conforms(name, sample) {
                panic!("a {name} sample is no {name}: {why}");
            }
        }
    }
}
