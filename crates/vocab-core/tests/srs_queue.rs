//! G+ tests for the daily queue (`vocab_core::srs`, docs/architecture.md
//! § 7.1, Milestone L3) and for introducing curriculum words into it.
//!
//! The pure builder gets fixture rows in deliberately scrambled order, so the
//! assertions are about the order it produces, not the order it was handed.
//! The store-backed builder is checked against the user's local day in
//! offsets on both sides of UTC, with `FixedClock` as the only time source.

use std::collections::HashSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Utc};
use vocab_core::{
    srs::{
        build_daily_queue, build_daily_queue_from, introduce_new_item, submit_review, QueueItem,
        ReviewRating, SrsScheduler,
    },
    store::{SrsStateRow, Store, StoreError},
    time::{Clock, FixedClock},
    CaptureOrigin, Card, CurriculumItem, ScriptMetadata, ScriptVariant,
};

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid test timestamp")
        .with_timezone(&Utc)
}

fn offset(hours: i32) -> FixedOffset {
    FixedOffset::east_opt(hours * 3600).expect("valid offset")
}

fn item(hanzi: &str, order: i64) -> CurriculumItem {
    CurriculumItem {
        hanzi: hanzi.into(),
        traditional: None,
        hsk_level: 1,
        frequency: Some(order * 10),
        curriculum_order: order,
        pinyin: format!("pinyin of {hanzi}"),
        pos: None,
        meaning_en: Some(format!("{hanzi} (en)")),
        meaning_de: Some(format!("{hanzi} (de)")),
        cloze_sentence: None,
        example_sentence_source: None,
        example_sentence_pinyin: None,
        example_sentence_target: None,
        vocab_audio_path: None,
        sentence_audio_path: None,
        image_path: None,
    }
}

/// A state that is due at `due`; the queue looks at nothing else.
fn due_state(card_id: i64, due: DateTime<Utc>) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: Some(4.0),
        difficulty: Some(5.0),
        interval_days: 4,
        repetitions: 2,
        lapses: 0,
        fsrs_state: "review".into(),
        elapsed_days: 4,
        due_date: due,
        last_reviewed_at: Some(due - Duration::days(4)),
        last_rating: Some(3),
    }
}

fn review(card_id: i64, due: &str) -> QueueItem {
    QueueItem::Review {
        card_id,
        due_date: at(due),
    }
}

fn new(hanzi: &str, curriculum_order: i64) -> QueueItem {
    QueueItem::New {
        hanzi: hanzi.into(),
        curriculum_order,
    }
}

fn learned(hanzi: &[&str]) -> HashSet<String> {
    hanzi.iter().map(|h| (*h).to_owned()).collect()
}

/// Curriculum in an order that is neither `curriculum_order` nor its reverse,
/// with gaps in the numbering.
fn scrambled_curriculum() -> Vec<CurriculumItem> {
    vec![
        item("你好", 5),
        item("我", 1),
        item("学习", 30),
        item("中文", 7),
        item("是", 2),
        item("好", 12),
    ]
}

fn chinese_card(hanzi: &str, captured_at: DateTime<Utc>) -> Card {
    let mut card = Card::new(hanzi, "Bedeutung", "zh-CN", "de", captured_at);
    card.script_metadata = ScriptMetadata::Chinese {
        pinyin: format!("pinyin of {hanzi}"),
        hsk_level: None,
        script_variant: ScriptVariant::Simplified,
    };
    card
}

mod pure_builder {
    use super::*;

    #[test]
    fn every_review_comes_before_any_new_item() {
        let due = [
            due_state(2, at("2026-10-31T09:00:00.000Z")),
            due_state(1, at("2026-10-30T09:00:00.000Z")),
        ];
        let queue = build_daily_queue_from(&due, &scrambled_curriculum(), &learned(&[]), 2);
        assert_eq!(
            queue,
            [
                review(1, "2026-10-30T09:00:00.000Z"),
                review(2, "2026-10-31T09:00:00.000Z"),
                new("我", 1),
                new("是", 2),
            ]
        );
    }

