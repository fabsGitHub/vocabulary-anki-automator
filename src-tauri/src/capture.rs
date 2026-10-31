//! The capture flow: hotkey -> copy the selection -> capture pipeline ->
//! approval window (Milestones B3 and D).
//!
//! The hotkey path and `capture_text` share everything after the clipboard,
//! so tests and E2E runs exercise the real pipeline wiring without synthetic
//! keystrokes. The clipboard part keeps the pre-refactor keystroke sequence
//! and timing, which is what works across apps on macOS and Windows: wait for
//! the user to let go of the hotkey, release any modifier still down, send
//! Cmd/Ctrl+C, then give the pasteboard time to update. Two things changed:
//! the user's previous clipboard text is put back afterwards, and an empty
//! clipboard is reported instead of silently dropped, because on macOS it
//! almost always means the app lacks the Accessibility permission that
//! synthetic keystrokes need.

use std::{
    sync::{Arc, OnceLock},
    thread,
    time::Duration,
};

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
use vocab_core::{
    curriculum::CurriculumIndex,
    pipeline::{CapturePipeline, CaptureRequest, PipelineError, PipelineWarning},
    providers::{ProviderRegistry, StandardProviders},
    Card,
};

use crate::{
    config::{AppConfig, CaptureProfile},
    state::AppState,
};

/// Label of the approval window (docs/architecture.md § 8.3).
pub const PREVIEW_LABEL: &str = "preview";
/// Emitted to the `preview` window with a [`PendingCapture`].
pub const EVENT_TRANSLATION_PAYLOAD: &str = "translation-payload";
/// Emitted app-wide with a [`CaptureErrorPayload`].
pub const EVENT_CAPTURE_ERROR: &str = "capture-error";
/// Emitted app-wide, without payload, after a capture is saved, deleted or
/// synced.
pub const EVENT_CAPTURES_CHANGED: &str = "captures-changed";

/// Time for the user to let go of the hotkey before the copy keystroke.
const RELEASE_DELAY: Duration = Duration::from_millis(350);
/// Pause between the synthetic key events.
const KEY_STEP: Duration = Duration::from_millis(25);
/// Time for the pasteboard to receive the copied text.
const COPY_DELAY: Duration = Duration::from_millis(300);

/// A capture waiting for approval: what `get_pending_payload` returns and the
/// `translation-payload` event carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingCapture {
    pub card: Card,
    /// Provider failures the capture survived, for the approval window.
    pub warnings: Vec<PipelineWarning>,
    /// The hotkey profile it was captured with; `None` for the default one.
    pub profile_id: Option<String>,
}

/// Payload of `capture-error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureErrorPayload {
    pub message: String,
}

/// Builds the providers a capture (or a TTS re-run) uses. Injected so the
/// test suites run the real flow against fakes instead of the network.
pub trait ProviderFactory: Send + Sync {
    fn registry(&self, config: &AppConfig) -> ProviderRegistry;
}

impl<F> ProviderFactory for F
where
    F: Fn(&AppConfig) -> ProviderRegistry + Send + Sync,
{
    fn registry(&self, config: &AppConfig) -> ProviderRegistry {
        self(config)
    }
}

/// [`ProviderRegistry::standard`] with the configured MyMemory contact e-mail
/// and the bundled HSK curriculum, which is parsed once, on first use.
#[derive(Default)]
pub struct StandardProviderFactory {
    curriculum: OnceLock<Arc<CurriculumIndex>>,
}

impl ProviderFactory for StandardProviderFactory {
    fn registry(&self, config: &AppConfig) -> ProviderRegistry {
        let curriculum = self
            .curriculum
            .get_or_init(|| Arc::new(CurriculumIndex::bundled()))
            .clone();
        ProviderRegistry::standard(StandardProviders {
            contact_email: config.translation.contact_email.clone(),
            curriculum,
        })
    }
}

