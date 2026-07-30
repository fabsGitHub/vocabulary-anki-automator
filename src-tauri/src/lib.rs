mod config;
mod api_client;
mod anki;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_clipboard_manager::ClipboardExt;
use enigo::{Enigo, Key, Keyboard, Settings};
use std::time::Duration;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// Safe thread-bound global memory repository holding the current window payload state 
#[derive(Default)]
pub struct PendingPayload(pub Mutex<Option<serde_json::Value>>);

/// Locates the native platform-specific root folder for Anki 2 data structures
fn get_anki_dir() -> Option<PathBuf> {
    match std::env::consts::OS {
        "macos" => dirs::home_dir().map(|p| p.join("Library/Application Support/Anki2")),
        "windows" => std::env::var("APPDATA").ok().map(|p| PathBuf::from(p).join("Anki2")),
        _ => dirs::home_dir().map(|p| p.join(".local/share/Anki2")),
    }
}

/// Checks if the Anki executable exists on the system
fn check_anki_installed() -> bool {
    match std::env::consts::OS {
        "macos" => PathBuf::from("/Applications/Anki.app").exists(),
        "windows" => {
            PathBuf::from(r"C:\Program Files\Anki\Anki.exe").exists() || 
            PathBuf::from(r"C:\Program Files (x86)\Anki\Anki.exe").exists()
        }
        _ => false,
    }
}

/// Automates the downloading and OS-level installation of Anki
async fn download_and_install_anki(_app_handle: &AppHandle) -> Result<(), String> {
    if check_anki_installed() {
        log::info!("Anki Desktop environment validation passed. Target already present on disk.");
        return Ok(());
    }

    log::warn!("Anki Desktop binary environment target missing. Initializing automated background provisioning setup.");
    let temp_dir = std::env::temp_dir();
    
    match std::env::consts::OS {
        "windows" => {
            let installer_url = "https://github.com/ankitects/anki/releases/download/24.06.3/anki-24.06.3-windows-qt6.exe";
            let installer_path = temp_dir.join("anki_installer.exe");
            
            log::info!("Downloading automated Windows package payload installer from upstream sources...");
            let response = reqwest::get(installer_url).await.map_err(|e| e.to_string())?;
            let bytes = response.bytes().await.map_err(|e| e.to_string())?;
            fs::write(&installer_path, bytes).map_err(|e| e.to_string())?;

            log::info!("Executing background execution matrix for silent setup configurations...");
            Command::new(&installer_path)
                .arg("/S")
                .status()
                .map_err(|e| format!("Failed to run installer execution routine: {}", e))?;
        }
        "macos" => {
            let installer_url = "https://github.com/ankitects/anki/releases/download/24.06.3/anki-24.06.3-mac-apple-qt6.dmg";
            let dmg_path = temp_dir.join("anki_installer.dmg");
            
            log::info!("Downloading automated Apple Silicon DMG container from upstream sources...");
            let response = reqwest::get(installer_url).await.map_err(|e| e.to_string())?;
            let bytes = response.bytes().await.map_err(|e| e.to_string())?;
            fs::write(&dmg_path, bytes).map_err(|e| e.to_string())?;

            log::info!("Mounting and cloning system image definitions directly into Applications root...");
            Command::new("hdiutil").arg("attach").arg(&dmg_path).status().map_err(|e| e.to_string())?;
            Command::new("cp").arg("-R").arg("/Volumes/Anki/Anki.app").arg("/Applications/").status().map_err(|e| e.to_string())?;
            Command::new("hdiutil").arg("detach").arg("/Volumes/Anki").status().map_err(|e| e.to_string())?;
        }
        _ => return Err("Unsupported OS deployment target architecture verified for auto extraction standard.".into()),
    }
    
    tokio::time::sleep(Duration::from_secs(3)).await;
    log::info!("Anki background installation sequences completed successfully.");
    Ok(())
}

