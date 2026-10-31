//! HTTP-level tests of the network providers against a local wiremock server:
//! request shape (path, query encoding, User-Agent), response mapping end to
//! end, and transport failures. Nothing here reaches the real network.

use std::net::TcpListener;

use vocab_core::{
    providers::{
        http::{BROWSER_USER_AGENT, USER_AGENT},
        DatamuseProvider, DictionaryApiProvider, ExampleProvider, GoogleTtsProvider,
        MyMemoryProvider, RarityProvider, RarityResult, TranslationProvider, TtsProvider,
    },
    ProviderError,
};
use wiremock::{
    matchers::{any, header, method, path, query_param, query_param_is_missing},
    Mock, MockServer, ResponseTemplate,
};

const MYMEMORY_SUCCESS: &str = include_str!("fixtures/mymemory/success.json");
const MYMEMORY_QUOTA: &str = include_str!("fixtures/mymemory/quota_exceeded.json");
const MYMEMORY_QUOTA_200: &str = include_str!("fixtures/mymemory/quota_warning_status_200.json");
const MYMEMORY_INVALID_TARGET: &str =
    include_str!("fixtures/mymemory/invalid_target_language.json");
const MYMEMORY_INVALID_SOURCE: &str =
    include_str!("fixtures/mymemory/invalid_source_language.json");
const MYMEMORY_STATUS_STRING: &str = include_str!("fixtures/mymemory/status_as_string.json");
const MYMEMORY_ENTITIES: &str = include_str!("fixtures/mymemory/html_entities.json");
const DATAMUSE_SERENDIPITY: &str = include_str!("fixtures/datamuse/frequency_serendipity.json");
const DATAMUSE_HOUSE: &str = include_str!("fixtures/datamuse/frequency_house.json");
const DATAMUSE_SYNONYMS: &str = include_str!("fixtures/datamuse/synonyms_serendipity.json");
const DATAMUSE_EMPTY: &str = include_str!("fixtures/datamuse/empty.json");
const DICTIONARY_HELLO: &str = include_str!("fixtures/dictionary_api/hello.json");
const DICTIONARY_NO_EXAMPLE: &str = include_str!("fixtures/dictionary_api/no_example.json");
const DICTIONARY_NOT_FOUND: &str = include_str!("fixtures/dictionary_api/not_found.json");

fn json(status: u16, body: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_raw(body.as_bytes().to_vec(), "application/json")
}

/// A base URL nobody listens on: bind an ephemeral port, then release it.
fn closed_port_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("local address").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

/// The raw (still percent-encoded) query strings the server received.
async fn raw_queries(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .map(|request| request.url.query().unwrap_or_default().to_string())
        .collect()
}

mod mymemory {
    use super::*;

    #[tokio::test]
    async fn sends_text_langpair_email_and_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/get"))
            .and(query_param("q", "serendipity"))
            .and(query_param("langpair", "en|de"))
            .and(query_param("de", "me@example.com"))
            .and(header("user-agent", USER_AGENT))
            .respond_with(json(200, MYMEMORY_SUCCESS))
            .expect(1)
            .mount(&server)
            .await;

