//! Learn's settings (`LearnSettings` in apps/learn/src/platform/types.ts):
//! the language new curriculum words are explained in, and the AnkiConnect
//! endpoint, deck, note type and field mapping that export and import use.
//!
//! They live in the shared database (`app_state` key [`SETTINGS_KEY`],
//! docs/architecture.md § 6.4) rather than in a file of Learn's own: the
//! store is the one place Learn keeps state on every platform it targets,
//! mobile included, and the web build keeps the same JSON in IndexedDB.
//!
//! As in Automator's config: **lenient load, strict save.** A stored value
//! that does not parse any more (a hand edit, a shape from another build)
//! loads as the defaults instead of stopping the app, and is left in place
//! until the next save; missing fields get their defaults. `save_settings`
//! normalizes its input and refuses whatever [`LearnSettings::validate`]
//! rejects, so the settings form cannot store something export or import
//! would trip over.

use anki_bridge::{CardField, FieldMapping};
use serde::{Deserialize, Serialize};
use tauri::Url;
use vocab_core::store::{Store, StoreResult};

use crate::error::Error;

/// `app_state` key holding the settings as JSON.
pub const SETTINGS_KEY: &str = "learn.settings";

/// The language of the meanings new curriculum words get
/// (`introduce_new_item`'s `target_lang`). German meanings are not bundled
/// yet (§ 6.4), so a German learner's new words fall back to English, and the
/// card's own `target_lang` says which one it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeaningLang {
    De,
    /// The default, because it is the language every bundled meaning has.
    #[default]
    En,
}

impl MeaningLang {
    /// The canonical language code.
    pub fn as_str(self) -> &'static str {
        match self {
            MeaningLang::De => "de",
            MeaningLang::En => "en",
        }
    }
}

/// Everything Learn persists besides learning progress.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnSettings {
    /// The language of new curriculum words' meanings.
    pub meaning_lang: MeaningLang,
    pub anki: AnkiSettings,
}

/// Where Anki export and import go, and how cards map onto notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnkiSettings {
    /// The AnkiConnect endpoint, normally on this machine.
    pub connect_url: String,
    /// The deck export writes to; import asks for its deck each time.
    pub deck_name: String,
    /// The Anki note type ("model").
    pub model_name: String,
    /// Which card value goes into which field of `model_name`; import reads
    /// notes back through the same mapping.
    pub field_mapping: FieldMapping,
    /// Also add the meaning -> hanzi note on export.
    pub create_reverse: bool,
}

impl Default for AnkiSettings {
    /// A Chinese note type in its own deck. Reverse notes are off because a
    /// Chinese note type normally has its own recognition and production card
    /// templates, which make two cards out of one note.
    fn default() -> Self {
        Self {
            connect_url: anki_bridge::DEFAULT_URL.to_string(),
            deck_name: "Chinese::HSK".to_string(),
            model_name: "Chinese (HSK)".to_string(),
            field_mapping: FieldMapping::chinese_default(),
            create_reverse: false,
        }
    }
}

impl LearnSettings {
    /// Trims every name and the URL, and turns a blank audio field into none.
    pub fn normalize(&mut self) {
        let anki = &mut self.anki;
        anki.connect_url = anki.connect_url.trim().to_string();
        anki.deck_name = anki.deck_name.trim().to_string();
        anki.model_name = anki.model_name.trim().to_string();
        for binding in &mut anki.field_mapping.fields {
            binding.anki_field = binding.anki_field.trim().to_string();
        }
        anki.field_mapping.audio_field = anki
            .field_mapping
            .audio_field
            .take()
            .map(|field| field.trim().to_string())
            .filter(|field| !field.is_empty());
    }

    /// Checks [normalized](Self::normalize) settings: the AnkiConnect URL is
    /// http(s), deck and note type are named, and the field mapping names
    /// every field and has a front and a back, which import needs to read a
    /// note back into a card.
    pub fn validate(&self) -> Result<(), Error> {
        let anki = &self.anki;
        let mut problems = Vec::new();
        let url_ok = Url::parse(&anki.connect_url)
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
        let fields = &anki.field_mapping.fields;
        if fields.iter().any(|binding| binding.anki_field.is_empty()) {
            problems.push("Every mapped Anki field needs a name.".to_string());
        }
        for (source, what) in [(CardField::Front, "word"), (CardField::Back, "meaning")] {
            if !fields.iter().any(|binding| binding.source == source) {
                problems.push(format!(
                    "The field mapping needs an Anki field for the {what}."
                ));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(Error::InvalidSettings(problems))
        }
    }
}

/// The stored settings, or the defaults when there are none or they do not
/// parse (logged; the stored value is kept until the next save).
pub fn load(store: &Store) -> StoreResult<LearnSettings> {
    let Some(text) = store.get_state(SETTINGS_KEY)? else {
        return Ok(LearnSettings::default());
    };
    Ok(match serde_json::from_str::<LearnSettings>(&text) {
        Ok(mut settings) => {
            settings.normalize();
            settings
        }
        Err(err) => {
            log::error!("Using the default settings: the stored ones do not parse ({err}).");
            LearnSettings::default()
        }
    })
}

/// Stores `settings` as they are; validating them is the caller's job.
pub fn save(store: &Store, settings: &LearnSettings) -> StoreResult<()> {
    store.set_state(SETTINGS_KEY, &serde_json::to_string(settings)?)
}

#[cfg(test)]
mod tests {
    use anki_bridge::FieldBinding;
    use serde_json::json;

