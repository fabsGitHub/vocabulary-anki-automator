//! Behaviour of the quiz engine's pool and exercise generators
//! (docs/architecture.md § 7.2, Milestone M2). Isolation over a large
//! curriculum is `quiz_isolation.rs`'s job; this file pins what each exercise
//! contains, how it is chosen and how answers are checked.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::json;
use vocab_core::{
    quiz::{
        check_answer, generate_exercise, ChoiceDirection, ChoiceOption, Exercise, ExerciseAnswer,
        ExerciseKind, LearnedVocabularyPool, MultipleChoiceExercise, QuizError,
        SentenceBuilderExercise, Tile,
    },
    store::{SrsStateRow, Store},
    CaptureOrigin, Card, ScriptMetadata, ScriptVariant,
};

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid test timestamp")
        .with_timezone(&Utc)
}

fn zh(front: &str, back: &str, pinyin: &str) -> Card {
    let mut card = Card::new(front, back, "zh-CN", "de", at("2026-10-01T08:00:00.000Z"));
    card.script_metadata = ScriptMetadata::Chinese {
        pinyin: pinyin.into(),
        hsk_level: Some(1),
        script_variant: ScriptVariant::Simplified,
    };
    card
}

fn en(front: &str, back: &str) -> Card {
    Card::new(front, back, "en", "de", at("2026-10-01T08:00:00.000Z"))
}

fn example(mut card: Card, source: &str, target: &str) -> Card {
    card.example_source = source.into();
    card.example_target = target.into();
    card
}

fn state(card_id: i64) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: None,
        difficulty: None,
        interval_days: 0,
        repetitions: 0,
        lapses: 0,
        fsrs_state: "new".into(),
        elapsed_days: 0,
        due_date: at("2026-11-01T08:00:00.000Z"),
        last_reviewed_at: None,
        last_rating: None,
    }
}

/// A store to which cards are added as learned or merely captured.
struct Deck {
    store: Store,
}

impl Deck {
    fn new() -> Self {
        Self {
            store: Store::open_in_memory().expect("in-memory store"),
        }
    }

    /// Inserts `card` with an `srs_state` row.
    fn learn(&self, card: Card) -> i64 {
        let id = self.capture(card);
        self.store.upsert_srs_state(&state(id)).expect("srs state");
        id
    }

    /// Inserts `card` without an `srs_state` row.
    fn capture(&self, card: Card) -> i64 {
        self.store
            .insert_capture(&card, CaptureOrigin::Capture, false)
            .expect("insert capture")
    }

    fn pool(&self) -> LearnedVocabularyPool {
        LearnedVocabularyPool::load(&self.store).expect("load pool")
    }
}

fn seeds() -> std::ops::Range<u32> {
    0..300
}

fn choice(pool: &LearnedVocabularyPool, card_id: i64, seed: u32) -> MultipleChoiceExercise {
    match generate_exercise(pool, card_id, ExerciseKind::MultipleChoice, seed) {
        Ok(Exercise::MultipleChoice(exercise)) => exercise,
        other => panic!("expected a multiple-choice exercise, got {other:?}"),
    }
}

fn builder(pool: &LearnedVocabularyPool, card_id: i64, seed: u32) -> SentenceBuilderExercise {
    match generate_exercise(pool, card_id, ExerciseKind::SentenceBuilder, seed) {
        Ok(Exercise::SentenceBuilder(exercise)) => exercise,
        other => panic!("expected a sentence-builder exercise, got {other:?}"),
    }
}

fn texts(options: &[ChoiceOption]) -> Vec<&str> {
    options.iter().map(|option| option.text.as_str()).collect()
}

/// The texts of the solution tiles, in sentence order.
fn solution_texts(tiles: &[Tile], solution: &[u32]) -> Vec<String> {
    solution
        .iter()
        .map(|id| {
            tiles
                .iter()
                .find(|tile| tile.id == *id)
                .expect("solution names a tile")
                .text
                .clone()
        })
        .collect()
}

fn distractor_texts(tiles: &[Tile], solution: &[u32]) -> Vec<String> {
    tiles
        .iter()
        .filter(|tile| !solution.contains(&tile.id))
        .map(|tile| tile.text.clone())
        .collect()
}

mod pool {
    use super::*;

    #[test]
    fn holds_exactly_the_captures_with_an_srs_state_in_card_id_order() {
        let deck = Deck::new();
        let first = deck.capture(zh("学习", "lernen", "xué xí"));
        let unlearned = deck.capture(zh("学生", "Schüler", "xué sheng"));
        let third = deck.capture(en("house", "Haus"));
        // States written out of card order.
        deck.store.upsert_srs_state(&state(third)).unwrap();
        deck.store.upsert_srs_state(&state(first)).unwrap();

        let pool = deck.pool();
        let ids: Vec<i64> = pool.entries().iter().map(|entry| entry.card_id).collect();
        assert_eq!(ids, [first, third]);
        assert_eq!(
            pool.get(first).map(|entry| entry.front.as_str()),
            Some("学习")
        );
        assert!(pool.get(unlearned).is_none());
        assert!(pool.get(9999).is_none());
        assert_eq!(pool.learned_hanzi_chars(), HashSet::from(['学', '习']));
    }

    #[test]
    fn entries_carry_the_card_fields_exercises_need() {
        let deck = Deck::new();
        let mut word = example(zh(" 学习 ", "lernen", "xué xí"), "我学习。", "Ich lerne.");
        word.script_metadata = ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(2),
            script_variant: ScriptVariant::Simplified,
        };
        word.audio_front = Some(vec![0xff, 0xfb]);
        let chinese = deck.learn(word);
        let mut silent = en("house", "Haus");
        silent.audio_front = Some(Vec::new());
        let english = deck.learn(silent);
        let mut blank_pinyin = zh("好", "gut", "  ");
        blank_pinyin.target_lang = "en".into();
        let blank = deck.learn(blank_pinyin);

        let pool = deck.pool();
        let entry = pool.get(chinese).unwrap();
        assert_eq!(entry.front, " 学习 ");
        assert_eq!(entry.back, "lernen");
        assert_eq!(entry.pinyin.as_deref(), Some("xué xí"));
        assert_eq!(entry.hsk_level, Some(2));
        assert_eq!(
            (entry.source_lang.as_str(), entry.target_lang.as_str()),
            ("zh-CN", "de")
        );
        assert_eq!(entry.example_source, "我学习。");
        assert_eq!(entry.example_target, "Ich lerne.");
        assert_eq!(entry.script_kind, "chinese");
        assert!(entry.has_audio);

        let entry = pool.get(english).unwrap();
        assert_eq!(entry.pinyin, None);
        assert_eq!(entry.hsk_level, None);
        assert_eq!(entry.script_kind, "none");
        assert!(!entry.has_audio, "an empty recording is no recording");

