//! Installing AnkiConnect into Anki's add-on folder (Milestone E).
//!
//! The pre-refactor code fetched `master/ankiconnect.py`, a file that no
//! longer exists (the download was a 404 page), wrote it unverified next to a
//! stub `__init__.py`, and configured AnkiConnect to accept requests from any
//! web origin (`"*"`), which lets every website the user visits read and
//! change their collection while Anki runs. Now the add-on comes from one
//! pinned commit, every file is verified before it is written, the folder
//! appears in one rename (Anki never sees half an add-on), an existing add-on
//! is never overwritten, and only this app's own webview origin is allowed.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};

use super::{
    download::{download_verified, sha256_hex, Downloader},
    ensure_absent, require_consent, AnkiPaths, ScratchDir, SetupError,
};
use crate::config::{ConsentAction, ConsentState};

/// AnkiConnect's AnkiWeb id, which is also its folder name in `addons21`.
pub const ADDON_ID: &str = "2055492159";

/// The final commit of `FooSoft/anki-connect` (the repository is archived).
pub const COMMIT: &str = "4064fa142785975255457abd6a496015f5b71f38";

/// One file of the add-on and its SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedFile {
    pub name: &'static str,
    pub sha256: &'static str,
}

/// The add-on: every file of `plugin/` at [`COMMIT`] that Anki loads.
pub const FILES: [PinnedFile; 4] = [
    PinnedFile {
        name: "__init__.py",
        sha256: "2185b1243b924152cbac7a7668b40c0bf713902619bfc8ede5c42465066d0f05",
    },
    PinnedFile {
        name: "edit.py",
        sha256: "4c6659247322be76c3015b8f8944ce20498186782880a3c8ff14a48d2e3f58b2",
    },
    PinnedFile {
        name: "util.py",
        sha256: "cc0e12de6f809d57d40b29f3eb5f0db1eadad93e19f35fc63a2cfe250216c3b9",
    },
    PinnedFile {
        name: "web.py",
        sha256: "b519b4c6fefa3d2058df926c43ade1cf590d595655ee3b20afdf7ccd2a053a78",
    },
];

/// Far above the largest pinned file; anything bigger is not AnkiConnect.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The `__init__.py` the pre-refactor app wrote. Its add-on never loaded
/// (the code it imports was a 404 page), so it does not count as installed.
const LEGACY_STUB: &str = "from . import ankiconnect\n";

/// Where `name` is downloaded from: the raw file at the pinned commit.
pub fn file_url(name: &str) -> String {
    format!("https://raw.githubusercontent.com/FooSoft/anki-connect/{COMMIT}/plugin/{name}")
}

/// The web origins allowed to call AnkiConnect: this app's own webview
/// (`tauri://localhost` on macOS and Linux, `http://tauri.localhost` on
/// Windows) and, in debug builds only, the Vite dev server. The Rust backend
/// sends no `Origin` header and needs no entry.
pub fn app_origins() -> Vec<&'static str> {
    app_origins_for(cfg!(windows), cfg!(debug_assertions))
}

/// [`app_origins`] for a given platform and build type.
pub fn app_origins_for(windows: bool, debug: bool) -> Vec<&'static str> {
    let mut origins = vec![if windows {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }];
    if debug {
        origins.push("http://localhost:1420");
    }
    origins
}

/// The add-on's `config.json`: listen on loopback only, on the standard port,
/// and accept browser requests from `origins` only.
pub fn addon_config(origins: &[&str]) -> Value {
    json!({
        "apiKey": null,
        "webBindAddress": "127.0.0.1",
        "webBindPort": 8765,
        "webCorsOriginList": origins,
    })
}

/// Whether `dir` holds an AnkiConnect add-on: an `__init__.py` that is not
/// the stub the pre-refactor app left behind.
pub fn is_installed(dir: &Path) -> bool {
    let init = dir.join("__init__.py");
    if !init.is_file() {
        return false;
    }
    match fs::read_to_string(&init) {
        Ok(text) => text != LEGACY_STUB,
        // Unreadable as text (or at all): something is there, so do not
        // offer to install over it.
        Err(_) => true,
    }
}

/// Installs the pinned AnkiConnect into `paths.ankiconnect_dir()` and returns
/// that folder. Needs consent; refuses when anything is already there.
pub async fn provision(
    consent: &ConsentState,
    paths: &AnkiPaths,
    downloader: &dyn Downloader,
) -> Result<PathBuf, SetupError> {
    provision_files(consent, paths, downloader, &FILES).await
}

