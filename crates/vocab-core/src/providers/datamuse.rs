//! Datamuse word-frequency rarity check (Milestone B1, English only).
//!
//! Moved out of the pre-refactor `ApiClient::analyze_rarity`. What survives:
//! the word is trimmed and lowercased, rarity means fewer than
//! [`RARE_BELOW_PER_MILLION`] occurrences per million words (Datamuse's `f:`
//! tag, from Google Books Ngrams), and a synonym is looked up only for rare
//! words.
//!
//! What changed, and why:
//! - Phrases are rejected as `Unsupported` instead of silently reported as
//!   "not rare"; the pipeline skips rarity for phrases anyway (Milestone H).
//! - `sp=` is Datamuse's *spelled-like* constraint, with fuzzy matching and
//!   `*`/`?` wildcards, so its first hit can be a different word (`hipopatamus`
//!   comes back as `hippopotamus`). Only an exact, case-insensitive match
//!   counts; anything else means Datamuse does not know the word. The old code
//!   took the first hit's frequency and could flag a word as rare on behalf of
//!   another one.
//! - A word without an `f:` tag is unknown, not rare (the old code defaulted
//!   the frequency to 0 and therefore called it rare).
//! - A failed synonym lookup is logged and degrades to `synonym: None`: the
//!   frequency verdict is still worth keeping.

use async_trait::async_trait;
use reqwest::Url;
use serde::Deserialize;

use super::{
    http::{endpoint, is_success, malformed, status_error, HttpClient},
    RarityProvider, RarityResult,
};
use crate::error::ProviderError;

/// Words occurring less often than this per million words are rare.
pub const RARE_BELOW_PER_MILLION: f64 = 5.0;

/// [`RarityProvider`] backed by `GET {base}/words?sp=WORD&md=f&max=1` and, for
/// rare words, `GET {base}/words?rel_syn=WORD&max=1`.
#[derive(Debug, Clone)]
pub struct DatamuseProvider {
    http: HttpClient,
    base_url: String,
}

/// One entry of a Datamuse `/words` response.
#[derive(Debug, Deserialize)]
struct WordEntry {
    word: String,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

impl DatamuseProvider {
    pub const NAME: &'static str = "datamuse";
    pub const DEFAULT_BASE_URL: &'static str = "https://api.datamuse.com";

    /// The production service.
    pub fn new() -> Self {
        Self::with_base_url(Self::DEFAULT_BASE_URL)
    }

    /// Same as [`new`](Self::new), against another server (tests use wiremock).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            http: HttpClient::new(),
            base_url: base_url.into(),
        }
    }

    fn words_url(&self, params: &[(&str, &str)]) -> Result<Url, ProviderError> {
        let mut url = endpoint(Self::NAME, &self.base_url, &["words"])?;
        url.query_pairs_mut().extend_pairs(params);
        Ok(url)
    }

    async fn synonym(&self, word: &str) -> Result<Option<String>, ProviderError> {
        let url = self.words_url(&[("rel_syn", word), ("max", "1")])?;
        let response = self.http.get(Self::NAME, url).await?;
        parse_synonym(response.status, &response.text())
    }
}

impl Default for DatamuseProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RarityProvider for DatamuseProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn analyze(&self, word: &str) -> Result<RarityResult, ProviderError> {
        let word = word.trim().to_lowercase();
        let unsupported = |what: &str| ProviderError::Unsupported {
            provider: Self::NAME.to_string(),
            what: what.to_string(),
        };
        if word.is_empty() {
            return Err(unsupported("empty text"));
        }
        if word.contains(char::is_whitespace) {
            return Err(unsupported("multi-word phrases"));
        }

        let url = self.words_url(&[("sp", word.as_str()), ("md", "f"), ("max", "1")])?;
        let response = self.http.get(Self::NAME, url).await?;
        let Some(frequency) = parse_frequency(response.status, &response.text(), &word)? else {
            return Ok(RarityResult::default());
        };
        if !is_rare(frequency) {
            return Ok(RarityResult {
                is_rare: false,
                frequency_per_million: Some(frequency),
                synonym: None,
            });
        }
        let synonym = self.synonym(&word).await.unwrap_or_else(|err| {
            log::warn!("datamuse synonym lookup failed, keeping the rarity result: {err}");
            None
        });
        Ok(RarityResult {
            is_rare: true,
            frequency_per_million: Some(frequency),
            synonym,
        })
    }
}

/// Whether a word occurring `frequency_per_million` times per million words is
/// rare: strictly below [`RARE_BELOW_PER_MILLION`].
pub fn is_rare(frequency_per_million: f64) -> bool {
    frequency_per_million < RARE_BELOW_PER_MILLION
}

/// Interprets a `words?sp=WORD&md=f&max=1` response: the frequency per million
/// of `word`, or `None` when Datamuse does not know it (no exact match, or no
/// `f:` tag). An `f:` tag that is not a non-negative number is `Malformed`.
pub fn parse_frequency(status: u16, body: &str, word: &str) -> Result<Option<f64>, ProviderError> {
    let entries = parse_entries(status, body)?;
    let word = word.trim().to_lowercase();
    let Some(entry) = entries
        .iter()
        .find(|e| e.word.trim().to_lowercase() == word)
    else {
        return Ok(None);
    };
    let Some(raw) = entry
        .tags
        .iter()
        .flatten()
        .find_map(|tag| tag.strip_prefix("f:"))
    else {
        return Ok(None);
    };
    raw.trim()
        .parse::<f64>()
        .ok()
        .filter(|f| f.is_finite() && *f >= 0.0)
        .map(Some)
        .ok_or_else(|| {
            malformed(
                DatamuseProvider::NAME,
                format!("unusable frequency tag f:{raw}"),
            )
        })
}

