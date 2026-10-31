//! Card <-> Anki note mapping: the only place in the workspace that knows Anki
//! field names (decision 4).
//!
//! `vocab_core::Card` is deliberately Anki-agnostic. A [`FieldMapping`] says
//! which card value goes into which field of the user's note type, so any note
//! type works (Milestone H); the pre-refactor code hardcoded AppleStyleFinalV3's
//! `Front`/`Back`/`Context Sentence`/`Notes`/`Front Audio`. That note type and
//! a Chinese one (Milestone J5) ship as built-in mappings. The serde shape of
//! the mapping is the Automator config's `anki.field_mapping`
//! (docs/architecture.md § 8.1).
//!
//! [`card_from_note`] is the inverse, used by Learn's Anki import, so a card
//! written by [`map_card`] reads back as the same card.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vocab_core::{Card, ScriptMetadata, ScriptVariant};

use crate::{client::NoteInfo, html};

/// A card value that can be written to, and read back from, an Anki field.
/// Serialized as its snake_case name (`"example_source"`, `"hsk_level"`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardField {
    /// The side asked first: `card.front` on the forward note, `card.back`
    /// on the reverse note.
    Front,
    /// The answer side: `card.back` forward, `card.front` reversed.
    Back,
    /// The example sentence in the language of this note's `Front`.
    ExampleSource,
    /// The example sentence in the language of this note's `Back`.
    ExampleTarget,
    /// Both example sentences in display order (this note's front language
    /// first), one per line: the legacy single-field example format.
    Example,
    Notes,
    /// Tone-marked pinyin; empty for cards without Chinese script metadata.
    Pinyin,
    /// The HSK level as a number (`"3"`); empty when unknown.
    HskLevel,
}

/// "Write `source` into the Anki field named `anki_field`."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldBinding {
    pub anki_field: String,
    pub source: CardField,
}

impl FieldBinding {
    pub fn new(anki_field: impl Into<String>, source: CardField) -> Self {
        Self {
            anki_field: anki_field.into(),
            source,
        }
    }
}

/// How cards map onto the fields of one Anki note type.
///
/// Anki fields the mapping does not mention are left empty. When several
/// bindings name the same Anki field, the first one wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMapping {
    pub fields: Vec<FieldBinding>,
    /// The field that receives the `[sound:...]` reference of attached TTS
    /// audio. `None` (or blank) sends no audio at all.
    #[serde(default)]
    pub audio_field: Option<String>,
}

impl FieldMapping {
    /// The AppleStyleFinalV3 note type the Automator has always targeted.
    pub fn apple_style_default() -> Self {
        Self {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("Context Sentence", CardField::Example),
                FieldBinding::new("Notes", CardField::Notes),
            ],
            audio_field: Some("Front Audio".to_string()),
        }
    }

    /// A Chinese note type with Hanzi/Pinyin/Meaning/Sentence/Audio fields
    /// (Milestone J5).
    pub fn chinese_default() -> Self {
        Self {
            fields: vec![
                FieldBinding::new("Hanzi", CardField::Front),
                FieldBinding::new("Pinyin", CardField::Pinyin),
                FieldBinding::new("Meaning", CardField::Back),
                FieldBinding::new("Sentence", CardField::Example),
            ],
            audio_field: Some("Audio".to_string()),
        }
    }

    /// The configured audio field, unless it is unset or blank.
    pub fn audio_field(&self) -> Option<&str> {
        self.audio_field
            .as_deref()
            .filter(|field| !field.trim().is_empty())
    }

    /// The Anki fields this mapping writes (audio field included) that a note
    /// type with `model_fields` (from `modelFieldNames`) does not have, in
    /// mapping order. Names are compared the way AnkiConnect matches them:
    /// text fields case-insensitively, the audio field exactly. AnkiConnect
    /// silently drops whatever does not match, so a non-empty result is what
    /// the settings UI flags as "this note type does not fit the mapping"
    /// (Milestone J5).
    pub fn missing_fields(&self, model_fields: &[String]) -> Vec<String> {
        let mut missing: Vec<String> = Vec::new();
        let mut flag = |name: &str, known: bool| {
            if !known && !missing.iter().any(|m| m == name) {
                missing.push(name.to_string());
            }
        };
        for binding in &self.fields {
            let lowered = binding.anki_field.to_lowercase();
            let known = model_fields
                .iter()
                .any(|field| field.to_lowercase() == lowered);
            flag(&binding.anki_field, known);
        }
        if let Some(audio) = self.audio_field() {
            flag(audio, model_fields.iter().any(|field| field == audio));
        }
        missing
    }

    fn binds(&self, source: CardField) -> bool {
        self.fields.iter().any(|binding| binding.source == source)
    }
}