/// [`provision`] with the file list as a parameter, so tests can pin fake
/// contents. Steps: consent, refuse an existing folder, download every file
/// into a private temp dir and verify it, copy the verified bytes plus our
/// `config.json` into a staging folder next to `addons21` (same file system,
/// so the final rename is atomic), check once more that the target is free,
/// rename. Any failure leaves `addons21` exactly as it was.
pub(crate) async fn provision_files(
    consent: &ConsentState,
    paths: &AnkiPaths,
    downloader: &dyn Downloader,
    files: &[PinnedFile],
) -> Result<PathBuf, SetupError> {
    require_consent(consent, ConsentAction::AnkiconnectProvision)?;
    let target = paths.ankiconnect_dir();
    ensure_absent(&target)?;

    let downloads = ScratchDir::new_in(&paths.temp_dir, "vocab-automator-ankiconnect-")?;
    for file in files {
        let dest = downloads.path().join(file.name);
        download_verified(
            downloader,
            &file_url(file.name),
            file.sha256,
            MAX_FILE_BYTES,
            &dest,
        )
        .await?;
    }
    log::info!(
        "All {} AnkiConnect files match their pinned checksums.",
        files.len()
    );

    let addons = paths.addons_dir();
    fs::create_dir_all(&addons).map_err(SetupError::io("create", &addons))?;
    let staging = ScratchDir::new_in(&paths.anki_base, ".vocab-automator-staging-")?;
    let staged = staging.path().join(ADDON_ID);
    fs::create_dir(&staged).map_err(SetupError::io("create", &staged))?;
    for file in files {
        let source = downloads.path().join(file.name);
        let bytes = fs::read(&source).map_err(SetupError::io("read", &source))?;
        // The bytes that get installed are the bytes that were verified.
        let actual = sha256_hex(&bytes);
        if actual != file.sha256 {
            return Err(SetupError::ChecksumMismatch {
                url: file_url(file.name),
                expected: file.sha256.to_string(),
                actual,
            });
        }
        write_new(&staged.join(file.name), &bytes)?;
    }
    let config = serde_json::to_vec_pretty(&addon_config(&app_origins()))
        .map_err(|err| SetupError::Unavailable(format!("could not encode config.json: {err}")))?;
    write_new(&staged.join("config.json"), &config)?;

    ensure_absent(&target)?;
    fs::rename(&staged, &target).map_err(SetupError::io("move AnkiConnect into", &target))?;
    log::info!("AnkiConnect installed into {}.", target.display());
    Ok(target)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), SetupError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(SetupError::io("create", path))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(SetupError::io("write", path))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::setup::testing::{temp_paths, FakeDownloader};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    fn consented() -> ConsentState {
        let mut consent = ConsentState::default();
        consent.grant(
            ConsentAction::AnkiconnectProvision,
            Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap(),
        );
        consent
    }

    /// Fake add-on files, pinned to their own hashes and served by `fake`.
    fn fake_addon(fake: &FakeDownloader) -> Vec<PinnedFile> {
        let contents: [(&'static str, &'static str); 4] = [
            ("__init__.py", "from . import web\n"),
            ("edit.py", "# edit\n"),
            ("util.py", "# util\n"),
            ("web.py", "# web\n"),
        ];
        contents
            .iter()
            .map(|&(name, text)| {
                fake.serve(file_url(name), text.as_bytes());
                PinnedFile {
                    name,
                    sha256: Box::leak(sha256_hex(text.as_bytes()).into_boxed_str()),
                }
            })
            .collect()
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn the_pins_are_the_contract_values() {
        assert_eq!(ADDON_ID, "2055492159");
        assert_eq!(COMMIT, "4064fa142785975255457abd6a496015f5b71f38");
        let pins: Vec<(&str, &str)> = FILES.iter().map(|f| (f.name, f.sha256)).collect();
        assert_eq!(
            pins,
            [
                (
                    "__init__.py",
                    "2185b1243b924152cbac7a7668b40c0bf713902619bfc8ede5c42465066d0f05"
                ),
                (
                    "edit.py",
                    "4c6659247322be76c3015b8f8944ce20498186782880a3c8ff14a48d2e3f58b2"
                ),
                (
                    "util.py",
                    "cc0e12de6f809d57d40b29f3eb5f0db1eadad93e19f35fc63a2cfe250216c3b9"
                ),
                (
                    "web.py",
                    "b519b4c6fefa3d2058df926c43ade1cf590d595655ee3b20afdf7ccd2a053a78"
                ),
            ]
        );
        assert_eq!(
            file_url("web.py"),
            "https://raw.githubusercontent.com/FooSoft/anki-connect/\
             4064fa142785975255457abd6a496015f5b71f38/plugin/web.py"
        );
    }

    #[test]
    fn the_config_allows_only_the_app_origin() {
        let cases = [
            (false, false, json!(["tauri://localhost"])),
            (
                false,
                true,
                json!(["tauri://localhost", "http://localhost:1420"]),
            ),
            (true, false, json!(["http://tauri.localhost"])),
            (
                true,
                true,
                json!(["http://tauri.localhost", "http://localhost:1420"]),
            ),
        ];
        for (windows, debug, origins) in cases {
            assert_eq!(
                addon_config(&app_origins_for(windows, debug)),
                json!({
                    "apiKey": null,
                    "webBindAddress": "127.0.0.1",
                    "webBindPort": 8765,
                    "webCorsOriginList": origins,
                }),
                "windows {windows}, debug {debug}"
            );
        }
    }

    #[test]
    fn provisioning_installs_the_verified_files_and_the_exact_config() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        let fake = FakeDownloader::default();
        let files = fake_addon(&fake);

        let installed = block_on(provision_files(&consented(), &paths, &fake, &files)).unwrap();

        assert_eq!(installed, paths.ankiconnect_dir());
        assert_eq!(
            entries(&installed),
            ["__init__.py", "config.json", "edit.py", "util.py", "web.py"]
        );
        assert_eq!(
            fs::read_to_string(installed.join("web.py")).unwrap(),
            "# web\n"
        );
        let config: Value =
            serde_json::from_slice(&fs::read(installed.join("config.json")).unwrap()).unwrap();
        let expected_origins = if cfg!(windows) {
            json!(["http://tauri.localhost", "http://localhost:1420"])
        } else {
            json!(["tauri://localhost", "http://localhost:1420"])
        };
        // Tests are debug builds, so the dev-server origin is included.
        assert_eq!(
            config,
            json!({
                "apiKey": null,
                "webBindAddress": "127.0.0.1",
                "webBindPort": 8765,
                "webCorsOriginList": expected_origins,
            })
        );
        assert!(!fs::read_to_string(installed.join("config.json"))
            .unwrap()
            .contains('*'));
        assert!(is_installed(&installed));
        // Only the pinned URLs were fetched, in order.
        let urls: Vec<String> = files.iter().map(|f| file_url(f.name)).collect();
        assert_eq!(fake.requests(), urls);
        // No staging or download folders are left behind.
        assert_eq!(entries(&paths.anki_base), ["addons21"]);
        assert_eq!(entries(&paths.temp_dir), Vec::<String>::new());
    }

    #[test]
    fn one_tampered_file_installs_nothing() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        let fake = FakeDownloader::default();
        let files = fake_addon(&fake);
        fake.serve(file_url("util.py"), "import os; os.system('evil')\n");

        let err = block_on(provision_files(&consented(), &paths, &fake, &files)).unwrap_err();

        assert!(
            matches!(err, SetupError::ChecksumMismatch { .. }),
            "{err:?}"
        );
        assert!(!paths.ankiconnect_dir().exists());
        assert_eq!(entries(&paths.addons_dir()), Vec::<String>::new());
        assert_eq!(
            entries(&paths.temp_dir),
            Vec::<String>::new(),
            "downloads deleted"
        );
        // web.py, after the bad file, was never even requested.
        assert_eq!(fake.requests().len(), 3);
    }

    #[test]
    fn the_real_pins_reject_anything_but_the_real_files() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        let fake = FakeDownloader::default();
        for file in FILES {
            fake.serve(file_url(file.name), "not the pinned file");
        }

        let err = block_on(provision(&consented(), &paths, &fake)).unwrap_err();

        assert!(
            matches!(err, SetupError::ChecksumMismatch { .. }),
            "{err:?}"
        );
        assert!(!paths.ankiconnect_dir().exists());
    }

    #[test]
    fn an_existing_add_on_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        fs::create_dir_all(paths.ankiconnect_dir()).unwrap();
        fs::write(
            paths.ankiconnect_dir().join("__init__.py"),
            "# the user's own\n",
        )
        .unwrap();
        let fake = FakeDownloader::default();
        let files = fake_addon(&fake);

        let err = block_on(provision_files(&consented(), &paths, &fake, &files)).unwrap_err();

        assert!(matches!(err, SetupError::AlreadyExists(ref p) if *p == paths.ankiconnect_dir()));
        assert!(fake.requests().is_empty(), "nothing downloaded");
        assert_eq!(
            fs::read_to_string(paths.ankiconnect_dir().join("__init__.py")).unwrap(),
            "# the user's own\n"
        );
    }

    #[test]
    fn without_consent_nothing_is_downloaded() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        let fake = FakeDownloader::default();
        let files = fake_addon(&fake);

        let err = block_on(provision_files(
            &ConsentState::default(),
            &paths,
            &fake,
            &files,
        ))
        .unwrap_err();

        assert!(matches!(
            err,
            SetupError::ConsentRequired(ConsentAction::AnkiconnectProvision)
        ));
        assert!(fake.requests().is_empty());
        assert!(!paths.anki_base.exists());
    }

    #[test]
    fn the_legacy_stub_does_not_count_as_installed() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(ADDON_ID);
        assert!(!is_installed(&dir));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("__init__.py"), LEGACY_STUB).unwrap();
        assert!(!is_installed(&dir));
        fs::write(dir.join("__init__.py"), "import aqt\n").unwrap();
        assert!(is_installed(&dir));
    }
}
