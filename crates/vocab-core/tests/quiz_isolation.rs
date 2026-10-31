//! Milestone G+'s leak test, the one the plan marks "explicitly required": no
//! exercise may show vocabulary the learner has not learned (Milestone M1,
//! docs/architecture.md § 7.2).
//!
//! The database holds far more than the learner knows: a 300-word HSK
//! curriculum whose example sentences are full of unknown characters,
//! captures that were never introduced (no `srs_state` row), and trap words
//! such as 学生 that consist of learned characters without having been
//! learned, which a character check alone would let through. Every exercise
//! kind is generated for every learned card over 200+ seeds, and every
//! string of every exercise is checked against what this file knows to be
//! learned — never against the pool under test, which a broken query would
//! corrupt together with its own `learned_hanzi_chars`.
//!
//! If this test ever passes while a generator reads anything but
//! `LearnedVocabularyPool::load`, the test is broken (CLAUDE.md).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::Value;
use vocab_core::{
    language::{cjk_chars, is_cjk_char},
    quiz::{
        generate_exercise, ChoiceDirection, ChoiceOption, Exercise, ExerciseKind,
        LearnedVocabularyPool, QuizError, Tile,
    },
    store::{SrsStateRow, Store},
    CaptureOrigin, Card, CurriculumItem, ScriptMetadata, ScriptVariant,
};

/// One card the fixture writes into `captures`.
struct Word {
    front: &'static str,
    back: &'static str,
    pinyin: Option<&'static str>,
    pair: (&'static str, &'static str),
    example: (&'static str, &'static str),
    origin: CaptureOrigin,
    audio: bool,
}

const fn zh(
    front: &'static str,
    back: &'static str,
    pinyin: &'static str,
    example: (&'static str, &'static str),
    origin: CaptureOrigin,
    audio: bool,
) -> Word {
    Word {
        front,
        back,
        pinyin: Some(pinyin),
        pair: ("zh-CN", "de"),
        example,
        origin,
        audio,
    }
}

const fn other(
    front: &'static str,
    back: &'static str,
    pair: (&'static str, &'static str),
    example: (&'static str, &'static str),
    audio: bool,
) -> Word {
    Word {
        front,
        back,
        pinyin: None,
        pair,
        example,
        origin: CaptureOrigin::Capture,
        audio,
    }
}

const CAPTURE: CaptureOrigin = CaptureOrigin::Capture;
const CURRICULUM: CaptureOrigin = CaptureOrigin::Curriculum;

/// The learner's vocabulary: these get `srs_state` rows. Several example
/// sentences contain unlearned hanzi (的, 他, 气, 吗, 这, 书) and must never be
/// shown; 学生很喜欢学习中文 hides the trap word 学生 among learned words.
const LEARNED: [Word; 15] = [
    zh(
        "我们",
        "wir",
        "wǒ men",
        ("我们是朋友。", "Wir sind Freunde."),
        CAPTURE,
        true,
    ),
    zh(
        "学习",
        "lernen",
        "xué xí",
        (
            "学生很喜欢学习中文。",
            "Die Schüler lernen gern Chinesisch.",
        ),
        CURRICULUM,
        false,
    ),
    zh(
        "中文",
        "Chinesisch",
        "zhōng wén",
        ("我们的中文老师很好。", "Unser Chinesischlehrer ist gut."),
        CAPTURE,
        true,
    ),
    zh("喜欢", "mögen", "xǐ huan", ("", ""), CAPTURE, false),
    zh(
        "朋友",
        "Freund",
        "péng you",
        ("他是我的好朋友。", "Er ist mein guter Freund."),
        CURRICULUM,
        false,
    ),
    zh(
        "老师",
        "Lehrer",
        "lǎo shī",
        ("老师今天很好。", "Dem Lehrer geht es heute gut."),
        CAPTURE,
        true,
    ),
    zh(
        "今天",
        "heute",
        "jīn tiān",
        ("今天天气很好。", "Heute ist das Wetter gut."),
        CAPTURE,
        false,
    ),
    zh(
        "生活",
        "Leben",
        "shēng huó",
        ("你好！我们喜欢生活。", "Hallo! Wir mögen das Leben."),
        CURRICULUM,
        false,
    ),
    zh(
        "你",
        "du",
        "nǐ",
        ("你好吗？", "Wie geht es dir?"),
        CAPTURE,
        true,
    ),
    zh("好", "gut", "hǎo", ("", ""), CURRICULUM, false),
    zh(
        "是",
        "sein",
        "shì",
        ("这是我的书。", "Das ist mein Buch."),
        CAPTURE,
        false,
    ),
    zh("很", "sehr", "hěn", ("很好。", "Sehr gut."), CAPTURE, false),
    other(
        "serendipity",
        "glücklicher Zufall",
        ("en", "de"),
        ("It was pure serendipity.", "Es war reiner Zufall."),
        true,
    ),
    other(
        "house",
        "Haus",
        ("en", "de"),
        ("The house is big.", "Das Haus ist groß."),
        false,
    ),
    // Learned, but its translation 书 and its example's translation 我读书
    // hold hanzi no learned front has: only the English sides may be shown.
    other(
        "book",
        "书",
        ("en", "zh-CN"),
        ("I read a book.", "我读书。"),
        false,
    ),
];

