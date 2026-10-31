//! `Automator` — select text anywhere, press a hotkey, approve the generated
//! card, and it lands in Anki and in the local history shared with `Learn`
//! (decision 10). Desktop only, by design: global hotkeys have no mobile or
//! web equivalent.
//!
//! Module map (contracts: docs/architecture.md § 8):
//!
//! | module      | milestone  | contents                                                    |
//! |-------------|------------|-------------------------------------------------------------|
//! | `config`    | B2, H      | `config.json`: shape, legacy migration, validation          |
//! | `state`     | B3, F, N4  | runtime state and the injected services                     |
//! | `capture`   | B3, D, H   | hotkey -> clipboard -> pipeline -> approval window          |
//! | `shortcuts` | H          | the default hotkey plus one per hotkey profile              |
//! | `push`      | B3, F, J5  | save-then-push of approved cards, retry of unsynced ones     |
//! | `setup`     | E          | consent-gated, pinned and verified Anki/AnkiConnect install |
//! | `commands`  | all        | the IPC surface (§ 8.2)                                     |
//!
//! Startup only checks the Anki setup; nothing is downloaded, installed or
//! launched until the user clicks (Milestone E).

pub mod capture;
pub mod commands;
pub mod config;
pub mod push;
pub mod setup;
pub mod shortcuts;
pub mod state;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Builder, Manager, RunEvent, Runtime, WebviewWindowBuilder, Window, WindowEvent,
};

use crate::{
    capture::PREVIEW_LABEL,
    state::{AppServices, AppState},
};

/// Label of the settings window (docs/architecture.md § 8.3).
pub const MAIN_LABEL: &str = "main";

/// What the real app and the `tauri::test` mock app share: the invoke handler
/// with every command of § 8.2, and the approval window's close behaviour.
/// Plugins, the tray and [`AppState`] are added by [`run`] (tests manage
/// their own state).
pub fn configure<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder
        .invoke_handler(tauri::generate_handler![
            commands::settings::get_current_settings,
            commands::settings::apply_settings,
            commands::capture::get_pending_payload,
            commands::capture::discard_pending_payload,
            commands::capture::execute_card_generation,
            commands::capture::capture_text,
            commands::anki::anki_status,
            commands::anki::anki_model_field_names,
            commands::anki::launch_anki_desktop,
            commands::setup::get_setup_status,
            commands::setup::grant_consent,
            commands::setup::download_and_install_anki,
            commands::setup::auto_provision_ankiconnect,
            commands::setup::check_ankiconnect_installed,
            commands::history::list_recent_captures,
            commands::history::delete_capture,
            commands::history::sync_pending_captures,
            commands::app::capture_in_progress,
            commands::app::restart_app,
        ])
        .on_window_event(on_window_event)
}

/// Closing the approval window discards its capture, and the window is only
/// hidden: it is reused for the next capture (§ 8.3).
fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    if window.label() != PREVIEW_LABEL {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        if let Some(state) = window.try_state::<AppState>() {
            state.pending().clear();
        }
        if let Err(err) = window.hide() {
            log::warn!("Could not hide the approval window: {err}");
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = configure(tauri::Builder::default())
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(shortcuts::on_shortcut)
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            start(app)?;
            Ok(())
        })
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|app, event| on_run_event(app, &event)),
        Err(err) => {
            log::error!("Vocabulary Automator stopped with an error: {err}");
            eprintln!("Vocabulary Automator stopped with an error: {err}");
            std::process::exit(1);
        }
    }
}

/// The app lives in the tray, so closing its last window must not end it:
/// the hotkeys and the tray would go with it. Before the first capture builds
/// the (hidden, never destroyed) approval window, the settings window is the
/// only one, and closing it asks to exit with no exit code. The tray's Quit
/// (`app.exit(0)`) and `restart_app` carry a code and still end the app;
/// Cmd+Q on macOS ends it without asking.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent) {
    match event {
        RunEvent::ExitRequested {
            code: None, api, ..
        } => {
            log::info!("No window is open; Vocabulary Automator keeps running in the tray.");
            api.prevent_exit();
        }
        // Clicking the Dock icon while no window is visible brings back the
        // settings window, as the tray's Settings item does.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(err) = show_settings(&app) {
                    log::warn!("{err}");
                }
            });
        }
        _ => {}
    }
}

/// Startup: state, tray, hotkeys, and a check (only a check) of the Anki
/// setup.
fn start(app: &mut tauri::App) -> Result<(), String> {
    log::info!("Vocabulary Automator is starting.");
    let handle = app.handle().clone();
    let config_path = config::config_path(&handle)?;
    app.manage(AppState::new(AppServices::production(config_path)));
    build_tray(app).map_err(|err| format!("could not create the tray icon: {err}"))?;

    let state = app.state::<AppState>();
    if let Err(failures) = shortcuts::register_all(&handle, &state.config()) {
        log::warn!("Some hotkeys are not active:\n{failures}");
    }
    let status = setup::setup_status(state.setup(), &state.config().consent);
    log::info!(
        "Anki installed: {}. AnkiConnect installed: {}.",
        yes_no(status.anki_installed),
        yes_no(status.ankiconnect_installed)
    );
    log::info!("Vocabulary Automator is ready.");
    Ok(())
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&settings, &separator, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => {
                // Building a window inside an event handler deadlocks on
                // Windows (WebView2), so it happens on the async runtime.
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(err) = show_settings(&app) {
                        log::warn!("{err}");
                    }
                });
            }
            "quit" => {
                log::info!("Quit chosen from the tray menu.");
                app.exit(0);
            }
            _ => {}
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

/// Shows and focuses the settings window, building it again from
/// `tauri.conf.json` when the user closed it. Closing destroys a window, and
/// the app keeps running without it ([`on_run_event`]), so the tray must be
/// able to bring it back.
pub fn show_settings<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = match app.get_webview_window(MAIN_LABEL) {
        Some(window) => window,
        None => {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == MAIN_LABEL)
                .ok_or("The settings window is not configured.")?;
            WebviewWindowBuilder::from_config(app, config)
                .and_then(|builder| builder.build())
                .map_err(|err| format!("Could not reopen the settings window: {err}"))?
        }
    };
    window
        .show()
        .and_then(|()| window.set_focus())
        .map_err(|err| format!("Could not show the settings window: {err}"))
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}
