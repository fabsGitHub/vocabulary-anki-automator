//! Downloads for setup: streamed to disk under a size cap, hashed while they
//! stream, and checked against a pinned SHA-256 before anything else reads
//! the file (Milestone E).
//!
//! The transport sits behind [`Downloader`], which only hands out the body
//! chunk by chunk. The parts that matter for security (the cap, the hash, the
//! comparison, deleting a bad file) live in [`download_verified`] and run
//! unchanged when tests plug in a fake transport. The pre-refactor code read
//! whole responses into memory, wrote them to fixed names in the shared temp
//! dir and ran them unverified.

use std::{error::Error as StdError, fs, io, io::Write, path::Path, time::Duration};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use super::SetupError;

/// Limit for connecting to the download host.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest silence between two chunks. A whole-request timeout would cut
/// off a large installer on a slow line, so only stalls count.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

const USER_AGENT: &str = concat!(
    "vocabulary-automator/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/fabsGitHub/vocabulary-anki-automator)"
);

/// Fetches a URL whose body is read piece by piece.
#[async_trait]
pub trait Downloader: Send + Sync {
    /// Starts a GET of `url`. Non-success statuses are errors.
    async fn get(&self, url: &str) -> Result<Box<dyn DownloadBody>, SetupError>;
}

/// The body of a started download.
#[async_trait]
pub trait DownloadBody: Send {
    /// The length the server announced, if any. Only used to give up early;
    /// the cap is enforced on the bytes actually received.
    fn content_length(&self) -> Option<u64>;

    /// The next piece of the body; `None` once it is complete.
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SetupError>;
}

/// [`Downloader`] over HTTPS only (redirects included), without a
/// whole-request timeout but with connect and stall timeouts.
pub struct HttpDownloader {
    /// `Err` when the TLS backend could not be initialized; every download
    /// then fails with that message instead of the app panicking.
    client: Result<reqwest::Client, String>,
}

impl HttpDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .https_only(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .map_err(|err| describe(&err));
        Self { client }
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Downloader for HttpDownloader {
    async fn get(&self, url: &str) -> Result<Box<dyn DownloadBody>, SetupError> {
        let failed = |message: String| SetupError::Download {
            url: url.to_string(),
            message,
        };
        let client = self
            .client
            .as_ref()
            .map_err(|message| failed(format!("no HTTP client: {message}")))?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|err| failed(describe(&err)))?;
        let status = response.status();
        if !status.is_success() {
            return Err(failed(format!("HTTP {status}")));
        }
        Ok(Box::new(HttpBody {
            response,
            url: url.to_string(),
        }))
    }
}

struct HttpBody {
    response: reqwest::Response,
    url: String,
}

#[async_trait]
impl DownloadBody for HttpBody {
    fn content_length(&self) -> Option<u64> {
        self.response.content_length()
    }

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SetupError> {
        match self.response.chunk().await {
            Ok(chunk) => Ok(chunk.map(|bytes| bytes.to_vec())),
            Err(err) => Err(SetupError::Download {
                url: self.url.clone(),
                message: describe(&err),
            }),
        }
    }
}