/// Interprets a `words?rel_syn=WORD&max=1` response: the first synonym, if any.
pub fn parse_synonym(status: u16, body: &str) -> Result<Option<String>, ProviderError> {
    Ok(parse_entries(status, body)?
        .into_iter()
        .map(|entry| entry.word.trim().to_string())
        .find(|word| !word.is_empty()))
}

fn parse_entries(status: u16, body: &str) -> Result<Vec<WordEntry>, ProviderError> {
    if !is_success(status) {
        return Err(status_error(DatamuseProvider::NAME, status, body));
    }
    serde_json::from_str(body).map_err(|err| {
        malformed(
            DatamuseProvider::NAME,
            format!("expected a JSON array of words: {err}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERENDIPITY: &str =
        include_str!("../../tests/fixtures/datamuse/frequency_serendipity.json");
    const HOUSE: &str = include_str!("../../tests/fixtures/datamuse/frequency_house.json");
    const EMPTY: &str = include_str!("../../tests/fixtures/datamuse/empty.json");
    const OTHER_WORD: &str =
        include_str!("../../tests/fixtures/datamuse/spelled_like_other_word.json");
    const SYNONYMS: &str = include_str!("../../tests/fixtures/datamuse/synonyms_serendipity.json");

    #[test]
    fn rarity_threshold_is_strictly_below_five_per_million() {
        let cases = [
            (0.0, true),
            (0.292473, true),
            (4.999, true),
            (5.0, false),
            (5.001, false),
            (313.9, false),
        ];
        for (frequency, rare) in cases {
            assert_eq!(is_rare(frequency), rare, "is_rare({frequency})");
        }
    }

    #[test]
    fn parse_frequency_reads_the_f_tag_of_the_exact_word_only() {
        let cases: [(&str, &str, &str, Option<f64>); 9] = [
            ("real sample", SERENDIPITY, "serendipity", Some(0.292473)),
            (
                "query case and spacing",
                SERENDIPITY,
                " Serendipity ",
                Some(0.292473),
            ),
            ("common word", HOUSE, "house", Some(313.926397)),
            ("unknown word", EMPTY, "xyzzyq", None),
            (
                "spelled-like hit for another word",
                OTHER_WORD,
                "hipopatamus",
                None,
            ),
            (
                "no tags",
                r#"[{"word":"serendipity","score":1}]"#,
                "serendipity",
                None,
            ),
            (
                "null tags",
                r#"[{"word":"serendipity","tags":null}]"#,
                "serendipity",
                None,
            ),
            (
                "other tags first",
                r#"[{"word":"Serendipity","tags":["n","syn","f:2.5"]}]"#,
                "serendipity",
                Some(2.5),
            ),
            (
                "exact match is not the first entry",
                r#"[{"word":"serendipitous","tags":["f:0.1"]},{"word":"serendipity","tags":["f:0.3"]}]"#,
                "serendipity",
                Some(0.3),
            ),
        ];
        for (name, body, word, expected) in cases {
            assert_eq!(parse_frequency(200, body, word), Ok(expected), "{name}");
        }
    }

    #[test]
    fn parse_frequency_rejects_unusable_responses() {
        let malformed_cases = [
            ("not JSON", "<html>"),
            ("object instead of array", r#"{"word":"x"}"#),
            ("entry without word", r#"[{"tags":["f:1"]}]"#),
            (
                "f tag is not a number",
                r#"[{"word":"x","tags":["f:abc"]}]"#,
            ),
            ("f tag is NaN", r#"[{"word":"x","tags":["f:NaN"]}]"#),
            ("f tag is negative", r#"[{"word":"x","tags":["f:-1"]}]"#),
        ];
        for (name, body) in malformed_cases {
            assert!(
                matches!(
                    parse_frequency(200, body, "x"),
                    Err(ProviderError::Malformed { .. })
                ),
                "{name}"
            );
        }
        assert!(matches!(
            parse_frequency(500, "", "x"),
            Err(ProviderError::Http { status: 500, .. })
        ));
        assert!(matches!(
            parse_frequency(429, "", "x"),
            Err(ProviderError::QuotaExceeded { .. })
        ));
    }

    #[test]
    fn parse_synonym_takes_the_first_non_blank_word() {
        assert_eq!(parse_synonym(200, SYNONYMS), Ok(Some("luck".to_string())));
        assert_eq!(parse_synonym(200, EMPTY), Ok(None));
        assert_eq!(
            parse_synonym(200, r#"[{"word":" "},{"word":"fluke"}]"#),
            Ok(Some("fluke".to_string()))
        );
        assert!(matches!(
            parse_synonym(200, "nope"),
            Err(ProviderError::Malformed { .. })
        ));
        assert!(matches!(
            parse_synonym(502, ""),
            Err(ProviderError::Http { status: 502, .. })
        ));
    }

    #[test]
    fn words_url_encodes_the_word() {
        let provider = DatamuseProvider::with_base_url("http://127.0.0.1:9/");
        let url = provider
            .words_url(&[("sp", "rock'n'roll"), ("md", "f"), ("max", "1")])
            .unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:9/words?sp=rock%27n%27roll&md=f&max=1"
        );
        let url = provider
            .words_url(&[("rel_syn", "naïve"), ("max", "1")])
            .unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:9/words?rel_syn=na%C3%AFve&max=1"
        );
    }

    #[test]
    fn new_uses_the_production_endpoint() {
        let url = DatamuseProvider::new()
            .words_url(&[("sp", "house"), ("md", "f"), ("max", "1")])
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.datamuse.com/words?sp=house&md=f&max=1"
        );
    }
}
