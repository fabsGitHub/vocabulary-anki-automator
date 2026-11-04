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
//!   the next save writes the new shape. The one exception is the global
//!   field mapping: every file saved before it could be left unset holds the
//!   old built-in default, which the user never chose, so that value loads as
//!   unset and is matched by field name ([`AppConfig::normalize`]).
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
    /// Shortcut of the default profile. It detects the source language among
    /// `learning.languages` (falling back to `translation.source_lang`), or
    /// captures in `translation`'s pair when none are listed; the deck is
    /// [`AppConfig::filing`]'s choice.
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
    /// Languages the default hotkey detects among (empty = detection off,
    /// today's behaviour), each with its own deck and note type.
    pub learning: LearningConfig,
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
    /// Deck for this profile's cards instead of the one
    /// [`AppConfig::filing`] would choose for the language.
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
    /// and J5). `None`, the default: the note type's fields are matched by
    /// name whenever a card is pushed or checked, exactly as for a
    /// [`LearnedLanguage`] without a mapping. A missing key and a saved
    /// `FieldMapping::apple_style_default()` both load as `None`
    /// ([`AppConfig::normalize`]).
    pub field_mapping: Option<FieldMapping>,
}

/// The languages the user learns (Milestone O).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LearningConfig {
    /// In the order the approval window lists them.
    pub languages: Vec<LearnedLanguage>,
}

/// One language the user learns, and where its cards go
/// ([`AppConfig::filing`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnedLanguage {
    /// Canonical code; never `translation.target_lang`.
    pub lang: String,
    pub deck_name: String,
    /// The Anki note type ("model").
    pub model_name: String,
    /// `None`: the note type's fields are matched by name whenever a card is
    /// pushed or checked (`FieldMapping::auto_match`); the result is never
    /// written back, so a changed note type is matched afresh.
    pub field_mapping: Option<FieldMapping>,
    /// Also add the target -> source note.
    pub create_reverse: bool,
}

/// Which mapping a [`Filing`] pushes with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingChoice {
    /// A mapping from the config, used as saved.
    Saved(FieldMapping),
    /// Matched from the note type's field names at push and check time.
    Auto,
}

/// Where a card goes: deck, note type, field mapping and reverse note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filing {
    pub deck: String,
    pub model: String,
    pub mapping: MappingChoice,
    pub create_reverse: bool,
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

/// The shortcut and language pair one capture uses: the default profile or a
/// [`HotkeyProfile`], resolved against the rest of the config. Where its
/// cards go is [`AppConfig::filing`]'s decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureProfile {
    /// `None` for the default profile.
    pub id: Option<String>,
    pub shortcut: String,
    pub source_lang: String,
    pub target_lang: String,
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
            learning: LearningConfig::default(),
        }
    }
}

impl Default for LearnedLanguage {
    fn default() -> Self {
        Self {
            lang: String::new(),
            deck_name: String::new(),
            model_name: String::new(),
            field_mapping: None,
            create_reverse: true,
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
            field_mapping: None,
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
        }
    }

    /// Where a card captured with `profile_id` (`None`: the default hotkey)
    /// in `source_lang` goes. The learned language of `source_lang`, if any,
    /// brings its note type, mapping and reverse setting; a hotkey profile's
    /// own deck still wins for the deck. Without learned languages this is
    /// the global `anki` section, with the profile's deck. A mapping that is
    /// `None`, the entry's or the global one, is [`MappingChoice::Auto`].
    /// Blank names (only a hand edit can save them) fall back to the global
    /// ones, and an unknown `profile_id` (a profile deleted since the capture)
    /// counts as none.
    pub fn filing(&self, profile_id: Option<&str>, source_lang: &str) -> Filing {
        let lang = normalize_lang(source_lang);
        let entry = self
            .learning
            .languages
            .iter()
            .find(|entry| entry.lang == lang);
        let profile_deck = profile_id
            .and_then(|id| self.hotkey_profiles.iter().find(|profile| profile.id == id))
            .and_then(|profile| profile.deck_name.clone());
        let named = |name: &String| (!name.trim().is_empty()).then(|| name.clone());
        Filing {
            deck: profile_deck
                .or_else(|| entry.and_then(|entry| named(&entry.deck_name)))
                .unwrap_or_else(|| self.anki.deck_name.clone()),
            model: entry
                .and_then(|entry| named(&entry.model_name))
                .unwrap_or_else(|| self.anki.model_name.clone()),
            mapping: entry
                .map_or(&self.anki.field_mapping, |entry| &entry.field_mapping)
                .clone()
                .map_or(MappingChoice::Auto, MappingChoice::Saved),
            create_reverse: entry.map_or(self.anki.create_reverse, |entry| entry.create_reverse),
        }
    }

