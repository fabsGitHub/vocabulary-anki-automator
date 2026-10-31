//! Anki portability (Milestone I, decision 6): Learn's cards out to Anki and
//! an Anki deck into Learn, over AnkiConnect through `anki-bridge`, with the
//! endpoint, deck, note type and field mapping from Learn's settings.
//!
//! **Export** pushes every unsynced card Learn owns, with
//! `anki_bridge::push_card` (the sequence both apps share) and a reverse
//! note whose audio is synthesized the way Automator does it. Clipboard
//! captures (`origin = capture`) are left to Automator's own retry
//! (`sync_pending_captures`, docs/architecture.md § 8.2): only Automator knows
//! the deck and note type each of them was approved for, and Learn's Chinese
//! note type would be wrong for most of them. A card is marked synced once
//! its push succeeds, duplicates included; a card whose notes Anki all had
//! already counts as skipped. The first "Anki unreachable" ends the export,
//! because every later card would wait for the same timeout.
//!
//! **Import** reads the notes of a deck back into cards with
//! `anki_bridge::card_from_note` and stores the new ones as
//! `origin = anki_import`, already synced (they came from Anki). Decisions:
//!
//! - *Learn imports Chinese vocabulary*, because that is what it schedules: a
//!   note with hanzi on neither side is reported as not imported. Imported
//!   cards are `zh-CN` → the settings' meaning language.
//! - *A reverse note is read the right way round.* When only the back holds
//!   hanzi, the note is the reverse one `map_card` writes (meaning -> hanzi),
//!   so its sides are swapped back. It then has the same front as its forward
//!   note, and a deck with both imports each word once.
//! - *Fronts already in the store are skipped*, whatever their origin or
//!   script, so a re-import, or the import of a deck Automator filled, adds
//!   nothing twice; the same holds within one deck.
//! - *Imported hanzi are annotated* like Automator captures: pinyin and HSK
//!   level from the bundled curriculum, generated pinyin outside it, keeping
//!   whatever the note carried. Their `script_kind` is then `chinese`, so
//!   the daily queue adopts an imported word when its curriculum turn comes
//!   (§ 7.1) instead of inserting it again.
//! - Notes are read and annotated before the store is touched; the inserts
//!   then run in one transaction, with a savepoint per note so that one bad
//!   note does not undo the others.
//!
//! The store is never locked across an AnkiConnect or TTS request (§ 6.3).

use std::sync::Arc;

use anki_bridge::{
    card_from_note, deck_query, push_card, AnkiClient, AnkiTarget, NoteInfo, PushOptions,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use vocab_core::{
    curriculum::CurriculumIndex,
    language::contains_cjk,
    providers::{PinyinAnnotator, ScriptAnnotator, TtsProvider},
    store::{Store, StoreResult},
    time::Clock,
    CaptureOrigin, Card, ScriptMetadata, ScriptVariant, StoredCapture,
};

use crate::{audio, error::Error, settings::LearnSettings, state::AppState};

/// The language of every imported front.
const CHINESE: &str = "zh-CN";

/// What `anki_export` and `anki_import` return (`AnkiSyncReport` in
/// apps/learn/src/platform/types.ts).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnkiSyncReport {
    /// Cards pushed (export) or notes read (import).
    pub attempted: usize,
    pub succeeded: usize,
    /// Notes Anki already had (export) or fronts already captured (import).
    pub skipped: usize,
    pub failed: Vec<AnkiSyncFailure>,
}

/// A card that did not make it, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnkiSyncFailure {
    /// The card's id; `None` for an import, which stores nothing it fails.
    pub card_id: Option<i64>,
    pub front: String,
    pub error: String,
}

