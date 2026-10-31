//! AnkiConnect client.
//!
//! AnkiConnect (pinned in docs/architecture.md § 8.4) has one protocol for
//! every action: POST `{"action", "version": 6, "params"}`, read back
//! `{"result", "error"}`. Every way that exchange can go wrong is classified
//! into an [`AnkiError`] here, so no caller parses envelopes or error strings
//! itself. (The pre-refactor client returned `String` errors and ignored the
//! results of `createDeck` and `storeMediaFile` altogether.)
//!
//! Requests never go through a proxy: AnkiConnect is a loopback service, and a
//! system or environment proxy has no business seeing collection contents, or
//! turning "Anki is closed" into a proxy's error page.

use std::{collections::BTreeMap, time::Duration};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    error::AnkiError,
    mapper::{add_note_params, NoteSpec},
};

/// Where AnkiConnect listens unless configured otherwise.
pub const DEFAULT_URL: &str = "http://127.0.0.1:8765";

/// The AnkiConnect API version this client speaks.
pub const API_VERSION: u32 = 6;

/// Per-request timeout unless overridden (docs/architecture.md § 2).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// `notesInfo` is sent in batches of this many ids, so importing a large deck
/// never runs into the request timeout.
const NOTES_INFO_BATCH: usize = 500;

/// How much of an unexpected response body is quoted in an error message.
const BODY_SNIPPET_CHARS: usize = 120;

const USER_AGENT: &str = concat!(
    "vocabulary-anki-automator anki-bridge/",
    env!("CARGO_PKG_VERSION")
);

/// A client for one AnkiConnect endpoint. Cloning is cheap: clones share the
/// connection pool.
#[derive(Debug, Clone)]
pub struct AnkiClient {
    /// `Err` when the HTTP client could not be built (TLS backend
    /// initialization): every request then fails as `Unreachable` instead of
    /// the constructor panicking.
    http: Result<reqwest::Client, String>,
    url: String,
    timeout: Duration,
}

