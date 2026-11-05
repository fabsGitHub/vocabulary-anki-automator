//! Anki and AnkiConnect setup (Milestone E). Every download and install path
//! is treated as adversarial.
//!
//! The pre-refactor app downloaded and ran the Anki installer and wrote an
//! unpinned AnkiConnect into the user's Anki folder on every start, without
//! asking. Now:
//!
//! - **Startup only checks** ([`check_anki_installed`],
//!   [`check_ankiconnect_installed`]). Installing happens only from the two
//!   install commands, each refusing to run without its one-time consent in
//!   `config.consent` ([`require_consent`]).
//! - **Everything is pinned** (docs/architecture.md § 8.4): the AnkiConnect
//!   commit and the SHA-256 of each of its four files, and the Anki release
//!   with the SHA-256 of each installer. A download is hashed while it
//!   streams to disk and checked before anything else touches it; a mismatch
//!   deletes it ([`download`]).
//! - **Nothing is reachable from tests.** All locations come from an
//!   injected [`AnkiPaths`], all network access goes through a
//!   [`Downloader`](download::Downloader) and every process through a
//!   [`CommandRunner`]; the test suites pass temp dirs and fakes (Milestone A4
//!   hard rule).

pub mod anki_install;
pub mod ankiconnect;
pub mod download;

use std::{
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::{ConsentAction, ConsentState};
use download::{Downloader, HttpDownloader};

/// Every place setup reads or writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnkiPaths {
    /// Anki's data folder ("Anki2"), which holds `addons21`.
    pub anki_base: PathBuf,
    /// Where the Anki application lives: `/Applications` on macOS, Program
    /// Files on Windows.
    pub applications_dir: PathBuf,
    /// Scratch space for downloads and disk-image mounts.
    pub temp_dir: PathBuf,
}

impl AnkiPaths {
    /// The real locations on this machine. `ANKI_BASE` overrides the data
    /// folder, as it does for Anki itself.
    pub fn detect() -> Result<Self, String> {
        Self::for_platform(
            std::env::consts::OS,
            std::env::var_os("ANKI_BASE").map(PathBuf::from),
            dirs::data_dir(),
            std::env::var_os("ProgramFiles").map(PathBuf::from),
            std::env::temp_dir(),
        )
    }

    /// The rules behind [`detect`](Self::detect), with every input passed in.
    pub fn for_platform(
        os: &str,
        anki_base_override: Option<PathBuf>,
        data_dir: Option<PathBuf>,
        program_files: Option<PathBuf>,
        temp_dir: PathBuf,
    ) -> Result<Self, String> {
        let anki_base = match anki_base_override.filter(|dir| !dir.as_os_str().is_empty()) {
            Some(dir) => dir,
            None => data_dir
                .ok_or("the user's data folder is unknown (no home directory?)")?
                .join("Anki2"),
        };
        let applications_dir = match os {
            "macos" => PathBuf::from("/Applications"),
            "windows" => program_files.unwrap_or_else(|| PathBuf::from(r"C:\Program Files")),
            // Not used: Linux installs Anki through distribution packages.
            _ => PathBuf::from("/usr/local"),
        };
        Ok(Self {
            anki_base,
            applications_dir,
            temp_dir,
        })
    }

    pub fn addons_dir(&self) -> PathBuf {
        self.anki_base.join("addons21")
    }

    /// `addons21/2055492159`, AnkiConnect's AnkiWeb id.
    pub fn ankiconnect_dir(&self) -> PathBuf {
        self.addons_dir().join(ankiconnect::ADDON_ID)
    }

    /// The Anki application on this platform, `None` where it has no fixed
    /// location (Linux).
    pub fn anki_app(&self) -> Option<PathBuf> {
        anki_app_in(std::env::consts::OS, &self.applications_dir)
    }
}

fn anki_app_in(os: &str, applications_dir: &Path) -> Option<PathBuf> {
    match os {
        "macos" => Some(applications_dir.join("Anki.app")),
        "windows" => Some(applications_dir.join("Anki").join("anki.exe")),
        _ => None,
    }
}

/// Whether the Anki application is where this platform installs it.
pub fn check_anki_installed(paths: &AnkiPaths) -> bool {
    paths.anki_app().is_some_and(|app| app.exists())
}