/// Captured but never introduced: no `srs_state` row. 学生, 你好 and 老友 are
/// made of learned characters only, and so are the sentences of 你好 and 老友.
const NOT_LEARNED: [Word; 5] = [
    zh(
        "学生",
        "Schüler",
        "xué sheng",
        ("学生是我们的朋友。", "Die Schüler sind unsere Freunde."),
        CAPTURE,
        true,
    ),
    zh(
        "你好",
        "hallo",
        "nǐ hǎo",
        ("你好，老师！", "Hallo, Herr Lehrer!"),
        CAPTURE,
        true,
    ),
    zh(
        "他",
        "er",
        "tā",
        ("他很好。", "Ihm geht es gut."),
        CAPTURE,
        false,
    ),
    zh(
        "老友",
        "alter Freund",
        "lǎo yǒu",
        ("老友今天很好。", "Dem alten Freund geht es heute gut."),
        CAPTURE,
        false,
    ),
    zh(
        "事",
        "Sache",
        "shì",
        ("这是好事。", "Das ist eine gute Sache."),
        CAPTURE,
        false,
    ),
];

/// Unlearned curriculum words made of learned characters only.
const TRAP_WORDS: [&str; 12] = [
    "学生", "你好", "老友", "中学", "很好", "好友", "天生", "文学", "你们", "师生", "今生", "好学",
];

/// Unlearned single-character curriculum words. The first eight are learned
/// characters, so they may legitimately appear as fallback tiles.
const SINGLE_CHARACTER_WORDS: [&str; 10] =
    ["学", "生", "我", "中", "天", "老", "文", "喜", "他", "事"];

const CURRICULUM_SIZE: usize = 300;

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid test timestamp")
        .with_timezone(&Utc)
}

fn card(word: &Word) -> Card {
    let mut card = Card::new(
        word.front,
        word.back,
        word.pair.0,
        word.pair.1,
        at("2026-10-01T08:00:00.000Z"),
    );
    card.example_source = word.example.0.into();
    card.example_target = word.example.1.into();
    if let Some(pinyin) = word.pinyin {
        card.script_metadata = ScriptMetadata::Chinese {
            pinyin: pinyin.into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        };
    }
    if word.audio {
        card.audio_front = Some(vec![0xff, 0xfb, 0x90, 0x00]);
    }
    card
}

fn learned_state(card_id: i64) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: Some(3.2),
        difficulty: Some(5.1),
        interval_days: 3,
        repetitions: 2,
        lapses: 0,
        fsrs_state: "review".into(),
        elapsed_days: 1,
        due_date: at("2026-11-02T08:00:00.000Z"),
        last_reviewed_at: Some(at("2026-10-30T08:00:00.000Z")),
        last_rating: Some(3),
    }
}

fn learned_chars() -> HashSet<char> {
    LEARNED
        .iter()
        .flat_map(|word| word.front.chars())
        .filter(|c| is_cjk_char(*c))
        .collect()
}

