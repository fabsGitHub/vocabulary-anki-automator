//! Learn's practice commands through the IPC layer (Milestones M and I):
//! quizzes drawn from the learned vocabulary only, exercise results, lessons
//! and their once-a-day XP, and card audio, which is stored or synthesized
//! without holding the database across the TTS request.

mod common;

use std::{
    collections::{BTreeSet, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration as Elapsed, Instant},
};

use chrono::Duration;
use common::{b64, capture, chinese_capture, fake_audio, learned, start_time, Harness, Options};
use serde_json::{json, Value};
use vocab_core::{
    language::{cjk_chars, is_cjk_char},
    store::Store,
    CaptureOrigin, Card,
};

const KINDS: [&str; 4] = [
    "multiple_choice",
    "sentence_builder",
    "listening_choice",
    "listening_sentence",
];

fn exercise(harness: &Harness, card_id: i64, kind: &str, seed: u32) -> Result<Value, Value> {
    harness.invoke(
        "next_exercise",
        json!({ "cardId": card_id, "kind": kind, "seed": seed }),
    )
}

/// Every text an exercise shows or speaks, and every card id it refers to.
fn shown(exercise: &Value) -> (Vec<String>, Vec<i64>) {
    let mut texts = Vec::new();
    let mut ids = vec![exercise["card_id"].as_i64().unwrap()];
    for key in ["prompt", "sentence"] {
        if let Some(text) = exercise[key].as_str() {
            texts.push(text.to_string());
        }
    }
    for option in exercise["options"].as_array().into_iter().flatten() {
        texts.push(option["text"].as_str().unwrap().to_string());
        ids.push(option["card_id"].as_i64().unwrap());
    }
    for tile in exercise["tiles"].as_array().into_iter().flatten() {
        texts.push(tile["text"].as_str().unwrap().to_string());
    }
    if let Some(id) = exercise["sentence_card_id"].as_i64() {
        ids.push(id);
    }
    if exercise["audio"].is_object() {
        texts.push(exercise["audio"]["text"].as_str().unwrap().to_string());
        if let Some(id) = exercise["audio"]["audio_card_id"].as_i64() {
            ids.push(id);
        }
    }
    (texts, ids)
}

/// The leak test of Milestone M1, through the IPC layer: the database holds
/// a curriculum full of words the learner has not learned, Automator
/// captures that were never scheduled, and the trap word 学生, made of a
/// learned and an unlearned character. No exercise may show or refer to any
/// of them. What counts as learned is what this test introduced, never what
/// the pool under test says.
#[test]
fn exercises_only_show_learned_vocabulary() {
    let harness = Harness::with(Options {
        before_start: Some(Box::new(|store| {
            capture(
                store,
                &chinese_capture(
                    "朋友",
                    "Freund",
                    "péng you",
                    ("我们是朋友。", "Wir sind Freunde."),
                ),
            );
            capture(
                store,
                &chinese_capture(
                    "银行",
                    "Bank",
                    "yín háng",
                    ("我去银行。", "Ich gehe zur Bank."),
                ),
            );
            learned(
                store,
                &Card::new("Haus", "house", "de", "en", start_time()),
                start_time(),
            );
        })),
        ..Options::default()
    });
    harness.call("submit_daily_quota", json!({ "newCount": 5 }));
    let words = ["我", "你", "好", "学习", "中文"];
    let mut learned_ids: HashSet<i64> = words.iter().map(|word| harness.learn(word, 3)).collect();
    learned_ids.insert(3);
    let learned_hanzi: HashSet<char> = words.iter().flat_map(|word| cjk_chars(word)).collect();

    let pool = harness.call("list_learned_cards", json!({}));
    let pool_ids: HashSet<i64> = pool
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["card_id"].as_i64().unwrap())
        .collect();
    assert_eq!(pool_ids, learned_ids);

    let mut generated = BTreeSet::new();
    for &card_id in &learned_ids {
        for kind in KINDS {
            for seed in 0..40 {
                let Ok(exercise) = exercise(&harness, card_id, kind, seed) else {
                    continue;
                };
                generated.insert(kind);
                let (texts, ids) = shown(&exercise);
                for text in &texts {
                    let unlearned: Vec<char> = text
                        .chars()
                        .filter(|c| is_cjk_char(*c) && !learned_hanzi.contains(c))
                        .collect();
                    assert!(
                        unlearned.is_empty(),
                        "{kind} for card {card_id} (seed {seed}) shows {text:?}"
                    );
                }
                for id in ids {
                    assert!(
                        learned_ids.contains(&id),
                        "{kind} for card {card_id} (seed {seed}) uses card {id}"
                    );
                }
            }
        }
    }
    assert_eq!(
        generated,
        KINDS.into_iter().collect(),
        "every kind was tried"
    );

    for unlearned in [1, 2] {
        assert_eq!(
            harness.call_err(
                "next_exercise",
                json!({ "cardId": unlearned, "kind": "multiple_choice", "seed": 1 })
            ),
            format!("No exercise: card {unlearned} is not learned vocabulary.")
        );
    }
}