/// Whether an AnkiConnect add-on is in place (see
/// [`ankiconnect::is_installed`]).
pub fn check_ankiconnect_installed(paths: &AnkiPaths) -> bool {
    ankiconnect::is_installed(&paths.ankiconnect_dir())
}

/// Fails unless the user consented to `action`. Every install path calls
/// this itself, so no caller can skip it.
pub fn require_consent(consent: &ConsentState, action: ConsentAction) -> Result<(), SetupError> {
    if consent.is_granted(action) {
        Ok(())
    } else {
        Err(SetupError::ConsentRequired(action))
    }
}

/// What `get_setup_status` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupStatus {
    pub anki_installed: bool,
    pub ankiconnect_installed: bool,
    /// Whether a pinned installer exists for this OS and CPU.
    pub platform_supported: bool,
    /// The Anki release the installer is pinned to.
    pub anki_version: String,
    pub consent: ConsentState,
}

/// The current [`SetupStatus`]; unknown folders read as "not installed".
pub fn setup_status(env: &SetupEnv, consent: &ConsentState) -> SetupStatus {
    let paths = env.paths.as_ref().ok();
    SetupStatus {
        anki_installed: paths.is_some_and(check_anki_installed),
        ankiconnect_installed: paths.is_some_and(check_ankiconnect_installed),
        platform_supported: anki_install::installer_for(
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
        .is_some(),
        anki_version: anki_install::ANKI_VERSION.to_string(),
        consent: consent.clone(),
    }
}

/// Runs external programs: `hdiutil`/`ditto`, the Windows installer, and the
/// launcher behind `launch_anki_desktop`.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Runs `program` to completion. An error when it cannot be started or
    /// exits unsuccessfully; Windows' "needs elevation" refusal is
    /// [`SetupError::ElevationRequired`].
    async fn run(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError>;

    /// Starts `program` without waiting for it.
    fn spawn(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError>;
}

/// [`CommandRunner`] over real processes, with stdin closed so nothing can
/// ever wait for input.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

/// `ERROR_ELEVATION_REQUIRED`: Windows refuses to start a program whose
/// manifest asks for administrator rights without the UAC prompt.
const ERROR_ELEVATION_REQUIRED: i32 = 740;

#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError> {
        let output = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|err| start_error(program, err))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        Err(SetupError::Command {
            program: program_name(program),
            message: if detail.is_empty() {
                format!("exited with {}", output.status)
            } else {
                format!("exited with {}: {}", output.status, truncate(detail, 300))
            },
        })
    }

    fn spawn(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError> {
        std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|err| start_error(program, err))
    }
}

fn start_error(program: &Path, err: io::Error) -> SetupError {
    if err.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) {
        SetupError::ElevationRequired {
            program: program_name(program),
        }
    } else {
        SetupError::Command {
            program: program_name(program),
            message: format!("could not be started: {err}"),
        }
    }
}