/// Downloads `url` into `dest`, which must not exist yet, and checks that it
/// is at most `max_bytes` long and hashes to `sha256` (lowercase hex). On any
/// failure after `dest` was created, `dest` is deleted, so a file at `dest`
/// after `Ok` is always the pinned one.
pub async fn download_verified(
    downloader: &dyn Downloader,
    url: &str,
    sha256: &str,
    max_bytes: u64,
    dest: &Path,
) -> Result<(), SetupError> {
    let mut body = downloader.get(url).await?;
    let too_large = || SetupError::TooLarge {
        url: url.to_string(),
        max: max_bytes,
    };
    if body.content_length().is_some_and(|len| len > max_bytes) {
        return Err(too_large());
    }
    // `create_new` refuses an existing file or symlink: nothing planted at
    // `dest` is ever followed or overwritten.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(SetupError::io("create", dest))?;

    let streamed: Result<String, SetupError> = async {
        let mut hasher = Sha256::new();
        let mut received: u64 = 0;
        while let Some(chunk) = body.next_chunk().await? {
            received += chunk.len() as u64;
            if received > max_bytes {
                return Err(too_large());
            }
            hasher.update(&chunk);
            file.write_all(&chunk)
                .map_err(SetupError::io("write", dest))?;
        }
        file.sync_all().map_err(SetupError::io("write", dest))?;
        Ok(hex::encode(hasher.finalize()))
    }
    .await;
    drop(file);

    let failure = match streamed {
        Ok(actual) if actual == sha256.to_ascii_lowercase() => return Ok(()),
        Ok(actual) => SetupError::ChecksumMismatch {
            url: url.to_string(),
            expected: sha256.to_string(),
            actual,
        },
        Err(err) => err,
    };
    if let Err(err) = fs::remove_file(dest) {
        if err.kind() != io::ErrorKind::NotFound {
            log::error!(
                "Could not delete the rejected download {}: {err}",
                dest.display()
            );
        }
    }
    log::warn!("{failure}");
    Err(failure)
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// `err` followed by its causes; reqwest's own message stops at "error
/// sending request" and hides whether DNS, TLS or the connection failed.
fn describe(err: &(dyn StdError + 'static)) -> String {
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
    use crate::setup::testing::FakeDownloader;

    const URL: &str = "https://example.com/file.bin";

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    fn body() -> Vec<u8> {
        (0u8..=200).collect()
    }

    #[test]
    fn a_matching_download_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        let fake = FakeDownloader::default();
        fake.serve(URL, body());

        block_on(download_verified(
            &fake,
            URL,
            &sha256_hex(&body()),
            1024,
            &dest,
        ))
        .unwrap();

        assert_eq!(fs::read(&dest).unwrap(), body());
        assert_eq!(fake.requests(), [URL]);
    }

    #[test]
    fn the_hash_comparison_ignores_hex_case() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        let fake = FakeDownloader::default();
        fake.serve(URL, body());
        let upper = sha256_hex(&body()).to_ascii_uppercase();
        assert!(block_on(download_verified(&fake, URL, &upper, 1024, &dest)).is_ok());
    }

    #[test]
    fn a_mismatching_download_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        let fake = FakeDownloader::default();
        let mut tampered = body();
        tampered[100] ^= 1;
        fake.serve(URL, tampered.clone());

        let err = block_on(download_verified(
            &fake,
            URL,
            &sha256_hex(&body()),
            1024,
            &dest,
        ))
        .unwrap_err();

        match err {
            SetupError::ChecksumMismatch {
                url,
                expected,
                actual,
            } => {
                assert_eq!(url, URL);
                assert_eq!(expected, sha256_hex(&body()));
                assert_eq!(actual, sha256_hex(&tampered));
            }
            other => panic!("expected a checksum mismatch, got {other:?}"),
        }
        assert!(!dest.exists(), "the rejected file must be gone");
    }

    #[test]
    fn an_oversized_body_is_cut_off_and_deleted_even_without_a_length() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        let lying = FakeDownloader {
            // The server lies about the length; the received bytes count.
            announced_length: Some(10),
            ..FakeDownloader::default()
        };
        lying.serve(URL, body());

        let err = block_on(download_verified(
            &lying,
            URL,
            &sha256_hex(&body()),
            100,
            &dest,
        ))
        .unwrap_err();
        assert!(
            matches!(err, SetupError::TooLarge { max: 100, .. }),
            "{err:?}"
        );
        assert!(!dest.exists());

        // An honest oversized announcement fails before any file exists.
        let honest = FakeDownloader::default();
        honest.serve(URL, body());
        let err = block_on(download_verified(
            &honest,
            URL,
            &sha256_hex(&body()),
            100,
            &dest,
        ))
        .unwrap_err();
        assert!(matches!(err, SetupError::TooLarge { .. }), "{err:?}");
        assert!(!dest.exists());
    }

    #[test]
    fn exactly_the_cap_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        let fake = FakeDownloader::default();
        fake.serve(URL, body());
        let len = body().len() as u64;
        assert!(block_on(download_verified(
            &fake,
            URL,
            &sha256_hex(&body()),
            len,
            &dest
        ))
        .is_ok());
    }

    #[test]
    fn an_existing_destination_is_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        fs::write(&dest, "not yours").unwrap();
        let fake = FakeDownloader::default();
        fake.serve(URL, body());

        let err = block_on(download_verified(
            &fake,
            URL,
            &sha256_hex(&body()),
            1024,
            &dest,
        ))
        .unwrap_err();

        assert!(matches!(err, SetupError::Io { .. }), "{err:?}");
        assert_eq!(fs::read_to_string(&dest).unwrap(), "not yours");
    }

    #[test]
    fn transport_errors_leave_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("file.bin");
        let fake = FakeDownloader::default();
        let err = block_on(download_verified(&fake, URL, "00", 1024, &dest)).unwrap_err();
        assert!(matches!(err, SetupError::Download { .. }), "{err:?}");
        assert!(!dest.exists());
    }

    #[test]
    fn the_http_downloader_refuses_plain_http() {
        let downloader = HttpDownloader::new();
        // Refused by the client itself, before any connection is made.
        let result = block_on(downloader.get("http://127.0.0.1:1/anki.dmg"));
        let Err(SetupError::Download { message, .. }) = result else {
            panic!("plain HTTP must be refused");
        };
        assert!(message.contains("URL scheme is not allowed"), "{message}");
    }
}
