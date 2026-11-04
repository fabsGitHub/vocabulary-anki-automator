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
//! synthetic keystrokes need. The keystrokes are sent from the main thread
//! (see [`OnMainThread`]).

use std::{
    future::Future,
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
    let press_copy = || async { on_main_thread(app, press_copy_shortcut).await? };
    copy_through(app.clipboard(), press_copy, COPY_DELAY).await
}

/// Plain-text access to the clipboard, the only part [`copy_through`] needs.
trait TextClipboard {
    fn read(&self) -> Option<String>;
    fn write(&self, text: String) -> Result<(), String>;
}

impl<R: Runtime> TextClipboard for tauri_plugin_clipboard_manager::Clipboard<R> {
    fn read(&self) -> Option<String> {
        self.read_text().ok()
    }

    fn write(&self, text: String) -> Result<(), String> {
        self.write_text(text).map_err(|err| err.to_string())
    }
}

/// The clipboard steps around the copy keystroke: remember the user's text,
/// clear the clipboard (so "nothing was copied" can be told apart from an old
/// value), press copy, wait `copy_delay`, read, and put the user's text back,
/// also when the keystroke could not be sent.
async fn copy_through<F>(
    clipboard: &impl TextClipboard,
    press_copy: impl FnOnce() -> F,
    copy_delay: Duration,
) -> Result<String, String>
where
    F: Future<Output = Result<(), String>>,
{
    // Only text can be put back; other content (an image) is replaced by the
    // copied text, exactly as a manual copy would do.
    let previous = clipboard.read().filter(|text| !text.is_empty());
    if let Err(err) = clipboard.write(String::new()) {
        log::warn!("Could not clear the clipboard before copying: {err}");
    }
    let copied = match press_copy().await {
        Ok(()) => {
            tokio::time::sleep(copy_delay).await;
            Ok(clipboard.read().unwrap_or_default())
        }
        Err(message) => Err(message),
    };
    if let Some(previous) = previous {
        if let Err(err) = clipboard.write(previous) {
            log::warn!("Could not restore the previous clipboard text: {err}");
        }
    }
    let copied = copied?;
    if copied.trim().is_empty() {
        return Err(nothing_copied_message());
    }
    Ok(copied)
}

/// Proof that code runs on the main thread: only [`on_main_thread`] makes one.
/// The copy keystroke needs it on macOS: to find the key that types "c" in
/// the current keyboard layout, enigo asks the Text Input Sources API, and
/// since macOS 26 that API aborts the app when called from any other thread.
struct OnMainThread(());

/// Runs `f` on the main thread and returns its result. The main thread is
/// busy while `f` runs, so `f` must be short.
async fn on_main_thread<R: Runtime, T: Send + 'static>(
    app: &AppHandle<R>,
    f: impl FnOnce(OnMainThread) -> T + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        // Fails only if the capture waiting for it was dropped.
        let _ = sender.send(f(OnMainThread(())));
    })
    .map_err(|err| format!("The copy keystroke could not be sent: {err}"))?;
    receiver
        .await
        .map_err(|_| "The copy keystroke was not sent: the app is closing.".to_string())
}

