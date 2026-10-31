//! Learn's review flow through the IPC layer of a `tauri::test` mock app
//! (Milestones I, L2, L3): the daily prompt and its dismiss rule, the queue,
//! new words, ratings with session progress, XP and streaks, startup, and the
//! settings. The database is a temp file, the clock a `FixedClock`, and Anki
//! unreachable; the AnkiConnect conversations are in `tests/ipc_anki.rs`.

mod common;

use chrono::{Duration, NaiveDate};
use common::{
    capture, chinese_capture, instant, learned, start_time, test_settings, word, Harness, Options,
    UNREACHABLE_ANKI,
};
use serde_json::{json, Value};
use vocab_core::{
    curriculum::bundled_curriculum,
    gamification::STREAK_FREEZES_KEY,
    store::{default_db_path, DailySessionRow},
    time::{local_day_bounds, Clock},
    CaptureOrigin, Card, ScriptMetadata, ScriptVariant, StoredCapture,
};
use vocab_learn_lib::settings::{LearnSettings, MeaningLang, SETTINGS_KEY};

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn rate(harness: &Harness, card_id: i64, rating: u8) -> Value {
    harness.call(
        "submit_review_rating",
        json!({ "cardId": card_id, "rating": rating }),
    )
}

fn start(harness: &Harness, hanzi: &str) -> i64 {
    harness
        .call("start_new_item", json!({ "hanzi": hanzi }))
        .as_i64()
        .expect("a card id")
}

fn queue(harness: &Harness) -> Value {
    harness.call("get_daily_queue", json!({}))
}

/// The hanzi of the queue's new items, in order.
fn new_words(queue: &Value) -> Vec<String> {
    queue
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "new")
        .map(|item| item["hanzi"].as_str().unwrap().to_string())
        .collect()
}

fn stored(harness: &Harness, id: i64) -> StoredCapture {
    harness
        .open_store()
        .get_capture(id)
        .unwrap()
        .unwrap_or_else(|| panic!("card {id} should be stored"))
}

fn streak(current: u32, longest: u32, completed_today: bool) -> Value {
    json!({
        "current": current,
        "longest": longest,
        "freezes_available": 0,
        "completed_today": completed_today
    })
}

/// Two cards learned before today: 我 (also the first curriculum word) due
/// yesterday, and an English one due this morning.
fn with_two_reviews_due() -> Options {
    Options {
        before_start: Some(Box::new(|store| {
            learned(
                store,
                &chinese_capture("我", "ich", "wǒ", ("", "")),
                start_time() - Duration::days(1),
            );
            learned(
                store,
                &Card::new("Haus", "house", "de", "en", start_time()),
                start_time() - Duration::hours(1),
            );
        })),
        ..Options::default()
    }
}

