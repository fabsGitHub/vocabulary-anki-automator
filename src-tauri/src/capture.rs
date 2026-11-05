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
//!
//! With learned languages configured, the default hotkey chooses the source
//! language per capture before the pipeline runs (`vocab_core::detect`, with
//! lingua through [`LinguaGuesser`]); the pipeline itself is unchanged, and so
//! is every capture while the list is empty. When the guess is wrong, the
//! approval window re-runs the pending capture for the language the user
//! picks ([`rerun_capture`]).

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
    detect::{detect, Detection, DetectionMethod},
    language::{contains_cjk, normalize_lang, SUPPORTED_LANGS},
    pipeline::{CapturePipeline, CaptureRequest, PipelineError, PipelineWarning},
    providers::{ProviderRegistry, StandardProviders},
    Card,
};

use crate::{
    config::{AppConfig, CaptureProfile},
    detect::LinguaGuesser,
    push::LAST_SOURCE_LANG_KEY,
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
    /// How the source language was chosen; `None` when it was not detected
    /// (a hotkey profile fixes it, or no languages are learned).
    #[serde(default)]
    pub detection: Option<Detection>,
    /// The deck and note type the card will be filed under (from
    /// [`AppConfig::filing`]).
    #[serde(default)]
    pub filing: PendingFiling,
    /// Which capture this is: [`PendingPayload::set`](crate::state::PendingPayload::set)
    /// numbers every capture, and a re-run keeps its number. The approval
    /// window names it when it re-runs the card, so a newer capture is never
    /// re-run with the front of an older one.
    #[serde(default)]
    pub generation: u64,
}

/// Where a pending card will go, for the approval window to show. The push
/// decides again with the config of that moment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFiling {
    pub deck: String,
    /// The Anki note type ("model").
    pub model: String,
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
        let learned = state.config().detection_languages(&profile);
        if learned.is_empty() {
            log::info!(
                "Hotkey capture started ({} -> {}).",
                profile.source_lang,
                profile.target_lang
            );
        } else {
            log::info!(
                "Hotkey capture started (auto among [{}] -> {}).",
                learned.join(", "),
                profile.target_lang
            );
        }
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
/// too long) is reported as a `capture-error` event and returned. The
/// default hotkey with learned languages detects the source language first.
pub async fn run_capture<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    text: String,
    profile: &CaptureProfile,
) -> Result<(), PipelineError> {
    let config = state.config();
    let detection = detect_source(state, &config, profile, &text);
    let request = CaptureRequest {
        text,
        source_lang: detection
            .as_ref()
            .map_or_else(|| profile.source_lang.clone(), |found| found.lang.clone()),
        target_lang: profile.target_lang.clone(),
        captured_at: state.now(),
        tags: Vec::new(),
    };
    let mut pending = match build_pending(state, &config, request, profile.id.clone()).await {
        Ok(pending) => PendingCapture {
            detection,
            ..pending
        },
        Err(err) => {
            log::info!("Capture refused: {err}");
            emit_capture_error(app, pipeline_error_message(&err));
            return Err(err);
        }
    };
    pending.generation = state.pending().set(pending.clone());
    present_preview(app, &pending);
    Ok(())
}

/// Captures the pending card's text again as `source_lang`, the language the
/// user picked in the approval window, and shows the result in its place
/// (docs/architecture.md § 8.2): translation, example, audio and filing are
/// fetched again; `captured_at`, tags and hotkey profile stay. `text` is the
/// window's front, so an edit of it survives. Errors are returned for the
/// window to show, never emitted as `capture-error`, and leave the pending
/// capture as it was. `expected_generation` is the capture the window shows;
/// a newer one that arrived since is refused, not re-run with the old front.
///
/// Both the capture and the submission guard are held throughout: a hotkey
/// pressed meanwhile is ignored, and an approval waits for the new card
/// instead of saving the old one. Han text cannot be re-run as another
/// language, because the pipeline would read it as Chinese anyway.
pub async fn rerun_capture<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    source_lang: &str,
    text: String,
    expected_generation: u64,
) -> Result<PendingCapture, String> {
    let lang = normalize_lang(source_lang);
    if !SUPPORTED_LANGS.contains(&lang.as_str()) {
        return Err(format!(
            "\"{}\" is not a supported language.",
            source_lang.trim()
        ));
    }
    let Some(_capturing) = state.begin_capture() else {
        return Err("A capture is already running.".to_string());
    };
    let Some(_submitting) = state.begin_submit() else {
        return Err("The card is being saved.".to_string());
    };
    let Some((generation, previous)) = state.pending().snapshot() else {
        return Err("There is no card to switch.".to_string());
    };
    if generation != expected_generation {
        return Err("The card was saved or discarded in the meantime.".to_string());
    }
    if lang == previous.card.target_lang {
        return Err(format!(
            "Cards are translated into {lang}; choose another language."
        ));
    }
    if contains_cjk(&text) && lang != "zh-CN" {
        return Err("Text with Chinese characters is always captured as Chinese.".to_string());
    }
    let config = state.config();
    let request = CaptureRequest {
        text,
        source_lang: lang,
        target_lang: previous.card.target_lang.clone(),
        captured_at: previous.card.captured_at,
        tags: previous.card.tags.clone(),
    };
    let mut pending = build_pending(state, &config, request, previous.profile_id.clone())
        .await
        .map_err(|err| pipeline_error_message(&err))?;
    pending.detection = previous
        .detection
        .map(|found| chosen(found, &pending.card.source_lang));
    pending.generation = generation;
    if !state.pending().replace_if(generation, pending.clone()) {
        return Err("The card was saved or discarded in the meantime.".to_string());
    }
    log::info!(
        "Switched the pending capture to {}.",
        pending.card.source_lang
    );
    present_preview(app, &pending);
    Ok(pending)
}