    #[test]
    fn reviews_are_ordered_by_due_date_then_card_id() {
        let due = [
            due_state(9, at("2026-10-31T12:00:00.000Z")),
            due_state(4, at("2026-10-31T12:00:00.000Z")),
            due_state(7, at("2026-10-31T11:59:59.999Z")),
            due_state(1, at("2026-10-31T12:00:00.001Z")),
            due_state(6, at("2026-10-31T12:00:00.000Z")),
            due_state(3, at("2026-10-01T00:00:00.000Z")),
        ];
        let queue = build_daily_queue_from(&due, &[], &learned(&[]), 3);
        assert_eq!(
            queue,
            [
                review(3, "2026-10-01T00:00:00.000Z"),
                review(7, "2026-10-31T11:59:59.999Z"),
                review(4, "2026-10-31T12:00:00.000Z"),
                review(6, "2026-10-31T12:00:00.000Z"),
                review(9, "2026-10-31T12:00:00.000Z"),
                review(1, "2026-10-31T12:00:00.001Z"),
            ]
        );
    }

    #[test]
    fn new_items_follow_strict_curriculum_order_whatever_the_input_order() {
        let curriculum = scrambled_curriculum();
        assert_eq!(
            build_daily_queue_from(&[], &curriculum, &learned(&[]), 6),
            [
                new("我", 1),
                new("是", 2),
                new("你好", 5),
                new("中文", 7),
                new("好", 12),
                new("学习", 30),
            ]
        );
        assert_eq!(
            build_daily_queue_from(&[], &curriculum, &learned(&[]), 3),
            [new("我", 1), new("是", 2), new("你好", 5)]
        );
        let mut reversed = curriculum;
        reversed.reverse();
        assert_eq!(
            build_daily_queue_from(&[], &reversed, &learned(&[]), 3),
            [new("我", 1), new("是", 2), new("你好", 5)]
        );
    }

    #[test]
    fn learned_hanzi_are_skipped_even_when_they_come_first() {
        let queue = build_daily_queue_from(
            &[],
            &scrambled_curriculum(),
            &learned(&["我", "你好", "not in the curriculum"]),
            3,
        );
        assert_eq!(queue, [new("是", 2), new("中文", 7), new("好", 12)]);
    }

    #[test]
    fn a_quota_beyond_the_remaining_curriculum_offers_all_of_it() {
        let learned = learned(&["我", "是", "好"]);
        let remaining = [new("你好", 5), new("中文", 7), new("学习", 30)];
        for quota in [3, 4, 100, u32::MAX] {
            assert_eq!(
                build_daily_queue_from(&[], &scrambled_curriculum(), &learned, quota),
                remaining,
                "quota {quota}"
            );
        }
        let everything = learned_all(&scrambled_curriculum());
        assert!(build_daily_queue_from(&[], &scrambled_curriculum(), &everything, 5).is_empty());
    }

    fn learned_all(curriculum: &[CurriculumItem]) -> HashSet<String> {
        curriculum.iter().map(|item| item.hanzi.clone()).collect()
    }

    #[test]
    fn an_empty_curriculum_leaves_only_the_reviews() {
        let due = [due_state(1, at("2026-10-31T09:00:00.000Z"))];
        assert_eq!(
            build_daily_queue_from(&due, &[], &learned(&[]), 10),
            [review(1, "2026-10-31T09:00:00.000Z")]
        );
        assert!(build_daily_queue_from(&[], &[], &learned(&[]), 10).is_empty());
    }

    #[test]
    fn a_zero_quota_gives_exactly_the_reviews_of_any_other_quota() {
        let due = [
            due_state(2, at("2026-10-31T09:00:00.000Z")),
            due_state(1, at("2026-10-31T08:00:00.000Z")),
        ];
        let curriculum = scrambled_curriculum();
        let reviews_only = build_daily_queue_from(&due, &curriculum, &learned(&[]), 0);
        assert_eq!(
            reviews_only,
            [
                review(1, "2026-10-31T08:00:00.000Z"),
                review(2, "2026-10-31T09:00:00.000Z"),
            ]
        );
        for quota in [1, 4] {
            let queue = build_daily_queue_from(&due, &curriculum, &learned(&[]), quota);
            assert_eq!(queue[..2], reviews_only[..], "quota {quota}");
            assert_eq!(queue.len(), 2 + quota as usize, "quota {quota}");
        }
    }
}

mod store_backed {
    use super::*;

