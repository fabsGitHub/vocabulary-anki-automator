//! Chinese providers (Milestones J3, J4): pinyin annotation and example
//! sentences, both backed by the bundled HSK curriculum first.
//!
//! Neither provider touches the network. The curriculum is the authority for
//! the words it contains: its pinyin is curated per *word*, while generated
//! pinyin is looked up per *character*. That is why a curriculum hit always
//! wins and generation is only the fallback for captures outside the
//! curriculum.
//!
//! # Pinyin crate evaluation (J3)
//!
//! J3 asked to compare `pinyin` with `mandarin-to-pinyin` on tone-mark edge
//! cases before picking one. Only `pinyin` 0.11 was approved as a dependency,
//! so `mandarin-to-pinyin` was not evaluated. The tests in this module record
//! what `pinyin` does:
//!
//! - Tone marks are correct for regular syllables (你好 `nǐ hǎo`, 学习
//!   `xué xí`, 中国 `zhōng guó`), including tone-marked ü (女 `nǚ`, 绿 `lǜ`)
//!   and Extension A characters (㐀 `qiū`).
//! - Tones are citation tones: no tone sandhi (一 is always `yī`, 一个 gives
//!   `yī gè`, not the spoken `yí gè`), the same convention dictionaries use.
//! - Heteronyms get their most common reading regardless of the word, because
//!   the lookup is per character: 银行 (bank) comes out as `yín xíng` instead of
//!   `yín háng`, 长城 as `zhǎng chéng` instead of `cháng chéng`. The curriculum
//!   pinyin fixes this for every HSK word; captures outside it keep the
//!   limitation.
//!
//! Generated pinyin joins every syllable with a single space and passes
//! non-hanzi runs (Latin letters, digits, punctuation) through as their own
//! space-separated tokens, so `卡拉OK` reads `kǎ lā OK`. Whitespace in the input
//! only separates tokens; it never doubles a space.

use std::sync::Arc;

use async_trait::async_trait;
use pinyin::{Pinyin, ToPinyin};

use super::{CurriculumLookup, ExampleProvider, ScriptAnnotator};
use crate::{
    error::ProviderError,
    model::{ScriptMetadata, ScriptVariant},
};

/// [`ScriptAnnotator`] producing `ScriptMetadata::Chinese`: curriculum pinyin
/// and HSK level when the capture is an HSK word, generated pinyin otherwise.
#[derive(Clone)]
pub struct PinyinAnnotator {
    curriculum: Arc<dyn CurriculumLookup>,
}

impl PinyinAnnotator {
    pub const NAME: &'static str = "pinyin";

    pub fn new(curriculum: Arc<dyn CurriculumLookup>) -> Self {
        Self { curriculum }
    }
}

#[async_trait]
impl ScriptAnnotator for PinyinAnnotator {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    /// A curriculum hit gives its pinyin and `hsk_level`; a miss gives
    /// [`generate_pinyin`] and no level. Text without a single character that
    /// has a reading (e.g. Latin text under a zh-CN profile) is `Unsupported`,
    /// rather than "pinyin" that merely repeats the input.
    async fn annotate(&self, text: &str) -> Result<ScriptMetadata, ProviderError> {
        let text = text.trim();
        if let Some(item) = self.curriculum.lookup(text) {
            return Ok(ScriptMetadata::Chinese {
                pinyin: item.pinyin,
                hsk_level: Some(item.hsk_level),
                script_variant: ScriptVariant::Simplified,
            });
        }
        if !text.chars().any(|c| c.to_pinyin().is_some()) {
            return Err(ProviderError::Unsupported {
                provider: Self::NAME.to_string(),
                what: "text without Chinese characters".to_string(),
            });
        }
        Ok(ScriptMetadata::Chinese {
            pinyin: generate_pinyin(text),
            hsk_level: None,
            script_variant: ScriptVariant::Simplified,
        })
    }
}

/// [`ExampleProvider`] returning the curriculum's example sentence for an HSK
/// word, and `NotFound` for anything else (the pipeline then uses its zh-CN
/// fallback template).
#[derive(Clone)]
pub struct CurriculumExampleProvider {
    curriculum: Arc<dyn CurriculumLookup>,
}

impl CurriculumExampleProvider {
    pub const NAME: &'static str = "curriculum";

    pub fn new(curriculum: Arc<dyn CurriculumLookup>) -> Self {
        Self { curriculum }
    }
}

#[async_trait]
impl ExampleProvider for CurriculumExampleProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn fetch_example(&self, word: &str) -> Result<String, ProviderError> {
        self.curriculum
            .lookup(word.trim())
            .and_then(|item| item.example_sentence_source)
            .map(|sentence| sentence.trim().to_string())
            .filter(|sentence| !sentence.is_empty())
            .ok_or_else(|| ProviderError::NotFound {
                provider: Self::NAME.to_string(),
            })
    }
}

