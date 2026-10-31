//! Milestone G+ "Chinese clipboard capture end-to-end": hanzi captured
//! under an English configuration go through the production zh-CN provider
//! set over the real bundled curriculum, and the card is persisted and read
//! back with its script metadata.
//!
//! Only the two network providers (translation and TTS) are fakes. The zh-CN
//! set is taken from `ProviderRegistry::standard` itself, so its
//! `CurriculumExampleProvider`, `PinyinAnnotator` and missing rarity step are
//! the ones the apps run; constructing it opens no connection.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use vocab_core::{
    curriculum::CurriculumIndex,
    pipeline::{CaptureOutcome, CapturePipeline, CaptureRequest},
    providers::{
        chinese::generate_pinyin, CurriculumLookup, ExampleProvider, ProviderRegistry, ProviderSet,
        RarityProvider, RarityResult, StandardProviders, TranslationProvider, TtsProvider,
    },
    store::Store,
    CaptureOrigin, ProviderError, ScriptMetadata, ScriptVariant,
};

const MP3: &[u8] = &[0xFF, 0xFB, 0x90, 0x44];

/// The zh-CN example fallback (docs/architecture.md § 5).
fn fallback_sentence(word: &str) -> String {
    format!("我们在日常生活中使用“{word}”这个词。")
}

#[derive(Clone, Default)]
struct CallLog(Arc<Mutex<Vec<String>>>);

impl CallLog {
    fn push(&self, call: String) {
        self.0.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Stands in for MyMemory: `DE(text)`.
struct FakeTranslation(CallLog);
/// Stands in for Google TTS.
struct FakeTts(CallLog);
/// Every English-only provider: records the call and fails, so a capture
/// that reached it would show up in the log and in the warnings.
struct EnglishOnly(CallLog);

#[async_trait]
impl TranslationProvider for FakeTranslation {
    fn name(&self) -> &'static str {
        "fake_mymemory"
    }

    async fn translate(
        &self,
        text: &str,
        source: &str,
        target: &str,
    ) -> Result<String, ProviderError> {
        self.0.push(format!("translate {text} {source}->{target}"));
        Ok(format!("DE({text})"))
    }
}

#[async_trait]
impl TtsProvider for FakeTts {
    fn name(&self) -> &'static str {
        "fake_google_tts"
    }

    async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError> {
        self.0.push(format!("tts {text} {lang}"));
        Ok(MP3.to_vec())
    }
}

fn english_only_error() -> ProviderError {
    ProviderError::Unsupported {
        provider: "english_only".into(),
        what: "anything but English".into(),
    }
}

#[async_trait]
impl RarityProvider for EnglishOnly {
    fn name(&self) -> &'static str {
        "datamuse"
    }

    async fn analyze(&self, word: &str) -> Result<RarityResult, ProviderError> {
        self.0.push(format!("rarity {word}"));
        Err(english_only_error())
    }
}

#[async_trait]
impl ExampleProvider for EnglishOnly {
    fn name(&self) -> &'static str {
        "dictionary_api"
    }

    async fn fetch_example(&self, word: &str) -> Result<String, ProviderError> {
        self.0.push(format!("example {word}"));
        Err(english_only_error())
    }
}

#[async_trait]
impl TranslationProvider for EnglishOnly {
    fn name(&self) -> &'static str {
        "english_translation"
    }

    async fn translate(&self, text: &str, _: &str, _: &str) -> Result<String, ProviderError> {
        self.0.push(format!("english translate {text}"));
        Err(english_only_error())
    }
}

struct Capture {
    pipeline: CapturePipeline,
    curriculum: Arc<CurriculumIndex>,
    /// Calls to the zh-CN fakes.
    chinese: CallLog,
    /// Calls to anything outside the zh-CN set.
    elsewhere: CallLog,
}

/// The production registry's zh-CN set with fake translation and TTS; the
/// English and default sets are made of [`EnglishOnly`] tripwires.
fn capture() -> Capture {
    let curriculum = Arc::new(CurriculumIndex::bundled());
    let lookup: Arc<dyn CurriculumLookup> = curriculum.clone();
    let standard = ProviderRegistry::standard(StandardProviders {
        contact_email: None,
        curriculum: lookup,
    });
    let production = standard.resolve("zh-CN").clone();
    assert!(
        production.rarity.is_none(),
        "zh-CN has no rarity step by design"
    );
    assert_eq!(
        production.example.as_ref().map(|p| p.name()),
        Some("curriculum")
    );
    assert_eq!(
        production.annotator.as_ref().map(|p| p.name()),
        Some("pinyin")
    );

    let chinese = CallLog::default();
    let elsewhere = CallLog::default();
    let zh = ProviderSet {
        translation: Arc::new(FakeTranslation(chinese.clone())),
        tts: Some(Arc::new(FakeTts(chinese.clone()))),
        ..production
    };
    let tripwires = || {
        let english = Arc::new(EnglishOnly(elsewhere.clone()));
        ProviderSet {
            translation: english.clone(),
            rarity: Some(english.clone()),
            example: Some(english),
            tts: None,
            annotator: None,
        }
    };
    let registry = ProviderRegistry::new(tripwires())
        .with_language("en", tripwires())
        .with_language("zh-CN", zh);
    Capture {
        pipeline: CapturePipeline::new(registry, 100),
        curriculum,
        chinese,
        elsewhere,
    }
}

fn captured_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap()
}

async fn run(capture: &Capture, text: &str) -> CaptureOutcome {
    capture
        .pipeline
        .run(CaptureRequest {
            text: text.into(),
            source_lang: "en".into(),
            target_lang: "de".into(),
            captured_at: captured_at(),
            tags: vec!["clipboard".into()],
        })
        .await
        .expect("a valid capture")
}

