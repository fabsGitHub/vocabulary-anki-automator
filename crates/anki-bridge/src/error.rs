//! Typed AnkiConnect failures.
//!
//! The pre-refactor client flattened every failure into a `String`, so the app
//! could not tell "Anki is closed" (keep the card, sync later) from "Anki
//! already has this note" (nothing left to do) from "AnkiConnect rejected the
//! request" (a configuration problem the user has to fix). Those are exactly
//! the distinctions callers act on, so they are separate variants. The
//! `{ "kind": ... }` serde shape is part of the contract with both frontends
//! (docs/architecture.md § 9).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnkiError {
    /// No HTTP response: Anki is not running, AnkiConnect is not installed,
    /// the URL is wrong, or the request timed out.
    #[error("could not reach AnkiConnect: {message}")]
    Unreachable { message: String },
    /// AnkiConnect answered with an error, e.g. an unknown deck or note type,
    /// an empty first field, or a collection that is not open.
    #[error("AnkiConnect error: {message}")]
    Api { message: String },
    /// Anki refused the note because the deck already contains an identical
    /// one (`addNote` is always sent with `allowDuplicate: false`).
    #[error("duplicate note: {message}")]
    Duplicate { message: String },
    /// Something answered, but not with an AnkiConnect response of the
    /// expected shape (another service on the port, an incompatible version).
    #[error("unexpected response from AnkiConnect: {message}")]
    Malformed { message: String },
}

impl AnkiError {
    /// Classifies an error string reported by AnkiConnect. Its only duplicate
    /// error is "cannot create note because it is a duplicate"; matching the
    /// word case-insensitively keeps forks and future wording covered.
    pub(crate) fn from_api_message(message: String) -> Self {
        if message.to_lowercase().contains("duplicate") {
            AnkiError::Duplicate { message }
        } else {
            AnkiError::Api { message }
        }
    }

    /// The detail text, without the variant's prefix.
    pub fn message(&self) -> &str {
        match self {
            AnkiError::Unreachable { message }
            | AnkiError::Api { message }
            | AnkiError::Duplicate { message }
            | AnkiError::Malformed { message } => message,
        }
    }

    pub fn is_duplicate(&self) -> bool {
        matches!(self, AnkiError::Duplicate { .. })
    }

    /// True when Anki could not be reached at all, i.e. the request is worth
    /// retrying later unchanged.
    pub fn is_unreachable(&self) -> bool {
        matches!(self, AnkiError::Unreachable { .. })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn serializes_with_kind_tag() {
        let err = AnkiError::Duplicate {
            message: "cannot create note because it is a duplicate".into(),
        };
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({
                "kind": "duplicate",
                "message": "cannot create note because it is a duplicate"
            })
        );
        let back: AnkiError = serde_json::from_value(json!({
            "kind": "unreachable",
            "message": "connection refused"
        }))
        .unwrap();
        assert_eq!(
            back,
            AnkiError::Unreachable {
                message: "connection refused".into()
            }
        );
    }

    #[test]
    fn api_messages_are_classified_by_the_word_duplicate() {
        let cases = [
            ("cannot create note because it is a duplicate", true),
            ("Duplicate note", true),
            ("note is a DUPLICATE of 123", true),
            ("model was not found: AppleStyleFinalV3", false),
            ("cannot create note because it is empty", false),
            ("collection is not available", false),
        ];
        for (message, duplicate) in cases {
            let err = AnkiError::from_api_message(message.to_string());
            assert_eq!(err.is_duplicate(), duplicate, "{message}");
            assert_eq!(err.message(), message);
            if !duplicate {
                assert!(matches!(err, AnkiError::Api { .. }), "{message}");
            }
        }
    }

    #[test]
    fn display_names_the_failure() {
        let err = AnkiError::Unreachable {
            message: "connection refused".into(),
        };
        assert_eq!(
            err.to_string(),
            "could not reach AnkiConnect: connection refused"
        );
        assert!(err.is_unreachable());
        assert!(!AnkiError::Api {
            message: String::new()
        }
        .is_unreachable());
    }
}
