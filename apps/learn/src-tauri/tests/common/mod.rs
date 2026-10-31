//! Shared harness for Learn's `tauri::test` suites.
//!
//! Each [`Harness`] is a mock-runtime app built with the real invoke handler
//! (`vocab_learn_lib::configure`), so commands run exactly as in the app,
//! through the IPC layer. Everything outside the process is replaced: the
//! database is a file in a temp dir, the clock is a `FixedClock` in Berlin
//! winter time (UTC+1, so local and UTC dates disagree for an hour each
//! night), text-to-speech is a fake that records every request, and Anki is
//! unreachable unless a test points the settings at a wiremock AnkiConnect.
//! A guard test (`tests/ipc_review.rs`) fails if the harness ever points at
//! a real location.

#![allow(dead_code)]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use async_trait::async_trait;
use chrono::{DateTime, Duration, FixedOffset, TimeZone, Utc};
use serde_json::{json, Value};
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY},
    webview::InvokeRequest,
    App, Manager, WebviewWindow, WebviewWindowBuilder,
};
use tempfile::TempDir;
use vocab_core::{
    providers::TtsProvider,
    srs::SrsScheduler,
    store::Store,
    time::{Clock, FixedClock},
    CaptureOrigin, Card, CurriculumItem, ProviderError, ScriptMetadata, ScriptVariant,
};
use vocab_learn_lib::{
    settings::{self, LearnSettings},
    state::{AppServices, AppState},
};

/// Where Anki "runs" unless a test starts a mock: nothing listens on port 1,
/// so every request fails at once without leaving the machine.
pub const UNREACHABLE_ANKI: &str = "http://127.0.0.1:1";

/// UTC+1, Berlin in winter.
pub fn berlin() -> FixedOffset {
    FixedOffset::east_opt(3600).unwrap()
}

/// The fixed "now" every harness starts at: 08:00:00.123 on 2026-10-31 in
/// Berlin, i.e. 2026-10-31T07:00:00.123Z.
pub fn start_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 7, 0, 0).unwrap() + Duration::milliseconds(123)
}

/// An instant as the IPC layer serializes it.
pub fn instant(at: DateTime<Utc>) -> Value {
    serde_json::to_value(at).unwrap()
}

/// The audio the fake TTS returns for `text` in `lang`.
pub fn fake_audio(lang: &str, text: &str) -> Vec<u8> {
    format!("mp3:{lang}:{text}").into_bytes()
}

/// `bytes` the way the commands return audio.
pub fn b64(bytes: &[u8]) -> String {
    vocab_learn_lib::audio::encode(bytes)
}

type Hook = Box<dyn FnOnce() + Send>;

/// Records every request as `"LANG TEXT"` and answers with [`fake_audio`],
/// or with an error once [`fail`](Self::fail) was called.
#[derive(Clone, Default)]
pub struct FakeTts {
    calls: Arc<Mutex<Vec<String>>>,
    failing: Arc<AtomicBool>,
    while_speaking: Arc<Mutex<Option<Hook>>>,
}

impl FakeTts {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    pub fn fail(&self) {
        self.failing.store(true, Ordering::SeqCst);
    }

    /// Runs `hook` during the next request, as if the world changed while
    /// the request was on the network.
    pub fn while_speaking(&self, hook: impl FnOnce() + Send + 'static) {
        *self.while_speaking.lock().unwrap() = Some(Box::new(hook));
    }
}

#[async_trait]
impl TtsProvider for FakeTts {
    fn name(&self) -> &'static str {
        "fake_tts"
    }

    async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError> {
        self.calls.lock().unwrap().push(format!("{lang} {text}"));
        let hook = self.while_speaking.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        if self.failing.load(Ordering::SeqCst) {
            return Err(ProviderError::Network {
                provider: "fake_tts".into(),
                message: "offline".into(),
            });
        }
        Ok(fake_audio(lang, text))
    }
}

/// A curriculum word. `example` is the sentence and its English translation.
pub fn word(
    order: i64,
    hanzi: &str,
    pinyin: &str,
    meaning_en: &str,
    meaning_de: Option<&str>,
    example: Option<(&str, &str)>,
) -> CurriculumItem {
    CurriculumItem {
        hanzi: hanzi.into(),
        traditional: None,
        hsk_level: if order < 6 { 1 } else { 2 },
        frequency: Some(order),
        curriculum_order: order,
        pinyin: pinyin.into(),
        pos: None,
        meaning_en: Some(meaning_en.into()),
        meaning_de: meaning_de.map(str::to_string),
        cloze_sentence: None,
        example_sentence_source: example.map(|(sentence, _)| sentence.to_string()),
        example_sentence_pinyin: None,
        example_sentence_target: example.map(|(_, translation)| translation.to_string()),
        vocab_audio_path: None,
        sentence_audio_path: None,
        image_path: None,
    }
}

/// Hanzi, pinyin, English meaning, German meaning, example sentence and its
/// English translation.
type Entry = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static str,
);

/// The fixture curriculum, in `curriculum_order`. Only some words have a
/// German meaning, and most example sentences contain hanzi that are only
/// learned once their own word is.
const WORDS: [Entry; 10] = [
    (
        "我",
        "wǒ",
        "I; me",
        Some("ich"),
        "我是老师。",
        "I am a teacher.",
    ),
    ("你", "nǐ", "you", None, "你好吗？", "How are you?"),
    ("好", "hǎo", "good", Some("gut"), "你好。", "Hello."),
    (
        "学习",
        "xué xí",
        "to study",
        Some("lernen"),
        "我们一起学习中文。",
        "We study Chinese together.",
    ),
    (
        "中文",
        "zhōng wén",
        "Chinese (language)",
        None,
        "我学习中文。",
        "I study Chinese.",
    ),
    (
        "老师",
        "lǎo shī",
        "teacher",
        Some("Lehrer"),
        "他是我的老师。",
        "He is my teacher.",
    ),
    (
        "朋友",
        "péng you",
        "friend",
        None,
        "我们是朋友。",
        "We are friends.",
    ),
    (
        "学生",
        "xué sheng",
        "student",
        None,
        "我是学生。",
        "I am a student.",
    ),
    ("银行", "yín háng", "bank", None, "", ""),
    (
        "喜欢",
        "xǐ huan",
        "to like",
        None,
        "我喜欢中文。",
        "I like Chinese.",
    ),
];

