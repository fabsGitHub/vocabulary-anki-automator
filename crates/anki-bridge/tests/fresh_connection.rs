//! Every AnkiConnect request goes out on a connection of its own (see
//! `AnkiClient::new`).
//!
//! AnkiConnect's web server closes the socket once it has written a response,
//! but sends no `Connection: close` header, so an HTTP/1.1 client that pools
//! connections keeps the socket as keep-alive. A request sent on it before the
//! client has noticed the server's FIN reaches a closed socket and fails with
//! "Connection reset by peer". With the real server that is a race the client
//! sometimes wins (it hit `addNote` right after `createDeck`); the fake here
//! makes the client lose it every time: it answers the first request on a
//! connection like AnkiConnect does, and resets the connection as soon as a
//! second request arrives on it, which is what the real server's closed
//! socket does.
//!
//! Not wiremock: its server keeps connections alive, so a reused connection
//! works there and the bug would never show.

use std::sync::{Arc, Mutex};

use anki_bridge::{push_card, AnkiClient, AnkiTarget, FieldMapping, PushOptions};
use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use vocab_core::Card;

const DECK_ID: i64 = 1_519_323_742_721;
const NOTE_ID: i64 = 1_496_198_395_707;

/// What the fake saw, shared with its connection tasks.
#[derive(Default)]
struct Seen {
    /// The `action` of every request it answered, in order.
    actions: Vec<String>,
    /// Requests that arrived on a connection that had already been answered.
    reused: usize,
}

struct FakeAnkiConnect {
    url: String,
    seen: Arc<Mutex<Seen>>,
}

impl FakeAnkiConnect {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let url = format!("http://{}", listener.local_addr().expect("local address"));
        let seen = Arc::new(Mutex::new(Seen::default()));
        let tasks_seen = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve_connection(stream, Arc::clone(&tasks_seen)));
            }
        });
        Self { url, seen }
    }

    fn actions(&self) -> Vec<String> {
        self.seen.lock().unwrap().actions.clone()
    }

    fn reused(&self) -> usize {
        self.seen.lock().unwrap().reused
    }
}

/// One AnkiConnect connection: one request, one response, and nothing after.
async fn serve_connection(mut stream: TcpStream, seen: Arc<Mutex<Seen>>) {
    let Some(body) = read_request(&mut stream).await else {
        return;
    };
    let action = body["action"].as_str().unwrap_or_default().to_string();
    let result = match action.as_str() {
        "version" => json!(6),
        "deckNames" => json!(["Default", "english"]),
        "modelNames" => json!(["Basic", "AppleStyleFinalV3"]),
        "createDeck" => json!(DECK_ID),
        "addNote" => json!(NOTE_ID),
        _ => Value::Null,
    };
    seen.lock().unwrap().actions.push(action);
    // AnkiConnect's headers: a Content-Length and no Connection header.
    let payload = json!({ "result": result, "error": null }).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
        payload.len()
    );
    if stream.write_all(response.as_bytes()).await.is_err() {
        return;
    }
    // The real server has closed its socket at this point. A client that
    // closes too (end of file) is fine; one that sends another request gets
    // the reset a closed socket answers with.
    let mut next = [0_u8; 1];
    if matches!(stream.read(&mut next).await, Ok(n) if n > 0) {
        seen.lock().unwrap().reused += 1;
        stream.set_zero_linger().expect("set SO_LINGER 0");
        drop(stream);
    }
}

/// The JSON body of one HTTP request, or `None` when the client closed the
/// connection before sending a whole one.
async fn read_request(stream: &mut TcpStream) -> Option<Value> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        let n = stream.read(&mut chunk).await.ok().filter(|&n| n > 0)?;
        buffer.extend_from_slice(&chunk[..n]);
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
    let length: usize = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + length {
        let n = stream.read(&mut chunk).await.ok().filter(|&n| n > 0)?;
        buffer.extend_from_slice(&chunk[..n]);
    }
    serde_json::from_slice(&buffer[header_end..header_end + length]).ok()
}

fn card() -> Card {
    let captured_at = Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap();
    Card::new("serendipity", "glücklicher Zufall", "en", "de", captured_at)
}

#[tokio::test]
async fn a_push_sends_create_deck_and_add_note_on_connections_of_their_own() {
    let anki = FakeAnkiConnect::start().await;

    let outcome = push_card(
        &AnkiClient::new(anki.url.clone()),
        &card(),
        &AnkiTarget::new("english", "AppleStyleFinalV3"),
        &FieldMapping::apple_style_default(),
        &PushOptions::default(),
    )
    .await
    .expect("both requests reach AnkiConnect");

    assert_eq!(outcome.note_ids, vec![NOTE_ID]);
    assert_eq!(anki.actions(), ["createDeck", "addNote"]);
    assert_eq!(
        anki.reused(),
        0,
        "a request went out on an answered connection"
    );
}

#[tokio::test]
async fn back_to_back_status_requests_all_succeed() {
    // `anki_status` asks for the version, the decks and the note types in a
    // row, with one client.
    let anki = FakeAnkiConnect::start().await;
    let client = AnkiClient::new(anki.url.clone());

    assert_eq!(client.version().await, Ok(6));
    assert_eq!(
        client.deck_names().await,
        Ok(vec!["Default".to_string(), "english".to_string()])
    );
    assert_eq!(
        client.model_names().await,
        Ok(vec!["Basic".to_string(), "AppleStyleFinalV3".to_string()])
    );
    assert_eq!(anki.actions(), ["version", "deckNames", "modelNames"]);
    assert_eq!(
        anki.reused(),
        0,
        "a request went out on an answered connection"
    );
}
