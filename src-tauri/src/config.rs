use std::fs::{create_dir_all, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use serde::{Serialize, Deserialize};
use tauri::AppHandle;
use tauri::Manager;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeepLConfig {
    pub api_key: String,
    pub source_lang: String,
    pub target_lang: String,
    pub max_chars: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AnkiConfig {
    pub connect_url: String,
    pub deck_name: String,
    pub model_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppConfig {
    pub ui_lang: String,      // ✨ FIXED: Added to persist frontend language choices
    pub hotkeys: String,      // ✨ FIXED: Changed from struct to plain String to match JS payload
    pub deepl: DeepLConfig,
    pub anki: AnkiConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui_lang: "en".to_string(),
            hotkeys: "CommandOrControl+Shift+C".to_string(),
            deepl: DeepLConfig {
                api_key: String::new(),
                source_lang: "EN".to_string(),
                target_lang: "DE".to_string(),
                max_chars: 100,
            },
            anki: AnkiConfig {
                connect_url: "http://127.0.0.1:8765".to_string(),
                deck_name: "English::AutomatedVocabulary".to_string(),
                model_name: "AppleStyleFinalV3".to_string(),
            },
        }
    }
}

/// Computes and creates the application's secure application data system folder directory
pub fn get_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let mut path = app.path().app_data_dir().map_err(|e| e.to_string())?;
    create_dir_all(&path).map_err(|e| e.to_string())?;
    path.push("config.json");
    Ok(path)
}

/// Reads the local configuration state file or falls back cleanly onto system structural defaults
pub fn load_config(app: &AppHandle) -> AppConfig {
    if let Ok(path) = get_config_path(app) {
        if path.exists() {
            if let Ok(mut file) = File::open(path) {
                let mut contents = String::new();
                if file.read_to_string(&mut contents).is_ok() {
                    if let Ok(config) = serde_json::from_str::<AppConfig>(&contents) {
                        return config;
                    }
                }
            }
        }
    }
    AppConfig::default()
}

/// Serializes and writes data states safely back onto disk drives
pub fn save_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
    let path = get_config_path(app)?;
    let mut file = File::create(path).map_err(|e| e.to_string())?;
    let json_str = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    file.write_all(json_str.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}