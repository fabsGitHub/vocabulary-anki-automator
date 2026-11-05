//! Learn's IPC surface, pinned for the frontend (docs/architecture.md § 10
//! and § 11).
//!
//! The commands and the TypeScript side (apps/learn/src/platform/types.ts,
//! `tauri.ts`, the web adapter) are written separately, so this test writes
//! what the Rust side actually does into a fixture the Vitest suite
//! `src/platform/ipc_shapes.test.ts` checks the TypeScript side against:
//!
//! - `shapes`: serde's JSON of representative values of every Rust type
//!   behind a type of `types.ts` (keyed by the TypeScript name), with every
//!   enum variant and both sides of every `Option` among them. Vitest checks
//!   each value against a schema that is compile-time equal to the
//!   TypeScript type, exact keys included, and through the web adapter's
//!   parsers.
//! - `commands`: every command of § 10 with the `PlatformAdapter` method that
//!   calls it, its camelCase argument names and its result type, as the
//!   TypeScript side spells them. All of it is checked here against the real
//!   invoke handler: each command runs through the IPC layer with exactly
//!   these arguments, each argument is shown to be read under its name (a
//!   value of the wrong type is refused naming it), and each result is
//!   exactly the serialization of the Rust type the TypeScript name stands
//!   for.
//!
//! The fixture is checked by default and rewritten with
//! `UPDATE_IPC_SHAPES=1 cargo test -p vocab-learn --test ipc_shapes`.
//! Values are compared parsed (floats up to their last bits), so neither
//! formatting nor another platform's maths library counts as a difference.

mod common;

use std::{collections::BTreeMap, fs, path::PathBuf};

use anki_bridge::{CardField, FieldBinding, FieldMapping};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use common::{
    capture, chinese_capture, first_difference, learned, start_time, Harness, Options, COMMANDS,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Map, Value};
use vocab_core::{
    gamification::{StatsSummary, StreakSummary},
    lessons::{Lesson, LessonCard},
    quiz::{
        AudioPrompt, ChoiceDirection, ChoiceOption, Exercise, ExerciseAnswer, ExerciseKind,
        ListeningChoiceExercise, ListeningSentenceExercise, MultipleChoiceExercise, PoolEntry,
        SentenceBuilderExercise, Tile,
    },
    srs::{DailyPromptState, QueueItem, RatingPreview, ReviewRating, SrsScheduler},
    store::{DailySessionRow, SrsStateRow},
    CaptureOrigin, Card, ScriptMetadata, ScriptVariant, StoredCapture,
};
use vocab_learn_lib::{
    anki::{AnkiSyncFailure, AnkiSyncReport},
    session::ReviewResult,
    settings::{AnkiSettings, LearnSettings, MeaningLang},
};

/// The fixture, relative to this crate.
const FIXTURE: &str = "../src/platform/__fixtures__/ipc_shapes.json";
/// Set to rewrite the fixture instead of checking it.
const UPDATE_ENV: &str = "UPDATE_IPC_SHAPES";

/// What each command resolves to, in the TypeScript spelling of § 10.
const RETURNS: &[(&str, &str)] = &[
    ("get_daily_prompt_state", "DailyPromptState"),
    ("submit_daily_quota", "DailySession"),
    ("dismiss_daily_prompt", "DailySession"),
    ("reopen_daily_prompt", "null"),
    ("get_daily_queue", "QueueItem[]"),
    ("start_new_item", "number"),
    ("start_new_capture", "number"),
    ("get_card", "StoredCapture"),
    ("preview_ratings", "RatingPreview[]"),
    ("submit_review_rating", "ReviewResult"),
    ("list_learned_cards", "LearnedCard[]"),
    ("next_exercise", "Exercise"),
    ("check_exercise", "boolean"),
    ("record_exercise_result", "number"),
    ("list_lessons", "Lesson[]"),
    ("complete_lesson", "number"),
    ("get_stats", "StatsSummary"),
    ("get_card_audio", "string | null"),
    ("synthesize_audio", "string | null"),
    ("get_settings", "LearnSettings"),
    ("save_settings", "null"),
    ("anki_export", "AnkiSyncReport"),
    ("anki_import", "AnkiSyncReport"),
];