        let entry = pool.get(blank).unwrap();
        assert_eq!(entry.pinyin, None, "blank pinyin");
        assert_eq!(entry.target_lang, "en");
    }

    #[test]
    fn learned_hanzi_are_the_ideographs_of_fronts_only() {
        let deck = Deck::new();
        deck.learn(example(
            zh("AA制", "getrennt zahlen", "AA zhì"),
            "我们AA制。",
            "Wir zahlen getrennt.",
        ));
        let mut book = example(en("book", "书"), "I read a book.", "我读书。");
        book.target_lang = "zh-CN".into();
        deck.learn(book);
        deck.capture(zh("他", "er", "tā"));
        assert_eq!(deck.pool().learned_hanzi_chars(), HashSet::from(['制']));
    }

    #[test]
    fn an_empty_store_has_an_empty_pool() {
        let pool = Deck::new().pool();
        assert!(pool.entries().is_empty());
        assert!(pool.learned_hanzi_chars().is_empty());
        assert_eq!(
            generate_exercise(&pool, 1, ExerciseKind::MultipleChoice, 0),
            Err(QuizError::NotInPool { card_id: 1 })
        );
    }
}

mod multiple_choice {
    use super::*;

    /// Six learned Chinese words and two English ones.
    fn deck() -> (Deck, HashMap<&'static str, i64>) {
        let deck = Deck::new();
        let mut ids = HashMap::new();
        for (front, back, pinyin) in [
            ("学习", "lernen", "xué xí"),
            ("中文", "Chinesisch", "zhōng wén"),
            ("朋友", "Freund", "péng you"),
            ("老师", "Lehrer", "lǎo shī"),
            ("今天", "heute", "jīn tiān"),
            ("喜欢", "mögen", "xǐ huan"),
        ] {
            ids.insert(front, deck.learn(zh(front, back, pinyin)));
        }
        ids.insert("house", deck.learn(en("house", "Haus")));
        ids.insert("tree", deck.learn(en("tree", "Baum")));
        (deck, ids)
    }

    #[test]
    fn options_are_the_answer_and_three_distinct_distractors_from_the_pool() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let target = ids["学习"];
        for seed in seeds() {
            let exercise = choice(&pool, target, seed);
            assert_eq!(exercise.options.len(), 4, "seed {seed}");
            let correct = &exercise.options[exercise.correct_index];
            assert_eq!(correct.card_id, target);
            let (prompt, answer) = match exercise.direction {
                ChoiceDirection::FrontToBack => ("学习", "lernen"),
                ChoiceDirection::BackToFront => ("lernen", "学习"),
                ChoiceDirection::FrontToPinyin => ("学习", "xué xí"),
            };
            assert_eq!(
                (exercise.prompt.as_str(), correct.text.as_str()),
                (prompt, answer)
            );
            let distinct: HashSet<&str> = texts(&exercise.options).into_iter().collect();
            assert_eq!(distinct.len(), 4, "{:?}", exercise.options);
            for option in &exercise.options {
                let entry = pool
                    .get(option.card_id)
                    .expect("options come from the pool");
                let side = match exercise.direction {
                    ChoiceDirection::FrontToBack => entry.back.as_str(),
                    ChoiceDirection::BackToFront => entry.front.as_str(),
                    ChoiceDirection::FrontToPinyin => entry.pinyin.as_deref().unwrap(),
                };
                assert_eq!(option.text, side);
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_exercise_and_seeds_vary_everything() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let target = ids["中文"];
        let mut directions = HashSet::new();
        let mut positions = HashSet::new();
        let mut option_sets = HashSet::new();
        for seed in seeds() {
            let exercise = choice(&pool, target, seed);
            assert_eq!(exercise, choice(&deck.pool(), target, seed), "seed {seed}");
            directions.insert(exercise.direction);
            positions.insert(exercise.correct_index);
            let mut set: Vec<String> = exercise.options.iter().map(|o| o.text.clone()).collect();
            set.sort();
            option_sets.insert(set);
        }
        assert_eq!(directions.len(), 3);
        assert_eq!(positions, HashSet::from([0, 1, 2, 3]));
        assert!(
            option_sets.len() > 20,
            "{} distinct option sets",
            option_sets.len()
        );
    }

    #[test]
    fn the_correct_option_lands_on_every_position_about_equally_often() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let mut counts = [0usize; 4];
        for seed in 0..4000 {
            counts[choice(&pool, ids["朋友"], seed).correct_index] += 1;
        }
        for count in counts {
            assert!((800..1200).contains(&count), "{counts:?}");
        }
    }