    /// The languages a capture with `profile` detects among, in settings
    /// order: the learned ones for the default hotkey, none for a hotkey
    /// profile (it keeps its fixed pair). Empty means no detection.
    pub fn detection_languages(&self, profile: &CaptureProfile) -> Vec<String> {
        if profile.id.is_some() {
            return Vec::new();
        }
        self.learning
            .languages
            .iter()
            .map(|entry| entry.lang.clone())
            .collect()
    }

    /// Canonical spelling of everything that has one: language codes through
    /// `normalize_lang`, trimmed text, blank optional values as `None`, a
    /// global field mapping equal to `FieldMapping::apple_style_default()` as
    /// `None`, and consent timestamps that do not parse as RFC 3339 dropped (a
    /// consent must say when it was given). Applied on every load and save.
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
        // Until the global mapping could be unset, every saved config held
        // this default whether or not the user ever looked at it, so it is no
        // choice: it is matched by field name like a missing one. On the
        // AppleStyleFinalV3 note type it was made for, the match is this same
        // mapping (`auto_match`'s tests pin that), so nothing changes there.
        if anki.field_mapping.as_ref() == Some(&FieldMapping::apple_style_default()) {
            anki.field_mapping = None;
        }

        // A learned language's own mapping stays as saved: entries had `None`
        // from the start, so a saved one is always a choice.
        for entry in &mut self.learning.languages {
            entry.lang = normalize_lang(&entry.lang);
            entry.deck_name = entry.deck_name.trim().to_string();
            entry.model_name = entry.model_name.trim().to_string();
        }

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
    /// is http(s); deck and note type are named, and a saved global mapping
    /// fills a field and names every one. Each learned language is supported,
    /// listed once and not the one cards are translated into, and names its
    /// deck and note type; a saved mapping of its own follows the global
    /// mapping's rules. A mapping matched by field name (`None`) needs no
    /// check. Fields a note type lacks are a warning in the approval window,
    /// never a validation problem.
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
        if let Some(mapping) = &anki.field_mapping {
            check_mapping(
                mapping,
                "The field mapping must fill at least one Anki field.",
                "Every mapped Anki field needs a name.",
                &mut problems,
            );
        }