#[test]
fn a_first_day_from_the_prompt_to_a_completed_session() {
    let harness = Harness::with(with_two_reviews_due());

    assert_eq!(
        harness.call("get_daily_prompt_state", json!({})),
        json!({ "needs_prompt": true, "today": "2026-10-31", "session": null })
    );

    let session = harness.call("submit_daily_quota", json!({ "newCount": 2 }));
    let opened = json!({
        "session_date": "2026-10-31",
        "requested_new_count": 2,
        "reviews_due_count": 2,
        "reviews_completed": 0,
        "new_items_completed": 0,
        "completed_at": null
    });
    assert_eq!(session, opened);
    assert_eq!(
        harness.call("get_daily_prompt_state", json!({})),
        json!({ "needs_prompt": false, "today": "2026-10-31", "session": opened })
    );

    // Reviews first, by due date; then the first two curriculum words that
    // are not learned yet (我 is).
    assert_eq!(
        queue(&harness),
        json!([
            { "kind": "review", "card_id": 1, "due_date": instant(start_time() - Duration::days(1)) },
            { "kind": "review", "card_id": 2, "due_date": instant(start_time() - Duration::hours(1)) },
            { "kind": "new", "hanzi": "你", "curriculum_order": 1 },
            { "kind": "new", "hanzi": "好", "curriculum_order": 2 }
        ])
    );

    for (card_id, done) in [(1, 1), (2, 2)] {
        let result = rate(&harness, card_id, 3);
        assert_eq!(result["xp_awarded"], 10, "a review");
        assert_eq!(result["session_completed"], false);
        assert_eq!(result["session"]["reviews_completed"], done);
        assert_eq!(result["next"]["card_id"], card_id);
        assert_eq!(result["next"]["fsrs_state"], "review");
        assert_eq!(result["streak"], streak(0, 0, false));
    }

    let ni = start(&harness, "你");
    let card = harness.call("get_card", json!({ "cardId": ni }));
    assert_eq!(card["origin"], "curriculum");
    assert_eq!(card["card"]["front"], "你");
    assert_eq!(card["card"]["back"], "you");
    assert_eq!(card["card"]["target_lang"], "en");
    assert_eq!(card["card"]["example_target"], "How are you?");
    let previews = harness.call("preview_ratings", json!({ "cardId": ni }));
    let ratings: Vec<&Value> = previews
        .as_array()
        .unwrap()
        .iter()
        .map(|preview| &preview["rating"])
        .collect();
    assert_eq!(ratings, [1, 2, 3, 4]);

    let first_new = rate(&harness, ni, 3);
    assert_eq!(first_new["xp_awarded"], 15, "a new word's first review");
    assert_eq!(first_new["session_completed"], false);
    assert_eq!(first_new["session"]["new_items_completed"], 1);
    assert_eq!(first_new["next"]["fsrs_state"], "learning");
    assert_eq!(new_words(&queue(&harness)), ["好"]);

    let hao = start(&harness, "好");
    let last = rate(&harness, hao, 3);
    assert_eq!(last["xp_awarded"], 15 + 50, "the completion bonus");
    assert_eq!(last["session_completed"], true);
    assert_eq!(
        last["session"],
        json!({
            "session_date": "2026-10-31",
            "requested_new_count": 2,
            "reviews_due_count": 2,
            "reviews_completed": 2,
            "new_items_completed": 2,
            "completed_at": instant(start_time())
        })
    );
    assert_eq!(last["streak"], streak(1, 1, true));

    let store = harness.open_store();
    let (_, end_of_today) = local_day_bounds(harness.clock.today(), harness.clock.local_offset());
    let due_today = store.list_srs_due_before(end_of_today).unwrap().len();
    assert_eq!(due_today, 2, "the two new words' learning steps");
    assert_eq!(
        harness.call("get_stats", json!({})),
        json!({
            "xp_total": 100,
            "xp_today": 100,
            "streak": streak(1, 1, true),
            "learned_count": 4,
            "due_today": due_today
        })
    );

    // The quota is used up: what is left today are reviews.
    let rest = queue(&harness);
    assert!(new_words(&rest).is_empty(), "{rest}");
    assert_eq!(rest.as_array().unwrap().len(), 2);
}

#[test]
fn new_words_get_meanings_in_the_settings_language() {
    let harness = Harness::new();
    let mut settings = test_settings();
    settings.meaning_lang = MeaningLang::De;
    harness.call("save_settings", json!({ "settings": settings }));
    harness.call("submit_daily_quota", json!({ "newCount": 2 }));

    let wo = stored(&harness, start(&harness, "我"));
    assert_eq!(wo.origin, CaptureOrigin::Curriculum);
    assert!(!wo.synced_to_anki);
    assert_eq!(
        (wo.card.back.as_str(), wo.card.target_lang.as_str()),
        ("ich", "de")
    );
    assert_eq!(wo.card.source_lang, "zh-CN");
    assert_eq!(wo.card.example_source, "我是老师。");
    assert_eq!(
        wo.card.example_target, "",
        "the curriculum's translations are English"
    );
    assert_eq!(
        wo.card.script_metadata,
        ScriptMetadata::Chinese {
            pinyin: "wǒ".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        }
    );

    // No German meaning bundled: English, and the card says so.
    let ni = stored(&harness, start(&harness, "你"));
    assert_eq!(
        (ni.card.back.as_str(), ni.card.target_lang.as_str()),
        ("you", "en")
    );
    assert_eq!(ni.card.example_target, "How are you?");
}

#[test]
fn starting_a_word_twice_gives_the_same_card_and_counts_nothing() {
    let harness = Harness::with(Options {
        before_start: Some(Box::new(|store| {
            // Captured in Automator, never scheduled: adopted, not duplicated.
            capture(store, &chinese_capture("你", "du", "nǐ", ("", "")));
        })),
        ..Options::default()
    });
    harness.call("submit_daily_quota", json!({ "newCount": 2 }));

    let first = start(&harness, "我");
    assert_eq!(start(&harness, " 我 "), first);
    assert_eq!(start(&harness, "你"), 1, "the Automator capture");

    let session = harness.call("get_daily_prompt_state", json!({}))["session"].clone();
    assert_eq!(session["new_items_completed"], 0);
    assert_eq!(harness.call("get_stats", json!({}))["xp_total"], 0);
    let store = harness.open_store();
    assert_eq!(store.list_captures(None).unwrap().len(), 2);
    assert_eq!(store.count_srs_states().unwrap(), 2);
}