    #[test]
    fn pinyin_is_only_asked_of_chinese_cards() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let mut directions = HashSet::new();
        for seed in seeds() {
            let exercise = choice(&pool, ids["house"], seed);
            directions.insert(exercise.direction);
            assert_eq!(
                exercise.prompt_lang,
                if exercise.direction == ChoiceDirection::FrontToBack {
                    "en"
                } else {
                    "de"
                }
            );
        }
        assert_eq!(
            directions,
            HashSet::from([ChoiceDirection::FrontToBack, ChoiceDirection::BackToFront])
        );
    }

    #[test]
    fn languages_name_what_the_prompt_and_the_options_are_in() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let mut seen = HashMap::new();
        for seed in seeds() {
            let exercise = choice(&pool, ids["学习"], seed);
            seen.insert(
                exercise.direction,
                (exercise.prompt_lang.clone(), exercise.option_lang.clone()),
            );
        }
        let pair = |prompt: &str, options: &str| (prompt.to_owned(), options.to_owned());
        assert_eq!(seen[&ChoiceDirection::FrontToBack], pair("zh-CN", "de"));
        assert_eq!(seen[&ChoiceDirection::BackToFront], pair("de", "zh-CN"));
        assert_eq!(
            seen[&ChoiceDirection::FrontToPinyin],
            pair("zh-CN", "zh-CN")
        );
    }

    #[test]
    fn distractors_come_from_the_same_language_pair_first() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let chinese: HashSet<i64> = ["学习", "中文", "朋友", "老师", "今天", "喜欢"]
            .iter()
            .map(|front| ids[front])
            .collect();
        for seed in seeds() {
            let exercise = choice(&pool, ids["老师"], seed);
            for option in &exercise.options {
                assert!(
                    chinese.contains(&option.card_id),
                    "seed {seed}: {:?}",
                    exercise.options
                );
            }
        }
        // Every option is in the exercise's option language. All cards here
        // translate into German, so "tree" -> meaning may draw German
        // distractors from the Chinese cards too; asked the other way round
        // (meaning -> English word) only the one other English card fits, and
        // no Chinese word is offered as an English answer.
        let english: HashSet<i64> = [ids["tree"], ids["house"]].into();
        for seed in seeds() {
            let exercise = choice(&pool, ids["tree"], seed);
            match exercise.direction {
                ChoiceDirection::FrontToBack => {
                    assert_eq!(exercise.option_lang, "de");
                    assert_eq!(exercise.options.len(), 4, "seed {seed}");
                }
                ChoiceDirection::BackToFront => {
                    assert_eq!(exercise.option_lang, "en");
                    let ids: HashSet<i64> = exercise
                        .options
                        .iter()
                        .map(|option| option.card_id)
                        .collect();
                    assert_eq!(ids, english, "seed {seed}: {:?}", exercise.options);
                }
                ChoiceDirection::FrontToPinyin => panic!("an English card has no pinyin"),
            }
        }
    }

    /// Asked for the word, every option is a Chinese-source front: the other
    /// card of the pair comes first, then the other card in Chinese script,
    /// and only the last slot goes to a front without script metadata.
    #[test]
    fn distractors_prefer_the_pair_then_the_script_then_the_rest() {
        let deck = Deck::new();
        let target = deck.learn(zh("学习", "lernen", "xué xí"));
        let pair = deck.learn(zh("中文", "Chinesisch", "zhōng wén"));
        let mut friend = zh("朋友", "friend", "péng you");
        friend.target_lang = "en".into();
        let script = deck.learn(friend);
        let rest: HashSet<i64> = ["OK", "App"]
            .into_iter()
            .map(|front| {
                deck.learn(Card::new(
                    front,
                    front.to_lowercase(),
                    "zh-CN",
                    "en",
                    at("2026-10-01T08:00:00.000Z"),
                ))
            })
            .collect();
        let pool = deck.pool();
        let mut asked = 0;
        for seed in seeds() {
            let exercise = choice(&pool, target, seed);
            if exercise.direction != ChoiceDirection::BackToFront {
                continue;
            }
            asked += 1;
            let ids: HashSet<i64> = exercise
                .options
                .iter()
                .map(|option| option.card_id)
                .collect();
            assert!(
                ids.contains(&pair) && ids.contains(&script),
                "seed {seed}: {:?}",
                exercise.options
            );
            assert_eq!(
                ids.intersection(&rest).count(),
                1,
                "seed {seed}: {:?}",
                exercise.options
            );
        }
        assert!(asked > 50, "{asked}");
    }

    #[test]
    fn no_option_is_a_second_correct_answer() {
        let deck = Deck::new();
        let target = deck.learn(zh("学习", "lernen", "xué xí"));
        // The same word captured again with another translation, a word with
        // the same translation, a translation that differs only in case and
        // spacing, and the target's pinyin written without the space.
        let twin = deck.learn(zh("学习", "studieren", "xué xí"));
        let synonym = deck.learn(zh("学", "lernen", "xué"));
        let lookalike = deck.learn(zh("念书", " Lernen ", "xuéxí"));
        deck.learn(zh("朋友", "Freund", "péng you"));
        deck.learn(zh("老师", "Lehrer", "lǎo shī"));
        let pool = deck.pool();
        for seed in seeds() {
            let exercise = choice(&pool, target, seed);
            let distractors: Vec<i64> = exercise
                .options
                .iter()
                .map(|option| option.card_id)
                .filter(|id| *id != target)
                .collect();
            assert!(
                !distractors.contains(&twin),
                "shares the prompt or the answer: {exercise:?}"
            );
            match exercise.direction {
                ChoiceDirection::FrontToBack | ChoiceDirection::FrontToPinyin => {
                    assert!(
                        !distractors.contains(&lookalike),
                        "reads like the answer: {exercise:?}"
                    )
                }
                ChoiceDirection::BackToFront => {
                    assert!(
                        !distractors.contains(&synonym),
                        "shares the prompt: {exercise:?}"
                    )
                }
            }
        }
    }

    #[test]
    fn a_card_with_nothing_to_compare_against_is_too_small_a_pool() {
        let deck = Deck::new();
        let alone = deck.learn(zh("学习", "lernen", "xué xí"));
        deck.capture(zh("中文", "Chinesisch", "zhōng wén"));
        let pool = deck.pool();
        for kind in [ExerciseKind::MultipleChoice, ExerciseKind::ListeningChoice] {
            assert_eq!(
                generate_exercise(&pool, alone, kind, 3),
                Err(QuizError::PoolTooSmall { card_id: alone })
            );
        }

        // A second learned word makes a two-option question.
        deck.learn(zh("朋友", "Freund", "péng you"));
        let pool = deck.pool();
        for seed in seeds() {
            assert_eq!(choice(&pool, alone, seed).options.len(), 2);
        }
    }

    #[test]
    fn a_card_without_a_translation_is_only_asked_for_what_it_has() {
        let deck = Deck::new();
        let untranslated = deck.learn(zh("学习", "", "xué xí"));
        let english = deck.learn(en("serendipity", " "));
        deck.learn(zh("朋友", "Freund", "péng you"));
        let pool = deck.pool();
        for seed in seeds() {
            assert_eq!(
                choice(&pool, untranslated, seed).direction,
                ChoiceDirection::FrontToPinyin
            );
        }
        assert_eq!(
            generate_exercise(&pool, english, ExerciseKind::MultipleChoice, 0),
            Err(QuizError::PoolTooSmall { card_id: english })
        );
    }

    #[test]
    fn cards_that_are_not_learned_are_never_quizzed() {
        let deck = Deck::new();
        deck.learn(zh("学习", "lernen", "xué xí"));
        deck.learn(zh("朋友", "Freund", "péng you"));
        let captured = deck.capture(zh("中文", "Chinesisch", "zhōng wén"));
        let pool = deck.pool();
        for kind in ExerciseKind::ALL {
            for card_id in [captured, 4242] {
                assert_eq!(
                    generate_exercise(&pool, card_id, kind, 1),
                    Err(QuizError::NotInPool { card_id })
                );
            }
        }
    }
}

mod listening_choice {
    use super::*;

    #[test]
    fn plays_the_stored_recording_or_names_what_to_synthesize() {
        let deck = Deck::new();
        let mut recorded = zh("学习", "lernen", "xué xí");
        recorded.audio_front = Some(vec![0xff, 0xfb, 0x90]);
        let recorded = deck.learn(recorded);
        let silent = deck.learn(en("house", "Haus"));
        deck.learn(zh("朋友", "Freund", "péng you"));
        deck.learn(en("tree", "Baum"));
        let pool = deck.pool();

        for (card_id, front, lang, audio_card_id) in [
            (recorded, "学习", "zh-CN", Some(recorded)),
            (silent, "house", "en", None),
        ] {
            for seed in seeds() {
                let Ok(Exercise::ListeningChoice(exercise)) =
                    generate_exercise(&pool, card_id, ExerciseKind::ListeningChoice, seed)
                else {
                    panic!("expected a listening exercise");
                };
                assert_eq!(exercise.audio.audio_card_id, audio_card_id);
                assert_eq!(
                    (exercise.audio.text.as_str(), exercise.audio.lang.as_str()),
                    (front, lang)
                );
                assert_eq!(exercise.options[exercise.correct_index].text, front);
                assert_eq!(exercise.options[exercise.correct_index].card_id, card_id);
                for option in &exercise.options {
                    assert_eq!(option.text, pool.get(option.card_id).unwrap().front);
                }
                assert_eq!(exercise.options.len(), 4);
            }
        }
    }

