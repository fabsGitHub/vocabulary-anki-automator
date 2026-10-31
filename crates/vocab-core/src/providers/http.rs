//! Shared HTTP plumbing for the network providers.
//!
//! docs/architecture.md § 2 requires every HTTP client to have a 10 s timeout
//! and a descriptive User-Agent; building clients here keeps that in one
//! place. This module also owns the single translation from transport failures
//! to `ProviderError::Network`, which reduces each provider to "build the URL"
//! plus a pure `parse_*` function over status and body that unit tests drive
//! with canned fixtures.
//!
//! Error messages never contain the request URL (`reqwest::Error::without_url`):
//! it carries the user's captured text and, for MyMemory, their contact e-mail,
//! and these messages end up in the approval window and in log files.
//!
//! A non-success status maps to `ProviderError::Http`, except
//! `429 Too Many Requests`, which every provider reports as `QuotaExceeded`:
//! the service is refusing more requests for now, which is exactly what that
//! variant tells the UI.

use std::{borrow::Cow, error::Error as StdError, time::Duration};

use reqwest::{header::CONTENT_TYPE, StatusCode, Url};

use crate::error::ProviderError;

/// Timeout for a whole request, connect to last body byte.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// User-Agent of every provider request, so service operators can tell who is
/// calling and where to complain.
pub const USER_AGENT: &str =
    "vocabulary-automator/0.1 (+https://github.com/fabsGitHub/vocabulary-anki-automator)";

/// Google's unofficial `translate_tts` endpoint refuses non-browser agents, so
/// the TTS provider keeps the browser-like agent the pre-refactor code sent.
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0";

/// Longest response excerpt quoted in an error message, in characters.
const SNIPPET_CHARS: usize = 200;

/// A `reqwest::Client` configured per § 2, plus the error mapping.
///
/// Building a client only fails in a broken environment, e.g. when the TLS
/// backend cannot initialize. The failure is kept instead of panicking, and
/// every request then reports it as `Network`, so such a setup degrades
/// captures instead of crashing the app.
#[derive(Debug, Clone)]
pub struct HttpClient {
    client: Result<reqwest::Client, String>,
}

/// Status, content type and body of a response, whatever the status was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl HttpClient {
    /// A client sending [`USER_AGENT`].
    pub fn new() -> Self {
        Self::with_user_agent(USER_AGENT)
    }

    /// A client sending `user_agent`, with the standard timeout.
    pub fn with_user_agent(user_agent: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(user_agent)
            .build()
            .map_err(|err| describe(&err));
        if let Err(message) = &client {
            log::error!("could not build the HTTP client: {message}");
        }
        Self { client }
    }

    /// GETs `url` and returns the response whatever its status; only
    /// transport failures (connect, TLS, timeout, broken body) are errors.
    pub async fn get(&self, provider: &str, url: Url) -> Result<HttpResponse, ProviderError> {
        let client = self
            .client
            .as_ref()
            .map_err(|message| network(provider, format!("HTTP client unavailable: {message}")))?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|err| network(provider, describe(&err.without_url())))?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .bytes()
            .await
            .map_err(|err| network(provider, describe(&err.without_url())))?;
        Ok(HttpResponse {
            status,
            content_type,
            body: body.to_vec(),
        })
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpResponse {
    /// The body as text; invalid UTF-8 is replaced rather than rejected, since
    /// the JSON parsers report anything unusable as `Malformed` anyway.
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }
}

