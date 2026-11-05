//! `fetch` and the SHA-256 manifest: the first run pins, later runs verify,
//! and any difference is a hard error that writes nothing. Files are served
//! from memory; only the last test starts a (loopback) HTTP server.

use std::{collections::HashMap, sync::Mutex};

use hsk_import::{
    sources::{
        fetch_sources, pinned_files, Fetcher, HttpFetcher, Manifest, MirrorFetcher, PinnedFile,
    },
    Error,
};
use sha2::{Digest, Sha256};

/// Serves every pinned URL from memory and counts requests.
struct InMemory {
    files: HashMap<String, Vec<u8>>,
    requests: Mutex<Vec<String>>,
}

impl InMemory {
    fn new(content: impl Fn(&PinnedFile) -> Vec<u8>) -> Self {
        Self {
            files: pinned_files()
                .iter()
                .map(|file| (file.url.clone(), content(file)))
                .collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn upstream() -> Self {
        Self::new(|file| format!("content of {}", file.path).into_bytes())
    }
}

impl Fetcher for InMemory {
    fn fetch(&self, url: &str) -> hsk_import::Result<Vec<u8>> {
        self.requests.lock().unwrap().push(url.to_string());
        self.files
            .get(url)
            .cloned()
            .ok_or_else(|| Error::Http(format!("{url}: HTTP 404 Not Found")))
    }
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn the_first_fetch_writes_the_files_and_pins_them_in_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let (out, manifest_path) = (dir.path().join("src"), dir.path().join("hsk_sources.json"));
    let upstream = InMemory::upstream();

    let report = fetch_sources(&upstream, &out, &manifest_path).unwrap();

    assert!(report.manifest_written);
    let pinned = pinned_files();
    assert_eq!(upstream.requests.lock().unwrap().len(), pinned.len());
    let manifest = Manifest::load(&manifest_path).unwrap().unwrap();
    assert_eq!(manifest.files, report.files);
    assert_eq!(manifest.files.len(), pinned.len());
    for (entry, file) in manifest.files.iter().zip(&pinned) {
        let content = std::fs::read(out.join(&file.path)).unwrap();
        assert_eq!(content, upstream.files[&file.url], "{}", file.path);
        assert_eq!(entry.url, file.url);
        assert_eq!(entry.path, file.path);
        assert_eq!(entry.sha256, sha256(&content));
        assert_eq!(entry.bytes, content.len() as u64);
    }
    let commits: Vec<&str> = manifest.sources.iter().map(|s| s.commit.as_str()).collect();
    assert_eq!(
        commits,
        [
            "7ac65bf1a6387d35f1ade478906172a19311c7f9",
            "4ff9e3915ce87baaecd7ebe263085573a4ea3192"
        ]
    );
    assert!(manifest.sources.iter().all(|s| s.license == "MIT"));
}

#[test]
fn a_later_fetch_verifies_against_the_manifest_and_leaves_it_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (out, manifest_path) = (dir.path().join("src"), dir.path().join("hsk_sources.json"));
    fetch_sources(&InMemory::upstream(), &out, &manifest_path).unwrap();
    let pinned_manifest = std::fs::read(&manifest_path).unwrap();

    let again = fetch_sources(
        &InMemory::upstream(),
        &dir.path().join("again"),
        &manifest_path,
    )
    .unwrap();

    assert!(!again.manifest_written);
    assert_eq!(std::fs::read(&manifest_path).unwrap(), pinned_manifest);
    let file = &pinned_files()[8];
    assert_eq!(
        std::fs::read(dir.path().join("again").join(&file.path)).unwrap(),
        format!("content of {}", file.path).into_bytes()
    );
}

#[test]
fn changed_upstream_content_is_a_hard_error_that_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (out, manifest_path) = (dir.path().join("src"), dir.path().join("hsk_sources.json"));
    fetch_sources(&InMemory::upstream(), &out, &manifest_path).unwrap();
    let pinned_manifest = std::fs::read(&manifest_path).unwrap();
    let tampered_file = pinned_files()[3].clone();
    // Same length, different bytes, so only the hash can tell.
    let tampered = InMemory::new(|file| {
        let mut content = format!("content of {}", file.path).into_bytes();
        if file.url == tampered_file.url {
            content[0] ^= 0x20;
        }
        content
    });
    let fresh = dir.path().join("fresh");

