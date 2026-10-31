//! The Automator lives in the tray (docs/architecture.md § 8.3): closing its
//! last window must not end it, or the hotkeys and the tray go with it.
//!
//! Runs the mock runtime's event loop, which raises `ExitRequested` with no
//! code when the last window closes, exactly like the desktop runtime.

use std::{cell::RefCell, rc::Rc};

use tauri::{utils::config::WindowConfig, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

#[test]
fn closing_the_settings_window_before_any_capture_keeps_the_app_running() {
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().app.windows.push(WindowConfig {
        label: vocab_automator_lib::MAIN_LABEL.into(),
        ..Default::default()
    });
    let app = vocab_automator_lib::configure(tauri::test::mock_builder())
        .build(context)
        .expect("mock app");

    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&seen);
    app.run(move |handle, event| {
        let mut log = log.borrow_mut();
        match event {
            RunEvent::Ready => {
                // No capture has built the approval window yet, so the
                // settings window is the only one.
                handle
                    .get_webview_window(vocab_automator_lib::MAIN_LABEL)
                    .expect("the settings window")
                    .close()
                    .expect("close the settings window");
            }
            RunEvent::ExitRequested { .. } if log.is_empty() => {
                log.push("exit requested");
                vocab_automator_lib::on_run_event(handle, &event);
            }
            RunEvent::MainEventsCleared if log.as_slice() == ["exit requested"] => {
                // The loop goes on, so the app is still running. End the test
                // with a second last-window close the handler is not asked
                // about: the mock runtime has no `exit`.
                log.push("still running");
                WebviewWindowBuilder::new(handle, "end-of-test", WebviewUrl::default())
                    .build()
                    .expect("a window to close")
                    .destroy()
                    .expect("destroy it");
            }
            RunEvent::Exit => log.push("exit"),
            _ => {}
        }
    });

    assert_eq!(*seen.borrow(), ["exit requested", "still running", "exit"]);
}
