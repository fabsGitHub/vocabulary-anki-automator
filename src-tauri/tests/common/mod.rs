//! Shared harness for the `tauri::test` suites (Milestone G).
//!
//! Each [`Harness`] is a mock-runtime app built with the real invoke handler
//! (`vocab_automator_lib::configure`), so commands run exactly as in the app,
//! through the IPC layer. Everything outside the process is replaced: the
//! config, the SQLite file and Anki's folders live in a temp dir, the
//! providers are deterministic fakes, the clock is fixed, and setup's
//! downloader and process runner only record what they were asked to do. A
//! guard test (`tests/ipc.rs`) fails if the harness ever points at a real
//! location.

#![allow(dead_code)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY},
    webview::InvokeRequest,
    App, Listener, Manager, WebviewWindow, WebviewWindowBuilder,
};
use tempfile::TempDir;
use vocab_automator_lib::{
    config::{save_config_to, AppConfig},
    setup::{
        download::{DownloadBody, Downloader},
        AnkiPaths, CommandRunner, SetupEnv, SetupError,
    },
    state::{AppServices, AppState},
};
use vocab_core::{
    providers::{
        ExampleProvider, ProviderRegistry, ProviderSet, RarityProvider, RarityResult,
        ScriptAnnotator, TranslationProvider, TtsProvider,
    },
    store::Store,
    time::FixedClock,
    ProviderError, ScriptMetadata, ScriptVariant,
};

/// Where Anki is "running" when a test wants it offline: nothing listens on
/// port 1, so every request fails at once without leaving the machine.
pub const UNREACHABLE_ANKI: &str = "http://127.0.0.1:1";

/// The fixed "now" of every harness: 2026-10-31T12:00:00.123Z.
pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap() + chrono::TimeDelta::milliseconds(123)
}

/// The words the fake translator knows; anything else becomes `[lang] text`.
const DICTIONARY: &[(&str, &str, &str)] = &[
    ("serendipity", "de", "glücklicher Zufall"),
    ("house", "de", "Haus"),
    ("学习", "de", "lernen"),
];

/// Every call a fake provider received, e.g. `"tts de glücklicher Zufall"`.
#[derive(Clone, Default)]
pub struct CallLog(Arc<Mutex<Vec<String>>>);

impl CallLog {
    fn push(&self, call: String) {
        self.0.lock().unwrap().push(call);
    }

    pub fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    pub fn tts_calls(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|call| call.starts_with("tts "))
            .collect()
    }
}

/// The audio the fake TTS returns for `text` in `lang`.
pub fn fake_audio(lang: &str, text: &str) -> Vec<u8> {
    format!("mp3:{lang}:{text}").into_bytes()
}

struct FakeTranslation(CallLog);
struct FakeRarity(CallLog);
struct FakeTts(CallLog);
struct FakeExample(CallLog);
struct FakeAnnotator(CallLog);

#[async_trait]
impl TranslationProvider for FakeTranslation {
    fn name(&self) -> &'static str {
        "fake_translation"
    }

    async fn translate(
        &self,
        text: &str,
        source: &str,
        target: &str,
    ) -> Result<String, ProviderError> {
        self.0.push(format!("translate {source}->{target} {text}"));
        if text == "quota" {
            return Err(ProviderError::QuotaExceeded {
                provider: "fake_translation".into(),
                message: "come back tomorrow".into(),
            });
        }
        Ok(DICTIONARY
            .iter()
            .find(|(word, lang, _)| *word == text && *lang == target)
            .map(|(_, _, translation)| translation.to_string())
            .unwrap_or_else(|| format!("[{target}] {text}")))
    }
}

/// Only "sesquipedalian" is rare.
#[async_trait]
impl RarityProvider for FakeRarity {
    fn name(&self) -> &'static str {
        "fake_rarity"
    }

    async fn analyze(&self, word: &str) -> Result<RarityResult, ProviderError> {
        self.0.push(format!("rarity {word}"));
        Ok(RarityResult {
            is_rare: word == "sesquipedalian",
            frequency_per_million: Some(0.1),
            synonym: Some("long".into()),
        })
    }
}

#[async_trait]
impl TtsProvider for FakeTts {
    fn name(&self) -> &'static str {
        "fake_tts"
    }

    async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError> {
        self.0.push(format!("tts {lang} {text}"));
        Ok(fake_audio(lang, text))
    }
}

