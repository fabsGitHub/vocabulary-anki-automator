//! Typed provider failures (Milestone C).
//!
//! The pre-refactor `translate()` swallowed every failure into an empty
//! string. These variants keep "quota exhausted", "network down" and "the
//! service answered with something we can't parse" apart all the way to the
//! UI, which receives them serialized as `{ "kind": "...", ... }`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderError {
    /// The request never got a response (DNS, TLS, connection refused, timeout).
    #[error("could not reach {provider}: {message}")]
    Network { provider: String, message: String },
    /// The service answered but refuses to serve more requests for now.
    #[error("{provider} quota exceeded: {message}")]
    QuotaExceeded { provider: String, message: String },
    /// The service answered with a non-success HTTP or API status.
    #[error("{provider} returned status {status}: {message}")]
    Http {
        provider: String,
        status: u16,
        message: String,
    },
    /// The response arrived but did not have the expected shape.
    #[error("unexpected response from {provider}: {message}")]
    Malformed { provider: String, message: String },
    /// The service has no result for this input (e.g. unknown word).
    #[error("{provider} has no result for this input")]
    NotFound { provider: String },
    /// The provider cannot handle this input or language.
    #[error("{provider} does not support {what}")]
    Unsupported { provider: String, what: String },
}

impl ProviderError {
    pub fn provider(&self) -> &str {
        match self {
            ProviderError::Network { provider, .. }
            | ProviderError::QuotaExceeded { provider, .. }
            | ProviderError::Http { provider, .. }
            | ProviderError::Malformed { provider, .. }
            | ProviderError::NotFound { provider }
            | ProviderError::Unsupported { provider, .. } => provider,
        }
    }

    pub fn is_quota_exceeded(&self) -> bool {
        matches!(self, ProviderError::QuotaExceeded { .. })
    }

    /// Maps a `reqwest` transport error to `Network` (or `Http` when the
    /// error carries a status code, `Malformed` for a body that failed to
    /// decode). The request URL is dropped first: it carries the captured text
    /// and MyMemory's contact e-mail, and these messages are logged and shown
    /// in the UI. The cause chain is kept, because reqwest's own message stops
    /// at "error sending request".
    pub fn from_reqwest(provider: &str, err: reqwest::Error) -> Self {
        let err = err.without_url();
        let message = crate::providers::http::describe(&err);
        let provider = provider.to_string();
        match err.status() {
            Some(status) => ProviderError::Http {
                provider,
                status: status.as_u16(),
                message,
            },
            None if err.is_decode() => ProviderError::Malformed { provider, message },
            None => ProviderError::Network { provider, message },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_kind_tag() {
        let err = ProviderError::QuotaExceeded {
            provider: "mymemory".into(),
            message: "next available in 10 hours".into(),
        };
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            serde_json::json!({
                "kind": "quota_exceeded",
                "provider": "mymemory",
                "message": "next available in 10 hours"
            })
        );
        assert!(err.is_quota_exceeded());
        assert_eq!(err.provider(), "mymemory");
    }

    /// The URL of a failed request holds the user's captured text and their
    /// contact e-mail; neither may leak into the error shown in the UI.
    #[tokio::test]
    async fn transport_errors_never_carry_the_request_url() {
        // Port 1 on loopback refuses the connection (or a sandbox forbids
        // it); either way the request fails fast and offline, and reqwest's
        // error names the full URL.
        let err = reqwest::Client::new()
            .get("http://127.0.0.1:1/get?q=private+capture&de=me%40example.com")
            .send()
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("private"),
            "precondition: reqwest includes the URL"
        );

        let mapped = ProviderError::from_reqwest("mymemory", err);
        let ProviderError::Network { provider, message } = &mapped else {
            panic!("expected a network error, got {mapped:?}");
        };
        assert_eq!(provider, "mymemory");
        assert!(!message.contains("private"), "{message}");
        assert!(!message.contains("example.com"), "{message}");
        assert!(
            message.len() > "error sending request".len(),
            "cause chain kept: {message}"
        );
    }
}