/// Tone-marked pinyin for `text`, one reading per character (see the module
/// docs for the heteronym limitation). Syllables are joined by single spaces,
/// runs of other characters are kept verbatim as their own tokens, and
/// whitespace is collapsed: `"学习 ABC"` and `"学习ABC"` both give
/// `"xué xí ABC"`.
pub fn generate_pinyin(text: &str) -> String {
    let mut tokens: Vec<String> = Vec::new();
    let mut run = String::new();
    for c in text.chars() {
        let syllable = c.to_pinyin().map(Pinyin::with_tone);
        if (syllable.is_some() || c.is_whitespace()) && !run.is_empty() {
            tokens.push(std::mem::take(&mut run));
        }
        match syllable {
            Some(syllable) => tokens.push(syllable.to_string()),
            None if c.is_whitespace() => {}
            None => run.push(c),
        }
    }
    if !run.is_empty() {
        tokens.push(run);
    }
    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{curriculum::CurriculumIndex, model::CurriculumItem};

    fn item(hanzi: &str, pinyin: &str, level: u8, example: Option<&str>) -> CurriculumItem {
        CurriculumItem {
            hanzi: hanzi.into(),
            traditional: None,
            hsk_level: level,
            frequency: None,
            curriculum_order: 1,
            pinyin: pinyin.into(),
            pos: None,
            meaning_en: None,
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

    fn curriculum(items: Vec<CurriculumItem>) -> Arc<dyn CurriculumLookup> {
        Arc::new(CurriculumIndex::new(items))
    }

    fn chinese(pinyin: &str, hsk_level: Option<u8>) -> ScriptMetadata {
        ScriptMetadata::Chinese {
            pinyin: pinyin.into(),
            hsk_level,
            script_variant: ScriptVariant::Simplified,
        }
    }

    #[test]
    fn generates_tone_marked_pinyin() {
        let cases = [
            ("你好", "nǐ hǎo"),
            ("学习", "xué xí"),
            ("中国", "zhōng guó"),
            ("女", "nǚ"),
            ("绿", "lǜ"),
            ("一", "yī"),
            ("一个", "yī gè"),
            ("吗", "ma"),
            ("\u{3400}", "qiū"),
            // Heteronyms: per-character lookup takes the most common reading.
            ("银行", "yín xíng"),
            ("长城", "zhǎng chéng"),
        ];
        for (text, expected) in cases {
            assert_eq!(generate_pinyin(text), expected, "generate_pinyin({text:?})");
        }
    }

    #[test]
    fn generated_pinyin_keeps_other_characters_and_collapses_whitespace() {
        let cases = [
            ("学习ABC", "xué xí ABC"),
            ("ABC学习", "ABC xué xí"),
            ("学习 ABC", "xué xí ABC"),
            ("卡拉OK", "kǎ lā OK"),
            ("T恤", "T xù"),
            ("3D打印", "3D dǎ yìn"),
            ("你好！", "nǐ hǎo ！"),
            ("  你好 \t\n 世界  ", "nǐ hǎo shì jiè"),
            ("hello world", "hello world"),
            ("", ""),
        ];
        for (text, expected) in cases {
            assert_eq!(generate_pinyin(text), expected, "generate_pinyin({text:?})");
        }
    }

    #[tokio::test]
    async fn curriculum_hit_wins_over_generated_pinyin() {
        let annotator = PinyinAnnotator::new(curriculum(vec![item("银行", "yín háng", 3, None)]));
        assert_eq!(
            annotator.annotate(" 银行 ").await,
            Ok(chinese("yín háng", Some(3)))
        );

        let without = PinyinAnnotator::new(curriculum(vec![]));
        assert_eq!(
            without.annotate("银行").await,
            Ok(chinese("yín xíng", None))
        );
        assert_eq!(
            without.annotate("学习ABC").await,
            Ok(chinese("xué xí ABC", None))
        );
    }

    #[tokio::test]
    async fn text_without_hanzi_is_unsupported() {
        let annotator = PinyinAnnotator::new(curriculum(vec![]));
        for text in ["hello", "", "   ", "。！"] {
            assert_eq!(
                annotator.annotate(text).await,
                Err(ProviderError::Unsupported {
                    provider: "pinyin".into(),
                    what: "text without Chinese characters".into()
                }),
                "{text:?}"
            );
        }
    }

    #[tokio::test]
    async fn curriculum_example_returns_the_sentence_or_not_found() {
        let provider = CurriculumExampleProvider::new(curriculum(vec![
            item("学习", "xué xí", 1, Some(" 我们一起学习中文。 ")),
            item("你好", "nǐ hǎo", 1, None),
            item("银行", "yín háng", 3, Some("  ")),
        ]));
        assert_eq!(
            provider.fetch_example("学习 ").await,
            Ok("我们一起学习中文。".to_string())
        );
        let not_found = Err(ProviderError::NotFound {
            provider: "curriculum".into(),
        });
        for word in ["你好", "银行", "没有"] {
            assert_eq!(provider.fetch_example(word).await, not_found, "{word}");
        }
    }
}
