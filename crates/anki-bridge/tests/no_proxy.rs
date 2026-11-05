//! AnkiConnect traffic must never go through a proxy (see `client.rs`). This
//! points every proxy variable reqwest honours at a dead address and checks
//! that requests still reach the mock AnkiConnect directly; without
//! `ClientBuilder::no_proxy` reqwest would send them to the proxy, because it
//! does not exempt loopback addresses on its own.
//!
//! A test binary of its own, with a single test, because it changes the
//! process-wide environment.

use anki_bridge::AnkiClient;
use serde_json::json;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn proxy_environment_variables_are_ignored() {
    // Port 9 (discard) has no listener: a proxied request fails to connect.
    let dead_proxy = "http://127.0.0.1:9";
    for var in ["NO_PROXY", "no_proxy", "REQUEST_METHOD"] {
        std::env::remove_var(var);
    }
    for var in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        std::env::set_var(var, dead_proxy);
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "result": 6, "error": null })),
        )
        .mount(&server)
        .await;

    assert_eq!(AnkiClient::new(server.uri()).version().await, Ok(6));
}
