//! MyMemory translation (Milestone C, decision 2).
//!
//! MyMemory replaced DeepL because it is keyless and free: anyone who clones
//! the repo gets working translations with zero setup. The price is a loose
//! API. Failures usually arrive as HTTP 200 with an API-level `responseStatus`
//! (sent as a number or as a string, depending on the code path), and the daily
//! quota is announced in three different ways: `responseStatus: 429`,
//! `quotaFinished: true`, or a `MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE
//! TRANSLATIONS` text in place of the translation, sometimes with status 200.
//! [`parse_translation`] folds all of that into typed `ProviderError`s, so the
//! approval window can tell "quota used up" from "network down" (the
//! pre-refactor `translate()` returned an empty string for both). Checking the
//! quota first matters: a status-200 warning would otherwise be put on the card
//! as if it were the translation.
//!
//! An optional contact e-mail is sent as `de=`, which is MyMemory's documented
//! way to raise the anonymous daily quota.

use async_trait::async_trait;
use reqwest::Url;
use serde_json::Value;

use super::{
    http::{endpoint, is_success, malformed, snippet, status_error, HttpClient},
    TranslationProvider,
};
use crate::error::ProviderError;

/// Marker MyMemory puts in `translatedText`/`responseDetails` once the free
/// quota is used up.
const QUOTA_WARNING: &str = "MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS";

/// Longest entity name [`decode_html_entities`] looks at (`#x10FFFF` plus room
/// for leading zeros).
const MAX_ENTITY_NAME: usize = 10;

/// [`TranslationProvider`] backed by `GET {base}/get?q=..&langpair=SRC|TGT`.
#[derive(Debug, Clone)]
pub struct MyMemoryProvider {
    http: HttpClient,
    base_url: String,
    contact_email: Option<String>,
}

impl MyMemoryProvider {
    pub const NAME: &'static str = "mymemory";
    pub const DEFAULT_BASE_URL: &'static str = "https://api.mymemory.translated.net";

    /// The production service. A blank `contact_email` counts as none.
    pub fn new(contact_email: Option<String>) -> Self {
        Self::with_base_url(Self::DEFAULT_BASE_URL, contact_email)
    }

    /// Same as [`new`](Self::new), against another server (tests use wiremock).
    pub fn with_base_url(base_url: impl Into<String>, contact_email: Option<String>) -> Self {
        Self {
            http: HttpClient::new(),
            base_url: base_url.into(),
            contact_email: contact_email
                .map(|email| email.trim().to_string())
                .filter(|email| !email.is_empty()),
        }
    }

    fn request_url(&self, text: &str, source: &str, target: &str) -> Result<Url, ProviderError> {
        let mut url = endpoint(Self::NAME, &self.base_url, &["get"])?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("q", text)
                .append_pair("langpair", &format!("{source}|{target}"));
            if let Some(email) = &self.contact_email {
                query.append_pair("de", email);
            }
        }
        Ok(url)
    }
}

#[async_trait]
impl TranslationProvider for MyMemoryProvider {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    async fn translate(
        &self,
        text: &str,
        source: &str,
        target: &str,
    ) -> Result<String, ProviderError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ProviderError::Unsupported {
                provider: Self::NAME.to_string(),
                what: "empty text".to_string(),
            });
        }
        let url = self.request_url(text, source, target)?;
        let response = self.http.get(Self::NAME, url).await?;
        parse_translation(response.status, &response.text())
    }
}

/// Interprets a MyMemory `/get` response (see the module docs for the order of
/// checks). Returns the trimmed, entity-decoded `responseData.translatedText`;
/// a blank translation is `NotFound`.
pub fn parse_translation(status: u16, body: &str) -> Result<String, ProviderError> {
    let json = serde_json::from_str::<Value>(body).ok();
    if let Some(message) = quota_message(status, json.as_ref(), body) {
        return Err(ProviderError::QuotaExceeded {
            provider: MyMemoryProvider::NAME.to_string(),
            message,
        });
    }
    if !is_success(status) {
        return Err(match json.as_ref().and_then(api_message) {
            Some(message) => http_error(status, message),
            None => status_error(MyMemoryProvider::NAME, status, body),
        });
    }
    let json = json.ok_or_else(|| {
        malformed(
            MyMemoryProvider::NAME,
            format!("response is not JSON: {}", snippet(body)),
        )
    })?;
    if let Some(api_status) = response_status(&json)?.filter(|code| *code != 200) {
        let message = api_message(&json).unwrap_or_else(|| format!("responseStatus {api_status}"));
        return Err(http_error(api_status, message));
    }
    let translated = json
        .pointer("/responseData/translatedText")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            malformed(
                MyMemoryProvider::NAME,
                "responseData.translatedText is missing",
            )
        })?;
    let translated = decode_html_entities(translated).trim().to_string();
    if translated.is_empty() {
        return Err(ProviderError::NotFound {
            provider: MyMemoryProvider::NAME.to_string(),
        });
    }
    Ok(translated)
}

