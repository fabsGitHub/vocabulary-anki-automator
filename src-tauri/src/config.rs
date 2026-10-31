//! Automator settings: `config.json` in the app data dir (docs/architecture.md
//! § 8.1).
//!
//! Three decisions shape this module:
//!
//! - **Old files keep working.** Before the provider refactor the file had a
//!   provider-specific section (with an API key) where `translation` is now,
//!   and uppercase language codes. That section is read through a serde alias,
//!   the key is dropped like any unknown field, codes are normalized and every
//!   field added since gets its default, so upgrading keeps every setting and
//!   the next save writes the new shape.
//! - **Lenient load, strict save.** A file that a hand edit made invalid still
//!   loads, so the user's deck, note type and consent survive (a shortcut that
//!   does not parse is simply not registered). `apply_settings` refuses to save
//!   anything [`AppConfig::validate`] rejects, so the settings form can never
//!   produce such a file.
//! - **Atomic writes.** The file is written to a temporary sibling and renamed
//!   over the old one: a crash mid-write cannot leave a truncated file behind,
//!   which would otherwise load as the defaults and lose everything.
//!
//! [`load_config_from`] and [`save_config_to`] take a path so tests run against
//! a temp dir (Milestone G); [`config_path`] is the only part that needs an
//! `AppHandle`.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anki_bridge::FieldMapping;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use vocab_core::language::{normalize_lang, SUPPORTED_LANGS};

use crate::shortcuts::parse_shortcut;

/// File name inside the app data dir.
pub const CONFIG_FILE_NAME: &str = "config.json";

/// Upper bound for `translation.max_chars`: longer selections are almost
/// always accidental (a whole paragraph), and every provider call carries the
/// text in its URL.
pub const MAX_CHARS_LIMIT: usize = 500;

/// Translation services the pipeline can use (decision 2: MyMemory is the
/// keyless default and, so far, the only one).
pub const TRANSLATION_PROVIDERS: &[&str] = &["mymemory"];

/// Everything the Automator persists. Every field has a default, so a file
/// from any earlier version loads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// UI language of both windows (a vue-i18n locale).
    pub ui_lang: String,
    /// Shortcut of the default profile, which captures in `translation`'s
    /// language pair into `anki.deck_name`.
    pub hotkeys: String,
    /// Extra shortcuts, each with its own language pair and optional deck
    /// (Milestone H).
    pub hotkey_profiles: Vec<HotkeyProfile>,
    /// Files written before the provider refactor call this section `deepl`.
    #[serde(alias = "deepl")]
    pub translation: TranslationConfig,
    pub anki: AnkiConfig,
    /// Written only by `grant_consent` (Milestone E).
    pub consent: ConsentState,
}

/// One extra capture shortcut (Milestone H).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyProfile {
    /// Stable identifier, reported back as `PendingCapture.profile_id`.
    pub id: String,
    /// Shown in the settings and approval windows.
    pub label: String,
    pub shortcut: String,
    pub source_lang: String,
    pub target_lang: String,
    /// Deck for this profile's cards instead of `anki.deck_name`.
    pub deck_name: Option<String>,
}

/// The default profile's language pair and the pipeline's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslationConfig {
    pub source_lang: String,
    pub target_lang: String,
    /// Longest accepted capture, in characters (`1..=MAX_CHARS_LIMIT`).
    pub max_chars: usize,
    /// One of [`TRANSLATION_PROVIDERS`].
    pub provider: String,
    /// Sent to MyMemory, which then raises its free daily quota (Milestone C).
    pub contact_email: Option<String>,
}

/// Where approved cards go and how they are laid out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnkiConfig {
    pub connect_url: String,
    pub deck_name: String,
    /// The Anki note type ("model").
    pub model_name: String,
    /// Also add the target -> source note.
    pub create_reverse: bool,
    /// Which card value goes into which field of `model_name` (Milestones H
    /// and J5).
    pub field_mapping: FieldMapping,
}

/// When the user agreed to each install action; RFC 3339 UTC timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsentState {
    pub anki_install_granted_at: Option<String>,
    pub ankiconnect_provision_granted_at: Option<String>,
}