impl AnkiClient {
    /// A client for the AnkiConnect endpoint at `url` (normally
    /// [`DEFAULT_URL`]), with the default timeout.
    pub fn new(url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .no_proxy()
            .build()
            .map_err(|err| error_chain(&err));
        Self {
            http,
            url: url.into(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Overrides the per-request timeout, e.g. for a quick "is Anki running?"
    /// probe.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// AnkiConnect's API version (`version`).
    pub async fn version(&self) -> Result<u32, AnkiError> {
        self.invoke("version", None).await
    }

    /// Every deck name; subdecks appear as `Parent::Child`.
    pub async fn deck_names(&self) -> Result<Vec<String>, AnkiError> {
        self.invoke("deckNames", None).await
    }

    /// The note types ("models") in the collection.
    pub async fn model_names(&self) -> Result<Vec<String>, AnkiError> {
        self.invoke("modelNames", None).await
    }

    /// The field names of note type `model`, in order.
    pub async fn model_field_names(&self, model: &str) -> Result<Vec<String>, AnkiError> {
        self.invoke("modelFieldNames", Some(json!({ "modelName": model })))
            .await
    }

    /// Creates `deck` (`::` separates subdecks) and returns its id. AnkiConnect
    /// returns the existing deck's id when it already exists.
    pub async fn create_deck(&self, deck: &str) -> Result<i64, AnkiError> {
        self.invoke("createDeck", Some(json!({ "deck": deck })))
            .await
    }

    /// Adds one note and returns its id. A note Anki already has in the
    /// target deck fails with [`AnkiError::Duplicate`].
    pub async fn add_note(&self, note: &NoteSpec) -> Result<i64, AnkiError> {
        self.invoke("addNote", Some(add_note_params(note))).await
    }

    /// Note ids matching an Anki search query (see [`deck_query`]).
    pub async fn find_notes(&self, query: &str) -> Result<Vec<i64>, AnkiError> {
        self.invoke("findNotes", Some(json!({ "query": query })))
            .await
    }

    /// The notes with the given ids. Ids that no longer exist are skipped
    /// (AnkiConnect reports them as `{}`); an empty slice sends no request.
    pub async fn notes_info(&self, ids: &[i64]) -> Result<Vec<NoteInfo>, AnkiError> {
        let mut notes = Vec::with_capacity(ids.len());
        for batch in ids.chunks(NOTES_INFO_BATCH) {
            let entries: Vec<Value> = self
                .invoke("notesInfo", Some(json!({ "notes": batch })))
                .await?;
            for entry in entries {
                notes.extend(parse_note_info(entry)?);
            }
        }
        Ok(notes)
    }

    async fn invoke<T: DeserializeOwned>(
        &self,
        action: &str,
        params: Option<Value>,
    ) -> Result<T, AnkiError> {
        let http = self
            .http
            .as_ref()
            .map_err(|message| AnkiError::Unreachable {
                message: format!("could not create the HTTP client: {message}"),
            })?;
        log::debug!("AnkiConnect {action} -> {}", self.url);
        let response = http
            .post(&self.url)
            .timeout(self.timeout)
            .json(&request_body(action, params))
            .send()
            .await
            .map_err(|err| self.transport_error(&err))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|err| self.transport_error(&err))?;
        parse_response(action, status, &body)
    }

    fn transport_error(&self, err: &reqwest::Error) -> AnkiError {
        let message = if err.is_timeout() {
            format!("no response from {} within {:?}", self.url, self.timeout)
        } else {
            error_chain(err)
        };
        AnkiError::Unreachable { message }
    }
}

/// One note as AnkiConnect's `notesInfo` reports it (camelCase on the wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteInfo {
    pub note_id: i64,
    #[serde(default)]
    pub model_name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Field name -> content, as stored by Anki (HTML).
    pub fields: BTreeMap<String, NoteField>,
    /// Ids of the cards Anki generated from this note.
    #[serde(default)]
    pub cards: Vec<i64>,
    /// Last modification, in seconds since the Unix epoch (`mod`).
    #[serde(default, rename = "mod")]
    pub modified: Option<i64>,
}

impl NoteInfo {
    /// The content of field `name`: an exact name match first, then a
    /// case-insensitive one, the way AnkiConnect matches field names.
    pub fn field(&self, name: &str) -> Option<&str> {
        if let Some(field) = self.fields.get(name) {
            return Some(&field.value);
        }
        let lowered = name.to_lowercase();
        self.fields
            .iter()
            .find(|(key, _)| key.to_lowercase() == lowered)
            .map(|(_, field)| field.value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteField {
    pub value: String,
    /// Position of the field in the note type.
    #[serde(default)]
    pub order: u32,
}

/// An Anki search matching every note in `deck` and its subdecks, for
/// [`AnkiClient::find_notes`]. Anki's wildcards (`*`, `_`), quotes and
/// backslashes are escaped, so the deck name matches literally.
pub fn deck_query(deck: &str) -> String {
    let mut query = String::with_capacity(deck.len() + 8);
    query.push_str("\"deck:");
    for c in deck.chars() {
        if matches!(c, '\\' | '"' | '*' | '_') {
            query.push('\\');
        }
        query.push(c);
    }
    query.push('"');
    query
}

/// The request envelope. Actions without parameters (`version`, `deckNames`,
/// `modelNames`) omit the `params` key entirely.
fn request_body(action: &str, params: Option<Value>) -> Value {
    let mut body = json!({ "action": action, "version": API_VERSION });
    if let Some(params) = params {
        body["params"] = params;
    }
    body
}

/// Classifies a response: an envelope `error` is `Api` (or `Duplicate`), a
/// body that is not an envelope, a non-2xx status without an error, or a
/// result of the wrong type is `Malformed`.
fn parse_response<T: DeserializeOwned>(
    action: &str,
    status: u16,
    body: &[u8],
) -> Result<T, AnkiError> {
    let malformed = |detail: String| AnkiError::Malformed {
        message: format!("{action}: {detail}"),
    };
    let mut envelope = match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(envelope))
            if envelope.contains_key("result") || envelope.contains_key("error") =>
        {
            envelope
        }
        _ => {
            return Err(malformed(format!(
                "HTTP {status} with {}, not an AnkiConnect response",
                snippet(body)
            )))
        }
    };
    match envelope.remove("error") {
        None | Some(Value::Null) => {}
        Some(Value::String(message)) => return Err(AnkiError::from_api_message(message)),
        Some(other) => return Err(AnkiError::from_api_message(other.to_string())),
    }
    if !(200..300).contains(&status) {
        return Err(malformed(format!("HTTP {status} without an error message")));
    }
    let result = envelope.remove("result").unwrap_or(Value::Null);
    serde_json::from_value(result).map_err(|err| malformed(format!("unexpected result: {err}")))
}

fn parse_note_info(entry: Value) -> Result<Option<NoteInfo>, AnkiError> {
    match entry {
        Value::Object(fields) if fields.is_empty() => Ok(None),
        entry => serde_json::from_value(entry)
            .map(Some)
            .map_err(|err| AnkiError::Malformed {
                message: format!("notesInfo: unexpected note: {err}"),
            }),
    }
}

fn snippet(body: &[u8]) -> String {
    if body.is_empty() {
        return "an empty body".to_string();
    }
    let text = String::from_utf8_lossy(body);
    let mut quoted: String = text.chars().take(BODY_SNIPPET_CHARS).collect();
    if text.chars().nth(BODY_SNIPPET_CHARS).is_some() {
        quoted.push('…');
    }
    format!("body {quoted:?}")
}

/// `err` and its sources joined by ": " (skipping repeats): reqwest's own
/// message ("error sending request for url ...") hides the actual cause.
fn error_chain(err: &dyn std::error::Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_omits_params_for_parameterless_actions() {
        assert_eq!(
            request_body("deckNames", None),
            json!({ "action": "deckNames", "version": 6 })
        );
        assert_eq!(
            request_body("createDeck", Some(json!({ "deck": "A::B" }))),
            json!({ "action": "createDeck", "version": 6, "params": { "deck": "A::B" } })
        );
    }

    enum Expect {
        Ok(&'static [&'static str]),
        Api(&'static str),
        Duplicate(&'static str),
        /// Malformed, with a fragment its message must contain.
        Malformed(&'static str),
    }

    #[test]
    fn parse_response_table() {
        let cases: [(&str, u16, &str, Expect); 15] = [
            (
                "ok",
                200,
                r#"{"result": ["Default", "Vocab"], "error": null}"#,
                Expect::Ok(&["Default", "Vocab"]),
            ),
            (
                "ok without error key",
                200,
                r#"{"result": []}"#,
                Expect::Ok(&[]),
            ),
            (
                "duplicate",
                200,
                r#"{"result": null, "error": "cannot create note because it is a duplicate"}"#,
                Expect::Duplicate("cannot create note because it is a duplicate"),
            ),
            (
                "api error",
                200,
                r#"{"result": null, "error": "model was not found: Nope"}"#,
                Expect::Api("model was not found: Nope"),
            ),
            (
                "non-string error",
                200,
                r#"{"result": null, "error": {"code": 1}}"#,
                Expect::Api(r#"{"code":1}"#),
            ),
            (
                "api error with 500",
                500,
                r#"{"result": null, "error": "boom"}"#,
                Expect::Api("boom"),
            ),
            (
                "html",
                200,
                "<html>not anki</html>",
                Expect::Malformed("<html>not anki</html>"),
            ),
            ("empty body", 200, "", Expect::Malformed("an empty body")),
            (
                "json string",
                200,
                r#""Forbidden""#,
                Expect::Malformed("not an AnkiConnect response"),
            ),
            (
                "json array",
                200,
                "[1, 2]",
                Expect::Malformed("not an AnkiConnect response"),
            ),
            (
                "no envelope keys",
                200,
                r#"{"decks": []}"#,
                Expect::Malformed("not an AnkiConnect response"),
            ),
            (
                "wrong result type",
                200,
                r#"{"result": 5, "error": null}"#,
                Expect::Malformed("unexpected result"),
            ),
            (
                "null result",
                200,
                r#"{"result": null, "error": null}"#,
                Expect::Malformed("unexpected result"),
            ),
            (
                "500 without error",
                500,
                r#"{"result": ["x"], "error": null}"#,
                Expect::Malformed("HTTP 500"),
            ),
            ("403 empty", 403, "", Expect::Malformed("HTTP 403")),
        ];
        for (name, status, body, expect) in cases {
            let got = parse_response::<Vec<String>>("deckNames", status, body.as_bytes());
            match (expect, got) {
                (Expect::Ok(want), Ok(got)) => assert_eq!(got, want, "{name}"),
                (Expect::Api(want), Err(AnkiError::Api { message })) => {
                    assert_eq!(message, want, "{name}")
                }
                (Expect::Duplicate(want), Err(AnkiError::Duplicate { message })) => {
                    assert_eq!(message, want, "{name}")
                }
                (Expect::Malformed(fragment), Err(AnkiError::Malformed { message })) => {
                    assert!(message.starts_with("deckNames: "), "{name}: {message}");
                    assert!(message.contains(fragment), "{name}: {message}");
                }
                (_, got) => panic!("{name}: unexpected {got:?}"),
            }
        }
    }

    #[test]
    fn parse_response_reads_scalar_results() {
        assert_eq!(
            parse_response::<u32>("version", 200, br#"{"result": 6, "error": null}"#),
            Ok(6)
        );
        assert_eq!(
            parse_response::<i64>(
                "createDeck",
                200,
                br#"{"result": 1519323742721, "error": null}"#
            ),
            Ok(1_519_323_742_721)
        );
        assert!(matches!(
            parse_response::<u32>("version", 200, br#"{"result": "six", "error": null}"#),
            Err(AnkiError::Malformed { .. })
        ));
        assert!(matches!(
            parse_response::<u32>("version", 200, br#"{"result": -1, "error": null}"#),
            Err(AnkiError::Malformed { .. })
        ));
    }

    #[test]
    fn long_bodies_are_quoted_truncated() {
        let body = "x".repeat(500);
        let Err(AnkiError::Malformed { message }) =
            parse_response::<u32>("version", 200, body.as_bytes())
        else {
            panic!("expected Malformed");
        };
        assert!(
            message.contains(&format!("{}…", "x".repeat(120))),
            "{message}"
        );
        assert!(!message.contains(&"x".repeat(121)), "{message}");
    }

    #[test]
    fn notes_info_entries_parse_and_placeholders_are_skipped() {
        let entry = json!({
            "noteId": 1502298033753_i64,
            "profile": "User 1",
            "modelName": "AppleStyleFinalV3",
            "tags": ["vocab-automator"],
            "fields": {
                "Front": { "value": "serendipity", "order": 0 },
                "Back": { "value": "glücklicher Zufall", "order": 1 }
            },
            "mod": 1718377864,
            "cards": [1498938915662_i64]
        });
        let note = parse_note_info(entry).unwrap().unwrap();
        assert_eq!(
            note,
            NoteInfo {
                note_id: 1_502_298_033_753,
                model_name: "AppleStyleFinalV3".into(),
                tags: vec!["vocab-automator".into()],
                fields: BTreeMap::from([
                    (
                        "Front".to_string(),
                        NoteField {
                            value: "serendipity".into(),
                            order: 0
                        }
                    ),
                    (
                        "Back".to_string(),
                        NoteField {
                            value: "glücklicher Zufall".into(),
                            order: 1
                        }
                    ),
                ]),
                cards: vec![1_498_938_915_662],
                modified: Some(1_718_377_864),
            }
        );
        assert_eq!(note.field("Back"), Some("glücklicher Zufall"));
        assert_eq!(note.field("back"), Some("glücklicher Zufall"));
        assert_eq!(note.field("Notes"), None);

        assert_eq!(parse_note_info(json!({})), Ok(None));
        assert!(matches!(
            parse_note_info(json!({ "noteId": "x", "fields": {} })),
            Err(AnkiError::Malformed { .. })
        ));
        assert!(matches!(
            parse_note_info(json!(null)),
            Err(AnkiError::Malformed { .. })
        ));
    }

    #[test]
    fn deck_query_escapes_search_syntax() {
        let cases = [
            (
                "English::AutomatedVocabulary",
                r#""deck:English::AutomatedVocabulary""#,
            ),
            ("My Deck", r#""deck:My Deck""#),
            ("HSK_1*", r#""deck:HSK\_1\*""#),
            (r#"say "hi" \o/"#, r#""deck:say \"hi\" \\o/""#),
        ];
        for (deck, expected) in cases {
            assert_eq!(deck_query(deck), expected, "{deck}");
        }
    }

    #[test]
    fn timeout_defaults_to_ten_seconds() {
        let client = AnkiClient::new(DEFAULT_URL);
        assert_eq!(client.timeout(), Duration::from_secs(10));
        assert_eq!(client.url(), "http://127.0.0.1:8765");
        let quick = client.with_timeout(Duration::from_millis(1500));
        assert_eq!(quick.timeout(), Duration::from_millis(1500));
    }
}