    #[test]
    fn never_offers_a_word_that_sounds_the_same() {
        let deck = Deck::new();
        let is = deck.learn(zh("是", "sein", "shì"));
        let thing = deck.learn(zh("事", "Sache", "shì"));
        let city = deck.learn(zh("市", "Stadt", "Shì "));
        deck.learn(zh("十", "zehn", "shí"));
        deck.learn(zh("书", "Buch", "shū"));
        let pool = deck.pool();
        for seed in seeds() {
            let Ok(Exercise::ListeningChoice(exercise)) =
                generate_exercise(&pool, is, ExerciseKind::ListeningChoice, seed)
            else {
                panic!("expected a listening exercise");
            };
            let ids: Vec<i64> = exercise
                .options
                .iter()
                .map(|option| option.card_id)
                .collect();
            assert!(
                !ids.contains(&thing) && !ids.contains(&city),
                "{:?}",
                exercise.options
            );
            assert_eq!(exercise.options.len(), 3);
        }
    }
}

mod sentence_builder {
    use super::*;

    /// Learned words whose sentences use each other, plus an unlearned trap.
    fn deck() -> (Deck, HashMap<&'static str, i64>) {
        let deck = Deck::new();
        let mut ids = HashMap::new();
        let mut learn = |front: &'static str, card: Card| {
            ids.insert(front, deck.learn(card));
        };
        learn(
            "学习",
            example(
                zh("学习", "lernen", "xué xí"),
                "学生很喜欢学习中文。",
                "Die Schüler lernen gern Chinesisch.",
            ),
        );
        learn(
            "中文",
            example(
                zh("中文", "Chinesisch", "zhōng wén"),
                "中文很难。",
                "Chinesisch ist schwer.",
            ),
        );
        learn("喜欢", zh("喜欢", "mögen", "xǐ huan"));
        learn("很", zh("很", "sehr", "hěn"));
        learn(
            "生活",
            example(
                zh("生活", "Leben", "shēng huó"),
                "我们喜欢生活。",
                "Wir mögen das Leben.",
            ),
        );
        learn("我们", zh("我们", "wir", "wǒ men"));
        learn("朋友", zh("朋友", "Freund", "péng you"));
        learn(
            "serendipity",
            example(
                en("serendipity", "glücklicher Zufall"),
                "It was pure serendipity.",
                "Es war reiner Zufall.",
            ),
        );
        learn("house", en("house", "Haus"));
        learn("book", en("book", "Buch"));
        ids.insert("学生", deck.capture(zh("学生", "Schüler", "xué sheng")));
        (deck, ids)
    }

    #[test]
    fn uses_the_targets_own_sentence_when_it_qualifies() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        for seed in seeds() {
            let exercise = builder(&pool, ids["学习"], seed);
            assert_eq!(exercise.sentence_card_id, ids["学习"]);
            assert_eq!(exercise.sentence, "学生很喜欢学习中文。");
            assert_eq!(exercise.prompt, "Die Schüler lernen gern Chinesisch.");
            assert_eq!(exercise.sentence_lang, "zh-CN");
        }
    }

    #[test]
    fn cuts_chinese_by_learned_fronts_only_with_single_character_fallback() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let exercise = builder(&pool, ids["学习"], 0);
        // 学生 is captured but not learned, and 学 and 生 are no learned
        // words: they come out one character each.
        assert_eq!(
            solution_texts(&exercise.tiles, &exercise.solution),
            ["学", "生", "很", "喜欢", "学习", "中文"]
        );
        assert!(exercise.tiles.iter().all(|tile| tile.text != "学生"));
    }

    #[test]
    fn otherwise_borrows_a_sentence_that_uses_the_target() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let mut used = HashSet::new();
        for seed in seeds() {
            let exercise = builder(&pool, ids["喜欢"], seed);
            used.insert(exercise.sentence_card_id);
            let lender = pool.get(exercise.sentence_card_id).unwrap();
            assert_eq!(exercise.sentence, lender.example_source);
            assert_eq!(exercise.prompt, lender.example_target);
            assert!(exercise.sentence.contains("喜欢"));
        }
        // Both sentences with 喜欢 are picked, never one without it.
        assert_eq!(used, HashSet::from([ids["学习"], ids["生活"]]));
    }

    #[test]
    fn a_sentence_with_an_unlearned_hanzi_is_never_used() {
        let (deck, ids) = deck();
        // 难 is not a learned character, so 中文很难 is out; 中文 is borrowed.
        let pool = deck.pool();
        for seed in seeds() {
            let exercise = builder(&pool, ids["中文"], seed);
            assert_eq!(exercise.sentence_card_id, ids["学习"], "seed {seed}");
        }
    }

    #[test]
    fn a_word_no_learned_sentence_uses_has_no_sentence_exercise() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        for kind in [
            ExerciseKind::SentenceBuilder,
            ExerciseKind::ListeningSentence,
        ] {
            assert_eq!(
                generate_exercise(&pool, ids["朋友"], kind, 0),
                Err(QuizError::NoEligibleSentence {
                    card_id: ids["朋友"]
                })
            );
        }
    }

    #[test]
    fn a_one_tile_sentence_is_not_worth_ordering() {
        let deck = Deck::new();
        let word = deck.learn(example(zh("学习", "lernen", "xué xí"), "学习！", "Lernen!"));
        deck.learn(zh("中文", "Chinesisch", "zhōng wén"));
        let pool = deck.pool();
        assert_eq!(
            generate_exercise(&pool, word, ExerciseKind::SentenceBuilder, 0),
            Err(QuizError::NoEligibleSentence { card_id: word })
        );
    }

    #[test]
    fn a_translation_with_unlearned_hanzi_rules_out_the_text_prompt_only() {
        let deck = Deck::new();
        let mut book = example(en("book", "Buch"), "I read a book.", "我读书。");
        book.target_lang = "zh-CN".into();
        let book = deck.learn(book);
        deck.learn(en("house", "Haus"));
        let pool = deck.pool();
        assert_eq!(
            generate_exercise(&pool, book, ExerciseKind::SentenceBuilder, 0),
            Err(QuizError::NoEligibleSentence { card_id: book })
        );
        let Ok(Exercise::ListeningSentence(exercise)) =
            generate_exercise(&pool, book, ExerciseKind::ListeningSentence, 0)
        else {
            panic!("the sentence itself is fine to hear");
        };
        assert_eq!(exercise.sentence, "I read a book.");
    }

    #[test]
    fn latin_sentences_are_cut_at_whitespace_and_match_the_word_in_any_case() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let exercise = builder(&pool, ids["serendipity"], 5);
        assert_eq!(
            solution_texts(&exercise.tiles, &exercise.solution),
            ["It", "was", "pure", "serendipity"]
        );
        assert_eq!(exercise.sentence_lang, "en");

        let deck = Deck::new();
        let word = deck.learn(example(
            en("Serendipity", "Zufall"),
            "pure serendipity.",
            "reiner Zufall.",
        ));
        deck.learn(en("luck", "Glück"));
        let exercise = builder(&deck.pool(), word, 0);
        assert_eq!(
            solution_texts(&exercise.tiles, &exercise.solution),
            ["pure", "serendipity"]
        );
    }

    #[test]
    fn distractor_tiles_are_learned_fronts_of_the_language_that_do_not_occur() {
        let (deck, ids) = deck();
        // A German word shares the English sentence's script but not its
        // language, and a pattern is a learned Chinese front but no word of
        // a sentence: neither may become a distractor tile.
        deck.learn(Card::new(
            "Haus",
            "house",
            "de",
            "en",
            at("2026-10-01T08:00:00.000Z"),
        ));
        deck.learn(zh("一…就…", "sobald", "yī … jiù …"));
        let pool = deck.pool();
        let chinese_fronts: HashSet<&str> =
            ["学习", "中文", "喜欢", "很", "生活", "我们", "朋友"].into();
        let mut counts = HashSet::new();
        for seed in seeds() {
            let exercise = builder(&pool, ids["生活"], seed);
            let distractors = distractor_texts(&exercise.tiles, &exercise.solution);
            counts.insert(distractors.len());
            for text in &distractors {
                assert!(chinese_fronts.contains(text.as_str()), "{text}");
                assert!(!exercise.sentence.contains(text.as_str()), "{text} occurs");
            }
            let distinct: HashSet<&String> = distractors.iter().collect();
            assert_eq!(distinct.len(), distractors.len());

            let exercise = builder(&pool, ids["serendipity"], seed);
            for text in distractor_texts(&exercise.tiles, &exercise.solution) {
                assert!(["house", "book"].contains(&text.as_str()), "{text}");
            }
        }
        assert_eq!(counts, HashSet::from([2]));

        // No candidates, no distractors.
        let deck = Deck::new();
        let only = deck.learn(example(
            en("rain", "Regen"),
            "the rain falls.",
            "der Regen fällt.",
        ));
        let exercise = builder(&deck.pool(), only, 0);
        assert!(distractor_texts(&exercise.tiles, &exercise.solution).is_empty());
    }

    #[test]
    fn tiles_are_shuffled_with_ids_that_are_their_positions() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let mut orders = HashSet::new();
        for seed in seeds() {
            let exercise = builder(&pool, ids["学习"], seed);
            for (position, tile) in exercise.tiles.iter().enumerate() {
                assert_eq!(tile.id as usize, position);
            }
            assert_eq!(exercise.tiles.len(), exercise.solution.len() + 2);
            orders.insert(exercise.solution.clone());
            assert!(check_answer(
                &Exercise::SentenceBuilder(exercise.clone()),
                &ExerciseAnswer::Order {
                    tile_ids: exercise.solution.clone()
                }
            ));
        }
        assert!(orders.len() > 100, "{} distinct orders", orders.len());
    }

    #[test]
    fn listening_sentence_speaks_the_sentence_in_its_language() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        for seed in seeds() {
            let Ok(Exercise::ListeningSentence(exercise)) = generate_exercise(
                &pool,
                ids["serendipity"],
                ExerciseKind::ListeningSentence,
                seed,
            ) else {
                panic!("expected a listening sentence");
            };
            assert_eq!(exercise.audio.text, "It was pure serendipity.");
            assert_eq!(exercise.audio.lang, "en");
            assert_eq!(exercise.audio.audio_card_id, None);
            assert_eq!(exercise.sentence, exercise.audio.text);
            assert_eq!(exercise.sentence_card_id, ids["serendipity"]);
            // The same sentence and tiles as the text variant with this seed.
            let text = builder(&pool, ids["serendipity"], seed);
            assert_eq!(
                (exercise.tiles, exercise.solution),
                (text.tiles, text.solution)
            );
        }
    }
}