/// 300 distinct words: the learned Chinese words (the curriculum-introduced
/// ones with the sentence their capture copied), the trap and
/// single-character words, and filler words of unlearned hanzi. Every other
/// example sentence mixes a learned word into unlearned hanzi, so a
/// curriculum sentence that leaks passes the "contains the target" test.
fn curriculum() -> Vec<CurriculumItem> {
    let learned = learned_chars();
    let unlearned: Vec<char> = ('\u{5000}'..='\u{9FFF}')
        .filter(|c| !learned.contains(c))
        .take(1200)
        .collect();
    let chinese: Vec<&Word> = LEARNED.iter().filter(|w| w.pinyin.is_some()).collect();

    let mut words: Vec<(String, Option<&Word>)> = chinese
        .iter()
        .map(|word| (word.front.to_owned(), Some(*word)))
        .collect();
    words.extend(
        TRAP_WORDS
            .iter()
            .chain(&SINGLE_CHARACTER_WORDS)
            .map(|word| ((*word).to_owned(), None)),
    );
    let mut filler = unlearned.chunks(2);
    while words.len() < CURRICULUM_SIZE {
        let pair: String = filler.next().expect("enough filler hanzi").iter().collect();
        words.push((pair, None));
    }

    words
        .into_iter()
        .enumerate()
        .map(|(order, (hanzi, learned_word))| {
            let u = |n: usize| unlearned[(600 + n) % unlearned.len()];
            let (pinyin, meaning_de, sentence) = match learned_word {
                Some(word) if word.origin == CaptureOrigin::Curriculum => (
                    word.pinyin.unwrap_or_default().to_owned(),
                    word.back.to_owned(),
                    word.example.0.to_owned(),
                ),
                _ => {
                    let inner = chinese[order % chinese.len()].front;
                    (
                        format!("pīn {order}"),
                        format!("Bedeutung {order}"),
                        format!(
                            "{}{}{inner}{}的{hanzi}。",
                            u(3 * order),
                            u(3 * order + 1),
                            u(3 * order + 2)
                        ),
                    )
                }
            };
            CurriculumItem {
                hanzi,
                traditional: None,
                hsk_level: (order % 6) as u8 + 1,
                frequency: Some(order as i64),
                curriculum_order: order as i64,
                pinyin,
                pos: None,
                meaning_en: Some(format!("meaning {order}")),
                meaning_de: Some(meaning_de),
                cloze_sentence: None,
                example_sentence_source: (!sentence.is_empty()).then_some(sentence),
                example_sentence_pinyin: None,
                example_sentence_target: Some(format!("Beispielsatz {order}")),
                vocab_audio_path: None,
                sentence_audio_path: None,
                image_path: None,
            }
        })
        .collect()
}

/// What the fixture knows to be learned, built from its own tables.
struct Truth {
    learned: HashMap<i64, &'static Word>,
    not_learned: HashMap<i64, &'static Word>,
    learned_chars: HashSet<char>,
    learned_fronts: HashSet<&'static str>,
    /// Curriculum words and captured fronts that are not learned fronts.
    unlearned_words: HashSet<String>,
    curriculum: Vec<CurriculumItem>,
}

struct Fixture {
    store: Store,
    truth: Truth,
}

impl Fixture {
    fn new() -> Self {
        let mut store = Store::open_in_memory().expect("in-memory store");
        let curriculum = curriculum();
        store
            .replace_curriculum(&curriculum)
            .expect("seed the curriculum");

        let mut learned = HashMap::new();
        let mut not_learned = HashMap::new();
        // Interleaved, so learned and unlearned card ids alternate.
        for (index, word) in LEARNED.iter().enumerate() {
            let id = store
                .insert_capture(&card(word), word.origin, false)
                .expect("insert learned capture");
            store
                .upsert_srs_state(&learned_state(id))
                .expect("make it learned");
            learned.insert(id, word);
            if let Some(word) = NOT_LEARNED.get(index) {
                let id = store
                    .insert_capture(&card(word), word.origin, false)
                    .expect("insert unlearned capture");
                not_learned.insert(id, word);
            }
        }

        let learned_fronts: HashSet<&str> = LEARNED.iter().map(|word| word.front).collect();
        let unlearned_words = curriculum
            .iter()
            .map(|item| item.hanzi.as_str())
            .chain(NOT_LEARNED.iter().map(|word| word.front))
            .filter(|word| !learned_fronts.contains(word))
            .map(str::to_owned)
            .collect();
        let truth = Truth {
            learned,
            not_learned,
            learned_chars: learned_chars(),
            learned_fronts,
            unlearned_words,
            curriculum,
        };
        Self { store, truth }
    }

