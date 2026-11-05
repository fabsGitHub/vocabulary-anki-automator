//! Contextual lessons from the learner's own captures (docs/architecture.md
//! § 7.3, Milestone I).

use chrono::{DateTime, Duration, Utc};
use vocab_core::{
    lessons::{build_lessons, lessons_from_store, Lesson, LessonCard, CLOZE_BLANK},
    store::Store,
    CaptureOrigin, Card, ScriptMetadata, ScriptVariant, StoredCapture,
};

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid test timestamp")
        .with_timezone(&Utc)
}

/// A clipboard capture `minutes` after a fixed start.
fn capture(
    id: i64,
    minutes: i64,
    front: &str,
    pair: (&str, &str),
    example: (&str, &str),
) -> StoredCapture {
    let mut card = Card::new(
        front,
        format!("{front} (translated)"),
        pair.0,
        pair.1,
        at("2026-10-01T08:00:00.000Z") + Duration::minutes(minutes),
    );
    card.example_source = example.0.into();
    card.example_target = example.1.into();
    StoredCapture {
        id,
        card,
        origin: CaptureOrigin::Capture,
        synced_to_anki: false,
    }
}

fn english(id: i64, minutes: i64, front: &str) -> StoredCapture {
    let sentence = format!("I like the word {front}.");
    capture(
        id,
        minutes,
        front,
        ("en", "de"),
        (&sentence, "Ich mag das Wort."),
    )
}

fn chinese(id: i64, minutes: i64, front: &str, sentence: &str, pinyin: &str) -> StoredCapture {
    let mut capture = capture(id, minutes, front, ("zh-CN", "de"), (sentence, "Ein Satz."));
    capture.card.script_metadata = ScriptMetadata::Chinese {
        pinyin: pinyin.into(),
        hsk_level: Some(1),
        script_variant: ScriptVariant::Simplified,
    };
    capture
}

fn card_ids(lesson: &Lesson) -> Vec<i64> {
    lesson.cards.iter().map(|card| card.card_id).collect()
}

fn only_card(captures: &[StoredCapture]) -> Option<LessonCard> {
    let lessons = build_lessons(captures, 10);
    assert!(lessons.len() <= 1, "{lessons:?}");
    lessons.into_iter().next().map(|mut lesson| {
        assert_eq!(lesson.cards.len(), 1, "{lesson:?}");
        lesson.cards.remove(0)
    })
}

#[test]
fn a_card_is_the_word_in_its_captured_sentence() {
    let mut captured = capture(
        7,
        0,
        " serendipity ",
        ("en", "de"),
        ("It was pure serendipity. ", " Es war reiner Zufall."),
    );
    captured.card.back = " glücklicher Zufall".into();
    let card = only_card(&[captured]).expect("a lesson card");
    assert_eq!(
        card,
        LessonCard {
            card_id: 7,
            word: "serendipity".into(),
            translation: "glücklicher Zufall".into(),
            sentence_source: "It was pure serendipity.".into(),
            sentence_target: "Es war reiner Zufall.".into(),
            cloze: "It was pure ____.".into(),
            pinyin: None,
        }
    );
    assert_eq!(CLOZE_BLANK, "____");
}

#[test]
fn only_clipboard_captures_with_a_usable_sentence_pair_qualify() {
    let good = ("It was pure serendipity.", "Es war reiner Zufall.");
    let mut curriculum = capture(2, 1, "serendipity", ("en", "de"), good);
    curriculum.origin = CaptureOrigin::Curriculum;
    let mut imported = capture(3, 2, "serendipity", ("en", "de"), good);
    imported.origin = CaptureOrigin::AnkiImport;
    let captures = [
        capture(1, 0, "serendipity", ("en", "de"), good),
        curriculum,
        imported,
        capture(
            4,
            3,
            "house",
            ("en", "de"),
            ("The home is big.", "Das Heim ist groß."),
        ),
        capture(5, 4, "house", ("en", "de"), ("The house is big.", "  ")),
        capture(6, 5, "house", ("en", "de"), ("", "Das Haus ist groß.")),
        capture(7, 6, "  ", ("en", "de"), good),
    ];
    let lessons = build_lessons(&captures, 10);
    assert_eq!(lessons.len(), 1);
    assert_eq!(card_ids(&lessons[0]), [1]);
}

#[test]
fn the_cloze_blanks_the_first_occurrence_ignoring_case_outside_hanzi() {
    let cases: [(&str, &str, Option<&str>); 9] = [
        (
            "Serendipity",
            "Pure serendipity, serendipity!",
            Some("Pure ____, serendipity!"),
        ),
        ("serendipity", "Serendipity is rare.", Some("____ is rare.")),
        ("über", "Über alles, über.", Some("____ alles, über.")),
        ("Straße", "Die STRAẞE ist lang.", Some("Die ____ ist lang.")),
        (
            "学习",
            "我喜欢学习，学习中文。",
            Some("我喜欢____，学习中文。"),
        ),
        ("学习", "我喜欢學習中文。", None),
        // The word where it stands alone, not the letters inside another one.
        ("cat", "Concatenate the cat.", Some("Concatenate the ____.")),
        ("Cat", "Concatenate. Cat!", Some("Concatenate. ____!")),
        // A word that only occurs inflected is still blanked where it occurs.
        ("run", "He is running.", Some("He is ____ning.")),
    ];
    for (word, sentence, cloze) in cases {
        let found = only_card(&[capture(
            1,
            0,
            word,
            ("en", "de"),
            (sentence, "Übersetzung."),
        )]);
        assert_eq!(
            found.map(|card| card.cloze),
            cloze.map(str::to_owned),
            "{word} in {sentence}"
        );
    }
}

