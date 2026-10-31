//! Installing Anki itself (Milestone E): the pinned release, verified before
//! it is mounted or executed.
//!
//! The pre-refactor installer saved an unverified download under a fixed
//! name in the shared temp dir, always the Apple-silicon build (also on Intel
//! Macs), mounted it at whatever `/Volumes/Anki` turned out to be, copied
//! from there with `cp -R` and left the image attached when a step failed.
//! Now the installer matching this OS and CPU is downloaded into a private
//! temp dir and checked against the SHA-256 from the release's signed
//! checksum file. On macOS it is attached read-only, without showing up in
//! the Finder, at a mount point inside that private dir; `Anki.app` is
//! copied with `ditto` (which keeps the code signature intact); and the image
//! is detached whatever happened. On Windows the verified installer runs
//! silently. Linux users install Anki from their distribution.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use super::{
    check_anki_installed,
    download::{download_verified, Downloader},
    ensure_absent, os_args, require_consent, AnkiPaths, CommandRunner, ScratchDir, SetupError,
};
use crate::config::{ConsentAction, ConsentState};

/// The Anki release the installer is pinned to.
pub const ANKI_VERSION: &str = "24.06.3";

/// Far above the size of any pinned installer.
const MAX_INSTALLER_BYTES: u64 = 600 * 1024 * 1024;

const HDIUTIL: &str = "/usr/bin/hdiutil";
const DITTO: &str = "/usr/bin/ditto";

/// How an installer file is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    /// A disk image holding `Anki.app`.
    MacDiskImage,
    /// An NSIS installer, run silently.
    WindowsExe,
}

/// One release asset and its SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallerPin {
    pub file: &'static str,
    pub sha256: &'static str,
    pub kind: InstallerKind,
}

/// `(std::env::consts::OS, std::env::consts::ARCH)` -> installer, from
/// `anki-24.06.3-checksums.txt` (docs/architecture.md § 8.4).
const INSTALLERS: [(&str, &str, InstallerPin); 3] = [
    (
        "macos",
        "aarch64",
        InstallerPin {
            file: "anki-24.06.3-mac-apple-qt6.dmg",
            sha256: "ce2f728c98ab3711450fbc066b8fbe27e983684e5d6195be5f451475d1a48d35",
            kind: InstallerKind::MacDiskImage,
        },
    ),
    (
        "macos",
        "x86_64",
        InstallerPin {
            file: "anki-24.06.3-mac-intel-qt6.dmg",
            sha256: "51045da7f1a189119fb0117e995ea108aa4440616ffc8f43f4a4ed735a7befaa",
            kind: InstallerKind::MacDiskImage,
        },
    ),
    (
        "windows",
        "x86_64",
        InstallerPin {
            file: "anki-24.06.3-windows-qt6.exe",
            sha256: "e11ade8b94dbb5374102669ea589bc1072ea07b93897f3b36fa045f5192ef0ba",
            kind: InstallerKind::WindowsExe,
        },
    ),
];

/// The pinned installer for `os`/`arch`, `None` where there is none.
pub fn installer_for(os: &str, arch: &str) -> Option<&'static InstallerPin> {
    INSTALLERS
        .iter()
        .find(|(pin_os, pin_arch, _)| *pin_os == os && *pin_arch == arch)
        .map(|(_, _, pin)| pin)
}

/// The release asset URL of `file`.
pub fn installer_url(file: &str) -> String {
    format!("https://github.com/ankitects/anki/releases/download/{ANKI_VERSION}/{file}")
}

/// Installs the pinned Anki for this machine and says what happened. Needs
/// consent; does nothing when Anki is already installed.
pub async fn install(
    consent: &ConsentState,
    paths: &AnkiPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
) -> Result<String, SetupError> {
    require_consent(consent, ConsentAction::AnkiInstall)?;
    if check_anki_installed(paths) {
        return Ok("Anki is already installed.".to_string());
    }
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let pin =
        installer_for(os, arch).ok_or_else(|| SetupError::Unsupported(format!("{os} ({arch})")))?;
    install_pinned(paths, downloader, runner, pin).await
}