/// `found` after the user chose `lang` in the approval window: certain, and
/// still offering the same languages (plus `lang`, should it be new).
fn chosen(found: Detection, lang: &str) -> Detection {
    let mut candidates = found.candidates;
    if !candidates.iter().any(|code| code == lang) {
        candidates.push(lang.to_string());
    }
    Detection {
        lang: lang.to_string(),
        method: DetectionMethod::Chosen,
        confident: true,
        script: found.script,
        candidates,
        ranked: found.ranked,
    }
}

/// Runs `request` through the pipeline and files the result for
/// `profile_id`; the caller adds the detection. Shared by a capture and its
/// re-run.
async fn build_pending(
    state: &AppState,
    config: &AppConfig,
    request: CaptureRequest,
    profile_id: Option<String>,
) -> Result<PendingCapture, PipelineError> {
    let pipeline = CapturePipeline::new(
        state.providers().registry(config),
        config.translation.max_chars,
    );
    let outcome = pipeline.run(request).await?;
    log::info!(
        "Captured {} characters ({} -> {}), {} warning(s).",
        outcome.card.front.chars().count(),
        outcome.card.source_lang,
        outcome.card.target_lang,
        outcome.warnings.len()
    );
    let filing = config.filing(profile_id.as_deref(), &outcome.card.source_lang);
    Ok(PendingCapture {
        card: outcome.card,
        warnings: outcome.warnings,
        profile_id,
        detection: None,
        filing: PendingFiling {
            deck: filing.deck,
            model: filing.model,
        },
        // Numbered once it is stored.
        generation: 0,
    })
}

/// Which of the learned languages `text` is in, for a capture with
/// `profile`: `None` when the profile does not detect (a hotkey profile, or
/// no learned languages) and for text the pipeline will refuse anyway (empty
/// or too long), so those captures run exactly as before. Han text for a
/// learner whose own language is Chinese is also left alone: the pipeline
/// already reverses the pair for it.
fn detect_source(
    state: &AppState,
    config: &AppConfig,
    profile: &CaptureProfile,
    text: &str,
) -> Option<Detection> {
    let learned = config.detection_languages(profile);
    let trimmed = text.trim();
    if learned.is_empty()
        || trimmed.is_empty()
        || trimmed.chars().count() > config.translation.max_chars
    {
        return None;
    }
    // Without a history (Anki-only mode) there is just no tie-break.
    let last = state
        .with_store(|store| store.get_state(LAST_SOURCE_LANG_KEY))
        .ok()
        .flatten();
    let found = detect(
        trimmed,
        &learned,
        &profile.source_lang,
        last.as_deref(),
        &LinguaGuesser,
    );
    log::info!(
        "Detected {} ({:?}, confident: {}).",
        found.lang,
        found.method,
        found.confident
    );
    (found.lang != normalize_lang(&profile.target_lang)).then_some(found)
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
            " If text was selected, allow Polywort under System Settings > \
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
            detection: None,
            filing: PendingFiling {
                deck: "german".into(),
                model: "Basic".into(),
            },
            generation: 3,
        };
        let json = serde_json::to_value(&pending).unwrap();
        let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "card",
                "detection",
                "filing",
                "generation",
                "profile_id",
                "warnings"
            ]
        );
        assert_eq!(json["generation"], 3);
        assert_eq!(json["detection"], serde_json::Value::Null);
        assert_eq!(json["profile_id"], "de-en");
        assert_eq!(
            json["filing"],
            serde_json::json!({ "deck": "german", "model": "Basic" })
        );
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