fn returns(command: &str) -> &'static str {
    RETURNS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|&(_, returns)| returns)
        .unwrap_or_else(|| panic!("no result type for {command}"))
}

fn json_of<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).expect("serializable")
}

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("an RFC 3339 instant")
        .with_timezone(&Utc)
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("a date")
}

// ---------------------------------------------------------------------------
// Representative values

fn chinese_metadata() -> ScriptMetadata {
    ScriptMetadata::Chinese {
        pinyin: "xué xí".into(),
        hsk_level: Some(1),
        script_variant: ScriptVariant::Simplified,
    }
}

/// An English card with everything set, captured at an instant with
/// milliseconds.
fn english_card() -> Card {
    Card {
        front: "serendipity".into(),
        back: "glücklicher Zufall".into(),
        example_source: "It was pure serendipity.".into(),
        example_target: "Es war reiner Zufall.".into(),
        notes: "Rare word.".into(),
        tags: vec!["vocab-automator".into(), "en-de".into()],
        source_lang: "en".into(),
        target_lang: "de".into(),
        captured_at: instant("2026-10-31T12:00:00.123Z"),
        audio_front: Some(b"ID3 mp3 bytes".to_vec()),
        script_metadata: ScriptMetadata::None,
    }
}

/// A Chinese card outside the curriculum, without audio, captured on a whole
/// second (serde then writes no fraction).
fn chinese_card() -> Card {
    Card {
        front: "學習".into(),
        back: "to study".into(),
        example_source: String::new(),
        example_target: String::new(),
        notes: String::new(),
        tags: Vec::new(),
        source_lang: "zh-CN".into(),
        target_lang: "en".into(),
        captured_at: instant("2026-10-31T12:00:00Z"),
        audio_front: None,
        script_metadata: ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: None,
            script_variant: ScriptVariant::Traditional,
        },
    }
}

fn open_session() -> DailySessionRow {
    DailySessionRow {
        session_date: date("2026-10-31"),
        requested_new_count: 5,
        reviews_due_count: 3,
        reviews_completed: 1,
        new_items_completed: 0,
        completed_at: None,
    }
}

fn completed_session() -> DailySessionRow {
    DailySessionRow {
        reviews_completed: 3,
        new_items_completed: 5,
        completed_at: Some(instant("2026-10-31T12:30:00.250Z")),
        ..open_session()
    }
}

fn streak() -> StreakSummary {
    StreakSummary {
        current: 3,
        longest: 7,
        freezes_available: 1,
        completed_today: true,
    }
}

/// A card's state before and after its first review.
fn srs_states() -> (SrsStateRow, SrsStateRow) {
    let scheduler = SrsScheduler::default();
    let now = instant("2026-10-31T12:00:00.123Z");
    let new = scheduler.initial_state(3, now);
    let reviewed = scheduler
        .review(&new, ReviewRating::Good, now, date("2026-10-31"))
        .next;
    (new, reviewed)
}

fn options(texts: &[(i64, &str)]) -> Vec<ChoiceOption> {
    texts
        .iter()
        .map(|&(card_id, text)| ChoiceOption {
            card_id,
            text: text.into(),
        })
        .collect()
}

fn tiles(texts: &[&str]) -> Vec<Tile> {
    texts
        .iter()
        .zip(0..)
        .map(|(text, id)| Tile {
            id,
            text: (*text).into(),
        })
        .collect()
}

