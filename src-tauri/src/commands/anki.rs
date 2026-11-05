//! `anki_status`, `anki_model_field_names`, `suggest_field_mapping`,
//! `check_pending_fields`, `create_recommended_note_type`,
//! `launch_anki_desktop`.
//!
//! The frontend used to call AnkiConnect itself, which is why AnkiConnect had
//! to accept every web origin. These commands ask through `anki-bridge`
//! instead: requests from the backend carry no `Origin`, so AnkiConnect's
//! CORS list can stay limited to the app (Milestone E).

use std::{path::Path, time::Duration};

use anki_bridge::{recommended_note_type, AnkiClient, AnkiError, AutoMatch, FieldMapping};
use serde::{Deserialize, Serialize};
use tauri::State;
use vocab_core::Card;

use crate::{
    filing::{check_fields, FieldCheck},
    setup::os_args,
    state::AppState,
};

/// How long the status probe and the approval window's field check wait for
/// Anki: the settings window polls the one, the approval window waits for the
/// other, and a closed Anki should show up as offline quickly.
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// What `anki_status` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnkiStatus {
    /// AnkiConnect answered.
    pub online: bool,
    /// AnkiConnect's API version.
    pub version: Option<u32>,
    pub decks: Vec<String>,
    /// Note types.
    pub models: Vec<String>,
    /// The first thing that went wrong.
    pub error: Option<String>,
}

/// Whether AnkiConnect answers at the configured URL, plus the decks and
/// note types the settings form offers. Never fails: problems end up in
/// `error`.
#[tauri::command]
pub async fn anki_status(state: State<'_, AppState>) -> Result<AnkiStatus, String> {
    let client = AnkiClient::new(state.config().anki.connect_url).with_timeout(STATUS_TIMEOUT);
    let version = match client.version().await {
        Ok(version) => version,
        Err(err) => {
            return Ok(AnkiStatus {
                online: false,
                version: None,
                decks: Vec::new(),
                models: Vec::new(),
                error: Some(err.to_string()),
            })
        }
    };
    let mut error = None;
    let mut keep_first = |message: String| {
        error.get_or_insert(message);
    };
    let decks = client.deck_names().await.unwrap_or_else(|err| {
        keep_first(err.to_string());
        Vec::new()
    });
    let models = client.model_names().await.unwrap_or_else(|err| {
        keep_first(err.to_string());
        Vec::new()
    });
    Ok(AnkiStatus {
        online: true,
        version: Some(version),
        decks,
        models,
        error,
    })
}

/// The fields of note type `model`, in order: the settings form compares
/// them with the field mapping (Milestone J5).
#[tauri::command]
pub async fn anki_model_field_names(
    state: State<'_, AppState>,
    model: String,
) -> Result<Vec<String>, String> {
    AnkiClient::new(state.config().anki.connect_url)
        .model_field_names(&model)
        .await
        .map_err(|err| err.to_string())
}

/// A mapping for note type `model` matched from its field names, for cards
/// `source_lang` -> `target_lang` (`FieldMapping::auto_match`): the settings
/// form's "Match by field name". Nothing is saved; the form applies it.
#[tauri::command]
pub async fn suggest_field_mapping(
    state: State<'_, AppState>,
    model: String,
    source_lang: String,
    target_lang: String,
) -> Result<AutoMatch, String> {
    let fields = AnkiClient::new(state.config().anki.connect_url)
        .model_field_names(&model)
        .await
        .map_err(|err| err.to_string())?;
    Ok(FieldMapping::auto_match(
        &fields,
        &source_lang,
        &target_lang,
    ))
}

/// What a push of the pending card would drop: its note type's fields
/// against the mapping its filing uses (`filing::check_fields`). `card` is
/// the card as the approval window would submit it, with the user's edits (a
/// typed-in example the note type has no field for is lost too); without it
/// the pending card is checked. `None` when nothing is pending. Reads from
/// Anki only; the approval window shows the result as a warning, and
/// approving stays possible.
#[tauri::command]
pub async fn check_pending_fields(
    state: State<'_, AppState>,
    card: Option<Card>,
) -> Result<Option<FieldCheck>, String> {
    let Some(pending) = state.pending().get() else {
        return Ok(None);
    };
    let config = state.config();
    // Filing stays keyed on the pending capture (profile and language); the
    // draft only changes values.
    let filing = config.filing(pending.profile_id.as_deref(), &pending.card.source_lang);
    let card = card.unwrap_or(pending.card);
    let client = AnkiClient::new(config.anki.connect_url).with_timeout(STATUS_TIMEOUT);
    Ok(Some(check_fields(&client, &filing, &card).await))
}