#[test]
fn mistakes_are_refused_with_a_readable_message() {
    let harness = Harness::new();
    let unscheduled = capture(
        &harness.open_store(),
        &Card::new("Haus", "house", "de", "en", start_time()),
    );

    assert_eq!(
        harness.call_err("start_new_item", json!({ "hanzi": "龙" })),
        "\"龙\" is not in the HSK curriculum."
    );
    assert_eq!(
        harness.call_err("get_card", json!({ "cardId": 999 })),
        "There is no card 999."
    );
    for command in ["submit_review_rating", "preview_ratings"] {
        assert_eq!(
            harness.call_err(command, json!({ "cardId": unscheduled, "rating": 3 })),
            format!("Card {unscheduled} is not scheduled for review.")
        );
    }
    let card = start(&harness, "我");
    let err = harness.call_err(
        "submit_review_rating",
        json!({ "cardId": card, "rating": 5 }),
    );
    assert!(
        err.contains("a review rating is 1 (Again) to 4 (Easy), not 5"),
        "{err}"
    );
    assert_eq!(harness.open_store().review_log_for_card(card).unwrap(), []);
}

/// L2's documented dismiss rule: closing the prompt means reviews only today;
/// the menu action asks again, and a later answer keeps the progress.
#[test]
fn dismissing_the_prompt_means_reviews_only_today() {
    let harness = Harness::with(Options {
        before_start: Some(Box::new(|store| {
            learned(
                store,
                &Card::new("Haus", "house", "de", "en", start_time()),
                start_time() - Duration::hours(1),
            );
        })),
        ..Options::default()
    });

    let dismissed = harness.call("dismiss_daily_prompt", json!({}));
    assert_eq!(
        dismissed,
        json!({
            "session_date": "2026-10-31",
            "requested_new_count": 0,
            "reviews_due_count": 1,
            "reviews_completed": 0,
            "new_items_completed": 0,
            "completed_at": null
        })
    );
    assert_eq!(
        harness.call("get_daily_prompt_state", json!({}))["needs_prompt"],
        false
    );
    let reviews_only = queue(&harness);
    assert_eq!(
        reviews_only,
        json!([{ "kind": "review", "card_id": 1, "due_date": instant(start_time() - Duration::hours(1)) }])
    );

    let result = rate(&harness, 1, 3);
    assert_eq!(result["session_completed"], true);
    assert_eq!(result["xp_awarded"], 10 + 50);
    assert_eq!(result["streak"], streak(1, 1, true));

    harness.call("reopen_daily_prompt", json!({}));
    let reopened = harness.call("get_daily_prompt_state", json!({}));
    assert_eq!(reopened["needs_prompt"], true);
    assert_eq!(reopened["session"]["reviews_completed"], 1);

    let answered = harness.call("submit_daily_quota", json!({ "newCount": 1 }));
    assert_eq!(answered["requested_new_count"], 1);
    assert_eq!(answered["reviews_completed"], 1, "progress is kept");
    assert_eq!(
        answered["completed_at"],
        instant(start_time()),
        "a completed day stays completed"
    );
    assert_eq!(new_words(&queue(&harness)), ["我"]);
    assert_eq!(harness.call("get_stats", json!({}))["xp_total"], 60);
}

#[test]
fn a_day_with_nothing_to_do_never_completes() {
    let harness = Harness::new();

    let session = harness.call("dismiss_daily_prompt", json!({}));

    assert_eq!(session["reviews_due_count"], 0);
    assert_eq!(session["completed_at"], Value::Null);
    assert_eq!(queue(&harness), json!([]));
    let stats = harness.call("get_stats", json!({}));
    assert_eq!(stats["xp_total"], 0);
    assert_eq!(stats["streak"], streak(0, 0, false));
}

/// An answer to the reopened prompt that lowers the quota to what is done
/// completes the session too, and pays the bonus like a rating would.
#[test]
fn lowering_the_quota_to_what_is_done_completes_the_session() {
    let harness = Harness::new();
    harness.call("submit_daily_quota", json!({ "newCount": 2 }));
    let first = harness.learn("我", 3);
    assert_eq!(
        harness.call("get_daily_prompt_state", json!({}))["session"]["completed_at"],
        Value::Null
    );

    harness.call("reopen_daily_prompt", json!({}));
    let session = harness.call("submit_daily_quota", json!({ "newCount": 1 }));

    assert_eq!(session["completed_at"], instant(start_time()));
    let stats = harness.call("get_stats", json!({}));
    assert_eq!(stats["xp_total"], 15 + 50);
    assert_eq!(stats["streak"], streak(1, 1, true));

    // Answering again changes nothing more.
    harness.call("submit_daily_quota", json!({ "newCount": 0 }));
    assert_eq!(harness.call("get_stats", json!({}))["xp_total"], 15 + 50);
    assert_eq!(rate(&harness, first, 3)["xp_awarded"], 10);
}