    fn seeded_store(curriculum: &[CurriculumItem]) -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store.replace_curriculum(curriculum).unwrap();
        store
    }

    fn insert(store: &Store, card: &Card) -> i64 {
        store
            .insert_capture(card, CaptureOrigin::Capture, false)
            .unwrap()
    }

    #[test]
    fn the_queue_comes_from_due_states_the_curriculum_and_chinese_captures() {
        let store = seeded_store(&scrambled_curriculum());
        let captured = at("2026-10-01T00:00:00Z");
        // A Chinese capture of the first curriculum word hides it from the new
        // items; an English capture with the same text as another does not.
        let chinese = insert(&store, &chinese_card("我", captured));
        let english = insert(&store, &Card::new("是", "is", "en", "de", captured));
        store
            .upsert_srs_state(&due_state(chinese, at("2026-10-31T10:00:00.000Z")))
            .unwrap();
        store
            .upsert_srs_state(&due_state(english, at("2026-10-31T06:00:00.000Z")))
            .unwrap();
        let clock = FixedClock::utc(at("2026-10-31T08:00:00.000Z"));

        assert_eq!(
            build_daily_queue(&store, 2, &clock).unwrap(),
            [
                review(english, "2026-10-31T06:00:00.000Z"),
                review(chinese, "2026-10-31T10:00:00.000Z"),
                new("是", 2),
                new("你好", 5),
            ]
        );
        assert_eq!(
            build_daily_queue(&store, 0, &clock).unwrap(),
            [
                review(english, "2026-10-31T06:00:00.000Z"),
                review(chinese, "2026-10-31T10:00:00.000Z"),
            ]
        );
        assert_eq!(build_daily_queue(&store, 99, &clock).unwrap().len(), 2 + 5);
    }

    #[test]
    fn due_today_ends_at_the_end_of_the_local_day_not_the_utc_one() {
        // 08:00 local time on Oct 31 in Tokyo (UTC+9: still Oct 30 in UTC),
        // New York (UTC-5) and Berlin (UTC+1).
        for hours in [9, -5, 1] {
            let offset = offset(hours);
            let local = |text: &str| -> DateTime<Utc> {
                offset
                    .from_local_datetime(
                        &chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.3f")
                            .unwrap(),
                    )
                    .unwrap()
                    .with_timezone(&Utc)
            };
            let store = Store::open_in_memory().unwrap();
            let dues = [
                ("long overdue", local("2026-09-01 12:00:00.000"), true),
                ("start of today", local("2026-10-31 00:00:00.000"), true),
                ("now", local("2026-10-31 08:00:00.000"), true),
                ("this evening", local("2026-10-31 21:30:00.000"), true),
                ("last millisecond", local("2026-10-31 23:59:59.999"), true),
                (
                    "tomorrow at midnight",
                    local("2026-11-01 00:00:00.000"),
                    false,
                ),
                ("tomorrow morning", local("2026-11-01 07:00:00.000"), false),
            ];
            let mut expected = Vec::new();
            for (index, (_, due, included)) in dues.iter().enumerate() {
                let id = insert(
                    &store,
                    &chinese_card(&format!("字{index}"), at("2026-01-01T00:00:00Z")),
                );
                store.upsert_srs_state(&due_state(id, *due)).unwrap();
                if *included {
                    expected.push(QueueItem::Review {
                        card_id: id,
                        due_date: *due,
                    });
                }
            }
            let clock = FixedClock::new(local("2026-10-31 08:00:00.000"), offset);
            assert_eq!(
                clock.today(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
            );
            assert_eq!(
                build_daily_queue(&store, 0, &clock).unwrap(),
                expected,
                "UTC{hours:+}"
            );

            // One millisecond before local midnight the evening's cards are
            // still today's; at midnight tomorrow's join them.
            clock.set(local("2026-10-31 23:59:59.999"));
            assert_eq!(build_daily_queue(&store, 0, &clock).unwrap().len(), 5);
            clock.set(local("2026-11-01 00:00:00.000"));
            assert_eq!(build_daily_queue(&store, 0, &clock).unwrap().len(), 7);
        }
    }

    #[test]
    fn introducing_a_word_turns_its_new_item_into_a_review() {
        let store = seeded_store(&scrambled_curriculum());
        let scheduler = SrsScheduler::default();
        let clock = FixedClock::utc(at("2026-10-31T08:00:00.000Z"));
        assert_eq!(
            build_daily_queue(&store, 2, &clock).unwrap(),
            [new("我", 1), new("是", 2)]
        );

        let card_id = introduce_new_item(&store, &scheduler, "我", "de", &clock).unwrap();
        assert_eq!(
            build_daily_queue(&store, 2, &clock).unwrap(),
            [
                review(card_id, "2026-10-31T08:00:00.000Z"),
                new("是", 2),
                new("你好", 5),
            ]
        );

        // Once reviewed, the learning step comes back later today.
        let outcome =
            submit_review(&store, &scheduler, card_id, ReviewRating::Good, &clock).unwrap();
        assert_eq!(
            build_daily_queue(&store, 0, &clock).unwrap(),
            [QueueItem::Review {
                card_id,
                due_date: outcome.next.due_date,
            }]
        );
    }
}

