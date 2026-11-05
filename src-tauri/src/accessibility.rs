//! The macOS Accessibility permission, which the simulated Cmd+C of every
//! capture needs (`capture::press_copy_shortcut`): without it macOS drops the
//! keystroke and nothing is copied.
//!
//! No app can grant the permission itself, not even a root installer (the
//! TCC database is protected by SIP). The setup assistant therefore checks it
//! up front ([`Accessibility::is_trusted`], polled while its step is shown),
//! and its button adds Polywort to the Accessibility list, shows macOS's own
//! alert ([`Accessibility::prompt`]) and opens the list
//! ([`Accessibility::open_settings`]), so all the user does is flip the switch.
//!
//! Every call goes through the [`Accessibility`] trait, injected through
//! `AppServices`: the test suites pass fakes and never reach the real API.
//! Other platforms need no permission ([`SystemAccessibility`] reports it as
//! not required), and the assistant hides the step there.
//!
//! macOS ties the permission to the app's code signature. The app is ad-hoc
//! signed, so every update is a new signature: the entry in the list may still
//! show as switched on while `AXIsProcessTrusted` says no, and only removing
//! and adding it again helps (the assistant says so).

use serde::{Deserialize, Serialize};

/// The Accessibility list in System Settings (Privacy & Security).
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

/// What `get_accessibility_status` and `request_accessibility` report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityStatus {
    /// Whether this platform asks for the permission at all (macOS only).
    pub required: bool,
    /// `AXIsProcessTrusted()`; always true where the permission is not
    /// required.
    pub trusted: bool,
}

/// The permission check and the two ways of asking for it.
pub trait Accessibility: Send + Sync {
    /// Whether this platform needs the permission for the copy keystroke.
    fn required(&self) -> bool;

    /// `AXIsProcessTrusted()`: whether the user switched Polywort on. Cheap
    /// enough to poll every second.
    fn is_trusted(&self) -> bool;

    /// `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt`:
    /// adds Polywort (switched off) to the Accessibility list and shows macOS's
    /// alert pointing there, unless it is trusted already. Returns the trust.
    fn prompt(&self) -> bool;

    /// Opens System Settings at the Accessibility list ([`SETTINGS_URL`]).
    fn open_settings(&self) -> Result<(), String>;

    fn status(&self) -> AccessibilityStatus {
        if self.required() {
            AccessibilityStatus {
                required: true,
                trusted: self.is_trusted(),
            }
        } else {
            AccessibilityStatus {
                required: false,
                trusted: true,
            }
        }
    }
}

/// The real thing: ApplicationServices on macOS, nothing to ask for elsewhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemAccessibility;

#[cfg(target_os = "macos")]
impl Accessibility for SystemAccessibility {
    fn required(&self) -> bool {
        true
    }

    fn is_trusted(&self) -> bool {
        macos::is_trusted()
    }

    fn prompt(&self) -> bool {
        macos::prompt()
    }

    fn open_settings(&self) -> Result<(), String> {
        let status = std::process::Command::new("/usr/bin/open")
            .arg(SETTINGS_URL)
            .stdin(std::process::Stdio::null())
            .status()
            .map_err(|err| format!("System Settings could not be opened: {err}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("System Settings could not be opened ({status})."))
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl Accessibility for SystemAccessibility {
    fn required(&self) -> bool {
        false
    }

    fn is_trusted(&self) -> bool {
        true
    }

    fn prompt(&self) -> bool {
        true
    }

    fn open_settings(&self) -> Result<(), String> {
        Err("This system needs no Accessibility permission.".to_string())
    }
}

/// Direct FFI to ApplicationServices and CoreFoundation: three functions and
/// three constants do not justify a binding crate.
#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    type Boolean = u8;
    type CFTypeRef = *const c_void;
    type CFIndex = isize;

    /// `CFDictionaryKeyCallBacks` / `CFDictionaryValueCallBacks`: only their
    /// addresses are passed on.
    #[repr(C)]
    struct CallBacks {
        _opaque: [u8; 0],
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> Boolean;
        fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> Boolean;
        static kAXTrustedCheckOptionPrompt: CFTypeRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: CFTypeRef;
        static kCFTypeDictionaryKeyCallBacks: CallBacks;
        static kCFTypeDictionaryValueCallBacks: CallBacks;
        fn CFDictionaryCreate(
            allocator: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: CFIndex,
            key_callbacks: *const CallBacks,
            value_callbacks: *const CallBacks,
        ) -> CFTypeRef;
        fn CFRelease(object: CFTypeRef);
    }

    pub(super) fn is_trusted() -> bool {
        // SAFETY: takes no arguments and only reads the process's TCC state.
        unsafe { AXIsProcessTrusted() != 0 }
    }

    pub(super) fn prompt() -> bool {
        // SAFETY: the dictionary holds two immortal constants, is created with
        // the CFType callbacks (which retain them), outlives the call and is
        // released exactly once. A null dictionary (out of memory) is passed
        // as "no options", which only skips the alert.
        unsafe {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            );
            let trusted = AXIsProcessTrustedWithOptions(options) != 0;
            if !options.is_null() {
                CFRelease(options);
            }
            trusted
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A fake for the unit tests; the `tauri::test` suites have their own.

    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    use super::*;

    /// Reports `trusted` and records `prompt` and `open_settings`.
    #[derive(Clone, Default)]
    pub(crate) struct FakeAccessibility {
        pub(crate) required: bool,
        pub(crate) trusted: Arc<AtomicBool>,
        pub(crate) calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Accessibility for FakeAccessibility {
        fn required(&self) -> bool {
            self.required
        }

        fn is_trusted(&self) -> bool {
            self.trusted.load(Ordering::SeqCst)
        }

        fn prompt(&self) -> bool {
            self.calls.lock().unwrap().push("prompt");
            self.is_trusted()
        }

        fn open_settings(&self) -> Result<(), String> {
            self.calls.lock().unwrap().push("open_settings");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{testing::FakeAccessibility, *};

    #[test]
    fn a_platform_without_the_permission_reports_it_as_granted() {
        let fake = FakeAccessibility::default();
        assert_eq!(
            fake.status(),
            AccessibilityStatus {
                required: false,
                trusted: true
            }
        );
    }

    #[test]
    fn the_status_follows_the_switch() {
        let fake = FakeAccessibility {
            required: true,
            ..Default::default()
        };
        assert!(!fake.status().trusted);
        fake.trusted.store(true, Ordering::SeqCst);
        assert!(fake.status().trusted);
        assert!(
            fake.calls.lock().unwrap().is_empty(),
            "checking asks nothing"
        );
    }

    #[test]
    fn the_settings_url_names_the_accessibility_list() {
        assert!(SETTINGS_URL.starts_with("x-apple.systempreferences:"));
        assert!(SETTINGS_URL.ends_with("?Privacy_Accessibility"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn only_macos_needs_the_permission() {
        assert_eq!(
            SystemAccessibility.status(),
            AccessibilityStatus {
                required: false,
                trusted: true
            }
        );
        assert!(SystemAccessibility.open_settings().is_err());
    }
}
