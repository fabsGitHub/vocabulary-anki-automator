//! The capture pipeline (Milestones B1, H, J4): captured text -> translation
//! -> rarity -> example sentence -> TTS -> script annotation -> `Card`.
//!
//! It replaces the inline logic of the pre-refactor
//! `trigger_clipboard_sequence`, with three deliberate changes:
//!
//! - **Provider failures never fail a capture.** Each one becomes a typed
//!   [`PipelineWarning`] and leaves its field empty; the approval window shows
//!   the warnings and the user types in what is missing. The old code
//!   swallowed every failure into an empty string, so "quota used up" and
//!   "network down" looked identical. Only invalid input ([`PipelineError`])
//!   stops a capture.
//! - **Providers are chosen per language** by the `ProviderRegistry`, from the
//!   source language after CJK sniffing, so Chinese captures never reach the
//!   English-only Datamuse and dictionaryapi.dev providers.
//! - **The length limit counts characters, not bytes.** The old check used
//!   `str::len`, which rejected a 34-character hanzi phrase under the default
//!   100 limit because every hanzi is three bytes in UTF-8.
//!
//! The steps run one after another in the order of docs/architecture.md § 5,
//! which also fixes the order of the warnings.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    error::ProviderError,
    language::{effective_source_lang, is_phrase, normalize_lang},
    model::Card,
    providers::{ProviderRegistry, RarityResult},
};

/// One capture to enrich into a card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRequest {
    /// The captured text, untrimmed.
    pub text: String,
    /// As configured; the pipeline normalizes it and overrides it with `zh-CN`
    /// when the text contains CJK ideographs.
    pub source_lang: String,
    /// As configured; normalized by the pipeline.
    pub target_lang: String,
    pub captured_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

/// The enriched card plus everything that went wrong on the way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureOutcome {
    pub card: Card,
    pub warnings: Vec<PipelineWarning>,
}

/// A provider failure the capture survived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineWarning {
    pub stage: PipelineStage,
    pub error: ProviderError,
}

/// The pipeline step a [`PipelineWarning`] comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStage {
    Translate,
    Rarity,
    Example,
    ExampleTranslate,
    Tts,
    Annotate,
}

impl PipelineStage {
    /// The serialized name (`"example_translate"`, ...).
    pub fn as_str(&self) -> &'static str {
        match self {
            PipelineStage::Translate => "translate",
            PipelineStage::Rarity => "rarity",
            PipelineStage::Example => "example",
            PipelineStage::ExampleTranslate => "example_translate",
            PipelineStage::Tts => "tts",
            PipelineStage::Annotate => "annotate",
        }
    }
}

/// Input the pipeline refuses to process.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PipelineError {
    #[error("nothing to capture: the text is empty")]
    Empty,
    /// `len` and `max` are in characters.
    #[error("the text is {len} characters long; captures are limited to {max}")]
    TooLong { len: usize, max: usize },
}

/// Runs captures through the providers of a [`ProviderRegistry`].
#[derive(Debug, Clone)]
pub struct CapturePipeline {
    registry: ProviderRegistry,
    max_chars: usize,
}

impl CapturePipeline {
    /// `max_chars` is the longest accepted capture, in characters, after
    /// trimming.
    pub fn new(registry: ProviderRegistry, max_chars: usize) -> Self {
        Self {
            registry,
            max_chars,
        }
    }

    /// The providers, e.g. to re-run TTS after the user edited a card's front.
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// The longest accepted capture, in characters.
    pub fn max_chars(&self) -> usize {
        self.max_chars
    }

    /// Enriches `req` into a card (docs/architecture.md § 5). Fails only for
    /// empty or overlong text; provider failures end up in `warnings`.
    pub async fn run(&self, req: CaptureRequest) -> Result<CaptureOutcome, PipelineError> {
        let text = req.text.trim();
        if text.is_empty() {
            return Err(PipelineError::Empty);
        }
        let len = text.chars().count();
        if len > self.max_chars {
            return Err(PipelineError::TooLong {
                len,
                max: self.max_chars,
            });
        }

        let source = effective_source_lang(&req.source_lang, text);
        let target = normalize_lang(&req.target_lang);
        let set = self.registry.resolve(&source);
        let mut warnings = Vec::new();
        let mut card = Card::new(text, "", source.as_str(), target.as_str(), req.captured_at);

        let translation = set.translation.translate(text, &source, &target).await;
        card.back =
            record(&mut warnings, PipelineStage::Translate, translation).unwrap_or_default();

        if let Some(rarity) = set.rarity.as_ref().filter(|_| !is_phrase(text)) {
            let analysis = rarity.analyze(text).await;
            if let Some(result) = record(&mut warnings, PipelineStage::Rarity, analysis) {
                card.notes = rarity_note(&result);
            }
        }

        if let Some(example) = &set.example {
            card.example_source = match example.fetch_example(text).await {
                Ok(sentence) if !sentence.trim().is_empty() => sentence.trim().to_string(),
                Ok(_) | Err(ProviderError::NotFound { .. }) => fallback_example(&source, text),
                Err(error) => {
                    warn(&mut warnings, PipelineStage::Example, error);
                    fallback_example(&source, text)
                }
            };
            if !card.example_source.is_empty() {
                let translation = set
                    .translation
                    .translate(&card.example_source, &source, &target)
                    .await;
                card.example_target =
                    record(&mut warnings, PipelineStage::ExampleTranslate, translation)
                        .unwrap_or_default();
            }
        }

        if let Some(tts) = &set.tts {
            let audio = tts.synthesize(text, &source).await;
            card.audio_front =
                record(&mut warnings, PipelineStage::Tts, audio).filter(|bytes| !bytes.is_empty());
        }

        if let Some(annotator) = &set.annotator {
            let metadata = annotator.annotate(text).await;
            card.script_metadata =
                record(&mut warnings, PipelineStage::Annotate, metadata).unwrap_or_default();
        }

        card.tags = req.tags;
        Ok(CaptureOutcome { card, warnings })
    }
}

