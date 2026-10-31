//! Getting approved cards into Anki: the approval itself
//! (`execute_card_generation`) and the retry of everything Anki missed
//! (`sync_pending_captures`), docs/architecture.md § 8.2.
//!
//! The local store is the source of truth and Anki a destination that may be
//! closed. So a card is saved first, always, and only then pushed; Anki being
//! offline is a normal outcome (`synced_to_anki: false`), not an error, and
//! the retry picks the card up later. A push that Anki answers with
//! "duplicate" counts as synced, because the note it wanted is there.
//!
//! The store is only touched between network calls, never across one
//! (§ 6.3): Learn may be writing to the same file, and a transaction held
//! while AnkiConnect or TTS takes ten seconds would make its writes time out.
//!
//! A card approved into a profile's deck remembers that deck in the store's
//! `app_state` table until it is synced, so a retry after a restart still
//! goes to the right deck instead of the default one. The deck is written in
//! the same transaction as the card, before Anki is asked: a retry that runs
//! while the approval is still talking to Anki, or after the app quit in the
//! middle of it, must already know it.

use anki_bridge::{push_card, AnkiClient, AnkiTarget, PushOptions};
use serde::{Deserialize, Serialize};
use vocab_core::{
    language::normalize_lang,
    providers::ProviderRegistry,
    store::{Store, StoreResult},
    CaptureOrigin, Card, ScriptMetadata,
};

use crate::{capture::PendingCapture, config::AppConfig, state::AppState};

/// What `execute_card_generation` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitResult {
    /// The new `captures` row; `0` when the history is unavailable and the
    /// card went to Anki only (SQLite never hands out id 0).
    pub capture_id: i64,
    pub synced_to_anki: bool,
    /// Notes Anki created (none for duplicates).
    pub note_ids: Vec<i64>,
    /// Why the card is not in Anki yet.
    pub anki_error: Option<String>,
}

/// What `sync_pending_captures` returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncReport {
    /// Captures a push was tried for.
    pub attempted: usize,
    pub synced: usize,
    pub failed: Vec<SyncFailure>,
}

/// A capture that is still not in Anki, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncFailure {
    pub id: i64,
    pub error: String,
}

/// `app_state` key that holds the deck of unsynced capture `id`.
pub fn deck_key(id: i64) -> String {
    format!("automator.unsynced_deck.{id}")
}

