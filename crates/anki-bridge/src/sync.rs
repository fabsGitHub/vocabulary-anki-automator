//! Pushing one card to Anki: the `createDeck` + `addNote` sequence both apps
//! run (Automator's `execute_card_generation` and `sync_pending_captures`,
//! Learn's Anki export), so it is written once.
//!
//! A duplicate is not a failure here. A note Anki already has is exactly the
//! state a push wants to reach, and retries re-send notes that made it in on
//! an earlier attempt (say the forward note did and the reverse one did not),
//! so `Duplicate` is counted instead of returned. Every other error stops the
//! push and is returned unchanged.

use vocab_core::Card;

use crate::{
    client::AnkiClient,
    error::AnkiError,
    mapper::{map_card, AnkiTarget, Direction, FieldMapping, NoteSpec},
};

/// What a push sends besides the forward note. `Default` is forward-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushOptions {
    /// Also add the reverse (target -> source) note.
    pub create_reverse: bool,
    /// Audio for the reverse note's front: TTS of `card.back` in the card's
    /// `target_lang`. Ignored unless `create_reverse` is set.
    pub reverse_audio: Option<Vec<u8>>,
}

/// The result of a push that reached Anki. The card counts as synced whenever
/// a push returns `Ok`, duplicates included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushOutcome {
    /// Ids of the notes Anki created, forward note first.
    pub note_ids: Vec<i64>,
    /// Notes Anki rejected because the deck already has them.
    pub duplicates: usize,
}

/// Creates `target.deck` if needed, then adds the forward note (with
/// `card.audio_front`) and, when `options.create_reverse` is set, the reverse
/// note (with `options.reverse_audio`).
///
/// A `createDeck` error saying the deck already exists is ignored, and a
/// `Duplicate` on either note is counted in [`PushOutcome::duplicates`]. Any
/// other error is returned as soon as it happens, so a failed forward note
/// means no reverse note is attempted. A card whose `back` is blank (the
/// translation failed and nothing was typed in) gets no reverse note: its
/// front field would be empty, Anki would refuse it on every retry, and the
/// capture would never count as synced.
pub async fn push_card(
    client: &AnkiClient,
    card: &Card,
    target: &AnkiTarget,
    mapping: &FieldMapping,
    options: &PushOptions,
) -> Result<PushOutcome, AnkiError> {
    push_to(client, card, target, mapping, options).await
}

/// The two calls a push makes. [`AnkiClient`] is the only real
/// implementation; the seam lets the push sequence be unit-tested against an
/// in-memory fake, without HTTP.
trait NoteSink {
    async fn create_deck(&self, deck: &str) -> Result<i64, AnkiError>;
    async fn add_note(&self, note: &NoteSpec) -> Result<i64, AnkiError>;
}

impl NoteSink for AnkiClient {
    async fn create_deck(&self, deck: &str) -> Result<i64, AnkiError> {
        AnkiClient::create_deck(self, deck).await
    }

    async fn add_note(&self, note: &NoteSpec) -> Result<i64, AnkiError> {
        AnkiClient::add_note(self, note).await
    }
}

