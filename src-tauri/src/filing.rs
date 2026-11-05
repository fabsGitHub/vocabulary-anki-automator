//! The field mapping a [`Filing`] pushes with (docs/architecture.md § 8.1):
//! the saved one, or, where none was saved (a learned language without one,
//! or a global mapping never customized), the note type's fields matched by
//! name (`FieldMapping::auto_match`). And the approval window's check of what
//! that mapping would drop ([`check_fields`]).
//!
//! The match is made whenever a card is pushed and never written back, so a
//! note type whose fields changed is matched afresh. It needs the fields from
//! Anki first, one `modelFieldNames` request; a saved mapping needs none and
//! sends exactly `createDeck` and the notes.
//!
//! AnkiConnect drops a value whose field the note type lacks without an
//! error, so the check is the only place the user learns about it before the
//! card is in Anki. It reads from Anki and writes nothing, for every config,
//! and runs as its own command: a slow Anki never delays showing a capture.

use std::collections::HashMap;

use anki_bridge::{AnkiClient, AnkiError, FieldMapping, LostValue};
use serde::{Deserialize, Serialize};
use vocab_core::Card;

use crate::config::{Filing, MappingChoice};

/// What `check_pending_fields` found out about the pending card's note type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FieldCheck {
    /// Anki could not be reached (closed, or too slow to answer).
    Unchecked,
    /// Anki answered with an error, e.g. the note type does not exist.
    Failed { model: String, error: String },
    /// The note type's fields were read; `lost` lists what a push would drop
    /// (empty: everything arrives).
    Checked {
        model: String,
        /// The mapping was matched by field name, not saved.
        auto_matched: bool,
        lost: Vec<LostValue>,
    },
}

/// The mapping a push of `card` uses: the saved one, or the auto-match of
/// the note type's fields for the card's language pair.
pub async fn resolve_mapping(
    client: &AnkiClient,
    filing: &Filing,
    card: &Card,
) -> Result<FieldMapping, AnkiError> {
    resolve_mapping_with(client, filing, card, &mut HashMap::new()).await
}

/// [`resolve_mapping`] for many cards in a row: `known` keeps each note
/// type's fields, so a sync run asks Anki once per note type. The match
/// itself is made per card, since the language pair may differ.
pub async fn resolve_mapping_with(
    client: &AnkiClient,
    filing: &Filing,
    card: &Card,
    known: &mut HashMap<String, Vec<String>>,
) -> Result<FieldMapping, AnkiError> {
    if let MappingChoice::Saved(mapping) = &filing.mapping {
        return Ok(mapping.clone());
    }
    if !known.contains_key(&filing.model) {
        let fields = client.model_field_names(&filing.model).await?;
        known.insert(filing.model.clone(), fields);
    }
    Ok(matched(&known[&filing.model], card))
}

/// Reads the fields of `filing`'s note type once and compares the mapping a
/// push of `card` would use with them ([`FieldMapping::lost_values`]).
pub async fn check_fields(client: &AnkiClient, filing: &Filing, card: &Card) -> FieldCheck {
    let model = filing.model.clone();
    let fields = match client.model_field_names(&filing.model).await {
        Ok(fields) => fields,
        Err(err) if err.is_unreachable() => return FieldCheck::Unchecked,
        Err(err) => {
            return FieldCheck::Failed {
                model,
                error: err.message().to_string(),
            }
        }
    };
    let (mapping, auto_matched) = match &filing.mapping {
        MappingChoice::Saved(mapping) => (mapping.clone(), false),
        MappingChoice::Auto => (matched(&fields, card), true),
    };
    FieldCheck::Checked {
        model,
        auto_matched,
        lost: mapping.lost_values(card, &fields),
    }
}

/// The auto-match of a note type with `fields` for `card`'s language pair.
fn matched(fields: &[String], card: &Card) -> FieldMapping {
    FieldMapping::auto_match(fields, &card.source_lang, &card.target_lang).mapping
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    /// Nothing listens on port 1, so any request would fail.
    const UNREACHABLE: &str = "http://127.0.0.1:1";

    fn card() -> Card {
        Card::new(
            "serendipity",
            "glücklicher Zufall",
            "en",
            "de",
            Utc.timestamp_millis_opt(0).unwrap(),
        )
    }

    fn filing(mapping: MappingChoice) -> Filing {
        Filing {
            deck: "english".into(),
            model: "AppleStyleFinalV3".into(),
            mapping,
            create_reverse: true,
        }
    }

    #[test]
    fn a_saved_mapping_is_used_without_asking_anki() {
        let client = AnkiClient::new(UNREACHABLE);
        let saved = filing(MappingChoice::Saved(FieldMapping::chinese_default()));

        let mapping = tauri::async_runtime::block_on(resolve_mapping(&client, &saved, &card()));

        assert_eq!(mapping, Ok(FieldMapping::chinese_default()));
    }

    #[test]
    fn known_fields_are_matched_without_asking_anki_again() {
        let client = AnkiClient::new(UNREACHABLE);
        let fields = [
            "Front",
            "Back",
            "English Sentence",
            "German Sentence",
            "Audio",
        ]
        .map(String::from)
        .to_vec();
        let mut known = HashMap::from([("AppleStyleFinalV3".to_string(), fields.clone())]);

        let mapping = tauri::async_runtime::block_on(resolve_mapping_with(
            &client,
            &filing(MappingChoice::Auto),
            &card(),
            &mut known,
        ));

        assert_eq!(
            mapping,
            Ok(FieldMapping::auto_match(&fields, "en", "de").mapping)
        );
    }

    #[test]
    fn an_unreachable_anki_leaves_the_fields_unchecked() {
        let client = AnkiClient::new(UNREACHABLE);
        let saved = filing(MappingChoice::Saved(FieldMapping::default()));

        let check = tauri::async_runtime::block_on(check_fields(&client, &saved, &card()));

        assert_eq!(check, FieldCheck::Unchecked);
    }

    #[test]
    fn field_checks_have_the_contract_shape() {
        let checks = [
            FieldCheck::Unchecked,
            FieldCheck::Failed {
                model: "Nope".into(),
                error: "model was not found: Nope".into(),
            },
            FieldCheck::Checked {
                model: "AppleStyleFinalV3".into(),
                auto_matched: false,
                lost: vec![LostValue::Audio {
                    audio_field: Some("Front Audio".into()),
                }],
            },
        ];
        assert_eq!(
            serde_json::to_value(&checks).unwrap(),
            serde_json::json!([
                { "status": "unchecked" },
                { "status": "failed", "model": "Nope", "error": "model was not found: Nope" },
                {
                    "status": "checked",
                    "model": "AppleStyleFinalV3",
                    "auto_matched": false,
                    "lost": [{ "kind": "audio", "audio_field": "Front Audio" }]
                }
            ])
        );
    }

    #[test]
    fn an_unreachable_anki_is_the_error_of_an_auto_mapping() {
        let client = AnkiClient::new(UNREACHABLE);

        let err = tauri::async_runtime::block_on(resolve_mapping(
            &client,
            &filing(MappingChoice::Auto),
            &card(),
        ))
        .unwrap_err();

        assert!(err.is_unreachable(), "{err:?}");
    }
}