mod answers {
    use super::*;

    fn choice_exercise() -> Exercise {
        Exercise::MultipleChoice(MultipleChoiceExercise {
            card_id: 1,
            direction: ChoiceDirection::FrontToBack,
            prompt: "学习".into(),
            prompt_lang: "zh-CN".into(),
            option_lang: "de".into(),
            options: ["Freund", "lernen", "Lehrer"]
                .iter()
                .enumerate()
                .map(|(i, text)| ChoiceOption {
                    card_id: i as i64 + 1,
                    text: (*text).into(),
                })
                .collect(),
            correct_index: 1,
        })
    }

    /// `你好你好很` with solution 你 好 你 好: equal texts are interchangeable.
    fn order_exercise() -> Exercise {
        Exercise::SentenceBuilder(SentenceBuilderExercise {
            card_id: 1,
            sentence_card_id: 1,
            prompt: "Hallo, hallo".into(),
            sentence: "你好，你好！".into(),
            sentence_lang: "zh-CN".into(),
            tiles: ["你", "好", "你", "好", "很"]
                .iter()
                .enumerate()
                .map(|(id, text)| Tile {
                    id: id as u32,
                    text: (*text).into(),
                })
                .collect(),
            solution: vec![0, 1, 2, 3],
        })
    }

    #[test]
    fn a_choice_is_correct_only_at_the_correct_index() {
        let exercise = choice_exercise();
        let answer = |index| ExerciseAnswer::Choice { index };
        assert!(check_answer(&exercise, &answer(1)));
        for wrong in [0, 2, 3, usize::MAX] {
            assert!(!check_answer(&exercise, &answer(wrong)), "{wrong}");
        }
        assert!(!check_answer(
            &exercise,
            &ExerciseAnswer::Order { tile_ids: vec![1] }
        ));
    }

    #[test]
    fn an_order_is_correct_when_its_tile_texts_spell_the_solution() {
        let exercise = order_exercise();
        let order = |ids: &[u32]| ExerciseAnswer::Order {
            tile_ids: ids.to_vec(),
        };
        for right in [[0, 1, 2, 3], [2, 1, 0, 3], [0, 3, 2, 1], [2, 3, 0, 1]] {
            assert!(check_answer(&exercise, &order(&right)), "{right:?}");
        }
        let wrong: [&[u32]; 8] = [
            &[1, 0, 2, 3],    // wrong order
            &[0, 1, 2, 4],    // a distractor
            &[0, 1, 2],       // too short
            &[0, 1, 2, 3, 4], // too long
            &[0, 1, 0, 3],    // a tile used twice
            &[0, 1, 2, 9],    // no such tile
            &[],
            &[4, 4, 4, 4],
        ];
        for ids in wrong {
            assert!(!check_answer(&exercise, &order(ids)), "{ids:?}");
        }
        assert!(!check_answer(
            &exercise,
            &ExerciseAnswer::Choice { index: 0 }
        ));
    }