/// An install action that needs the user's consent (the `action` argument of
/// `grant_consent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentAction {
    /// Download and run the pinned Anki installer.
    AnkiInstall,
    /// Download the pinned AnkiConnect files into Anki's add-on folder.
    AnkiconnectProvision,
}

/// The shortcut, language pair and deck one capture uses: the default profile
/// or a [`HotkeyProfile`], resolved against the rest of the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureProfile {
    /// `None` for the default profile.
    pub id: Option<String>,
    pub shortcut: String,
    pub source_lang: String,
    pub target_lang: String,
    pub deck_name: String,
}

/// Why the settings could not be loaded, saved or accepted.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{} is not a valid settings file: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("could not encode the settings: {0}")]
    Encode(serde_json::Error),
    /// Every problem [`AppConfig::validate`] found, one sentence each.
    #[error("{}", .0.join("\n"))]
    Invalid(Vec<String>),
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui_lang: "en".to_string(),
            hotkeys: "CommandOrControl+Shift+C".to_string(),
            hotkey_profiles: Vec::new(),
            translation: TranslationConfig::default(),
            anki: AnkiConfig::default(),
            consent: ConsentState::default(),
        }
    }
}

impl Default for TranslationConfig {
    fn default() -> Self {
        Self {
            source_lang: "en".to_string(),
            target_lang: "de".to_string(),
            max_chars: 100,
            provider: TRANSLATION_PROVIDERS[0].to_string(),
            contact_email: None,
        }
    }
}

impl Default for AnkiConfig {
    fn default() -> Self {
        Self {
            connect_url: anki_bridge::DEFAULT_URL.to_string(),
            deck_name: "English::AutomatedVocabulary".to_string(),
            model_name: "AppleStyleFinalV3".to_string(),
            // The pre-refactor app always added both directions.
            create_reverse: true,
            field_mapping: FieldMapping::default(),
        }
    }
}

impl AppConfig {
    /// The profile behind [`hotkeys`](Self::hotkeys).
    pub fn default_profile(&self) -> CaptureProfile {
        CaptureProfile {
            id: None,
            shortcut: self.hotkeys.clone(),
            source_lang: self.translation.source_lang.clone(),
            target_lang: self.translation.target_lang.clone(),
            deck_name: self.anki.deck_name.clone(),
        }
    }

    /// `None` resolves to the default profile, `Some(id)` to that hotkey
    /// profile; an unknown id is `None`.
    pub fn profile(&self, id: Option<&str>) -> Option<CaptureProfile> {
        match id {
            None => Some(self.default_profile()),
            Some(id) => self
                .hotkey_profiles
                .iter()
                .find(|profile| profile.id == id)
                .map(|profile| self.resolve(profile)),
        }
    }

    /// The default profile followed by every hotkey profile, in config order.
    pub fn profiles(&self) -> Vec<CaptureProfile> {
        std::iter::once(self.default_profile())
            .chain(self.hotkey_profiles.iter().map(|p| self.resolve(p)))
            .collect()
    }

    fn resolve(&self, profile: &HotkeyProfile) -> CaptureProfile {
        CaptureProfile {
            id: Some(profile.id.clone()),
            shortcut: profile.shortcut.clone(),
            source_lang: profile.source_lang.clone(),
            target_lang: profile.target_lang.clone(),
            deck_name: profile
                .deck_name
                .clone()
                .unwrap_or_else(|| self.anki.deck_name.clone()),
        }
    }

