//! Language-aware provider routing (Milestone J4).
//!
//! The pre-refactor pipeline called every enrichment service for every
//! capture. Datamuse and dictionaryapi.dev only know English, so a Chinese
//! capture got an English "rarity" verdict and an English fallback sentence.
//! The registry resolves one [`ProviderSet`] per capture from its (already
//! CJK-sniffed) source language, and an absent provider in a set means "this
//! step does not apply to this language", not "this step failed".

use std::{collections::HashMap, fmt, sync::Arc};

use super::{
    CurriculumExampleProvider, CurriculumLookup, DatamuseProvider, DictionaryApiProvider,
    ExampleProvider, GoogleTtsProvider, MyMemoryProvider, PinyinAnnotator, RarityProvider,
    ScriptAnnotator, TranslationProvider, TtsProvider,
};
use crate::language::normalize_lang;

/// The providers used for captures in one source language. Every step except
/// translation is optional.
#[derive(Clone)]
pub struct ProviderSet {
    pub translation: Arc<dyn TranslationProvider>,
    pub rarity: Option<Arc<dyn RarityProvider>>,
    pub example: Option<Arc<dyn ExampleProvider>>,
    pub tts: Option<Arc<dyn TtsProvider>>,
    pub annotator: Option<Arc<dyn ScriptAnnotator>>,
}

impl ProviderSet {
    /// A set that only translates; fill the other fields with struct-update
    /// syntax (`ProviderSet { tts: Some(..), ..ProviderSet::new(t) }`).
    pub fn new(translation: Arc<dyn TranslationProvider>) -> Self {
        Self {
            translation,
            rarity: None,
            example: None,
            tts: None,
            annotator: None,
        }
    }
}

/// Lists provider names, since the trait objects themselves are opaque.
impl fmt::Debug for ProviderSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSet")
            .field("translation", &self.translation.name())
            .field("rarity", &self.rarity.as_ref().map(|p| p.name()))
            .field("example", &self.example.as_ref().map(|p| p.name()))
            .field("tts", &self.tts.as_ref().map(|p| p.name()))
            .field("annotator", &self.annotator.as_ref().map(|p| p.name()))
            .finish()
    }
}

/// A default [`ProviderSet`] plus per-language overrides, keyed by canonical
/// language code.
#[derive(Debug, Clone)]
pub struct ProviderRegistry {
    default: ProviderSet,
    by_language: HashMap<String, ProviderSet>,
}

/// What [`ProviderRegistry::standard`] needs from the app.
#[derive(Clone)]
pub struct StandardProviders {
    /// Raises MyMemory's daily quota when set (Milestone C).
    pub contact_email: Option<String>,
    /// Backs the Chinese example sentences and pinyin.
    pub curriculum: Arc<dyn CurriculumLookup>,
}

impl ProviderRegistry {
    /// A registry that uses `default` for every language until
    /// [`with_language`](Self::with_language) adds overrides.
    pub fn new(default: ProviderSet) -> Self {
        Self {
            default,
            by_language: HashMap::new(),
        }
    }

    /// Uses `set` for `lang` (normalized, so `"ZH"` and `"zh-CN"` are the same
    /// key), replacing any earlier set for it.
    pub fn with_language(mut self, lang: &str, set: ProviderSet) -> Self {
        self.by_language.insert(normalize_lang(lang), set);
        self
    }

    /// The set for `source_lang` after normalization, or the default. Matching
    /// is exact on the canonical code: `en-US` does not fall back to `en`.
    pub fn resolve(&self, source_lang: &str) -> &ProviderSet {
        self.by_language
            .get(&normalize_lang(source_lang))
            .unwrap_or(&self.default)
    }