mod introducing {
    use super::*;

    fn word() -> CurriculumItem {
        CurriculumItem {
            hanzi: "学习".into(),
            traditional: Some("學習".into()),
            hsk_level: 1,
            frequency: Some(250),
            curriculum_order: 3,
            pinyin: "xué xí".into(),
            pos: Some("v".into()),
            meaning_en: Some("to study".into()),
            meaning_de: Some("lernen".into()),
            cloze_sentence: None,
            example_sentence_source: Some("我学习中文。".into()),
            example_sentence_pinyin: Some("wǒ xué xí zhōng wén".into()),
            example_sentence_target: Some("I study Chinese.".into()),
            vocab_audio_path: Some("xuexi.mp3".into()),
            sentence_audio_path: None,
            image_path: None,
        }
    }

    fn store_with(items: &[CurriculumItem]) -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store.replace_curriculum(items).unwrap();
        store
    }

    fn clock() -> FixedClock {
        FixedClock::new(
            at("2026-10-31T08:00:00.000Z") + Duration::nanoseconds(456_789),
            offset(1),
        )
    }

    #[test]
    fn a_new_word_becomes_a_curriculum_card_with_a_new_state() {
        let store = store_with(&[word(), item("你好", 1)]);
        let scheduler = SrsScheduler::default();
        let clock = clock();

        let card_id = introduce_new_item(&store, &scheduler, "学习", "de", &clock).unwrap();

        let stored = store.get_capture(card_id).unwrap().unwrap();
        assert_eq!(stored.origin, CaptureOrigin::Curriculum);
        assert!(!stored.synced_to_anki);
        let mut expected = Card::new(
            "学习",
            "lernen",
            "zh-CN",
            "de",
            at("2026-10-31T08:00:00.000Z"),
        );
        expected.example_source = "我学习中文。".into();
        // The curriculum's example translation is English, so a German card
        // carries the sentence without a mismatched translation.
        expected.example_target = String::new();
        expected.script_metadata = ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        };
        assert_eq!(stored.card, expected);

        assert_eq!(
            store.get_srs_state(card_id).unwrap(),
            Some(scheduler.initial_state(card_id, clock.now()))
        );
        assert_eq!(
            store.get_srs_state(card_id).unwrap().unwrap().due_date,
            at("2026-10-31T08:00:00.000Z")
        );
        assert!(store.chinese_capture_fronts().unwrap().contains("学习"));
        assert!(store.learned_chinese_fronts().unwrap().contains("学习"));
    }

    #[test]
    fn an_english_card_keeps_the_curriculum_s_english_example_translation() {
        let store = store_with(&[word()]);
        let card_id =
            introduce_new_item(&store, &SrsScheduler::default(), "学习", "en", &clock()).unwrap();
        let card = store.get_capture(card_id).unwrap().unwrap().card;
        assert_eq!(
            (card.back.as_str(), card.target_lang.as_str()),
            ("to study", "en")
        );
        assert_eq!(card.example_target, "I study Chinese.");
    }

    #[test]
    fn the_back_is_german_for_german_learners_else_english_else_empty() {
        let with = |de: Option<&str>, en: Option<&str>| CurriculumItem {
            meaning_de: de.map(str::to_owned),
            meaning_en: en.map(str::to_owned),
            ..word()
        };
        // (target language asked for, meanings, expected back and its language)
        let cases = [
            ("de", with(Some("lernen"), Some("to study")), "lernen", "de"),
            ("DE", with(Some("lernen"), Some("to study")), "lernen", "de"),
            ("de", with(None, Some("to study")), "to study", "en"),
            ("de", with(Some("  "), Some("to study")), "to study", "en"),
            (
                "en",
                with(Some("lernen"), Some("to study")),
                "to study",
                "en",
            ),
            (
                "fr",
                with(Some("lernen"), Some("to study")),
                "to study",
                "en",
            ),
            ("de", with(None, None), "", "de"),
            ("de", with(Some(""), Some(" ")), "", "de"),
            ("fr", with(Some("lernen"), None), "", "fr"),
        ];
        for (target_lang, curriculum_item, back, back_lang) in cases {
            let store = store_with(std::slice::from_ref(&curriculum_item));
            let id = introduce_new_item(
                &store,
                &SrsScheduler::default(),
                "学习",
                target_lang,
                &clock(),
            )
            .unwrap();
            let card = store.get_capture(id).unwrap().unwrap().card;
            assert_eq!(
                (card.back.as_str(), card.target_lang.as_str()),
                (back, back_lang),
                "{target_lang} with de {:?}, en {:?}",
                curriculum_item.meaning_de,
                curriculum_item.meaning_en
            );
        }
    }

    #[test]
    fn introducing_again_returns_the_same_card_and_keeps_its_progress() {
        let store = store_with(&[word()]);
        let scheduler = SrsScheduler::default();
        let clock = clock();
        let card_id = introduce_new_item(&store, &scheduler, "学习", "de", &clock).unwrap();
        submit_review(&store, &scheduler, card_id, ReviewRating::Good, &clock).unwrap();
        let reviewed = store.get_srs_state(card_id).unwrap();

        clock.advance(Duration::days(1));
        for target_lang in ["de", "en"] {
            assert_eq!(
                introduce_new_item(&store, &scheduler, "学习", target_lang, &clock).unwrap(),
                card_id
            );
        }
        assert_eq!(store.list_captures(None).unwrap().len(), 1);
        assert_eq!(store.count_srs_states().unwrap(), 1);
        assert_eq!(store.get_srs_state(card_id).unwrap(), reviewed);
    }

    #[test]
    fn a_word_captured_in_automator_is_adopted_rather_than_duplicated() {
        let store = store_with(&[word()]);
        let scheduler = SrsScheduler::default();
        let clock = clock();
        let captured = store
            .insert_capture(
                &chinese_card("学习", at("2026-10-01T00:00:00Z")),
                CaptureOrigin::Capture,
                true,
            )
            .unwrap();
        let later_duplicate = store
            .insert_capture(
                &chinese_card("学习", at("2026-10-02T00:00:00Z")),
                CaptureOrigin::Capture,
                false,
            )
            .unwrap();
        // Captured but never scheduled: not learned, so the curriculum still
        // offers it, and nothing else would ever bring it to a review.
        assert_eq!(
            build_daily_queue(&store, 5, &clock).unwrap(),
            [QueueItem::New {
                hanzi: "学习".into(),
                curriculum_order: 3
            }]
        );

        let card_id = introduce_new_item(&store, &scheduler, "学习", "de", &clock).unwrap();
        assert_eq!(card_id, captured);
        // Adopted and scheduled: now a review, no longer a new word.
        let queue = build_daily_queue(&store, 5, &clock).unwrap();
        assert!(
            matches!(queue.as_slice(), [QueueItem::Review { card_id, .. }] if *card_id == captured),
            "{queue:?}"
        );
        assert_eq!(
            store.get_srs_state(captured).unwrap(),
            Some(scheduler.initial_state(captured, clock.now()))
        );
        assert_eq!(store.get_srs_state(later_duplicate).unwrap(), None);
        assert_eq!(store.list_captures(None).unwrap().len(), 2);
        let adopted = store.get_capture(captured).unwrap().unwrap();
        assert_eq!(adopted.origin, CaptureOrigin::Capture);
        assert_eq!(adopted.card.back, "Bedeutung");
    }

    #[test]
    fn a_word_that_is_neither_captured_nor_in_the_curriculum_is_not_found() {
        let store = store_with(&[word()]);
        for hanzi in ["没有", " 学习", ""] {
            assert!(
                matches!(
                    introduce_new_item(&store, &SrsScheduler::default(), hanzi, "de", &clock()),
                    Err(StoreError::NotFound(_))
                ),
                "{hanzi:?}"
            );
        }
        assert!(store.list_captures(None).unwrap().is_empty());
        assert_eq!(store.count_srs_states().unwrap(), 0);
    }
}

#[test]
fn queue_items_serialize_in_the_documented_shape() {
    let items = [review(7, "2026-10-31T09:00:00.000Z"), new("学习", 3)];
    let json = serde_json::to_value(&items).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            { "kind": "review", "card_id": 7, "due_date": "2026-10-31T09:00:00Z" },
            { "kind": "new", "hanzi": "学习", "curriculum_order": 3 }
        ])
    );
    let back: Vec<QueueItem> = serde_json::from_value(json).unwrap();
    assert_eq!(back, items);
}