    let result = fetch_sources(&tampered, &fresh, &manifest_path);

    match result {
        Err(Error::Integrity(message)) => {
            assert!(message.contains(&tampered_file.url), "{message}");
            assert!(message.contains("SHA-256"), "{message}");
        }
        other => panic!("expected an integrity error, got {other:?}"),
    }
    assert!(!fresh.exists(), "nothing may be written after a mismatch");
    assert_eq!(std::fs::read(&manifest_path).unwrap(), pinned_manifest);

    // Into the old directory: the verified files from the first run stay.
    assert!(fetch_sources(&tampered, &out, &manifest_path).is_err());
    assert_eq!(
        std::fs::read(out.join(&pinned_files()[0].path)).unwrap(),
        format!("content of {}", pinned_files()[0].path).into_bytes()
    );
}

#[test]
fn a_manifest_written_for_other_pins_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (out, manifest_path) = (dir.path().join("src"), dir.path().join("hsk_sources.json"));
    fetch_sources(&InMemory::upstream(), &out, &manifest_path).unwrap();

    let mut manifest = Manifest::load(&manifest_path).unwrap().unwrap();
    manifest.files[0].url = manifest.files[0].url.replace("7ac65bf1", "00000000");
    std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
    assert!(matches!(
        fetch_sources(&InMemory::upstream(), &out, &manifest_path),
        Err(Error::Integrity(message)) if message.contains("no entry")
    ));

    manifest.files.pop();
    std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
    assert!(matches!(
        fetch_sources(&InMemory::upstream(), &out, &manifest_path),
        Err(Error::Integrity(message)) if message.contains("are pinned")
    ));
}

#[test]
fn a_failed_download_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut upstream = InMemory::upstream();
    let missing = pinned_files()[9].url.clone();
    upstream.files.remove(&missing);
    let out = dir.path().join("src");
    let manifest_path = dir.path().join("hsk_sources.json");

    let result = fetch_sources(&upstream, &out, &manifest_path);

    assert!(matches!(result, Err(Error::Http(message)) if message.contains(&missing)));
    assert!(!out.exists());
    assert!(!manifest_path.exists());
}

#[test]
fn a_mirror_with_the_source_layout_fetches_like_a_download() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = InMemory::upstream();
    let mirror = dir.path().join("mirror");
    for file in pinned_files() {
        let path = mirror.join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &upstream.files[&file.url]).unwrap();
    }
    let manifest_path = dir.path().join("hsk_sources.json");
    let downloaded = fetch_sources(&upstream, &dir.path().join("a"), &manifest_path).unwrap();

    let mirrored = fetch_sources(
        &MirrorFetcher::new(&mirror),
        &dir.path().join("b"),
        &manifest_path,
    )
    .unwrap();

    assert!(!mirrored.manifest_written);
    assert_eq!(mirrored.files, downloaded.files);
    std::fs::write(mirror.join(&pinned_files()[0].path), b"edited").unwrap();
    assert!(matches!(
        fetch_sources(
            &MirrorFetcher::new(&mirror),
            &dir.path().join("c"),
            &manifest_path
        ),
        Err(Error::Integrity(_))
    ));
    assert!(MirrorFetcher::new(&mirror)
        .fetch("https://example.invalid/not-pinned")
        .is_err());
}

/// The real client against a loopback server: a 404 page must be an error,
/// never bytes that end up pinned. Needs local port binding.
#[test]
fn the_http_fetcher_returns_bodies_and_refuses_error_statuses() {
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = runtime.block_on(MockServer::start());
    runtime.block_on(
        Mock::given(method("GET"))
            .and(path("/data.json"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"[1,2,3]".to_vec()))
            .mount(&server),
    );
    runtime.block_on(
        Mock::given(method("GET"))
            .and(path("/gone.json"))
            .respond_with(ResponseTemplate::new(404).set_body_string("404: Not Found"))
            .mount(&server),
    );
    let fetcher = HttpFetcher::new().unwrap();

    assert_eq!(
        fetcher
            .fetch(&format!("{}/data.json", server.uri()))
            .unwrap(),
        b"[1,2,3]"
    );
    match fetcher.fetch(&format!("{}/gone.json", server.uri())) {
        Err(Error::Http(message)) => assert!(message.contains("404"), "{message}"),
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}
