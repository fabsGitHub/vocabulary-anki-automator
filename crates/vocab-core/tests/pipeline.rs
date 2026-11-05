//! Capture-pipeline semantics (docs/architecture.md § 5) against recording
//! fakes of every provider trait, plus the real Chinese providers over an
//! in-memory curriculum. Nothing here touches the network.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use vocab_core::{
    curriculum::CurriculumIndex,
    pipeline::{CapturePipeline, CaptureRequest, PipelineError, PipelineStage, PipelineWarning},
    providers::{
        CurriculumExampleProvider, ExampleProvider, PinyinAnnotator, ProviderRegistry, ProviderSet,
        RarityProvider, RarityResult, ScriptAnnotator, TranslationProvider, TtsProvider,
    },
    CurriculumItem, ProviderError, ScriptMetadata, ScriptVariant,
};

const MP3: &[u8] = &[0xFF, 0xF3, 0x44, 0xC4];

/// Every provider call in order, e.g. `"translate serendipity en->de"`.
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

/// Fakes answer from a plain function of their input, so each test states its
/// scenario in one line.
type Reply<T> = fn(&str) -> Result<T, ProviderError>;

struct FakeTranslation(CallLog, Reply<String>);
struct FakeRarity(CallLog, Reply<RarityResult>);
struct FakeExample(CallLog, Reply<String>);
struct FakeTts(CallLog, Reply<Vec<u8>>);
struct FakeAnnotator(CallLog, Reply<ScriptMetadata>);

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
        self.0.push(format!("translate {text} {source}->{target}"));
        (self.1)(text)
    }
}

#[async_trait]
impl RarityProvider for FakeRarity {
    fn name(&self) -> &'static str {
        "fake_rarity"
    }

    async fn analyze(&self, word: &str) -> Result<RarityResult, ProviderError> {
        self.0.push(format!("rarity {word}"));
        (self.1)(word)
    }
}

#[async_trait]
impl ExampleProvider for FakeExample {
    fn name(&self) -> &'static str {
        "fake_example"
    }

    async fn fetch_example(&self, word: &str) -> Result<String, ProviderError> {
        self.0.push(format!("example {word}"));
        (self.1)(word)
    }
}

#[async_trait]
impl TtsProvider for FakeTts {
    fn name(&self) -> &'static str {
        "fake_tts"
    }

    async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError> {
        self.0.push(format!("tts {text} {lang}"));
        (self.1)(text)
    }
}

#[async_trait]
impl ScriptAnnotator for FakeAnnotator {
    fn name(&self) -> &'static str {
        "fake_annotator"
    }

    async fn annotate(&self, text: &str) -> Result<ScriptMetadata, ProviderError> {
        self.0.push(format!("annotate {text}"));
        (self.1)(text)
    }
}

/// Builds fakes that all record into one shared [`CallLog`].
#[derive(Default)]
struct Fakes {
    log: CallLog,
}

impl Fakes {
    fn translation(&self, reply: Reply<String>) -> Arc<dyn TranslationProvider> {
        Arc::new(FakeTranslation(self.log.clone(), reply))
    }

    fn rarity(&self, reply: Reply<RarityResult>) -> Option<Arc<dyn RarityProvider>> {
        Some(Arc::new(FakeRarity(self.log.clone(), reply)))
    }

    fn example(&self, reply: Reply<String>) -> Option<Arc<dyn ExampleProvider>> {
        Some(Arc::new(FakeExample(self.log.clone(), reply)))
    }

    fn tts(&self, reply: Reply<Vec<u8>>) -> Option<Arc<dyn TtsProvider>> {
        Some(Arc::new(FakeTts(self.log.clone(), reply)))
    }

    fn annotator(&self, reply: Reply<ScriptMetadata>) -> Option<Arc<dyn ScriptAnnotator>> {
        Some(Arc::new(FakeAnnotator(self.log.clone(), reply)))
    }

    /// Every step present and succeeding.
    fn full_set(&self) -> ProviderSet {
        ProviderSet {
            translation: self.translation(bracketed),
            rarity: self.rarity(rare_with_synonym),
            example: self.example(pure_example),
            tts: self.tts(audio),
            annotator: self.annotator(annotation),
        }
    }