    /// Canonical spelling of everything that has one: language codes through
    /// `normalize_lang`, trimmed text, blank optional values as `None`, and
    /// consent timestamps that do not parse as RFC 3339 dropped (a consent
    /// must say when it was given). Applied on every load and save.
    pub fn normalize(&mut self) {
        self.ui_lang = self.ui_lang.trim().to_string();
        self.hotkeys = self.hotkeys.trim().to_string();

        let translation = &mut self.translation;
        translation.source_lang = normalize_lang(&translation.source_lang);
        translation.target_lang = normalize_lang(&translation.target_lang);
        translation.provider = translation.provider.trim().to_lowercase();
        if translation.provider.is_empty() {
            translation.provider = TRANSLATION_PROVIDERS[0].to_string();
        }
        translation.contact_email = non_blank(translation.contact_email.take());

        for profile in &mut self.hotkey_profiles {
            profile.id = profile.id.trim().to_string();
            profile.label = profile.label.trim().to_string();
            profile.shortcut = profile.shortcut.trim().to_string();
            profile.source_lang = normalize_lang(&profile.source_lang);
            profile.target_lang = normalize_lang(&profile.target_lang);
            profile.deck_name = non_blank(profile.deck_name.take());
        }

        let anki = &mut self.anki;
        anki.connect_url = anki.connect_url.trim().to_string();
        anki.deck_name = anki.deck_name.trim().to_string();
        anki.model_name = anki.model_name.trim().to_string();

        for granted_at in [
            &mut self.consent.anki_install_granted_at,
            &mut self.consent.ankiconnect_provision_granted_at,
        ] {
            if let Some(value) = granted_at.as_deref() {
                if DateTime::parse_from_rfc3339(value.trim()).is_err() {
                    log::warn!("Ignoring a consent record with an unreadable timestamp: {value:?}");
                    *granted_at = None;
                }
            }
        }
    }

    /// Checks a [normalized](Self::normalize) config: every shortcut parses,
    /// needs a real modifier and is used once; profile ids are present and
    /// unique; languages are supported and differ within a pair; `max_chars`
    /// is in `1..=MAX_CHARS_LIMIT`; the provider is known; the AnkiConnect URL
    /// is http(s); deck, note type and every mapped field are named.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut problems = Vec::new();

        let mut shortcuts: Vec<(tauri_plugin_global_shortcut::Shortcut, String)> = Vec::new();
        let mut check_shortcut =
            |owner: String, text: &str, problems: &mut Vec<String>| match parse_shortcut(text) {
                Ok(shortcut) => match shortcuts.iter().find(|(seen, _)| *seen == shortcut) {
                    Some((_, first)) => problems.push(format!(
                        "{owner} uses the same shortcut as {}.",
                        lowercase_first(first)
                    )),
                    None => shortcuts.push((shortcut, owner)),
                },
                Err(message) => problems.push(format!("{owner}: {message}")),
            };

        check_shortcut(
            "The default hotkey".to_string(),
            &self.hotkeys,
            &mut problems,
        );
        check_pair(
            "Translation",
            &self.translation.source_lang,
            &self.translation.target_lang,
            &mut problems,
        );

        let mut ids: Vec<&str> = Vec::new();
        for (index, profile) in self.hotkey_profiles.iter().enumerate() {
            let owner = profile_name(profile, index);
            if profile.id.is_empty() {
                problems.push(format!("{owner} needs an id."));
            } else if ids.contains(&profile.id.as_str()) {
                problems.push(format!(
                    "{owner} reuses the id \"{}\"; ids must be unique.",
                    profile.id
                ));
            } else {
                ids.push(&profile.id);
            }
            check_shortcut(owner.clone(), &profile.shortcut, &mut problems);
            check_pair(
                &owner,
                &profile.source_lang,
                &profile.target_lang,
                &mut problems,
            );
        }

        let max_chars = self.translation.max_chars;
        if !(1..=MAX_CHARS_LIMIT).contains(&max_chars) {
            problems.push(format!(
                "The capture limit must be between 1 and {MAX_CHARS_LIMIT} characters, not {max_chars}."
            ));
        }
        if !TRANSLATION_PROVIDERS.contains(&self.translation.provider.as_str()) {
            problems.push(format!(
                "Unknown translation provider \"{}\" (available: {}).",
                self.translation.provider,
                TRANSLATION_PROVIDERS.join(", ")
            ));
        }

        let anki = &self.anki;
        let url_ok = reqwest::Url::parse(&anki.connect_url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host());
        if !url_ok {
            problems.push(format!(
                "The AnkiConnect address \"{}\" is not an http:// URL.",
                anki.connect_url
            ));
        }
        if anki.deck_name.is_empty() {
            problems.push("The Anki deck name must not be empty.".to_string());
        }
        if anki.model_name.is_empty() {
            problems.push("The Anki note type must not be empty.".to_string());
        }
        if anki.field_mapping.fields.is_empty() {
            problems.push("The field mapping must fill at least one Anki field.".to_string());
        }
        if anki
            .field_mapping
            .fields
            .iter()
            .any(|binding| binding.anki_field.trim().is_empty())
        {
            problems.push("Every mapped Anki field needs a name.".to_string());
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Invalid(problems))
        }
    }
}