#[test]
fn answers_are_checked_and_only_correct_ones_earn_xp() {
    let harness = Harness::new();
    harness.call("submit_daily_quota", json!({ "newCount": 3 }));
    let wo = harness.learn("我", 3);
    harness.learn("你", 3);
    harness.learn("好", 3);
    let xp_before = harness.call("get_stats", json!({}))["xp_total"]
        .as_i64()
        .unwrap();

    let exercise = harness.call(
        "next_exercise",
        json!({ "cardId": wo, "kind": "multiple_choice", "seed": 7 }),
    );
    assert_eq!(
        exercise,
        harness.call(
            "next_exercise",
            json!({ "cardId": wo, "kind": "multiple_choice", "seed": 7 })
        ),
        "the same seed gives the same exercise"
    );
    let correct = exercise["correct_index"].as_u64().unwrap();
    let wrong = (correct + 1) % exercise["options"].as_array().unwrap().len() as u64;
    let check = |answer: Value| {
        harness.call(
            "check_exercise",
            json!({ "exercise": exercise, "answer": answer }),
        )
    };
    assert_eq!(check(json!({ "kind": "choice", "index": correct })), true);
    assert_eq!(check(json!({ "kind": "choice", "index": wrong })), false);
    assert_eq!(check(json!({ "kind": "order", "tile_ids": [0] })), false);

    let record = |correct: bool| {
        harness.call(
            "record_exercise_result",
            json!({ "cardId": wo, "kind": "multiple_choice", "correct": correct }),
        )
    };
    assert_eq!(record(true), 5);
    assert_eq!(record(false), 0);
    let stats = harness.call("get_stats", json!({}));
    assert_eq!(stats["xp_total"].as_i64().unwrap(), xp_before + 5);

    let unlearned = capture(
        &harness.open_store(),
        &Card::new("Haus", "house", "de", "en", start_time()),
    );
    assert_eq!(
        harness.call_err(
            "record_exercise_result",
            json!({ "cardId": unlearned, "kind": "listening_choice", "correct": true })
        ),
        format!("Card {unlearned} is not learned vocabulary.")
    );
    let err = harness.call_err(
        "next_exercise",
        json!({ "cardId": wo, "kind": "essay", "seed": 1 }),
    );
    assert!(err.contains("unknown variant `essay`"), "{err}");
}

/// Six English clipboard captures with sentences make two lessons; a
/// curriculum word and a capture without a translated sentence make none.
fn with_captured_sentences() -> Options {
    Options {
        before_start: Some(Box::new(|store| {
            for (index, word) in ["cat", "dog", "bird", "fish", "horse", "mouse"]
                .iter()
                .enumerate()
            {
                let mut card = Card::new(
                    *word,
                    format!("{word} (de)"),
                    "en",
                    "de",
                    start_time() - Duration::days(10 - index as i64),
                );
                card.example_source = format!("The {word} sleeps.");
                card.example_target = format!("Die {word} schläft.");
                capture(store, &card);
            }
            let mut untranslated = Card::new("tree", "Baum", "en", "de", start_time());
            untranslated.example_source = "A tree.".into();
            capture(store, &untranslated);
        })),
        ..Options::default()
    }
}

#[test]
fn lessons_come_from_clipboard_captures_and_pay_once_a_day() {
    let harness = Harness::with(with_captured_sentences());
    harness.call("submit_daily_quota", json!({ "newCount": 1 }));
    harness.learn("我", 3);

    let lessons = harness.call("list_lessons", json!({}));
    let lessons = lessons.as_array().unwrap();
    assert_eq!(lessons.len(), 2);
    assert_eq!(lessons[0]["id"], "lesson-1");
    assert_eq!(lessons[0]["title"], "EN → DE · Lesson 1");
    assert_eq!(lessons[0]["cards"].as_array().unwrap().len(), 5);
    assert_eq!(
        lessons[0]["cards"][0],
        json!({
            "card_id": 1,
            "word": "cat",
            "translation": "cat (de)",
            "sentence_source": "The cat sleeps.",
            "sentence_target": "Die cat schläft.",
            "cloze": "The ____ sleeps.",
            "pinyin": null
        })
    );
    assert_eq!(lessons[1]["id"], "lesson-6");
    let xp_before = harness.call("get_stats", json!({}))["xp_total"]
        .as_i64()
        .unwrap();

    let complete = |id: &str| harness.call("complete_lesson", json!({ "lessonId": id }));
    assert_eq!(complete("lesson-1"), 20);
    assert_eq!(complete("lesson-1"), 0, "once per lesson per day");
    assert_eq!(complete("lesson-6"), 20, "another lesson pays on its own");
    harness.clock.advance(Duration::days(1));
    assert_eq!(complete("lesson-1"), 20, "and again the next day");
    assert_eq!(
        harness.call_err("complete_lesson", json!({ "lessonId": "lesson-7" })),
        "There is no lesson \"lesson-7\"."
    );

    let stats = harness.call("get_stats", json!({}));
    assert_eq!(stats["xp_total"].as_i64().unwrap(), xp_before + 60);
    assert_eq!(stats["xp_today"], 20);
}