/// [`WORDS`] as curriculum items.
pub fn curriculum() -> Vec<CurriculumItem> {
    WORDS
        .iter()
        .zip(0..)
        .map(|(&(hanzi, pinyin, en, de, sentence, translation), order)| {
            let example = (!sentence.is_empty()).then_some((sentence, translation));
            word(order, hanzi, pinyin, en, de, example)
        })
        .collect()
}

/// The settings every harness starts with: the defaults, with Anki
/// unreachable.
pub fn test_settings() -> LearnSettings {
    let mut settings = LearnSettings::default();
    settings.anki.connect_url = UNREACHABLE_ANKI.to_string();
    settings
}

/// Prepares the database before the app starts.
pub type Prepare = Box<dyn FnOnce(&Store)>;

/// How a [`Harness`] starts.
pub struct Options {
    pub curriculum: Result<Vec<CurriculumItem>, String>,
    /// `false` simulates a database that failed to open.
    pub with_store: bool,
    /// Written into the database before startup.
    pub settings: LearnSettings,
    /// Runs on the database before startup.
    pub before_start: Option<Prepare>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            curriculum: Ok(curriculum()),
            with_store: true,
            settings: test_settings(),
            before_start: None,
        }
    }
}

pub struct Harness {
    pub app: App<MockRuntime>,
    pub main: WebviewWindow<MockRuntime>,
    pub dir: TempDir,
    pub clock: Arc<FixedClock>,
    pub tts: FakeTts,
}

impl Harness {
    pub fn new() -> Self {
        Self::with(Options::default())
    }

    pub fn with(options: Options) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let db_path = dir.path().join("vocab.sqlite3");
        let store = if options.with_store {
            let store = Store::open(&db_path).expect("test database");
            settings::save(&store, &options.settings).expect("initial settings");
            if let Some(prepare) = options.before_start {
                prepare(&store);
            }
            Ok(store)
        } else {
            Err("simulated: the database could not be opened".to_string())
        };
        let clock = Arc::new(FixedClock::new(start_time(), berlin()));
        let tts = FakeTts::default();
        let state = AppState::new(AppServices {
            store,
            clock: clock.clone(),
            tts: Arc::new(tts.clone()),
            curriculum: options.curriculum,
        });
        let app = vocab_learn_lib::configure(mock_builder())
            .manage(state)
            .build(mock_context(noop_assets()))
            .expect("mock app");
        let main = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("main window");
        Self {
            app,
            main,
            dir,
            clock,
            tts,
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

    pub fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.path().join("vocab.sqlite3")
    }

    /// A second connection to the database, the way Automator has it open.
    pub fn open_store(&self) -> Store {
        Store::open(self.db_path()).expect("second connection")
    }

    /// Points the settings at the AnkiConnect at `url`, through
    /// `save_settings`.
    pub fn use_anki(&self, url: &str) {
        let mut settings = self.call("get_settings", json!({}));
        settings["anki"]["connect_url"] = json!(url);
        self.call("save_settings", json!({ "settings": settings }));
    }

    /// Introduces curriculum word `hanzi` and gives it its first rating.
    pub fn learn(&self, hanzi: &str, rating: u8) -> i64 {
        let id = self.call("start_new_item", json!({ "hanzi": hanzi }));
        let id = id.as_i64().expect("a card id");
        self.call(
            "submit_review_rating",
            json!({ "cardId": id, "rating": rating }),
        );
        id
    }
}

/// Stores a clipboard capture the way Automator does, unscheduled; returns
/// its id.
pub fn capture(store: &Store, card: &Card) -> i64 {
    store
        .insert_capture(card, CaptureOrigin::Capture, true)
        .expect("capture")
}

/// A Chinese card as Automator captures it (`zh-CN` -> `de`).
pub fn chinese_capture(front: &str, back: &str, pinyin: &str, example: (&str, &str)) -> Card {
    let mut card = Card::new(
        front,
        back,
        "zh-CN",
        "de",
        start_time() - Duration::days(30),
    );
    card.example_source = example.0.into();
    card.example_target = example.1.into();
    card.script_metadata = ScriptMetadata::Chinese {
        pinyin: pinyin.into(),
        hsk_level: None,
        script_variant: ScriptVariant::Simplified,
    };
    card
}

/// Stores `card` as a learned card (with an `srs_state` row) that is due at
/// `due`; returns its id.
pub fn learned(store: &Store, card: &Card, due: DateTime<Utc>) -> i64 {
    let id = capture(store, card);
    let mut state = SrsScheduler::default().initial_state(id, due);
    // A card reviewed before: in the review state, due at `due`.
    state.fsrs_state = "review".into();
    state.stability = Some(3.0);
    state.difficulty = Some(5.0);
    state.interval_days = 3;
    state.repetitions = 2;
    state.last_reviewed_at = Some(due - Duration::days(3));
    state.last_rating = Some(3);
    store.upsert_srs_state(&state).expect("srs state");
    id
}

/// The local date of `clock`, as the IPC layer serializes dates.
pub fn today(clock: &FixedClock) -> String {
    clock.today().format("%Y-%m-%d").to_string()
}