impl ConsentState {
    /// The recorded timestamp for `action`, as stored.
    pub fn granted_at(&self, action: ConsentAction) -> Option<&str> {
        match action {
            ConsentAction::AnkiInstall => self.anki_install_granted_at.as_deref(),
            ConsentAction::AnkiconnectProvision => self.ankiconnect_provision_granted_at.as_deref(),
        }
    }

    /// Whether the user consented to `action`. Only a record that parses as
    /// an RFC 3339 timestamp counts.
    pub fn is_granted(&self, action: ConsentAction) -> bool {
        self.granted_at(action)
            .is_some_and(|at| DateTime::parse_from_rfc3339(at.trim()).is_ok())
    }

    /// Records consent to `action` at `at` (RFC 3339 UTC with milliseconds,
    /// docs/architecture.md § 2), replacing an earlier record.
    pub fn grant(&mut self, action: ConsentAction, at: DateTime<Utc>) {
        let stamp = Some(at.to_rfc3339_opts(SecondsFormat::Millis, true));
        match action {
            ConsentAction::AnkiInstall => self.anki_install_granted_at = stamp,
            ConsentAction::AnkiconnectProvision => self.ankiconnect_provision_granted_at = stamp,
        }
    }
}

impl ConsentAction {
    /// The action as the subject of a sentence ("Installing Anki ...").
    pub fn describe(&self) -> &'static str {
        match self {
            ConsentAction::AnkiInstall => "Installing Anki",
            ConsentAction::AnkiconnectProvision => "Installing AnkiConnect",
        }
    }
}

/// `<app data dir>/config.json`.
pub fn config_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(CONFIG_FILE_NAME))
        .map_err(|err| format!("could not locate the app data folder: {err}"))
}

/// Reads and [normalizes](AppConfig::normalize) the settings at `path`. A
/// missing file is the default config; an unreadable or unparsable one is an
/// error. Does not [validate](AppConfig::validate).
pub fn load_config_from(path: &Path) -> Result<AppConfig, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(AppConfig::default()),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };
    let mut config: AppConfig =
        serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    config.normalize();
    Ok(config)
}

/// Writes `config`, [normalized](AppConfig::normalize), to `path` atomically:
/// into a uniquely named sibling first, which is then renamed over `path`.
/// Creates the parent directory. Does not [validate](AppConfig::validate);
/// callers taking user input do that first.
pub fn save_config_to(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    let mut config = config.clone();
    config.normalize();
    let json = serde_json::to_string_pretty(&config).map_err(ConfigError::Encode)?;

    let write_error = |source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(write_error)?;
    }
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| CONFIG_FILE_NAME.to_string());
    let temp = path.with_file_name(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let written = write_new_file(&temp, format!("{json}\n").as_bytes())
        .and_then(|()| fs::rename(&temp, path));
    if let Err(source) = written {
        if let Err(cleanup) = fs::remove_file(&temp) {
            if cleanup.kind() != io::ErrorKind::NotFound {
                log::warn!("Could not remove {}: {cleanup}", temp.display());
            }
        }
        return Err(write_error(source));
    }
    Ok(())
}

/// Creates `path` (failing if anything already exists there, so a planted
/// symlink is never followed) and writes `bytes` durably.
fn write_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "Translation: ..." problems for a source/target pair.
fn check_pair(owner: &str, source: &str, target: &str, problems: &mut Vec<String>) {
    for (role, code) in [("source", source), ("target", target)] {
        if !SUPPORTED_LANGS.contains(&code) {
            problems.push(format!(
                "{owner}: the {role} language \"{code}\" is not supported (supported: {}).",
                SUPPORTED_LANGS.join(", ")
            ));
        }
    }
    if source == target {
        problems.push(format!(
            "{owner}: the source and target language are both \"{source}\"."
        ));
    }
}