    use super::*;

    #[test]
    fn the_defaults_have_the_contract_shape() {
        assert_eq!(
            serde_json::to_value(LearnSettings::default()).unwrap(),
            json!({
                "meaning_lang": "en",
                "anki": {
                    "connect_url": "http://127.0.0.1:8765",
                    "deck_name": "Chinese::HSK",
                    "model_name": "Chinese (HSK)",
                    "field_mapping": {
                        "fields": [
                            { "anki_field": "Hanzi", "source": "front" },
                            { "anki_field": "Pinyin", "source": "pinyin" },
                            { "anki_field": "Meaning", "source": "back" },
                            { "anki_field": "Sentence", "source": "example" }
                        ],
                        "audio_field": "Audio"
                    },
                    "create_reverse": false
                }
            })
        );
        assert!(LearnSettings::default().validate().is_ok());
    }

    #[test]
    fn missing_fields_get_their_defaults() {
        let settings: LearnSettings = serde_json::from_value(json!({
            "meaning_lang": "de",
            "anki": { "deck_name": "Mine" }
        }))
        .unwrap();
        assert_eq!(settings.meaning_lang, MeaningLang::De);
        assert_eq!(settings.anki.deck_name, "Mine");
        assert_eq!(settings.anki.model_name, "Chinese (HSK)");
    }

    #[test]
    fn normalize_trims_names_and_drops_a_blank_audio_field() {
        let mut settings = LearnSettings::default();
        settings.anki.connect_url = " http://localhost:8765 ".into();
        settings.anki.deck_name = " Deck ".into();
        settings.anki.model_name = "\tModel\n".into();
        settings.anki.field_mapping = FieldMapping {
            fields: vec![FieldBinding::new(" Front ", CardField::Front)],
            audio_field: Some("  ".into()),
        };

        settings.normalize();

        assert_eq!(settings.anki.connect_url, "http://localhost:8765");
        assert_eq!(settings.anki.deck_name, "Deck");
        assert_eq!(settings.anki.model_name, "Model");
        assert_eq!(settings.anki.field_mapping.fields[0].anki_field, "Front");
        assert_eq!(settings.anki.field_mapping.audio_field, None);
    }

    #[test]
    fn validate_names_every_problem() {
        let mut settings = LearnSettings::default();
        settings.anki.connect_url = "ftp://127.0.0.1".into();
        settings.anki.deck_name.clear();
        settings.anki.model_name.clear();
        settings.anki.field_mapping = FieldMapping {
            fields: vec![FieldBinding::new("", CardField::Pinyin)],
            audio_field: None,
        };

        let Err(Error::InvalidSettings(problems)) = settings.validate() else {
            panic!("invalid settings must be refused");
        };

        assert_eq!(
            problems,
            [
                "The AnkiConnect address \"ftp://127.0.0.1\" is not an http:// URL.",
                "The Anki deck name must not be empty.",
                "The Anki note type must not be empty.",
                "Every mapped Anki field needs a name.",
                "The field mapping needs an Anki field for the word.",
                "The field mapping needs an Anki field for the meaning.",
            ]
        );
    }

    #[test]
    fn settings_round_trip_through_the_store_and_survive_a_bad_value() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(load(&store).unwrap(), LearnSettings::default());

        let settings = LearnSettings {
            meaning_lang: MeaningLang::De,
            anki: AnkiSettings {
                create_reverse: true,
                ..AnkiSettings::default()
            },
        };
        save(&store, &settings).unwrap();
        assert_eq!(load(&store).unwrap(), settings);

        store.set_state(SETTINGS_KEY, "{ broken").unwrap();
        assert_eq!(load(&store).unwrap(), LearnSettings::default());
        assert_eq!(
            store.get_state(SETTINGS_KEY).unwrap().as_deref(),
            Some("{ broken"),
            "kept until the next save"
        );
    }
}