#[test]
fn the_prompt_comes_back_the_next_day_and_the_streak_grows() {
    let harness = Harness::new();
    harness.call("submit_daily_quota", json!({ "newCount": 1 }));
    let wo = harness.learn("我", 3);
    assert_eq!(
        harness.call("get_stats", json!({}))["streak"],
        streak(1, 1, true)
    );

    harness.clock.advance(Duration::days(1));
    let prompt = harness.call("get_daily_prompt_state", json!({}));
    assert_eq!(
        prompt,
        json!({ "needs_prompt": true, "today": "2026-11-01", "session": null })
    );
    assert_eq!(
        harness.call("get_stats", json!({}))["streak"],
        streak(1, 1, false),
        "yesterday still counts while today is open"
    );

    let session = harness.call("submit_daily_quota", json!({ "newCount": 0 }));
    assert_eq!(session["reviews_due_count"], 1, "yesterday's word is due");
    let result = rate(&harness, wo, 3);
    assert_eq!(result["session_completed"], true);
    assert_eq!(result["streak"], streak(2, 2, true));
}

#[test]
fn startup_seeds_the_bundled_curriculum_and_spends_due_freezes() {
    let bundled = bundled_curriculum().unwrap();
    let harness = Harness::with(Options {
        curriculum: Ok(bundled.clone()),
        before_start: Some(Box::new(|store| {
            store
                .upsert_daily_session(&DailySessionRow {
                    session_date: day("2026-10-29"),
                    requested_new_count: 1,
                    reviews_due_count: 0,
                    reviews_completed: 0,
                    new_items_completed: 1,
                    completed_at: Some(start_time() - Duration::days(2)),
                })
                .unwrap();
            store.set_state(STREAK_FREEZES_KEY, "1").unwrap();
        })),
        ..Options::default()
    });

    let store = harness.open_store();
    assert_eq!(store.curriculum_count().unwrap(), bundled.len() as i64);
    assert_eq!(
        store.list_streak_freeze_dates().unwrap(),
        [day("2026-10-30")],
        "the missed day is frozen at startup"
    );
    assert_eq!(
        store.get_state(STREAK_FREEZES_KEY).unwrap().as_deref(),
        Some("0")
    );
    assert_eq!(harness.call("get_stats", json!({}))["streak"]["current"], 2);

    harness.call("submit_daily_quota", json!({ "newCount": 3 }));
    let first_three: Vec<String> = bundled[..3].iter().map(|item| item.hanzi.clone()).collect();
    assert_eq!(new_words(&queue(&harness)), first_three);
}

#[test]
fn a_curriculum_that_cannot_be_loaded_is_reported_where_it_is_needed() {
    let harness = Harness::with(Options {
        curriculum: Err("simulated: the bundled curriculum does not parse".into()),
        ..Options::default()
    });
    let expected =
        "The HSK curriculum is unavailable (simulated: the bundled curriculum does not parse).";

    assert_eq!(harness.call_err("get_daily_queue", json!({})), expected);
    assert_eq!(
        harness.call_err("start_new_item", json!({ "hanzi": "我" })),
        expected
    );
    // Everything else works.
    assert_eq!(
        harness.call("get_daily_prompt_state", json!({}))["needs_prompt"],
        true
    );
    assert_eq!(harness.call("get_stats", json!({}))["xp_total"], 0);
}

#[test]
fn a_curriculum_seeding_failure_is_retried() {
    // Two words with the same order: seeding fails at startup and on retry.
    let harness = Harness::with(Options {
        curriculum: Ok(vec![
            word(0, "我", "wǒ", "I", None, None),
            word(0, "你", "nǐ", "you", None, None),
        ]),
        ..Options::default()
    });

    let err = harness.call_err("get_daily_queue", json!({}));

    assert!(
        err.starts_with("The HSK curriculum is unavailable (database error:"),
        "{err}"
    );
    assert_eq!(harness.open_store().curriculum_count().unwrap(), 0);
}

