//! Provider interfaces (decision 3). Every external enrichment source sits
//! behind one of these traits so it can be swapped (MyMemory today, an
//! offline Argos model later) and faked in tests.
//!
//! Traits use `async_trait` so they stay object-safe: the language-aware
//! `ProviderRegistry` (Milestone J4) holds them as `Arc<dyn Trait>`.

pub mod chinese;
pub mod datamuse;
pub mod dictionary_api;
pub mod google_tts;
pub mod http;
pub mod mymemory;
pub mod registry;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use chinese::{CurriculumExampleProvider, PinyinAnnotator};
pub use datamuse::DatamuseProvider;
pub use dictionary_api::DictionaryApiProvider;
pub use google_tts::GoogleTtsProvider;
pub use http::HttpClient;
pub use mymemory::MyMemoryProvider;
pub use registry::{ProviderRegistry, ProviderSet, StandardProviders};

use crate::{
    error::ProviderError,
    model::{CurriculumItem, ScriptMetadata},
};

#[async_trait]
pub trait TranslationProvider: Send + Sync {
    /// Short stable identifier, used in errors and logs (e.g. `"mymemory"`).
    fn name(&self) -> &'static str;

    /// Translates `text` from `source` to `target` (canonical language codes,
    /// see `crate::language::normalize_lang`).
    async fn translate(
        &self,
        text: &str,
        source: &str,
        target: &str,
    ) -> Result<String, ProviderError>;
}

#[async_trait]
pub trait ExampleProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// An example sentence using `word`, in the word's own language.
    async fn fetch_example(&self, word: &str) -> Result<String, ProviderError>;
}

#[async_trait]
pub trait RarityProvider: Send + Sync {
    fn name(&self) -> &'static str;

    async fn analyze(&self, word: &str) -> Result<RarityResult, ProviderError>;
}

#[async_trait]
pub trait TtsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    /// MP3 bytes of `text` spoken in `lang` (canonical language code).
    async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError>;
}

/// Produces script-specific annotations for a capture (Milestone J3/J4):
/// pinyin and HSK level for Chinese.
#[async_trait]
pub trait ScriptAnnotator: Send + Sync {
    fn name(&self) -> &'static str;

    async fn annotate(&self, text: &str) -> Result<ScriptMetadata, ProviderError>;
}

/// Read-only access to the bundled curriculum. Implemented by
/// `crate::curriculum::CurriculumIndex` (in memory, `Send + Sync`), so async
/// providers never need a database connection.
pub trait CurriculumLookup: Send + Sync {
    fn lookup(&self, hanzi: &str) -> Option<CurriculumItem>;
}

/// Result of a word-frequency analysis.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RarityResult {
    pub is_rare: bool,
    /// Occurrences per million words, when the provider reports it.
    pub frequency_per_million: Option<f64>,
    /// A more common synonym, offered when the word is rare.
    pub synonym: Option<String>,
}