/// Saves `card` and pushes it to Anki (§ 8.2). `pending` is the capture the
/// card was approved from; when the user changed its front, TTS (and, for a
/// script annotation they did not edit, the annotator) runs again for the new
/// text. Errors only when the card could not be stored anywhere.
pub async fn submit_card(
    state: &AppState,
    config: &AppConfig,
    card: Card,
    pending: Option<&PendingCapture>,
) -> Result<SubmitResult, String> {
    let mut card = tidy(card)?;
    let front_changed = pending.is_some_and(|pending| pending.card.front != card.front);
    let rederive_script = front_changed
        && pending.is_some_and(|pending| pending.card.script_metadata == card.script_metadata);
    if front_changed {
        // The audio speaks the old text; never store or send it.
        card.audio_front = None;
    } else if card.audio_front.is_none() {
        card.audio_front = pending.and_then(|pending| pending.card.audio_front.clone());
    }
    let profile_id = pending.and_then(|pending| pending.profile_id.as_deref());
    let deck = config
        .profile(profile_id)
        .unwrap_or_else(|| config.default_profile())
        .deck_name;
    let profile_deck = (deck != config.anki.deck_name).then_some(deck.as_str());

    // 1. Save first, always, together with a profile deck.
    let capture_id = if state.has_store() {
        let id = state
            .with_store(|store| {
                store.with_transaction(|store| {
                    let id = store.insert_capture(&card, CaptureOrigin::Capture, false)?;
                    if let Some(deck) = profile_deck {
                        store.set_state(&deck_key(id), deck)?;
                    }
                    Ok(id)
                })
            })
            .map_err(|err| format!("The card was not saved: {err}"))?;
        log::info!("Saved capture {id}.");
        Some(id)
    } else {
        None
    };

    let registry = state.providers().registry(config);
    if front_changed {
        let set = registry.resolve(&card.source_lang);
        if let Some(tts) = &set.tts {
            card.audio_front = match tts.synthesize(&card.front, &card.source_lang).await {
                Ok(audio) if !audio.is_empty() => Some(audio),
                Ok(_) => None,
                Err(err) => {
                    log::warn!("Could not synthesize audio for the edited front: {err}");
                    None
                }
            };
        }
        if rederive_script {
            if let Some(annotator) = &set.annotator {
                card.script_metadata =
                    annotator.annotate(&card.front).await.unwrap_or_else(|err| {
                        log::warn!("Could not annotate the edited front: {err}");
                        ScriptMetadata::None
                    });
            }
        }
        if let Some(id) = capture_id {
            if let Err(err) = state.with_store(|store| store.update_capture_card(id, &card)) {
                log::error!("Could not store the new audio of capture {id}: {err}");
            }
        }
    }

    // 2. + 3. Map with reverse-side audio, push.
    let options = push_options(&registry, config, &card).await;
    let client = AnkiClient::new(config.anki.connect_url.clone());
    let target = AnkiTarget::new(deck.clone(), config.anki.model_name.clone());
    let pushed = push_card(
        &client,
        &card,
        &target,
        &config.anki.field_mapping,
        &options,
    )
    .await;

    // 4. Mark synced, or leave the capture (and its deck) for the retry.
    match (pushed, capture_id) {
        (Ok(outcome), Some(id)) => {
            if let Err(err) = state.with_store(|store| mark_synced(store, id)) {
                log::error!("Capture {id} is in Anki but could not be marked synced: {err}");
            }
            log::info!(
                "Capture {id} is in Anki ({} new note(s), {} duplicate(s)).",
                outcome.note_ids.len(),
                outcome.duplicates
            );
            Ok(SubmitResult {
                capture_id: id,
                synced_to_anki: true,
                note_ids: outcome.note_ids,
                anki_error: None,
            })
        }
        (Ok(outcome), None) => Ok(SubmitResult {
            capture_id: 0,
            synced_to_anki: true,
            note_ids: outcome.note_ids,
            anki_error: None,
        }),
        (Err(err), Some(id)) => {
            log::warn!("Capture {id} is saved but not in Anki yet: {err}");
            Ok(SubmitResult {
                capture_id: id,
                synced_to_anki: false,
                note_ids: Vec::new(),
                anki_error: Some(err.to_string()),
            })
        }
        (Err(err), None) => Err(format!(
            "The card was not saved: the capture history is unavailable and Anki did not take \
             it ({err})."
        )),
    }
}

/// Pushes every unsynced Automator capture, oldest first (§ 8.2). Stops at
/// the first "Anki unreachable": the rest would fail the same way, each after
/// its own timeout. Rows Learn created (curriculum items, imports) are left
/// to Learn's own Anki export.
pub async fn sync_pending(state: &AppState, config: &AppConfig) -> Result<SyncReport, String> {
    let _exclusive = state.sync_lock().lock().await;
    let captures = state.with_store(|store| store.list_unsynced_captures())?;
    let registry = state.providers().registry(config);
    let client = AnkiClient::new(config.anki.connect_url.clone());
    let mut report = SyncReport::default();
    for capture in captures
        .into_iter()
        .filter(|capture| capture.origin == CaptureOrigin::Capture)
    {
        let id = capture.id;
        report.attempted += 1;
        let deck = state
            .with_store(|store| store.get_state(&deck_key(id)))
            .ok()
            .flatten()
            .unwrap_or_else(|| config.anki.deck_name.clone());
        let options = push_options(&registry, config, &capture.card).await;
        let target = AnkiTarget::new(deck, config.anki.model_name.clone());
        match push_card(
            &client,
            &capture.card,
            &target,
            &config.anki.field_mapping,
            &options,
        )
        .await
        {
            Ok(_) => {
                let marked = state.with_store(|store| mark_synced(store, id));
                match marked {
                    Ok(()) => report.synced += 1,
                    Err(error) => report.failed.push(SyncFailure { id, error }),
                }
            }
            Err(err) => {
                let unreachable = err.is_unreachable();
                report.failed.push(SyncFailure {
                    id,
                    error: err.to_string(),
                });
                if unreachable {
                    break;
                }
            }
        }
    }
    log::info!(
        "Anki sync: {} of {} attempted capture(s) synced.",
        report.synced,
        report.attempted
    );
    Ok(report)
}