/// Pushes every unsynced card Learn created to Anki (see the module docs).
pub async fn export(state: &AppState) -> Result<AnkiSyncReport, Error> {
    let _exclusive = state.begin_anki_sync().ok_or(Error::AnkiBusy)?;
    let settings = state.settings();
    let anki = &settings.anki;
    let cards: Vec<StoredCapture> = state
        .with_store(|store| store.list_unsynced_captures())?
        .into_iter()
        .filter(|capture| capture.origin != CaptureOrigin::Capture)
        .collect();
    let client = AnkiClient::new(anki.connect_url.clone());
    let target = AnkiTarget::new(anki.deck_name.clone(), anki.model_name.clone());
    let mut report = AnkiSyncReport::default();
    for capture in cards {
        report.attempted += 1;
        let options = push_options(state.tts(), &settings, &capture.card).await;
        let failure = |error: String| AnkiSyncFailure {
            card_id: Some(capture.id),
            front: capture.card.front.clone(),
            error,
        };
        match push_card(
            &client,
            &capture.card,
            &target,
            &anki.field_mapping,
            &options,
        )
        .await
        {
            Ok(outcome) => {
                match state.with_store(|store| store.set_synced_to_anki(capture.id, true)) {
                    Ok(()) if outcome.note_ids.is_empty() => report.skipped += 1,
                    Ok(()) => report.succeeded += 1,
                    Err(err) => report.failed.push(failure(err.to_string())),
                }
            }
            Err(err) => {
                let unreachable = err.is_unreachable();
                report.failed.push(failure(err.to_string()));
                if unreachable {
                    break;
                }
            }
        }
    }
    log::info!(
        "Anki export: {} card(s) attempted, {} added, {} already there, {} failed.",
        report.attempted,
        report.succeeded,
        report.skipped,
        report.failed.len()
    );
    Ok(report)
}

/// The reverse note, as Automator sends it: on when configured, with TTS of
/// the back in the card's target language. A blank back gets no reverse note
/// anyway, so no audio is made for it.
async fn push_options(tts: &dyn TtsProvider, settings: &LearnSettings, card: &Card) -> PushOptions {
    let create_reverse = settings.anki.create_reverse;
    let reverse_audio = if create_reverse && !card.back.trim().is_empty() {
        audio::synthesize(tts, &card.back, &card.target_lang).await
    } else {
        None
    };
    PushOptions {
        create_reverse,
        reverse_audio,
    }
}

/// Imports the notes of `deck` (see the module docs). Fails only when Anki
/// cannot be asked for the deck at all; problems with single notes are in
/// the report.
pub async fn import(state: &AppState, deck: &str) -> Result<AnkiSyncReport, Error> {
    let deck = deck.trim();
    if deck.is_empty() {
        return Err(Error::NoDeck);
    }
    state.require_store()?;
    let _exclusive = state.begin_anki_sync().ok_or(Error::AnkiBusy)?;
    let settings = state.settings();
    let client = AnkiClient::new(settings.anki.connect_url.clone());
    let note_ids = client.find_notes(&deck_query(deck)).await?;
    let notes = client.notes_info(&note_ids).await?;

    let mut report = AnkiSyncReport {
        attempted: notes.len(),
        ..AnkiSyncReport::default()
    };
    let captured_at = state.clock().now();
    let annotator = Annotator::new(state.curriculum());
    let mut cards = Vec::with_capacity(notes.len());
    for note in &notes {
        match imported_card(note, &settings, captured_at, &annotator).await {
            Ok(card) => cards.push(card),
            Err(failure) => report.failed.push(failure),
        }
    }

    let outcomes = state.with_store(|store| store_imports(store, &cards))?;
    for (card, outcome) in cards.iter().zip(outcomes) {
        match outcome {
            Ok(true) => report.succeeded += 1,
            Ok(false) => report.skipped += 1,
            Err(error) => report.failed.push(AnkiSyncFailure {
                card_id: None,
                front: card.front.clone(),
                error,
            }),
        }
    }
    log::info!(
        "Anki import from {deck:?}: {} note(s) read, {} imported, {} already captured, {} \
         failed.",
        report.attempted,
        report.succeeded,
        report.skipped,
        report.failed.len()
    );
    Ok(report)
}