#[tauri::command]
fn launch_anki_desktop() -> Result<String, String> {
    log::info!("Received execution trigger to launch localized Anki Desktop window pipeline.");
    match std::env::consts::OS {
        "macos" => {
            Command::new("open").arg("-a").arg("Anki").spawn().map_err(|e| e.to_string())?;
            Ok("Launching macOS Anki execution framework context...".to_string())
        }
        "windows" => {
            let win_paths = [r"C:\Program Files\Anki\Anki.exe", r"C:\Program Files (x86)\Anki\Anki.exe"];
            for path in win_paths.iter() {
                if PathBuf::from(path).exists() {
                    Command::new(path).spawn().map_err(|e| e.to_string())?;
                    return Ok("Launching Windows Anki process matrix...".to_string());
                }
            }
            log::error!("Anki workspace binary targeting paths completely empty.");
            Err("Anki.exe target binary not located inside default paths.".to_string())
        }
        _ => Err("Unsupported runtime target OS profile configuration layout.".to_string())
    }
}

#[tauri::command]
fn check_ankiconnect_installed() -> bool {
    let check_status = get_anki_dir().map(|p| p.join("addons21").join("2055492159").exists()).unwrap_or(false);
    log::info!("Diagnosing local disk volumes for target addon validation: {}", check_status);
    check_status
}

#[tauri::command]
async fn auto_provision_ankiconnect() -> Result<String, String> {
    log::info!("Initializing safe write-layer workspace directories for AnkiConnect configurations.");
    let anki_path = get_anki_dir().ok_or("Could not locate native Anki storage data path.")?;
    let plugin_dir = anki_path.join("addons21").join("2055492159");
    
    fs::create_dir_all(&plugin_dir).map_err(|e| e.to_string())?;

    let init_content = "from . import ankiconnect\n";
    let config_content = r#"{"apiKey": null, "webBindAddress": "127.0.0.1", "webBindPort": 8765, "webCorsOriginList": ["*"]}"#;

    fs::write(plugin_dir.join("__init__.py"), init_content).map_err(|e| e.to_string())?;
    fs::write(plugin_dir.join("config.json"), config_content).map_err(|e| e.to_string())?;

    log::info!("Streaming python network mapping routines straight from master code source endpoints...");
    let script_url = "https://raw.githubusercontent.com/FooSoft/anki-connect/master/ankiconnect.py";
    let response = reqwest::get(script_url).await.map_err(|e| e.to_string())?;
    let script_bytes = response.bytes().await.map_err(|e| e.to_string())?;

    fs::write(plugin_dir.join("ankiconnect.py"), script_bytes).map_err(|e| e.to_string())?;
    log::info!("AnkiConnect network scripting infrastructure layers deployed securely.");
    Ok("Provisioned AnkiConnect.".to_string())
}

#[tauri::command]
fn get_current_settings(app: AppHandle) -> serde_json::Value {
    log::debug!("Retrieving current tracking profile parameters state machine map.");
    serde_json::to_value(config::load_config(&app)).unwrap_or(serde_json::Value::Null)
}

#[tauri::command]
fn apply_settings(app: AppHandle, new_cfg: config::AppConfig) -> Result<(), String> {
    log::info!("Writing new execution tracking adjustments to user runtime configurations.");
    let old_cfg = config::load_config(&app);
    config::save_config(&app, &new_cfg)?;
    
    if old_cfg.hotkeys != new_cfg.hotkeys {
        log::info!("Hotkey modification map vector variance traced. Recycling key binding hooks.");
        if let Ok(old_sc) = old_cfg.hotkeys.parse::<Shortcut>() {
            let _ = app.global_shortcut().unregister(old_sc);
        }
        if let Ok(new_sc) = new_cfg.hotkeys.parse::<Shortcut>() {
            app.global_shortcut().register(new_sc).map_err(|e| e.to_string())?;
            log::info!("Successfully registered and tracked mapping shortcut: {}", new_cfg.hotkeys);
        }
    }
    Ok(())
}