    /// A set whose every provider panics: proves a capture never reached it.
    fn untouchable_set(&self) -> ProviderSet {
        ProviderSet {
            translation: self.translation(|text| panic!("unexpected translate({text:?})")),
            rarity: self.rarity(|text| panic!("unexpected rarity({text:?})")),
            example: self.example(|text| panic!("unexpected example({text:?})")),
            tts: self.tts(|text| panic!("unexpected tts({text:?})")),
            annotator: self.annotator(|text| panic!("unexpected annotate({text:?})")),
        }
    }

    /// `en` gets `english`; everything else hits the untouchable default.
    fn english_pipeline(&self, english: ProviderSet) -> CapturePipeline {
        CapturePipeline::new(
            ProviderRegistry::new(self.untouchable_set()).with_language("en", english),
            100,
        )
    }
}

fn bracketed(text: &str) -> Result<String, ProviderError> {
    Ok(format!("<{text}>"))
}

fn rare_with_synonym(_: &str) -> Result<RarityResult, ProviderError> {
    Ok(RarityResult {
        is_rare: true,
        frequency_per_million: Some(0.29),
        synonym: Some("luck".into()),
    })
}

fn pure_example(word: &str) -> Result<String, ProviderError> {
    Ok(format!("It was pure {word}."))
}

fn audio(_: &str) -> Result<Vec<u8>, ProviderError> {
    Ok(MP3.to_vec())
}

fn annotation(_: &str) -> Result<ScriptMetadata, ProviderError> {
    Ok(ScriptMetadata::Chinese {
        pinyin: "fake".into(),
        hsk_level: Some(2),
        script_variant: ScriptVariant::Simplified,
    })
}

fn network(provider: &str) -> ProviderError {
    ProviderError::Network {
        provider: provider.into(),
        message: "connection refused".into(),
    }
}

fn warning(stage: PipelineStage, error: ProviderError) -> PipelineWarning {
    PipelineWarning { stage, error }
}

fn captured_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap()
}

fn request(text: &str, source_lang: &str, target_lang: &str) -> CaptureRequest {
    CaptureRequest {
        text: text.into(),
        source_lang: source_lang.into(),
        target_lang: target_lang.into(),
        captured_at: captured_at(),
        tags: vec!["vocab".into(), "reading".into()],
    }
}

fn hsk_item(hanzi: &str, pinyin: &str, level: u8, example: Option<&str>) -> CurriculumItem {
    CurriculumItem {
        hanzi: hanzi.into(),
        traditional: None,
        hsk_level: level,
        frequency: None,
        curriculum_order: 1,
        pinyin: pinyin.into(),
        pos: None,
        meaning_en: Some("to study".into()),
        meaning_de: None,
        cloze_sentence: None,
        example_sentence_source: example.map(str::to_string),
        example_sentence_pinyin: None,
        example_sentence_target: None,
        vocab_audio_path: None,
        sentence_audio_path: None,
        image_path: None,
    }
}

/// The production zh-CN wiring with fakes in place of the network providers.
fn chinese_set(fakes: &Fakes, curriculum: Vec<CurriculumItem>) -> ProviderSet {
    let curriculum = Arc::new(CurriculumIndex::new(curriculum));
    ProviderSet {
        translation: fakes.translation(bracketed),
        rarity: None,
        example: Some(Arc::new(CurriculumExampleProvider::new(curriculum.clone()))),
        tts: fakes.tts(audio),
        annotator: Some(Arc::new(PinyinAnnotator::new(curriculum))),
    }
}

#[tokio::test]
async fn happy_path_populates_every_card_field() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(fakes.full_set());

    let outcome = pipeline
        .run(request("  serendipity \n", "EN", "DE"))
        .await
        .unwrap();

    assert_eq!(outcome.warnings, vec![]);
    let card = outcome.card;
    assert_eq!(card.front, "serendipity");
    assert_eq!(card.back, "<serendipity>");
    assert_eq!(card.notes, "Rare word. More common synonym: luck");
    assert_eq!(card.example_source, "It was pure serendipity.");
    assert_eq!(card.example_target, "<It was pure serendipity.>");
    assert_eq!(card.audio_front.as_deref(), Some(MP3));
    assert_eq!(card.script_metadata, annotation("").unwrap());
    assert_eq!(card.tags, vec!["vocab", "reading"]);
    assert_eq!(card.captured_at, captured_at());
    assert_eq!(
        (card.source_lang.as_str(), card.target_lang.as_str()),
        ("en", "de")
    );
    assert_eq!(
        fakes.log.calls(),
        vec![
            "translate serendipity en->de",
            "rarity serendipity",
            "example serendipity",
            "translate It was pure serendipity. en->de",
            "tts serendipity en",
            "annotate serendipity",
        ]
    );
}