fn program_name(program: &Path) -> String {
    program
        .file_name()
        .unwrap_or(program.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// The folders plus the downloader and runner setup uses.
pub struct SetupEnv {
    pub paths: Result<AnkiPaths, String>,
    pub downloader: Arc<dyn Downloader>,
    pub runner: Arc<dyn CommandRunner>,
}

impl SetupEnv {
    /// The platform's folders, HTTPS downloads and real processes.
    pub fn production() -> Self {
        let paths = AnkiPaths::detect();
        if let Err(err) = &paths {
            log::error!("Anki's folders could not be located: {err}");
        }
        Self {
            paths,
            downloader: Arc::new(HttpDownloader::new()),
            runner: Arc::new(SystemCommandRunner),
        }
    }

    /// No folders, and a downloader and runner that refuse everything: for
    /// tests that have no business reaching setup at all.
    pub fn unavailable(why: &str) -> Self {
        let refuse = Arc::new(Refuse(why.to_string()));
        Self {
            paths: Err(why.to_string()),
            downloader: refuse.clone(),
            runner: refuse,
        }
    }

    pub fn paths(&self) -> Result<&AnkiPaths, SetupError> {
        self.paths
            .as_ref()
            .map_err(|why| SetupError::Paths(why.clone()))
    }
}

/// Fails every download and every process start.
struct Refuse(String);

#[async_trait]
impl Downloader for Refuse {
    async fn get(&self, url: &str) -> Result<Box<dyn download::DownloadBody>, SetupError> {
        Err(SetupError::Unavailable(format!(
            "refusing to download {url}: {}",
            self.0
        )))
    }
}

#[async_trait]
impl CommandRunner for Refuse {
    async fn run(&self, program: &Path, _: &[OsString]) -> Result<(), SetupError> {
        Err(self.refusal(program))
    }

    fn spawn(&self, program: &Path, _: &[OsString]) -> Result<(), SetupError> {
        Err(self.refusal(program))
    }
}

impl Refuse {
    fn refusal(&self, program: &Path) -> SetupError {
        SetupError::Unavailable(format!("refusing to run {}: {}", program.display(), self.0))
    }
}

/// Everything that can stop a setup action, worded for the user.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("{} needs your consent first. Confirm it in the setup dialog, then try again.", .0.describe())]
    ConsentRequired(ConsentAction),
    #[error(
        "Automatic Anki installation is not available on {0}. Install Anki from \
         https://apps.ankiweb.net instead."
    )]
    Unsupported(String),
    #[error(
        "{} already exists, so nothing was changed. If it is broken, remove it and try again.",
        .0.display()
    )]
    AlreadyExists(PathBuf),
    #[error("Could not download {url}: {message}")]
    Download { url: String, message: String },
    #[error("The download from {url} is larger than the {max} bytes allowed; it was deleted.")]
    TooLarge { url: String, max: u64 },
    #[error(
        "The download from {url} does not match its pinned SHA-256 (expected {expected}, got \
         {actual}). It was deleted and nothing was installed."
    )]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("{program} failed: {message}")]
    Command { program: String, message: String },
    #[error("{program} needs administrator rights to run.")]
    ElevationRequired { program: String },
    #[error("Could not locate Anki's folders: {0}")]
    Paths(String),
    #[error("{0}")]
    Unavailable(String),
}

impl SetupError {
    /// An `Io` error that says what was being done to which path.
    pub(crate) fn io(action: &str, path: &Path) -> impl FnOnce(io::Error) -> SetupError {
        let context = format!("Could not {action} {}", path.display());
        move |source| SetupError::Io { context, source }
    }
}

/// A private, uniquely named directory that is deleted with its contents when
/// dropped, unless [`keep`](Self::keep) was called.
pub(crate) struct ScratchDir {
    path: PathBuf,
    keep: bool,
}

impl ScratchDir {
    /// Creates `<parent>/<prefix><random>` (and `parent` if needed). The name
    /// is unpredictable and creation fails if it exists, so nothing planted
    /// in a shared temp dir is ever reused; on Unix only the user can enter
    /// it.
    pub(crate) fn new_in(parent: &Path, prefix: &str) -> Result<Self, SetupError> {
        fs::create_dir_all(parent).map_err(SetupError::io("create", parent))?;
        let path = parent.join(format!("{prefix}{}", uuid::Uuid::new_v4().simple()));
        create_private_dir(&path).map_err(SetupError::io("create", &path))?;
        Ok(Self { path, keep: false })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Leaves the directory on disk (used when a disk image may still be
    /// mounted inside it: deleting through a mount point is never safe).
    pub(crate) fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        if let Err(err) = fs::remove_dir_all(&self.path) {
            if err.kind() != io::ErrorKind::NotFound {
                log::warn!("Could not remove {}: {err}", self.path.display());
            }
        }
    }
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

/// Fails with [`SetupError::AlreadyExists`] when anything, even a dangling
/// symlink, is at `path`.
pub(crate) fn ensure_absent(path: &Path) -> Result<(), SetupError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(SetupError::AlreadyExists(path.to_path_buf())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(SetupError::io("inspect", path)(err)),
    }
}

/// `args` as owned OS strings, for [`CommandRunner`].
pub(crate) fn os_args<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect()
}

#[cfg(test)]
pub(crate) mod testing {
    //! Fakes for the setup traits, shared by the setup unit tests.

    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use super::*;
    use download::DownloadBody;

