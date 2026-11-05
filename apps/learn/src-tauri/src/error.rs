//! The one error type every command returns.
//!
//! It crosses the IPC edge as a plain sentence (docs/architecture.md § 2, "a
//! typed error that serializes"), so the frontend can show it as it is: the
//! TypeScript contract says every method rejects with a human-readable string.
//! The variants that name a precondition (no such card, not scheduled yet,
//! not in the curriculum, ...) exist so that the common mistakes read as
//! what they are instead of as a database failure.

use anki_bridge::AnkiError;
use serde::{Serialize, Serializer};
use vocab_core::{quiz::QuizError, store::StoreError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The shared database could not be opened at startup.
    #[error("The vocabulary database is unavailable ({0}).")]
    StoreUnavailable(String),
    #[error("The vocabulary database failed ({0}).")]
    Store(#[from] StoreError),
    /// `hsk_curriculum` could not be filled with the bundled curriculum.
    #[error("The HSK curriculum is unavailable ({0}).")]
    CurriculumUnavailable(String),
    #[error("There is no card {0}.")]
    NoSuchCard(i64),
    /// The card has no `srs_state` row: it was never introduced.
    #[error("Card {0} is not scheduled for review.")]
    NotScheduled(i64),
    /// Exercise results are only recorded for learned cards.
    #[error("Card {0} is not learned vocabulary.")]
    NotLearned(i64),
    #[error("\"{0}\" is not in the HSK curriculum.")]
    NotInCurriculum(String),
    #[error("There is no lesson \"{0}\".")]
    NoSuchLesson(String),
    #[error("No exercise: {0}.")]
    Quiz(#[from] QuizError),
    /// Every problem the settings validation found, one sentence each.
    #[error("{}", .0.join("\n"))]
    InvalidSettings(Vec<String>),
    #[error("{0}")]
    Anki(#[from] AnkiError),
    #[error("An Anki export or import is already running.")]
    AnkiBusy,
    #[error("Choose the Anki deck to import.")]
    NoDeck,
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_serialize_as_their_sentence() {
        let cases = [
            (Error::NoSuchCard(7), "There is no card 7."),
            (
                Error::StoreUnavailable("could not open /x: disk full".into()),
                "The vocabulary database is unavailable (could not open /x: disk full).",
            ),
            (
                Error::Quiz(QuizError::NotInPool { card_id: 3 }),
                "No exercise: card 3 is not learned vocabulary.",
            ),
            (
                Error::InvalidSettings(vec!["First.".into(), "Second.".into()]),
                "First.\nSecond.",
            ),
            (
                Error::Anki(AnkiError::Unreachable {
                    message: "connection refused".into(),
                }),
                "could not reach AnkiConnect: connection refused",
            ),
        ];
        for (error, sentence) in cases {
            assert_eq!(serde_json::to_value(&error).unwrap(), sentence);
        }
    }
}