    fn pool(&self) -> LearnedVocabularyPool {
        LearnedVocabularyPool::load(&self.store).expect("load the pool")
    }
}

fn seeds() -> impl Iterator<Item = u32> {
    (0..200).chain([0x8000_0000, 0xDEAD_BEEF, u32::MAX])
}

/// How a string is shown, for the one exemption the checker makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Prompt,
    Option,
    Audio,
    Sentence,
    Tile,
}

fn options(options: &[ChoiceOption]) -> impl Iterator<Item = (Role, &str)> {
    options
        .iter()
        .map(|option| (Role::Option, option.text.as_str()))
}

fn tiles(tiles: &[Tile]) -> impl Iterator<Item = (Role, &str)> {
    tiles.iter().map(|tile| (Role::Tile, tile.text.as_str()))
}

fn shown_texts(exercise: &Exercise) -> Vec<(Role, &str)> {
    match exercise {
        Exercise::MultipleChoice(e) => {
            let mut texts = vec![(Role::Prompt, e.prompt.as_str())];
            texts.extend(options(&e.options));
            texts
        }
        Exercise::ListeningChoice(e) => {
            let mut texts = vec![(Role::Audio, e.audio.text.as_str())];
            texts.extend(options(&e.options));
            texts
        }
        Exercise::SentenceBuilder(e) => {
            let mut texts = vec![
                (Role::Prompt, e.prompt.as_str()),
                (Role::Sentence, e.sentence.as_str()),
            ];
            texts.extend(tiles(&e.tiles));
            texts
        }
        Exercise::ListeningSentence(e) => {
            let mut texts = vec![
                (Role::Audio, e.audio.text.as_str()),
                (Role::Sentence, e.sentence.as_str()),
            ];
            texts.extend(tiles(&e.tiles));
            texts
        }
    }
}

/// Every string anywhere in the serialized exercise, including fields this
/// file does not know about yet.
fn all_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| all_strings(item, out)),
        Value::Object(fields) => fields.values().for_each(|field| all_strings(field, out)),
        _ => {}
    }
}

fn prompt_side(word: &Word, direction: ChoiceDirection) -> &str {
    match direction {
        ChoiceDirection::FrontToBack | ChoiceDirection::FrontToPinyin => word.front,
        ChoiceDirection::BackToFront => word.back,
    }
}

fn answer_side(word: &Word, direction: ChoiceDirection) -> &str {
    match direction {
        ChoiceDirection::FrontToBack => word.back,
        ChoiceDirection::BackToFront => word.front,
        ChoiceDirection::FrontToPinyin => word.pinyin.unwrap_or_default(),
    }
}

impl Truth {
    /// Everything wrong with `exercise`; empty when it only shows learned
    /// vocabulary, taken from where it should be taken.
    fn violations(&self, exercise: &Exercise) -> Vec<String> {
        let mut found = Vec::new();

        let json = serde_json::to_value(exercise).expect("exercises serialize");
        let mut strings = Vec::new();
        all_strings(&json, &mut strings);
        for text in &strings {
            for c in cjk_chars(text) {
                if !self.learned_chars.contains(&c) {
                    found.push(format!("unlearned hanzi {c} in {text:?}"));
                }
            }
        }

        for (role, text) in shown_texts(exercise) {
            let text = text.trim();
            let fallback_tile = role == Role::Tile
                && text.chars().count() == 1
                && text.chars().all(|c| self.learned_chars.contains(&c));
            if self.unlearned_words.contains(text) && !fallback_tile {
                found.push(format!("unlearned word {text:?} shown as {role:?}"));
            }
        }

        found.extend(self.provenance(exercise));
        found
    }

