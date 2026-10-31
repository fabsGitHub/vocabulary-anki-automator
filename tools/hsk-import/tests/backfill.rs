//! `backfill-de` against a scripted translation provider: request order and
//! content, the per-item save that makes it resumable, and how each kind of
//! provider failure ends (or does not end) a run. No network.

use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Mutex,
    time::Duration,
};

use hsk_import::{
    backfill::{backfill_de, BackfillOptions, Stop},
    output::{read_overrides, write_overrides, Overrides},
};
use vocab_core::{providers::TranslationProvider, CurriculumItem, ProviderError};

type Reply = Box<dyn Fn(usize, &str) -> Result<String, ProviderError> + Send + Sync>;

/// Answers request `n` (0-based) with `reply(n, text)`, and records each
/// request together with how many entries the overrides file held when it
/// arrived.
struct Scripted {
    overrides: PathBuf,
    reply: Reply,
    requests: Mutex<Vec<(String, String, String, usize)>>,
}

impl Scripted {
    fn new(
        overrides: &Path,
        reply: impl Fn(usize, &str) -> Result<String, ProviderError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            overrides: overrides.to_path_buf(),
            reply: Box::new(reply),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn respond(&self, text: &str, source: &str, target: &str) -> Result<String, ProviderError> {
        let saved = read_overrides(&self.overrides)
            .unwrap()
            .map_or(0, |file| file.len());
        let mut requests = self.requests.lock().unwrap();
        let n = requests.len();
        requests.push((text.into(), source.into(), target.into(), saved));
        (self.reply)(n, text)
    }

    fn texts(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(text, ..)| text.clone())
            .collect()
    }
}

/// `TranslationProvider` is declared with `#[async_trait]`, which this crate
/// does not depend on, so the fake implements the signature the macro
/// expands to and returns an already finished future.
impl TranslationProvider for Scripted {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn translate<'life0, 'life1, 'life2, 'life3, 'async_trait>(
        &'life0 self,
        text: &'life1 str,
        source: &'life2 str,
        target: &'life3 str,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        'life2: 'async_trait,
        'life3: 'async_trait,
        Self: 'async_trait,
    {
        let result = self.respond(text, source, target);
        Box::pin(async move { result })
    }
}

fn german(_: usize, text: &str) -> Result<String, ProviderError> {
    Ok(format!("DE[{text}]"))
}

fn item(hanzi: &str, order: i64, meaning_en: Option<&str>) -> CurriculumItem {
    CurriculumItem {
        hanzi: hanzi.into(),
        traditional: None,
        hsk_level: 1,
        frequency: Some(order),
        curriculum_order: order,
        pinyin: "x".into(),
        pos: None,
        meaning_en: meaning_en.map(str::to_string),
        meaning_de: None,
        cloze_sentence: None,
        example_sentence_source: None,
        example_sentence_pinyin: None,
        example_sentence_target: None,
        vocab_audio_path: None,
        sentence_audio_path: None,
        image_path: None,
    }
}

/// Deliberately not in curriculum order.
fn curriculum() -> Vec<CurriculumItem> {
    vec![
        item("我", 2, Some("I; me; my")),
        item("的", 0, Some("of; ~'s (possessive particle)")),
        item(
            "吧",
            3,
            Some("(modal particle indicating suggestion; or surmise); ...right?"),
        ),
        item("了", 1, Some("(completed action marker)")),
    ]
}

fn no_delay() -> BackfillOptions {
    BackfillOptions {
        limit: None,
        delay: Duration::ZERO,
    }
}

fn quota() -> ProviderError {
    ProviderError::QuotaExceeded {
        provider: "mymemory".into(),
        message: "NEXT AVAILABLE IN 23 HOURS".into(),
    }
}

fn overrides_of(pairs: &[(&str, &str)]) -> Overrides {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[tokio::test]
async fn translates_first_senses_in_curriculum_order_and_saves_after_each() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("de.json");
    let provider = Scripted::new(&path, german);

    let report = backfill_de(&provider, &curriculum(), &path, &no_delay())
        .await
        .unwrap();

    assert_eq!(report.stop, Stop::Completed);
    let requests = provider.requests.lock().unwrap().clone();
    assert_eq!(
        requests,
        vec![
            ("of".into(), "en".into(), "de".into(), 0),
            (
                "(completed action marker)".into(),
                "en".into(),
                "de".into(),
                1
            ),
            ("I".into(), "en".into(), "de".into(), 2),
            (
                "(modal particle indicating suggestion; or surmise)".into(),
                "en".into(),
                "de".into(),
                3
            ),
        ],
        "first sense only, en -> de, curriculum order, file saved before the next request"
    );
    assert_eq!(
        read_overrides(&path).unwrap().unwrap(),
        overrides_of(&[
            ("的", "DE[of]"),
            ("了", "DE[(completed action marker)]"),
            ("我", "DE[I]"),
            (
                "吧",
                "DE[(modal particle indicating suggestion; or surmise)]"
            ),
        ])
    );
    assert_eq!(report.translated.len(), 4);
    assert_eq!((report.remaining, report.already_done), (0, 0));
}

