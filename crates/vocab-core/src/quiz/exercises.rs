//! The four exercise types of Milestone M2 and their generators.
//!
//! A generator receives the pool, the target entry (taken from that pool)
//! and the seeded RNG, and nothing else. Two rules sit on top of "taken from
//! the pool":
//!
//! - **Every shown text contains learned hanzi only.** Fronts pass by
//!   definition, but the translation of a card whose target language is
//!   Chinese (`book` -> `书`) can hold characters no learned front has. Such a
//!   text is never shown: not as a prompt, an option or a sentence prompt.
//! - **No second correct answer.** A distractor never shares the prompt (a
//!   word captured twice with two translations) or, for listening, the
//!   pinyin (`是`/`事`), and options compare case- and spacing-insensitively,
//!   so two options never read the same.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    pool::{LearnedVocabularyPool, PoolEntry},
    segment::{self, Dictionary},
    QuizError,
};
use crate::{
    language::{contains_cjk, is_cjk_char},
    rng::Mulberry32,
};

/// Distractors next to the correct option of a choice exercise, at most.
const CHOICE_DISTRACTORS: usize = 3;
/// Extra tiles that do not belong to the sentence, at most.
const DISTRACTOR_TILES: usize = 2;
/// A sentence needs this many tiles to be worth ordering.
const MIN_SENTENCE_TILES: usize = 2;
/// `captures.script_kind` of Chinese cards.
const CHINESE: &str = "chinese";

/// Which side of a card a multiple-choice question shows and which it asks
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceDirection {
    /// Shows the front, asks for the translation.
    FrontToBack,
    /// Shows the translation, asks for the front.
    BackToFront,
    /// Shows a Chinese front, asks for its pinyin.
    FrontToPinyin,
}

impl ChoiceDirection {
    /// The order in which a generator considers the directions.
    const ALL: [Self; 3] = [Self::FrontToBack, Self::BackToFront, Self::FrontToPinyin];

    fn prompt(self, entry: &PoolEntry) -> &str {
        match self {
            Self::FrontToBack | Self::FrontToPinyin => &entry.front,
            Self::BackToFront => &entry.back,
        }
    }

    fn answer(self, entry: &PoolEntry) -> &str {
        match self {
            Self::FrontToBack => &entry.back,
            Self::BackToFront => &entry.front,
            Self::FrontToPinyin if entry.script_kind == CHINESE => {
                entry.pinyin.as_deref().unwrap_or_default()
            }
            Self::FrontToPinyin => "",
        }
    }

    fn answer_key(self, text: &str) -> String {
        match self {
            Self::FrontToPinyin => pinyin_key(text),
            Self::FrontToBack | Self::BackToFront => text_key(text),
        }
    }

    fn prompt_lang(self, entry: &PoolEntry) -> &str {
        match self {
            Self::FrontToBack | Self::FrontToPinyin => &entry.source_lang,
            Self::BackToFront => &entry.target_lang,
        }
    }

    fn option_lang(self, entry: &PoolEntry) -> &str {
        match self {
            Self::FrontToBack => &entry.target_lang,
            Self::BackToFront | Self::FrontToPinyin => &entry.source_lang,
        }
    }
}

/// One answer option of a choice exercise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// The pool entry the text was taken from; the target's id for the
    /// correct option.
    pub card_id: i64,
    pub text: String,
}

/// Pick the matching translation, front or pinyin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultipleChoiceExercise {
    /// The card being practised.
    pub card_id: i64,
    pub direction: ChoiceDirection,
    pub prompt: String,
    /// Language of `prompt`.
    pub prompt_lang: String,
    /// Language of the options; pinyin counts as the card's source language.
    pub option_lang: String,
    /// The correct option and one to three distractors, shuffled.
    pub options: Vec<ChoiceOption>,
    pub correct_index: usize,
}

/// What a listening exercise plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPrompt {
    /// The card whose stored `audio_front` to play (`get_card_audio`);
    /// `None` when there is no stored recording and `text` is synthesized.
    pub audio_card_id: Option<i64>,
    /// What is spoken.
    pub text: String,
    /// The language to synthesize `text` in.
    pub lang: String,
}

/// Hear the target word, pick it among learned fronts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListeningChoiceExercise {
    pub card_id: i64,
    pub audio: AudioPrompt,
    /// Fronts: the target's and one to three distractors, shuffled.
    pub options: Vec<ChoiceOption>,
    pub correct_index: usize,
}

/// A word tile. `id` is the tile's position in its exercise's `tiles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    pub id: u32,
    pub text: String,
}