    /// Whether every part of `exercise` comes from the learned card it claims.
    fn provenance(&self, exercise: &Exercise) -> Vec<String> {
        let Some(&target) = self.learned.get(&exercise.card_id()) else {
            return vec![format!("target card {} is not learned", exercise.card_id())];
        };
        let mut found = Vec::new();
        match exercise {
            Exercise::MultipleChoice(e) => {
                if e.prompt != prompt_side(target, e.direction) {
                    found.push(format!("prompt {:?} is not the target's", e.prompt));
                }
                found.extend(self.option_provenance(
                    e.card_id,
                    &e.options,
                    e.correct_index,
                    |word| answer_side(word, e.direction),
                ));
            }
            Exercise::ListeningChoice(e) => {
                found.extend(self.option_provenance(
                    e.card_id,
                    &e.options,
                    e.correct_index,
                    |word| word.front,
                ));
                if e.audio.text != target.front || e.audio.lang != target.pair.0 {
                    found.push(format!("audio {:?} is not the target's front", e.audio));
                }
                let expected = target.audio.then_some(e.card_id);
                if e.audio.audio_card_id != expected {
                    found.push(format!(
                        "audio card {:?}, expected {expected:?}",
                        e.audio.audio_card_id
                    ));
                }
            }
            Exercise::SentenceBuilder(e) => {
                found.extend(self.sentence_provenance(
                    e.sentence_card_id,
                    &e.sentence,
                    &e.tiles,
                    &e.solution,
                ));
                if let Some(word) = self.learned.get(&e.sentence_card_id) {
                    if e.prompt != word.example.1 {
                        found.push(format!(
                            "prompt {:?} is not card {}'s translation",
                            e.prompt, e.sentence_card_id
                        ));
                    }
                }
            }
            Exercise::ListeningSentence(e) => {
                found.extend(self.sentence_provenance(
                    e.sentence_card_id,
                    &e.sentence,
                    &e.tiles,
                    &e.solution,
                ));
                if e.audio.text != e.sentence || e.audio.audio_card_id.is_some() {
                    found.push(format!("audio {:?} is not the sentence", e.audio));
                }
            }
        }
        found
    }

    fn option_provenance(
        &self,
        target_id: i64,
        options: &[ChoiceOption],
        correct: usize,
        side: impl Fn(&Word) -> &str,
    ) -> Vec<String> {
        let mut found = Vec::new();
        if options.get(correct).map(|option| option.card_id) != Some(target_id) {
            found.push(format!("option {correct} is not the target card"));
        }
        let distinct: HashSet<&str> = options.iter().map(|option| option.text.as_str()).collect();
        if distinct.len() != options.len() {
            found.push(format!("options repeat a text: {options:?}"));
        }
        for option in options {
            match self.learned.get(&option.card_id) {
                None => found.push(format!(
                    "option {:?} comes from card {}, which is not learned",
                    option.text, option.card_id
                )),
                Some(&word) if side(word) != option.text => found.push(format!(
                    "option {:?} is not what learned card {} says ({:?})",
                    option.text,
                    option.card_id,
                    side(word)
                )),
                Some(_) => {}
            }
        }
        found
    }

    fn sentence_provenance(
        &self,
        card_id: i64,
        sentence: &str,
        tiles: &[Tile],
        solution: &[u32],
    ) -> Vec<String> {
        let mut found = Vec::new();
        match self.learned.get(&card_id) {
            None => found.push(format!(
                "sentence {sentence:?} comes from card {card_id}, which is not learned"
            )),
            Some(word) if word.example.0 != sentence => found.push(format!(
                "sentence {sentence:?} is not learned card {card_id}'s example {:?}",
                word.example.0
            )),
            Some(_) => {}
        }
        for tile in tiles {
            let in_solution = solution.contains(&tile.id);
            if in_solution && !sentence.contains(tile.text.as_str()) {
                found.push(format!("tile {:?} is not part of the sentence", tile.text));
            }
            if !in_solution && !self.learned_fronts.contains(tile.text.as_str()) {
                found.push(format!(
                    "distractor tile {:?} is not a learned front",
                    tile.text
                ));
            }
        }
        found
    }
}

