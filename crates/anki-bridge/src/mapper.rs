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
//!
//! [`FieldMapping::auto_match`] builds a mapping from a note type's field
//! names alone (the keyword tables at the end of this file), and
//! [`FieldMapping::lost_values`] says what a push of a card would drop.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use vocab_core::{language::normalize_lang, Card, ScriptMetadata, ScriptVariant};

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
    /// `card.example_source` (the sentence in `card.source_lang`) on both
    /// notes: never swapped, so a field named after a language ("English
    /// Sentence") keeps that language on the reverse note too.
    ExampleInSourceLang,
    /// `card.example_target` (the sentence in `card.target_lang`) on both
    /// notes: never swapped.
    ExampleInTargetLang,
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
/// bindings name the same Anki field (compared case-insensitively, as
/// AnkiConnect compares field names), the first one wins.
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
        for binding in self.effective_bindings() {
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

    /// What a push of `card` onto a note type with `model_fields` would drop:
    /// the approval window's warning. Unlike [`missing_fields`](Self::missing_fields)
    /// it looks at the card, so an empty value is never reported. Only the
    /// forward note is checked; the reverse note writes the same fields.
    ///
    /// The result lists the [`LostValue::MissingField`] entries in mapping
    /// order, then [`LostValue::Unmapped`], then [`LostValue::Audio`].
    pub fn lost_values(&self, card: &Card, model_fields: &[String]) -> Vec<LostValue> {
        let has = |name: &str| {
            let lowered = name.to_lowercase();
            model_fields
                .iter()
                .any(|field| field.to_lowercase() == lowered)
        };
        let mut lost = Vec::new();
        let mut delivered: Vec<CardField> = Vec::new();
        let mut reported: Vec<CardField> = Vec::new();
        for binding in self.effective_bindings() {
            let value = field_value(card, Direction::Forward, binding.source);
            if has(&binding.anki_field) {
                delivered.push(binding.source);
            } else if !value.trim().is_empty() {
                lost.push(LostValue::MissingField {
                    anki_field: binding.anki_field.clone(),
                    source: binding.source,
                });
                reported.push(binding.source);
            }
        }
        // A value counts as written when any binding carries its content: an
        // example half also travels in its relative or combined field.
        let covers = |kind: CardField, set: &[CardField]| {
            set.iter().any(|&bound| match kind {
                CardField::ExampleInSourceLang => matches!(
                    bound,
                    CardField::ExampleInSourceLang | CardField::ExampleSource | CardField::Example
                ),
                CardField::ExampleInTargetLang => matches!(
                    bound,
                    CardField::ExampleInTargetLang | CardField::ExampleTarget | CardField::Example
                ),
                other => bound == other,
            })
        };
        let checks = [
            (CardField::Front, card.front.as_str()),
            (CardField::Back, card.back.as_str()),
            (CardField::ExampleInSourceLang, card.example_source.as_str()),
            (CardField::ExampleInTargetLang, card.example_target.as_str()),
            (CardField::Notes, card.notes.as_str()),
            (CardField::Pinyin, card.pinyin().unwrap_or_default()),
        ];
        for (kind, value) in checks {
            if !value.trim().is_empty() && !covers(kind, &delivered) && !covers(kind, &reported) {
                lost.push(LostValue::Unmapped { source: kind });
            }
        }
        if card
            .audio_front
            .as_ref()
            .is_some_and(|audio| !audio.is_empty())
        {
            let audio = self.audio_field();
            if !audio.is_some_and(|audio| model_fields.iter().any(|field| field == audio)) {
                lost.push(LostValue::Audio {
                    audio_field: audio.map(str::to_string),
                });
            }
        }
        lost
    }

    /// A mapping for a note type with `model_fields` (in `modelFieldNames`
    /// order) and cards `source_lang` -> `target_lang`, from the field names
    /// alone: "English Sentence" gets the English example, "Audio" the
    /// pronunciation, "Add Reverse" nothing. The rules and keyword tables are
    /// at the end of this file; both built-in mappings come out of their own
    /// field names exactly. Language codes are normalized first.
    pub fn auto_match(model_fields: &[String], source_lang: &str, target_lang: &str) -> AutoMatch {
        match_fields(
            model_fields,
            &normalize_lang(source_lang),
            &normalize_lang(target_lang),
        )
    }

    /// The bindings that take effect: the first one for each Anki field. Both
    /// directions use these, so import never reads a binding export skipped.
    fn effective_bindings(&self) -> impl Iterator<Item = &FieldBinding> {
        self.fields
            .iter()
            .enumerate()
            .filter(|&(index, binding)| {
                let name = binding.anki_field.to_lowercase();
                !self.fields[..index]
                    .iter()
                    .any(|earlier| earlier.anki_field.to_lowercase() == name)
            })
            .map(|(_, binding)| binding)
    }

    fn binds(&self, source: CardField) -> bool {
        self.effective_bindings()
            .any(|binding| binding.source == source)
    }
}

impl Default for FieldMapping {
    fn default() -> Self {
        Self::apple_style_default()
    }
}

/// A card value a push would not deliver ([`FieldMapping::lost_values`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LostValue {
    /// A binding whose Anki field the note type lacks, while the card has a
    /// value for it.
    MissingField {
        anki_field: String,
        source: CardField,
    },
    /// A non-empty card value no existing field receives (and no
    /// `MissingField` already reports).
    Unmapped { source: CardField },
    /// Audio bytes but no audio field, or one the note type lacks (compared
    /// exactly, as AnkiConnect attaches media).
    Audio { audio_field: Option<String> },
}