#[tauri::command]
fn get_pending_payload(state: tauri::State<'_, PendingPayload>) -> Option<serde_json::Value> {
    state.0.lock().ok().and_then(|guard| guard.clone())
}

#[tauri::command]
async fn execute_card_generation(app: AppHandle, front: String, back: String, example: String, note: String) -> Result<String, String> {
    log::info!("Executing secure forward/reversed pairwise flashcard injection sequences.");
    let cfg = config::load_config(&app);
    let anki_client = anki::AnkiClient::new(cfg.anki.connect_url);
    
    // 1. Forward generation (Source -> Target) (Supplying 8 arguments)
    anki_client.add_vocab_card(
        &cfg.anki.deck_name, 
        &cfg.anki.model_name, 
        &front, 
        &back, 
        &example, 
        &note,
        &cfg.deepl.source_lang,
        &cfg.deepl.target_lang,
    ).await?;
    
    // 2. Reversed generation (Target -> Source) (Supplying 8 arguments)
    anki_client.add_vocab_card(
        &cfg.anki.deck_name, 
        &cfg.anki.model_name, 
        &back, 
        &front, 
        &example, 
        &note,
        &cfg.deepl.target_lang,
        &cfg.deepl.source_lang,
    ).await?;
    
    log::info!("Pairwise injection sequences executed without connection tracking faults.");
    Ok("Pairwise flashcards securely synchronized!".to_string())
}