/// Downloads and verifies `pin`, then installs it. Split from [`install`] so
/// tests can pin fake installers.
pub(crate) async fn install_pinned(
    paths: &AnkiPaths,
    downloader: &dyn Downloader,
    runner: &dyn CommandRunner,
    pin: &InstallerPin,
) -> Result<String, SetupError> {
    let scratch = ScratchDir::new_in(&paths.temp_dir, "vocab-automator-anki-")?;
    let installer = scratch.path().join(pin.file);
    download_verified(
        downloader,
        &installer_url(pin.file),
        pin.sha256,
        MAX_INSTALLER_BYTES,
        &installer,
    )
    .await?;
    log::info!("{} matches its pinned checksum.", pin.file);
    match pin.kind {
        InstallerKind::MacDiskImage => {
            install_from_disk_image(&installer, scratch, &paths.applications_dir, runner).await
        }
        InstallerKind::WindowsExe => run_windows_installer(&installer, runner).await,
    }
}

/// Attaches `image` at a mount point inside `scratch`, copies `Anki.app` into
/// `applications_dir` and always detaches again. If the image might still be
/// attached afterwards, `scratch` is left on disk rather than deleted
/// through the mount point.
async fn install_from_disk_image(
    image: &Path,
    scratch: ScratchDir,
    applications_dir: &Path,
    runner: &dyn CommandRunner,
) -> Result<String, SetupError> {
    let mount_point = scratch.path().join("mount");
    fs::create_dir(&mount_point).map_err(SetupError::io("create", &mount_point))?;
    let attach_args = os_args([
        OsString::from("attach"),
        "-nobrowse".into(),
        "-readonly".into(),
        "-mountpoint".into(),
        mount_point.clone().into(),
        image.into(),
    ]);
    let attached = runner.run(Path::new(HDIUTIL), &attach_args).await;
    let copied = match &attached {
        Ok(()) => copy_app(&mount_point, applications_dir, runner).await,
        Err(_) => Ok(PathBuf::new()),
    };
    let detached = detach(&mount_point, runner).await;
    if !is_empty_dir(&mount_point) {
        let kept = scratch.keep();
        log::error!(
            "The Anki disk image may still be attached at {}; left {} in place.",
            mount_point.display(),
            kept.display()
        );
    }
    attached?;
    let installed = copied?;
    detached?;
    Ok(format!(
        "Anki {ANKI_VERSION} was installed to {}.",
        installed.display()
    ))
}

async fn copy_app(
    mount_point: &Path,
    applications_dir: &Path,
    runner: &dyn CommandRunner,
) -> Result<PathBuf, SetupError> {
    let source = mount_point.join("Anki.app");
    if !source.is_dir() {
        return Err(SetupError::Unavailable(
            "The Anki disk image does not contain Anki.app; nothing was installed.".to_string(),
        ));
    }
    let target = applications_dir.join("Anki.app");
    ensure_absent(&target)?;
    runner
        .run(
            Path::new(DITTO),
            &os_args([source.as_os_str(), target.as_os_str()]),
        )
        .await?;
    Ok(target)
}

/// `hdiutil detach`, forced on a second attempt (e.g. when Spotlight still
/// has a file open on the image).
async fn detach(mount_point: &Path, runner: &dyn CommandRunner) -> Result<(), SetupError> {
    let args = os_args([OsString::from("detach"), mount_point.into()]);
    if let Err(err) = runner.run(Path::new(HDIUTIL), &args).await {
        log::warn!("Detaching the Anki disk image failed ({err}); forcing it.");
        let mut forced = args;
        forced.push("-force".into());
        runner.run(Path::new(HDIUTIL), &forced).await?;
    }
    Ok(())
}

fn is_empty_dir(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none())
}

/// Runs the verified installer silently (`/S`). When Windows refuses because
/// the installer asks for administrator rights, it is started again through
/// PowerShell's `Start-Process -Verb RunAs`, which shows the UAC prompt.
async fn run_windows_installer(
    installer: &Path,
    runner: &dyn CommandRunner,
) -> Result<String, SetupError> {
    match runner.run(installer, &os_args(["/S"])).await {
        Ok(()) => {}
        Err(SetupError::ElevationRequired { .. }) => {
            log::info!("The Anki installer needs administrator rights; asking for them.");
            runner
                .run(&powershell(), &elevated_install_args(installer))
                .await?;
        }
        Err(err) => return Err(err),
    }
    Ok(format!("Anki {ANKI_VERSION} was installed."))
}