#[tokio::test]
async fn translation_failure_leaves_back_empty_and_warns() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        translation: fakes.translation(|text| match text {
            "serendipity" => Err(network("fake_translation")),
            other => bracketed(other),
        }),
        ..fakes.full_set()
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert_eq!(
        outcome.warnings,
        vec![warning(
            PipelineStage::Translate,
            network("fake_translation")
        )]
    );
    assert_eq!(outcome.card.back, "");
    assert_eq!(outcome.card.example_target, "<It was pure serendipity.>");
    assert_eq!(outcome.card.audio_front.as_deref(), Some(MP3));
}

#[tokio::test]
async fn quota_errors_reach_the_warnings_as_quota_exceeded() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        translation: fakes.translation(|_| {
            Err(ProviderError::QuotaExceeded {
                provider: "mymemory".into(),
                message: "NEXT AVAILABLE IN 10 HOURS".into(),
            })
        }),
        ..fakes.full_set()
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert!(outcome.warnings.iter().all(|w| w.error.is_quota_exceeded()));
    let quota = serde_json::json!({
        "kind": "quota_exceeded",
        "provider": "mymemory",
        "message": "NEXT AVAILABLE IN 10 HOURS"
    });
    assert_eq!(
        serde_json::to_value(&outcome.warnings).unwrap(),
        serde_json::json!([
            { "stage": "translate", "error": quota },
            { "stage": "example_translate", "error": quota },
        ])
    );
    assert_eq!(outcome.card.back, "");
    assert_eq!(outcome.card.example_source, "It was pure serendipity.");
    assert_eq!(outcome.card.example_target, "");
}

#[tokio::test]
async fn phrases_skip_rarity_but_are_translated() {
    for phrase in ["ice cream", "kick\tthe bucket", "break\na leg"] {
        let fakes = Fakes::default();
        let pipeline = fakes.english_pipeline(ProviderSet {
            rarity: fakes.rarity(|text| panic!("rarity must not run for phrases: {text:?}")),
            ..fakes.full_set()
        });

        let outcome = pipeline.run(request(phrase, "en", "de")).await.unwrap();

        assert_eq!(outcome.warnings, vec![], "{phrase:?}");
        assert_eq!(outcome.card.back, format!("<{phrase}>"));
        assert_eq!(outcome.card.notes, "");
        assert!(
            !fakes
                .log
                .calls()
                .iter()
                .any(|call| call.starts_with("rarity")),
            "{phrase:?}"
        );
        assert_eq!(fakes.log.calls()[0], format!("translate {phrase} en->de"));
    }
}

#[tokio::test]
async fn rarity_verdicts_become_notes() {
    let cases: [(Reply<RarityResult>, &str, Vec<PipelineWarning>); 4] = [
        (
            rare_with_synonym,
            "Rare word. More common synonym: luck",
            vec![],
        ),
        (
            |_| {
                Ok(RarityResult {
                    is_rare: true,
                    frequency_per_million: Some(1.0),
                    synonym: None,
                })
            },
            "Rare word.",
            vec![],
        ),
        (
            |_| {
                Ok(RarityResult {
                    is_rare: false,
                    frequency_per_million: Some(300.0),
                    synonym: None,
                })
            },
            "",
            vec![],
        ),
        (
            |_| Err(network("fake_rarity")),
            "",
            vec![warning(PipelineStage::Rarity, network("fake_rarity"))],
        ),
    ];
    for (reply, notes, warnings) in cases {
        let fakes = Fakes::default();
        let pipeline = fakes.english_pipeline(ProviderSet {
            rarity: fakes.rarity(reply),
            ..fakes.full_set()
        });

        let outcome = pipeline
            .run(request("serendipity", "en", "de"))
            .await
            .unwrap();

        assert_eq!(outcome.card.notes, notes);
        assert_eq!(outcome.warnings, warnings);
    }
}