/// Rebuild a learned example sentence from shuffled tiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentenceBuilderExercise {
    pub card_id: i64,
    /// The pool entry whose example sentence is used: `card_id` itself when
    /// its own sentence qualifies, else another entry whose sentence uses the
    /// target's front.
    pub sentence_card_id: i64,
    /// The sentence's translation (that entry's `example_target`); empty when
    /// the capture has none.
    pub prompt: String,
    /// The sentence as captured, punctuation included.
    pub sentence: String,
    pub sentence_lang: String,
    /// The sentence's tiles plus up to two distractor tiles, shuffled.
    pub tiles: Vec<Tile>,
    /// Tile ids in sentence order.
    pub solution: Vec<u32>,
}

/// Hear a learned example sentence, rebuild it from shuffled tiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListeningSentenceExercise {
    pub card_id: i64,
    /// As in [`SentenceBuilderExercise::sentence_card_id`].
    pub sentence_card_id: i64,
    /// Always synthesized: captures store audio of the word, not of the
    /// sentence.
    pub audio: AudioPrompt,
    /// The sentence as captured, shown once answered.
    pub sentence: String,
    pub tiles: Vec<Tile>,
    pub solution: Vec<u32>,
}

pub(super) fn multiple_choice(
    pool: &LearnedVocabularyPool,
    target: &PoolEntry,
    rng: &mut Mulberry32,
) -> Result<MultipleChoiceExercise, QuizError> {
    let askable: Vec<(ChoiceDirection, Vec<&PoolEntry>)> = ChoiceDirection::ALL
        .into_iter()
        .filter_map(|direction| {
            let candidates = choice_candidates(pool, target, direction)?;
            (!candidates.is_empty()).then_some((direction, candidates))
        })
        .collect();
    if askable.is_empty() {
        return Err(QuizError::PoolTooSmall {
            card_id: target.card_id,
        });
    }
    let (direction, candidates) = &askable[pick(rng, askable.len())];
    let direction = *direction;
    let (options, correct_index) = choose_options(
        target,
        direction.answer(target).trim(),
        candidates.clone(),
        |entry| direction.answer(entry),
        |text| direction.answer_key(text),
        rng,
    );
    Ok(MultipleChoiceExercise {
        card_id: target.card_id,
        direction,
        prompt: direction.prompt(target).trim().to_owned(),
        prompt_lang: direction.prompt_lang(target).to_owned(),
        option_lang: direction.option_lang(target).to_owned(),
        options,
        correct_index,
    })
}

/// The entries that can be wrong answers to `target` asked in `direction`,
/// in pool order; `None` when the target itself cannot be asked that way
/// (a blank or not-shown prompt or answer, or no pinyin).
fn choice_candidates<'a>(
    pool: &'a LearnedVocabularyPool,
    target: &PoolEntry,
    direction: ChoiceDirection,
) -> Option<Vec<&'a PoolEntry>> {
    let hanzi = pool.hanzi();
    let prompt_key = text_key(shown(direction.prompt(target), hanzi)?);
    let answer_key = direction.answer_key(shown(direction.answer(target), hanzi)?);
    Some(
        pool.entries()
            .iter()
            .filter(|entry| entry.card_id != target.card_id)
            .filter(|entry| text_key(direction.prompt(entry)) != prompt_key)
            .filter(|entry| {
                shown(direction.answer(entry), hanzi)
                    .is_some_and(|answer| direction.answer_key(answer) != answer_key)
            })
            .collect(),
    )
}

pub(super) fn listening_choice(
    pool: &LearnedVocabularyPool,
    target: &PoolEntry,
    rng: &mut Mulberry32,
) -> Result<ListeningChoiceExercise, QuizError> {
    let hanzi = pool.hanzi();
    let too_small = QuizError::PoolTooSmall {
        card_id: target.card_id,
    };
    let Some(front) = shown(&target.front, hanzi) else {
        return Err(too_small);
    };
    let front_key = text_key(front);
    let sound = target.pinyin.as_deref().map(pinyin_key);
    let candidates: Vec<&PoolEntry> = pool
        .entries()
        .iter()
        .filter(|entry| entry.card_id != target.card_id)
        .filter(|entry| shown(&entry.front, hanzi).is_some_and(|text| text_key(text) != front_key))
        .filter(|entry| sound.is_none() || entry.pinyin.as_deref().map(pinyin_key) != sound)
        .collect();
    if candidates.is_empty() {
        return Err(too_small);
    }
    let (options, correct_index) = choose_options(
        target,
        front,
        candidates,
        |entry| &entry.front,
        text_key,
        rng,
    );
    Ok(ListeningChoiceExercise {
        card_id: target.card_id,
        audio: AudioPrompt {
            audio_card_id: target.has_audio.then_some(target.card_id),
            text: front.to_owned(),
            lang: target.source_lang.clone(),
        },
        options,
        correct_index,
    })
}