    #[test]
    fn listening_exercises_are_checked_like_their_text_variants() {
        let deck = Deck::new();
        let word = deck.learn(example(
            zh("学习", "lernen", "xué xí"),
            "我们学习。",
            "Wir lernen.",
        ));
        deck.learn(zh("我们", "wir", "wǒ men"));
        let pool = deck.pool();
        let Ok(listening @ Exercise::ListeningChoice(_)) =
            generate_exercise(&pool, word, ExerciseKind::ListeningChoice, 4)
        else {
            panic!("expected a listening choice");
        };
        let Exercise::ListeningChoice(inner) = &listening else {
            unreachable!()
        };
        let correct = inner.correct_index;
        assert!(check_answer(
            &listening,
            &ExerciseAnswer::Choice { index: correct }
        ));
        assert!(!check_answer(
            &listening,
            &ExerciseAnswer::Choice { index: 1 - correct }
        ));

        let Ok(sentence @ Exercise::ListeningSentence(_)) =
            generate_exercise(&pool, word, ExerciseKind::ListeningSentence, 4)
        else {
            panic!("expected a listening sentence");
        };
        let Exercise::ListeningSentence(inner) = &sentence else {
            unreachable!()
        };
        let mut order = inner.solution.clone();
        assert!(check_answer(
            &sentence,
            &ExerciseAnswer::Order {
                tile_ids: order.clone()
            }
        ));
        order.reverse();
        assert!(!check_answer(
            &sentence,
            &ExerciseAnswer::Order { tile_ids: order }
        ));
    }
}

/// Parity vectors for the TypeScript port in Learn's web build (§ 12 item 7):
/// a fixed pool and seed must keep producing exactly these exercises. A
/// change here is a change to the documented algorithm (`quiz` module docs)
/// and has to be made in the port too.
mod parity {
    use super::*;

    fn deck() -> (Deck, HashMap<&'static str, i64>) {
        let deck = Deck::new();
        let ids = [
            (
                "学习",
                example(
                    zh("学习", "lernen", "xué xí"),
                    "我们学习中文。",
                    "Wir lernen Chinesisch.",
                ),
            ),
            ("中文", zh("中文", "Chinesisch", "zhōng wén")),
            ("我们", zh("我们", "wir", "wǒ men")),
            ("朋友", zh("朋友", "Freund", "péng you")),
            ("老师", zh("老师", "Lehrer", "lǎo shī")),
            ("house", en("house", "Haus")),
        ]
        .into_iter()
        .map(|(front, card)| (front, deck.learn(card)))
        .collect();
        (deck, ids)
    }

    /// Direction, option texts and correct index; or tile texts and solution.
    fn summary(exercise: &Exercise) -> String {
        let tiles = |tiles: &[Tile]| -> Vec<String> {
            tiles.iter().map(|tile| tile.text.clone()).collect()
        };
        match exercise {
            Exercise::MultipleChoice(e) => {
                format!(
                    "{:?} {:?} {}",
                    e.direction,
                    texts(&e.options),
                    e.correct_index
                )
            }
            Exercise::ListeningChoice(e) => format!("{:?} {}", texts(&e.options), e.correct_index),
            Exercise::SentenceBuilder(e) => format!("{:?} {:?}", tiles(&e.tiles), e.solution),
            Exercise::ListeningSentence(e) => format!("{:?} {:?}", tiles(&e.tiles), e.solution),
        }
    }

    #[test]
    fn generator_output_for_fixed_seeds() {
        let (deck, ids) = deck();
        let pool = deck.pool();
        let tiles_0 = r#"["学习", "中文", "老师", "朋友", "我们"] [4, 0, 1]"#;
        let tiles_42 = r#"["学习", "我们", "老师", "朋友", "中文"] [1, 0, 4]"#;
        let tiles_max = r#"["老师", "学习", "朋友", "中文", "我们"] [4, 1, 3]"#;
        let expected: [(u32, ExerciseKind, &str); 12] = [
            (
                0,
                ExerciseKind::MultipleChoice,
                r#"FrontToBack ["wir", "Lehrer", "lernen", "Freund"] 2"#,
            ),
            (0, ExerciseKind::SentenceBuilder, tiles_0),
            (
                0,
                ExerciseKind::ListeningChoice,
                r#"["朋友", "中文", "学习", "老师"] 2"#,
            ),
            (0, ExerciseKind::ListeningSentence, tiles_0),
            (
                42,
                ExerciseKind::MultipleChoice,
                r#"BackToFront ["朋友", "学习", "老师", "中文"] 1"#,
            ),
            (42, ExerciseKind::SentenceBuilder, tiles_42),
            (
                42,
                ExerciseKind::ListeningChoice,
                r#"["我们", "学习", "朋友", "中文"] 1"#,
            ),
            (42, ExerciseKind::ListeningSentence, tiles_42),
            (
                u32::MAX,
                ExerciseKind::MultipleChoice,
                r#"FrontToPinyin ["lǎo shī", "wǒ men", "xué xí", "péng you"] 2"#,
            ),
            (u32::MAX, ExerciseKind::SentenceBuilder, tiles_max),
            (
                u32::MAX,
                ExerciseKind::ListeningChoice,
                r#"["老师", "我们", "学习", "朋友"] 2"#,
            ),
            (u32::MAX, ExerciseKind::ListeningSentence, tiles_max),
        ];
        for (seed, kind, vector) in expected {
            let exercise = generate_exercise(&pool, ids["学习"], kind, seed).unwrap();
            assert_eq!(summary(&exercise), vector, "seed {seed}, {kind:?}");
        }
    }
}

mod shapes {
    use super::*;

