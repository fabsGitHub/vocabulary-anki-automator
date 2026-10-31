//! The pinned upstream files and their SHA-256 manifest (`fetch`).
//!
//! The bundled curriculum is regenerated rarely and by hand, so the inputs
//! are pinned twice: by commit in the URLs below, and by content in
//! `crates/vocab-core/data/hsk_sources.json`. The first fetch records every
//! file's size and hash; every later fetch must reproduce them exactly or it
//! fails before writing anything. A raw.githubusercontent.com URL at a commit
//! should never change, so a mismatch means something between here and
//! upstream did, and that is not something to retry around.
//!
//! The LICENSE files are pinned with the data because the MIT license is what
//! allows shipping the derived curriculum; the copies next to the bundled JSON
//! are taken from these downloads.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    error::{Error, Result},
    output::write_atomic,
};

/// drkameleon/complete-hsk-vocabulary (MIT): meanings, forms, frequency.
pub const VOCABULARY_REPO: &str = "drkameleon/complete-hsk-vocabulary";
pub const VOCABULARY_COMMIT: &str = "7ac65bf1a6387d35f1ade478906172a19311c7f9";
/// Its directory under the source directory.
pub const VOCABULARY_DIR: &str = "complete-hsk-vocabulary";

/// ivankra/hsk30 (MIT): the official list, one row per reading.
pub const OFFICIAL_REPO: &str = "ivankra/hsk30";
pub const OFFICIAL_COMMIT: &str = "4ff9e3915ce87baaecd7ebe263085573a4ea3192";
pub const OFFICIAL_DIR: &str = "hsk30";

/// Where the manifest lives in this repository.
pub const DEFAULT_MANIFEST: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/vocab-core/data/hsk_sources.json"
);

/// Per-read and connect timeout of downloads. A whole-request timeout as in
/// the apps' clients (docs/architecture.md § 2) would cut off the 4 MB level
/// 7–9 file on a slow line; this still never waits more than 10 s for a byte.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

const USER_AGENT: &str =
    "hsk-import/0.1 (vocabulary-automator ETL; +https://github.com/fabsGitHub/vocabulary-anki-automator)";

/// One pinned file: where it comes from and where it goes under the source
/// directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedFile {
    /// Relative to the source directory, `/`-separated.
    pub path: String,
    pub url: String,
}

/// Every file `fetch` downloads, in a fixed order.
pub fn pinned_files() -> Vec<PinnedFile> {
    let raw = |repo: &str, commit: &str, path: &str| {
        format!("https://raw.githubusercontent.com/{repo}/{commit}/{path}")
    };
    let mut files: Vec<PinnedFile> = crate::vocabulary::LEVELS
        .map(|level| {
            let path = format!("wordlists/exclusive/new/{level}.json");
            PinnedFile {
                url: raw(VOCABULARY_REPO, VOCABULARY_COMMIT, &path),
                path: format!("{VOCABULARY_DIR}/{path}"),
            }
        })
        .collect();
    files.push(PinnedFile {
        path: format!("{VOCABULARY_DIR}/LICENSE"),
        url: raw(VOCABULARY_REPO, VOCABULARY_COMMIT, "LICENSE"),
    });
    for path in ["hsk30.csv", "LICENSE"] {
        files.push(PinnedFile {
            path: format!("{OFFICIAL_DIR}/{path}"),
            url: raw(OFFICIAL_REPO, OFFICIAL_COMMIT, path),
        });
    }
    files
}

/// Where the official list lives under the source directory.
pub fn official_file(source_dir: &Path) -> PathBuf {
    source_dir.join(OFFICIAL_DIR).join("hsk30.csv")
}

/// `hsk_sources.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub sources: Vec<SourceRepo>,
    pub files: Vec<ManifestEntry>,
}

/// Provenance of one upstream repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRepo {
    pub repository: String,
    pub commit: String,
    pub license: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

impl ManifestEntry {
    pub fn describe(file: &PinnedFile, content: &[u8]) -> Self {
        Self {
            path: file.path.clone(),
            url: file.url.clone(),
            sha256: hex::encode(Sha256::digest(content)),
            bytes: content.len() as u64,
        }
    }
}

impl Manifest {
    fn new(files: Vec<ManifestEntry>) -> Self {
        let repo = |repository: &str, commit: &str| SourceRepo {
            repository: format!("https://github.com/{repository}"),
            commit: commit.to_string(),
            license: "MIT".to_string(),
        };
        Self {
            sources: vec![
                repo(VOCABULARY_REPO, VOCABULARY_COMMIT),
                repo(OFFICIAL_REPO, OFFICIAL_COMMIT),
            ],
            files,
        }
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|err| Error::json(path, err)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(Error::io(path, err)),
        }
    }

    /// Checks that this manifest pins exactly `files` and that `content`
    /// (downloaded for `file`) matches its entry.
    fn verify(&self, file: &PinnedFile, content: &[u8]) -> Result<()> {
        let entry = self
            .files
            .iter()
            .find(|entry| entry.url == file.url)
            .ok_or_else(|| {
                Error::Integrity(format!(
                    "the manifest has no entry for {}; it was written for other pins \
                     (delete it to re-pin deliberately)",
                    file.url
                ))
            })?;
        let actual = ManifestEntry::describe(file, content);
        if actual.sha256 != entry.sha256 || actual.bytes != entry.bytes || entry.path != file.path {
            return Err(Error::Integrity(format!(
                "{} does not match the manifest: expected {} bytes with SHA-256 {}, \
                 got {} bytes with SHA-256 {}",
                file.url, entry.bytes, entry.sha256, actual.bytes, actual.sha256
            )));
        }
        Ok(())
    }
}