/// Which paths of the generators a run went through, so the leak test cannot
/// pass by generating nothing.
#[derive(Default)]
struct Coverage {
    generated: HashMap<ExerciseKind, usize>,
    directions: HashSet<ChoiceDirection>,
    stored_audio: bool,
    synthesized_audio: bool,
    borrowed_sentence: bool,
    distractor_tiles: bool,
    latin_sentence: bool,
    trap_sentence: bool,
    fallback_tiles: bool,
}

impl Coverage {
    fn record(&mut self, exercise: &Exercise, truth: &Truth) {
        *self.generated.entry(exercise.kind()).or_default() += 1;
        match exercise {
            Exercise::MultipleChoice(e) => {
                self.directions.insert(e.direction);
            }
            Exercise::ListeningChoice(e) => match e.audio.audio_card_id {
                Some(_) => self.stored_audio = true,
                None => self.synthesized_audio = true,
            },
            Exercise::SentenceBuilder(e) => self.record_sentence(
                e.card_id,
                e.sentence_card_id,
                &e.sentence,
                &e.tiles,
                &e.solution,
                truth,
            ),
            Exercise::ListeningSentence(e) => self.record_sentence(
                e.card_id,
                e.sentence_card_id,
                &e.sentence,
                &e.tiles,
                &e.solution,
                truth,
            ),
        }
    }

    fn record_sentence(
        &mut self,
        card_id: i64,
        sentence_card_id: i64,
        sentence: &str,
        tiles: &[Tile],
        solution: &[u32],
        truth: &Truth,
    ) {
        self.borrowed_sentence |= sentence_card_id != card_id;
        self.distractor_tiles |= tiles.len() > solution.len();
        self.latin_sentence |= !sentence.chars().any(is_cjk_char);
        self.trap_sentence |= sentence.contains("学生");
        self.fallback_tiles |= tiles
            .iter()
            .any(|tile| truth.unlearned_words.contains(&tile.text));
    }

    fn assert_complete(&self) {
        for kind in ExerciseKind::ALL {
            assert!(
                self.generated.get(&kind).copied().unwrap_or(0) > 100,
                "too few {kind:?} exercises: {:?}",
                self.generated
            );
        }
        assert_eq!(
            self.directions.len(),
            3,
            "directions: {:?}",
            self.directions
        );
        assert!(
            self.stored_audio && self.synthesized_audio,
            "both audio sources"
        );
        assert!(
            self.borrowed_sentence,
            "a sentence borrowed from another entry"
        );
        assert!(self.distractor_tiles, "distractor tiles");
        assert!(self.latin_sentence, "a sentence without hanzi");
        assert!(self.trap_sentence, "the sentence hiding the trap word 学生");
        assert!(self.fallback_tiles, "single-character fallback tiles");
    }
}

#[test]
fn the_fixture_has_far_more_vocabulary_than_the_learner_knows() {
    let fixture = Fixture::new();
    let truth = &fixture.truth;
    assert_eq!(
        fixture.store.curriculum_count().unwrap(),
        CURRICULUM_SIZE as i64
    );
    let distinct: HashSet<&str> = truth
        .curriculum
        .iter()
        .map(|item| item.hanzi.as_str())
        .collect();
    assert_eq!(distinct.len(), CURRICULUM_SIZE);
    assert_eq!(truth.learned.len(), LEARNED.len());
    assert_eq!(truth.not_learned.len(), NOT_LEARNED.len());
    assert!(truth.unlearned_words.len() >= CURRICULUM_SIZE - LEARNED.len());
    for trap in TRAP_WORDS {
        assert!(truth.unlearned_words.contains(trap), "{trap}");
        assert!(
            trap.chars().all(|c| truth.learned_chars.contains(&c)),
            "{trap}"
        );
    }
    let unlearned_hanzi = truth
        .curriculum
        .iter()
        .filter_map(|item| item.example_sentence_source.as_deref())
        .filter(|sentence| {
            sentence
                .chars()
                .any(|c| is_cjk_char(c) && !truth.learned_chars.contains(&c))
        })
        .count();
    assert!(unlearned_hanzi > CURRICULUM_SIZE - 10, "{unlearned_hanzi}");
}