#[async_trait]
impl ExampleProvider for FakeExample {
    fn name(&self) -> &'static str {
        "fake_example"
    }

    async fn fetch_example(&self, word: &str) -> Result<String, ProviderError> {
        self.0.push(format!("example {word}"));
        Ok(format!("An example with {word}."))
    }
}

#[async_trait]
impl ScriptAnnotator for FakeAnnotator {
    fn name(&self) -> &'static str {
        "fake_annotator"
    }

    async fn annotate(&self, text: &str) -> Result<ScriptMetadata, ProviderError> {
        self.0.push(format!("annotate {text}"));
        Ok(ScriptMetadata::Chinese {
            pinyin: if text == "学习" {
                "xué xí".to_string()
            } else {
                format!("pinyin of {text}")
            },
            hsk_level: (text == "学习").then_some(1),
            script_variant: ScriptVariant::Simplified,
        })
    }
}

/// English gets rarity and example providers, Chinese an annotator, every
/// language translation and TTS; all record into `log`.
fn fake_registry(log: &CallLog) -> ProviderRegistry {
    let translation: Arc<dyn TranslationProvider> = Arc::new(FakeTranslation(log.clone()));
    let tts: Arc<dyn TtsProvider> = Arc::new(FakeTts(log.clone()));
    let default = ProviderSet {
        tts: Some(tts),
        ..ProviderSet::new(translation)
    };
    let english = ProviderSet {
        rarity: Some(Arc::new(FakeRarity(log.clone()))),
        example: Some(Arc::new(FakeExample(log.clone()))),
        ..default.clone()
    };
    let chinese = ProviderSet {
        annotator: Some(Arc::new(FakeAnnotator(log.clone()))),
        ..default.clone()
    };
    ProviderRegistry::new(default)
        .with_language("en", english)
        .with_language("zh-CN", chinese)
}

/// A served URL and its body.
type Served = (String, Vec<u8>);

