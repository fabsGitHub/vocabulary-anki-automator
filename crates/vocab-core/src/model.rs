//! The neutral card model (decisions 4 and 7).
//!
//! Nothing in this module knows about Anki: field names such as `Front`,
//! `Context Sentence` or `Front Audio` exist only in `anki-bridge`'s mapper.
//! The serde shape of these types is a contract with both frontends; see
//! docs/architecture.md § "Serialized shapes" before renaming anything.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A vocabulary card, independent of where it came from (clipboard capture,
/// HSK curriculum, Anki import) and of where it goes (Anki, the local store).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Card {
    /// The captured expression, in `source_lang`.
    pub front: String,
    /// Its translation, in `target_lang`.
    pub back: String,
    /// Example sentence in `source_lang`; empty when none was found.
    #[serde(default)]
    pub example_source: String,
    /// `example_source` translated into `target_lang`; empty when none.
    #[serde(default)]
    pub example_target: String,
    /// Free-form notes (rarity hints, synonyms, ...).
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Canonical language code, see [`crate::language::normalize_lang`].
    pub source_lang: String,
    /// Canonical language code, see [`crate::language::normalize_lang`].
    pub target_lang: String,
    pub captured_at: DateTime<Utc>,
    /// TTS audio (MP3) of `front`, spoken in `source_lang`. Serialized as a
    /// base64 string (or `null`) so it survives the Tauri IPC round trip.
    #[serde(default, with = "base64_bytes_opt")]
    pub audio_front: Option<Vec<u8>>,
    /// Script-specific annotations (decision 7).
    #[serde(default)]
    pub script_metadata: ScriptMetadata,
}

impl Card {
    /// A card with only the required fields set; everything else empty.
    pub fn new(
        front: impl Into<String>,
        back: impl Into<String>,
        source_lang: impl Into<String>,
        target_lang: impl Into<String>,
        captured_at: DateTime<Utc>,
    ) -> Self {
        Self {
            front: front.into(),
            back: back.into(),
            example_source: String::new(),
            example_target: String::new(),
            notes: String::new(),
            tags: Vec::new(),
            source_lang: source_lang.into(),
            target_lang: target_lang.into(),
            captured_at,
            audio_front: None,
            script_metadata: ScriptMetadata::None,
        }
    }

    /// `example_source` and `example_target` joined by a newline, skipping
    /// empty halves. This is the legacy single-field example format.
    pub fn combined_example(&self) -> String {
        [self.example_source.as_str(), self.example_target.as_str()]
            .iter()
            .filter(|s| !s.trim().is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The pinyin, when this card carries Chinese script metadata.
    pub fn pinyin(&self) -> Option<&str> {
        match &self.script_metadata {
            ScriptMetadata::Chinese { pinyin, .. } => Some(pinyin.as_str()),
            ScriptMetadata::None => None,
        }
    }

    /// The HSK level, when this card carries Chinese script metadata and the
    /// word is in the bundled curriculum.
    pub fn hsk_level(&self) -> Option<u8> {
        match &self.script_metadata {
            ScriptMetadata::Chinese { hsk_level, .. } => *hsk_level,
            ScriptMetadata::None => None,
        }
    }
}

/// Script-specific card annotations (decision 7). New scripts (Japanese
/// furigana/JLPT, Korean romanization, ...) become new variants here instead
/// of new top-level `Card` fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScriptMetadata {
    #[default]
    None,
    Chinese {
        /// Tone-marked pinyin, syllables separated by single spaces.
        pinyin: String,
        /// Official HSK 3.0 level. `7` stands for the combined 7–9 band, which
        /// is how the upstream dataset publishes the advanced levels.
        hsk_level: Option<u8>,
        #[serde(default)]
        script_variant: ScriptVariant,
    },
}

impl ScriptMetadata {
    /// Stable lowercase name stored in `captures.script_kind`.
    pub fn kind(&self) -> &'static str {
        match self {
            ScriptMetadata::None => "none",
            ScriptMetadata::Chinese { .. } => "chinese",
        }
    }
}

/// HSK is a Simplified-Chinese standard, so Simplified is the default;
/// Traditional is a stretch goal (OpenCC conversion), not MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptVariant {
    #[default]
    Simplified,
    Traditional,
}

impl ScriptVariant {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScriptVariant::Simplified => "simplified",
            ScriptVariant::Traditional => "traditional",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "simplified" => Some(ScriptVariant::Simplified),
            "traditional" => Some(ScriptVariant::Traditional),
            _ => None,
        }
    }
}