#[test]
fn no_exercise_ever_shows_vocabulary_that_was_not_learned() {
    let fixture = Fixture::new();
    let truth = &fixture.truth;
    let pool = fixture.pool();

    let mut coverage = Coverage::default();
    let mut leaks = Vec::new();
    let mut errors: HashMap<(i64, ExerciseKind), QuizError> = HashMap::new();
    for &card_id in truth.learned.keys() {
        for kind in ExerciseKind::ALL {
            for seed in seeds() {
                match generate_exercise(&pool, card_id, kind, seed) {
                    Ok(exercise) => {
                        coverage.record(&exercise, truth);
                        leaks.extend(truth.violations(&exercise).into_iter().map(|leak| {
                            format!("{kind:?} for card {card_id}, seed {seed}: {leak}")
                        }));
                    }
                    Err(err) => {
                        assert!(
                            !matches!(err, QuizError::NotInPool { .. }),
                            "learned card {card_id} is missing from the pool"
                        );
                        errors.insert((card_id, kind), err);
                    }
                }
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "{} leaks; the first ones:\n{}",
        leaks.len(),
        leaks
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    coverage.assert_complete();

    // Only `book`'s Chinese sides are unshowable, and it has nothing else to
    // be asked about in a choice or with a translated prompt.
    let book = *truth
        .learned
        .iter()
        .find(|(_, word)| word.front == "book")
        .map(|(id, _)| id)
        .unwrap();
    let mut expected = HashMap::new();
    expected.insert(
        (book, ExerciseKind::MultipleChoice),
        QuizError::PoolTooSmall { card_id: book },
    );
    expected.insert(
        (book, ExerciseKind::SentenceBuilder),
        QuizError::NoEligibleSentence { card_id: book },
    );
    assert_eq!(errors, expected);
}

#[test]
fn curriculum_rows_and_unintroduced_captures_never_reach_the_pool() {
    let fixture = Fixture::new();
    let truth = &fixture.truth;
    let pool = fixture.pool();

    let pool_ids: HashSet<i64> = pool.entries().iter().map(|entry| entry.card_id).collect();
    let learned_ids: HashSet<i64> = truth.learned.keys().copied().collect();
    assert_eq!(pool_ids, learned_ids);
    assert_eq!(pool.learned_hanzi_chars(), truth.learned_chars);
    for entry in pool.entries() {
        assert!(
            truth.learned_fronts.contains(entry.front.as_str()),
            "{}",
            entry.front
        );
        assert!(
            !truth.unlearned_words.contains(&entry.front),
            "{}",
            entry.front
        );
    }
    for (&card_id, word) in &truth.not_learned {
        assert!(pool.get(card_id).is_none(), "{}", word.front);
        for kind in ExerciseKind::ALL {
            for seed in [0, 1, 99] {
                assert_eq!(
                    generate_exercise(&pool, card_id, kind, seed),
                    Err(QuizError::NotInPool { card_id }),
                    "{} {kind:?}",
                    word.front
                );
            }
        }
    }

    // An srs_state row is what makes a card learned, and all that does.
    let (&he, _) = truth
        .not_learned
        .iter()
        .find(|(_, word)| word.front == "他")
        .unwrap();
    assert!(!pool.learned_hanzi_chars().contains(&'他'));
    fixture.store.upsert_srs_state(&learned_state(he)).unwrap();
    let pool = fixture.pool();
    assert_eq!(pool.get(he).map(|entry| entry.front.as_str()), Some("他"));
    assert!(pool.learned_hanzi_chars().contains(&'他'));
    assert!(fixture.store.delete_capture(he).unwrap());
    let pool = fixture.pool();
    assert!(pool.get(he).is_none());
    assert!(!pool.learned_hanzi_chars().contains(&'他'));
}

#[test]
fn the_pool_query_reads_srs_state_and_never_the_curriculum() {
    let query = LearnedVocabularyPool::QUERY;
    assert!(query.contains("INNER JOIN srs_state"), "{query}");
    assert!(!query.contains("hsk_curriculum"), "{query}");
}

/// The checker itself must catch leaks, or the test above proves nothing.
#[test]
fn the_leak_checker_flags_deliberately_leaky_exercises() {
    let fixture = Fixture::new();
    let truth = &fixture.truth;
    let pool = fixture.pool();
    let id_of = |front: &str| {
        *truth
            .learned
            .iter()
            .chain(&truth.not_learned)
            .find(|(_, word)| word.front == front)
            .map(|(id, _)| id)
            .unwrap()
    };
    let generate = |front: &str, kind: ExerciseKind| {
        let exercise = generate_exercise(&pool, id_of(front), kind, 7).unwrap();
        assert_eq!(
            truth.violations(&exercise),
            Vec::<String>::new(),
            "untampered {kind:?}"
        );
        exercise
    };
    let flagged = |exercise: &Exercise, mentions: &str| {
        let found = truth.violations(exercise);
        assert!(
            found.iter().any(|problem| problem.contains(mentions)),
            "expected a violation mentioning {mentions:?}, got {found:?}"
        );
    };

    let Exercise::MultipleChoice(choice) = generate("学习", ExerciseKind::MultipleChoice) else {
        unreachable!()
    };
    let distractor = (choice.correct_index + 1) % choice.options.len();

    // A curriculum word as distractor: learned characters, unlearned word.
    let mut leaky = choice.clone();
    leaky.options[distractor].text = "学生".into();
    flagged(&Exercise::MultipleChoice(leaky), "unlearned word \"学生\"");

    // A captured but never introduced card as distractor.
    let mut leaky = choice.clone();
    leaky.options[distractor] = ChoiceOption {
        card_id: id_of("他"),
        text: "er".into(),
    };
    flagged(&Exercise::MultipleChoice(leaky), "which is not learned");

    // A curriculum meaning: no hanzi at all, caught by provenance.
    let mut leaky = choice.clone();
    leaky.options[distractor].text = "Bedeutung 42".into();
    flagged(&Exercise::MultipleChoice(leaky), "is not what learned card");

    // An unlearned hanzi in the prompt.
    let mut leaky = choice;
    leaky.prompt = "书".into();
    flagged(&Exercise::MultipleChoice(leaky), "unlearned hanzi 书");

    let Exercise::SentenceBuilder(builder) = generate("学习", ExerciseKind::SentenceBuilder)
    else {
        unreachable!()
    };
    // The trap word as an extra tile: every character is learned.
    let mut leaky = builder.clone();
    leaky.tiles.push(Tile {
        id: leaky.tiles.len() as u32,
        text: "学生".into(),
    });
    flagged(
        &Exercise::SentenceBuilder(leaky),
        "unlearned word \"学生\" shown as Tile",
    );

    // A tile made of an unlearned character.
    let mut leaky = builder.clone();
    leaky.tiles[0].text = "他".into();
    flagged(&Exercise::SentenceBuilder(leaky), "unlearned hanzi 他");

    // A curriculum sentence.
    let mut leaky = builder;
    leaky.sentence = truth.curriculum[200]
        .example_sentence_source
        .clone()
        .unwrap();
    flagged(&Exercise::SentenceBuilder(leaky), "is not learned card");

    let Exercise::ListeningChoice(listening) = generate("老师", ExerciseKind::ListeningChoice)
    else {
        unreachable!()
    };
    let mut leaky = listening;
    leaky.audio.text = "他很好".into();
    flagged(&Exercise::ListeningChoice(leaky), "unlearned hanzi 他");

    let Exercise::ListeningSentence(sentence) = generate("老师", ExerciseKind::ListeningSentence)
    else {
        unreachable!()
    };
    // The sentence of a captured but never introduced card, all learned hanzi.
    let mut leaky = sentence;
    leaky.sentence_card_id = id_of("你好");
    leaky.sentence = "你好，老师！".into();
    leaky.audio.text = leaky.sentence.clone();
    flagged(&Exercise::ListeningSentence(leaky), "which is not learned");
}