/// One exercise of every kind, each consistent with its own solution.
fn exercises() -> Vec<Exercise> {
    vec![
        Exercise::MultipleChoice(MultipleChoiceExercise {
            card_id: 3,
            direction: ChoiceDirection::FrontToBack,
            prompt: "学习".into(),
            prompt_lang: "zh-CN".into(),
            option_lang: "de".into(),
            options: options(&[(5, "Freund"), (3, "lernen"), (4, "Lehrer")]),
            correct_index: 1,
        }),
        Exercise::SentenceBuilder(SentenceBuilderExercise {
            card_id: 3,
            sentence_card_id: 4,
            prompt: "Ich lerne Chinesisch.".into(),
            sentence: "我学习中文。".into(),
            sentence_lang: "zh-CN".into(),
            tiles: tiles(&["中文", "我", "朋友", "学习", "。"]),
            solution: vec![1, 3, 0, 4],
        }),
        Exercise::ListeningChoice(ListeningChoiceExercise {
            card_id: 3,
            audio: AudioPrompt {
                audio_card_id: Some(3),
                text: "学习".into(),
                lang: "zh-CN".into(),
            },
            options: options(&[(3, "学习"), (5, "朋友")]),
            correct_index: 0,
        }),
        Exercise::ListeningSentence(ListeningSentenceExercise {
            card_id: 3,
            sentence_card_id: 3,
            audio: AudioPrompt {
                audio_card_id: None,
                text: "我学习中文。".into(),
                lang: "zh-CN".into(),
            },
            sentence: "我学习中文。".into(),
            tiles: tiles(&["学习", "中文", "我"]),
            solution: vec![2, 0, 1],
        }),
    ]
}

fn lesson_cards() -> Vec<LessonCard> {
    vec![
        LessonCard {
            card_id: 7,
            word: "serendipity".into(),
            translation: "glücklicher Zufall".into(),
            sentence_source: "It was pure serendipity.".into(),
            sentence_target: "Es war reiner Zufall.".into(),
            cloze: "It was pure ____.".into(),
            pinyin: None,
        },
        LessonCard {
            card_id: 8,
            word: "学习".into(),
            translation: "lernen".into(),
            sentence_source: "我学习中文。".into(),
            sentence_target: "Ich lerne Chinesisch.".into(),
            cloze: "我____中文。".into(),
            pinyin: Some("xué xí".into()),
        },
    ]
}