        let provider =
            MyMemoryProvider::with_base_url(server.uri(), Some("  me@example.com ".into()));
        assert_eq!(provider.name(), "mymemory");
        assert_eq!(
            provider.translate("  serendipity\n", "en", "de").await,
            Ok("Serendipität".to_string())
        );
    }

    #[tokio::test]
    async fn omits_de_without_a_usable_contact_email() {
        let server = MockServer::start().await;
        Mock::given(path("/get"))
            .and(query_param_is_missing("de"))
            .and(query_param("langpair", "de|zh-CN"))
            .respond_with(json(200, MYMEMORY_STATUS_STRING))
            .expect(3)
            .mount(&server)
            .await;

        for email in [None, Some(String::new()), Some(" \t ".to_string())] {
            let provider = MyMemoryProvider::with_base_url(server.uri(), email);
            assert_eq!(
                provider.translate("Haus", "de", "zh-CN").await,
                Ok("Haus".to_string())
            );
        }
    }

    #[tokio::test]
    async fn encodes_user_text_instead_of_interpolating_it() {
        let server = MockServer::start().await;
        let texts = [
            "naïve",
            "rock'n'roll",
            "C++",
            "fish & chips=yes",
            "50% off?",
        ];
        for text in texts {
            // query_param compares decoded values: with raw interpolation
            // "C++" would arrive as "C  " and "&"/"=" would split the query.
            Mock::given(path("/get"))
                .and(query_param("q", text))
                .respond_with(json(200, MYMEMORY_STATUS_STRING))
                .expect(1)
                .mount(&server)
                .await;
        }
        let provider = MyMemoryProvider::with_base_url(server.uri(), None);
        for text in texts {
            assert_eq!(
                provider.translate(text, "en", "de").await,
                Ok("Haus".to_string()),
                "{text}"
            );
        }
        let queries = raw_queries(&server).await;
        assert!(
            queries.contains(&"q=C%2B%2B&langpair=en%7Cde".to_string()),
            "{queries:?}"
        );
        assert!(
            queries.contains(&"q=na%C3%AFve&langpair=en%7Cde".to_string()),
            "{queries:?}"
        );
    }

    #[tokio::test]
    async fn maps_quota_signals_to_quota_exceeded() {
        let cases = [
            (
                "all signals, status 200",
                200,
                MYMEMORY_QUOTA,
                "NEXT AVAILABLE IN",
            ),
            (
                "all signals, status 429",
                429,
                MYMEMORY_QUOTA,
                "NEXT AVAILABLE IN",
            ),
            (
                "warning text with status 200",
                200,
                MYMEMORY_QUOTA_200,
                "23 HOURS",
            ),
        ];
        for (name, status, body, needle) in cases {
            let server = MockServer::start().await;
            Mock::given(path("/get"))
                .respond_with(json(status, body))
                .mount(&server)
                .await;
            let provider = MyMemoryProvider::with_base_url(server.uri(), None);
            match provider.translate("serendipity", "en", "de").await {
                Err(err @ ProviderError::QuotaExceeded { .. }) => {
                    assert!(err.is_quota_exceeded());
                    assert!(err.to_string().contains(needle), "{name}: {err}");
                }
                other => panic!("{name}: expected QuotaExceeded, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn invalid_languages_are_http_errors_with_the_service_message() {
        let cases = [
            (
                "target, HTTP 200 + string 403",
                200,
                MYMEMORY_INVALID_TARGET,
                "IS AN INVALID TARGET LANGUAGE",
            ),
            (
                "source, HTTP 403 + number 403",
                403,
                MYMEMORY_INVALID_SOURCE,
                "INVALID SOURCE LANGUAGE",
            ),
        ];
        for (name, status, body, needle) in cases {
            let server = MockServer::start().await;
            Mock::given(path("/get"))
                .respond_with(json(status, body))
                .mount(&server)
                .await;
            let provider = MyMemoryProvider::with_base_url(server.uri(), None);
            match provider.translate("Haus", "de", "xx").await {
                Err(ProviderError::Http {
                    provider,
                    status,
                    message,
                }) => {
                    assert_eq!(provider, "mymemory", "{name}");
                    assert_eq!(status, 403, "{name}");
                    assert!(message.contains(needle), "{name}: {message}");
                }
                other => panic!("{name}: expected Http, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn decodes_entities_and_accepts_string_status() {
        let server = MockServer::start().await;
        Mock::given(query_param("q", "entities"))
            .respond_with(json(200, MYMEMORY_ENTITIES))
            .mount(&server)
            .await;
        Mock::given(query_param("q", "house"))
            .respond_with(json(200, MYMEMORY_STATUS_STRING))
            .mount(&server)
            .await;
        let provider = MyMemoryProvider::with_base_url(server.uri(), None);
        assert_eq!(
            provider.translate("entities", "en", "de").await,
            Ok("Tom & Jerry's \"Haus\" <3 über ärger".to_string())
        );
        assert_eq!(
            provider.translate("house", "en", "de").await,
            Ok("Haus".to_string())
        );
    }

    #[tokio::test]
    async fn malformed_bodies_and_server_errors_are_typed() {
        let server = MockServer::start().await;
        Mock::given(query_param("q", "html"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>maintenance</html>"))
            .mount(&server)
            .await;
        Mock::given(query_param("q", "shape"))
            .respond_with(json(200, r#"{"responseStatus":200,"responseData":{}}"#))
            .mount(&server)
            .await;
        Mock::given(query_param("q", "down"))
            .respond_with(ResponseTemplate::new(503).set_body_string("<html>busy</html>"))
            .mount(&server)
            .await;
        let provider = MyMemoryProvider::with_base_url(server.uri(), None);
        for text in ["html", "shape"] {
            assert!(
                matches!(
                    provider.translate(text, "en", "de").await,
                    Err(ProviderError::Malformed { .. })
                ),
                "{text}"
            );
        }
        assert_eq!(
            provider.translate("down", "en", "de").await,
            Err(ProviderError::Http {
                provider: "mymemory".into(),
                status: 503,
                message: "Service Unavailable".into()
            })
        );
    }

    #[tokio::test]
    async fn unreachable_server_is_a_network_error_without_user_data() {
        let provider =
            MyMemoryProvider::with_base_url(closed_port_url(), Some("me@example.com".into()));
        match provider.translate("secret phrase", "en", "de").await {
            Err(ProviderError::Network { provider, message }) => {
                assert_eq!(provider, "mymemory");
                // reqwest's own Display stops at "error sending request"; the
                // cause chain is what tells the user it was the connection.
                assert!(message.starts_with("error sending request: "), "{message}");
                assert!(message.contains("tcp connect error"), "{message}");
                // The request URL carries the text and the e-mail; neither may
                // leak into a message that is shown and logged.
                for secret in ["secret", "example.com", "127.0.0.1"] {
                    assert!(!message.contains(secret), "{message}");
                }
            }
            other => panic!("expected Network, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn blank_text_is_unsupported_and_sends_nothing() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(json(200, MYMEMORY_SUCCESS))
            .expect(0)
            .mount(&server)
            .await;
        let provider = MyMemoryProvider::with_base_url(server.uri(), None);
        for text in ["", "  \n"] {
            assert!(matches!(
                provider.translate(text, "en", "de").await,
                Err(ProviderError::Unsupported { .. })
            ));
        }
    }
}

mod datamuse {
    use super::*;

    #[tokio::test]
    async fn rare_word_gets_frequency_and_synonym() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/words"))
            .and(query_param("sp", "serendipity"))
            .and(query_param("md", "f"))
            .and(query_param("max", "1"))
            .and(header("user-agent", USER_AGENT))
            .respond_with(json(200, DATAMUSE_SERENDIPITY))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/words"))
            .and(query_param("rel_syn", "serendipity"))
            .and(query_param("max", "1"))
            .respond_with(json(200, DATAMUSE_SYNONYMS))
            .expect(1)
            .mount(&server)
            .await;

        let provider = DatamuseProvider::with_base_url(server.uri());
        assert_eq!(provider.name(), "datamuse");
        assert_eq!(
            provider.analyze("  Serendipity\n").await,
            Ok(RarityResult {
                is_rare: true,
                frequency_per_million: Some(0.292473),
                synonym: Some("luck".into()),
            })
        );
    }

    #[tokio::test]
    async fn common_and_unknown_words_skip_the_synonym_lookup() {
        let server = MockServer::start().await;
        Mock::given(query_param("sp", "house"))
            .respond_with(json(200, DATAMUSE_HOUSE))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(query_param("sp", "xyzzyq"))
            .respond_with(json(200, DATAMUSE_EMPTY))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(query_param_is_missing("sp"))
            .respond_with(json(200, DATAMUSE_SYNONYMS))
            .expect(0)
            .mount(&server)
            .await;

        let provider = DatamuseProvider::with_base_url(server.uri());
        assert_eq!(
            provider.analyze("house").await,
            Ok(RarityResult {
                is_rare: false,
                frequency_per_million: Some(313.926397),
                synonym: None,
            })
        );
        assert_eq!(
            provider.analyze("xyzzyq").await,
            Ok(RarityResult::default())
        );
    }

    #[tokio::test]
    async fn failed_synonym_lookup_keeps_the_rarity_verdict() {
        let server = MockServer::start().await;
        Mock::given(query_param("sp", "serendipity"))
            .respond_with(json(200, DATAMUSE_SERENDIPITY))
            .mount(&server)
            .await;
        Mock::given(query_param("rel_syn", "serendipity"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let provider = DatamuseProvider::with_base_url(server.uri());
        assert_eq!(
            provider.analyze("serendipity").await,
            Ok(RarityResult {
                is_rare: true,
                frequency_per_million: Some(0.292473),
                synonym: None,
            })
        );
    }

    #[tokio::test]
    async fn phrases_are_unsupported_and_send_nothing() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(json(200, DATAMUSE_EMPTY))
            .expect(0)
            .mount(&server)
            .await;
        let provider = DatamuseProvider::with_base_url(server.uri());
        for phrase in ["ice cream", " ice\tcream ", "line\nbreak"] {
            assert_eq!(
                provider.analyze(phrase).await,
                Err(ProviderError::Unsupported {
                    provider: "datamuse".into(),
                    what: "multi-word phrases".into()
                }),
                "{phrase:?}"
            );
        }
    }

    #[tokio::test]
    async fn encodes_the_word_and_maps_http_errors() {
        let server = MockServer::start().await;
        Mock::given(query_param("sp", "rock'n'roll"))
            .respond_with(json(200, DATAMUSE_EMPTY))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(query_param("sp", "broken"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let provider = DatamuseProvider::with_base_url(server.uri());
        assert_eq!(
            provider.analyze("rock'n'roll").await,
            Ok(RarityResult::default())
        );
        assert!(raw_queries(&server)
            .await
            .contains(&"sp=rock%27n%27roll&md=f&max=1".to_string()));
        assert!(matches!(
            provider.analyze("broken").await,
            Err(ProviderError::Http { status: 500, .. })
        ));
    }

    #[tokio::test]
    async fn unreachable_server_is_a_network_error() {
        let provider = DatamuseProvider::with_base_url(closed_port_url());
        assert!(matches!(
            provider.analyze("serendipity").await,
            Err(ProviderError::Network { .. })
        ));
    }
}

mod dictionary_api {
    use super::*;

    #[tokio::test]
    async fn returns_the_first_example_of_the_entry() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v2/entries/en/hello"))
            .and(header("user-agent", USER_AGENT))
            .respond_with(json(200, DICTIONARY_HELLO))
            .expect(1)
            .mount(&server)
            .await;
        let provider = DictionaryApiProvider::with_base_url(server.uri());
        assert_eq!(provider.name(), "dictionary_api");
        assert_eq!(
            provider.fetch_example(" Hello ").await,
            Ok("Hello, everyone.".to_string())
        );
    }

    #[tokio::test]
    async fn not_found_and_example_less_entries_are_not_found() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v2/entries/en/xyzzyq"))
            .respond_with(json(404, DICTIONARY_NOT_FOUND))
            .mount(&server)
            .await;
        Mock::given(path("/api/v2/entries/en/hello"))
            .respond_with(json(200, DICTIONARY_NO_EXAMPLE))
            .mount(&server)
            .await;
        let provider = DictionaryApiProvider::with_base_url(server.uri());
        for word in ["xyzzyq", "hello"] {
            assert_eq!(
                provider.fetch_example(word).await,
                Err(ProviderError::NotFound {
                    provider: "dictionary_api".into()
                }),
                "{word}"
            );
        }
    }

    #[tokio::test]
    async fn keeps_the_word_in_one_path_segment() {
        let server = MockServer::start().await;
        // wiremock's path matcher compares the encoded path.
        for encoded in [
            "/api/v2/entries/en/and%2For",
            "/api/v2/entries/en/na%C3%AFve",
        ] {
            Mock::given(path(encoded))
                .respond_with(json(200, DICTIONARY_HELLO))
                .expect(1)
                .mount(&server)
                .await;
        }
        let provider = DictionaryApiProvider::with_base_url(format!("{}/", server.uri()));
        for word in ["and/or", "Naïve"] {
            assert!(provider.fetch_example(word).await.is_ok(), "{word}");
        }
    }

    #[tokio::test]
    async fn dot_segments_are_unsupported_and_send_nothing() {
        // URL syntax would resolve "." and ".." away, and the request would
        // ask for /api/v2/entries/en without the word.
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(json(200, DICTIONARY_HELLO))
            .expect(0)
            .mount(&server)
            .await;
        let provider = DictionaryApiProvider::with_base_url(server.uri());
        for word in [".", " .. "] {
            assert!(
                matches!(
                    provider.fetch_example(word).await,
                    Err(ProviderError::Unsupported { .. })
                ),
                "{word:?}"
            );
        }
    }

    #[tokio::test]
    async fn server_errors_and_unreachable_servers_are_typed() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
            .mount(&server)
            .await;
        let provider = DictionaryApiProvider::with_base_url(server.uri());
        assert!(matches!(
            provider.fetch_example("hello").await,
            Err(ProviderError::Http { status: 502, .. })
        ));

        let unreachable = DictionaryApiProvider::with_base_url(closed_port_url());
        assert!(matches!(
            unreachable.fetch_example("hello").await,
            Err(ProviderError::Network { .. })
        ));
    }
}

mod google_tts {
    use super::*;

    const MP3: &[u8] = &[0xFF, 0xF3, 0x44, 0xC4, 0x00, 0x00];

    #[tokio::test]
    async fn sends_the_language_verbatim_with_a_browser_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/translate_tts"))
            .and(query_param("ie", "UTF-8"))
            .and(query_param("tl", "zh-CN"))
            .and(query_param("client", "tw-ob"))
            .and(query_param("q", "你好"))
            .and(header("user-agent", BROWSER_USER_AGENT))
            .respond_with(ResponseTemplate::new(200).set_body_raw(MP3.to_vec(), "audio/mpeg"))
            .expect(1)
            .mount(&server)
            .await;
        let provider = GoogleTtsProvider::with_base_url(server.uri());
        assert_eq!(provider.name(), "google_tts");
        assert_eq!(
            provider.synthesize(" 你好 ", "zh-CN").await,
            Ok(MP3.to_vec())
        );
    }

    #[tokio::test]
    async fn empty_or_non_audio_responses_are_malformed() {
        let server = MockServer::start().await;
        Mock::given(query_param("q", "empty"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(Vec::new(), "audio/mpeg"))
            .mount(&server)
            .await;
        Mock::given(query_param("q", "html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(b"<html>unusual traffic</html>".to_vec(), "text/html"),
            )
            .mount(&server)
            .await;
        let provider = GoogleTtsProvider::with_base_url(server.uri());
        for text in ["empty", "html"] {
            assert!(
                matches!(
                    provider.synthesize(text, "en").await,
                    Err(ProviderError::Malformed { .. })
                ),
                "{text}"
            );
        }
    }

    #[tokio::test]
    async fn http_failures_are_typed() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(500).set_body_string("<html>oops</html>"))
            .mount(&server)
            .await;
        let provider = GoogleTtsProvider::with_base_url(server.uri());
        assert_eq!(
            provider.synthesize("hello", "en").await,
            Err(ProviderError::Http {
                provider: "google_tts".into(),
                status: 500,
                message: "Internal Server Error".into()
            })
        );

        let unreachable = GoogleTtsProvider::with_base_url(closed_port_url());
        assert!(matches!(
            unreachable.synthesize("hello", "en").await,
            Err(ProviderError::Network { .. })
        ));
    }
}