#[test]
fn chinese_cards_carry_their_pinyin() {
    let captures = [
        chinese(1, 0, "学习", "我喜欢学习。", "xué xí"),
        chinese(2, 1, "中文", "中文很难。", "  "),
    ];
    let lessons = build_lessons(&captures, 10);
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].cards[0].pinyin.as_deref(), Some("xué xí"));
    assert_eq!(lessons[0].cards[0].cloze, "我喜欢____。");
    assert_eq!(lessons[0].cards[1].pinyin, None);
}

#[test]
fn lessons_group_language_pairs_in_capture_order_and_are_chunked() {
    // Deliberately out of order: lessons follow captured_at, then id.
    let captures = [
        english(14, 40, "five"),
        chinese(20, 5, "学习", "我喜欢学习。", "xué xí"),
        english(10, 0, "one"),
        capture(
            30,
            50,
            "maison",
            ("en", "fr"),
            ("A maison here.", "Une maison ici."),
        ),
        english(12, 20, "three"),
        english(11, 10, "two"),
        chinese(21, 25, "中文", "我学习中文。", "zhōng wén"),
        english(13, 20, "four"),
    ];
    let lessons = build_lessons(&captures, 2);
    let summary: Vec<(&str, &str, &str, Vec<i64>)> = lessons
        .iter()
        .map(|lesson| {
            (
                lesson.id.as_str(),
                lesson.title.as_str(),
                lesson.source_lang.as_str(),
                card_ids(lesson),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("lesson-10", "EN → DE · Lesson 1", "en", vec![10, 11]),
            ("lesson-12", "EN → DE · Lesson 2", "en", vec![12, 13]),
            ("lesson-14", "EN → DE · Lesson 3", "en", vec![14]),
            ("lesson-20", "ZH-CN → DE · Lesson 1", "zh-CN", vec![20, 21]),
            ("lesson-30", "EN → FR · Lesson 1", "en", vec![30]),
        ]
    );
    assert!(lessons.iter().all(|lesson| lesson.cards.len() <= 2));
    assert_eq!(lessons[4].target_lang, "fr");
}

#[test]
fn lesson_ids_stay_put_when_newer_captures_arrive() {
    let mut captures: Vec<StoredCapture> = (1..=5)
        .map(|id| english(id, id * 10, &format!("word{id}")))
        .collect();
    let before: Vec<String> = build_lessons(&captures, 2)
        .into_iter()
        .map(|lesson| lesson.id)
        .collect();
    assert_eq!(before, ["lesson-1", "lesson-3", "lesson-5"]);

    captures.push(english(6, 60, "word6"));
    captures.push(english(7, 70, "word7"));
    let after = build_lessons(&captures, 2);
    let ids: Vec<&str> = after.iter().map(|lesson| lesson.id.as_str()).collect();
    assert_eq!(ids, ["lesson-1", "lesson-3", "lesson-5", "lesson-7"]);
    assert_eq!(card_ids(&after[2]), [5, 6]);
}

#[test]
fn a_lesson_size_of_zero_means_one_card_per_lesson() {
    let captures = [english(1, 0, "one"), english(2, 1, "two")];
    let lessons = build_lessons(&captures, 0);
    assert_eq!(lessons.len(), 2);
    assert_eq!(lessons[1].title, "EN → DE · Lesson 2");
    assert!(build_lessons(&[], 5).is_empty());
}

#[test]
fn lessons_from_store_use_the_stored_clipboard_captures() {
    let store = Store::open_in_memory().unwrap();
    let insert = |capture: &StoredCapture, origin| {
        store.insert_capture(&capture.card, origin, false).unwrap()
    };
    let first = insert(&english(0, 30, "later"), CaptureOrigin::Capture);
    let second = insert(&english(0, 10, "earlier"), CaptureOrigin::Capture);
    insert(
        &chinese(0, 0, "学习", "我喜欢学习。", "xué xí"),
        CaptureOrigin::Curriculum,
    );
    let lessons = lessons_from_store(&store, 5).unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].id, format!("lesson-{second}"));
    assert_eq!(card_ids(&lessons[0]), [second, first]);
    assert_eq!(lessons[0].cards[0].cloze, "I like the word ____.");
}

#[test]
fn lessons_serialize_with_snake_case_fields() {
    let lessons = build_lessons(&[chinese(3, 0, "学习", "我喜欢学习。", "xué xí")], 4);
    assert_eq!(
        serde_json::to_value(&lessons).unwrap(),
        serde_json::json!([{
            "id": "lesson-3",
            "title": "ZH-CN → DE · Lesson 1",
            "source_lang": "zh-CN",
            "target_lang": "de",
            "cards": [{
                "card_id": 3,
                "word": "学习",
                "translation": "学习 (translated)",
                "sentence_source": "我喜欢学习。",
                "sentence_target": "Ein Satz.",
                "cloze": "我喜欢____。",
                "pinyin": "xué xí"
            }]
        }])
    );
}