/// What `create_recommended_note_type` set up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendedNoteType {
    /// The note type (`anki_bridge::RECOMMENDED_NOTE_TYPE`).
    pub model_name: String,
    /// The mapping that fills every one of its fields
    /// (`FieldMapping::recommended`).
    pub mapping: FieldMapping,
    /// `false` when Anki already had a note type of that name with every
    /// field, which is then used as it is.
    pub created: bool,
    /// Whether matching the note type's fields by name gives exactly
    /// `mapping` for every language pair (`FieldMapping::is_matched_by_name`):
    /// then the settings leave the mapping `null` instead of saving it.
    pub matches_by_name: bool,
}

/// Creates the recommended note type in the user's Anki collection
/// (`anki_bridge::recommended_note_type`), or reuses a note type of that name
/// that has all its fields. One of another shape is refused with a message,
/// and no existing note type is ever changed: AnkiConnect's `createModel`
/// only adds. It writes to the collection, so only the settings' button calls
/// it, never startup, and the tests only against a mock AnkiConnect. The
/// config is not touched; the settings form selects the note type and the
/// user saves.
#[tauri::command]
pub async fn create_recommended_note_type(
    state: State<'_, AppState>,
) -> Result<RecommendedNoteType, String> {
    let client = AnkiClient::new(state.config().anki.connect_url);
    let anki = |err: AnkiError| err.to_string();
    let spec = recommended_note_type();
    let mapping = FieldMapping::recommended();
    let exists = client
        .model_names()
        .await
        .map_err(anki)?
        .contains(&spec.name);
    let fields = if exists {
        let fields = client.model_field_names(&spec.name).await.map_err(anki)?;
        let missing = mapping.missing_fields(&fields);
        if !missing.is_empty() {
            return Err(format!(
                "Anki already has a note type named “{}”, but some of its fields are missing: \
                 {}. It was left as it is: rename it in Anki (Tools → Manage Note Types) and \
                 try again, or choose it and customize the field mapping.",
                spec.name,
                missing
                    .iter()
                    .map(|field| format!("“{field}”"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        log::info!("Anki already has the note type {:?}; using it.", spec.name);
        fields
    } else {
        client
            .create_model(&spec.name, &spec.fields, &spec.css, &spec.templates)
            .await
            .map_err(anki)?;
        log::info!("Created the note type {:?} in Anki.", spec.name);
        spec.fields
    };
    Ok(RecommendedNoteType {
        matches_by_name: mapping.is_matched_by_name(&fields),
        model_name: spec.name,
        mapping,
        created: !exists,
    })
}

/// Starts the Anki desktop app.
///
/// On macOS this waits for `open -a Anki`, which only asks LaunchServices and
/// returns at once, so its exit status is the one reliable signal that no app
/// named Anki exists (anywhere, not just in /Applications). Spawning it
/// without waiting reported "Starting Anki…" even when Anki was missing.
#[tauri::command]
pub async fn launch_anki_desktop(state: State<'_, AppState>) -> Result<String, String> {
    let setup = state.setup();
    let started = match std::env::consts::OS {
        "macos" => {
            let opened = setup
                .runner
                .run(Path::new("/usr/bin/open"), &os_args(["-a", "Anki"]))
                .await;
            if let Err(err) = opened {
                log::warn!("open -a Anki failed: {err}");
                return Err("Anki was not found on this Mac. Install it from \
                     https://apps.ankiweb.net or with the setup button."
                    .to_string());
            }
            Ok(())
        }
        "windows" => {
            let app = setup
                .paths()
                .map_err(|err| err.to_string())?
                .anki_app()
                .filter(|app| app.exists())
                .ok_or("Anki is not installed in its default location.")?;
            setup.runner.spawn(&app, &[])
        }
        // Distribution packages put `anki` on the PATH.
        _ => setup.runner.spawn(Path::new("anki"), &[]),
    };
    started.map_err(|err| format!("Could not start Anki: {err}"))?;
    log::info!("Starting Anki.");
    Ok("Starting Anki…".to_string())
}