/// The correct option plus up to [`CHOICE_DISTRACTORS`] distractors with
/// distinct texts, in random order, and the index of the correct one.
///
/// Distractors are drawn uniformly, but from the same language pair first,
/// then the same script, then the rest: the candidates are shuffled and then
/// stably sorted by that preference.
fn choose_options<'a>(
    target: &PoolEntry,
    correct: &str,
    mut candidates: Vec<&'a PoolEntry>,
    text_of: impl Fn(&'a PoolEntry) -> &'a str,
    key_of: impl Fn(&str) -> String,
    rng: &mut Mulberry32,
) -> (Vec<ChoiceOption>, usize) {
    rng.shuffle(&mut candidates);
    candidates.sort_by_key(|entry| preference(target, entry));
    let mut seen = HashSet::from([key_of(correct)]);
    let mut options: Vec<ChoiceOption> = candidates
        .into_iter()
        .filter_map(|entry| {
            let text = text_of(entry).trim();
            seen.insert(key_of(text)).then(|| ChoiceOption {
                card_id: entry.card_id,
                text: text.to_owned(),
            })
        })
        .take(CHOICE_DISTRACTORS)
        .collect();
    rng.shuffle(&mut options);
    let correct_index = pick(rng, options.len() + 1);
    options.insert(
        correct_index,
        ChoiceOption {
            card_id: target.card_id,
            text: correct.to_owned(),
        },
    );
    (options, correct_index)
}

/// 0 for the target's language pair, 1 for its script, 2 for anything else.
fn preference(target: &PoolEntry, entry: &PoolEntry) -> u8 {
    if entry.source_lang == target.source_lang && entry.target_lang == target.target_lang {
        0
    } else if entry.script_kind == target.script_kind {
        1
    } else {
        2
    }
}

pub(super) fn sentence_builder(
    pool: &LearnedVocabularyPool,
    target: &PoolEntry,
    rng: &mut Mulberry32,
) -> Result<SentenceBuilderExercise, QuizError> {
    let dictionary = Dictionary::from_pool(pool);
    let source = pick_sentence(pool, target, true, &dictionary, rng)?;
    let entry = source.entry;
    let sentence = source.sentence.to_owned();
    let (tiles, solution) = build_tiles(pool, source, &dictionary, rng);
    Ok(SentenceBuilderExercise {
        card_id: target.card_id,
        sentence_card_id: entry.card_id,
        prompt: entry.example_target.trim().to_owned(),
        sentence,
        sentence_lang: entry.source_lang.clone(),
        tiles,
        solution,
    })
}

pub(super) fn listening_sentence(
    pool: &LearnedVocabularyPool,
    target: &PoolEntry,
    rng: &mut Mulberry32,
) -> Result<ListeningSentenceExercise, QuizError> {
    let dictionary = Dictionary::from_pool(pool);
    let source = pick_sentence(pool, target, false, &dictionary, rng)?;
    let entry = source.entry;
    let sentence = source.sentence.to_owned();
    let (tiles, solution) = build_tiles(pool, source, &dictionary, rng);
    Ok(ListeningSentenceExercise {
        card_id: target.card_id,
        sentence_card_id: entry.card_id,
        audio: AudioPrompt {
            audio_card_id: None,
            text: sentence.clone(),
            lang: entry.source_lang.clone(),
        },
        sentence,
        tiles,
        solution,
    })
}

/// A pool entry's example sentence that a sentence exercise can use, already
/// cut into tiles.
struct SentenceSource<'a> {
    entry: &'a PoolEntry,
    /// The trimmed `example_source`.
    sentence: &'a str,
    tiles: Vec<String>,
}

/// The target's own sentence when it qualifies, else one picked at random
/// from the other entries' qualifying sentences.
fn pick_sentence<'a>(
    pool: &'a LearnedVocabularyPool,
    target: &'a PoolEntry,
    needs_translation: bool,
    dictionary: &Dictionary,
    rng: &mut Mulberry32,
) -> Result<SentenceSource<'a>, QuizError> {
    let usable = |entry| usable_sentence(pool, target, entry, needs_translation, dictionary);
    if let Some(own) = usable(target) {
        return Ok(own);
    }
    let mut others: Vec<SentenceSource<'a>> = pool
        .entries()
        .iter()
        .filter(|entry| entry.card_id != target.card_id)
        .filter_map(usable)
        .collect();
    if others.is_empty() {
        return Err(QuizError::NoEligibleSentence {
            card_id: target.card_id,
        });
    }
    let index = pick(rng, others.len());
    Ok(others.swap_remove(index))
}