    #[test]
    fn exercises_and_answers_serialize_with_snake_case_kind_tags() {
        let deck = Deck::new();
        let word = deck.learn(example(
            zh("学习", "lernen", "xué xí"),
            "我们学习。",
            "Wir lernen.",
        ));
        deck.learn(zh("我们", "wir", "wǒ men"));
        let pool = deck.pool();

        let expected_keys: [(ExerciseKind, &[&str]); 4] = [
            (
                ExerciseKind::MultipleChoice,
                &[
                    "card_id",
                    "correct_index",
                    "direction",
                    "kind",
                    "option_lang",
                    "options",
                    "prompt",
                    "prompt_lang",
                ],
            ),
            (
                ExerciseKind::SentenceBuilder,
                &[
                    "card_id",
                    "kind",
                    "prompt",
                    "sentence",
                    "sentence_card_id",
                    "sentence_lang",
                    "solution",
                    "tiles",
                ],
            ),
            (
                ExerciseKind::ListeningChoice,
                &["audio", "card_id", "correct_index", "kind", "options"],
            ),
            (
                ExerciseKind::ListeningSentence,
                &[
                    "audio",
                    "card_id",
                    "kind",
                    "sentence",
                    "sentence_card_id",
                    "solution",
                    "tiles",
                ],
            ),
        ];
        for (kind, keys) in expected_keys {
            let exercise = generate_exercise(&pool, word, kind, 0).unwrap();
            assert_eq!(exercise.kind(), kind);
            assert_eq!(exercise.card_id(), word);
            let value = serde_json::to_value(&exercise).unwrap();
            assert_eq!(value["kind"], kind.as_str());
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
            let mut found: Vec<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            found.sort_unstable();
            assert_eq!(found, keys, "{kind:?}");
            let back: Exercise = serde_json::from_value(value).unwrap();
            assert_eq!(back, exercise);
        }

        let Exercise::ListeningChoice(listening) =
            generate_exercise(&pool, word, ExerciseKind::ListeningChoice, 0).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(
            serde_json::to_value(&listening.audio).unwrap(),
            json!({ "audio_card_id": null, "text": "学习", "lang": "zh-CN" })
        );
        assert_eq!(
            serde_json::to_value(ChoiceDirection::FrontToPinyin).unwrap(),
            "front_to_pinyin"
        );

        assert_eq!(
            serde_json::to_value(ExerciseAnswer::Choice { index: 2 }).unwrap(),
            json!({ "kind": "choice", "index": 2 })
        );
        let order: ExerciseAnswer =
            serde_json::from_value(json!({ "kind": "order", "tile_ids": [1, 0] })).unwrap();
        assert_eq!(
            order,
            ExerciseAnswer::Order {
                tile_ids: vec![1, 0]
            }
        );
        assert_eq!(
            serde_json::to_value(QuizError::PoolTooSmall { card_id: 3 }).unwrap(),
            json!({ "kind": "pool_too_small", "card_id": 3 })
        );
        assert_eq!(
            serde_json::from_value::<ExerciseKind>(json!("listening_sentence")).unwrap(),
            ExerciseKind::ListeningSentence
        );
    }
}

/// The rules above, each pinned on a hand-made deck, rechecked on messy
/// seeded decks: the same word captured twice, fronts that differ only in
/// case, homophones and pinyin written with and without spaces, blank
/// pinyin, translations into Chinese, sentences full of punctuation and
/// unlearned hanzi, and captures that were never learned.
mod randomized {
    use super::*;
    use vocab_core::{
        language::{cjk_chars, contains_cjk},
        quiz::PoolEntry,
        rng::Mulberry32,
    };

    const HANZI: &[char] = &[
        '我', '你', '他', '们', '学', '习', '生', '中', '文', '喜', '欢', '很', '好', '是', '朋',
        '友', '老', '师', '今', '天', '气', '的', '吗', '这', '书',
    ];
    const LATIN: &[&str] = &[
        "house",
        "House",
        "cat",
        "Concatenate",
        "the",
        "The",
        "run",
        "running",
        "über",
        "Über",
        "don't",
        "tree",
    ];
    const PINYIN: &[&str] = &["shì", "Shì ", "xué xí", "xuéxí", "  ", "hǎo", "nǐ hǎo"];
    const GLUE: &[&str] = &["，", "。", "！", " ", "", "", ""];