#[test]
fn card_audio_is_the_stored_recording_or_synthesized_once() {
    let harness = Harness::with(Options {
        before_start: Some(Box::new(|store| {
            let mut card = Card::new("Haus", "house", "de", "en", start_time());
            card.audio_front = Some(b"recorded".to_vec());
            capture(store, &card);
        })),
        ..Options::default()
    });

    assert_eq!(
        harness.call("get_card_audio", json!({ "cardId": 1 })),
        b64(b"recorded")
    );
    assert!(harness.tts.calls().is_empty(), "nothing to synthesize");

    let wo = harness.call("start_new_item", json!({ "hanzi": "我" }));
    let expected = b64(&fake_audio("zh-CN", "我"));
    assert_eq!(
        harness.call("get_card_audio", json!({ "cardId": wo })),
        expected
    );
    assert_eq!(
        harness.call("get_card_audio", json!({ "cardId": wo })),
        expected
    );
    assert_eq!(
        harness.tts.calls(),
        ["zh-CN 我"],
        "synthesized once, then kept"
    );
    assert_eq!(
        harness.call("get_card", json!({ "cardId": wo }))["card"]["audio_front"],
        expected
    );

    harness.tts.fail();
    let ni = harness.call("start_new_item", json!({ "hanzi": "你" }));
    assert_eq!(
        harness.call("get_card_audio", json!({ "cardId": ni })),
        Value::Null
    );
    assert_eq!(
        harness.call("get_card", json!({ "cardId": ni }))["card"]["audio_front"],
        Value::Null
    );
    assert_eq!(
        harness.call_err("get_card_audio", json!({ "cardId": 999 })),
        "There is no card 999."
    );
}

#[test]
fn sentences_are_synthesized_in_the_canonical_language() {
    let harness = Harness::new();

    assert_eq!(
        harness.call(
            "synthesize_audio",
            json!({ "text": " 我学习中文。 ", "lang": "zh_cn" })
        ),
        b64(&fake_audio("zh-CN", "我学习中文。"))
    );
    assert_eq!(
        harness.call("synthesize_audio", json!({ "text": "  ", "lang": "en" })),
        Value::Null
    );
    assert_eq!(harness.tts.calls(), ["zh-CN 我学习中文。"]);

    harness.tts.fail();
    assert_eq!(
        harness.call("synthesize_audio", json!({ "text": "Hallo", "lang": "de" })),
        Value::Null
    );
}

/// § 6.3: TTS is a network request, and no transaction may be open while it
/// runs. Here "Automator" writes to the database in the middle of the
/// request; with a transaction held, that write would wait out SQLite's
/// five-second busy timeout and fail.
#[test]
fn the_database_stays_writable_while_audio_is_synthesized() {
    let harness = Harness::new();
    let wo = harness.call("start_new_item", json!({ "hanzi": "我" }));
    let outcome: Arc<Mutex<Option<(bool, Elapsed)>>> = Arc::default();
    let (db, seen) = (harness.db_path(), outcome.clone());
    harness.tts.while_speaking(move || {
        let started = Instant::now();
        let written = Store::open(db)
            .and_then(|store| store.set_state("automator.probe", "written"))
            .is_ok();
        *seen.lock().unwrap() = Some((written, started.elapsed()));
    });

    harness.call("get_card_audio", json!({ "cardId": wo }));

    let (written, took) = outcome.lock().unwrap().expect("TTS ran");
    assert!(written, "the other app's write failed");
    assert!(
        took < Elapsed::from_secs(2),
        "the other app waited {took:?}"
    );
}

/// The recording is kept only for the card it was made from: Automator
/// editing the front while TTS runs means the audio speaks the old text.
#[test]
fn audio_is_not_kept_for_a_card_that_changed_meanwhile() {
    let harness = Harness::new();
    let wo = harness
        .call("start_new_item", json!({ "hanzi": "我" }))
        .as_i64()
        .unwrap();
    let db: PathBuf = harness.db_path();
    harness.tts.while_speaking(move || {
        let store = Store::open(db).unwrap();
        let mut card = store.get_capture(wo).unwrap().unwrap().card;
        card.front = "我们".into();
        store.update_capture_card(wo, &card).unwrap();
    });

    let audio = harness.call("get_card_audio", json!({ "cardId": wo }));

    assert_eq!(audio, b64(&fake_audio("zh-CN", "我")), "still played");
    let stored = harness.open_store().get_capture(wo).unwrap().unwrap();
    assert_eq!(stored.card.front, "我们");
    assert_eq!(stored.card.audio_front, None, "not kept for the new front");
    assert_eq!(stored.origin, CaptureOrigin::Curriculum);
}