/// Where a stored capture row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOrigin {
    /// Captured from the clipboard by Automator.
    #[default]
    Capture,
    /// Introduced from the bundled HSK curriculum by Learn's daily queue.
    Curriculum,
    /// Imported from an existing Anki collection.
    AnkiImport,
}

impl CaptureOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            CaptureOrigin::Capture => "capture",
            CaptureOrigin::Curriculum => "curriculum",
            CaptureOrigin::AnkiImport => "anki_import",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "capture" => Some(CaptureOrigin::Capture),
            "curriculum" => Some(CaptureOrigin::Curriculum),
            "anki_import" => Some(CaptureOrigin::AnkiImport),
            _ => None,
        }
    }
}

/// A card as persisted in the local store (`captures` table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredCapture {
    pub id: i64,
    pub card: Card,
    pub origin: CaptureOrigin,
    pub synced_to_anki: bool,
}

/// One row of the bundled HSK 3.0 curriculum (`hsk_curriculum` table and
/// `crates/vocab-core/data/hsk_curriculum.json`). Reference data, never
/// user data: "learned" means having an `srs_state` row, not appearing here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurriculumItem {
    /// Simplified form; primary key.
    pub hanzi: String,
    #[serde(default)]
    pub traditional: Option<String>,
    /// HSK 3.0 level, 1–7 (7 = the combined 7–9 band).
    pub hsk_level: u8,
    #[serde(default)]
    pub frequency: Option<i64>,
    /// Global order: `hsk_level` ascending, then `frequency` ascending.
    pub curriculum_order: i64,
    pub pinyin: String,
    #[serde(default)]
    pub pos: Option<String>,
    #[serde(default)]
    pub meaning_en: Option<String>,
    #[serde(default)]
    pub meaning_de: Option<String>,
    #[serde(default)]
    pub cloze_sentence: Option<String>,
    #[serde(default)]
    pub example_sentence_source: Option<String>,
    #[serde(default)]
    pub example_sentence_pinyin: Option<String>,
    #[serde(default)]
    pub example_sentence_target: Option<String>,
    #[serde(default)]
    pub vocab_audio_path: Option<String>,
    #[serde(default)]
    pub sentence_audio_path: Option<String>,
    #[serde(default)]
    pub image_path: Option<String>,
}

/// Serde adapter: `Option<Vec<u8>>` <-> base64 string or `null`.
pub mod base64_bytes_opt {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => s.serialize_some(&STANDARD.encode(bytes)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let encoded: Option<String> = Option::deserialize(d)?;
        match encoded {
            Some(text) if !text.is_empty() => STANDARD
                .decode(text.as_bytes())
                .map(Some)
                .map_err(serde::de::Error::custom),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn sample() -> Card {
        let mut card = Card::new(
            "serendipity",
            "glücklicher Zufall",
            "en",
            "de",
            Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap(),
        );
        card.example_source = "It was pure serendipity.".into();
        card.example_target = "Es war reiner Zufall.".into();
        card.audio_front = Some(vec![0xff, 0xfb, 0x90, 0x00]);
        card
    }

    #[test]
    fn card_round_trips_through_json_with_base64_audio() {
        let card = sample();
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["audio_front"], "//uQAA==");
        assert_eq!(
            json["script_metadata"],
            serde_json::json!({ "kind": "none" })
        );
        assert_eq!(json["captured_at"], "2026-10-31T12:00:00Z");
        let back: Card = serde_json::from_value(json).unwrap();
        assert_eq!(back, card);
    }

    #[test]
    fn card_deserializes_with_optional_fields_missing() {
        let card: Card = serde_json::from_value(serde_json::json!({
            "front": "Haus",
            "back": "house",
            "source_lang": "de",
            "target_lang": "en",
            "captured_at": "2026-10-31T12:00:00Z",
            "audio_front": null
        }))
        .unwrap();
        assert_eq!(card.example_source, "");
        assert!(card.tags.is_empty());
        assert_eq!(card.audio_front, None);
        assert_eq!(card.script_metadata, ScriptMetadata::None);
    }

    #[test]
    fn chinese_metadata_shape_is_tagged() {
        let meta = ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        };
        assert_eq!(
            serde_json::to_value(&meta).unwrap(),
            serde_json::json!({
                "kind": "chinese",
                "pinyin": "xué xí",
                "hsk_level": 1,
                "script_variant": "simplified"
            })
        );
        assert_eq!(meta.kind(), "chinese");
    }

    #[test]
    fn combined_example_skips_empty_halves() {
        let mut card = sample();
        assert_eq!(
            card.combined_example(),
            "It was pure serendipity.\nEs war reiner Zufall."
        );
        card.example_target.clear();
        assert_eq!(card.combined_example(), "It was pure serendipity.");
    }
}
