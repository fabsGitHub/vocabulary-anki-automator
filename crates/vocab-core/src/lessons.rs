//! Babbel-style contextual lessons (Milestone I): short lessons made from the
//! learner's own clipboard captures, each card a word shown in the sentence
//! it was captured with, so the context is something the learner actually
//! read rather than a generic curriculum.
//!
//! - **Only clipboard captures** (`origin = capture`) qualify. Curriculum
//!   words already reach the learner through the daily queue, and Anki
//!   imports are someone else's deck.
//! - **A card needs a sentence pair whose source contains the word**, so the
//!   cloze has something to blank: the word's first occurrence becomes
//!   [`CLOZE_BLANK`]. Hanzi are matched exactly, other scripts ignoring case
//!   (`Serendipity` in "pure serendipity"), since a sentence often starts
//!   with the word.
//! - **Lessons never mix language pairs.** Cards are grouped by
//!   `(source_lang, target_lang)` in capture order and cut into lessons of
//!   `cards_per_lesson`. A lesson's id names its first card
//!   (`lesson-<card id>`), so it stays stable while captures are added and
//!   can key completion state.

use serde::{Deserialize, Serialize};

use crate::{
    model::{CaptureOrigin, StoredCapture},
    quiz::segment::find_word,
    store::{Store, StoreResult},
};

/// What replaces the word in a cloze sentence.
pub const CLOZE_BLANK: &str = "____";

/// One word in context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonCard {
    /// The capture's id.
    pub card_id: i64,
    /// The captured expression (`front`).
    pub word: String,
    /// Its translation (`back`); empty when none was saved.
    pub translation: String,
    /// The example sentence (`example_source`).
    pub sentence_source: String,
    /// Its translation (`example_target`).
    pub sentence_target: String,
    /// `sentence_source` with the word's first occurrence replaced by
    /// [`CLOZE_BLANK`].
    pub cloze: String,
    /// The pinyin of a Chinese card.
    pub pinyin: Option<String>,
}

/// A run of cards of one language pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lesson {
    /// `lesson-<card id of the first card>`.
    pub id: String,
    /// E.g. `EN → DE · Lesson 2`, numbered per language pair.
    pub title: String,
    pub source_lang: String,
    pub target_lang: String,
    pub cards: Vec<LessonCard>,
}

/// The lessons that `captures` make, ordered by the first capture of each
/// language pair and then by lesson number. `captures` may come in any
/// order; cards follow `captured_at`, then id. A `cards_per_lesson` of 0 is
/// treated as 1.
pub fn build_lessons(captures: &[StoredCapture], cards_per_lesson: usize) -> Vec<Lesson> {
    let mut usable: Vec<&StoredCapture> = captures
        .iter()
        .filter(|capture| capture.origin == CaptureOrigin::Capture)
        .collect();
    usable.sort_by(|a, b| {
        a.card
            .captured_at
            .cmp(&b.card.captured_at)
            .then(a.id.cmp(&b.id))
    });

    let mut pairs: Vec<((&str, &str), Vec<LessonCard>)> = Vec::new();
    for capture in usable {
        let Some(card) = lesson_card(capture) else {
            continue;
        };
        let pair = (
            capture.card.source_lang.as_str(),
            capture.card.target_lang.as_str(),
        );
        match pairs.iter_mut().find(|(known, _)| *known == pair) {
            Some((_, cards)) => cards.push(card),
            None => pairs.push((pair, vec![card])),
        }
    }

    let size = cards_per_lesson.max(1);
    pairs
        .into_iter()
        .flat_map(|((source_lang, target_lang), cards)| {
            cards
                .chunks(size)
                .enumerate()
                .map(|(index, chunk)| Lesson {
                    id: format!("lesson-{}", chunk[0].card_id),
                    title: format!(
                        "{} → {} · Lesson {}",
                        source_lang.to_uppercase(),
                        target_lang.to_uppercase(),
                        index + 1
                    ),
                    source_lang: source_lang.to_owned(),
                    target_lang: target_lang.to_owned(),
                    cards: chunk.to_vec(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// [`build_lessons`] over every clipboard capture in the store.
pub fn lessons_from_store(store: &Store, cards_per_lesson: usize) -> StoreResult<Vec<Lesson>> {
    let captures = store.list_captures(Some(CaptureOrigin::Capture))?;
    Ok(build_lessons(&captures, cards_per_lesson))
}

/// The lesson card of a capture whose example sentence contains its front
/// and has a translation.
fn lesson_card(capture: &StoredCapture) -> Option<LessonCard> {
    let card = &capture.card;
    let word = card.front.trim();
    let sentence = card.example_source.trim();
    let sentence_target = card.example_target.trim();
    if sentence_target.is_empty() {
        return None;
    }
    let found = find_word(sentence, word)?;
    Some(LessonCard {
        card_id: capture.id,
        word: word.to_owned(),
        translation: card.back.trim().to_owned(),
        sentence_source: sentence.to_owned(),
        sentence_target: sentence_target.to_owned(),
        cloze: format!(
            "{}{CLOZE_BLANK}{}",
            &sentence[..found.start],
            &sentence[found.end..]
        ),
        pinyin: card
            .pinyin()
            .map(str::trim)
            .filter(|pinyin| !pinyin.is_empty())
            .map(str::to_owned),
    })
}