fn powershell() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

/// `Start-Process` for `installer` with `/S`, waiting for it and passing its
/// exit code on. The path is a PowerShell single-quoted literal (no variable
/// expansion; a `'` is written twice).
fn elevated_install_args(installer: &Path) -> Vec<OsString> {
    let path = installer.to_string_lossy().replace('\'', "''");
    os_args([
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        format!(
            "$p = Start-Process -FilePath '{path}' -ArgumentList '/S' -Verb RunAs -Wait \
             -PassThru; exit $p.ExitCode"
        ),
    ])
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::setup::{
        download::sha256_hex,
        testing::{temp_paths, FakeDownloader, FakeRunner},
    };

    const FAKE_IMAGE: &[u8] = b"not really a disk image";

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    fn consented() -> ConsentState {
        let mut consent = ConsentState::default();
        consent.grant(
            ConsentAction::AnkiInstall,
            Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap(),
        );
        consent
    }

    fn fake_pin(kind: InstallerKind) -> InstallerPin {
        InstallerPin {
            file: "anki-test.dmg",
            sha256: Box::leak(sha256_hex(FAKE_IMAGE).into_boxed_str()),
            kind,
        }
    }

    fn serving_image() -> FakeDownloader {
        let fake = FakeDownloader::default();
        fake.serve(installer_url("anki-test.dmg"), FAKE_IMAGE);
        fake
    }

    /// A runner that behaves like hdiutil/ditto: attaching puts `Anki.app`
    /// into the mount point, detaching empties it, ditto creates the target.
    /// `fail` names a program whose first call fails.
    fn fake_mac_tools(fail: Option<&'static str>) -> FakeRunner {
        let failed_once = std::sync::atomic::AtomicBool::new(false);
        FakeRunner::new(move |program, args| {
            let tool = Path::new(program)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let action = args.first().map(String::as_str).unwrap_or_default();
            let key = if tool == "hdiutil" {
                action.to_string()
            } else {
                tool.clone()
            };
            if fail == Some(key.as_str())
                && !failed_once.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(SetupError::Command {
                    program: tool,
                    message: "simulated failure".into(),
                });
            }
            match key.as_str() {
                "attach" => fs::create_dir_all(Path::new(&args[4]).join("Anki.app")).unwrap(),
                "detach" => {
                    let _ = fs::remove_dir_all(Path::new(&args[1]).join("Anki.app"));
                }
                "ditto" => fs::create_dir_all(&args[1]).unwrap(),
                other => panic!("unexpected call {other}"),
            }
            Ok(())
        })
    }

    #[test]
    fn the_pins_are_the_contract_values() {
        let cases = [
            (
                "macos",
                "aarch64",
                Some((
                    "anki-24.06.3-mac-apple-qt6.dmg",
                    "ce2f728c98ab3711450fbc066b8fbe27e983684e5d6195be5f451475d1a48d35",
                )),
            ),
            (
                "macos",
                "x86_64",
                Some((
                    "anki-24.06.3-mac-intel-qt6.dmg",
                    "51045da7f1a189119fb0117e995ea108aa4440616ffc8f43f4a4ed735a7befaa",
                )),
            ),
            (
                "windows",
                "x86_64",
                Some((
                    "anki-24.06.3-windows-qt6.exe",
                    "e11ade8b94dbb5374102669ea589bc1072ea07b93897f3b36fa045f5192ef0ba",
                )),
            ),
            ("windows", "aarch64", None),
            ("linux", "x86_64", None),
            ("freebsd", "x86_64", None),
        ];
        for (os, arch, expected) in cases {
            let actual = installer_for(os, arch).map(|pin| (pin.file, pin.sha256));
            assert_eq!(actual, expected, "{os} {arch}");
        }
        assert_eq!(
            installer_url("anki-24.06.3-windows-qt6.exe"),
            "https://github.com/ankitects/anki/releases/download/24.06.3/anki-24.06.3-windows-qt6.exe"
        );
    }

    #[test]
    fn a_verified_image_is_attached_copied_and_detached() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        fs::create_dir_all(&paths.applications_dir).unwrap();
        let runner = fake_mac_tools(None);

        let message = block_on(install_pinned(
            &paths,
            &serving_image(),
            &runner,
            &fake_pin(InstallerKind::MacDiskImage),
        ))
        .unwrap();

        let target = paths.applications_dir.join("Anki.app");
        assert!(target.is_dir());
        assert_eq!(
            message,
            format!("Anki 24.06.3 was installed to {}.", target.display())
        );
        let calls = runner.calls();
        assert_eq!(calls.len(), 3, "{calls:?}");
        let (program, attach) = &calls[0];
        assert_eq!(program, HDIUTIL);
        assert_eq!(
            &attach[..4],
            ["attach", "-nobrowse", "-readonly", "-mountpoint"]
        );
        let mount_point = PathBuf::from(&attach[4]);
        assert!(mount_point.starts_with(&paths.temp_dir), "{mount_point:?}");
        assert!(attach[5].ends_with("anki-test.dmg"));
        assert_eq!(
            calls[1],
            (
                DITTO.to_string(),
                vec![
                    mount_point.join("Anki.app").to_string_lossy().into_owned(),
                    target.to_string_lossy().into_owned()
                ]
            )
        );
        assert_eq!(
            calls[2],
            (
                HDIUTIL.to_string(),
                vec![
                    "detach".to_string(),
                    mount_point.to_string_lossy().into_owned()
                ]
            )
        );
        assert_eq!(
            fs::read_dir(&paths.temp_dir).unwrap().count(),
            0,
            "scratch removed"
        );
    }

    #[test]
    fn the_image_is_detached_even_when_copying_fails() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        fs::create_dir_all(&paths.applications_dir).unwrap();
        let runner = fake_mac_tools(Some("ditto"));

        let err = block_on(install_pinned(
            &paths,
            &serving_image(),
            &runner,
            &fake_pin(InstallerKind::MacDiskImage),
        ))
        .unwrap_err();

        assert!(matches!(err, SetupError::Command { ref program, .. } if program == "ditto"));
        let actions: Vec<String> = runner
            .calls()
            .iter()
            .map(|(_, args)| args[0].clone())
            .collect();
        assert_eq!(actions.first().map(String::as_str), Some("attach"));
        assert_eq!(actions.last().map(String::as_str), Some("detach"));
        assert_eq!(fs::read_dir(&paths.temp_dir).unwrap().count(), 0);
    }

    #[test]
    fn a_stuck_image_is_force_detached_and_never_deleted_through() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        fs::create_dir_all(&paths.applications_dir).unwrap();
        // The first detach fails; the forced one succeeds.
        let runner = fake_mac_tools(Some("detach"));

        let result = block_on(install_pinned(
            &paths,
            &serving_image(),
            &runner,
            &fake_pin(InstallerKind::MacDiskImage),
        ));

        assert!(result.is_ok(), "{result:?}");
        let last = runner.calls().last().cloned().unwrap();
        assert_eq!(last.1.first().map(String::as_str), Some("detach"));
        assert_eq!(last.1.last().map(String::as_str), Some("-force"));

        // A detach that keeps failing leaves the scratch dir (and the image's
        // contents) alone instead of deleting through the mount point.
        let stuck_root = tempfile::tempdir().unwrap();
        let stuck = temp_paths(stuck_root.path());
        fs::create_dir_all(&stuck.applications_dir).unwrap();
        let always_failing_detach = FakeRunner::new(|program, args| {
            match args.first().map(String::as_str) {
                Some("attach") => fs::create_dir_all(Path::new(&args[4]).join("Anki.app")).unwrap(),
                Some("detach") => {
                    return Err(SetupError::Command {
                        program: program.to_string(),
                        message: "resource busy".into(),
                    })
                }
                _ => fs::create_dir_all(&args[1]).unwrap(),
            }
            Ok(())
        });
        let err = block_on(install_pinned(
            &stuck,
            &serving_image(),
            &always_failing_detach,
            &fake_pin(InstallerKind::MacDiskImage),
        ))
        .unwrap_err();
        assert!(err.to_string().contains("resource busy"), "{err}");
        let kept: Vec<_> = fs::read_dir(&stuck.temp_dir).unwrap().collect();
        assert_eq!(kept.len(), 1, "the scratch dir is kept");
        let mount = kept[0].as_ref().unwrap().path().join("mount");
        assert!(mount.join("Anki.app").exists());
    }

    #[test]
    fn a_tampered_installer_is_never_mounted_or_run() {
        for kind in [InstallerKind::MacDiskImage, InstallerKind::WindowsExe] {
            let root = tempfile::tempdir().unwrap();
            let paths = temp_paths(root.path());
            let fake = FakeDownloader::default();
            fake.serve(installer_url("anki-test.dmg"), b"tampered".to_vec());
            let runner = FakeRunner::new(|program, _| panic!("{program} must not run"));

            let err =
                block_on(install_pinned(&paths, &fake, &runner, &fake_pin(kind))).unwrap_err();

            assert!(
                matches!(err, SetupError::ChecksumMismatch { .. }),
                "{err:?}"
            );
            assert!(runner.calls().is_empty());
            assert_eq!(
                fs::read_dir(&paths.temp_dir).unwrap().count(),
                0,
                "download deleted"
            );
        }
    }

    #[test]
    fn an_existing_app_is_not_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        fs::create_dir_all(paths.applications_dir.join("Anki.app")).unwrap();
        let runner = fake_mac_tools(None);

        let err = block_on(install_pinned(
            &paths,
            &serving_image(),
            &runner,
            &fake_pin(InstallerKind::MacDiskImage),
        ))
        .unwrap_err();

        assert!(matches!(err, SetupError::AlreadyExists(_)), "{err:?}");
        let actions: Vec<String> = runner
            .calls()
            .iter()
            .map(|(_, args)| args[0].clone())
            .collect();
        assert_eq!(
            actions,
            ["attach", "detach"],
            "no ditto, but still detached"
        );
    }

    #[test]
    fn the_windows_installer_runs_silently_and_elevates_when_asked() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        let pin = fake_pin(InstallerKind::WindowsExe);

        let plain = FakeRunner::new(|_, _| Ok(()));
        let message = block_on(install_pinned(&paths, &serving_image(), &plain, &pin)).unwrap();
        assert_eq!(message, "Anki 24.06.3 was installed.");
        let calls = plain.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.ends_with("anki-test.dmg"));
        assert_eq!(calls[0].1, ["/S"]);

        let elevating = FakeRunner::new(|program, _| {
            if program.ends_with("anki-test.dmg") {
                Err(SetupError::ElevationRequired {
                    program: "anki-test.dmg".into(),
                })
            } else {
                Ok(())
            }
        });
        block_on(install_pinned(&paths, &serving_image(), &elevating, &pin)).unwrap();
        let calls = elevating.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].0.ends_with("powershell.exe"), "{:?}", calls[1]);
        let command = calls[1].1.last().unwrap();
        assert!(command.contains("-Verb RunAs -Wait"), "{command}");
        assert!(command.contains("-ArgumentList '/S'"), "{command}");
        assert!(
            command.contains(&calls[0].0),
            "the verified file is the one elevated"
        );
    }

    #[test]
    fn powershell_quoting_doubles_single_quotes() {
        let args = elevated_install_args(Path::new(r"C:\Users\O'Brien\anki.exe"));
        let command = args.last().unwrap().to_string_lossy().into_owned();
        assert!(
            command.contains(r"-FilePath 'C:\Users\O''Brien\anki.exe'"),
            "{command}"
        );
    }

    #[test]
    fn install_requires_consent_and_skips_an_installed_anki() {
        let root = tempfile::tempdir().unwrap();
        let paths = temp_paths(root.path());
        let fake = FakeDownloader::default();
        let runner = FakeRunner::new(|program, _| panic!("{program} must not run"));

        let err = block_on(install(&ConsentState::default(), &paths, &fake, &runner)).unwrap_err();
        assert!(matches!(
            err,
            SetupError::ConsentRequired(ConsentAction::AnkiInstall)
        ));
        assert!(fake.requests().is_empty());

        if let Some(app) = paths.anki_app() {
            fs::create_dir_all(app).unwrap();
            let message = block_on(install(&consented(), &paths, &fake, &runner)).unwrap();
            assert_eq!(message, "Anki is already installed.");
            assert!(fake.requests().is_empty());
        }
    }
}