#[test]
fn without_a_database_every_command_says_why() {
    let harness = Harness::with(Options {
        with_store: false,
        ..Options::default()
    });
    let expected =
        "The vocabulary database is unavailable (simulated: the database could not be opened).";

    let calls = [
        ("get_daily_prompt_state", json!({})),
        ("submit_daily_quota", json!({ "newCount": 1 })),
        ("dismiss_daily_prompt", json!({})),
        ("reopen_daily_prompt", json!({})),
        ("get_daily_queue", json!({})),
        ("start_new_item", json!({ "hanzi": "我" })),
        ("get_card", json!({ "cardId": 1 })),
        ("preview_ratings", json!({ "cardId": 1 })),
        ("submit_review_rating", json!({ "cardId": 1, "rating": 3 })),
        ("list_learned_cards", json!({})),
        (
            "next_exercise",
            json!({ "cardId": 1, "kind": "multiple_choice", "seed": 1 }),
        ),
        (
            "record_exercise_result",
            json!({ "cardId": 1, "kind": "multiple_choice", "correct": true }),
        ),
        ("get_stats", json!({})),
        ("list_lessons", json!({})),
        ("complete_lesson", json!({ "lessonId": "lesson-1" })),
        ("get_card_audio", json!({ "cardId": 1 })),
        ("save_settings", json!({ "settings": test_settings() })),
        ("anki_export", json!({})),
        ("anki_import", json!({ "deck": "Chinese::HSK" })),
    ];
    for (command, args) in calls {
        assert_eq!(harness.call_err(command, args), expected, "{command}");
    }
    assert_eq!(
        harness.call("get_settings", json!({})),
        serde_json::to_value(LearnSettings::default()).unwrap()
    );
    assert!(harness.tts.calls().is_empty());
}

#[test]
fn settings_round_trip_through_the_shared_database() {
    let harness = Harness::new();
    assert_eq!(
        harness.call("get_settings", json!({})),
        serde_json::to_value(test_settings()).unwrap()
    );

    let mut settings = harness.call("get_settings", json!({}));
    settings["meaning_lang"] = json!("de");
    settings["anki"]["deck_name"] = json!("  Mandarin::HSK  ");
    settings["anki"]["create_reverse"] = json!(true);
    assert_eq!(
        harness.call("save_settings", json!({ "settings": settings })),
        Value::Null
    );

    let saved = harness.call("get_settings", json!({}));
    assert_eq!(saved["meaning_lang"], "de");
    assert_eq!(saved["anki"]["deck_name"], "Mandarin::HSK", "trimmed");
    assert_eq!(saved["anki"]["create_reverse"], true);
    let stored = harness
        .open_store()
        .get_state(SETTINGS_KEY)
        .unwrap()
        .expect("stored as JSON");
    assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), saved);
}

#[test]
fn invalid_settings_are_refused_and_nothing_is_saved() {
    let harness = Harness::new();
    let mut settings = harness.call("get_settings", json!({}));
    settings["anki"]["connect_url"] = json!("localhost");
    settings["anki"]["model_name"] = json!(" ");

    let err = harness.call_err("save_settings", json!({ "settings": settings }));

    assert_eq!(
        err,
        "The AnkiConnect address \"localhost\" is not an http:// URL.\n\
         The Anki note type must not be empty."
    );
    assert_eq!(
        harness.call("get_settings", json!({})),
        serde_json::to_value(test_settings()).unwrap()
    );

    let mut unknown_language = harness.call("get_settings", json!({}));
    unknown_language["meaning_lang"] = json!("fr");
    let err = harness.call_err("save_settings", json!({ "settings": unknown_language }));
    assert!(err.contains("unknown variant `fr`"), "{err}");
}

/// Milestone A4: no test may reach the real database, a real Anki or the
/// network. This fails as soon as the harness is wired to a default location
/// or a real provider.
#[test]
fn the_harness_never_points_at_real_locations() {
    let harness = Harness::new();

    assert_ne!(harness.db_path(), default_db_path());
    assert!(harness.db_path().starts_with(harness.dir.path()));
    assert_eq!(
        harness.call("get_settings", json!({}))["anki"]["connect_url"],
        UNREACHABLE_ANKI
    );
    let wo = harness.call("start_new_item", json!({ "hanzi": "我" }));
    let audio = harness.call("get_card_audio", json!({ "cardId": wo }));
    assert_eq!(audio, common::b64(&common::fake_audio("zh-CN", "我")));
    assert_eq!(harness.tts.calls(), ["zh-CN 我"], "the fake answered");
}