fn profile_name(profile: &HotkeyProfile, index: usize) -> String {
    let name = if profile.label.is_empty() {
        &profile.id
    } else {
        &profile.label
    };
    if name.is_empty() {
        format!("Hotkey profile {}", index + 1)
    } else {
        format!("Hotkey profile \"{name}\"")
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use anki_bridge::{CardField, FieldBinding};
    use chrono::TimeZone;
    use serde_json::{json, Value};

    use super::*;

    const LEGACY_FIXTURE: &str = include_str!("../tests/fixtures/legacy_config.json");

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn chinese_profile() -> HotkeyProfile {
        HotkeyProfile {
            id: "zh-de".into(),
            label: "Chinese → German".into(),
            shortcut: "CommandOrControl+Shift+H".into(),
            source_lang: "zh-CN".into(),
            target_lang: "de".into(),
            deck_name: Some("Chinese::HSK".into()),
        }
    }

    fn keys(value: &Value) -> Vec<&str> {
        value
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn defaults_have_the_contract_shape() {
        let json = serde_json::to_value(AppConfig::default()).unwrap();
        assert_eq!(
            json,
            json!({
                "ui_lang": "en",
                "hotkeys": "CommandOrControl+Shift+C",
                "hotkey_profiles": [],
                "translation": {
                    "source_lang": "en",
                    "target_lang": "de",
                    "max_chars": 100,
                    "provider": "mymemory",
                    "contact_email": null
                },
                "anki": {
                    "connect_url": "http://127.0.0.1:8765",
                    "deck_name": "English::AutomatedVocabulary",
                    "model_name": "AppleStyleFinalV3",
                    "create_reverse": true,
                    "field_mapping": {
                        "fields": [
                            { "anki_field": "Front", "source": "front" },
                            { "anki_field": "Back", "source": "back" },
                            { "anki_field": "Context Sentence", "source": "example" },
                            { "anki_field": "Notes", "source": "notes" }
                        ],
                        "audio_field": "Front Audio"
                    }
                },
                "consent": {
                    "anki_install_granted_at": null,
                    "ankiconnect_provision_granted_at": null
                }
            })
        );
        assert!(AppConfig::default().validate().is_ok());
    }

    #[test]
    fn save_then_load_round_trips_in_a_temp_dir() {
        let dir = temp_dir();
        // A parent that does not exist yet, as on first start.
        let path = dir.path().join("app-data").join(CONFIG_FILE_NAME);
        let mut config = AppConfig {
            ui_lang: "de".into(),
            hotkey_profiles: vec![chinese_profile()],
            ..AppConfig::default()
        };
        config.translation.max_chars = 250;
        config.translation.contact_email = Some("me@example.com".into());
        config.anki.create_reverse = false;
        config.anki.field_mapping = FieldMapping::chinese_default();
        config.consent.grant(
            ConsentAction::AnkiInstall,
            Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap(),
        );

        save_config_to(&path, &config).unwrap();
        let loaded = load_config_from(&path).unwrap();

        assert_eq!(loaded, config);
        assert_eq!(
            loaded.consent.anki_install_granted_at.as_deref(),
            Some("2026-10-31T12:00:00.000Z")
        );
    }

    #[test]
    fn saving_replaces_the_file_and_leaves_no_temp_files() {
        let dir = temp_dir();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "{ \"ui_lang\": \"fr\" }").unwrap();

        let mut config = AppConfig {
            ui_lang: "it".into(),
            ..AppConfig::default()
        };
        save_config_to(&path, &config).unwrap();
        config.ui_lang = "es".into();
        save_config_to(&path, &config).unwrap();

        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(entries, [CONFIG_FILE_NAME]);
        assert_eq!(load_config_from(&path).unwrap().ui_lang, "es");
    }

    #[test]
    fn a_failed_save_keeps_the_old_file() {
        let dir = temp_dir();
        // The target is a directory, so the final rename must fail.
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "x").unwrap();

        let err = save_config_to(&path, &AppConfig::default()).unwrap_err();

        assert!(matches!(err, ConfigError::Write { .. }), "{err:?}");
        assert!(path.join("keep").exists());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, [CONFIG_FILE_NAME], "no temp file left behind");
    }

    #[test]
    fn a_missing_file_loads_as_the_defaults() {
        let dir = temp_dir();
        let loaded = load_config_from(&dir.path().join(CONFIG_FILE_NAME)).unwrap();
        assert_eq!(loaded, AppConfig::default());
    }

    #[test]
    fn an_unparsable_file_is_an_error_not_the_defaults() {
        let dir = temp_dir();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, "{ \"ui_lang\": ").unwrap();
        let err = load_config_from(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "{err:?}");
    }

    #[test]
    fn the_legacy_file_loads_with_every_setting_preserved() {
        let dir = temp_dir();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(&path, LEGACY_FIXTURE).unwrap();

        let loaded = load_config_from(&path).unwrap();

        let expected = AppConfig {
            ui_lang: "de".into(),
            hotkeys: "CommandOrControl+Alt+V".into(),
            hotkey_profiles: Vec::new(),
            translation: TranslationConfig {
                source_lang: "en".into(),
                target_lang: "fr".into(),
                max_chars: 80,
                provider: "mymemory".into(),
                contact_email: None,
            },
            anki: AnkiConfig {
                connect_url: "http://localhost:8765".into(),
                deck_name: "Vocabulary::Mined".into(),
                model_name: "Basic (and reversed card)".into(),
                create_reverse: true,
                field_mapping: FieldMapping::apple_style_default(),
            },
            consent: ConsentState::default(),
        };
        assert_eq!(loaded, expected);
        assert!(loaded.validate().is_ok());

        // The next save writes only the new shape: no legacy section, no key.
        save_config_to(&path, &loaded).unwrap();
        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            keys(&saved),
            [
                "anki",
                "consent",
                "hotkey_profiles",
                "hotkeys",
                "translation",
                "ui_lang"
            ]
        );
        assert_eq!(
            keys(&saved["translation"]),
            [
                "contact_email",
                "max_chars",
                "provider",
                "source_lang",
                "target_lang"
            ]
        );
        assert_eq!(load_config_from(&path).unwrap(), expected);
    }

    #[test]
    fn a_partial_file_fills_every_missing_field_with_its_default() {
        let dir = temp_dir();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(
            &path,
            r#"{ "anki": { "deck_name": "Mine" }, "hotkey_profiles": [{ "id": "x", "shortcut": "Alt+X" }] }"#,
        )
        .unwrap();

        let loaded = load_config_from(&path).unwrap();

        assert_eq!(loaded.anki.deck_name, "Mine");
        assert_eq!(loaded.anki.model_name, "AppleStyleFinalV3");
        assert!(loaded.anki.create_reverse);
        assert_eq!(loaded.translation, TranslationConfig::default());
        assert_eq!(loaded.hotkey_profiles[0].deck_name, None);
        assert_eq!(loaded.hotkey_profiles[0].source_lang, "");
    }

    #[test]
    fn normalize_canonicalizes_codes_text_and_blank_values() {
        let mut config = AppConfig::default();
        config.translation.source_lang = " ZH_cn ".into();
        config.translation.target_lang = "DE".into();
        config.translation.provider = " MyMemory ".into();
        config.translation.contact_email = Some("  me@example.com ".into());
        config.hotkey_profiles.push(HotkeyProfile {
            id: " fr ".into(),
            label: " French ".into(),
            shortcut: " Alt+F ".into(),
            source_lang: "FR".into(),
            target_lang: "En".into(),
            deck_name: Some("   ".into()),
        });
        config.anki.deck_name = " Deck ".into();
        config.consent.anki_install_granted_at = Some("yesterday".into());
        config.consent.ankiconnect_provision_granted_at = Some("2026-10-31T12:00:00.000Z".into());

        config.normalize();

        assert_eq!(config.translation.source_lang, "zh-CN");
        assert_eq!(config.translation.target_lang, "de");
        assert_eq!(config.translation.provider, "mymemory");
        assert_eq!(
            config.translation.contact_email.as_deref(),
            Some("me@example.com")
        );
        let profile = &config.hotkey_profiles[0];
        assert_eq!(
            (
                profile.id.as_str(),
                profile.label.as_str(),
                profile.shortcut.as_str()
            ),
            ("fr", "French", "Alt+F")
        );
        assert_eq!(
            (profile.source_lang.as_str(), profile.target_lang.as_str()),
            ("fr", "en")
        );
        assert_eq!(profile.deck_name, None);
        assert_eq!(config.anki.deck_name, "Deck");
        assert_eq!(config.consent.anki_install_granted_at, None);
        assert!(config
            .consent
            .is_granted(ConsentAction::AnkiconnectProvision));
    }

    #[test]
    fn an_empty_contact_email_becomes_none() {
        let mut config = AppConfig::default();
        config.translation.contact_email = Some("   ".into());
        config.normalize();
        assert_eq!(config.translation.contact_email, None);
    }

    #[test]
    fn validation_reports_each_problem() {
        type Case = (&'static str, fn(&mut AppConfig), &'static str);
        let cases: Vec<Case> = vec![
            ("unparsable default hotkey", |c| c.hotkeys = "Ctrl+Shift".into(), "The default hotkey: \"Ctrl+Shift\" is not a valid shortcut"),
            ("empty default hotkey", |c| c.hotkeys = String::new(), "The default hotkey: no shortcut is set"),
            ("hotkey without a modifier", |c| c.hotkeys = "C".into(), "needs Ctrl, Cmd or Alt"),
            ("shift is not enough", |c| c.hotkeys = "Shift+C".into(), "needs Ctrl, Cmd or Alt"),
            ("unsupported source", |c| c.translation.source_lang = "pt".into(), "Translation: the source language \"pt\" is not supported"),
            ("unsupported target", |c| c.translation.target_lang = "ja".into(), "Translation: the target language \"ja\" is not supported"),
            ("same pair", |c| c.translation.target_lang = "en".into(), "Translation: the source and target language are both \"en\""),
            ("zero limit", |c| c.translation.max_chars = 0, "between 1 and 500 characters, not 0"),
            ("limit too high", |c| c.translation.max_chars = 501, "between 1 and 500 characters, not 501"),
            ("unknown provider", |c| c.translation.provider = "argos".into(), "Unknown translation provider \"argos\""),
            ("bad url", |c| c.anki.connect_url = "127.0.0.1:8765".into(), "is not an http:// URL"),
            ("non-http url", |c| c.anki.connect_url = "file:///etc/passwd".into(), "is not an http:// URL"),
            ("empty deck", |c| c.anki.deck_name = String::new(), "deck name must not be empty"),
            ("empty model", |c| c.anki.model_name = String::new(), "note type must not be empty"),
            ("empty mapping", |c| c.anki.field_mapping.fields.clear(), "at least one Anki field"),
            (
                "unnamed field",
                |c| c.anki.field_mapping.fields.push(FieldBinding::new(" ", CardField::Notes)),
                "Every mapped Anki field needs a name",
            ),
            (
                "profile without id",
                |c| c.hotkey_profiles.push(HotkeyProfile { id: String::new(), ..chinese_profile() }),
                "Hotkey profile \"Chinese → German\" needs an id",
            ),
            (
                "duplicate profile id",
                |c| {
                    c.hotkey_profiles.push(chinese_profile());
                    c.hotkey_profiles.push(HotkeyProfile { shortcut: "Alt+J".into(), label: String::new(), ..chinese_profile() });
                },
                "Hotkey profile \"zh-de\" reuses the id \"zh-de\"",
            ),
            (
                "profile language",
                |c| c.hotkey_profiles.push(HotkeyProfile { target_lang: "zh-TW".into(), ..chinese_profile() }),
                "Hotkey profile \"Chinese → German\": the target language \"zh-TW\" is not supported",
            ),
            (
                "profile shortcut",
                |c| c.hotkey_profiles.push(HotkeyProfile { shortcut: "Ctrl+Nope".into(), ..chinese_profile() }),
                "Hotkey profile \"Chinese → German\": \"Ctrl+Nope\" is not a valid shortcut",
            ),
            (
                "profile reuses the default hotkey",
                |c| c.hotkey_profiles.push(HotkeyProfile { shortcut: "CommandOrControl+Shift+C".into(), ..chinese_profile() }),
                "Hotkey profile \"Chinese → German\" uses the same shortcut as the default hotkey.",
            ),
            (
                "same keys spelled differently",
                |c| c.hotkey_profiles.push(HotkeyProfile { shortcut: "shift + commandorcontrol + c".into(), ..chinese_profile() }),
                "uses the same shortcut as the default hotkey.",
            ),
            (
                "two profiles share a shortcut",
                |c| {
                    c.hotkey_profiles.push(chinese_profile());
                    c.hotkey_profiles.push(HotkeyProfile { id: "zh-en".into(), label: "Chinese → English".into(), ..chinese_profile() });
                },
                "Hotkey profile \"Chinese → English\" uses the same shortcut as hotkey profile \"Chinese → German\".",
            ),
        ];
        for (name, mutate, expected) in cases {
            let mut config = AppConfig::default();
            mutate(&mut config);
            match config.validate() {
                Err(ConfigError::Invalid(problems)) => assert!(
                    problems.iter().any(|problem| problem.contains(expected)),
                    "{name}: {problems:?} lacks {expected:?}"
                ),
                other => panic!("{name}: expected a validation error, got {other:?}"),
            }
        }
    }

    #[test]
    fn validation_accepts_profiles_function_keys_and_every_supported_pair() {
        let mut config = AppConfig {
            hotkeys: "F8".into(),
            hotkey_profiles: vec![chinese_profile()],
            ..AppConfig::default()
        };
        config.hotkey_profiles.push(HotkeyProfile {
            id: "it-en".into(),
            label: String::new(),
            shortcut: "Alt+I".into(),
            source_lang: "it".into(),
            target_lang: "en".into(),
            deck_name: None,
        });
        config.translation.max_chars = MAX_CHARS_LIMIT;
        config.anki.connect_url = "https://anki.example.com:8765/".into();
        assert!(config.validate().is_ok(), "{:?}", config.validate());
    }

    #[test]
    fn all_problems_are_reported_together() {
        let mut config = AppConfig::default();
        config.translation.max_chars = 0;
        config.anki.deck_name = String::new();
        let err = config.validate().unwrap_err();
        let ConfigError::Invalid(problems) = &err else {
            panic!("{err:?}");
        };
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(err.to_string(), problems.join("\n"));
    }

    #[test]
    fn profiles_resolve_their_pair_and_deck() {
        let mut config = AppConfig::default();
        config.hotkey_profiles.push(chinese_profile());
        config.hotkey_profiles.push(HotkeyProfile {
            id: "fr".into(),
            shortcut: "Alt+F".into(),
            source_lang: "fr".into(),
            target_lang: "de".into(),
            ..HotkeyProfile::default()
        });

        let default = config.profile(None).unwrap();
        assert_eq!(default.id, None);
        assert_eq!(
            (default.source_lang.as_str(), default.target_lang.as_str()),
            ("en", "de")
        );
        assert_eq!(default.deck_name, "English::AutomatedVocabulary");

        let chinese = config.profile(Some("zh-de")).unwrap();
        assert_eq!(chinese.source_lang, "zh-CN");
        assert_eq!(chinese.deck_name, "Chinese::HSK");

        let french = config.profile(Some("fr")).unwrap();
        assert_eq!(
            french.deck_name, "English::AutomatedVocabulary",
            "no override"
        );

        assert_eq!(config.profile(Some("missing")), None);
        let ids: Vec<_> = config.profiles().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, [None, Some("zh-de".into()), Some("fr".into())]);
    }

    #[test]
    fn consent_counts_only_valid_timestamps() {
        let mut consent = ConsentState::default();
        assert!(!consent.is_granted(ConsentAction::AnkiInstall));
        consent.anki_install_granted_at = Some("not a time".into());
        assert!(!consent.is_granted(ConsentAction::AnkiInstall));

        consent.grant(
            ConsentAction::AnkiInstall,
            Utc.with_ymd_and_hms(2026, 10, 31, 9, 30, 0).unwrap(),
        );
        assert!(consent.is_granted(ConsentAction::AnkiInstall));
        assert!(!consent.is_granted(ConsentAction::AnkiconnectProvision));
        assert_eq!(
            consent.granted_at(ConsentAction::AnkiInstall),
            Some("2026-10-31T09:30:00.000Z")
        );
        assert_eq!(
            serde_json::to_value(ConsentAction::AnkiconnectProvision).unwrap(),
            "ankiconnect_provision"
        );
    }
}