/// `(script_kind, pinyin, hsk_level, script_variant)` exactly as stored.
fn stored_script_columns(
    db: &std::path::Path,
    id: i64,
) -> (String, Option<String>, Option<i64>, Option<String>) {
    Connection::open(db)
        .unwrap()
        .query_row(
            "SELECT script_kind, pinyin, hsk_level, script_variant FROM captures WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap()
}

#[tokio::test]
async fn hanzi_under_an_english_config_become_a_chinese_card_from_the_curriculum() {
    let capture = capture();
    let item = capture
        .curriculum
        .lookup("学习")
        .expect("学习 is an HSK word");
    assert_eq!((item.pinyin.as_str(), item.hsk_level), ("xué xí", 1));
    assert_eq!(
        item.example_sentence_source, None,
        "precondition: the bundled dataset has no example sentences"
    );

    let outcome = run(&capture, " 学习 ").await;

    assert_eq!(outcome.warnings, vec![]);
    let card = &outcome.card;
    assert_eq!(
        (card.source_lang.as_str(), card.target_lang.as_str()),
        ("zh-CN", "de"),
        "CJK text is routed to zh-CN whatever the configured source"
    );
    assert_eq!(card.front, "学习");
    assert_eq!(card.back, "DE(学习)");
    assert_eq!(card.notes, "", "no rarity verdict for Chinese");
    assert_eq!(
        card.script_metadata,
        ScriptMetadata::Chinese {
            pinyin: item.pinyin.clone(),
            hsk_level: Some(item.hsk_level),
            script_variant: ScriptVariant::Simplified,
        },
        "pinyin and level come from the curriculum"
    );
    assert_eq!(card.example_source, fallback_sentence("学习"));
    assert_eq!(
        card.example_target,
        format!("DE({})", fallback_sentence("学习"))
    );
    assert_eq!(card.audio_front.as_deref(), Some(MP3));
    assert_eq!(card.tags, ["clipboard"]);
    assert_eq!(
        capture.chinese.calls(),
        [
            "translate 学习 zh-CN->de".to_string(),
            format!("translate {} zh-CN->de", fallback_sentence("学习")),
            "tts 学习 zh-CN".to_string(),
        ]
    );
    assert_eq!(
        capture.elsewhere.calls(),
        Vec::<String>::new(),
        "no English-only provider (Datamuse rarity, dictionaryapi examples) ran"
    );

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("vocab.sqlite3");
    let id = {
        let store = Store::open(&db).unwrap();
        store
            .insert_capture(card, CaptureOrigin::Capture, false)
            .unwrap()
    };
    assert_eq!(
        stored_script_columns(&db, id),
        (
            "chinese".to_string(),
            Some("xué xí".to_string()),
            Some(1),
            Some("simplified".to_string())
        )
    );

    // Read back through a new connection, as the other app would.
    let store = Store::open(&db).unwrap();
    let stored = store.get_capture(id).unwrap().expect("the capture exists");
    assert_eq!(stored.card, *card);
    assert_eq!(stored.card.script_metadata.kind(), "chinese");
    assert_eq!(stored.card.pinyin(), Some("xué xí"));
    assert_eq!(stored.card.hsk_level(), Some(1));
    assert_eq!(
        (stored.origin, stored.synced_to_anki),
        (CaptureOrigin::Capture, false)
    );
    assert_eq!(
        store
            .find_capture_by_front("学习", "chinese")
            .unwrap()
            .map(|c| c.id),
        Some(id)
    );
    assert!(store.chinese_capture_fronts().unwrap().contains("学习"));
}

/// 学习's curriculum pinyin happens to equal the generated one; for a
/// heteronym it does not, which shows where the card's pinyin came from.
#[tokio::test]
async fn curriculum_pinyin_beats_the_per_character_reading_of_a_heteronym() {
    let capture = capture();
    assert_eq!(
        generate_pinyin("银行"),
        "yín xíng",
        "precondition: per character, 行 reads xíng"
    );

    let outcome = run(&capture, "银行").await;

    assert_eq!(outcome.warnings, vec![]);
    assert_eq!(outcome.card.pinyin(), Some("yín háng"));
    assert_eq!(outcome.card.hsk_level(), Some(2));
    assert_eq!(capture.elsewhere.calls(), Vec::<String>::new());
}

#[tokio::test]
async fn hanzi_outside_the_curriculum_get_generated_pinyin_and_no_level() {
    let capture = capture();
    assert!(
        capture.curriculum.lookup("饕餮").is_none(),
        "precondition: 饕餮 is not an HSK word"
    );

    let outcome = run(&capture, "饕餮").await;

    assert_eq!(outcome.warnings, vec![]);
    let card = &outcome.card;
    assert_eq!(card.source_lang, "zh-CN");
    assert_eq!(generate_pinyin("饕餮"), "tāo tiè");
    assert_eq!(
        card.script_metadata,
        ScriptMetadata::Chinese {
            pinyin: "tāo tiè".into(),
            hsk_level: None,
            script_variant: ScriptVariant::Simplified,
        }
    );
    assert_eq!(card.example_source, fallback_sentence("饕餮"));
    assert_eq!(capture.elsewhere.calls(), Vec::<String>::new());

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("vocab.sqlite3");
    let store = Store::open(&db).unwrap();
    let id = store
        .insert_capture(card, CaptureOrigin::Capture, false)
        .unwrap();
    assert_eq!(
        stored_script_columns(&db, id),
        (
            "chinese".to_string(),
            Some("tāo tiè".to_string()),
            None,
            Some("simplified".to_string())
        )
    );
    let stored = store.get_capture(id).unwrap().unwrap();
    assert_eq!(stored.card, *card);
    assert_eq!(stored.card.hsk_level(), None);
}