/// Records every URL and serves the bodies it was given (404 otherwise).
#[derive(Clone, Default)]
pub struct RecordingDownloader {
    bodies: Arc<Mutex<Vec<Served>>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl RecordingDownloader {
    pub fn serve(&self, url: impl Into<String>, body: impl Into<Vec<u8>>) {
        self.bodies.lock().unwrap().push((url.into(), body.into()));
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

struct OneChunk(Option<Vec<u8>>);

#[async_trait]
impl DownloadBody for OneChunk {
    fn content_length(&self) -> Option<u64> {
        self.0.as_ref().map(|body| body.len() as u64)
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SetupError> {
        Ok(self.0.take())
    }
}

#[async_trait]
impl Downloader for RecordingDownloader {
    async fn get(&self, url: &str) -> Result<Box<dyn DownloadBody>, SetupError> {
        self.requests.lock().unwrap().push(url.to_string());
        let body = self
            .bodies
            .lock()
            .unwrap()
            .iter()
            .find(|(served, _)| served == url)
            .map(|(_, body)| body.clone());
        match body {
            Some(body) => Ok(Box::new(OneChunk(Some(body)))),
            None => Err(SetupError::Download {
                url: url.to_string(),
                message: "HTTP 404 Not Found".into(),
            }),
        }
    }
}

/// A program and its arguments.
pub type Call = (String, Vec<String>);

/// Records every program it is asked to run or start, and runs none.
#[derive(Clone, Default)]
pub struct RecordingRunner {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl RecordingRunner {
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, program: &Path, args: &[OsString]) {
        self.calls.lock().unwrap().push((
            program.to_string_lossy().into_owned(),
            args.iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        ));
    }
}

#[async_trait]
impl CommandRunner for RecordingRunner {
    async fn run(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError> {
        self.record(program, args);
        Ok(())
    }

    fn spawn(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError> {
        self.record(program, args);
        Ok(())
    }
}

/// How a [`Harness`] starts.
pub struct Options {
    pub config: AppConfig,
    /// `false` simulates a database that failed to open (Anki-only mode).
    pub with_store: bool,
}

impl Default for Options {
    fn default() -> Self {
        let mut config = AppConfig::default();
        config.anki.connect_url = UNREACHABLE_ANKI.to_string();
        Self {
            config,
            with_store: true,
        }
    }
}

pub struct Harness {
    pub app: App<MockRuntime>,
    pub main: WebviewWindow<MockRuntime>,
    pub dir: TempDir,
    pub providers: CallLog,
    pub downloader: RecordingDownloader,
    pub runner: RecordingRunner,
    events: Arc<Mutex<Vec<(String, Value)>>>,
}

impl Harness {
    /// A harness whose Anki is unreachable.
    pub fn new() -> Self {
        Self::with(Options::default())
    }

    /// A harness talking to AnkiConnect at `url`.
    pub fn with_anki(url: &str) -> Self {
        let mut options = Options::default();
        options.config.anki.connect_url = url.to_string();
        Self::with(options)
    }

    pub fn with(options: Options) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("app-data").join("config.json");
        save_config_to(&config_path, &options.config).expect("initial config");
        let store = if options.with_store {
            Store::open(dir.path().join("vocab.sqlite3")).map_err(|err| err.to_string())
        } else {
            Err("simulated: the database could not be opened".to_string())
        };
        let providers = CallLog::default();
        let downloader = RecordingDownloader::default();
        let runner = RecordingRunner::default();
        let registry = fake_registry(&providers);
        let state = AppState::new(AppServices {
            config_path,
            store,
            providers: Arc::new(move |_: &AppConfig| registry.clone()),
            clock: Arc::new(FixedClock::utc(now())),
            setup: SetupEnv {
                paths: Ok(temp_anki_paths(dir.path())),
                downloader: Arc::new(downloader.clone()),
                runner: Arc::new(runner.clone()),
            },
        });
        let app = vocab_automator_lib::configure(mock_builder())
            .manage(state)
            .build(mock_context(noop_assets()))
            .expect("mock app");
        let main = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("main window");

        let events = Arc::new(Mutex::new(Vec::new()));
        for name in ["captures-changed", "capture-error", "translation-payload"] {
            let events = events.clone();
            app.listen_any(name, move |event| {
                let payload = serde_json::from_str(event.payload()).unwrap_or(Value::Null);
                events.lock().unwrap().push((name.to_string(), payload));
            });
        }
        Self {
            app,
            main,
            dir,
            providers,
            downloader,
            runner,
            events,
        }
    }

    /// Invokes `cmd` with `args` (camelCase keys, as the frontend sends them)
    /// from the main window, through the IPC layer.
    pub fn invoke(&self, cmd: &str, args: Value) -> Result<Value, Value> {
        let url = if cfg!(windows) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        };
        get_ipc_response(
            &self.main,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: url.parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|body| body.deserialize::<Value>().expect("JSON response"))
    }

    /// [`invoke`](Self::invoke), expecting success.
    pub fn call(&self, cmd: &str, args: Value) -> Value {
        self.invoke(cmd, args)
            .unwrap_or_else(|err| panic!("{cmd} failed: {err}"))
    }

    /// [`invoke`](Self::invoke), expecting a string error.
    pub fn call_err(&self, cmd: &str, args: Value) -> String {
        match self.invoke(cmd, args) {
            Ok(value) => panic!("{cmd} should fail, returned {value}"),
            Err(Value::String(message)) => message,
            Err(other) => panic!("{cmd} failed with a non-string error {other}"),
        }
    }

    /// Payloads of every `name` event so far.
    pub fn events(&self, name: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(event, _)| event == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    pub fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.path().join("vocab.sqlite3")
    }

    /// A second connection to the database, the way Learn would open it.
    pub fn open_store(&self) -> Store {
        Store::open(self.db_path()).expect("second connection")
    }

    pub fn anki_paths(&self) -> AnkiPaths {
        temp_anki_paths(self.dir.path())
    }
}

fn temp_anki_paths(root: &Path) -> AnkiPaths {
    AnkiPaths {
        anki_base: root.join("Anki2"),
        applications_dir: root.join("Applications"),
        temp_dir: root.join("tmp"),
    }
}

/// `card` (a serialized `Card`) with `field` set to `value`.
pub fn with_field(mut card: Value, field: &str, value: Value) -> Value {
    card[field] = value;
    card
}

/// `bytes` the way a `Card` serializes its audio (base64).
pub fn b64(bytes: &[u8]) -> String {
    vocab_core::model::base64_bytes_opt::serialize(
        &Some(bytes.to_vec()),
        serde_json::value::Serializer,
    )
    .expect("base64")
    .as_str()
    .expect("a string")
    .to_string()
}