/// `Some(message)` when the response says the quota is used up, in any of the
/// three ways MyMemory says it (or as a plain HTTP 429). The message is
/// MyMemory's own text when there is one, because it says when the quota
/// resets.
fn quota_message(status: u16, json: Option<&Value>, body: &str) -> Option<String> {
    let has_warning = |text: &str| text.to_uppercase().contains(QUOTA_WARNING);
    let signalled = match json {
        Some(json) => {
            json.get("quotaFinished").and_then(Value::as_bool) == Some(true)
                || matches!(response_status(json), Ok(Some(429)))
                || [
                    json.pointer("/responseData/translatedText"),
                    json.get("responseDetails"),
                ]
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(has_warning)
        }
        None => has_warning(body),
    };
    if !(signalled || status == 429) {
        return None;
    }
    Some(
        json.and_then(api_message)
            .unwrap_or_else(|| "the free daily translation quota is used up".to_string()),
    )
}

/// `responseStatus`, which MyMemory sends as a number or as a numeric string.
/// Absent or `null` is `None`; anything else non-numeric is `Malformed`.
fn response_status(json: &Value) -> Result<Option<u16>, ProviderError> {
    let parsed = match json.get("responseStatus") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Number(number)) => number.as_u64().and_then(|n| u16::try_from(n).ok()),
        Some(Value::String(text)) => text.trim().parse::<u16>().ok(),
        Some(_) => None,
    };
    parsed.map(Some).ok_or_else(|| {
        malformed(
            MyMemoryProvider::NAME,
            format!("unexpected responseStatus {}", json["responseStatus"]),
        )
    })
}