    fn any<'a, T>(rng: &mut Mulberry32, items: &'a [T]) -> &'a T {
        &items[rng.below(items.len() as u32) as usize]
    }

    fn hanzi(rng: &mut Mulberry32, max_len: u32) -> String {
        (0..1 + rng.below(max_len))
            .map(|_| *any(rng, HANZI))
            .collect()
    }

    fn chinese_sentence(rng: &mut Mulberry32, words: &[String]) -> String {
        let mut sentence = String::new();
        for _ in 0..1 + rng.below(5) {
            if !words.is_empty() && rng.below(2) == 0 {
                sentence.push_str(any(rng, words));
            } else {
                sentence.push_str(&hanzi(rng, 2));
            }
            sentence.push_str(any(rng, GLUE));
        }
        sentence
    }

    fn latin_sentence(rng: &mut Mulberry32) -> String {
        (0..1 + rng.below(5))
            .map(|_| format!("{}{}", any(rng, LATIN), any(rng, &["", "", ",", "!"])))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn messy_deck(seed: u32) -> Deck {
        let mut rng = Mulberry32::new(seed);
        let deck = Deck::new();
        let mut words = Vec::new();
        for _ in 0..4 + rng.below(16) {
            let mut card = match rng.below(5) {
                0..=2 => {
                    let front = hanzi(&mut rng, 3);
                    let mut card = zh(&front, any(&mut rng, LATIN), any(&mut rng, PINYIN));
                    words.push(front);
                    if rng.below(2) == 0 {
                        card.target_lang = "en".into();
                    }
                    card.example_source = chinese_sentence(&mut rng, &words);
                    card.example_target = latin_sentence(&mut rng);
                    card
                }
                3 => example(
                    en(any(&mut rng, LATIN), any(&mut rng, LATIN)),
                    &latin_sentence(&mut rng),
                    &latin_sentence(&mut rng),
                ),
                _ => {
                    let mut card = example(
                        en(any(&mut rng, LATIN), &hanzi(&mut rng, 2)),
                        &latin_sentence(&mut rng),
                        &chinese_sentence(&mut rng, &words),
                    );
                    card.target_lang = "zh-CN".into();
                    card
                }
            };
            if rng.below(3) == 0 {
                card.audio_front = Some(vec![0xff, 0xfb]);
            }
            if rng.below(5) == 0 {
                deck.capture(card);
            } else {
                deck.learn(card);
            }
        }
        deck
    }

    fn fold(text: &str) -> String {
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    fn fold_pinyin(text: &str) -> String {
        text.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_lowercase()
    }

    /// What `direction` shows of `entry` (`prompt`) or asks for, trimmed.
    fn side(entry: &PoolEntry, direction: ChoiceDirection, prompt: bool) -> &str {
        let text = match (direction, prompt) {
            (ChoiceDirection::BackToFront, true) | (ChoiceDirection::FrontToBack, false) => {
                entry.back.as_str()
            }
            (ChoiceDirection::FrontToPinyin, false) => entry.pinyin.as_deref().unwrap_or_default(),
            _ => entry.front.as_str(),
        };
        text.trim()
    }

    /// Everything wrong with a choice: the correct option must be the
    /// target's, texts must differ, and no distractor may be a second right
    /// answer (the same answer, or a card that the prompt also describes).
    fn choice_problems(
        pool: &LearnedVocabularyPool,
        target: &PoolEntry,
        options: &[ChoiceOption],
        correct_index: usize,
        answer: impl Fn(&PoolEntry) -> &str,
        prompt: impl Fn(&PoolEntry) -> &str,
        key: impl Fn(&str) -> String,
    ) -> Vec<String> {
        let mut found = Vec::new();
        let correct = &options[correct_index];
        if !(2..=4).contains(&options.len())
            || correct.card_id != target.card_id
            || correct.text != answer(target)
        {
            found.push(format!("correct option {correct:?} of {options:?}"));
        }
        let keys: HashSet<String> = options.iter().map(|option| key(&option.text)).collect();
        if keys.len() != options.len() {
            found.push(format!("options read alike: {options:?}"));
        }
        for (index, option) in options.iter().enumerate() {
            if index == correct_index {
                continue;
            }
            let Some(entry) = pool.get(option.card_id) else {
                found.push(format!("{option:?} is not in the pool"));
                continue;
            };
            if entry.card_id == target.card_id || option.text != answer(entry) {
                found.push(format!("{option:?} is not another card's answer"));
            }
            if fold(prompt(entry)) == fold(prompt(target)) {
                found.push(format!("{option:?} is described by the prompt too"));
            }
        }
        found
    }

    fn sentence_problems(
        pool: &LearnedVocabularyPool,
        target: &PoolEntry,
        exercise: &Exercise,
        (sentence_card_id, sentence, tiles, solution): (i64, &str, &[Tile], &[u32]),
    ) -> Vec<String> {
        let mut found = Vec::new();
        let lender = pool.get(sentence_card_id).expect("the lender is learned");
        let word = target.front.trim();
        let uses_word = if contains_cjk(word) {
            sentence.contains(word)
        } else {
            sentence.to_lowercase().contains(&word.to_lowercase())
        };
        if lender.example_source.trim() != sentence || !uses_word {
            found.push(format!("sentence {sentence:?} for {word:?}"));
        }
        if tiles
            .iter()
            .enumerate()
            .any(|(position, tile)| tile.id as usize != position)
        {
            found.push("tile ids are not positions".into());
        }
        let spelled = solution_texts(tiles, solution);
        // The decks' sentences hold no combining marks, so letters and
        // digits are all the word characters there are.
        let expected: Vec<String> = if contains_cjk(sentence) {
            vec![sentence.chars().filter(|c| c.is_alphanumeric()).collect()]
        } else {
            sentence
                .split_whitespace()
                .map(|token| token.trim_matches(|c: char| !c.is_alphanumeric()))
                .filter(|word| !word.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let joined = if contains_cjk(sentence) {
            vec![spelled.concat()]
        } else {
            spelled.clone()
        };
        if joined != expected {
            found.push(format!("solution {spelled:?} does not spell {sentence:?}"));
        }
        let solution_keys: HashSet<String> = spelled.iter().map(|text| fold(text)).collect();
        for text in distractor_texts(tiles, solution) {
            if solution_keys.contains(&fold(&text)) {
                found.push(format!("distractor {text:?} is also a sentence tile"));
            }
        }
        let answer = ExerciseAnswer::Order {
            tile_ids: solution.to_vec(),
        };
        if !check_answer(exercise, &answer) {
            found.push("the solution is not accepted".into());
        }
        found
    }

    fn shown(exercise: &Exercise) -> Vec<&str> {
        match exercise {
            Exercise::MultipleChoice(e) => [e.prompt.as_str()]
                .into_iter()
                .chain(texts(&e.options))
                .collect(),
            Exercise::ListeningChoice(e) => [e.audio.text.as_str()]
                .into_iter()
                .chain(texts(&e.options))
                .collect(),
            Exercise::SentenceBuilder(e) => [e.prompt.as_str(), e.sentence.as_str()]
                .into_iter()
                .chain(e.tiles.iter().map(|tile| tile.text.as_str()))
                .collect(),
            Exercise::ListeningSentence(e) => [e.audio.text.as_str(), e.sentence.as_str()]
                .into_iter()
                .chain(e.tiles.iter().map(|tile| tile.text.as_str()))
                .collect(),
        }
    }

    #[test]
    fn messy_decks_never_break_the_exercise_rules() {
        let mut generated: HashMap<ExerciseKind, usize> = HashMap::new();
        let mut problems = Vec::new();
        for deck_seed in 0..40 {
            let deck = messy_deck(deck_seed);
            let pool = deck.pool();
            let learned = pool.learned_hanzi_chars();
            for target in pool.entries() {
                for kind in ExerciseKind::ALL {
                    for seed in 0..3 {
                        let Ok(exercise) = generate_exercise(&pool, target.card_id, kind, seed)
                        else {
                            continue;
                        };
                        *generated.entry(kind).or_default() += 1;
                        let mut found: Vec<String> = shown(&exercise)
                            .into_iter()
                            .flat_map(cjk_chars)
                            .filter(|c| !learned.contains(c))
                            .map(|c| format!("unlearned hanzi {c}"))
                            .collect();
                        found.extend(match &exercise {
                            Exercise::MultipleChoice(e) => choice_problems(
                                &pool,
                                target,
                                &e.options,
                                e.correct_index,
                                |entry| side(entry, e.direction, false),
                                |entry| side(entry, e.direction, true),
                                |text| match e.direction {
                                    ChoiceDirection::FrontToPinyin => fold_pinyin(text),
                                    _ => fold(text),
                                },
                            ),
                            Exercise::ListeningChoice(e) => {
                                let mut found = choice_problems(
                                    &pool,
                                    target,
                                    &e.options,
                                    e.correct_index,
                                    |entry| entry.front.trim(),
                                    |entry| entry.front.trim(),
                                    fold,
                                );
                                let sound = target.pinyin.as_deref().map(fold_pinyin);
                                for option in &e.options {
                                    let entry = pool.get(option.card_id).expect("in the pool");
                                    if entry.card_id != target.card_id
                                        && sound.is_some()
                                        && entry.pinyin.as_deref().map(fold_pinyin) == sound
                                    {
                                        found.push(format!("{option:?} sounds the same"));
                                    }
                                }
                                found
                            }
                            Exercise::SentenceBuilder(e) => sentence_problems(
                                &pool,
                                target,
                                &exercise,
                                (e.sentence_card_id, &e.sentence, &e.tiles, &e.solution),
                            ),
                            Exercise::ListeningSentence(e) => sentence_problems(
                                &pool,
                                target,
                                &exercise,
                                (e.sentence_card_id, &e.sentence, &e.tiles, &e.solution),
                            ),
                        });
                        problems.extend(found.into_iter().map(|problem| {
                            format!(
                                "deck {deck_seed}, card {}, {kind:?}, seed {seed}: {problem}",
                                target.card_id
                            )
                        }));
                    }
                }
            }
        }
        assert!(
            problems.is_empty(),
            "{} problems, the first ones:\n{}",
            problems.len(),
            problems[..problems.len().min(10)].join("\n")
        );
        for kind in ExerciseKind::ALL {
            assert!(
                generated.get(&kind).copied().unwrap_or(0) > 200,
                "too few {kind:?}: {generated:?}"
            );
        }
    }
}
