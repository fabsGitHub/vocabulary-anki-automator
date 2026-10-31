//! Quiz engine (Milestone M): Duolingo-style exercises built from the
//! learner's own learned vocabulary and nothing else.
//!
//! **Isolation (M1)** is the one correctness property the spec requires: an
//! exercise never shows a word the learner has not learned. It is kept in
//! one place rather than in each exercise type:
//!
//! - All content comes from a [`LearnedVocabularyPool`], which only
//!   [`LearnedVocabularyPool::load`] creates, with one query: captures that
//!   have an `srs_state` row. `hsk_curriculum` is never read, not even as the
//!   dictionary that cuts Chinese sentences into tiles.
//! - [`generate_exercise`] hands each generator the pool, the target entry
//!   and a seeded RNG, nothing else.
//! - Every hanzi an exercise shows is in
//!   [`LearnedVocabularyPool::learned_hanzi_chars`].
//!
//! `tests/quiz_isolation.rs` (Milestone G+'s leak test) generates every
//! exercise type for every learned card over hundreds of seeds and checks
//! each shown string against an independently built list of learned words.
//!
//! **Determinism.** An exercise is a pure function of the pool, the card, the
//! kind and the seed; the only randomness is a [`Mulberry32`] seeded with
//! `seed`. The Learn web build re-implements the generators in TypeScript,
//! and produces the same exercises only if it draws the random numbers in
//! exactly this order (`shuffle` is [`Mulberry32::shuffle`], `pick(n)` is
//! [`Mulberry32::below`]; candidate lists start in pool order, i.e. by card
//! id; texts are compared ignoring case and runs of whitespace, pinyin
//! ignoring all whitespace). `tests/quiz_exercises.rs` pins parity vectors.
//!
//! - *Multiple choice:* `pick(askable.len())` among the directions
//!   `front_to_back`, `back_to_front`, `front_to_pinyin` that have a prompt,
//!   an answer and at least one distractor candidate; `shuffle(candidates)`,
//!   then a stable sort by preference (same language pair, same script, the
//!   rest); the first three texts that differ from the answer and from each
//!   other are the distractors; `shuffle(distractors)`;
//!   `correct_index = pick(distractors.len() + 1)`, where the answer is
//!   inserted.
//! - *Listening choice:* the same from `shuffle(candidates)` on, over fronts.
//! - *Sentence builder / listening sentence:* the target's own sentence when
//!   it qualifies, else `pick(others.len())` among the other qualifying
//!   sentences; `shuffle(distractor candidates)`, of which the first two are
//!   kept; `shuffle(sentence tiles ++ distractor tiles)`; tile ids are the
//!   resulting positions.

mod exercises;
mod pool;
pub(crate) mod segment;

use serde::{Deserialize, Serialize};

pub use exercises::{
    AudioPrompt, ChoiceDirection, ChoiceOption, ListeningChoiceExercise, ListeningSentenceExercise,
    MultipleChoiceExercise, SentenceBuilderExercise, Tile,
};
pub use pool::{LearnedVocabularyPool, PoolEntry};

use crate::rng::Mulberry32;

/// The exercise types of Milestone M2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExerciseKind {
    MultipleChoice,
    SentenceBuilder,
    ListeningChoice,
    ListeningSentence,
}

impl ExerciseKind {
    pub const ALL: [ExerciseKind; 4] = [
        ExerciseKind::MultipleChoice,
        ExerciseKind::SentenceBuilder,
        ExerciseKind::ListeningChoice,
        ExerciseKind::ListeningSentence,
    ];

    /// The serialized name (`"listening_sentence"`, ...).
    pub fn as_str(&self) -> &'static str {
        match self {
            ExerciseKind::MultipleChoice => "multiple_choice",
            ExerciseKind::SentenceBuilder => "sentence_builder",
            ExerciseKind::ListeningChoice => "listening_choice",
            ExerciseKind::ListeningSentence => "listening_sentence",
        }
    }
}

/// A generated exercise, solution included: the frontend checks answers
/// itself and reports the result (`record_exercise_result`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Exercise {
    MultipleChoice(MultipleChoiceExercise),
    SentenceBuilder(SentenceBuilderExercise),
    ListeningChoice(ListeningChoiceExercise),
    ListeningSentence(ListeningSentenceExercise),
}