/// `Some(value)` on success; on failure, [`warn`]s and returns `None`.
fn record<T>(
    warnings: &mut Vec<PipelineWarning>,
    stage: PipelineStage,
    result: Result<T, ProviderError>,
) -> Option<T> {
    result.map_err(|error| warn(warnings, stage, error)).ok()
}

/// Logs a provider failure and records it as a warning for `stage`.
fn warn(warnings: &mut Vec<PipelineWarning>, stage: PipelineStage, error: ProviderError) {
    log::warn!("capture pipeline: {} step failed: {error}", stage.as_str());
    warnings.push(PipelineWarning { stage, error });
}

/// The card note for a rarity verdict; empty for common or unknown words.
fn rarity_note(result: &RarityResult) -> String {
    if !result.is_rare {
        return String::new();
    }
    match result.synonym.as_deref().map(str::trim) {
        Some(synonym) if !synonym.is_empty() => {
            format!("Rare word. More common synonym: {synonym}")
        }
        _ => "Rare word.".to_string(),
    }
}

/// The sentence used when a language's example provider has nothing for
/// `word`, so the card still shows the word in a sentence. Empty for languages
/// without a template: an English template on a German card would be worse
/// than no example.
fn fallback_example(lang: &str, word: &str) -> String {
    match lang {
        "en" => format!("We use the expression '{word}' in everyday context."),
        "zh-CN" => format!("我们在日常生活中使用“{word}”这个词。"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_and_errors_serialize_to_the_contract_shapes() {
        let warning = PipelineWarning {
            stage: PipelineStage::ExampleTranslate,
            error: ProviderError::QuotaExceeded {
                provider: "mymemory".into(),
                message: "come back tomorrow".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&warning).unwrap(),
            serde_json::json!({
                "stage": "example_translate",
                "error": {
                    "kind": "quota_exceeded",
                    "provider": "mymemory",
                    "message": "come back tomorrow"
                }
            })
        );
        assert_eq!(
            serde_json::to_value(PipelineError::Empty).unwrap(),
            serde_json::json!({ "kind": "empty" })
        );
        assert_eq!(
            serde_json::to_value(PipelineError::TooLong { len: 51, max: 50 }).unwrap(),
            serde_json::json!({ "kind": "too_long", "len": 51, "max": 50 })
        );
    }

    #[test]
    fn stage_names_match_serde() {
        let stages = [
            PipelineStage::Translate,
            PipelineStage::Rarity,
            PipelineStage::Example,
            PipelineStage::ExampleTranslate,
            PipelineStage::Tts,
            PipelineStage::Annotate,
        ];
        for stage in stages {
            assert_eq!(
                serde_json::to_value(stage).unwrap(),
                serde_json::Value::String(stage.as_str().to_string())
            );
        }
    }

    #[test]
    fn rarity_note_texts() {
        let rare = |synonym: Option<&str>| RarityResult {
            is_rare: true,
            frequency_per_million: Some(0.3),
            synonym: synonym.map(str::to_string),
        };
        assert_eq!(
            rarity_note(&rare(Some("luck"))),
            "Rare word. More common synonym: luck"
        );
        assert_eq!(rarity_note(&rare(None)), "Rare word.");
        assert_eq!(rarity_note(&rare(Some("  "))), "Rare word.");
        assert_eq!(rarity_note(&RarityResult::default()), "");
        let common = RarityResult {
            is_rare: false,
            frequency_per_million: Some(300.0),
            synonym: Some("ignored".into()),
        };
        assert_eq!(rarity_note(&common), "");
    }

    #[test]
    fn fallback_templates_exist_for_english_and_chinese_only() {
        assert_eq!(
            fallback_example("en", "serendipity"),
            "We use the expression 'serendipity' in everyday context."
        );
        assert_eq!(
            fallback_example("zh-CN", "银行"),
            "我们在日常生活中使用“银行”这个词。"
        );
        assert_eq!(fallback_example("de", "Haus"), "");
    }

    #[test]
    fn errors_have_readable_messages() {
        assert_eq!(
            PipelineError::TooLong { len: 51, max: 50 }.to_string(),
            "the text is 51 characters long; captures are limited to 50"
        );
        assert_eq!(
            PipelineError::Empty.to_string(),
            "nothing to capture: the text is empty"
        );
    }
}