/// Marks capture `id` synced and forgets its remembered deck, atomically.
fn mark_synced(store: &Store, id: i64) -> StoreResult<()> {
    store.with_transaction(|store| {
        store.set_synced_to_anki(id, true)?;
        store.delete_state(&deck_key(id))
    })
}

/// The reverse note's settings: on when configured, with TTS of the back in
/// the target language. No audio is synthesized for a blank back, which
/// gets no reverse note anyway.
async fn push_options(registry: &ProviderRegistry, config: &AppConfig, card: &Card) -> PushOptions {
    let create_reverse = config.anki.create_reverse;
    let mut reverse_audio = None;
    if create_reverse && !card.back.trim().is_empty() {
        if let Some(tts) = &registry.resolve(&card.target_lang).tts {
            match tts.synthesize(&card.back, &card.target_lang).await {
                Ok(audio) if !audio.is_empty() => reverse_audio = Some(audio),
                Ok(_) => {}
                Err(err) => log::warn!("No audio for the reverse note: {err}"),
            }
        }
    }
    PushOptions {
        create_reverse,
        reverse_audio,
    }
}

/// Trims the text the user edited, canonicalizes the language codes and
/// refuses a card without a front (Anki would refuse the note on every try).
fn tidy(mut card: Card) -> Result<Card, String> {
    card.front = card.front.trim().to_string();
    card.back = card.back.trim().to_string();
    if card.front.is_empty() {
        return Err("The card needs a front.".to_string());
    }
    card.source_lang = normalize_lang(&card.source_lang);
    card.target_lang = normalize_lang(&card.target_lang);
    Ok(card)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    #[test]
    fn tidy_trims_and_refuses_an_empty_front() {
        let card = Card::new(
            " Haus ",
            " house\n",
            "DE",
            "EN",
            Utc.timestamp_millis_opt(0).unwrap(),
        );
        let tidied = tidy(card).unwrap();
        assert_eq!(
            (tidied.front.as_str(), tidied.back.as_str()),
            ("Haus", "house")
        );
        assert_eq!(
            (tidied.source_lang.as_str(), tidied.target_lang.as_str()),
            ("de", "en")
        );

        let empty = Card::new("   ", "x", "de", "en", Utc.timestamp_millis_opt(0).unwrap());
        assert_eq!(tidy(empty).unwrap_err(), "The card needs a front.");
    }

    #[test]
    fn results_have_the_contract_shapes() {
        let result = SubmitResult {
            capture_id: 7,
            synced_to_anki: false,
            note_ids: Vec::new(),
            anki_error: Some("could not reach AnkiConnect: refused".into()),
        };
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({
                "capture_id": 7,
                "synced_to_anki": false,
                "note_ids": [],
                "anki_error": "could not reach AnkiConnect: refused"
            })
        );
        let report = SyncReport {
            attempted: 2,
            synced: 1,
            failed: vec![SyncFailure {
                id: 3,
                error: "x".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({ "attempted": 2, "synced": 1, "failed": [{ "id": 3, "error": "x" }] })
        );
    }

    #[test]
    fn deck_keys_are_namespaced_per_capture() {
        assert_eq!(deck_key(42), "automator.unsynced_deck.42");
    }
}
