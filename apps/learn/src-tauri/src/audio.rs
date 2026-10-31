//! Audio for the review and listening screens (Milestone M2): a card's
//! stored recording of its front, or text-to-speech.
//!
//! TTS is a network request, so the store is never locked while it runs
//! (docs/architecture.md § 6.3). A card whose front had to be synthesized
//! gets the recording written back afterwards, in a short transaction of its
//! own, so it is synthesized only once. That write is skipped when the card
//! changed in the meantime (Automator edited the front, the card was deleted,
//! or it got a recording another way), and a write that fails only means the
//! next request synthesizes again; neither fails the request.
//!
//! Missing audio is not an error: a TTS failure is logged and the result is
//! "no audio" (`null` for the frontend), which the screens handle anyway.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use vocab_core::{language::normalize_lang, providers::TtsProvider, store::Store, StoredCapture};

use crate::{error::Error, state::AppState};

/// `bytes` as the frontend receives audio: base64, like `Card.audio_front`.
pub fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// The recording of card `card_id`'s front: the stored one, else TTS of the
/// front in the card's own language, then kept with the card.
pub async fn card_audio(state: &AppState, card_id: i64) -> Result<Option<Vec<u8>>, Error> {
    let capture = state.with_store(|store| {
        store
            .get_capture(card_id)?
            .ok_or(Error::NoSuchCard(card_id))
    })?;
    let card = &capture.card;
    if let Some(stored) = card.audio_front.as_ref().filter(|audio| !audio.is_empty()) {
        return Ok(Some(stored.clone()));
    }
    let Some(audio) = synthesize(state.tts(), &card.front, &card.source_lang).await else {
        return Ok(None);
    };
    match state.with_store(|store| keep_recording(store, &capture, &audio)) {
        Ok(true) => log::debug!("Kept the synthesized audio of card {card_id}."),
        Ok(false) => log::debug!("Card {card_id} changed while its audio was synthesized."),
        Err(err) => log::warn!("Could not keep the audio of card {card_id}: {err}"),
    }
    Ok(Some(audio))
}

/// `text` spoken in `lang`, or `None` when there is nothing to say or TTS
/// fails.
pub async fn synthesize(tts: &dyn TtsProvider, text: &str, lang: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    match tts.synthesize(text, &normalize_lang(lang)).await {
        Ok(audio) if !audio.is_empty() => Some(audio),
        Ok(_) => {
            log::warn!("Text-to-speech returned no audio.");
            None
        }
        Err(err) => {
            log::warn!("Text-to-speech failed: {err}");
            None
        }
    }
}

/// Stores `audio` as the front recording of `read`'s card, if the card is
/// still the one that was read: same front, still without a recording.
/// Returns whether it wrote.
fn keep_recording(store: &Store, read: &StoredCapture, audio: &[u8]) -> Result<bool, Error> {
    Ok(store.with_transaction(|store| {
        let Some(mut current) = store.get_capture(read.id)? else {
            return Ok(false);
        };
        let unchanged = current.card.front == read.card.front
            && current
                .card
                .audio_front
                .as_ref()
                .is_none_or(|stored| stored.is_empty());
        if !unchanged {
            return Ok(false);
        }
        current.card.audio_front = Some(audio.to_vec());
        store.update_capture_card(read.id, &current.card)?;
        Ok(true)
    })?)
}