/// What [`FieldMapping::auto_match`] made of a note type's fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoMatch {
    /// Bindings in note-type order.
    pub mapping: FieldMapping,
    /// Fields bound only by position (no name matched): worth a look.
    pub guessed: Vec<String>,
    /// Fields left empty (the audio field excluded).
    pub unmatched: Vec<String>,
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
/// Anki fields hold HTML and card values are plain text, so every value has
/// `&`, `<` and `>` escaped and its line breaks written as `<br>`.
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
    let fields = mapping
        .effective_bindings()
        .map(|binding| {
            let value = field_value(card, direction, binding.source);
            (binding.anki_field.clone(), html::escape_text(&value))
        })
        .collect();
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
        CardField::ExampleInSourceLang => card.example_source.clone(),
        CardField::ExampleInTargetLang => card.example_target.clone(),
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
/// `ExampleSource`/`ExampleTarget` bindings are read as they are, or else
/// `ExampleInSourceLang`/`ExampleInTargetLang` ones (on a forward note both
/// name the same half); a half
/// without a binding of its own comes from the `Example` field: what is left
/// once the bound half is removed, or, with neither half bound, the text
/// before and after its first line break. `Pinyin`/`HskLevel` bindings
/// rebuild `ScriptMetadata::Chinese` when either has a value. The card gets
/// the note's tags, no audio, and the given languages (canonical codes) and
/// timestamp.
///
pub fn card_from_note(
    note: &NoteInfo,
    mapping: &FieldMapping,
    source_lang: &str,
    target_lang: &str,
    captured_at: DateTime<Utc>,
) -> Option<Card> {
    let read = |source: CardField| -> String {
        mapping
            .effective_bindings()
            .filter(|binding| binding.source == source)
            .filter_map(|binding| note.field(&binding.anki_field))
            .map(html::to_plain_text)
            .find(|value| !value.is_empty())
            .unwrap_or_default()
    };
    let read_bound = |source: CardField| mapping.binds(source).then(|| read(source));

    let front = read(CardField::Front);
    let back = read(CardField::Back);
    if front.is_empty() || back.is_empty() {
        return None;
    }
    let mut card = Card::new(front, back, source_lang, target_lang, captured_at);
    (card.example_source, card.example_target) = example_halves(
        read_bound(CardField::ExampleSource).or_else(|| read_bound(CardField::ExampleInSourceLang)),
        read_bound(CardField::ExampleTarget).or_else(|| read_bound(CardField::ExampleInTargetLang)),
        &read(CardField::Example),
    );
    card.notes = read(CardField::Notes);
    card.tags = note.tags.clone();
    card.script_metadata = chinese_metadata(read(CardField::Pinyin), &read(CardField::HskLevel));
    Some(card)
}

/// (example_source, example_target) from the explicitly bound halves (`None`
/// when unbound) and the combined `Example` field. map_card writes `Example`
/// as the non-blank halves joined by `\n`, so a known half is removed from
/// it; without a known half the split falls at the first line break.
fn example_halves(
    source: Option<String>,
    target: Option<String>,
    example: &str,
) -> (String, String) {
    match (source, target) {
        (Some(source), Some(target)) => (source, target),
        (Some(source), None) => {
            let target =
                remove_half(example, &source, true).unwrap_or_else(|| split_example(example).1);
            (source, target)
        }
        (None, Some(target)) => {
            let source =
                remove_half(example, &target, false).unwrap_or_else(|| split_example(example).0);
            (source, target)
        }
        (None, None) => split_example(example),
    }
}