impl Default for FieldMapping {
    fn default() -> Self {
        Self::apple_style_default()
    }
}

/// Which way round a card is written. `Reverse` is the extra target -> source
/// note created when `anki.create_reverse` is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Forward,
    Reverse,
}

impl Direction {
    fn file_tag(self) -> &'static str {
        match self {
            Direction::Forward => "fwd",
            Direction::Reverse => "rev",
        }
    }
}

/// Where notes go: an Anki deck and note type ("model" in AnkiConnect).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AnkiTarget {
    pub deck: String,
    pub model: String,
}

impl AnkiTarget {
    pub fn new(deck: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            deck: deck.into(),
            model: model.into(),
        }
    }
}

/// One Anki note, ready to send. [`add_note_params`] turns it into
/// AnkiConnect's `addNote` parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSpec {
    pub deck: String,
    pub model: String,
    /// Anki field name -> field content.
    pub fields: BTreeMap<String, String>,
    pub tags: Vec<String>,
    pub audio: Option<AudioAttachment>,
}

/// Audio sent inline with `addNote`; Anki stores the file in its media folder
/// and appends `[sound:filename]` to each of `fields`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioAttachment {
    pub filename: String,
    /// The MP3 bytes, base64-encoded as AnkiConnect expects.
    pub data_b64: String,
    pub fields: Vec<String>,
}

/// Maps `card` onto `mapping`'s note type for `direction`.
///
/// `audio` is the MP3 for this note's front (normally `card.audio_front`
/// forward and TTS of `card.back` reversed). It is attached only when it is
/// non-empty and the mapping has an audio field, as
/// `vocab_<captured_at millis>_<fwd|rev>.mp3`, so the forward and reverse
/// recordings of one card never collide.
pub fn map_card(
    card: &Card,
    direction: Direction,
    target: &AnkiTarget,
    mapping: &FieldMapping,
    audio: Option<&[u8]>,
) -> NoteSpec {
    let mut fields = BTreeMap::new();
    for binding in &mapping.fields {
        fields
            .entry(binding.anki_field.clone())
            .or_insert_with(|| field_value(card, direction, binding.source));
    }
    let audio = match (audio, mapping.audio_field()) {
        (Some(bytes), Some(field)) if !bytes.is_empty() => Some(AudioAttachment {
            filename: format!(
                "vocab_{}_{}.mp3",
                card.captured_at.timestamp_millis(),
                direction.file_tag()
            ),
            data_b64: STANDARD.encode(bytes),
            fields: vec![field.to_string()],
        }),
        _ => None,
    };
    NoteSpec {
        deck: target.deck.clone(),
        model: target.model.clone(),
        fields,
        tags: card.tags.clone(),
        audio,
    }
}

fn field_value(card: &Card, direction: Direction, source: CardField) -> String {
    let (front, back, example_front, example_back) = match direction {
        Direction::Forward => (
            &card.front,
            &card.back,
            &card.example_source,
            &card.example_target,
        ),
        Direction::Reverse => (
            &card.back,
            &card.front,
            &card.example_target,
            &card.example_source,
        ),
    };
    match source {
        CardField::Front => front.clone(),
        CardField::Back => back.clone(),
        CardField::ExampleSource => example_front.clone(),
        CardField::ExampleTarget => example_back.clone(),
        CardField::Example => combine_examples(example_front, example_back),
        CardField::Notes => card.notes.clone(),
        CardField::Pinyin => card.pinyin().unwrap_or_default().to_string(),
        CardField::HskLevel => card
            .hsk_level()
            .map(|level| level.to_string())
            .unwrap_or_default(),
    }
}

