//! dictionaryapi.dev example sentences (Milestone B1, English only).
//!
//! Moved out of the pre-refactor `ApiClient::fetch_example`, which returned a
//! made-up "We use the expression '…' in everyday context." sentence whenever
//! the lookup failed. That fallback is a pipeline decision (it is per language
//! and must not hide real failures), so it now lives in `crate::pipeline`, and
//! this provider reports `NotFound` instead: for a 404 and for entries that
//! carry no example at all.
//!
//! The word is a path segment of the URL; it is percent-encoded as one segment
//! (`and/or` stays one word) instead of being interpolated raw. `.` and `..`
//! cannot be carried in a path at all and are refused as `Unsupported`.

use async_trait::async_trait;
use reqwest::Url;
use serde_json::Value;

use super::{
    http::{endpoint, is_success, malformed, status_error, HttpClient},
    ExampleProvider,
};
use crate::error::ProviderError;

/// [`ExampleProvider`] backed by `GET {base}/api/v2/entries/en/WORD`.
#[derive(Debug, Clone)]
pub struct DictionaryApiProvider {
    http: HttpClient,
    base_url: String,
}

impl DictionaryApiProvider {
    pub const NAME: &'static str = "dictionary_api";
    pub const DEFAULT_BASE_URL: &'static str = "https://api.dictionaryapi.dev";

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

    fn entry_url(&self, word: &str) -> Result<Url, ProviderError> {
        endpoint(
            Self::NAME,
            &self.base_url,
            &["api", "v2", "entries", "en", word],
        )
    }
}

impl Default for DictionaryApiProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExampleProvider for DictionaryApiProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn fetch_example(&self, word: &str) -> Result<String, ProviderError> {
        let word = word.trim().to_lowercase();
        if word.is_empty() {
            return Err(ProviderError::Unsupported {
                provider: Self::NAME.to_string(),
                what: "empty text".to_string(),
            });
        }
        let response = self.http.get(Self::NAME, self.entry_url(&word)?).await?;
        parse_example(response.status, &response.text())
    }
}

/// Interprets an `entries/en/WORD` response: the first non-blank
/// `example` of any definition, scanning entries, then meanings, then
/// definitions in order. 404 and "no example anywhere" are `NotFound`.
pub fn parse_example(status: u16, body: &str) -> Result<String, ProviderError> {
    let not_found = || ProviderError::NotFound {
        provider: DictionaryApiProvider::NAME.to_string(),
    };
    if status == 404 {
        return Err(not_found());
    }
    if !is_success(status) {
        return Err(status_error(DictionaryApiProvider::NAME, status, body));
    }
    let json: Value = serde_json::from_str(body).map_err(|err| {
        malformed(
            DictionaryApiProvider::NAME,
            format!("response is not JSON: {err}"),
        )
    })?;
    let entries = json.as_array().ok_or_else(|| {
        malformed(
            DictionaryApiProvider::NAME,
            "expected a JSON array of entries",
        )
    })?;
    entries
        .iter()
        .filter_map(|entry| entry.get("meanings")?.as_array())
        .flatten()
        .filter_map(|meaning| meaning.get("definitions")?.as_array())
        .flatten()
        .filter_map(|definition| definition.get("example")?.as_str())
        .map(str::trim)
        .find(|example| !example.is_empty())
        .map(str::to_string)
        .ok_or_else(not_found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = include_str!("../../tests/fixtures/dictionary_api/hello.json");
    const NO_EXAMPLE: &str = include_str!("../../tests/fixtures/dictionary_api/no_example.json");
    const NOT_FOUND: &str = include_str!("../../tests/fixtures/dictionary_api/not_found.json");

    #[test]
    fn finds_the_first_example_across_meanings() {
        // The real sample's first two meanings (noun, verb) have no example;
        // the first one sits in the third meaning.
        assert_eq!(
            parse_example(200, HELLO),
            Ok("Hello, everyone.".to_string())
        );
        assert_eq!(
            parse_example(
                200,
                r#"[{"meanings":[]},{"meanings":[{"definitions":[{"example":"  Second entry.  "}]}]}]"#
            ),
            Ok("Second entry.".to_string())
        );
    }

    #[test]
    fn missing_examples_and_404_are_not_found() {
        let cases = [
            ("404 body", 404, NOT_FOUND),
            ("entries without examples", 200, NO_EXAMPLE),
            ("empty array", 200, "[]"),
            ("entry without meanings", 200, r#"[{"word":"x"}]"#),
            (
                "non-string example",
                200,
                r#"[{"meanings":[{"definitions":[{"example":7}]}]}]"#,
            ),
        ];
        for (name, status, body) in cases {
            assert_eq!(
                parse_example(status, body),
                Err(ProviderError::NotFound {
                    provider: "dictionary_api".into()
                }),
                "{name}"
            );
        }
    }

    #[test]
    fn other_failures_are_typed() {
        assert!(matches!(
            parse_example(500, "oops"),
            Err(ProviderError::Http { status: 500, ref message, .. }) if message.contains("oops")
        ));
        assert!(matches!(
            parse_example(429, ""),
            Err(ProviderError::QuotaExceeded { .. })
        ));
        assert!(matches!(
            parse_example(200, "<html>"),
            Err(ProviderError::Malformed { .. })
        ));
        assert!(matches!(
            parse_example(200, NOT_FOUND),
            Err(ProviderError::Malformed { .. })
        ));
    }

    #[test]
    fn entry_url_keeps_the_word_in_one_segment() {
        let provider = DictionaryApiProvider::with_base_url("https://api.dictionaryapi.dev");
        let cases = [
            ("hello", "/api/v2/entries/en/hello"),
            ("and/or", "/api/v2/entries/en/and%2For"),
            ("naïve", "/api/v2/entries/en/na%C3%AFve"),
            ("rock'n'roll", "/api/v2/entries/en/rock'n'roll"),
            ("what?", "/api/v2/entries/en/what%3F"),
            ("c#", "/api/v2/entries/en/c%23"),
        ];
        for (word, path) in cases {
            let url = provider.entry_url(word).unwrap();
            assert_eq!(url.path(), path, "entry_url({word:?})");
            assert_eq!(url.query(), None);
        }
        for dots in [".", ".."] {
            assert!(
                matches!(
                    provider.entry_url(dots),
                    Err(ProviderError::Unsupported { provider: ref name, .. }) if name == "dictionary_api"
                ),
                "entry_url({dots:?})"
            );
        }
    }

    #[test]
    fn new_uses_the_production_endpoint() {
        assert_eq!(
            DictionaryApiProvider::new()
                .entry_url("hello")
                .unwrap()
                .as_str(),
            "https://api.dictionaryapi.dev/api/v2/entries/en/hello"
        );
    }
}