/// MyMemory's human-readable explanation of a failure: `responseDetails`, or
/// the text it put in place of the translation.
fn api_message(json: &Value) -> Option<String> {
    [
        json.get("responseDetails"),
        json.pointer("/responseData/translatedText"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|text| !text.is_empty())
    .map(decode_html_entities)
}

fn http_error(status: u16, message: String) -> ProviderError {
    ProviderError::Http {
        provider: MyMemoryProvider::NAME.to_string(),
        status,
        message,
    }
}

/// Decodes the HTML entities MyMemory leaves in its output: `&amp;`, `&lt;`,
/// `&gt;`, `&quot;`, `&apos;`, and numeric `&#NNN;`/`&#xHH;` references such as
/// `&#39;`. Single pass, so `&amp;lt;` becomes `&lt;`, not `<`. Anything it
/// does not recognize, including a bare `&`, is kept as-is.
pub fn decode_html_entities(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        decoded.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let entity = rest
            .find(';')
            .filter(|end| *end <= MAX_ENTITY_NAME + 1)
            .and_then(|end| decode_entity(&rest[1..end]).map(|c| (c, end)));
        match entity {
            Some((c, end)) => {
                decoded.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let reference = name.strip_prefix('#')?;
            let code = match reference.strip_prefix(['x', 'X']) {
                Some(hex) if !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                    u32::from_str_radix(hex, 16).ok()?
                }
                Some(_) => return None,
                None if !reference.is_empty() && reference.bytes().all(|b| b.is_ascii_digit()) => {
                    reference.parse().ok()?
                }
                None => return None,
            };
            char::from_u32(code).filter(|c| *c != '\0')
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUCCESS: &str = include_str!("../../tests/fixtures/mymemory/success.json");
    const QUOTA_EXCEEDED: &str = include_str!("../../tests/fixtures/mymemory/quota_exceeded.json");
    const QUOTA_WARNING_200: &str =
        include_str!("../../tests/fixtures/mymemory/quota_warning_status_200.json");
    const INVALID_TARGET: &str =
        include_str!("../../tests/fixtures/mymemory/invalid_target_language.json");
    const INVALID_SOURCE: &str =
        include_str!("../../tests/fixtures/mymemory/invalid_source_language.json");
    const STATUS_AS_STRING: &str =
        include_str!("../../tests/fixtures/mymemory/status_as_string.json");
    const HTML_ENTITIES: &str = include_str!("../../tests/fixtures/mymemory/html_entities.json");

    /// What a parse is expected to produce, loosely enough to table-drive.
    #[derive(Debug)]
    enum Expect {
        Text(&'static str),
        Quota(&'static str),
        Http(u16, &'static str),
        Malformed,
        NotFound,
    }

    fn check(name: &str, status: u16, body: &str, expect: &Expect) {
        let result = parse_translation(status, body);
        let ok = match (expect, &result) {
            (Expect::Text(want), Ok(got)) => got == want,
            (Expect::Quota(needle), Err(ProviderError::QuotaExceeded { provider, message })) => {
                provider == "mymemory" && message.contains(needle)
            }
            (
                Expect::Http(want, needle),
                Err(ProviderError::Http {
                    provider,
                    status,
                    message,
                }),
            ) => provider == "mymemory" && status == want && message.contains(needle),
            (Expect::Malformed, Err(ProviderError::Malformed { provider, .. })) => {
                provider == "mymemory"
            }
            (Expect::NotFound, Err(ProviderError::NotFound { provider })) => provider == "mymemory",
            _ => false,
        };
        assert!(ok, "{name}: expected {expect:?}, got {result:?}");
    }

    #[test]
    fn parses_fixtures() {
        let cases = [
            (
                "real success sample",
                200,
                SUCCESS,
                Expect::Text("Serendipität"),
            ),
            (
                "responseStatus as a string",
                200,
                STATUS_AS_STRING,
                Expect::Text("Haus"),
            ),
            (
                "html entities",
                200,
                HTML_ENTITIES,
                Expect::Text("Tom & Jerry's \"Haus\" <3 über ärger"),
            ),
            (
                "all quota signals",
                200,
                QUOTA_EXCEEDED,
                Expect::Quota("NEXT AVAILABLE IN"),
            ),
            (
                "quota signals with HTTP 429",
                429,
                QUOTA_EXCEEDED,
                Expect::Quota("NEXT AVAILABLE"),
            ),
            (
                "quota warning with status 200",
                200,
                QUOTA_WARNING_200,
                Expect::Quota("23 HOURS"),
            ),
            (
                "invalid target, string 403",
                200,
                INVALID_TARGET,
                Expect::Http(403, "IS AN INVALID TARGET LANGUAGE"),
            ),
            (
                "invalid source, HTTP 403",
                403,
                INVALID_SOURCE,
                Expect::Http(403, "INVALID SOURCE LANGUAGE"),
            ),
        ];
        for (name, status, body, expect) in &cases {
            check(name, *status, body, expect);
        }
    }

    #[test]
    fn detects_each_quota_signal_on_its_own() {
        let cases = [
            (
                "responseStatus 429",
                200,
                r#"{"responseData":{"translatedText":"x"},"responseStatus":429,"responseDetails":"slow down"}"#,
                Expect::Quota("slow down"),
            ),
            (
                "responseStatus \"429\"",
                200,
                r#"{"responseData":{"translatedText":"x"},"responseStatus":"429"}"#,
                Expect::Quota("x"),
            ),
            (
                "quotaFinished",
                200,
                r#"{"responseData":{"translatedText":"Haus"},"quotaFinished":true,"responseStatus":200}"#,
                Expect::Quota("Haus"),
            ),
            (
                "warning in responseDetails only",
                200,
                r#"{"responseData":{"translatedText":""},"responseDetails":"MyMemory warning: You used all available free translations for today.","responseStatus":200}"#,
                Expect::Quota("for today"),
            ),
            (
                "warning in a non-JSON body",
                200,
                "MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY",
                Expect::Quota("quota is used up"),
            ),
            ("bare HTTP 429", 429, "", Expect::Quota("quota is used up")),
        ];
        for (name, status, body, expect) in &cases {
            check(name, *status, body, expect);
        }
    }

    #[test]
    fn maps_other_failures() {
        let cases = [
            ("not JSON", 200, "<html>oops</html>", Expect::Malformed),
            ("empty body", 200, "", Expect::Malformed),
            (
                "no responseData",
                200,
                r#"{"responseStatus":200}"#,
                Expect::Malformed,
            ),
            (
                "translatedText is not a string",
                200,
                r#"{"responseData":{"translatedText":null},"responseStatus":200}"#,
                Expect::Malformed,
            ),
            (
                "unparsable responseStatus",
                200,
                r#"{"responseData":{"translatedText":"Haus"},"responseStatus":"OK"}"#,
                Expect::Malformed,
            ),
            (
                "blank translation",
                200,
                r#"{"responseData":{"translatedText":"  "},"responseStatus":200}"#,
                Expect::NotFound,
            ),
            (
                "HTTP 503 with an HTML page",
                503,
                "<html>down</html>",
                Expect::Http(503, "Unavailable"),
            ),
            (
                "HTTP 500 without a body",
                500,
                "",
                Expect::Http(500, "Internal Server Error"),
            ),
            (
                "API status 500 without details",
                200,
                r#"{"responseData":{"translatedText":""},"responseStatus":500}"#,
                Expect::Http(500, "responseStatus 500"),
            ),
            (
                "missing responseStatus is fine",
                200,
                r#"{"responseData":{"translatedText":" Haus "}}"#,
                Expect::Text("Haus"),
            ),
        ];
        for (name, status, body, expect) in &cases {
            check(name, *status, body, expect);
        }
    }

    #[test]
    fn decodes_html_entities_in_one_pass() {
        let cases = [
            ("Tom &amp; Jerry", "Tom & Jerry"),
            ("&lt;b&gt;bold&lt;/b&gt;", "<b>bold</b>"),
            ("&quot;Haus&quot;", "\"Haus\""),
            ("it&#39;s", "it's"),
            ("it&apos;s", "it's"),
            ("Gr&#252;&#223;e", "Grüße"),
            ("&#xE4;&#XC4;&#x00e4;", "äÄä"),
            ("&#128512;", "😀"),
            ("&amp;lt;", "&lt;"),
            ("&&amp;", "&&"),
            ("AT&T", "AT&T"),
            ("fish & chips; peas", "fish & chips; peas"),
            ("trailing &", "trailing &"),
            ("&amp", "&amp"),
            ("&nbsp;", "&nbsp;"),
            ("&#;", "&#;"),
            ("&#x;", "&#x;"),
            ("&#+39;", "&#+39;"),
            ("&#xD800;", "&#xD800;"),
            ("&#0;", "&#0;"),
            ("&#99999999999;", "&#99999999999;"),
            ("&averyveryverylongname;", "&averyveryverylongname;"),
            ("学习 &amp; 工作", "学习 & 工作"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(decode_html_entities(input), expected, "decode({input:?})");
        }
    }

    #[test]
    fn request_url_encodes_text_and_sends_email_only_when_set() {
        let with_email =
            MyMemoryProvider::with_base_url("http://127.0.0.1:9", Some(" me@example.com ".into()));
        let cases = [
            (
                "serendipity",
                "q=serendipity&langpair=en%7Cde&de=me%40example.com",
            ),
            ("naïve", "q=na%C3%AFve&langpair=en%7Cde&de=me%40example.com"),
            (
                "rock'n'roll",
                "q=rock%27n%27roll&langpair=en%7Cde&de=me%40example.com",
            ),
            ("C++", "q=C%2B%2B&langpair=en%7Cde&de=me%40example.com"),
            (
                "a&b=c d",
                "q=a%26b%3Dc+d&langpair=en%7Cde&de=me%40example.com",
            ),
        ];
        for (text, query) in cases {
            let url = with_email.request_url(text, "en", "de").unwrap();
            assert_eq!(url.path(), "/get");
            assert_eq!(url.query(), Some(query), "request_url({text:?})");
        }

        for email in [None, Some(String::new()), Some("   ".to_string())] {
            let provider = MyMemoryProvider::with_base_url("http://127.0.0.1:9", email.clone());
            let url = provider.request_url("Haus", "de", "zh-CN").unwrap();
            assert_eq!(
                url.query(),
                Some("q=Haus&langpair=de%7Czh-CN"),
                "email {email:?}"
            );
        }
    }
}