/// `shapes`: the TypeScript type name and its samples.
fn shapes() -> Map<String, Value> {
    let (new_state, reviewed_state) = srs_states();
    let scheduler = SrsScheduler::default();
    let previews: Vec<RatingPreview> =
        scheduler.preview(&new_state, instant("2026-10-31T12:00:00.123Z"));
    let stored = |id, card, origin, synced_to_anki| StoredCapture {
        id,
        card,
        origin,
        synced_to_anki,
    };
    let custom_mapping = FieldMapping {
        fields: vec![
            FieldBinding::new("Front", CardField::Front),
            FieldBinding::new("Back", CardField::Back),
        ],
        audio_field: None,
    };
    let entries: Vec<(&str, Vec<Value>)> = vec![
        (
            "ScriptMetadata",
            vec![
                json_of(ScriptMetadata::None),
                json_of(chinese_metadata()),
                json_of(chinese_card().script_metadata),
            ],
        ),
        (
            "Card",
            vec![json_of(english_card()), json_of(chinese_card())],
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
            "DailySession",
            vec![json_of(open_session()), json_of(completed_session())],
        ),
        (
            "DailyPromptState",
            vec![
                json_of(DailyPromptState {
                    needs_prompt: true,
                    today: date("2026-10-31"),
                    session: None,
                }),
                json_of(DailyPromptState {
                    needs_prompt: false,
                    today: date("2026-10-31"),
                    session: Some(completed_session()),
                }),
            ],
        ),
        (
            "QueueItem",
            vec![
                json_of(QueueItem::Review {
                    card_id: 3,
                    due_date: instant("2026-10-31T12:10:00Z"),
                }),
                json_of(QueueItem::New {
                    hanzi: "学习".into(),
                    curriculum_order: 4,
                }),
                json_of(QueueItem::NewCapture { card_id: 9 }),
            ],
        ),
        ("Rating", ReviewRating::ALL.iter().map(json_of).collect()),
        ("RatingPreview", previews.iter().map(json_of).collect()),
        (
            "SrsState",
            vec![json_of(&new_state), json_of(&reviewed_state)],
        ),
        ("StreakSummary", vec![json_of(streak())]),
        (
            "StatsSummary",
            vec![json_of(StatsSummary {
                xp_total: 1250,
                xp_today: 65,
                streak: streak(),
                learned_count: 42,
                due_today: 7,
            })],
        ),
        (
            "ReviewResult",
            vec![
                json_of(ReviewResult {
                    next: reviewed_state.clone(),
                    xp_awarded: 65,
                    session: Some(completed_session()),
                    session_completed: true,
                    streak: streak(),
                }),
                json_of(ReviewResult {
                    next: reviewed_state,
                    xp_awarded: 10,
                    session: None,
                    session_completed: false,
                    streak: StreakSummary {
                        current: 0,
                        longest: 0,
                        freezes_available: 0,
                        completed_today: false,
                    },
                }),
            ],
        ),
        (
            "LearnedCard",
            vec![
                json_of(PoolEntry {
                    card_id: 3,
                    front: "学习".into(),
                    back: "lernen".into(),
                    pinyin: Some("xué xí".into()),
                    hsk_level: Some(1),
                    source_lang: "zh-CN".into(),
                    target_lang: "de".into(),
                    example_source: "我学习中文。".into(),
                    example_target: "Ich lerne Chinesisch.".into(),
                    script_kind: "chinese".into(),
                    has_audio: true,
                }),
                json_of(PoolEntry {
                    card_id: 7,
                    front: "serendipity".into(),
                    back: String::new(),
                    pinyin: None,
                    hsk_level: None,
                    source_lang: "en".into(),
                    target_lang: "de".into(),
                    example_source: String::new(),
                    example_target: String::new(),
                    script_kind: "none".into(),
                    has_audio: false,
                }),
            ],
        ),
        (
            "ExerciseKind",
            ExerciseKind::ALL.iter().map(json_of).collect(),
        ),
        (
            "ChoiceDirection",
            [
                ChoiceDirection::FrontToBack,
                ChoiceDirection::BackToFront,
                ChoiceDirection::FrontToPinyin,
            ]
            .iter()
            .map(json_of)
            .collect(),
        ),
        ("Exercise", exercises().iter().map(json_of).collect()),
        (
            "ExerciseAnswer",
            vec![
                json_of(ExerciseAnswer::Choice { index: 1 }),
                json_of(ExerciseAnswer::Order {
                    tile_ids: vec![1, 3, 0, 4],
                }),
            ],
        ),
        (
            "Lesson",
            vec![json_of(Lesson {
                id: "lesson-7".into(),
                title: "EN → DE · Lesson 1".into(),
                source_lang: "en".into(),
                target_lang: "de".into(),
                cards: lesson_cards(),
            })],
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
            "FieldMapping",
            vec![
                json_of(FieldMapping::chinese_default()),
                json_of(FieldMapping::apple_style_default()),
                json_of(&custom_mapping),
            ],
        ),
        (
            "LearnSettings",
            vec![
                json_of(LearnSettings::default()),
                json_of(LearnSettings {
                    meaning_lang: MeaningLang::De,
                    anki: AnkiSettings {
                        connect_url: "http://localhost:8765".into(),
                        deck_name: "Mine::Chinese".into(),
                        model_name: "Basic".into(),
                        field_mapping: custom_mapping,
                        create_reverse: true,
                    },
                }),
            ],
        ),
        (
            "AnkiSyncReport",
            vec![
                json_of(AnkiSyncReport::default()),
                json_of(AnkiSyncReport {
                    attempted: 4,
                    succeeded: 1,
                    skipped: 1,
                    failed: vec![
                        AnkiSyncFailure {
                            card_id: Some(4),
                            front: "好".into(),
                            error: "AnkiConnect refused the note: model was not found".into(),
                        },
                        AnkiSyncFailure {
                            card_id: None,
                            front: String::new(),
                            error: "The note has hanzi on neither side.".into(),
                        },
                    ],
                }),
            ],
        ),
    ];
    entries
        .into_iter()
        .map(|(name, samples)| (name.to_string(), Value::Array(samples)))
        .collect()
}