    /// The production wiring (docs/architecture.md § 4): MyMemory and Google
    /// TTS everywhere; Datamuse and dictionaryapi.dev for English only; the
    /// curriculum example and pinyin for Chinese, whose rarity step is
    /// deliberately empty because the HSK level already is the difficulty
    /// signal.
    pub fn standard(config: StandardProviders) -> Self {
        let translation: Arc<dyn TranslationProvider> =
            Arc::new(MyMemoryProvider::new(config.contact_email));
        let tts: Arc<dyn TtsProvider> = Arc::new(GoogleTtsProvider::new());
        let default = ProviderSet {
            tts: Some(tts),
            ..ProviderSet::new(translation)
        };
        let english = ProviderSet {
            rarity: Some(Arc::new(DatamuseProvider::new())),
            example: Some(Arc::new(DictionaryApiProvider::new())),
            ..default.clone()
        };
        let chinese = ProviderSet {
            example: Some(Arc::new(CurriculumExampleProvider::new(
                config.curriculum.clone(),
            ))),
            annotator: Some(Arc::new(PinyinAnnotator::new(config.curriculum))),
            ..default.clone()
        };
        Self::new(default)
            .with_language("en", english)
            .with_language("zh-CN", chinese)
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::{curriculum::CurriculumIndex, error::ProviderError};

    struct Named(&'static str);

    #[async_trait]
    impl TranslationProvider for Named {
        fn name(&self) -> &'static str {
            self.0
        }

        async fn translate(&self, _: &str, _: &str, _: &str) -> Result<String, ProviderError> {
            Ok(String::new())
        }
    }

    fn set(name: &'static str) -> ProviderSet {
        ProviderSet::new(Arc::new(Named(name)))
    }

    /// `(translation, rarity, example, tts, annotator)` provider names.
    type Wiring = (
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    );

    fn wiring(set: &ProviderSet) -> Wiring {
        (
            set.translation.name(),
            set.rarity.as_ref().map(|p| p.name()),
            set.example.as_ref().map(|p| p.name()),
            set.tts.as_ref().map(|p| p.name()),
            set.annotator.as_ref().map(|p| p.name()),
        )
    }

    #[test]
    fn resolve_normalizes_and_falls_back_to_the_default() {
        let registry = ProviderRegistry::new(set("default"))
            .with_language("EN", set("english"))
            .with_language("zh", set("chinese"));
        let cases = [
            ("en", "english"),
            ("EN", "english"),
            (" en ", "english"),
            ("zh-CN", "chinese"),
            ("zh_cn", "chinese"),
            ("ZH-Hans", "chinese"),
            ("de", "default"),
            ("en-US", "default"),
            ("", "default"),
        ];
        for (lang, expected) in cases {
            assert_eq!(
                registry.resolve(lang).translation.name(),
                expected,
                "{lang:?}"
            );
        }
    }

    #[test]
    fn with_language_replaces_an_earlier_set() {
        let registry = ProviderRegistry::new(set("default"))
            .with_language("de", set("first"))
            .with_language("DE", set("second"));
        assert_eq!(registry.resolve("de").translation.name(), "second");
    }

    #[test]
    fn standard_wiring_matches_the_contract_table() {
        let registry = ProviderRegistry::standard(StandardProviders {
            contact_email: Some("me@example.com".into()),
            curriculum: Arc::new(CurriculumIndex::default()),
        });
        let cases: [(&str, Wiring); 5] = [
            (
                "en",
                (
                    "mymemory",
                    Some("datamuse"),
                    Some("dictionary_api"),
                    Some("google_tts"),
                    None,
                ),
            ),
            (
                "zh-CN",
                (
                    "mymemory",
                    None,
                    Some("curriculum"),
                    Some("google_tts"),
                    Some("pinyin"),
                ),
            ),
            ("de", ("mymemory", None, None, Some("google_tts"), None)),
            ("fr", ("mymemory", None, None, Some("google_tts"), None)),
            ("pt", ("mymemory", None, None, Some("google_tts"), None)),
        ];
        for (lang, expected) in cases {
            assert_eq!(wiring(registry.resolve(lang)), expected, "{lang}");
        }
    }

    #[test]
    fn standard_shares_one_translation_and_tts_instance() {
        let registry = ProviderRegistry::standard(StandardProviders {
            contact_email: None,
            curriculum: Arc::new(CurriculumIndex::default()),
        });
        let en = registry.resolve("en");
        let zh = registry.resolve("zh-CN");
        let other = registry.resolve("it");
        assert!(Arc::ptr_eq(&en.translation, &zh.translation));
        assert!(Arc::ptr_eq(&en.translation, &other.translation));
        assert!(Arc::ptr_eq(
            en.tts.as_ref().unwrap(),
            other.tts.as_ref().unwrap()
        ));
    }

    #[test]
    fn debug_lists_provider_names() {
        let debug = format!("{:?}", set("only"));
        assert_eq!(
            debug,
            "ProviderSet { translation: \"only\", rarity: None, example: None, tts: None, annotator: None }"
        );
    }
}