#[tokio::test]
async fn cjk_text_is_routed_to_the_chinese_set_despite_an_english_config() {
    let fakes = Fakes::default();
    let registry = ProviderRegistry::new(fakes.untouchable_set())
        .with_language("en", fakes.untouchable_set())
        .with_language(
            "zh-CN",
            chinese_set(
                &fakes,
                vec![hsk_item("学习", "xué xí", 1, Some("我们一起学习中文。"))],
            ),
        );
    let pipeline = CapturePipeline::new(registry, 100);

    let outcome = pipeline.run(request(" 学习 ", "en", "de")).await.unwrap();

    assert_eq!(outcome.warnings, vec![]);
    let card = outcome.card;
    assert_eq!(
        (card.source_lang.as_str(), card.target_lang.as_str()),
        ("zh-CN", "de")
    );
    assert_eq!(card.front, "学习");
    assert_eq!(card.back, "<学习>");
    assert_eq!(card.notes, "");
    assert_eq!(card.example_source, "我们一起学习中文。");
    assert_eq!(card.example_target, "<我们一起学习中文。>");
    assert_eq!(
        card.script_metadata,
        ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        }
    );
    assert_eq!(card.pinyin(), Some("xué xí"));
    assert_eq!(card.hsk_level(), Some(1));
    assert_eq!(
        fakes.log.calls(),
        vec![
            "translate 学习 zh-CN->de",
            "translate 我们一起学习中文。 zh-CN->de",
            "tts 学习 zh-CN",
        ]
    );
}

/// An en -> zh-CN learner who copies Chinese text: sniffing makes the source
/// zh-CN, which equals the target, so the configured pair is reversed.
#[tokio::test]
async fn sniffing_into_the_target_language_reverses_the_pair() {
    let fakes = Fakes::default();
    let registry = ProviderRegistry::new(fakes.untouchable_set())
        .with_language("en", fakes.untouchable_set())
        .with_language(
            "zh-CN",
            chinese_set(&fakes, vec![hsk_item("学习", "xué xí", 1, None)]),
        );
    let pipeline = CapturePipeline::new(registry, 100);

    let outcome = pipeline.run(request("学习", "EN", "zh-cn")).await.unwrap();

    let card = &outcome.card;
    assert_eq!(
        (card.source_lang.as_str(), card.target_lang.as_str()),
        ("zh-CN", "en")
    );
    assert_eq!(card.back, "<学习>");
    assert_eq!(fakes.log.calls()[0], "translate 学习 zh-CN->en");
    assert!(
        outcome
            .warnings
            .iter()
            .all(|w| w.stage != PipelineStage::Translate),
        "{:?}",
        outcome.warnings
    );
}

#[tokio::test]
async fn example_not_found_uses_the_language_fallback() {
    let fakes = Fakes::default();
    let english = ProviderSet {
        example: fakes.example(|_| {
            Err(ProviderError::NotFound {
                provider: "fake_example".into(),
            })
        }),
        ..fakes.full_set()
    };
    let registry = ProviderRegistry::new(fakes.untouchable_set())
        .with_language("en", english)
        .with_language("zh-CN", chinese_set(&fakes, vec![]));
    let pipeline = CapturePipeline::new(registry, 100);

    let en = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();
    assert_eq!(en.warnings, vec![]);
    assert_eq!(
        en.card.example_source,
        "We use the expression 'serendipity' in everyday context."
    );
    assert_eq!(
        en.card.example_target,
        "<We use the expression 'serendipity' in everyday context.>"
    );

    // Outside the curriculum: fallback sentence and generated pinyin, no level.
    let zh = pipeline.run(request("银行", "zh-CN", "en")).await.unwrap();
    assert_eq!(zh.warnings, vec![]);
    assert_eq!(zh.card.example_source, "我们在日常生活中使用“银行”这个词。");
    assert_eq!(
        zh.card.example_target,
        "<我们在日常生活中使用“银行”这个词。>"
    );
    assert_eq!(zh.card.pinyin(), Some("yín xíng"));
    assert_eq!(zh.card.hsk_level(), None);
}