        let mut learned: Vec<&str> = Vec::new();
        for entry in &self.learning.languages {
            let lang = entry.lang.as_str();
            if !SUPPORTED_LANGS.contains(&lang) {
                problems.push(format!(
                    "Learned language \"{lang}\" is not supported (supported: {}).",
                    SUPPORTED_LANGS.join(", ")
                ));
            } else if learned.contains(&lang) {
                problems.push(format!("Learned language \"{lang}\" is listed twice."));
            } else if lang == self.translation.target_lang {
                problems.push(format!(
                    "Learned language \"{lang}\" is the language cards are translated into."
                ));
            }
            learned.push(lang);
            if entry.deck_name.is_empty() {
                problems.push(format!("Learned language \"{lang}\" needs a deck."));
            }
            if entry.model_name.is_empty() {
                problems.push(format!("Learned language \"{lang}\" needs a note type."));
            }
            if let Some(mapping) = &entry.field_mapping {
                check_mapping(
                    mapping,
                    &format!(
                        "The field mapping of learned language \"{lang}\" must fill at least one \
                         Anki field."
                    ),
                    &format!(
                        "Every mapped Anki field of learned language \"{lang}\" needs a name."
                    ),
                    &mut problems,
                );
            }
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

/// The rules every saved mapping follows: at least one binding, and every
/// binding names its Anki field.
fn check_mapping(mapping: &FieldMapping, empty: &str, unnamed: &str, problems: &mut Vec<String>) {
    if mapping.fields.is_empty() {
        problems.push(empty.to_string());
    }
    if mapping
        .fields
        .iter()
        .any(|binding| binding.anki_field.trim().is_empty())
    {
        problems.push(unnamed.to_string());
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

    /// French into its own deck, matched by field name.
    fn french() -> LearnedLanguage {
        LearnedLanguage {
            lang: "fr".into(),
            deck_name: "french".into(),
            model_name: "AppleStyleFinalV3".into(),
            field_mapping: None,
            create_reverse: true,
        }
    }

    /// Chinese with its own note type and mapping, without reverse notes.
    fn chinese() -> LearnedLanguage {
        LearnedLanguage {
            lang: "zh-CN".into(),
            deck_name: "chinese".into(),
            model_name: "Chinese (HSK)".into(),
            field_mapping: Some(FieldMapping::chinese_default()),
            create_reverse: false,
        }
    }

    /// The global `anki` section, its mapping never customized, as a filing
    /// into `deck`.
    fn global_filing(config: &AppConfig, deck: &str) -> Filing {
        assert_eq!(config.anki.field_mapping, None);
        Filing {
            deck: deck.into(),
            model: config.anki.model_name.clone(),
            mapping: MappingChoice::Auto,
            create_reverse: config.anki.create_reverse,
        }
    }

    /// `FieldMapping::apple_style_default()` as the builds before the global
    /// mapping could be unset wrote it into every saved config.
    fn old_default_mapping_json() -> Value {
        json!({
            "fields": [
                { "anki_field": "Front", "source": "front" },
                { "anki_field": "Back", "source": "back" },
                { "anki_field": "Context Sentence", "source": "example" },
                { "anki_field": "Notes", "source": "notes" }
            ],
            "audio_field": "Front Audio"
        })
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
                    "field_mapping": null
                },
                "consent": {
                    "anki_install_granted_at": null,
                    "ankiconnect_provision_granted_at": null
                },
                "learning": { "languages": [] }
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
        config.anki.field_mapping = Some(FieldMapping::chinese_default());
        config.consent.grant(
            ConsentAction::AnkiInstall,
            Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap(),
        );
        config.learning.languages = vec![french(), chinese()];

        save_config_to(&path, &config).unwrap();
        let loaded = load_config_from(&path).unwrap();

        assert_eq!(loaded, config);
        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            saved["anki"]["field_mapping"],
            serde_json::to_value(FieldMapping::chinese_default()).unwrap()
        );
        assert_eq!(
            saved["learning"]["languages"][0]["field_mapping"],
            Value::Null
        );
        assert_eq!(
            saved["learning"]["languages"][1]["field_mapping"],
            serde_json::to_value(FieldMapping::chinese_default()).unwrap()
        );
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
                field_mapping: None,
            },
            consent: ConsentState::default(),
            learning: LearningConfig::default(),
        };
        assert_eq!(loaded, expected);
        assert!(loaded.learning.languages.is_empty());
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
                "learning",
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
        config.learning.languages = vec![
            LearnedLanguage {
                lang: " EN ".into(),
                deck_name: " english ".into(),
                model_name: " Basic ".into(),
                ..LearnedLanguage::default()
            },
            LearnedLanguage {
                lang: "zh".into(),
                ..chinese()
            },
        ];
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
        let english = &config.learning.languages[0];
        assert_eq!(
            (
                english.lang.as_str(),
                english.deck_name.as_str(),
                english.model_name.as_str()
            ),
            ("en", "english", "Basic")
        );
        assert_eq!(config.learning.languages[1].lang, "zh-CN");
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
            (
                "empty mapping",
                |c| c.anki.field_mapping = Some(FieldMapping { fields: Vec::new(), audio_field: None }),
                "The field mapping must fill at least one Anki field.",
            ),
            (
                "unnamed field",
                |c| {
                    let mut mapping = FieldMapping::chinese_default();
                    mapping.fields.push(FieldBinding::new(" ", CardField::Notes));
                    c.anki.field_mapping = Some(mapping);
                },
                "Every mapped Anki field needs a name.",
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
            (
                "unsupported learned language",
                |c| c.learning.languages.push(LearnedLanguage { lang: "pt".into(), ..french() }),
                "Learned language \"pt\" is not supported (supported: en, de, fr, es, it, zh-CN).",
            ),
            (
                "learned language listed twice",
                |c| c.learning.languages.extend([french(), LearnedLanguage { deck_name: "other".into(), ..french() }]),
                "Learned language \"fr\" is listed twice.",
            ),
            (
                "learned native language",
                |c| c.learning.languages.push(LearnedLanguage { lang: "de".into(), ..french() }),
                "Learned language \"de\" is the language cards are translated into.",
            ),
            (
                "learned language without a deck",
                |c| c.learning.languages.push(LearnedLanguage { deck_name: String::new(), ..french() }),
                "Learned language \"fr\" needs a deck.",
            ),
            (
                "learned language without a note type",
                |c| c.learning.languages.push(LearnedLanguage { model_name: String::new(), ..french() }),
                "Learned language \"fr\" needs a note type.",
            ),
            (
                "empty learned mapping",
                |c| c.learning.languages.push(LearnedLanguage { field_mapping: Some(FieldMapping { fields: Vec::new(), audio_field: None }), ..french() }),
                "The field mapping of learned language \"fr\" must fill at least one Anki field.",
            ),
            (
                "unnamed learned field",
                |c| {
                    let mut mapping = FieldMapping::chinese_default();
                    mapping.fields.push(FieldBinding::new("", CardField::Notes));
                    c.learning.languages.push(LearnedLanguage { field_mapping: Some(mapping), ..chinese() });
                },
                "Every mapped Anki field of learned language \"zh-CN\" needs a name.",
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
    fn validation_accepts_learned_languages_with_and_without_a_mapping() {
        let mut config = AppConfig::default();
        config.learning.languages = vec![
            LearnedLanguage {
                lang: "en".into(),
                deck_name: "english".into(),
                ..french()
            },
            french(),
            chinese(),
        ];
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
        assert_eq!(
            config.filing(None, &default.source_lang).deck,
            "English::AutomatedVocabulary"
        );

        let chinese = config.profile(Some("zh-de")).unwrap();
        assert_eq!(chinese.source_lang, "zh-CN");
        assert_eq!(
            config
                .filing(chinese.id.as_deref(), &chinese.source_lang)
                .deck,
            "Chinese::HSK"
        );

        let french = config.profile(Some("fr")).unwrap();
        assert_eq!(
            config
                .filing(french.id.as_deref(), &french.source_lang)
                .deck,
            "English::AutomatedVocabulary",
            "no override"
        );

        assert_eq!(config.profile(Some("missing")), None);
        let ids: Vec<_> = config.profiles().into_iter().map(|p| p.id).collect();
        assert_eq!(ids, [None, Some("zh-de".into()), Some("fr".into())]);
    }

    /// The migration guarantee: a file from before learned languages files
    /// every card into the deck, note type and reverse setting it always did,
    /// whatever its language or profile. Its mapping, never chosen, is
    /// matched by field name, which on the classic note type gives the old
    /// default back (`mapper.rs`'s auto-match tests, `ipc_anki.rs`).
    #[test]
    fn without_learned_languages_cards_are_filed_as_before() {
        let dir = temp_dir();
        let path = dir.path().join(CONFIG_FILE_NAME);
        fs::write(
            &path,
            r#"{
                "hotkey_profiles": [{ "id": "zh-de", "shortcut": "Alt+H", "source_lang": "zh-CN",
                                      "target_lang": "de", "deck_name": "Chinese::HSK" }],
                "anki": { "deck_name": "Mine", "model_name": "Basic", "create_reverse": false }
            }"#,
        )
        .unwrap();
        let config = load_config_from(&path).unwrap();
        assert!(config.learning.languages.is_empty());

        assert_eq!(config.filing(None, "en"), global_filing(&config, "Mine"));
        assert_eq!(config.filing(None, "zh-CN"), global_filing(&config, "Mine"));
        assert_eq!(
            config.filing(Some("zh-de"), "zh-CN"),
            global_filing(&config, "Chinese::HSK")
        );
        assert_eq!(
            config.filing(Some("deleted"), "zh-CN"),
            global_filing(&config, "Mine")
        );
    }

    /// The user's file (no `field_mapping` key) and the files older builds
    /// wrote (the built-in default, or `null`): none of them is a choice, so
    /// all match the note type by field name, and the next save writes
    /// `null`.
    #[test]
    fn a_global_mapping_never_chosen_is_matched_by_field_name() {
        assert_eq!(
            old_default_mapping_json(),
            serde_json::to_value(FieldMapping::apple_style_default()).unwrap()
        );
        let files = [
            (
                "no key",
                json!({ "anki": { "connect_url": "http://127.0.0.1:8765", "deck_name": "english",
                                  "model_name": "AppleStyleFinalV3" } }),
            ),
            (
                "null",
                json!({ "anki": { "deck_name": "english", "field_mapping": null } }),
            ),
            (
                "the old default",
                json!({ "anki": { "deck_name": "english",
                                  "field_mapping": old_default_mapping_json() } }),
            ),
        ];
        for (name, file) in files {
            let dir = temp_dir();
            let path = dir.path().join(CONFIG_FILE_NAME);
            fs::write(&path, file.to_string()).unwrap();

            let config = load_config_from(&path).unwrap();

            assert_eq!(config.anki.field_mapping, None, "{name}");
            assert_eq!(
                config.filing(None, "en"),
                global_filing(&config, "english"),
                "{name}"
            );
            assert!(config.validate().is_ok(), "{name}: {:?}", config.validate());
            save_config_to(&path, &config).unwrap();
            let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(saved["anki"]["field_mapping"], Value::Null, "{name}");
        }
    }

    /// Only the exact old default counts as unset: any change to it, or any
    /// other mapping, is the user's and is pushed as saved.
    #[test]
    fn a_customized_global_mapping_is_used_as_saved() {
        let mut no_audio = FieldMapping::apple_style_default();
        no_audio.audio_field = None;
        let mut reordered = FieldMapping::apple_style_default();
        reordered.fields.swap(2, 3);
        for mapping in [no_audio, reordered, FieldMapping::chinese_default()] {
            let mut config = AppConfig::default();
            config.anki.field_mapping = Some(mapping.clone());
            config.normalize();

            assert_eq!(config.anki.field_mapping.as_ref(), Some(&mapping));
            assert_eq!(
                config.filing(None, "en").mapping,
                MappingChoice::Saved(mapping)
            );
        }

        let mut config = AppConfig::default();
        config.anki.field_mapping = Some(FieldMapping::apple_style_default());
        config.normalize();
        assert_eq!(config.anki.field_mapping, None, "the old default itself");
    }

    #[test]
    fn a_learned_language_keeps_even_the_old_default_as_its_choice() {
        let mut config = AppConfig::default();
        config.learning.languages = vec![LearnedLanguage {
            field_mapping: Some(FieldMapping::apple_style_default()),
            ..french()
        }];
        config.normalize();

        assert_eq!(
            config.filing(None, "fr").mapping,
            MappingChoice::Saved(FieldMapping::apple_style_default())
        );
    }

    #[test]
    fn a_learned_language_brings_its_deck_note_type_mapping_and_reverse_setting() {
        let mut config = AppConfig::default();
        config.hotkey_profiles.push(chinese_profile());
        config.hotkey_profiles.push(HotkeyProfile {
            id: "fr-de".into(),
            shortcut: "Alt+F".into(),
            source_lang: "fr".into(),
            target_lang: "de".into(),
            ..HotkeyProfile::default()
        });
        config.learning.languages = vec![french(), chinese()];

        let auto_french = Filing {
            deck: "french".into(),
            model: "AppleStyleFinalV3".into(),
            mapping: MappingChoice::Auto,
            create_reverse: true,
        };
        assert_eq!(config.filing(None, "fr"), auto_french);
        assert_eq!(
            config.filing(None, "FR"),
            auto_french,
            "codes are normalized"
        );
        assert_eq!(
            config.filing(None, "zh"),
            Filing {
                deck: "chinese".into(),
                model: "Chinese (HSK)".into(),
                mapping: MappingChoice::Saved(FieldMapping::chinese_default()),
                create_reverse: false,
            }
        );
        // No entry for English: the global section, as before.
        assert_eq!(
            config.filing(None, "en"),
            global_filing(&config, "English::AutomatedVocabulary")
        );

        // A profile's own deck wins for the deck only.
        assert_eq!(
            config.filing(Some("zh-de"), "zh-CN"),
            Filing {
                deck: "Chinese::HSK".into(),
                ..config.filing(None, "zh-CN")
            }
        );
        // A profile without a deck uses the entry's.
        assert_eq!(config.filing(Some("fr-de"), "fr"), auto_french);
        // A deleted profile counts as none.
        assert_eq!(config.filing(Some("deleted"), "zh-CN").deck, "chinese");
    }

    #[test]
    fn blank_names_of_a_hand_edited_entry_fall_back_to_the_global_ones() {
        let mut config = AppConfig::default();
        config.learning.languages = vec![LearnedLanguage {
            deck_name: " ".into(),
            model_name: String::new(),
            ..french()
        }];

        let filing = config.filing(None, "fr");

        assert_eq!(filing.deck, "English::AutomatedVocabulary");
        assert_eq!(filing.model, "AppleStyleFinalV3");
        assert_eq!(filing.mapping, MappingChoice::Auto);
    }

    #[test]
    fn only_the_default_hotkey_detects_among_the_learned_languages() {
        let mut config = AppConfig::default();
        config.hotkey_profiles.push(chinese_profile());
        assert!(config
            .detection_languages(&config.default_profile())
            .is_empty());

        config.learning.languages = vec![chinese(), french()];

        assert_eq!(
            config.detection_languages(&config.default_profile()),
            ["zh-CN", "fr"]
        );
        let profile = config.profile(Some("zh-de")).unwrap();
        assert!(config.detection_languages(&profile).is_empty());
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