// ---------------------------------------------------------------------------
// The commands, through the real invoke handler

/// `Ok` when `value` is exactly serde's JSON of a value of the Rust type
/// behind the TypeScript `returns`: it deserializes, and serializing that
/// again gives `value` back (no key missing, none extra, nothing defaulted).
fn conforms(returns: &str, value: &Value) -> Result<(), String> {
    fn exactly<T: Serialize + DeserializeOwned>(value: &Value) -> Result<(), String> {
        let parsed: T = serde_json::from_value(value.clone()).map_err(|err| err.to_string())?;
        let again = json_of(parsed);
        match first_difference(value, &again, "$") {
            None => Ok(()),
            Some(at) => Err(format!("not a serialized value of the type, at {at}")),
        }
    }
    if let Some(element) = returns.strip_suffix("[]") {
        let items = value.as_array().ok_or("not an array")?;
        return items.iter().try_for_each(|item| conforms(element, item));
    }
    let kind = |ok: bool| {
        ok.then_some(())
            .ok_or_else(|| "a value of another kind".to_string())
    };
    match returns {
        "null" => kind(value.is_null()),
        "boolean" => kind(value.is_boolean()),
        "number" => kind(value.is_i64()),
        "string | null" => kind(value.is_string() || value.is_null()),
        "DailyPromptState" => exactly::<DailyPromptState>(value),
        "DailySession" => exactly::<DailySessionRow>(value),
        "QueueItem" => exactly::<QueueItem>(value),
        "StoredCapture" => exactly::<StoredCapture>(value),
        "RatingPreview" => exactly::<RatingPreview>(value),
        "ReviewResult" => exactly::<ReviewResult>(value),
        "LearnedCard" => exactly::<PoolEntry>(value),
        "Exercise" => exactly::<Exercise>(value),
        "Lesson" => exactly::<Lesson>(value),
        "StatsSummary" => exactly::<StatsSummary>(value),
        "LearnSettings" => exactly::<LearnSettings>(value),
        "AnkiSyncReport" => exactly::<AnkiSyncReport>(value),
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

/// Runs a session that calls every command once, each with arguments as the
/// frontend sends them; returns the arguments and the result of each.
fn run_every_command() -> BTreeMap<&'static str, (Value, Result<Value, Value>)> {
    let harness = Harness::with(Options {
        before_start: Some(Box::new(|store| {
            // Learned vocabulary for the exercises, and a capture whose
            // sentence makes a lesson.
            learned(
                store,
                &chinese_capture("学生", "Student", "xué sheng", ("我是学生。", "")),
                start_time() - Duration::days(1),
            );
            let mut lesson = Card::new("house", "Haus", "en", "de", start_time());
            lesson.example_source = "The house is big.".into();
            lesson.example_target = "Das Haus ist groß.".into();
            capture(store, &lesson);
        })),
        ..Options::default()
    });
    let mut results = BTreeMap::new();
    let mut call = |command: &'static str, args: Value| {
        let result = harness.invoke(command, args.clone());
        results.insert(command, (args, result.clone()));
        result.unwrap_or(Value::Null)
    };
    call("get_daily_prompt_state", json!({}));
    call("submit_daily_quota", json!({ "newCount": 2 }));
    call("dismiss_daily_prompt", json!({}));
    call("reopen_daily_prompt", json!({}));
    let queue = call("get_daily_queue", json!({}));
    let capture_id = queue
        .as_array()
        .and_then(|items| items.iter().find(|item| item["kind"] == "new_capture"))
        .map(|item| item["card_id"].clone())
        .expect("the unstarted lesson capture is offered first");
    call("start_new_capture", json!({ "cardId": capture_id }));
    let card_id = call("start_new_item", json!({ "hanzi": "我" }));
    call("get_card", json!({ "cardId": card_id }));
    call("preview_ratings", json!({ "cardId": card_id }));
    call(
        "submit_review_rating",
        json!({ "cardId": card_id, "rating": 3 }),
    );
    call("list_learned_cards", json!({}));
    let exercise = call(
        "next_exercise",
        json!({ "cardId": card_id, "kind": "multiple_choice", "seed": 7 }),
    );
    let index = exercise["correct_index"].clone();
    call(
        "check_exercise",
        json!({ "exercise": exercise, "answer": { "kind": "choice", "index": index } }),
    );
    call(
        "record_exercise_result",
        json!({ "cardId": card_id, "kind": "multiple_choice", "correct": true }),
    );
    let lessons = call("list_lessons", json!({}));
    call("complete_lesson", json!({ "lessonId": lessons[0]["id"] }));
    call("get_stats", json!({}));
    call("get_card_audio", json!({ "cardId": card_id }));
    call(
        "synthesize_audio",
        json!({ "text": "我是学生。", "lang": "zh-CN" }),
    );
    let settings = call("get_settings", json!({}));
    call("save_settings", json!({ "settings": settings }));
    call("anki_export", json!({}));
    call("anki_import", json!({ "deck": "Chinese::HSK" }));

    // Each argument is read under its name: a value of the wrong type is
    // refused before the command runs, naming it.
    for &(_, command, names) in COMMANDS {
        let (args, _) = &results[command];
        for name in names {
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
    results
}

#[test]
fn every_command_takes_its_arguments_and_returns_its_type() {
    let results = run_every_command();
    assert_eq!(
        results.len(),
        COMMANDS.len(),
        "every command of the table ran once"
    );
    for &(_, command, names) in COMMANDS {
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
                if let Err(why) = conforms(returns(command), value) {
                    panic!("{command} returned {value}: {why}");
                }
            }
            // Anki is unreachable in the harness: the import fails, but as
            // the command's own error, after Tauri took its arguments.
            Err(Value::String(err)) if command == "anki_import" => {
                assert!(err.contains("AnkiConnect"), "{err}");
            }
            Err(err) => panic!("{command} failed: {err}"),
        }
    }
}