/// § 5: an empty `Ok` example counts as `NotFound`, a blank one too, so the
/// card gets the fallback sentence rather than no sentence, and no warning.
#[tokio::test]
async fn a_blank_example_counts_as_not_found() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        example: fakes.example(|_| Ok(" \n ".into())),
        ..fakes.full_set()
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert_eq!(outcome.warnings, vec![]);
    assert_eq!(
        outcome.card.example_source,
        "We use the expression 'serendipity' in everyday context."
    );
}

#[tokio::test]
async fn other_example_errors_warn_and_fall_back() {
    let cases = [
        (
            "serendipity",
            "en",
            "We use the expression 'serendipity' in everyday context.",
        ),
        ("学习", "zh-CN", "我们在日常生活中使用“学习”这个词。"),
    ];
    for (text, lang, fallback) in cases {
        let fakes = Fakes::default();
        let failing_example = ProviderSet {
            example: fakes.example(|_| Err(network("fake_example"))),
            ..fakes.full_set()
        };
        let pipeline = CapturePipeline::new(
            ProviderRegistry::new(fakes.untouchable_set()).with_language(lang, failing_example),
            100,
        );

        let outcome = pipeline.run(request(text, lang, "de")).await.unwrap();

        assert_eq!(
            outcome.warnings,
            vec![warning(PipelineStage::Example, network("fake_example"))],
            "{lang}"
        );
        assert_eq!(outcome.card.example_source, fallback);
        assert_eq!(outcome.card.example_target, format!("<{fallback}>"));
    }
}

#[tokio::test]
async fn languages_without_an_example_provider_get_no_example_and_no_warning() {
    for (text, lang) in [("Haus", "DE"), ("maison", "fr")] {
        let fakes = Fakes::default();
        let others = ProviderSet {
            tts: fakes.tts(audio),
            ..ProviderSet::new(fakes.translation(bracketed))
        };
        let pipeline = CapturePipeline::new(
            ProviderRegistry::new(others).with_language("en", fakes.untouchable_set()),
            100,
        );

        let outcome = pipeline.run(request(text, lang, "en")).await.unwrap();

        let lang = lang.to_lowercase();
        assert_eq!(outcome.warnings, vec![], "{lang}");
        assert_eq!(outcome.card.example_source, "");
        assert_eq!(outcome.card.example_target, "");
        assert_eq!(outcome.card.source_lang, lang);
        assert_eq!(
            fakes.log.calls(),
            vec![
                format!("translate {text} {lang}->en"),
                format!("tts {text} {lang}")
            ]
        );
    }
}

#[tokio::test]
async fn example_translation_failure_is_a_warning() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        translation: fakes.translation(|text| {
            if text.starts_with("It was pure") {
                Err(network("fake_translation"))
            } else {
                bracketed(text)
            }
        }),
        ..fakes.full_set()
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert_eq!(
        outcome.warnings,
        vec![warning(
            PipelineStage::ExampleTranslate,
            network("fake_translation")
        )]
    );
    assert_eq!(outcome.card.back, "<serendipity>");
    assert_eq!(outcome.card.example_source, "It was pure serendipity.");
    assert_eq!(outcome.card.example_target, "");
}

#[tokio::test]
async fn tts_failure_is_a_warning_and_leaves_no_audio() {
    fn not_audio() -> ProviderError {
        ProviderError::Malformed {
            provider: "fake_tts".into(),
            message: "expected audio".into(),
        }
    }
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        tts: fakes.tts(|_| Err(not_audio())),
        ..fakes.full_set()
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert_eq!(
        outcome.warnings,
        vec![warning(PipelineStage::Tts, not_audio())]
    );
    assert_eq!(outcome.card.audio_front, None);
    assert_eq!(outcome.card.back, "<serendipity>");
    assert_eq!(outcome.card.script_metadata, annotation("").unwrap());
}

/// § 5: empty audio is no audio (`None`, not an empty recording that the
/// listening exercises would then try to play), and no warning.
#[tokio::test]
async fn empty_audio_is_no_audio() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        tts: fakes.tts(|_| Ok(Vec::new())),
        ..fakes.full_set()
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert_eq!(outcome.warnings, vec![]);
    assert_eq!(outcome.card.audio_front, None);
}