/// The card a note becomes, or why it is not imported.
async fn imported_card(
    note: &NoteInfo,
    settings: &LearnSettings,
    captured_at: DateTime<Utc>,
    annotator: &Annotator,
) -> Result<Card, AnkiSyncFailure> {
    let mut card = card_from_note(
        note,
        &settings.anki.field_mapping,
        CHINESE,
        settings.meaning_lang.as_str(),
        captured_at,
    )
    .ok_or_else(|| AnkiSyncFailure {
        card_id: None,
        front: String::new(),
        error: format!(
            "Anki note {} has no word or no meaning in the fields the mapping reads.",
            note.note_id
        ),
    })?;
    if !contains_cjk(&card.front) {
        if !contains_cjk(&card.back) {
            return Err(AnkiSyncFailure {
                card_id: None,
                front: card.front,
                error: "No hanzi on either side: Learn imports Chinese vocabulary.".to_string(),
            });
        }
        std::mem::swap(&mut card.front, &mut card.back);
        std::mem::swap(&mut card.example_source, &mut card.example_target);
    }
    annotator.annotate(&mut card).await;
    Ok(card)
}

/// Inserts every card whose front is new, in one transaction; per card,
/// whether it was inserted (`false`: the front is already captured) or why
/// not.
fn store_imports(store: &Store, cards: &[Card]) -> StoreResult<Vec<Result<bool, String>>> {
    store.with_transaction(|store| {
        Ok(cards
            .iter()
            .map(|card| {
                store
                    .with_transaction(|store| {
                        if front_is_captured(store, card)? {
                            return Ok(false);
                        }
                        store.insert_capture(card, CaptureOrigin::AnkiImport, true)?;
                        Ok(true)
                    })
                    .map_err(|err| format!("Could not store the card: {err}"))
            })
            .collect())
    })
}