    /// Serves canned bodies by URL, in small chunks, and records every URL.
    #[derive(Default, Clone)]
    pub(crate) struct FakeDownloader {
        pub(crate) bodies: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        pub(crate) requests: Arc<Mutex<Vec<String>>>,
        /// Reported as the content length instead of the real one.
        pub(crate) announced_length: Option<u64>,
    }

    impl FakeDownloader {
        pub(crate) fn serve(&self, url: impl Into<String>, body: impl Into<Vec<u8>>) {
            self.bodies.lock().unwrap().insert(url.into(), body.into());
        }

        pub(crate) fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    struct FakeBody {
        chunks: std::vec::IntoIter<Vec<u8>>,
        length: Option<u64>,
    }

    #[async_trait]
    impl DownloadBody for FakeBody {
        fn content_length(&self) -> Option<u64> {
            self.length
        }

        async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, SetupError> {
            Ok(self.chunks.next())
        }
    }

    #[async_trait]
    impl Downloader for FakeDownloader {
        async fn get(&self, url: &str) -> Result<Box<dyn DownloadBody>, SetupError> {
            self.requests.lock().unwrap().push(url.to_string());
            let body = self
                .bodies
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| SetupError::Download {
                    url: url.to_string(),
                    message: "HTTP 404 Not Found".to_string(),
                })?;
            let chunks: Vec<Vec<u8>> = body.chunks(7).map(<[u8]>::to_vec).collect();
            Ok(Box::new(FakeBody {
                chunks: chunks.into_iter(),
                length: self.announced_length.or(Some(body.len() as u64)),
            }))
        }
    }

    /// One call to a [`FakeRunner`]: the program's file name and its args.
    pub(crate) type Call = (String, Vec<String>);

    type Script = Box<dyn Fn(&str, &[String]) -> Result<(), SetupError> + Send + Sync>;

    /// Records every call and answers through a script.
    #[derive(Clone)]
    pub(crate) struct FakeRunner {
        pub(crate) calls: Arc<Mutex<Vec<Call>>>,
        script: Arc<Script>,
    }

    impl FakeRunner {
        pub(crate) fn new(
            script: impl Fn(&str, &[String]) -> Result<(), SetupError> + Send + Sync + 'static,
        ) -> Self {
            Self {
                calls: Arc::default(),
                script: Arc::new(Box::new(script)),
            }
        }

        pub(crate) fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError> {
            let name = program.to_string_lossy().into_owned();
            let args: Vec<String> = args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            self.calls
                .lock()
                .unwrap()
                .push((name.clone(), args.clone()));
            (self.script)(&name, &args)
        }
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError> {
            self.record(program, args)
        }

        fn spawn(&self, program: &Path, args: &[OsString]) -> Result<(), SetupError> {
            self.record(program, args)
        }
    }

