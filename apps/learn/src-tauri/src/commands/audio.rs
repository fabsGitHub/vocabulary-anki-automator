//! `get_card_audio`, `synthesize_audio` (Milestone M2): base64 MP3, or
//! `null` when there is none.

use tauri::State;

use crate::{audio, error::Error, state::AppState};

/// The recording of card `card_id`'s front: stored, else synthesized in the
/// card's language and kept with the card.
#[tauri::command]
pub async fn get_card_audio(
    state: State<'_, AppState>,
    card_id: i64,
) -> Result<Option<String>, Error> {
    Ok(audio::card_audio(&state, card_id)
        .await?
        .map(|bytes| audio::encode(&bytes)))
}

/// `text` spoken in `lang`, for the example sentences of listening
/// exercises, which have no stored recording.
#[tauri::command]
pub async fn synthesize_audio(
    state: State<'_, AppState>,
    text: String,
    lang: String,
) -> Result<Option<String>, Error> {
    Ok(audio::synthesize(state.tts(), &text, &lang)
        .await
        .map(|bytes| audio::encode(&bytes)))
}