/// Whether any capture already has `card`'s front, with `card`'s script or
/// none (a Chinese capture whose annotation failed is stored without one).
fn front_is_captured(store: &Store, card: &Card) -> StoreResult<bool> {
    let kinds = [card.script_metadata.kind(), ScriptMetadata::None.kind()];
    for kind in kinds {
        if store.find_capture_by_front(&card.front, kind)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Pinyin and HSK level for imported hanzi; see the module docs.
struct Annotator(PinyinAnnotator);

impl Annotator {
    fn new(curriculum: Arc<CurriculumIndex>) -> Self {
        Self(PinyinAnnotator::new(curriculum))
    }

    /// Gives `card` Chinese script metadata, filling in the pinyin or level
    /// the note did not carry and keeping what it did.
    async fn annotate(&self, card: &mut Card) {
        let (pinyin, hsk_level) = match &card.script_metadata {
            ScriptMetadata::Chinese {
                pinyin, hsk_level, ..
            } => (pinyin.trim().to_string(), *hsk_level),
            ScriptMetadata::None => (String::new(), None),
        };
        let (known_pinyin, known_level) = if pinyin.is_empty() || hsk_level.is_none() {
            match self.0.annotate(&card.front).await {
                Ok(ScriptMetadata::Chinese {
                    pinyin, hsk_level, ..
                }) => (pinyin, hsk_level),
                Ok(ScriptMetadata::None) | Err(_) => (String::new(), None),
            }
        } else {
            (String::new(), None)
        };
        card.script_metadata = ScriptMetadata::Chinese {
            pinyin: if pinyin.is_empty() {
                known_pinyin
            } else {
                pinyin
            },
            hsk_level: hsk_level.or(known_level),
            script_variant: ScriptVariant::Simplified,
        };
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use anki_bridge::{map_card, Direction, FieldMapping, NoteField};
    use chrono::TimeZone;
    use serde_json::json;
    use vocab_core::CurriculumItem;

    use super::*;

    fn captured_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap()
    }

    fn curriculum() -> Arc<CurriculumIndex> {
        Arc::new(CurriculumIndex::new([CurriculumItem {
            hanzi: "银行".into(),
            traditional: None,
            hsk_level: 3,
            frequency: None,
            curriculum_order: 1,
            pinyin: "yín háng".into(),
            pos: None,
            meaning_en: Some("bank".into()),
            meaning_de: None,
            cloze_sentence: None,
            example_sentence_source: None,
            example_sentence_pinyin: None,
            example_sentence_target: None,
            vocab_audio_path: None,
            sentence_audio_path: None,
            image_path: None,
        }]))
    }

    /// The note Anki would store for `card` written in `direction`.
    fn note(card: &Card, direction: Direction, mapping: &FieldMapping) -> NoteInfo {
        let spec = map_card(
            card,
            direction,
            &AnkiTarget::new("Deck", "Model"),
            mapping,
            None,
        );
        NoteInfo {
            note_id: 1,
            model_name: "Model".into(),
            tags: spec.tags,
            fields: spec
                .fields
                .into_iter()
                .enumerate()
                .map(|(order, (name, value))| {
                    (
                        name,
                        NoteField {
                            value,
                            order: order as u32,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            cards: vec![],
            modified: None,
        }
    }

    fn settings(mapping: FieldMapping) -> LearnSettings {
        let mut settings = LearnSettings::default();
        settings.anki.field_mapping = mapping;
        settings
    }

    fn chinese_card() -> Card {
        let mut card = Card::new("银行", "bank", CHINESE, "en", captured_at());
        card.example_source = "我在银行工作。".into();
        card.example_target = "I work at a bank.".into();
        card
    }

    #[test]
    fn a_reverse_note_imports_as_its_forward_card() {
        let mapping = FieldMapping::apple_style_default();
        let reverse = note(&chinese_card(), Direction::Reverse, &mapping);

        let card = tauri::async_runtime::block_on(imported_card(
            &reverse,
            &settings(mapping),
            captured_at(),
            &Annotator::new(curriculum()),
        ))
        .unwrap();

        assert_eq!((card.front.as_str(), card.back.as_str()), ("银行", "bank"));
        assert_eq!(card.example_source, "我在银行工作。");
        assert_eq!(card.example_target, "I work at a bank.");
        assert_eq!(
            (card.source_lang.as_str(), card.target_lang.as_str()),
            ("zh-CN", "en")
        );
        assert_eq!(
            card.script_metadata,
            ScriptMetadata::Chinese {
                pinyin: "yín háng".into(),
                hsk_level: Some(3),
                script_variant: ScriptVariant::Simplified,
            },
            "annotated from the curriculum"
        );
    }

    #[test]
    fn annotation_keeps_what_the_note_carried() {
        let mapping = FieldMapping::chinese_default();
        let mut card = chinese_card();
        card.script_metadata = ScriptMetadata::Chinese {
            pinyin: "yin2 hang2".into(),
            hsk_level: None,
            script_variant: ScriptVariant::Simplified,
        };
        let forward = note(&card, Direction::Forward, &mapping);

        let imported = tauri::async_runtime::block_on(imported_card(
            &forward,
            &settings(mapping),
            captured_at(),
            &Annotator::new(curriculum()),
        ))
        .unwrap();

        assert_eq!(
            imported.script_metadata,
            ScriptMetadata::Chinese {
                pinyin: "yin2 hang2".into(),
                hsk_level: Some(3),
                script_variant: ScriptVariant::Simplified,
            }
        );
    }

    #[test]
    fn notes_without_hanzi_or_without_a_meaning_are_not_imported() {
        let mapping = FieldMapping::apple_style_default();
        let english = Card::new("house", "Haus", "en", "de", captured_at());
        let blank = Card::new("银行", "", CHINESE, "en", captured_at());
        let annotator = Annotator::new(curriculum());

        let failures: Vec<AnkiSyncFailure> = [english, blank]
            .iter()
            .map(|card| {
                tauri::async_runtime::block_on(imported_card(
                    &note(card, Direction::Forward, &mapping),
                    &settings(mapping.clone()),
                    captured_at(),
                    &annotator,
                ))
                .unwrap_err()
            })
            .collect();

        assert_eq!(
            serde_json::to_value(&failures).unwrap(),
            json!([
                {
                    "card_id": null,
                    "front": "house",
                    "error": "No hanzi on either side: Learn imports Chinese vocabulary."
                },
                {
                    "card_id": null,
                    "front": "",
                    "error": "Anki note 1 has no word or no meaning in the fields the mapping reads."
                }
            ])
        );
    }

    #[test]
    fn the_report_has_the_contract_shape() {
        let report = AnkiSyncReport {
            attempted: 3,
            succeeded: 1,
            skipped: 1,
            failed: vec![AnkiSyncFailure {
                card_id: Some(4),
                front: "银行".into(),
                error: "x".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "attempted": 3,
                "succeeded": 1,
                "skipped": 1,
                "failed": [{ "card_id": 4, "front": "银行", "error": "x" }]
            })
        );
    }
}