/// `entry`'s example sentence, if it contains the target's front, shows
/// learned hanzi only (and so does its translation, when the exercise
/// displays it) and has at least [`MIN_SENTENCE_TILES`] tiles.
fn usable_sentence<'a>(
    pool: &LearnedVocabularyPool,
    target: &PoolEntry,
    entry: &'a PoolEntry,
    needs_translation: bool,
    dictionary: &Dictionary,
) -> Option<SentenceSource<'a>> {
    let hanzi = pool.hanzi();
    let sentence = shown(&entry.example_source, hanzi)?;
    segment::find_word(sentence, &target.front)?;
    if needs_translation && !learned_only(&entry.example_target, hanzi) {
        return None;
    }
    let tiles = segment::tiles(sentence, dictionary);
    (tiles.len() >= MIN_SENTENCE_TILES).then_some(SentenceSource {
        entry,
        sentence,
        tiles,
    })
}

/// The sentence's tiles plus distractor tiles, shuffled, with ids, and the
/// tile ids in sentence order.
fn build_tiles(
    pool: &LearnedVocabularyPool,
    source: SentenceSource<'_>,
    dictionary: &Dictionary,
    rng: &mut Mulberry32,
) -> (Vec<Tile>, Vec<u32>) {
    let distractors = distractor_tiles(pool, &source, dictionary, rng);
    let mut solution = vec![0; source.tiles.len()];
    let mut pieces: Vec<(Option<usize>, String)> = source
        .tiles
        .into_iter()
        .enumerate()
        .map(|(position, text)| (Some(position), text))
        .chain(distractors.into_iter().map(|text| (None, text)))
        .collect();
    rng.shuffle(&mut pieces);
    let tiles = pieces
        .into_iter()
        .enumerate()
        .map(|(index, (position, text))| {
            let id = index as u32;
            if let Some(position) = position {
                solution[position] = id;
            }
            Tile { id, text }
        })
        .collect();
    (tiles, solution)
}

/// Up to [`DISTRACTOR_TILES`] fronts of the sentence's language that could be
/// a tile of it but do not occur in it, so no arrangement that uses one can
/// spell the sentence.
fn distractor_tiles(
    pool: &LearnedVocabularyPool,
    source: &SentenceSource<'_>,
    dictionary: &Dictionary,
    rng: &mut Mulberry32,
) -> Vec<String> {
    let chinese = contains_cjk(source.sentence);
    let joined = source.tiles.concat();
    let mut seen = HashSet::new();
    let mut candidates: Vec<String> = pool
        .entries()
        .iter()
        .filter(|entry| entry.source_lang == source.entry.source_lang)
        .filter_map(|entry| front_as_tile(&entry.front, chinese, dictionary))
        .filter(|text| {
            if chinese {
                !joined.contains(text.as_str())
            } else {
                segment::find_word(source.sentence, text).is_none()
            }
        })
        .filter(|text| seen.insert(text_key(text)))
        .collect();
    rng.shuffle(&mut candidates);
    candidates.truncate(DISTRACTOR_TILES);
    candidates
}

/// A front as a tile of a Chinese sentence (a dictionary word) or of another
/// sentence (a single word without hanzi).
fn front_as_tile(front: &str, chinese: bool, dictionary: &Dictionary) -> Option<String> {
    let front = front.trim();
    if chinese {
        return dictionary.contains(front).then(|| front.to_owned());
    }
    if contains_cjk(front) || front.contains(char::is_whitespace) {
        return None;
    }
    segment::word_tile(front)
}

pub(super) fn choice_is_correct(
    options: &[ChoiceOption],
    correct_index: usize,
    index: usize,
) -> bool {
    index == correct_index && index < options.len()
}

/// Whether `given` spells the solution: as many tiles, none used twice, and
/// the same texts in the same order (tiles with equal texts are
/// interchangeable).
pub(super) fn order_is_correct(tiles: &[Tile], solution: &[u32], given: &[u32]) -> bool {
    let text = |id: u32| {
        tiles
            .iter()
            .find(|tile| tile.id == id)
            .map(|tile| &tile.text)
    };
    let mut used = HashSet::new();
    given.len() == solution.len()
        && given.iter().zip(solution).all(|(&given, &expected)| {
            used.insert(given) && text(given).is_some() && text(given) == text(expected)
        })
}

/// `text` trimmed, when it is not blank and every hanzi in it is learned.
fn shown<'a>(text: &'a str, hanzi: &HashSet<char>) -> Option<&'a str> {
    let text = text.trim();
    (!text.is_empty() && learned_only(text, hanzi)).then_some(text)
}

fn learned_only(text: &str, hanzi: &HashSet<char>) -> bool {
    text.chars().all(|c| !is_cjk_char(c) || hanzi.contains(&c))
}

/// Comparison key of an option or prompt: case and runs of whitespace do
/// not make two texts different.
fn text_key(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Comparison key of pinyin: the dataset writes `xuéxí`, the generator
/// `xué xí`; both are the same sound.
fn pinyin_key(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// A uniform index below `len`.
fn pick(rng: &mut Mulberry32, len: usize) -> usize {
    rng.below(u32::try_from(len).unwrap_or(u32::MAX)) as usize
}