/// The two halves joined by a newline, skipping blank ones; the same format
/// as `Card::combined_example`, but in this note's display order.
fn combine_examples(first: &str, second: &str) -> String {
    [first, second]
        .into_iter()
        .filter(|half| !half.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// AnkiConnect's `addNote` `params` for `note`, exactly as sent. Duplicates
/// are rejected within the target deck; the `audio` key is present only when
/// the note carries audio.
pub fn add_note_params(note: &NoteSpec) -> Value {
    let mut params = json!({
        "deckName": note.deck,
        "modelName": note.model,
        "fields": note.fields,
        "options": { "allowDuplicate": false, "duplicateScope": "deck" },
        "tags": note.tags,
    });
    if let Some(audio) = &note.audio {
        params["audio"] = json!([{
            "data": audio.data_b64,
            "filename": audio.filename,
            "fields": audio.fields,
        }]);
    }
    json!({ "note": params })
}

/// Reads an Anki note back into a card (Learn's Anki import), the inverse of
/// [`map_card`] with [`Direction::Forward`].
///
/// Field contents are converted from Anki's HTML to plain text (tags removed,
/// `&nbsp;`/`&amp;`/... decoded, `[sound:...]` references dropped). Returns
/// `None` unless both front and back are non-empty. Explicit
/// `ExampleSource`/`ExampleTarget` bindings are read as they are; otherwise an
/// `Example` binding is split at its first line break. `Pinyin`/`HskLevel`
/// bindings rebuild `ScriptMetadata::Chinese` when either has a value. The
/// card gets the note's tags, no audio, and the given languages (canonical
/// codes) and timestamp.
///
/// `captured_at` is a `chrono::DateTime<Utc>`, as in docs/architecture.md
/// § 9. It is accepted as any `Serialize` value only because chrono is not
/// yet a normal dependency of this crate (reported as a contract issue); a
/// value that does not serialize to an RFC 3339 timestamp yields `None`.
pub fn card_from_note(
    note: &NoteInfo,
    mapping: &FieldMapping,
    source_lang: &str,
    target_lang: &str,
    captured_at: impl Serialize,
) -> Option<Card> {
    let read = |source: CardField| -> String {
        mapping
            .fields
            .iter()
            .filter(|binding| binding.source == source)
            .filter_map(|binding| note.field(&binding.anki_field))
            .map(html::to_plain_text)
            .find(|value| !value.is_empty())
            .unwrap_or_default()
    };

    let front = read(CardField::Front);
    let back = read(CardField::Back);
    if front.is_empty() || back.is_empty() {
        return None;
    }
    let mut card = new_card(front, back, source_lang, target_lang, captured_at)?;
    (card.example_source, card.example_target) =
        if mapping.binds(CardField::ExampleSource) || mapping.binds(CardField::ExampleTarget) {
            (
                read(CardField::ExampleSource),
                read(CardField::ExampleTarget),
            )
        } else {
            split_example(&read(CardField::Example))
        };
    card.notes = read(CardField::Notes);
    card.tags = note.tags.clone();
    card.script_metadata = chinese_metadata(read(CardField::Pinyin), &read(CardField::HskLevel));
    Some(card)
}

/// Builds a card without naming `chrono::DateTime`, which this crate can only
/// do once chrono is a normal dependency: the timestamp travels through the
/// card's own serde representation (lossless RFC 3339).
fn new_card(
    front: String,
    back: String,
    source_lang: &str,
    target_lang: &str,
    captured_at: impl Serialize,
) -> Option<Card> {
    let captured_at = serde_json::to_value(captured_at).ok()?;
    serde_json::from_value(json!({
        "front": front,
        "back": back,
        "source_lang": source_lang,
        "target_lang": target_lang,
        "captured_at": captured_at,
    }))
    .ok()
}

fn split_example(example: &str) -> (String, String) {
    match example.split_once('\n') {
        Some((source, target)) => (source.trim().to_string(), target.trim().to_string()),
        None => (example.trim().to_string(), String::new()),
    }
}

fn chinese_metadata(pinyin: String, hsk_level: &str) -> ScriptMetadata {
    let hsk_level = parse_hsk_level(hsk_level);
    if pinyin.is_empty() && hsk_level.is_none() {
        return ScriptMetadata::None;
    }
    ScriptMetadata::Chinese {
        pinyin,
        hsk_level,
        script_variant: ScriptVariant::Simplified,
    }
}

/// The first number in `text` ("3", "HSK 3", "HSK3", "7-9") as an HSK 3.0
/// level. Levels 7–9 are one band in the curriculum, stored as 7.
fn parse_hsk_level(text: &str) -> Option<u8> {
    let digits: String = text
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    match digits.parse::<u8>().ok()? {
        level @ 1..=6 => Some(level),
        7..=9 => Some(7),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, TimeZone, Utc};

    use super::*;
    use crate::client::NoteField;

    const FWD_AUDIO: &[u8] = &[0xff, 0xfb, 0x90, 0x00];
    const REV_AUDIO: &[u8] = b"ID3\x03\x00";

    /// 2026-10-31T12:00:00.123Z: the millisecond part catches a filename
    /// built from whole seconds.
    fn captured_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap() + Duration::milliseconds(123)
    }

    fn english_card() -> Card {
        let mut card = Card::new(
            "serendipity",
            "glücklicher Zufall",
            "en",
            "de",
            captured_at(),
        );
        card.example_source = "It was pure serendipity.".into();
        card.example_target = "Es war reiner Zufall.".into();
        card.notes = "Rare word. More common synonym: luck".into();
        card.tags = vec!["vocab-automator".into(), "en-de".into()];
        card.audio_front = Some(FWD_AUDIO.to_vec());
        card
    }

    fn chinese_card() -> Card {
        let mut card = Card::new("学习", "lernen", "zh-CN", "de", captured_at());
        card.example_source = "我们一起学习中文。".into();
        card.example_target = "Wir lernen zusammen Chinesisch.".into();
        card.tags = vec!["hsk".into()];
        card.script_metadata = ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        };
        card
    }

    /// Every card field, each in its own Anki field.
    fn every_field_mapping() -> FieldMapping {
        FieldMapping {
            fields: vec![
                FieldBinding::new("F", CardField::Front),
                FieldBinding::new("B", CardField::Back),
                FieldBinding::new("ES", CardField::ExampleSource),
                FieldBinding::new("ET", CardField::ExampleTarget),
                FieldBinding::new("E", CardField::Example),
                FieldBinding::new("N", CardField::Notes),
                FieldBinding::new("P", CardField::Pinyin),
                FieldBinding::new("H", CardField::HskLevel),
            ],
            audio_field: Some("A".into()),
        }
    }

    fn target() -> AnkiTarget {
        AnkiTarget::new("Deck", "Model")
    }

    struct MapCase {
        name: &'static str,
        card: Card,
        mapping: FieldMapping,
        direction: Direction,
        audio: Option<&'static [u8]>,
        fields: &'static [(&'static str, &'static str)],
        /// (filename, base64 data, fields)
        attachment: Option<(&'static str, &'static str, &'static [&'static str])>,
    }

    #[test]
    fn map_card_table() {
        let no_audio_field = FieldMapping {
            audio_field: None,
            ..FieldMapping::apple_style_default()
        };
        let blank_audio_field = FieldMapping {
            audio_field: Some("  ".into()),
            ..FieldMapping::apple_style_default()
        };
        let mut target_only_example = english_card();
        target_only_example.example_source = " ".into();

        let cases = [
            MapCase {
                name: "apple forward with audio",
                card: english_card(),
                mapping: FieldMapping::apple_style_default(),
                direction: Direction::Forward,
                audio: Some(FWD_AUDIO),
                fields: &[
                    ("Front", "serendipity"),
                    ("Back", "glücklicher Zufall"),
                    (
                        "Context Sentence",
                        "It was pure serendipity.\nEs war reiner Zufall.",
                    ),
                    ("Notes", "Rare word. More common synonym: luck"),
                ],
                attachment: Some(("vocab_1793448000123_fwd.mp3", "//uQAA==", &["Front Audio"])),
            },
            MapCase {
                name: "apple reverse with audio",
                card: english_card(),
                mapping: FieldMapping::apple_style_default(),
                direction: Direction::Reverse,
                audio: Some(REV_AUDIO),
                fields: &[
                    ("Front", "glücklicher Zufall"),
                    ("Back", "serendipity"),
                    (
                        "Context Sentence",
                        "Es war reiner Zufall.\nIt was pure serendipity.",
                    ),
                    ("Notes", "Rare word. More common synonym: luck"),
                ],
                attachment: Some(("vocab_1793448000123_rev.mp3", "SUQzAwA=", &["Front Audio"])),
            },
            MapCase {
                name: "apple forward without audio bytes",
                card: english_card(),
                mapping: FieldMapping::apple_style_default(),
                direction: Direction::Forward,
                audio: None,
                fields: &[
                    ("Front", "serendipity"),
                    ("Back", "glücklicher Zufall"),
                    (
                        "Context Sentence",
                        "It was pure serendipity.\nEs war reiner Zufall.",
                    ),
                    ("Notes", "Rare word. More common synonym: luck"),
                ],
                attachment: None,
            },
            MapCase {
                name: "audio bytes but no audio field",
                card: english_card(),
                mapping: no_audio_field,
                direction: Direction::Forward,
                audio: Some(FWD_AUDIO),
                fields: &[
                    ("Front", "serendipity"),
                    ("Back", "glücklicher Zufall"),
                    (
                        "Context Sentence",
                        "It was pure serendipity.\nEs war reiner Zufall.",
                    ),
                    ("Notes", "Rare word. More common synonym: luck"),
                ],
                attachment: None,
            },
            MapCase {
                name: "audio bytes but blank audio field",
                card: english_card(),
                mapping: blank_audio_field,
                direction: Direction::Reverse,
                audio: Some(REV_AUDIO),
                fields: &[
                    ("Front", "glücklicher Zufall"),
                    ("Back", "serendipity"),
                    (
                        "Context Sentence",
                        "Es war reiner Zufall.\nIt was pure serendipity.",
                    ),
                    ("Notes", "Rare word. More common synonym: luck"),
                ],
                attachment: None,
            },
            MapCase {
                name: "empty audio bytes",
                card: english_card(),
                mapping: FieldMapping::apple_style_default(),
                direction: Direction::Forward,
                audio: Some(&[]),
                fields: &[
                    ("Front", "serendipity"),
                    ("Back", "glücklicher Zufall"),
                    (
                        "Context Sentence",
                        "It was pure serendipity.\nEs war reiner Zufall.",
                    ),
                    ("Notes", "Rare word. More common synonym: luck"),
                ],
                attachment: None,
            },
            MapCase {
                name: "blank example half is skipped",
                card: target_only_example,
                mapping: FieldMapping::apple_style_default(),
                direction: Direction::Forward,
                audio: None,
                fields: &[
                    ("Front", "serendipity"),
                    ("Back", "glücklicher Zufall"),
                    ("Context Sentence", "Es war reiner Zufall."),
                    ("Notes", "Rare word. More common synonym: luck"),
                ],
                attachment: None,
            },
            MapCase {
                name: "chinese forward with audio",
                card: chinese_card(),
                mapping: FieldMapping::chinese_default(),
                direction: Direction::Forward,
                audio: Some(FWD_AUDIO),
                fields: &[
                    ("Hanzi", "学习"),
                    ("Pinyin", "xué xí"),
                    ("Meaning", "lernen"),
                    (
                        "Sentence",
                        "我们一起学习中文。\nWir lernen zusammen Chinesisch.",
                    ),
                ],
                attachment: Some(("vocab_1793448000123_fwd.mp3", "//uQAA==", &["Audio"])),
            },
            MapCase {
                name: "chinese reverse with audio",
                card: chinese_card(),
                mapping: FieldMapping::chinese_default(),
                direction: Direction::Reverse,
                audio: Some(REV_AUDIO),
                fields: &[
                    ("Hanzi", "lernen"),
                    ("Pinyin", "xué xí"),
                    ("Meaning", "学习"),
                    (
                        "Sentence",
                        "Wir lernen zusammen Chinesisch.\n我们一起学习中文。",
                    ),
                ],
                attachment: Some(("vocab_1793448000123_rev.mp3", "SUQzAwA=", &["Audio"])),
            },
            MapCase {
                name: "chinese mapping on a card without chinese metadata",
                card: english_card(),
                mapping: FieldMapping::chinese_default(),
                direction: Direction::Forward,
                audio: None,
                fields: &[
                    ("Hanzi", "serendipity"),
                    ("Pinyin", ""),
                    ("Meaning", "glücklicher Zufall"),
                    (
                        "Sentence",
                        "It was pure serendipity.\nEs war reiner Zufall.",
                    ),
                ],
                attachment: None,
            },
            MapCase {
                name: "every field, chinese forward",
                card: chinese_card(),
                mapping: every_field_mapping(),
                direction: Direction::Forward,
                audio: None,
                fields: &[
                    ("F", "学习"),
                    ("B", "lernen"),
                    ("ES", "我们一起学习中文。"),
                    ("ET", "Wir lernen zusammen Chinesisch."),
                    ("E", "我们一起学习中文。\nWir lernen zusammen Chinesisch."),
                    ("N", ""),
                    ("P", "xué xí"),
                    ("H", "1"),
                ],
                attachment: None,
            },
            MapCase {
                name: "every field, english reverse",
                card: english_card(),
                mapping: every_field_mapping(),
                direction: Direction::Reverse,
                audio: Some(REV_AUDIO),
                fields: &[
                    ("F", "glücklicher Zufall"),
                    ("B", "serendipity"),
                    ("ES", "Es war reiner Zufall."),
                    ("ET", "It was pure serendipity."),
                    ("E", "Es war reiner Zufall.\nIt was pure serendipity."),
                    ("N", "Rare word. More common synonym: luck"),
                    ("P", ""),
                    ("H", ""),
                ],
                attachment: Some(("vocab_1793448000123_rev.mp3", "SUQzAwA=", &["A"])),
            },
        ];

        for case in cases {
            let note = map_card(
                &case.card,
                case.direction,
                &target(),
                &case.mapping,
                case.audio,
            );
            let expected_fields: BTreeMap<String, String> = case
                .fields
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect();
            let expected_audio = case
                .attachment
                .map(|(filename, data, fields)| AudioAttachment {
                    filename: filename.into(),
                    data_b64: data.into(),
                    fields: fields.iter().map(|f| f.to_string()).collect(),
                });
            assert_eq!(
                note,
                NoteSpec {
                    deck: "Deck".into(),
                    model: "Model".into(),
                    fields: expected_fields,
                    tags: case.card.tags.clone(),
                    audio: expected_audio,
                },
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn forward_example_matches_the_card_s_legacy_combined_example() {
        let pairs = [
            ("It was pure serendipity.", "Es war reiner Zufall."),
            ("", "Es war reiner Zufall."),
            ("It was pure serendipity.", "  "),
            ("", ""),
        ];
        let mapping = FieldMapping {
            fields: vec![FieldBinding::new("E", CardField::Example)],
            audio_field: None,
        };
        for (source, target_text) in pairs {
            let mut card = english_card();
            card.example_source = source.into();
            card.example_target = target_text.into();
            let note = map_card(&card, Direction::Forward, &target(), &mapping, None);
            assert_eq!(
                note.fields["E"],
                card.combined_example(),
                "{source:?}/{target_text:?}"
            );
        }
    }

    #[test]
    fn first_binding_wins_for_a_repeated_anki_field() {
        let mapping = FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Front", CardField::Pinyin),
            ],
            audio_field: None,
        };
        let note = map_card(
            &chinese_card(),
            Direction::Forward,
            &target(),
            &mapping,
            None,
        );
        assert_eq!(note.fields.len(), 1);
        assert_eq!(note.fields["Front"], "学习");
    }

    #[test]
    fn add_note_params_is_the_exact_ankiconnect_shape() {
        let card = english_card();
        let mapping = FieldMapping::apple_style_default();
        let target = AnkiTarget::new("English::AutomatedVocabulary", "AppleStyleFinalV3");

        let with_audio = map_card(
            &card,
            Direction::Forward,
            &target,
            &mapping,
            Some(FWD_AUDIO),
        );
        assert_eq!(
            add_note_params(&with_audio),
            json!({
                "note": {
                    "deckName": "English::AutomatedVocabulary",
                    "modelName": "AppleStyleFinalV3",
                    "fields": {
                        "Front": "serendipity",
                        "Back": "glücklicher Zufall",
                        "Context Sentence": "It was pure serendipity.\nEs war reiner Zufall.",
                        "Notes": "Rare word. More common synonym: luck"
                    },
                    "options": { "allowDuplicate": false, "duplicateScope": "deck" },
                    "tags": ["vocab-automator", "en-de"],
                    "audio": [{
                        "data": "//uQAA==",
                        "filename": "vocab_1793448000123_fwd.mp3",
                        "fields": ["Front Audio"]
                    }]
                }
            })
        );

        let without_audio = map_card(&card, Direction::Forward, &target, &mapping, None);
        let params = add_note_params(&without_audio);
        assert!(params["note"].get("audio").is_none(), "{params}");
        assert_eq!(params["note"]["tags"], json!(["vocab-automator", "en-de"]));
    }

    #[test]
    fn field_mapping_serde_shape() {
        assert_eq!(
            serde_json::to_value(FieldMapping::apple_style_default()).unwrap(),
            json!({
                "fields": [
                    { "anki_field": "Front", "source": "front" },
                    { "anki_field": "Back", "source": "back" },
                    { "anki_field": "Context Sentence", "source": "example" },
                    { "anki_field": "Notes", "source": "notes" }
                ],
                "audio_field": "Front Audio"
            })
        );
        assert_eq!(
            serde_json::to_value(FieldMapping::chinese_default()).unwrap(),
            json!({
                "fields": [
                    { "anki_field": "Hanzi", "source": "front" },
                    { "anki_field": "Pinyin", "source": "pinyin" },
                    { "anki_field": "Meaning", "source": "back" },
                    { "anki_field": "Sentence", "source": "example" }
                ],
                "audio_field": "Audio"
            })
        );

        let parsed: FieldMapping = serde_json::from_value(json!({
            "fields": [
                { "anki_field": "a", "source": "front" },
                { "anki_field": "b", "source": "back" },
                { "anki_field": "c", "source": "example_source" },
                { "anki_field": "d", "source": "example_target" },
                { "anki_field": "e", "source": "example" },
                { "anki_field": "f", "source": "notes" },
                { "anki_field": "g", "source": "pinyin" },
                { "anki_field": "h", "source": "hsk_level" }
            ]
        }))
        .unwrap();
        let sources: Vec<CardField> = parsed.fields.iter().map(|b| b.source).collect();
        assert_eq!(
            sources,
            [
                CardField::Front,
                CardField::Back,
                CardField::ExampleSource,
                CardField::ExampleTarget,
                CardField::Example,
                CardField::Notes,
                CardField::Pinyin,
                CardField::HskLevel,
            ]
        );
        assert_eq!(parsed.audio_field, None);
        assert!(serde_json::from_value::<FieldMapping>(json!({
            "fields": [{ "anki_field": "a", "source": "Front" }]
        }))
        .is_err());
    }

    #[test]
    fn default_mapping_is_apple_style() {
        assert_eq!(FieldMapping::default(), FieldMapping::apple_style_default());
    }

    #[test]
    fn missing_fields_lists_unmapped_note_type_fields() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let chinese = FieldMapping::chinese_default();
        assert_eq!(
            chinese.missing_fields(&names(&["hanzi", "Pinyin", "Meaning", "Sentence", "Audio"])),
            Vec::<String>::new()
        );
        assert_eq!(
            chinese.missing_fields(&names(&["Front", "Back", "Pinyin"])),
            names(&["Hanzi", "Meaning", "Sentence", "Audio"])
        );
        let no_audio = FieldMapping {
            audio_field: None,
            ..FieldMapping::apple_style_default()
        };
        assert_eq!(
            no_audio.missing_fields(&names(&["Front", "Back", "Notes"])),
            names(&["Context Sentence"])
        );
        // AnkiConnect attaches media only to an exactly named field.
        assert_eq!(
            FieldMapping::apple_style_default().missing_fields(&names(&[
                "front",
                "back",
                "context sentence",
                "notes",
                "front audio"
            ])),
            names(&["Front Audio"])
        );
    }

    /// The note as `notesInfo` would report it after Anki stored `spec`
    /// (audio appended to its field as a `[sound:...]` reference).
    fn stored(spec: &NoteSpec) -> NoteInfo {
        let mut fields: BTreeMap<String, String> = spec.fields.clone();
        if let Some(audio) = &spec.audio {
            for field in &audio.fields {
                fields
                    .entry(field.clone())
                    .or_default()
                    .push_str(&format!("[sound:{}]", audio.filename));
            }
        }
        NoteInfo {
            note_id: 1_700_000_000_000,
            model_name: spec.model.clone(),
            tags: spec.tags.clone(),
            fields: fields
                .into_iter()
                .enumerate()
                .map(|(order, (name, value))| {
                    let order = u32::try_from(order).unwrap();
                    (name, NoteField { value, order })
                })
                .collect(),
            cards: vec![1_700_000_000_001],
            modified: Some(1_793_448_000),
        }
    }

    #[test]
    fn card_from_note_round_trips_map_card() {
        let mut chinese_without_level = chinese_card();
        chinese_without_level.script_metadata = ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: None,
            script_variant: ScriptVariant::Simplified,
        };
        let cases = [
            (
                "apple style",
                english_card(),
                FieldMapping::apple_style_default(),
                english_card(),
            ),
            // chinese_default has no HskLevel or Notes binding, so those stay empty.
            (
                "chinese",
                chinese_card(),
                FieldMapping::chinese_default(),
                chinese_without_level,
            ),
            (
                "every field, chinese",
                chinese_card(),
                every_field_mapping(),
                chinese_card(),
            ),
            (
                "every field, english",
                english_card(),
                every_field_mapping(),
                english_card(),
            ),
        ];
        for (name, card, mapping, mut expected) in cases {
            let spec = map_card(
                &card,
                Direction::Forward,
                &target(),
                &mapping,
                card.audio_front.as_deref(),
            );
            let back = card_from_note(
                &stored(&spec),
                &mapping,
                &card.source_lang,
                &card.target_lang,
                card.captured_at,
            );
            expected.audio_front = None;
            assert_eq!(back, Some(expected), "{name}");
        }
    }

    fn note_with(fields: &[(&str, &str)]) -> NoteInfo {
        NoteInfo {
            note_id: 42,
            model_name: "AppleStyleFinalV3".into(),
            tags: vec!["imported".into()],
            fields: fields
                .iter()
                .enumerate()
                .map(|(order, (name, value))| {
                    let field = NoteField {
                        value: value.to_string(),
                        order: u32::try_from(order).unwrap(),
                    };
                    (name.to_string(), field)
                })
                .collect(),
            cards: vec![],
            modified: None,
        }
    }

    #[test]
    fn card_from_note_reads_anki_editor_html() {
        let note = note_with(&[
            ("Front", "Haus&nbsp;[sound:vocab_1_fwd.mp3]"),
            ("Back", "<b>house</b>"),
            (
                "Context Sentence",
                "Das Haus ist groß.<br>The house is big.",
            ),
            ("Notes", "<div>first</div><div>second &amp; third</div>"),
            ("Front Audio", "[sound:vocab_1_fwd.mp3]"),
        ]);
        let card = card_from_note(
            &note,
            &FieldMapping::apple_style_default(),
            "de",
            "en",
            captured_at(),
        )
        .unwrap();
        assert_eq!(card.front, "Haus");
        assert_eq!(card.back, "house");
        assert_eq!(card.example_source, "Das Haus ist groß.");
        assert_eq!(card.example_target, "The house is big.");
        assert_eq!(card.notes, "first\nsecond & third");
        assert_eq!(card.tags, ["imported"]);
        assert_eq!(
            (card.source_lang.as_str(), card.target_lang.as_str()),
            ("de", "en")
        );
        assert_eq!(card.captured_at, captured_at());
        assert_eq!(card.audio_front, None);
        assert_eq!(card.script_metadata, ScriptMetadata::None);
    }

    #[test]
    fn card_from_note_matches_field_names_case_insensitively() {
        let note = note_with(&[("front", "Haus"), ("BACK", "house")]);
        let card = card_from_note(
            &note,
            &FieldMapping::apple_style_default(),
            "de",
            "en",
            captured_at(),
        )
        .unwrap();
        assert_eq!((card.front.as_str(), card.back.as_str()), ("Haus", "house"));
        assert_eq!(
            (card.example_source.as_str(), card.notes.as_str()),
            ("", "")
        );
    }

    #[test]
    fn card_from_note_requires_front_and_back() {
        let mapping = FieldMapping::apple_style_default();
        let cases: [(&str, &[(&str, &str)]); 4] = [
            ("empty back", &[("Front", "Haus"), ("Back", "")]),
            (
                "markup-only front",
                &[("Front", "<br>&nbsp;"), ("Back", "house")],
            ),
            (
                "sound-only back",
                &[("Front", "Haus"), ("Back", "[sound:x.mp3]")],
            ),
            ("missing back field", &[("Front", "Haus")]),
        ];
        for (name, fields) in cases {
            let card = card_from_note(&note_with(fields), &mapping, "de", "en", captured_at());
            assert_eq!(card, None, "{name}");
        }
    }

    #[test]
    fn card_from_note_rebuilds_chinese_metadata() {
        let mapping = FieldMapping {
            fields: vec![
                FieldBinding::new("Hanzi", CardField::Front),
                FieldBinding::new("Meaning", CardField::Back),
                FieldBinding::new("Pinyin", CardField::Pinyin),
                FieldBinding::new("Level", CardField::HskLevel),
            ],
            audio_field: None,
        };
        let chinese = |pinyin: &str, hsk_level: Option<u8>| ScriptMetadata::Chinese {
            pinyin: pinyin.into(),
            hsk_level,
            script_variant: ScriptVariant::Simplified,
        };
        let cases = [
            ("xué xí", "HSK 1", chinese("xué xí", Some(1))),
            ("xué xí", "", chinese("xué xí", None)),
            ("", "4", chinese("", Some(4))),
            ("", "", ScriptMetadata::None),
            ("", "none", ScriptMetadata::None),
        ];
        for (pinyin, level, expected) in cases {
            let note = note_with(&[
                ("Hanzi", "学习"),
                ("Meaning", "lernen"),
                ("Pinyin", pinyin),
                ("Level", level),
            ]);
            let card = card_from_note(&note, &mapping, "zh-CN", "de", captured_at()).unwrap();
            assert_eq!(card.script_metadata, expected, "{pinyin:?}/{level:?}");
        }
    }

    #[test]
    fn card_from_note_keeps_the_exact_timestamp() {
        let note = note_with(&[("Front", "Haus"), ("Back", "house")]);
        let mapping = FieldMapping::apple_style_default();
        let precise = captured_at() + Duration::nanoseconds(456_789);
        let card = card_from_note(&note, &mapping, "de", "en", precise).unwrap();
        assert_eq!(card.captured_at, precise);
        assert_eq!(card_from_note(&note, &mapping, "de", "en", 42), None);
        assert_eq!(
            card_from_note(&note, &mapping, "de", "en", "yesterday"),
            None
        );
    }

    #[test]
    fn hsk_levels_parse_from_common_spellings() {
        let cases = [
            ("3", Some(3)),
            ("HSK 4", Some(4)),
            ("hsk5", Some(5)),
            (" 6 ", Some(6)),
            ("7", Some(7)),
            ("7-9", Some(7)),
            ("HSK 9", Some(7)),
            ("0", None),
            ("10", None),
            ("300", None),
            ("", None),
            ("advanced", None),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_hsk_level(text), expected, "{text:?}");
        }
    }
}