/// Downloads one URL. A trait so tests can serve files from memory.
pub trait Fetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>>;
}

/// The real [`Fetcher`]: reqwest on a private current-thread runtime.
pub struct HttpFetcher {
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

impl HttpFetcher {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(READ_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|err| Error::Http(format!("could not build the HTTP client: {err}")))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| Error::Http(format!("could not start the async runtime: {err}")))?;
        Ok(Self { client, runtime })
    }
}

impl Fetcher for HttpFetcher {
    /// Fails on transport errors and on any non-2xx status: a 404 page must
    /// never be recorded in the manifest as if it were the data.
    fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let failed = |err: reqwest::Error| Error::Http(format!("{url}: {}", describe(err)));
        self.runtime.block_on(async {
            let response = self.client.get(url).send().await.map_err(failed)?;
            let status = response.status();
            if !status.is_success() {
                return Err(Error::Http(format!("{url}: HTTP {status}")));
            }
            Ok(response.bytes().await.map_err(failed)?.to_vec())
        })
    }
}

/// A [`Fetcher`] that reads the pinned files from a local mirror with the
/// source-directory layout (`fetch --from DIR`), for machines where this
/// tool cannot download them itself; macOS's TLS stack, for one, does not
/// work inside a sandbox that hides the system trust daemon. The manifest
/// checks a mirror exactly as it checks a download, so a mirror can only
/// supply the pinned bytes (or, on the first run, pin what it holds, which a
/// later real download must then reproduce).
pub struct MirrorFetcher {
    root: PathBuf,
}

impl MirrorFetcher {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Fetcher for MirrorFetcher {
    fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        let file = pinned_files()
            .into_iter()
            .find(|file| file.url == url)
            .ok_or_else(|| Error::Http(format!("{url} is not a pinned file")))?;
        let path = self.root.join(&file.path);
        std::fs::read(&path).map_err(|err| Error::io(path, err))
    }
}

/// `err` and its causes, without the URL (the caller names it); reqwest's own
/// message stops at "error sending request", which hides whether DNS, TLS or
/// the connection failed.
fn describe(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut message = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.ends_with(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

/// What a fetch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchReport {
    pub files: Vec<ManifestEntry>,
    /// True on the first fetch, which wrote the manifest; false when the
    /// downloads were verified against an existing one.
    pub manifest_written: bool,
}

/// Downloads every [`pinned_files`] entry with `fetcher`, verifies it against
/// the manifest at `manifest_path` (or records the manifest when there is
/// none yet), and only then writes the files under `out_dir`. Any mismatch is
/// an [`Error::Integrity`] and leaves `out_dir` untouched.
pub fn fetch_sources(
    fetcher: &dyn Fetcher,
    out_dir: &Path,
    manifest_path: &Path,
) -> Result<FetchReport> {
    let manifest = Manifest::load(manifest_path)?;
    let pinned = pinned_files();
    if let Some(manifest) = &manifest {
        if manifest.files.len() != pinned.len() {
            return Err(Error::Integrity(format!(
                "{} lists {} files, but {} are pinned (delete it to re-pin deliberately)",
                manifest_path.display(),
                manifest.files.len(),
                pinned.len()
            )));
        }
    }
    let mut downloads = Vec::with_capacity(pinned.len());
    for file in &pinned {
        let content = fetcher.fetch(&file.url)?;
        if let Some(manifest) = &manifest {
            manifest.verify(file, &content)?;
        }
        downloads.push((file, content));
    }
    for (file, content) in &downloads {
        write_atomic(&out_dir.join(&file.path), content)?;
    }
    let files: Vec<ManifestEntry> = downloads
        .iter()
        .map(|(file, content)| ManifestEntry::describe(file, content))
        .collect();
    let manifest_written = manifest.is_none();
    if manifest_written {
        let mut json = serde_json::to_string_pretty(&Manifest::new(files.clone()))
            .map_err(|err| Error::json(manifest_path, err))?;
        json.push('\n');
        write_atomic(manifest_path, json.as_bytes())?;
    }
    Ok(FetchReport {
        files,
        manifest_written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_seven_level_files_the_official_list_and_both_licenses() {
        let files = pinned_files();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "complete-hsk-vocabulary/wordlists/exclusive/new/1.json",
                "complete-hsk-vocabulary/wordlists/exclusive/new/2.json",
                "complete-hsk-vocabulary/wordlists/exclusive/new/3.json",
                "complete-hsk-vocabulary/wordlists/exclusive/new/4.json",
                "complete-hsk-vocabulary/wordlists/exclusive/new/5.json",
                "complete-hsk-vocabulary/wordlists/exclusive/new/6.json",
                "complete-hsk-vocabulary/wordlists/exclusive/new/7.json",
                "complete-hsk-vocabulary/LICENSE",
                "hsk30/hsk30.csv",
                "hsk30/LICENSE",
            ]
        );
        assert_eq!(
            files[0].url,
            "https://raw.githubusercontent.com/drkameleon/complete-hsk-vocabulary/\
             7ac65bf1a6387d35f1ade478906172a19311c7f9/wordlists/exclusive/new/1.json"
        );
        assert_eq!(
            files[8].url,
            "https://raw.githubusercontent.com/ivankra/hsk30/\
             4ff9e3915ce87baaecd7ebe263085573a4ea3192/hsk30.csv"
        );
        assert!(DEFAULT_MANIFEST.ends_with("crates/vocab-core/data/hsk_sources.json"));
    }

    #[test]
    fn manifest_entries_hash_the_content() {
        let file = PinnedFile {
            path: "x".into(),
            url: "https://example.invalid/x".into(),
        };
        let entry = ManifestEntry::describe(&file, b"abc");
        assert_eq!(
            entry.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(entry.bytes, 3);
    }
}
