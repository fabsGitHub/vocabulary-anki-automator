//! The field mapping a [`Filing`] pushes with (docs/architecture.md § 8.1):
//! the saved one, or, for a learned language without one, the note type's
//! fields matched by name (`FieldMapping::auto_match`).
//!
//! The match is made whenever a card is pushed and never written back, so a
//! note type whose fields changed is matched afresh. It needs the fields from
//! Anki first, one `modelFieldNames` request; a saved mapping needs none, so a
//! config without learned languages sends exactly what it always did.

use std::collections::HashMap;

use anki_bridge::{AnkiClient, AnkiError, FieldMapping};
use vocab_core::Card;

use crate::config::{Filing, MappingChoice};

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
    let fields = &known[&filing.model];
    Ok(FieldMapping::auto_match(fields, &card.source_lang, &card.target_lang).mapping)
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