/// `example` without its `known` half (the first half when `known_first`),
/// or `None` when `known` is not where map_card would have put it.
fn remove_half(example: &str, known: &str, known_first: bool) -> Option<String> {
    if known.is_empty() {
        return Some(example.to_string());
    }
    if example == known {
        return Some(String::new());
    }
    let rest = if known_first {
        example.strip_prefix(known)?.strip_prefix('\n')?
    } else {
        example.strip_suffix(known)?.strip_suffix('\n')?
    };
    Some(rest.to_string())
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

// ---------------------------------------------------------------------------
// Matching a note type's fields by name (FieldMapping::auto_match)

/// Words that name each supported language inside an Anki field name, after
/// [`fold`]. German adjective endings (-e/-er/-es/-en/-em) are accepted on
/// the longer ones ("Englischer Satz", "Deutsche Übersetzung"); two- and
/// three-letter codes only as the last word ("Sentence (EN)", "satz_de").
const LANGUAGE_WORDS: &[(&str, &[&str], &[&str])] = &[
    (
        "en",
        &["english", "englisch", "anglais", "ingles", "inglese"],
        &["en", "eng"],
    ),
    (
        "de",
        &["german", "deutsch", "allemand", "aleman", "tedesco"],
        &["de", "ger", "deu"],
    ),
    (
        "fr",
        &[
            "french",
            "franzosisch",
            "franzoesisch",
            "francais",
            "frances",
            "francese",
        ],
        &["fr", "fra", "fre"],
    ),
    (
        "es",
        &["spanish", "spanisch", "espagnol", "espanol", "spagnolo"],
        &["es", "spa"],
    ),
    (
        "it",
        &["italian", "italienisch", "italien", "italiano"],
        &["it", "ita"],
    ),
    (
        "zh-CN",
        &[
            "chinese",
            "chinesisch",
            "chinois",
            "chino",
            "cinese",
            "mandarin",
            "hanzi",
            "zhongwen",
        ],
        &["zh", "chn", "cn"],
    ),
];
/// CJK spellings, matched as substrings of the raw name.
const LANGUAGE_CJK: &[(&str, &[&str])] =
    &[("zh-CN", &["中文", "汉语", "漢語", "汉字", "漢字", "普通话"])];

const AUDIO: &[&str] = &[
    "audio",
    "sound",
    "sounds",
    "pronunciation",
    "prononciation",
    "aussprache",
    "voice",
    "stimme",
    "tts",
    "mp3",
    "recording",
    "aufnahme",
];
const AUDIO_CJK: &[&str] = &["发音", "音频", "录音"];
const NOTES: &[&str] = &[
    "notes",
    "note",
    "notizen",
    "notiz",
    "anmerkung",
    "anmerkungen",
    "hinweis",
    "hinweise",
    "comment",
    "comments",
    "kommentar",
    "kommentare",
    "bemerkung",
    "bemerkungen",
    "remarks",
    "remarque",
    "remarques",
    "extra",
    "zusatz",
    "zusatzinfo",
    "info",
    "misc",
    "sonstiges",
];
const NOTES_CJK: &[&str] = &["备注", "注释"];
const EXAMPLE: &[&str] = &[
    "sentence",
    "sentences",
    "satz",
    "satze",
    "beispiel",
    "beispiele",
    "beispielsatz",
    "beispielsatze",
    "example",
    "examples",
    "exemple",
    "exemples",
    "phrase",
    "context",
    "kontext",
    "usage",
    "ejemplo",
    "frase",
    "esempio",
];
const EXAMPLE_CJK: &[&str] = &["例句", "句子"];
const TRANSLATION: &[&str] = &[
    "translation",
    "translated",
    "ubersetzung",
    "uebersetzung",
    "ubersetzt",
    "traduction",
    "traduccion",
    "traduzione",
    "meaning",
    "bedeutung",
];
const FRONT: &[&str] = &[
    "front",
    "vorderseite",
    "vorne",
    "recto",
    "question",
    "frage",
    "prompt",
];
const BACK: &[&str] = &[
    "back",
    "ruckseite",
    "rueckseite",
    "hinten",
    "verso",
    "answer",
    "antwort",
];
const WORD: &[&str] = &[
    "word",
    "wort",
    "vokabel",
    "vocab",
    "vocabulary",
    "term",
    "begriff",
    "expression",
    "ausdruck",
    "lemma",
    "headword",
    "mot",
    "palabra",
    "parola",
    "characters",
    "zeichen",
    "simplified",
];
const WORD_CJK: &[&str] = &["单词", "生词", "词语"];
const MEANING: &[&str] = &[
    "meaning",
    "meanings",
    "bedeutung",
    "translation",
    "ubersetzung",
    "uebersetzung",
    "definition",
    "gloss",
    "traduction",
    "signification",
    "significado",
    "significato",
];
const MEANING_CJK: &[&str] = &["意思", "释义", "翻译"];
const PINYIN: &[&str] = &["pinyin"];
/// Switches in the note type's own templates ("Add Reverse" in Anki's
/// "Basic (optional reversed card)"): filling one adds cards.
const NEVER: &[&str] = &["reverse", "reversed", "umgekehrt", "umkehren", "tags"];
const PINYIN_CJK: &[&str] = &["拼音"];
const HSK: &[&str] = &["hsk"];
/// Level words count as the HSK level only on a Chinese capture.
const LEVEL: &[&str] = &["level", "stufe", "niveau"];

/// Lowercase with accents and umlauts folded (a fixed table, so a port to
/// another language folds identically without Unicode normalization).
fn fold(c: char) -> String {
    match c {
        'ä' | 'à' | 'á' | 'â' | 'ã' | 'å' => "a".into(),
        'ö' | 'ò' | 'ó' | 'ô' | 'õ' => "o".into(),
        'ü' | 'ù' | 'ú' | 'û' => "u".into(),
        'é' | 'è' | 'ê' | 'ë' => "e".into(),
        'í' | 'ì' | 'î' | 'ï' => "i".into(),
        'ç' => "c".into(),
        'ñ' => "n".into(),
        'ß' => "ss".into(),
        other => other.to_string(),
    }
}

/// The words of a field name: camelCase and every non-alphanumeric split,
/// lowercased and folded. "EnglishSentence" -> ["english", "sentence"],
/// "Rückseite" -> ["ruckseite"], "satz_de" -> ["satz", "de"].
fn words(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && prev_lower && !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            prev_lower = c.is_lowercase();
            for l in c.to_lowercase() {
                current.push_str(&fold(l));
            }
        } else {
            prev_lower = false;
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// A field name, raw (for the CJK keywords) and as [`words`].
struct Name<'a> {
    raw: &'a str,
    words: Vec<String>,
}

impl Name<'_> {
    fn has(&self, list: &[&str]) -> bool {
        self.words.iter().any(|w| list.contains(&w.as_str()))
    }

    fn has_cjk(&self, list: &[&str]) -> bool {
        list.iter().any(|k| self.raw.contains(k))
    }

    /// German compounds: "Beispielsatz", "Kontextsatz" (not "Zusatz").
    fn has_example(&self) -> bool {
        self.has(EXAMPLE)
            || self.has_cjk(EXAMPLE_CJK)
            || self.words.iter().any(|w| {
                (w.ends_with("satz") && w != "zusatz")
                    || w.ends_with("beispiel")
                    || w.ends_with("satze")
            })
    }

    fn language(&self) -> Option<&'static str> {
        for (code, names, codes) in LANGUAGE_WORDS {
            let named = self.words.iter().any(|w| {
                names.iter().any(|n| {
                    w == n
                        || (n.len() > 4
                            && w.strip_prefix(n)
                                .is_some_and(|rest| ["e", "er", "es", "en", "em"].contains(&rest)))
                })
            });
            let coded = self.words.len() > 1
                && self
                    .words
                    .last()
                    .is_some_and(|w| codes.contains(&w.as_str()));
            if named || coded {
                return Some(code);
            }
        }
        LANGUAGE_CJK
            .iter()
            .find(|(_, spellings)| self.has_cjk(spellings))
            .map(|(code, _)| *code)
    }
}

/// What one field name looks like, before the one-binding-per-value rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Guess {
    Audio {
        word_audio: bool,
    },
    Text(CardField),
    /// An example field naming no language or side.
    GenericExample,
    /// Names a third language: leave it empty.
    OtherLanguage,
    /// Never filled, not even by position ("Add Reverse" adds a card).
    Never,
    Unknown,
}

/// The first rule that applies to `name` wins; `source` and `target` are
/// canonical codes.
fn classify(name: &Name, source: &str, target: &str) -> Guess {
    let lang = name.language();
    let side = |code: &str| {
        if code == source {
            Some(true)
        } else if code == target {
            Some(false)
        } else {
            None
        }
    };
    if name.has(NEVER) {
        return Guess::Never;
    }
    if name.has(AUDIO) || name.has_cjk(AUDIO_CJK) {
        // The audio is TTS of the note's front word; a sentence, back or
        // target-language audio field would get the wrong recording.
        let word_audio =
            !name.has_example() && !name.has(BACK) && lang.is_none_or(|code| code == source);
        return Guess::Audio { word_audio };
    }
    // Before examples, so "Zusatz" is not read as a sentence.
    if name.has(NOTES) || name.has_cjk(NOTES_CJK) {
        return Guess::Text(CardField::Notes);
    }
    if name.has_example() {
        return match lang.map(side) {
            Some(Some(true)) => Guess::Text(CardField::ExampleInSourceLang),
            Some(Some(false)) => Guess::Text(CardField::ExampleInTargetLang),
            Some(None) => Guess::OtherLanguage,
            None if name.has(TRANSLATION) => Guess::Text(CardField::ExampleInTargetLang),
            None if name.has(FRONT) => Guess::Text(CardField::ExampleSource),
            None if name.has(BACK) => Guess::Text(CardField::ExampleTarget),
            None => Guess::GenericExample,
        };
    }
    if name.has(PINYIN) || name.has_cjk(PINYIN_CJK) {
        return Guess::Text(CardField::Pinyin);
    }
    let chinese = source == "zh-CN" || target == "zh-CN";
    if name.has(HSK) || (chinese && name.has(LEVEL)) {
        return Guess::Text(CardField::HskLevel);
    }
    if name.has(FRONT) {
        return Guess::Text(CardField::Front);
    }
    if name.has(BACK) {
        return Guess::Text(CardField::Back);
    }
    if let Some(code) = lang {
        return match side(code) {
            Some(true) => Guess::Text(CardField::Front),
            Some(false) => Guess::Text(CardField::Back),
            None => Guess::OtherLanguage,
        };
    }
    if name.has(MEANING) || name.has_cjk(MEANING_CJK) {
        return Guess::Text(CardField::Back);
    }
    if name.has(WORD) || name.has_cjk(WORD_CJK) {
        return Guess::Text(CardField::Front);
    }
    Guess::Unknown
}