/// `base` with `segments` appended as path segments. Each segment is
/// percent-encoded on its own, so user text such as `and/or` cannot change the
/// path (the pre-refactor code interpolated raw words into URLs). A trailing
/// slash on `base` is ignored. An unusable base URL means no request can be
/// sent, which is reported as `Network`.
///
/// The dot segments `.` and `..` are `Unsupported`: URL syntax resolves them
/// instead of carrying them (the `url` crate silently drops them), so the
/// request would go to a different path without the user's text in it.
pub(crate) fn endpoint(
    provider: &str,
    base: &str,
    segments: &[&str],
) -> Result<Url, ProviderError> {
    let invalid = |reason: &str| network(provider, format!("invalid base URL {base:?}: {reason}"));
    let mut url = Url::parse(base).map_err(|err| invalid(&err.to_string()))?;
    if let Some(dots) = segments
        .iter()
        .find(|segment| matches!(**segment, "." | ".."))
    {
        return Err(ProviderError::Unsupported {
            provider: provider.to_string(),
            what: format!("the URL path segment {dots:?}"),
        });
    }
    url.path_segments_mut()
        .map_err(|()| invalid("it cannot carry a path"))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

pub(crate) fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// The error for a non-success HTTP `status` (see the module docs for 429).
/// The message is the status reason plus an excerpt of the body, unless the
/// body is an HTML error page, which would only be noise in the UI.
pub(crate) fn status_error(provider: &str, status: u16, body: &str) -> ProviderError {
    let reason = StatusCode::from_u16(status)
        .ok()
        .and_then(|code| code.canonical_reason())
        .unwrap_or("unexpected status");
    let excerpt = snippet(body);
    let message = if excerpt.is_empty() || excerpt.starts_with('<') {
        reason.to_string()
    } else {
        format!("{reason}: {excerpt}")
    };
    if status == 429 {
        ProviderError::QuotaExceeded {
            provider: provider.to_string(),
            message,
        }
    } else {
        ProviderError::Http {
            provider: provider.to_string(),
            status,
            message,
        }
    }
}

pub(crate) fn network(provider: &str, message: impl Into<String>) -> ProviderError {
    ProviderError::Network {
        provider: provider.to_string(),
        message: message.into(),
    }
}

pub(crate) fn malformed(provider: &str, message: impl Into<String>) -> ProviderError {
    ProviderError::Malformed {
        provider: provider.to_string(),
        message: message.into(),
    }
}

/// `body` trimmed and cut to [`SNIPPET_CHARS`] characters, for error messages.
pub(crate) fn snippet(body: &str) -> String {
    let body = body.trim();
    match body.char_indices().nth(SNIPPET_CHARS) {
        Some((cut, _)) => format!("{}…", &body[..cut]),
        None => body.to_string(),
    }
}

/// `err` followed by its chain of causes. reqwest's own `Display` stops at
/// "error sending request", which hides whether DNS, the connection or the
/// timeout failed.
fn describe(err: &(dyn StdError + 'static)) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !message.ends_with(&cause_text) {
            message.push_str(": ");
            message.push_str(&cause_text);
        }
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_appends_encoded_segments_to_any_base_shape() {
        let cases: [(&str, &[&str], &str); 5] = [
            (
                "http://127.0.0.1:9000",
                &["get"],
                "http://127.0.0.1:9000/get",
            ),
            (
                "http://127.0.0.1:9000/",
                &["get"],
                "http://127.0.0.1:9000/get",
            ),
            (
                "https://example.com/proxy/",
                &["get"],
                "https://example.com/proxy/get",
            ),
            (
                "https://api.dictionaryapi.dev",
                &["api", "v2", "entries", "en", "and/or"],
                "https://api.dictionaryapi.dev/api/v2/entries/en/and%2For",
            ),
            (
                "https://api.dictionaryapi.dev",
                &["api", "v2", "entries", "en", "naïve café"],
                "https://api.dictionaryapi.dev/api/v2/entries/en/na%C3%AFve%20caf%C3%A9",
            ),
        ];
        for (base, segments, expected) in cases {
            let url = endpoint("test", base, segments).unwrap();
            assert_eq!(url.as_str(), expected, "endpoint({base:?}, {segments:?})");
        }
    }

    #[test]
    fn endpoint_refuses_dot_segments_instead_of_dropping_them() {
        // `Url::path_segments_mut().extend` skips "." and "..", which turned
        // a capture of ".." into a request for /api/v2/entries/en.
        for dots in [".", ".."] {
            assert_eq!(
                endpoint(
                    "test",
                    "https://api.dictionaryapi.dev",
                    &["api", "v2", "entries", "en", dots]
                ),
                Err(ProviderError::Unsupported {
                    provider: "test".into(),
                    what: format!("the URL path segment {dots:?}"),
                }),
                "{dots:?}"
            );
        }
        // Only the exact dot segments are special; these stay one segment.
        let cases = [
            ("...", "/en/..."),
            (".hidden", "/en/.hidden"),
            ("a..", "/en/a.."),
            ("%2e", "/en/%252e"),
        ];
        for (segment, path) in cases {
            let url = endpoint("test", "https://example.com", &["en", segment]).unwrap();
            assert_eq!(url.path(), path, "{segment:?}");
        }
    }

    #[test]
    fn endpoint_rejects_unusable_bases_as_network_errors() {
        for base in ["", "not a url", "mailto:someone@example.com"] {
            match endpoint("test", base, &["get"]) {
                Err(ProviderError::Network { provider, message }) => {
                    assert_eq!(provider, "test");
                    assert!(message.contains("invalid base URL"), "{message}");
                }
                other => panic!("endpoint({base:?}) = {other:?}"),
            }
        }
    }

    #[test]
    fn status_error_maps_429_to_quota_and_hides_html_bodies() {
        assert_eq!(
            status_error("svc", 429, ""),
            ProviderError::QuotaExceeded {
                provider: "svc".into(),
                message: "Too Many Requests".into()
            }
        );
        assert_eq!(
            status_error("svc", 503, "<!DOCTYPE html><html>busy</html>"),
            ProviderError::Http {
                provider: "svc".into(),
                status: 503,
                message: "Service Unavailable".into()
            }
        );
        assert_eq!(
            status_error("svc", 500, " {\"error\":\"boom\"} \n"),
            ProviderError::Http {
                provider: "svc".into(),
                status: 500,
                message: "Internal Server Error: {\"error\":\"boom\"}".into()
            }
        );
        assert_eq!(
            status_error("svc", 599, ""),
            ProviderError::Http {
                provider: "svc".into(),
                status: 599,
                message: "unexpected status".into()
            }
        );
    }

    #[test]
    fn snippet_trims_and_truncates_on_char_boundaries() {
        assert_eq!(snippet("  short \n"), "short");
        let long = "学".repeat(SNIPPET_CHARS + 5);
        let cut = snippet(&long);
        assert_eq!(cut.chars().count(), SNIPPET_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(
            snippet(&"x".repeat(SNIPPET_CHARS)),
            "x".repeat(SNIPPET_CHARS)
        );
    }

    #[test]
    fn is_success_is_exactly_2xx() {
        assert!(is_success(200));
        assert!(is_success(204));
        assert!(!is_success(199));
        assert!(!is_success(301));
        assert!(!is_success(404));
    }

    #[test]
    fn describe_walks_the_cause_chain_without_repeating_it() {
        #[derive(Debug)]
        struct Layer(&'static str, Option<Box<Layer>>);
        impl std::fmt::Display for Layer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl StdError for Layer {
            fn source(&self) -> Option<&(dyn StdError + 'static)> {
                self.1
                    .as_deref()
                    .map(|inner| inner as &(dyn StdError + 'static))
            }
        }
        let err = Layer(
            "error sending request",
            Some(Box::new(Layer(
                "tcp connect error",
                Some(Box::new(Layer("tcp connect error", None))),
            ))),
        );
        assert_eq!(describe(&err), "error sending request: tcp connect error");
    }
}