fn trigger_clipboard_sequence(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        log::info!("Global shortcut activation trigger localized. Pausing to clear hardware keys...");
        
        // 1. Boosted delay buffer (No enigo instance exists here yet)
        tokio::time::sleep(Duration::from_millis(350)).await;

        let _ = app.clipboard().write_text("".to_string());

        // 2. Isolate Enigo to a synchronous block so it is dropped before any .await
        {
            if let Ok(mut enigo) = Enigo::new(&Settings::default()) {
                log::debug!("Virtual system driver interface linked successfully.");
                
                #[cfg(target_os = "macos")]
                {
                    let _ = enigo.key(Key::Shift, enigo::Direction::Release);
                    let _ = enigo.key(Key::Meta, enigo::Direction::Release);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    
                    let _ = enigo.key(Key::Meta, enigo::Direction::Press);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    
                    let _ = enigo.key(Key::Unicode('c'), enigo::Direction::Click); 
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    
                    let _ = enigo.key(Key::Meta, enigo::Direction::Release);
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = enigo.key(Key::Shift, enigo::Direction::Release);
                    let _ = enigo.key(Key::Control, enigo::Direction::Release);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    
                    let _ = enigo.key(Key::Control, enigo::Direction::Press);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    let _ = enigo.key(Key::Unicode('c'), enigo::Direction::Click);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                    let _ = enigo.key(Key::Control, enigo::Direction::Release);
                }
            } else {
                log::error!("Failed to obtain active input loop handle from Enigo OS layers.");
            }
        } // 💡 mut enigo is cleanly dropped here! It is no longer alive across the upcoming .await.

        // 3. Wait for pasteboard subsystem to update
        tokio::time::sleep(Duration::from_millis(300)).await;

        if let Ok(copied_text) = app.clipboard().read_text() {
            let copied_text = copied_text.trim().to_string();
            log::info!("Clipboard polling operation completed. String length evaluated: {}", copied_text);
            
            if copied_text.is_empty() { 
                log::warn!("Clipboard interface buffer contains empty data vectors. Dropping execution pipeline sequence.");
                return; 
            }

            let cfg = config::load_config(&app);
            if copied_text.len() > cfg.deepl.max_chars { 
                log::warn!("Traced layout text content length exceeds maximum permitted parameters.");
                return; 
            }

            log::info!("Contacting DeepL language engine APIs for processing...");
            let api = api_client::ApiClient::new();
            let translated = api.translate(&copied_text, &cfg.deepl.source_lang, &cfg.deepl.target_lang).await;
            let (is_rare, synonym) = api.analyze_rarity(&copied_text).await;
            
            let sample_src = api.fetch_example(&copied_text).await;
            let sample_tgt = api.translate(&sample_src, &cfg.deepl.source_lang, &cfg.deepl.target_lang).await;
            let combined_example = format!("{}\n{}", sample_src, sample_tgt);

            let note_meta = if is_rare { format!("Rare context word. Synonym alternative: {}", synonym.unwrap_or_default()) } else { String::new() };

            let payload = serde_json::json!({
                "front": copied_text,
                "back": translated,
                "example": combined_example,
                "notes": note_meta
            });

            let state = app.state::<PendingPayload>();
            if let Ok(mut guard) = state.0.lock() {
                *guard = Some(payload.clone());
            }

            if let Some(win) = app.get_webview_window("preview") {
                log::info!("Updating properties inside existing foreground window context view target.");
                let _ = win.emit("translation-payload", &payload);
                let _ = win.show();
                let _ = win.set_focus();
            } else {
                log::info!("Constructing safe application focus presentation screen wrapper elements.");
                let _ = WebviewWindowBuilder::new(&app, "preview", WebviewUrl::App("index.html".into()))
                    .title("Anki Card Approval")
                    .inner_size(480.0, 360.0)
                    .resizable(false)
                    .always_on_top(true)
                    .build();
            }
        } else {
            log::error!("System failed to pull textual contents out of OS core tracking buffers safely.");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() == ShortcutState::Released {
                        let config = config::load_config(app);
                        if let Ok(saved_shortcut) = config.hotkeys.parse::<Shortcut>() {
                            if shortcut == &saved_shortcut {
                                trigger_clipboard_sequence(app.clone());
                            }
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_current_settings, apply_settings, execute_card_generation,
            check_ankiconnect_installed, auto_provision_ankiconnect, launch_anki_desktop,
            get_pending_payload
        ])
        .setup(move |app| {
            log::info!("Bootstrapping application core interface matrices...");
            
            // Register global thread-safe state map
            app.manage(PendingPayload(Mutex::new(None)));

            let app_handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = download_and_install_anki(&app_handle).await;
                if !check_ankiconnect_installed() {
                    let _ = auto_provision_ankiconnect().await;
                }
                let _ = launch_anki_desktop();
            });

            let tray_menu = tauri::menu::Menu::with_items(app, &[
                &tauri::menu::MenuItem::with_id(app, "settings", "Settings Menu", true, None::<&str>).map_err(|e| e.to_string())?,
                &tauri::menu::PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?,
                &tauri::menu::MenuItem::with_id(app, "quit", "Quit", true, None::<&str>).map_err(|e| e.to_string())?,
            ]).map_err(|e| e.to_string())?;

            let app_icon = app.default_window_icon().cloned();
            let mut tray_builder = tauri::tray::TrayIconBuilder::new()
                .menu(&tray_menu)
                .on_menu_event(move |app, event| {
                    if event.id.as_ref() == "settings" {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    } else if event.id.as_ref() == "quit" {
                        log::info!("Application system termination sequence requested via tray click layout.");
                        app.exit(0);
                    }
                });

            if let Some(icon) = app_icon {
                tray_builder = tray_builder.icon(icon);
            }

            let _tray = tray_builder.build(app).map_err(|e| e.to_string())?;

            let config = config::load_config(app.app_handle());
            if let Ok(shortcut) = config.hotkeys.parse::<Shortcut>() {
                let _ = app.global_shortcut().register(shortcut);
                log::info!("Global monitoring system live. Tracking hotkey combination: {}", config.hotkeys);
            }
            
            log::info!("Application core startup routine completed cleanly.");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}