#[tokio::test]
async fn quota_stops_cleanly_and_a_rerun_resumes_where_it_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("de.json");
    let first = Scripted::new(&path, |n, text| {
        if n == 2 {
            Err(quota())
        } else {
            german(n, text)
        }
    });

    let report = backfill_de(&first, &curriculum(), &path, &no_delay())
        .await
        .unwrap();

    assert_eq!(
        report.stop,
        Stop::QuotaExceeded("NEXT AVAILABLE IN 23 HOURS".into())
    );
    assert_eq!(
        first.texts().len(),
        3,
        "nothing is requested after the quota error"
    );
    assert_eq!(report.remaining, 2);
    assert_eq!(
        read_overrides(&path).unwrap().unwrap(),
        overrides_of(&[("的", "DE[of]"), ("了", "DE[(completed action marker)]")])
    );

    let second = Scripted::new(&path, german);
    let report = backfill_de(&second, &curriculum(), &path, &no_delay())
        .await
        .unwrap();
    assert_eq!(report.stop, Stop::Completed);
    assert_eq!(
        second.texts(),
        ["I", "(modal particle indicating suggestion; or surmise)"]
    );
    assert_eq!((report.already_done, report.remaining), (2, 0));
    assert_eq!(read_overrides(&path).unwrap().unwrap().len(), 4);
}

#[tokio::test]
async fn the_limit_caps_requests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("de.json");
    let provider = Scripted::new(&path, german);
    let options = BackfillOptions {
        limit: Some(2),
        ..no_delay()
    };

    let report = backfill_de(&provider, &curriculum(), &path, &options)
        .await
        .unwrap();

    assert_eq!(report.stop, Stop::LimitReached);
    assert_eq!(provider.texts(), ["of", "(completed action marker)"]);
    assert_eq!(report.remaining, 2);

    let zero = Scripted::new(&path, german);
    let options = BackfillOptions {
        limit: Some(0),
        ..no_delay()
    };
    let report = backfill_de(&zero, &curriculum(), &path, &options)
        .await
        .unwrap();
    assert_eq!(report.stop, Stop::LimitReached);
    assert!(zero.texts().is_empty());
}

#[tokio::test]
async fn item_failures_are_skipped_and_retried_by_the_next_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("de.json");
    let provider = Scripted::new(&path, |n, text| match n {
        0 => Err(ProviderError::NotFound {
            provider: "mymemory".into(),
        }),
        2 => Err(ProviderError::Malformed {
            provider: "mymemory".into(),
            message: "no translatedText".into(),
        }),
        _ => german(n, text),
    });

    let report = backfill_de(&provider, &curriculum(), &path, &no_delay())
        .await
        .unwrap();

    assert_eq!(report.stop, Stop::Completed);
    assert_eq!(
        provider.texts().len(),
        4,
        "a failed item does not end the run"
    );
    let skipped: Vec<&str> = report.skipped.iter().map(|(h, _)| h.as_str()).collect();
    assert_eq!(skipped, ["的", "我"]);
    assert_eq!(report.remaining, 2);
    let saved = read_overrides(&path).unwrap().unwrap();
    assert_eq!(saved.keys().collect::<Vec<_>>(), ["了", "吧"]);

    let retry = Scripted::new(&path, german);
    backfill_de(&retry, &curriculum(), &path, &no_delay())
        .await
        .unwrap();
    assert_eq!(retry.texts(), ["of", "I"]);
}

#[tokio::test]
async fn network_failures_stop_the_run_and_keep_the_progress() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("de.json");
    let down = ProviderError::Network {
        provider: "mymemory".into(),
        message: "connection refused".into(),
    };
    let expected = down.clone();
    let provider = Scripted::new(&path, move |n, text| {
        if n == 1 {
            Err(down.clone())
        } else {
            german(n, text)
        }
    });

    let report = backfill_de(&provider, &curriculum(), &path, &no_delay())
        .await
        .unwrap();

    assert_eq!(report.stop, Stop::Failed(expected));
    assert_eq!(provider.texts().len(), 2);
    assert_eq!(
        read_overrides(&path).unwrap().unwrap(),
        overrides_of(&[("的", "DE[of]")])
    );
}

#[tokio::test]
async fn items_with_german_or_without_english_are_not_requested() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("de.json");
    write_overrides(&path, &overrides_of(&[("了", "(abgeschlossen)")])).unwrap();
    let mut items = curriculum();
    items[0].meaning_de = Some("ich".into()); // 我
    items[2].meaning_en = Some("  ".into()); // 吧
    items.push(item("么", 4, None));
    let provider = Scripted::new(&path, german);

    let report = backfill_de(&provider, &items, &path, &no_delay())
        .await
        .unwrap();

    assert_eq!(provider.texts(), ["of"]);
    assert_eq!(report.already_done, 2);
    assert_eq!(report.untranslatable, 2);
    assert_eq!(report.remaining, 0);
    let saved = read_overrides(&path).unwrap().unwrap();
    assert_eq!(saved.get("了").map(String::as_str), Some("(abgeschlossen)"));
    assert_eq!(saved.get("的").map(String::as_str), Some("DE[of]"));
}