async fn push_to(
    sink: &impl NoteSink,
    card: &Card,
    target: &AnkiTarget,
    mapping: &FieldMapping,
    options: &PushOptions,
) -> Result<PushOutcome, AnkiError> {
    match sink.create_deck(&target.deck).await {
        Ok(_) => {}
        Err(err) if deck_already_exists(&err) => {
            log::debug!("createDeck: deck already exists ({err})");
        }
        Err(err) => return Err(err),
    }

    let notes = [
        Some((Direction::Forward, card.audio_front.as_deref())),
        (options.create_reverse && !card.back.trim().is_empty())
            .then_some((Direction::Reverse, options.reverse_audio.as_deref())),
    ];
    let mut outcome = PushOutcome::default();
    for (direction, audio) in notes.into_iter().flatten() {
        let note = map_card(card, direction, target, mapping, audio);
        match sink.add_note(&note).await {
            Ok(id) => outcome.note_ids.push(id),
            Err(AnkiError::Duplicate { message }) => {
                log::info!("Anki already has the {direction:?} note: {message}");
                outcome.duplicates += 1;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(outcome)
}

/// AnkiConnect itself returns the existing id for a known deck, but other
/// implementations of the protocol answer with an "already exists" error.
fn deck_already_exists(err: &AnkiError) -> bool {
    match err {
        AnkiError::Api { message } | AnkiError::Duplicate { message } => {
            message.to_lowercase().contains("already exists")
        }
        AnkiError::Unreachable { .. } | AnkiError::Malformed { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use chrono::{TimeZone, Utc};

    use super::*;

    const FWD_AUDIO: &[u8] = &[0xff, 0xfb, 0x90, 0x00];
    const REV_AUDIO: &[u8] = b"ID3\x03\x00";
    const DUPLICATE: &str = "cannot create note because it is a duplicate";

    /// Records every call and answers from a script.
    #[derive(Default)]
    struct FakeAnki {
        create_deck_error: Option<AnkiError>,
        add_note_results: Mutex<VecDeque<Result<i64, AnkiError>>>,
        decks: Mutex<Vec<String>>,
        notes: Mutex<Vec<NoteSpec>>,
    }

    impl FakeAnki {
        fn answering(add_note_results: Vec<Result<i64, AnkiError>>) -> Self {
            Self {
                add_note_results: Mutex::new(add_note_results.into()),
                ..Self::default()
            }
        }

        fn failing_create_deck(err: AnkiError) -> Self {
            Self {
                create_deck_error: Some(err),
                ..Self::answering(vec![Ok(1), Ok(2)])
            }
        }

        fn decks(&self) -> Vec<String> {
            self.decks.lock().unwrap().clone()
        }

        fn notes(&self) -> Vec<NoteSpec> {
            self.notes.lock().unwrap().clone()
        }
    }

    impl NoteSink for FakeAnki {
        async fn create_deck(&self, deck: &str) -> Result<i64, AnkiError> {
            self.decks.lock().unwrap().push(deck.to_string());
            match &self.create_deck_error {
                Some(err) => Err(err.clone()),
                None => Ok(99),
            }
        }

        async fn add_note(&self, note: &NoteSpec) -> Result<i64, AnkiError> {
            self.notes.lock().unwrap().push(note.clone());
            self.add_note_results
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected addNote call")
        }
    }

    fn card() -> Card {
        let captured_at = Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap();
        let mut card = Card::new("serendipity", "glücklicher Zufall", "en", "de", captured_at);
        card.audio_front = Some(FWD_AUDIO.to_vec());
        card
    }

    fn target() -> AnkiTarget {
        AnkiTarget::new("English::AutomatedVocabulary", "AppleStyleFinalV3")
    }

    fn both_ways() -> PushOptions {
        PushOptions {
            create_reverse: true,
            reverse_audio: Some(REV_AUDIO.to_vec()),
        }
    }

    async fn push(anki: &FakeAnki, options: &PushOptions) -> Result<PushOutcome, AnkiError> {
        push_to(
            anki,
            &card(),
            &target(),
            &FieldMapping::apple_style_default(),
            options,
        )
        .await
    }

    fn api(message: &str) -> AnkiError {
        AnkiError::Api {
            message: message.into(),
        }
    }

    fn duplicate() -> AnkiError {
        AnkiError::from_api_message(DUPLICATE.into())
    }

    #[tokio::test]
    async fn creates_the_deck_then_forward_then_reverse_note() {
        let anki = FakeAnki::answering(vec![Ok(10), Ok(20)]);

        let outcome = push(&anki, &both_ways()).await;

        assert_eq!(
            outcome,
            Ok(PushOutcome {
                note_ids: vec![10, 20],
                duplicates: 0
            })
        );
        assert_eq!(anki.decks(), ["English::AutomatedVocabulary"]);
        let mapping = FieldMapping::apple_style_default();
        let expected = [
            map_card(
                &card(),
                Direction::Forward,
                &target(),
                &mapping,
                Some(FWD_AUDIO),
            ),
            map_card(
                &card(),
                Direction::Reverse,
                &target(),
                &mapping,
                Some(REV_AUDIO),
            ),
        ];
        assert_eq!(anki.notes(), expected);
        assert_eq!(anki.notes()[0].fields["Front"], "serendipity");
        assert_eq!(anki.notes()[1].fields["Front"], "glücklicher Zufall");
    }

    #[tokio::test]
    async fn a_blank_back_gets_no_reverse_note() {
        let anki = FakeAnki::answering(vec![Ok(10)]);
        let mut card = card();
        card.back = "  ".into();

        let outcome = push_to(
            &anki,
            &card,
            &target(),
            &FieldMapping::apple_style_default(),
            &both_ways(),
        )
        .await;

        assert_eq!(
            outcome,
            Ok(PushOutcome {
                note_ids: vec![10],
                duplicates: 0
            })
        );
        assert_eq!(anki.notes().len(), 1);
        assert_eq!(anki.notes()[0].fields["Front"], "serendipity");
    }

    #[tokio::test]
    async fn without_create_reverse_only_the_forward_note_is_added() {
        let anki = FakeAnki::answering(vec![Ok(10)]);
        let options = PushOptions {
            create_reverse: false,
            reverse_audio: Some(REV_AUDIO.to_vec()),
        };

        let outcome = push(&anki, &options).await;

        assert_eq!(
            outcome,
            Ok(PushOutcome {
                note_ids: vec![10],
                duplicates: 0
            })
        );
        let notes = anki.notes();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].fields["Front"], "serendipity");
    }

    #[tokio::test]
    async fn reverse_audio_never_falls_back_to_the_front_audio() {
        let anki = FakeAnki::answering(vec![Ok(10), Ok(20)]);
        let options = PushOptions {
            create_reverse: true,
            reverse_audio: None,
        };

        push(&anki, &options).await.unwrap();

        let notes = anki.notes();
        let forward_audio = notes[0].audio.as_ref().expect("forward note has audio");
        assert_eq!(forward_audio.data_b64, "//uQAA==");
        assert!(forward_audio.filename.ends_with("_fwd.mp3"));
        assert_eq!(
            notes[1].audio, None,
            "card.audio_front must not leak into the reverse note"
        );
    }

    #[tokio::test]
    async fn duplicates_are_counted_not_returned() {
        let cases = [
            (vec![Err(duplicate()), Ok(20)], vec![20], 1),
            (vec![Ok(10), Err(duplicate())], vec![10], 1),
            (vec![Err(duplicate()), Err(duplicate())], vec![], 2),
        ];
        for (results, note_ids, duplicates) in cases {
            let anki = FakeAnki::answering(results);
            let outcome = push(&anki, &both_ways()).await;
            assert_eq!(
                outcome,
                Ok(PushOutcome {
                    note_ids: note_ids.clone(),
                    duplicates
                })
            );
            assert_eq!(anki.notes().len(), 2, "{note_ids:?}");
        }
    }

    #[tokio::test]
    async fn an_existing_deck_is_not_an_error() {
        let anki = FakeAnki::failing_create_deck(api(
            "Deck 'English::AutomatedVocabulary' already exists",
        ));
        let outcome = push(&anki, &both_ways()).await;
        assert_eq!(
            outcome,
            Ok(PushOutcome {
                note_ids: vec![1, 2],
                duplicates: 0
            })
        );
    }

    #[tokio::test]
    async fn other_create_deck_errors_stop_before_any_note() {
        let cases = [
            api("collection is not available"),
            AnkiError::Unreachable {
                message: "connection refused".into(),
            },
        ];
        for err in cases {
            let anki = FakeAnki::failing_create_deck(err.clone());
            assert_eq!(push(&anki, &both_ways()).await, Err(err));
            assert!(anki.notes().is_empty());
        }
    }

    #[tokio::test]
    async fn a_failed_forward_note_skips_the_reverse_note() {
        let err = api("model was not found: AppleStyleFinalV3");
        let anki = FakeAnki::answering(vec![Err(err.clone()), Ok(20)]);
        assert_eq!(push(&anki, &both_ways()).await, Err(err));
        assert_eq!(anki.notes().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_reverse_note_is_returned() {
        let err = api("cannot create note because it is empty");
        let anki = FakeAnki::answering(vec![Ok(10), Err(err.clone())]);
        assert_eq!(push(&anki, &both_ways()).await, Err(err));
        assert_eq!(anki.notes().len(), 2);
    }

    /// Tauri commands and `tauri::async_runtime::spawn` need `Send` futures;
    /// this stops compiling if the `NoteSink` seam ever loses that.
    #[test]
    fn client_futures_are_send() {
        fn assert_send<T: Send>(_: &T) {}
        let client = AnkiClient::new(crate::client::DEFAULT_URL);
        let (card, target, mapping) = (card(), target(), FieldMapping::default());
        let options = both_ways();
        assert_send(&push_card(&client, &card, &target, &mapping, &options));
        assert_send(&client.find_notes("deck:x"));
        assert_send(&client.notes_info(&[1, 2]));
    }

    #[test]
    fn only_already_exists_api_errors_are_tolerated_for_create_deck() {
        let cases = [
            (api("Deck 'Vocab' already exists"), true),
            (api("deck ALREADY EXISTS"), true),
            (
                AnkiError::Duplicate {
                    message: "duplicate: already exists".into(),
                },
                true,
            ),
            (api("collection is not available"), false),
            (
                AnkiError::Unreachable {
                    message: "already exists".into(),
                },
                false,
            ),
            (
                AnkiError::Malformed {
                    message: "already exists".into(),
                },
                false,
            ),
        ];
        for (err, tolerated) in cases {
            assert_eq!(deck_already_exists(&err), tolerated, "{err:?}");
        }
    }
}