/// The parsed fixture this crate writes.
fn fixture() -> Value {
    let commands: Vec<Value> = COMMANDS
        .iter()
        .map(|&(method, command, args)| {
            json!({ "method": method, "command": command, "args": args, "returns": returns(command) })
        })
        .collect();
    json!({
        "about": "Generated by apps/learn/src-tauri/tests/ipc_shapes.rs from the Rust types and \
                  the real invoke handler; regenerate with `UPDATE_IPC_SHAPES=1 cargo test -p \
                  vocab-learn --test ipc_shapes`, never edit by hand.",
        "commands": commands,
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
             `{UPDATE_ENV}=1 cargo test -p vocab-learn --test ipc_shapes`",
            path.display()
        );
    }
}

#[test]
fn every_rust_type_behind_a_command_result_has_samples() {
    let shapes = shapes();
    for &(command, returns) in RETURNS {
        let name = returns.trim_end_matches("[]");
        if matches!(name, "null" | "boolean" | "number" | "string | null") {
            continue;
        }
        let samples = shapes[name].as_array().expect("samples");
        assert!(!samples.is_empty(), "{command}: no samples of {name}");
        for sample in samples {
            if let Err(why) = conforms(name, sample) {
                panic!("a {name} sample is no {name}: {why}");
            }
        }
    }
}
