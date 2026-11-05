//! Google Translate text-to-speech (Milestone B1; J4 for Chinese).
//!
//! The unofficial `translate_tts` endpoint the pre-refactor Anki client used
//! for front-side audio, kept as the TTS source only (MyMemory translates).
//! Two details survive from the old code because the endpoint depends on them:
//! the `client=tw-ob` parameter and a browser-like User-Agent
//! ([`BROWSER_USER_AGENT`]). The language code is passed through verbatim,
//! because the endpoint expects `zh-CN` spelled that way (the old code
//! lowercased it).
//!
//! The endpoint answers some failures with an HTML page and status 200, so a
//! response only counts as audio when it says so in its content type and is
//! not empty.

use async_trait::async_trait;
use reqwest::Url;

use super::{
    http::{endpoint, is_success, malformed, status_error, HttpClient, BROWSER_USER_AGENT},
    TtsProvider,
};
use crate::error::ProviderError;

/// [`TtsProvider`] backed by
/// `GET {base}/translate_tts?ie=UTF-8&tl=LANG&client=tw-ob&q=TEXT`.
#[derive(Debug, Clone)]
pub struct GoogleTtsProvider {
    http: HttpClient,
    base_url: String,
}

impl GoogleTtsProvider {
    pub const NAME: &'static str = "google_tts";
    pub const DEFAULT_BASE_URL: &'static str = "https://translate.google.com";

    /// The production service.
    pub fn new() -> Self {
        Self::with_base_url(Self::DEFAULT_BASE_URL)
    }

    /// Same as [`new`](Self::new), against another server (tests use wiremock).
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            http: HttpClient::with_user_agent(BROWSER_USER_AGENT),
            base_url: base_url.into(),
        }
    }

    fn tts_url(&self, text: &str, lang: &str) -> Result<Url, ProviderError> {
        let mut url = endpoint(Self::NAME, &self.base_url, &["translate_tts"])?;
        url.query_pairs_mut()
            .append_pair("ie", "UTF-8")
            .append_pair("tl", lang)
            .append_pair("client", "tw-ob")
            .append_pair("q", text);
        Ok(url)
    }
}

impl Default for GoogleTtsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TtsProvider for GoogleTtsProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn synthesize(&self, text: &str, lang: &str) -> Result<Vec<u8>, ProviderError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ProviderError::Unsupported {
                provider: Self::NAME.to_string(),
                what: "empty text".to_string(),
            });
        }
        let response = self.http.get(Self::NAME, self.tts_url(text, lang)?).await?;
        parse_audio(
            response.status,
            response.content_type.as_deref(),
            response.body,
        )
    }
}

/// Interprets a `translate_tts` response: the body, when the status is a
/// success, the content type is `audio/*` and the body is not empty.
pub fn parse_audio(
    status: u16,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Result<Vec<u8>, ProviderError> {
    if !is_success(status) {
        return Err(status_error(
            GoogleTtsProvider::NAME,
            status,
            &String::from_utf8_lossy(&body),
        ));
    }
    let media_type = content_type
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !media_type.starts_with("audio/") {
        return Err(malformed(
            GoogleTtsProvider::NAME,
            format!(
                "expected audio, got content type {:?}",
                content_type.unwrap_or("(none)")
            ),
        ));
    }
    if body.is_empty() {
        return Err(malformed(GoogleTtsProvider::NAME, "empty audio response"));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MP3_FRAME: &[u8] = &[0xFF, 0xF3, 0x44, 0xC4, 0x00];

    #[test]
    fn accepts_non_empty_audio_of_any_audio_type() {
        for content_type in [
            "audio/mpeg",
            "Audio/MPEG",
            "audio/mpeg; charset=binary",
            "audio/ogg",
        ] {
            assert_eq!(
                parse_audio(200, Some(content_type), MP3_FRAME.to_vec()),
                Ok(MP3_FRAME.to_vec()),
                "{content_type}"
            );
        }
    }

    #[test]
    fn rejects_empty_or_non_audio_bodies() {
        let cases: [(Option<&str>, &[u8]); 5] = [
            (Some("audio/mpeg"), b""),
            (Some("text/html; charset=UTF-8"), b"<html>captcha</html>"),
            (Some("application/json"), b"{}"),
            (Some("audioX/mpeg"), MP3_FRAME),
            (None, MP3_FRAME),
        ];
        for (content_type, body) in cases {
            assert!(
                matches!(
                    parse_audio(200, content_type, body.to_vec()),
                    Err(ProviderError::Malformed { .. })
                ),
                "{content_type:?}"
            );
        }
    }

    #[test]
    fn http_errors_are_typed() {
        assert!(matches!(
            parse_audio(500, Some("text/html"), b"<html>error</html>".to_vec()),
            Err(ProviderError::Http { status: 500, .. })
        ));
        assert!(matches!(
            parse_audio(404, Some("audio/mpeg"), MP3_FRAME.to_vec()),
            Err(ProviderError::Http { status: 404, .. })
        ));
    }

    #[test]
    fn tts_url_passes_the_language_verbatim_and_encodes_text() {
        let provider = GoogleTtsProvider::with_base_url("https://translate.google.com");
        let url = provider.tts_url("你好", "zh-CN").unwrap();
        assert_eq!(
            url.as_str(),
            "https://translate.google.com/translate_tts?ie=UTF-8&tl=zh-CN&client=tw-ob&q=%E4%BD%A0%E5%A5%BD"
        );
        let url = provider.tts_url("rock'n'roll & C++", "en").unwrap();
        assert_eq!(
            url.query(),
            Some("ie=UTF-8&tl=en&client=tw-ob&q=rock%27n%27roll+%26+C%2B%2B")
        );
    }

    #[test]
    fn new_uses_the_production_endpoint() {
        let url = GoogleTtsProvider::new().tts_url("hello", "en").unwrap();
        assert_eq!(
            url.as_str(),
            "https://translate.google.com/translate_tts?ie=UTF-8&tl=en&client=tw-ob&q=hello"
        );
    }
}