/// Starts a capture with `profile` in the background: copies the selection,
/// runs the pipeline and opens the approval window. A hotkey pressed while a
/// capture is still running is ignored.
pub fn start_hotkey_capture<R: Runtime>(app: &AppHandle<R>, profile: CaptureProfile) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let Some(_running) = state.begin_capture() else {
            log::info!("A capture is still running; ignoring the hotkey.");
            return;
        };
        log::info!(
            "Hotkey capture started ({} -> {}).",
            profile.source_lang,
            profile.target_lang
        );
        match copy_selection(&app).await {
            Ok(text) => {
                // Failures are reported to the user inside; nothing left to do.
                let _ = run_capture(&app, &state, text, &profile).await;
            }
            Err(message) => emit_capture_error(&app, message),
        }
    });
}

/// Runs `text` through the pipeline for `profile`, stores the result as the
/// pending capture and shows it in the approval window. Invalid input (empty,
/// too long) is reported as a `capture-error` event and returned.
pub async fn run_capture<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    text: String,
    profile: &CaptureProfile,
) -> Result<(), PipelineError> {
    let config = state.config();
    let pipeline = CapturePipeline::new(
        state.providers().registry(&config),
        config.translation.max_chars,
    );
    let request = CaptureRequest {
        text,
        source_lang: profile.source_lang.clone(),
        target_lang: profile.target_lang.clone(),
        captured_at: state.now(),
        tags: Vec::new(),
    };
    let outcome = match pipeline.run(request).await {
        Ok(outcome) => outcome,
        Err(err) => {
            log::info!("Capture refused: {err}");
            emit_capture_error(app, pipeline_error_message(&err));
            return Err(err);
        }
    };
    log::info!(
        "Captured {} characters ({} -> {}), {} warning(s).",
        outcome.card.front.chars().count(),
        outcome.card.source_lang,
        outcome.card.target_lang,
        outcome.warnings.len()
    );
    let pending = PendingCapture {
        card: outcome.card,
        warnings: outcome.warnings,
        profile_id: profile.id.clone(),
    };
    state.pending().set(pending.clone());
    present_preview(app, &pending);
    Ok(())
}

/// Shows `pending` in the approval window. An existing window (usually
/// hidden after the previous approval) gets the payload as an event; a new
/// one fetches it itself with `get_pending_payload` once its page has loaded.
fn present_preview<R: Runtime>(app: &AppHandle<R>, pending: &PendingCapture) {
    if let Some(window) = app.get_webview_window(PREVIEW_LABEL) {
        if let Err(err) = app.emit_to(PREVIEW_LABEL, EVENT_TRANSLATION_PAYLOAD, pending) {
            log::error!("Could not send the capture to the approval window: {err}");
        }
        if let Err(err) = window.show().and_then(|()| window.set_focus()) {
            log::warn!("Could not bring the approval window to the front: {err}");
        }
        return;
    }
    let built = WebviewWindowBuilder::new(app, PREVIEW_LABEL, WebviewUrl::App("index.html".into()))
        .title("Approve card")
        .inner_size(520.0, 640.0)
        .always_on_top(true)
        .focused(true)
        .build();
    if let Err(err) = built {
        log::error!("Could not open the approval window: {err}");
        emit_capture_error(
            app,
            format!("The approval window could not be opened: {err}"),
        );
    }
}

/// Emits `capture-error` to every window.
pub fn emit_capture_error<R: Runtime>(app: &AppHandle<R>, message: impl Into<String>) {
    let payload = CaptureErrorPayload {
        message: message.into(),
    };
    log::warn!("Capture error: {}", payload.message);
    if let Err(err) = app.emit(EVENT_CAPTURE_ERROR, payload) {
        log::error!("Could not report the capture error: {err}");
    }
}

/// Emits `captures-changed` to every window.
pub fn emit_captures_changed<R: Runtime>(app: &AppHandle<R>) {
    if let Err(err) = app.emit(EVENT_CAPTURES_CHANGED, ()) {
        log::error!("Could not announce the history change: {err}");
    }
}

/// What the user is told when the pipeline refuses a capture.
pub fn pipeline_error_message(err: &PipelineError) -> String {
    match err {
        PipelineError::Empty => "Nothing to capture: the selected text is empty.".to_string(),
        PipelineError::TooLong { len, max } => format!(
            "The selection is {len} characters long, but captures are limited to {max} \
             characters. Select less text or raise the limit in the settings."
        ),
    }
}