    pub(crate) fn temp_paths(root: &Path) -> AnkiPaths {
        AnkiPaths {
            anki_base: root.join("Anki2"),
            applications_dir: root.join("Applications"),
            temp_dir: root.join("tmp"),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    #[test]
    fn platform_defaults() {
        let data = Some(PathBuf::from("/home/me/.local/share"));
        let temp = PathBuf::from("/tmp");
        let linux =
            AnkiPaths::for_platform("linux", None, data.clone(), None, temp.clone()).unwrap();
        assert_eq!(
            linux.anki_base,
            PathBuf::from("/home/me/.local/share/Anki2")
        );
        assert_eq!(
            linux.addons_dir(),
            PathBuf::from("/home/me/.local/share/Anki2/addons21")
        );
        assert_eq!(
            linux.ankiconnect_dir(),
            PathBuf::from("/home/me/.local/share/Anki2/addons21/2055492159")
        );
        assert_eq!(linux.temp_dir, temp);

        let mac = AnkiPaths::for_platform(
            "macos",
            None,
            Some(PathBuf::from("/Users/me/Library/Application Support")),
            None,
            temp.clone(),
        )
        .unwrap();
        assert_eq!(
            mac.anki_base,
            PathBuf::from("/Users/me/Library/Application Support/Anki2")
        );
        assert_eq!(mac.applications_dir, PathBuf::from("/Applications"));
        assert_eq!(
            anki_app_in("macos", &mac.applications_dir),
            Some(PathBuf::from("/Applications/Anki.app"))
        );

        let windows = AnkiPaths::for_platform(
            "windows",
            None,
            Some(PathBuf::from("C:/Users/me/AppData/Roaming")),
            Some(PathBuf::from("D:/Programs")),
            temp.clone(),
        )
        .unwrap();
        assert_eq!(windows.applications_dir, PathBuf::from("D:/Programs"));
        assert_eq!(
            anki_app_in("windows", &windows.applications_dir),
            Some(PathBuf::from("D:/Programs").join("Anki").join("anki.exe"))
        );
        assert_eq!(anki_app_in("linux", Path::new("/usr/local")), None);
    }

    #[test]
    fn anki_base_override_wins_and_a_missing_home_is_an_error() {
        let paths = AnkiPaths::for_platform(
            "macos",
            Some(PathBuf::from("/tmp/disposable-profile")),
            None,
            None,
            PathBuf::from("/tmp"),
        )
        .unwrap();
        assert_eq!(paths.anki_base, PathBuf::from("/tmp/disposable-profile"));

        let blank_override = AnkiPaths::for_platform(
            "macos",
            Some(PathBuf::new()),
            None,
            None,
            PathBuf::from("/tmp"),
        );
        assert!(blank_override.is_err());
    }

    #[test]
    fn checks_look_only_at_the_given_folders() {
        let root = tempfile::tempdir().unwrap();
        let paths = testing::temp_paths(root.path());
        assert!(!check_anki_installed(&paths));
        assert!(!check_ankiconnect_installed(&paths));

        if let Some(app) = paths.anki_app() {
            fs::create_dir_all(&app).unwrap();
            assert!(check_anki_installed(&paths));
        }
        fs::create_dir_all(paths.ankiconnect_dir()).unwrap();
        fs::write(
            paths.ankiconnect_dir().join("__init__.py"),
            "# AnkiConnect\n",
        )
        .unwrap();
        assert!(check_ankiconnect_installed(&paths));
    }

    #[test]
    fn consent_is_required_per_action() {
        let mut consent = ConsentState::default();
        for action in [
            ConsentAction::AnkiInstall,
            ConsentAction::AnkiconnectProvision,
        ] {
            let err = require_consent(&consent, action).unwrap_err();
            assert!(matches!(err, SetupError::ConsentRequired(a) if a == action));
        }
        consent.grant(
            ConsentAction::AnkiconnectProvision,
            Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap(),
        );
        assert!(require_consent(&consent, ConsentAction::AnkiconnectProvision).is_ok());
        assert!(require_consent(&consent, ConsentAction::AnkiInstall).is_err());
        assert_eq!(
            require_consent(&ConsentState::default(), ConsentAction::AnkiInstall)
                .unwrap_err()
                .to_string(),
            "Installing Anki needs your consent first. Confirm it in the setup dialog, then try again."
        );
    }

    #[test]
    fn scratch_dirs_are_unique_private_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let first = ScratchDir::new_in(root.path(), "x-").unwrap();
        let second = ScratchDir::new_in(root.path(), "x-").unwrap();
        assert_ne!(first.path(), second.path());
        fs::write(first.path().join("file"), "data").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(first.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let first_path = first.path().to_path_buf();
        drop(first);
        assert!(!first_path.exists());

        let kept = second.keep();
        assert!(kept.exists(), "keep() leaves the directory alone");
    }

    #[test]
    fn ensure_absent_sees_dangling_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("thing");
        assert!(ensure_absent(&path).is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join("nowhere"), &path).unwrap();
            assert!(matches!(
                ensure_absent(&path),
                Err(SetupError::AlreadyExists(_))
            ));
        }
    }

    #[test]
    fn an_unavailable_env_refuses_everything() {
        let env = SetupEnv::unavailable("tests only");
        assert!(matches!(env.paths(), Err(SetupError::Paths(_))));
        let runner = env.runner.clone();
        let result = tauri::async_runtime::block_on(runner.run(Path::new("/bin/echo"), &[]));
        assert!(
            matches!(result, Err(SetupError::Unavailable(_))),
            "{result:?}"
        );
        assert!(env.runner.spawn(Path::new("/bin/echo"), &[]).is_err());
        let download = tauri::async_runtime::block_on(env.downloader.get("https://example.com/x"));
        assert!(download.is_err());
    }
}