impl Exercise {
    pub fn kind(&self) -> ExerciseKind {
        match self {
            Exercise::MultipleChoice(_) => ExerciseKind::MultipleChoice,
            Exercise::SentenceBuilder(_) => ExerciseKind::SentenceBuilder,
            Exercise::ListeningChoice(_) => ExerciseKind::ListeningChoice,
            Exercise::ListeningSentence(_) => ExerciseKind::ListeningSentence,
        }
    }

    /// The card being practised.
    pub fn card_id(&self) -> i64 {
        match self {
            Exercise::MultipleChoice(exercise) => exercise.card_id,
            Exercise::SentenceBuilder(exercise) => exercise.card_id,
            Exercise::ListeningChoice(exercise) => exercise.card_id,
            Exercise::ListeningSentence(exercise) => exercise.card_id,
        }
    }
}

/// The learner's answer to an exercise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExerciseAnswer {
    /// The index of the chosen option (choice exercises).
    Choice { index: usize },
    /// The ids of the placed tiles, in order (sentence exercises).
    Order { tile_ids: Vec<u32> },
}

/// Why no exercise of the requested kind exists for a card.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuizError {
    /// The card is unknown or has no `srs_state` row, so it is not learned
    /// vocabulary and must not be quizzed.
    #[error("card {card_id} is not learned vocabulary")]
    NotInPool { card_id: i64 },
    /// No way to ask about the card yields two options: the learned
    /// vocabulary has no other entry to serve as a distractor, or the card
    /// lacks the side to ask for (a blank translation, no pinyin).
    #[error("not enough learned vocabulary for a choice exercise about card {card_id}")]
    PoolTooSmall { card_id: i64 },
    /// No learned example sentence uses the card's front with learned hanzi
    /// only.
    #[error("no learned example sentence uses card {card_id}")]
    NoEligibleSentence { card_id: i64 },
}

/// Builds an exercise of `kind` about the learned card `card_id` from `pool`
/// alone. The same arguments always give the same exercise.
pub fn generate_exercise(
    pool: &LearnedVocabularyPool,
    card_id: i64,
    kind: ExerciseKind,
    seed: u32,
) -> Result<Exercise, QuizError> {
    let target = pool.get(card_id).ok_or(QuizError::NotInPool { card_id })?;
    let mut rng = Mulberry32::new(seed);
    match kind {
        ExerciseKind::MultipleChoice => {
            exercises::multiple_choice(pool, target, &mut rng).map(Exercise::MultipleChoice)
        }
        ExerciseKind::SentenceBuilder => {
            exercises::sentence_builder(pool, target, &mut rng).map(Exercise::SentenceBuilder)
        }
        ExerciseKind::ListeningChoice => {
            exercises::listening_choice(pool, target, &mut rng).map(Exercise::ListeningChoice)
        }
        ExerciseKind::ListeningSentence => {
            exercises::listening_sentence(pool, target, &mut rng).map(Exercise::ListeningSentence)
        }
    }
}

/// Whether `answer` solves `exercise`. An answer of the wrong shape (an
/// order for a choice exercise) is wrong, as is an order that uses a tile
/// twice or a tile the exercise does not have.
pub fn check_answer(exercise: &Exercise, answer: &ExerciseAnswer) -> bool {
    match (exercise, answer) {
        (Exercise::MultipleChoice(exercise), ExerciseAnswer::Choice { index }) => {
            exercises::choice_is_correct(&exercise.options, exercise.correct_index, *index)
        }
        (Exercise::ListeningChoice(exercise), ExerciseAnswer::Choice { index }) => {
            exercises::choice_is_correct(&exercise.options, exercise.correct_index, *index)
        }
        (Exercise::SentenceBuilder(exercise), ExerciseAnswer::Order { tile_ids }) => {
            exercises::order_is_correct(&exercise.tiles, &exercise.solution, tile_ids)
        }
        (Exercise::ListeningSentence(exercise), ExerciseAnswer::Order { tile_ids }) => {
            exercises::order_is_correct(&exercise.tiles, &exercise.solution, tile_ids)
        }
        _ => false,
    }
}