/// [`FieldMapping::auto_match`] for canonical `source`/`target` codes.
fn match_fields(model_fields: &[String], source: &str, target: &str) -> AutoMatch {
    let guesses: Vec<Guess> = model_fields
        .iter()
        .map(|raw| {
            let name = Name {
                raw,
                words: words(raw),
            };
            classify(&name, source, target)
        })
        .collect();

    // First field wins: a value goes to the first field that names it.
    let mut chosen: Vec<Option<CardField>> = vec![None; model_fields.len()];
    let taken = |field: CardField, chosen: &[Option<CardField>]| chosen.contains(&Some(field));
    for (i, guess) in guesses.iter().enumerate() {
        if let Guess::Text(field) = guess {
            if !taken(*field, &chosen) {
                chosen[i] = Some(*field);
            }
        }
    }
    // Generic example fields: the combined two-line Example when it is the
    // note type's only example field (what both presets do), otherwise the
    // captured-language half first, then the translation half.
    let example_fields = guesses
        .iter()
        .filter(|g| {
            matches!(
                g,
                Guess::GenericExample
                    | Guess::Text(
                        CardField::ExampleInSourceLang
                            | CardField::ExampleInTargetLang
                            | CardField::ExampleSource
                            | CardField::ExampleTarget
                    )
            )
        })
        .count();
    for (i, guess) in guesses.iter().enumerate() {
        if *guess != Guess::GenericExample {
            continue;
        }
        let pick = if example_fields == 1 {
            Some(CardField::Example)
        } else {
            [
                CardField::ExampleInSourceLang,
                CardField::ExampleInTargetLang,
            ]
            .into_iter()
            .find(|f| !taken(*f, &chosen))
        };
        chosen[i] = pick;
    }
    // Positional fallback: Anki refuses a note whose first field is empty,
    // and a note type always has a question and an answer side.
    let mut guessed = Vec::new();
    for field in [CardField::Front, CardField::Back] {
        if taken(field, &chosen) {
            continue;
        }
        let free = (0..model_fields.len())
            .find(|&i| chosen[i].is_none() && matches!(guesses[i], Guess::Unknown));
        if let Some(i) = free {
            chosen[i] = Some(field);
            guessed.push(model_fields[i].clone());
        }
    }
    let audio_field = guesses
        .iter()
        .position(|g| matches!(g, Guess::Audio { word_audio: true }))
        .map(|i| model_fields[i].clone());

    let fields: Vec<FieldBinding> = model_fields
        .iter()
        .zip(&chosen)
        .filter_map(|(name, field)| field.map(|f| FieldBinding::new(name.clone(), f)))
        .collect();
    let unmatched = model_fields
        .iter()
        .zip(&chosen)
        .filter(|(name, field)| field.is_none() && audio_field.as_deref() != Some(name.as_str()))
        .map(|(name, _)| name.clone())
        .collect();
    AutoMatch {
        mapping: FieldMapping {
            fields,
            audio_field,
        },
        guessed,
        unmatched,
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
                        "It was pure serendipity.<br>Es war reiner Zufall.",
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
                        "Es war reiner Zufall.<br>It was pure serendipity.",
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
                        "It was pure serendipity.<br>Es war reiner Zufall.",
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
                        "It was pure serendipity.<br>Es war reiner Zufall.",
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
                        "Es war reiner Zufall.<br>It was pure serendipity.",
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
                        "It was pure serendipity.<br>Es war reiner Zufall.",
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
                        "我们一起学习中文。<br>Wir lernen zusammen Chinesisch.",
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
                        "Wir lernen zusammen Chinesisch.<br>我们一起学习中文。",
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
                        "It was pure serendipity.<br>Es war reiner Zufall.",
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
                    ("E", "我们一起学习中文。<br>Wir lernen zusammen Chinesisch."),
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
                    ("E", "Es war reiner Zufall.<br>It was pure serendipity."),
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

    /// A note type with "English Sentence" and "German Sentence" fields: the
    /// reverse note swaps the sides, but each sentence stays in the field
    /// named after its language.
    #[test]
    fn language_bound_examples_are_never_swapped() {
        let card = english_card();
        for direction in [Direction::Forward, Direction::Reverse] {
            assert_eq!(
                field_value(&card, direction, CardField::ExampleInSourceLang),
                "It was pure serendipity.",
                "{direction:?}"
            );
            assert_eq!(
                field_value(&card, direction, CardField::ExampleInTargetLang),
                "Es war reiner Zufall.",
                "{direction:?}"
            );
        }
        // The relative halves do swap, which is what language-named fields
        // must not do.
        assert_eq!(
            field_value(&card, Direction::Reverse, CardField::ExampleSource),
            "Es war reiner Zufall."
        );

        let mapping = FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("English Sentence", CardField::ExampleInSourceLang),
                FieldBinding::new("German Sentence", CardField::ExampleInTargetLang),
            ],
            audio_field: Some("Audio".into()),
        };
        let reverse = map_card(&card, Direction::Reverse, &target(), &mapping, None);
        assert_eq!(
            reverse.fields,
            BTreeMap::from([
                ("Front".to_string(), "glücklicher Zufall".to_string()),
                ("Back".to_string(), "serendipity".to_string()),
                (
                    "English Sentence".to_string(),
                    "It was pure serendipity.".to_string()
                ),
                (
                    "German Sentence".to_string(),
                    "Es war reiner Zufall.".to_string()
                ),
            ])
        );
    }

    #[test]
    fn card_from_note_reads_language_bound_examples() {
        let mapping = FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("English Sentence", CardField::ExampleInSourceLang),
                FieldBinding::new("German Sentence", CardField::ExampleInTargetLang),
                FieldBinding::new("Notes", CardField::Notes),
            ],
            audio_field: Some("Audio".into()),
        };
        let card = english_card();
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
        let mut expected = card;
        expected.audio_front = None;
        assert_eq!(back, Some(expected));

        // One language-bound half next to the combined field: the other half
        // still comes out of `Example`.
        let mapping = FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("Sentence", CardField::Example),
                FieldBinding::new("German Sentence", CardField::ExampleInTargetLang),
            ],
            audio_field: None,
        };
        let mut card = english_card();
        card.notes.clear();
        card.audio_front = None;
        let spec = map_card(&card, Direction::Forward, &target(), &mapping, None);
        let back = card_from_note(&stored(&spec), &mapping, "en", "de", card.captured_at);
        assert_eq!(back, Some(card));
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
            // Same text and order as `Card::combined_example`; only the line
            // break is spelled the HTML way.
            assert_eq!(
                note.fields["E"],
                card.combined_example().replace('\n', "<br>"),
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

    /// AnkiConnect matches field names case-insensitively, so `front` is the
    /// same Anki field as `Front`: sending both would let the later JSON key
    /// overwrite the first binding inside Anki.
    #[test]
    fn first_binding_wins_case_insensitively() {
        let mapping = FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("front", CardField::Pinyin),
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
        assert_eq!(
            note.fields,
            BTreeMap::from([
                ("Front".to_string(), "学习".to_string()),
                ("Back".to_string(), "lernen".to_string()),
            ])
        );
    }

    /// Anki fields are HTML. Card text is plain text, so markup-like text is
    /// escaped on the way out (or Anki would render `<b>`, drop `<String>`
    /// and run a pasted `<script>`) and decoded again on import.
    #[test]
    fn text_is_escaped_for_anki_s_html_fields() {
        let mut card = english_card();
        card.front = "Vec<String>".into();
        card.back = "R&D <Abteilung>".into();
        card.example_source = "Use Vec<String> & co.".into();
        card.example_target = "<b>Beispiel</b>".into();
        card.notes = "<script>alert(1)</script>".into();
        let mapping = every_field_mapping();

        let forward = map_card(&card, Direction::Forward, &target(), &mapping, None);
        let reverse = map_card(&card, Direction::Reverse, &target(), &mapping, None);

        let fields = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect()
        };
        assert_eq!(
            forward.fields,
            fields(&[
                ("F", "Vec&lt;String&gt;"),
                ("B", "R&amp;D &lt;Abteilung&gt;"),
                ("ES", "Use Vec&lt;String&gt; &amp; co."),
                ("ET", "&lt;b&gt;Beispiel&lt;/b&gt;"),
                (
                    "E",
                    "Use Vec&lt;String&gt; &amp; co.<br>&lt;b&gt;Beispiel&lt;/b&gt;"
                ),
                ("N", "&lt;script&gt;alert(1)&lt;/script&gt;"),
                ("P", ""),
                ("H", ""),
            ])
        );
        assert_eq!(forward.fields["F"], reverse.fields["B"]);
        assert_eq!(forward.fields["ES"], reverse.fields["ET"]);
        assert_eq!(
            reverse.fields["E"],
            "&lt;b&gt;Beispiel&lt;/b&gt;<br>Use Vec&lt;String&gt; &amp; co."
        );

        let back = card_from_note(
            &stored(&forward),
            &mapping,
            &card.source_lang,
            &card.target_lang,
            card.captured_at,
        );
        card.audio_front = None;
        assert_eq!(back, Some(card));
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
                        "Context Sentence": "It was pure serendipity.<br>Es war reiner Zufall.",
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
                { "anki_field": "h", "source": "hsk_level" },
                { "anki_field": "i", "source": "example_in_source_lang" },
                { "anki_field": "j", "source": "example_in_target_lang" }
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
                CardField::ExampleInSourceLang,
                CardField::ExampleInTargetLang,
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
        // A binding shadowed by an earlier one for the same field is not sent,
        // so it is not reported a second time under another spelling.
        let shadowed = FieldMapping {
            fields: vec![
                FieldBinding::new("Word", CardField::Front),
                FieldBinding::new("WORD", CardField::Pinyin),
            ],
            audio_field: None,
        };
        assert_eq!(shadowed.missing_fields(&names(&["Back"])), names(&["Word"]));
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

    /// A note type with a combined example field plus a field for one of its
    /// halves: the other half comes out of the combined field, so the round
    /// trip holds for every example shape map_card can write, including a
    /// single half and a sentence that spans two lines.
    #[test]
    fn card_from_note_derives_an_unbound_example_half_from_example() {
        let with_one_half = |half: CardField, name: &str| FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("Sentence", CardField::Example),
                FieldBinding::new(name, half),
            ],
            audio_field: None,
        };
        let mappings = [
            with_one_half(CardField::ExampleTarget, "Translation"),
            with_one_half(CardField::ExampleSource, "Original"),
        ];
        let examples = [
            ("It was pure serendipity.", "Es war reiner Zufall."),
            ("It was pure serendipity.", ""),
            ("", "Es war reiner Zufall."),
            ("", ""),
            ("Line one.\nLine two.", "Zeile eins.\nZeile zwei."),
            ("Line one.\nLine two.", ""),
            ("", "Zeile eins.\nZeile zwei."),
        ];
        for mapping in &mappings {
            for (source, target_text) in examples {
                let mut card = english_card();
                card.example_source = source.into();
                card.example_target = target_text.into();
                card.notes.clear(); // not mapped
                card.audio_front = None;
                let spec = map_card(&card, Direction::Forward, &target(), mapping, None);
                let back = card_from_note(
                    &stored(&spec),
                    mapping,
                    &card.source_lang,
                    &card.target_lang,
                    card.captured_at,
                );
                assert_eq!(
                    back,
                    Some(card),
                    "{:?}: {source:?}/{target_text:?}",
                    mapping.fields[3].source
                );
            }
        }
    }

    /// map_card writes only the first binding for an Anki field, so import
    /// must not read a shadowed binding from the winner's content.
    #[test]
    fn card_from_note_ignores_shadowed_bindings() {
        for shadowed in ["Front", "FRONT"] {
            let mapping = FieldMapping {
                fields: vec![
                    FieldBinding::new("Front", CardField::Front),
                    FieldBinding::new("Back", CardField::Back),
                    FieldBinding::new(shadowed, CardField::Pinyin),
                ],
                audio_field: None,
            };
            let spec = map_card(
                &chinese_card(),
                Direction::Forward,
                &target(),
                &mapping,
                None,
            );
            let card =
                card_from_note(&stored(&spec), &mapping, "zh-CN", "de", captured_at()).unwrap();
            assert_eq!(card.front, "学习", "{shadowed}");
            assert_eq!(card.script_metadata, ScriptMetadata::None, "{shadowed}");
        }
    }

    #[test]
    fn card_from_note_takes_the_first_non_empty_of_several_bindings() {
        let mapping = FieldMapping {
            fields: vec![
                FieldBinding::new("Word", CardField::Front),
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
            ],
            audio_field: None,
        };
        let note = note_with(&[("Word", "<br>"), ("Front", "Haus"), ("Back", "house")]);
        let card = card_from_note(&note, &mapping, "de", "en", captured_at()).unwrap();
        assert_eq!(card.front, "Haus");
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

/// `FieldMapping::auto_match` and `lost_values`, with the user's
/// AppleStyleFinalV3 note type (Front, Back, English Sentence, German
/// Sentence, Audio) as the main case.
#[cfg(test)]
mod auto_match_tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn apple_v3() -> Vec<String> {
        names(&[
            "Front",
            "Back",
            "English Sentence",
            "German Sentence",
            "Audio",
        ])
    }

    fn english_card() -> Card {
        let captured_at = Utc.with_ymd_and_hms(2026, 11, 4, 12, 0, 0).unwrap();
        let mut card = Card::new("serendipity", "glücklicher Zufall", "en", "de", captured_at);
        card.example_source = "It was pure serendipity.".into();
        card.example_target = "Es war reiner Zufall.".into();
        card.audio_front = Some(b"ID3".to_vec());
        card
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// Anki field -> value, in mapping order, as map_card would send it
    /// (without the HTML escaping).
    fn map_fields(
        card: &Card,
        direction: Direction,
        mapping: &FieldMapping,
    ) -> Vec<(String, String)> {
        mapping
            .effective_bindings()
            .map(|b| (b.anki_field.clone(), field_value(card, direction, b.source)))
            .collect()
    }

    fn auto_match(model_fields: &[String], source_lang: &str, target_lang: &str) -> AutoMatch {
        FieldMapping::auto_match(model_fields, source_lang, target_lang)
    }

    fn lost_values(card: &Card, mapping: &FieldMapping, model_fields: &[String]) -> Vec<LostValue> {
        mapping.lost_values(card, model_fields)
    }

    fn unmapped(source: CardField) -> LostValue {
        LostValue::Unmapped { source }
    }

    #[test]
    fn apple_style_final_v3_en_de() {
        let m = auto_match(&apple_v3(), "en", "de");
        assert_eq!(
            m.mapping,
            FieldMapping {
                fields: vec![
                    FieldBinding::new("Front", CardField::Front),
                    FieldBinding::new("Back", CardField::Back),
                    FieldBinding::new("English Sentence", CardField::ExampleInSourceLang),
                    FieldBinding::new("German Sentence", CardField::ExampleInTargetLang),
                ],
                audio_field: Some("Audio".into()),
            }
        );
        assert!(m.unmatched.is_empty() && m.guessed.is_empty(), "{m:?}");
        assert!(m.mapping.missing_fields(&apple_v3()).is_empty());

        let card = english_card();
        assert_eq!(
            map_fields(&card, Direction::Forward, &m.mapping),
            pairs(&[
                ("Front", "serendipity"),
                ("Back", "glücklicher Zufall"),
                ("English Sentence", "It was pure serendipity."),
                ("German Sentence", "Es war reiner Zufall."),
            ])
        );
        // The reverse note: sides swap, language-named sentences do not.
        assert_eq!(
            map_fields(&card, Direction::Reverse, &m.mapping),
            pairs(&[
                ("Front", "glücklicher Zufall"),
                ("Back", "serendipity"),
                ("English Sentence", "It was pure serendipity."),
                ("German Sentence", "Es war reiner Zufall."),
            ])
        );
        assert_eq!(lost_values(&card, &m.mapping, &apple_v3()), vec![]);
    }

    /// Why relative ExampleSource/ExampleTarget are wrong for these names.
    #[test]
    fn relative_bindings_swap_language_named_fields_on_the_reverse_note() {
        let relative = FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("English Sentence", CardField::ExampleSource),
                FieldBinding::new("German Sentence", CardField::ExampleTarget),
            ],
            audio_field: Some("Audio".into()),
        };
        let reverse = map_fields(&english_card(), Direction::Reverse, &relative);
        assert_eq!(
            reverse[2],
            ("English Sentence".into(), "Es war reiner Zufall.".into())
        );
    }

    /// The user's report: the old default mapping on their note type. The
    /// example is reported once, as the missing "Context Sentence".
    #[test]
    fn today_s_default_mapping_loses_example_and_audio_on_apple_v3() {
        let default = FieldMapping::apple_style_default();
        assert_eq!(
            default.missing_fields(&apple_v3()),
            names(&["Context Sentence", "Notes", "Front Audio"])
        );
        let card = english_card();
        assert_eq!(
            lost_values(&card, &default, &apple_v3()),
            vec![
                LostValue::MissingField {
                    anki_field: "Context Sentence".into(),
                    source: CardField::Example
                },
                LostValue::Audio {
                    audio_field: Some("Front Audio".into())
                },
            ]
        );
    }

    #[test]
    fn french_card_on_apple_v3_leaves_the_english_sentence_empty() {
        let m = auto_match(&apple_v3(), "fr", "de");
        assert_eq!(
            m.mapping.fields,
            vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("German Sentence", CardField::ExampleInTargetLang),
            ]
        );
        assert_eq!(m.mapping.audio_field.as_deref(), Some("Audio"));
        assert_eq!(m.unmatched, names(&["English Sentence"]));
        let mut card = english_card();
        card.source_lang = "fr".into();
        card.front = "message".into();
        card.example_source = "J'ai reçu ton message.".into();
        assert_eq!(
            lost_values(&card, &m.mapping, &apple_v3()),
            vec![unmapped(CardField::ExampleInSourceLang)]
        );
    }

    #[test]
    fn german_source_flips_which_sentence_is_the_captured_one() {
        let m = auto_match(&apple_v3(), "de", "en");
        assert_eq!(
            m.mapping.fields[2],
            FieldBinding::new("English Sentence", CardField::ExampleInTargetLang)
        );
        assert_eq!(
            m.mapping.fields[3],
            FieldBinding::new("German Sentence", CardField::ExampleInSourceLang)
        );
    }

    /// Why the Automator matches a global mapping nobody customized by field
    /// name: on the classic note type the match is exactly the old default,
    /// for every language pair a capture can have, so those users' notes stay
    /// as they were.
    #[test]
    fn the_classic_note_type_matches_the_old_default_for_every_pair() {
        let classic = names(&["Front", "Back", "Context Sentence", "Notes", "Front Audio"]);
        let langs = vocab_core::language::SUPPORTED_LANGS;
        let mut pairs = 0;
        for source in langs {
            for target in langs.iter().filter(|target| *target != source) {
                let m = auto_match(&classic, source, target);
                assert_eq!(
                    m.mapping,
                    FieldMapping::apple_style_default(),
                    "{source} -> {target}"
                );
                assert!(
                    m.guessed.is_empty() && m.unmatched.is_empty(),
                    "{source} -> {target}: {m:?}"
                );
                pairs += 1;
            }
        }
        assert_eq!(pairs, 30);
    }

    #[test]
    fn reproduces_both_presets_from_their_field_names() {
        let apple = names(&["Front", "Back", "Context Sentence", "Notes", "Front Audio"]);
        let m = auto_match(&apple, "en", "de");
        assert_eq!(m.mapping, FieldMapping::apple_style_default());
        let chinese = names(&["Hanzi", "Pinyin", "Meaning", "Sentence", "Audio"]);
        let m = auto_match(&chinese, "zh-CN", "de");
        assert_eq!(m.mapping, FieldMapping::chinese_default());
    }

    #[test]
    fn german_field_names() {
        let model = names(&[
            "Vorderseite",
            "Rückseite",
            "Englischer Satz",
            "Deutscher Satz",
            "Aussprache",
            "Notizen",
        ]);
        let m = auto_match(&model, "en", "de");
        assert_eq!(
            m.mapping,
            FieldMapping {
                fields: vec![
                    FieldBinding::new("Vorderseite", CardField::Front),
                    FieldBinding::new("Rückseite", CardField::Back),
                    FieldBinding::new("Englischer Satz", CardField::ExampleInSourceLang),
                    FieldBinding::new("Deutscher Satz", CardField::ExampleInTargetLang),
                    FieldBinding::new("Notizen", CardField::Notes),
                ],
                audio_field: Some("Aussprache".into()),
            }
        );
    }

    #[test]
    fn chinese_note_type_with_german_names() {
        let model = names(&[
            "Hanzi",
            "Pinyin",
            "Bedeutung",
            "Beispielsatz",
            "Beispiel Übersetzung",
            "HSK",
            "Audio",
            "Zusatz",
        ]);
        let m = auto_match(&model, "zh-CN", "de");
        assert_eq!(
            m.mapping.fields,
            vec![
                FieldBinding::new("Hanzi", CardField::Front),
                FieldBinding::new("Pinyin", CardField::Pinyin),
                FieldBinding::new("Bedeutung", CardField::Back),
                FieldBinding::new("Beispielsatz", CardField::ExampleInSourceLang),
                FieldBinding::new("Beispiel Übersetzung", CardField::ExampleInTargetLang),
                FieldBinding::new("HSK", CardField::HskLevel),
                FieldBinding::new("Zusatz", CardField::Notes),
            ]
        );
        assert_eq!(m.mapping.audio_field.as_deref(), Some("Audio"));
    }

    #[test]
    fn basic_note_types() {
        let m = auto_match(&names(&["Front", "Back"]), "en", "de");
        assert_eq!(m.mapping.fields.len(), 2);
        assert_eq!(m.mapping.audio_field, None);
        // "Add Reverse" must stay empty, or Anki would add the reverse card.
        let m = auto_match(&names(&["Front", "Back", "Add Reverse"]), "en", "de");
        assert_eq!(m.unmatched, names(&["Add Reverse"]));
        // Not even by position.
        let m = auto_match(&names(&["Word", "Add Reverse"]), "en", "de");
        assert_eq!(
            m.mapping.fields,
            vec![FieldBinding::new("Word", CardField::Front)]
        );
        assert_eq!(m.unmatched, names(&["Add Reverse"]));
    }

    #[test]
    fn language_named_word_fields_and_codes() {
        let m = auto_match(
            &names(&[
                "English",
                "Deutsch",
                "Sentence (EN)",
                "Satz_DE",
                "EnglishAudio",
            ]),
            "en",
            "de",
        );
        assert_eq!(
            m.mapping,
            FieldMapping {
                fields: vec![
                    FieldBinding::new("English", CardField::Front),
                    FieldBinding::new("Deutsch", CardField::Back),
                    FieldBinding::new("Sentence (EN)", CardField::ExampleInSourceLang),
                    FieldBinding::new("Satz_DE", CardField::ExampleInTargetLang),
                ],
                audio_field: Some("EnglishAudio".into()),
            }
        );
    }

    #[test]
    fn sentence_audio_is_not_word_audio() {
        let m = auto_match(
            &names(&["Word", "Meaning", "Sentence", "Sentence Audio"]),
            "en",
            "de",
        );
        assert_eq!(m.mapping.audio_field, None);
        assert_eq!(m.unmatched, names(&["Sentence Audio"]));
        let m = auto_match(
            &names(&["Word", "Meaning", "Sentence Audio", "Word Audio"]),
            "en",
            "de",
        );
        assert_eq!(m.mapping.audio_field.as_deref(), Some("Word Audio"));
    }

    #[test]
    fn unknown_names_fall_back_to_position() {
        let m = auto_match(&names(&["Q", "A", "Sound"]), "en", "de");
        assert_eq!(
            m.mapping,
            FieldMapping {
                fields: vec![
                    FieldBinding::new("Q", CardField::Front),
                    FieldBinding::new("A", CardField::Back),
                ],
                audio_field: Some("Sound".into()),
            }
        );
        assert_eq!(m.guessed, names(&["Q", "A"]));
    }

    #[test]
    fn first_field_wins_for_a_repeated_value() {
        let m = auto_match(&names(&["Front", "Word", "Back"]), "en", "de");
        assert_eq!(m.mapping.fields.len(), 2);
        assert_eq!(m.unmatched, names(&["Word"]));
    }

    #[test]
    fn words_split_and_fold() {
        assert_eq!(words("EnglishSentence"), ["english", "sentence"]);
        assert_eq!(words("Rückseite"), ["ruckseite"]);
        assert_eq!(words("satz_de"), ["satz", "de"]);
        assert_eq!(words("Front Audio"), ["front", "audio"]);
        assert_eq!(words("Français"), ["francais"]);
        assert_eq!(words("HSK"), ["hsk"]);
    }

    #[test]
    fn auto_match_normalizes_its_language_codes() {
        let chinese = names(&["Hanzi", "Pinyin", "Meaning", "Sentence", "Level", "Audio"]);
        let m = auto_match(&chinese, "ZH", "DE");
        assert_eq!(
            m.mapping.fields.last(),
            Some(&FieldBinding::new("Level", CardField::HskLevel)),
            "a level field counts only on a Chinese capture"
        );
        let m = auto_match(&apple_v3(), " EN ", "De");
        assert_eq!(m, auto_match(&apple_v3(), "en", "de"));
    }

    #[test]
    fn lost_values_reports_only_values_the_card_has() {
        let auto = auto_match(&apple_v3(), "en", "de").mapping;
        assert_eq!(lost_values(&english_card(), &auto, &apple_v3()), vec![]);

        // Pinyin on a Chinese card with the apple mapping has nowhere to go.
        let mut chinese = english_card();
        chinese.front = "学习".into();
        chinese.source_lang = "zh-CN".into();
        chinese.script_metadata = ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Simplified,
        };
        let apple = names(&["Front", "Back", "Context Sentence", "Notes", "Front Audio"]);
        assert_eq!(
            lost_values(&chinese, &FieldMapping::apple_style_default(), &apple),
            vec![unmapped(CardField::Pinyin)]
        );

        // No audio bytes: no audio entry, whatever the audio field.
        let mut silent = english_card();
        silent.audio_front = None;
        let no_audio_field = FieldMapping {
            audio_field: None,
            ..auto.clone()
        };
        assert_eq!(lost_values(&silent, &no_audio_field, &apple_v3()), vec![]);
        silent.audio_front = Some(Vec::new());
        assert_eq!(lost_values(&silent, &no_audio_field, &apple_v3()), vec![]);
        assert_eq!(
            lost_values(&english_card(), &no_audio_field, &apple_v3()),
            vec![LostValue::Audio { audio_field: None }]
        );

        // AnkiConnect attaches media only to an exactly named field.
        let lowercase_audio = FieldMapping {
            audio_field: Some("audio".into()),
            ..auto.clone()
        };
        assert_eq!(
            lost_values(&english_card(), &lowercase_audio, &apple_v3()),
            vec![LostValue::Audio {
                audio_field: Some("audio".into())
            }]
        );

        // An empty value whose field is missing loses nothing.
        let with_notes = FieldMapping {
            fields: [
                auto.fields.clone(),
                vec![FieldBinding::new("Notes", CardField::Notes)],
            ]
            .concat(),
            ..auto.clone()
        };
        assert_eq!(
            lost_values(&english_card(), &with_notes, &apple_v3()),
            vec![]
        );
        let mut noted = english_card();
        noted.notes = "Rare word.".into();
        assert_eq!(
            lost_values(&noted, &with_notes, &apple_v3()),
            vec![LostValue::MissingField {
                anki_field: "Notes".into(),
                source: CardField::Notes
            }]
        );
        assert_eq!(
            lost_values(&noted, &auto, &apple_v3()),
            vec![unmapped(CardField::Notes)]
        );
    }

    #[test]
    fn lost_values_counts_any_field_that_carries_an_example_half() {
        let fields = names(&["Front", "Back", "Sentence"]);
        let with = |source: CardField| FieldMapping {
            fields: vec![
                FieldBinding::new("Front", CardField::Front),
                FieldBinding::new("Back", CardField::Back),
                FieldBinding::new("Sentence", source),
            ],
            audio_field: None,
        };
        let mut card = english_card();
        card.audio_front = None;
        assert_eq!(
            lost_values(&card, &with(CardField::Example), &fields),
            vec![]
        );
        assert_eq!(
            lost_values(&card, &with(CardField::ExampleSource), &fields),
            vec![unmapped(CardField::ExampleInTargetLang)]
        );
        assert_eq!(
            lost_values(&card, &with(CardField::ExampleInTargetLang), &fields),
            vec![unmapped(CardField::ExampleInSourceLang)]
        );
        // A case-insensitive name match delivers the text value.
        let shouty = names(&["FRONT", "BACK", "SENTENCE"]);
        assert_eq!(
            lost_values(&card, &with(CardField::Example), &shouty),
            vec![]
        );
    }

    #[test]
    fn serde_shapes() {
        assert_eq!(
            serde_json::to_value([
                LostValue::MissingField {
                    anki_field: "Context Sentence".into(),
                    source: CardField::Example,
                },
                LostValue::Unmapped {
                    source: CardField::ExampleInSourceLang,
                },
                LostValue::Audio {
                    audio_field: Some("Front Audio".into()),
                },
                LostValue::Audio { audio_field: None },
            ])
            .unwrap(),
            json!([
                { "kind": "missing_field", "anki_field": "Context Sentence", "source": "example" },
                { "kind": "unmapped", "source": "example_in_source_lang" },
                { "kind": "audio", "audio_field": "Front Audio" },
                { "kind": "audio", "audio_field": null }
            ])
        );
        let matched = auto_match(&names(&["Q", "Back", "Add Reverse", "Audio"]), "en", "de");
        let value = serde_json::to_value(&matched).unwrap();
        assert_eq!(
            value,
            json!({
                "mapping": {
                    "fields": [
                        { "anki_field": "Q", "source": "front" },
                        { "anki_field": "Back", "source": "back" }
                    ],
                    "audio_field": "Audio"
                },
                "guessed": ["Q"],
                "unmatched": ["Add Reverse"]
            })
        );
        assert_eq!(serde_json::from_value::<AutoMatch>(value).unwrap(), matched);
    }
}