/// Releases every modifier the hotkey may have left down, then sends the
/// platform's copy shortcut, on the main thread. That takes the three
/// `KEY_STEP` pauses plus enigo's own delay before each of the 8 key events
/// (20 ms on macOS, so about 235 ms in all). `Enigo` lives only inside this
/// synchronous function: it must never be held across an `.await`.
fn press_copy_shortcut(_: OnMainThread) -> Result<(), String> {
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
    use std::{
        panic::{self, AssertUnwindSafe},
        sync::{mpsc, Mutex},
    };

    use tauri::RunEvent;

    use super::*;

    /// A clipboard holding `text`; every write is recorded.
    struct FakeClipboard {
        text: Mutex<Option<String>>,
        writes: Mutex<Vec<String>>,
    }

    impl FakeClipboard {
        fn holding(text: Option<&str>) -> Self {
            Self {
                text: Mutex::new(text.map(str::to_string)),
                writes: Mutex::default(),
            }
        }

        /// What another app's Cmd/Ctrl+C would do.
        fn copy_into(&self, text: &str) {
            *self.text.lock().unwrap() = Some(text.to_string());
        }

        fn current(&self) -> Option<String> {
            self.text.lock().unwrap().clone()
        }
    }

    impl TextClipboard for FakeClipboard {
        fn read(&self) -> Option<String> {
            self.current()
        }

        fn write(&self, text: String) -> Result<(), String> {
            self.writes.lock().unwrap().push(text.clone());
            *self.text.lock().unwrap() = Some(text);
            Ok(())
        }
    }

    fn copy<F: Future<Output = Result<(), String>>>(
        clipboard: &FakeClipboard,
        press: impl FnOnce() -> F,
    ) -> Result<String, String> {
        tauri::async_runtime::block_on(copy_through(clipboard, press, Duration::ZERO))
    }

    #[test]
    fn the_selection_is_returned_and_the_users_text_put_back() {
        let clipboard = FakeClipboard::holding(Some("the user's text"));

        let copied = copy(&clipboard, || async {
            clipboard.copy_into("serendipity");
            Ok(())
        });

        assert_eq!(copied.as_deref(), Ok("serendipity"));
        assert_eq!(clipboard.current().as_deref(), Some("the user's text"));
        assert_eq!(*clipboard.writes.lock().unwrap(), ["", "the user's text"]);
    }

    #[test]
    fn nothing_copied_is_an_error_and_the_users_text_is_put_back() {
        let clipboard = FakeClipboard::holding(Some("the user's text"));

        let err = copy(&clipboard, || async { Ok(()) }).unwrap_err();

        assert_eq!(err, nothing_copied_message());
        assert_eq!(clipboard.current().as_deref(), Some("the user's text"));
    }

    #[test]
    fn a_failed_keystroke_still_puts_the_users_text_back() {
        let clipboard = FakeClipboard::holding(Some("the user's text"));

        let err = copy(&clipboard, || async {
            Err("no keyboard simulator".to_string())
        })
        .unwrap_err();

        assert_eq!(err, "no keyboard simulator");
        assert_eq!(clipboard.current().as_deref(), Some("the user's text"));
    }

    #[test]
    fn without_text_to_restore_the_selection_stays_on_the_clipboard() {
        let clipboard = FakeClipboard::holding(None);

        let copied = copy(&clipboard, || async {
            clipboard.copy_into("Haus");
            Ok(())
        });

        assert_eq!(copied.as_deref(), Ok("Haus"));
        assert_eq!(clipboard.current().as_deref(), Some("Haus"));
    }

    /// The mock event loop never ends on its own, so the test ends it by
    /// unwinding with this payload.
    const STOP: &str = "stop the event loop";

    /// Loop iterations to wait for the keystroke; each one sleeps 1 s.
    const MAX_IDLE_LOOPS: u32 = 5;

    #[test]
    fn the_copy_keystroke_runs_on_the_event_loop_thread() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app");
        let event_loop_thread = thread::current().id();
        let (sender, receiver) = mpsc::channel();
        let ran_on = Arc::new(Mutex::new(None));
        let seen = Arc::clone(&ran_on);

        let outcome = panic::catch_unwind(AssertUnwindSafe(move || {
            let mut sender = Some(sender);
            let mut idle_loops = 0;
            app.run(move |handle, event| match event {
                RunEvent::Ready => {
                    let handle = handle.clone();
                    let sender = sender.take().expect("one Ready event");
                    // From a task on the async runtime, like the hotkey capture.
                    tauri::async_runtime::spawn(async move {
                        let result = on_main_thread(&handle, |_| thread::current().id()).await;
                        sender.send(result).expect("the test is waiting");
                    });
                }
                RunEvent::MainEventsCleared => {
                    if let Ok(result) = receiver.try_recv() {
                        *seen.lock().unwrap() = Some(result);
                        panic::panic_any(STOP);
                    }
                    idle_loops += 1;
                    assert!(idle_loops < MAX_IDLE_LOOPS, "the keystroke never ran");
                }
                _ => {}
            });
        }));

        if let Err(payload) = outcome {
            if payload.downcast_ref::<&str>() != Some(&STOP) {
                panic::resume_unwind(payload);
            }
        }
        assert_eq!(*ran_on.lock().unwrap(), Some(Ok(event_loop_thread)));
    }

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