#[tokio::test]
async fn annotator_failure_is_a_warning_and_leaves_no_metadata() {
    fn unsupported() -> ProviderError {
        ProviderError::Unsupported {
            provider: "fake_annotator".into(),
            what: "this text".into(),
        }
    }
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        annotator: fakes.annotator(|_| Err(unsupported())),
        ..fakes.full_set()
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert_eq!(
        outcome.warnings,
        vec![warning(PipelineStage::Annotate, unsupported())]
    );
    assert_eq!(outcome.card.script_metadata, ScriptMetadata::None);
    assert_eq!(outcome.card.audio_front.as_deref(), Some(MP3));
}

#[tokio::test]
async fn every_failure_is_collected_in_stage_order() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(ProviderSet {
        translation: fakes.translation(|_| Err(network("t"))),
        rarity: fakes.rarity(|_| Err(network("r"))),
        example: fakes.example(|_| Err(network("e"))),
        tts: fakes.tts(|_| Err(network("s"))),
        annotator: fakes.annotator(|_| Err(network("a"))),
    });

    let outcome = pipeline
        .run(request("serendipity", "en", "de"))
        .await
        .unwrap();

    assert_eq!(
        outcome.warnings,
        vec![
            warning(PipelineStage::Translate, network("t")),
            warning(PipelineStage::Rarity, network("r")),
            warning(PipelineStage::Example, network("e")),
            warning(PipelineStage::ExampleTranslate, network("t")),
            warning(PipelineStage::Tts, network("s")),
            warning(PipelineStage::Annotate, network("a")),
        ]
    );
    let card = outcome.card;
    assert_eq!(card.front, "serendipity");
    assert_eq!((card.back.as_str(), card.notes.as_str()), ("", ""));
    assert_eq!(
        card.example_source,
        "We use the expression 'serendipity' in everyday context."
    );
    assert_eq!(card.example_target, "");
    assert_eq!(card.audio_front, None);
    assert_eq!(card.script_metadata, ScriptMetadata::None);
    assert_eq!(card.tags, vec!["vocab", "reading"]);
}

#[tokio::test]
async fn blank_text_is_rejected_before_any_provider_runs() {
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(fakes.untouchable_set());
    for text in ["", "   ", "\n\t "] {
        assert_eq!(
            pipeline.run(request(text, "en", "de")).await,
            Err(PipelineError::Empty),
            "{text:?}"
        );
    }
    assert!(fakes.log.calls().is_empty());
}

/// src-tauri keeps the pipeline in Tauri's managed state (`Send + Sync`) and
/// awaits `run` in async commands on a multi-threaded runtime (`Send`
/// futures). Holding a non-`Send` value across an `.await` in `run` would
/// break that integration without failing any other test in this crate.
#[test]
fn pipeline_and_its_future_can_cross_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>(_: &T) {}
    assert_send_sync::<CapturePipeline>();
    let fakes = Fakes::default();
    let pipeline = fakes.english_pipeline(fakes.full_set());
    let capture = pipeline.run(request("serendipity", "en", "de"));
    assert_send(&capture);
}

#[tokio::test]
async fn length_limit_counts_characters_not_bytes() {
    let fakes = Fakes::default();
    let lenient = ProviderSet {
        tts: fakes.tts(audio),
        ..ProviderSet::new(fakes.translation(bracketed))
    };
    let pipeline = CapturePipeline::new(ProviderRegistry::new(lenient), 50);

    let forty_hanzi = "学".repeat(40);
    assert_eq!(forty_hanzi.len(), 120, "3 UTF-8 bytes per hanzi");
    let outcome = pipeline
        .run(request(&forty_hanzi, "en", "de"))
        .await
        .unwrap();
    assert_eq!(outcome.card.front, forty_hanzi);
    assert_eq!(outcome.card.source_lang, "zh-CN");

    let exactly_fifty = format!("  {}  ", "a".repeat(50));
    assert!(pipeline
        .run(request(&exactly_fifty, "en", "de"))
        .await
        .is_ok());

    let calls_before = fakes.log.calls().len();
    for (text, len) in [
        ("学".repeat(51), 51),
        ("a".repeat(51), 51),
        ("学".repeat(120), 120),
    ] {
        assert_eq!(
            pipeline.run(request(&text, "en", "de")).await,
            Err(PipelineError::TooLong { len, max: 50 })
        );
    }
    assert_eq!(
        fakes.log.calls().len(),
        calls_before,
        "rejected captures call nothing"
    );
}