/// Copies the current selection in whatever app has focus and returns it,
/// putting the user's previous clipboard text back afterwards. Nothing
/// copied is an error with a hint at the usual cause.
async fn copy_selection<R: Runtime>(app: &AppHandle<R>) -> Result<String, String> {
    tokio::time::sleep(RELEASE_DELAY).await;
    let clipboard = app.clipboard();
    // Only text can be put back; other content (an image) is replaced by the
    // copied text, exactly as a manual copy would do.
    let previous = clipboard.read_text().ok().filter(|text| !text.is_empty());
    if let Err(err) = clipboard.write_text(String::new()) {
        log::warn!("Could not clear the clipboard before copying: {err}");
    }
    press_copy_shortcut()?;
    tokio::time::sleep(COPY_DELAY).await;
    let copied = clipboard.read_text().unwrap_or_default();
    if let Some(previous) = previous {
        if let Err(err) = clipboard.write_text(previous) {
            log::warn!("Could not restore the previous clipboard text: {err}");
        }
    }
    if copied.trim().is_empty() {
        return Err(nothing_copied_message());
    }
    Ok(copied)
}

/// Releases every modifier the hotkey may have left down, then sends the
/// platform's copy shortcut. `Enigo` lives only inside this synchronous
/// function: it must never be held across an `.await`.
fn press_copy_shortcut() -> Result<(), String> {
    let mut enigo = Enigo::new(&Settings::default()).map_err(|err| {
        log::error!("Could not create the keyboard simulator: {err}");
        nothing_copied_message()
    })?;
    let command = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    let mut failures = Vec::new();
    let mut step = |key: Key, direction: Direction| {
        if let Err(err) = enigo.key(key, direction) {
            failures.push(format!("{key:?} {direction:?}: {err}"));
        }
    };
    for modifier in [Key::Shift, Key::Control, Key::Alt, Key::Meta] {
        step(modifier, Direction::Release);
    }
    thread::sleep(KEY_STEP);
    step(command, Direction::Press);
    thread::sleep(KEY_STEP);
    step(Key::Unicode('c'), Direction::Click);
    thread::sleep(KEY_STEP);
    step(command, Direction::Release);
    if !failures.is_empty() {
        log::warn!("Some synthetic key events failed: {}", failures.join("; "));
    }
    Ok(())
}

fn nothing_copied_message() -> String {
    let mut message =
        "Nothing was copied. Select some text, then press the hotkey again.".to_string();
    if cfg!(target_os = "macos") {
        message.push_str(
            " If text was selected, allow Vocabulary Automator under System Settings > \
             Privacy & Security > Accessibility: macOS blocks the simulated Cmd+C without it.",
        );
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_capture_has_the_contract_shape() {
        let card = Card::new(
            "Haus",
            "house",
            "de",
            "en",
            chrono::DateTime::from_timestamp_millis(1_793_448_000_000).unwrap(),
        );
        let pending = PendingCapture {
            card,
            warnings: Vec::new(),
            profile_id: Some("de-en".into()),
        };
        let json = serde_json::to_value(&pending).unwrap();
        let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["card", "profile_id", "warnings"]);
        assert_eq!(json["profile_id"], "de-en");
        assert_eq!(json["card"]["captured_at"], "2026-10-31T12:00:00Z");
        assert_eq!(
            serde_json::from_value::<PendingCapture>(json).unwrap(),
            pending
        );
    }

    #[test]
    fn refusals_explain_what_to_do() {
        assert_eq!(
            pipeline_error_message(&PipelineError::TooLong { len: 140, max: 100 }),
            "The selection is 140 characters long, but captures are limited to 100 characters. \
             Select less text or raise the limit in the settings."
        );
        assert!(pipeline_error_message(&PipelineError::Empty).contains("empty"));
    }

    #[test]
    fn the_empty_clipboard_hint_names_the_macos_permission() {
        let message = nothing_copied_message();
        assert!(message.starts_with("Nothing was copied."));
        assert_eq!(
            message.contains("Accessibility"),
            cfg!(target_os = "macos"),
            "{message}"
        );
    }
}
