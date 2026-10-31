//! The Automator lives in the tray (docs/architecture.md § 8.3): closing its
//! last window must not end it, or the hotkeys and the tray go with it.
//!
//! The event-loop test runs the app `configure` builds, which `run` builds
//! too, so it covers the lifecycle plugin as the real app wires it. The mock
//! runtime raises `ExitRequested` without a code when the last window closes,
//! exactly like the desktop runtime, but cannot raise one with a code (its
//! `request_exit` is unimplemented), so `keeps_running` is also tested alone.

use std::{
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Mutex, PoisonError},
};

use tauri::{utils::config::WindowConfig, Manager, RunEvent};
use vocab_automator_lib::{configure, keeps_running, MAIN_LABEL};

/// A prevented exit never ends the mock loop, so the test ends it by
/// unwinding with this payload once it has seen the app keep running.
const STILL_RUNNING: &str = "the app is still running";

/// Loop iterations to wait for the exit request; each one sleeps 1 s.
const MAX_IDLE_LOOPS: u32 = 5;

#[test]
fn closing_the_settings_window_before_any_capture_keeps_the_app_running() {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().app.windows.push(WindowConfig {
        label: MAIN_LABEL.into(),
        ..Default::default()
    });
    let app = configure(tauri::test::mock_builder())
        .build(context)
        .expect("mock app");

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let log = Arc::clone(&seen);
    let outcome = panic::catch_unwind(AssertUnwindSafe(move || {
        let mut idle_loops = 0;
        app.run(move |handle, event| match event {
            RunEvent::Ready => {
                // No capture has built the approval window yet, so the
                // settings window is the only one.
                handle
                    .get_webview_window(MAIN_LABEL)
                    .expect("the settings window")
                    .close()
                    .expect("close the settings window");
            }
            RunEvent::ExitRequested { code, .. } => {
                log.lock()
                    .unwrap()
                    .push(format!("exit requested, code {code:?}"));
            }
            RunEvent::Exit => log.lock().unwrap().push("exit".to_owned()),
            RunEvent::MainEventsCleared => {
                let exit_requested = log.lock().unwrap().len() == 1;
                if exit_requested {
                    // The loop went on after the exit request.
                    log.lock().unwrap().push("still running".to_owned());
                    panic::panic_any(STILL_RUNNING);
                }
                idle_loops += 1;
                assert!(
                    idle_loops < MAX_IDLE_LOOPS,
                    "closing the last window raised no exit request"
                );
            }
            _ => {}
        });
    }));

    if let Err(payload) = outcome {
        if payload.downcast_ref::<&str>() != Some(&STILL_RUNNING) {
            panic::resume_unwind(payload);
        }
    }
    let seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(*seen, ["exit requested, code None", "still running"]);
}

#[test]
fn only_a_code_less_exit_request_keeps_the_app_running() {
    // The last window closed.
    assert!(keeps_running(None));
    // The tray's Quit (`app.exit(0)`) and a restart requested off the main
    // thread still end the app.
    assert!(!keeps_running(Some(0)));
    assert!(!keeps_running(Some(tauri::RESTART_EXIT_CODE)));
}
