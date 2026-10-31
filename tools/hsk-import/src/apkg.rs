//! `apkg-enrich`: example sentences, audio and images from the user's own
//! Anki deck for the curriculum words it shares (J2's apkg-reuse pass).
//!
//! The official dataset has no example sentences, no audio and no images.
//! The user's "Chinese Vocabulary (HSK 1-4).apkg" has all three for about
//! 1,400 words, so this pass matches each note's `Chinese` field against
//! `hanzi` and fills what is missing: `example_sentence_source`, `_pinyin`
//! and `_target` from `Sentence`, `Sentence Pinyin` and `English Sentence`,
//! and `vocab_audio_path`, `sentence_audio_path` and `image_path` from the
//! note's `[sound:...]` and `<img src>` references. `example_sentence_target`
//! is the deck's *English* sentence for now; a German one would need another
//! backfill. Existing values are never overwritten, so the pass can be re-run.
//!
//! Only the legacy package format is read: a zip holding the collection as
//! plain SQLite (`collection.anki21`, or `collection.anki2` from older
//! versions), a JSON `media` map from zip entry to file name, and the media
//! files under numeric names. Note fields are joined with `\x1f` in
//! `notes.flds`; their names come from the note type JSON in `col.models`.
//! Anki 2.1.50+ exports `collection.anki21b` (zstd, protobuf media map)
//! unless "Support older Anki versions" is ticked, and the approved
//! dependencies cannot read that, so it is refused with that instruction.
//!
//! Extracted media go to a directory of the user's choosing and are never
//! committed: the deck's media are the deck author's, not part of the MIT
//! sources. Shared decks are untrusted input, so field parsing is linear,
//! media names that could escape that directory are refused, and every entry
//! read from the zip is capped ([`ApkgLimits`]): a few kilobytes of deflate
//! can expand to gigabytes (a zip bomb), and the size an entry declares is
//! the packer's claim, not a limit. A media file is written under a temporary
//! name and renamed into place, so a refused or failed one leaves nothing
//! behind and an existing file or symlink of that name is replaced, never
//! written through.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::File,
    io::{self, BufReader, Read, Seek, Write},
    path::Path,
};

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use vocab_core::{providers::mymemory::decode_html_entities, CurriculumItem};
use zip::ZipArchive;

use crate::error::{Error, Result};

/// The new-format collection that this tool cannot read.
const ZSTD_COLLECTION: &str = "collection.anki21b";
/// Legacy collections, preferred first: an Anki 2.1 export carries a real
/// `collection.anki21` next to a stub `collection.anki2` for Anki 2.0.
const LEGACY_COLLECTIONS: &[&str] = &["collection.anki21", "collection.anki2"];

const REEXPORT_HINT: &str = "re-export the deck from Anki with \"Support older Anki versions\" \
     ticked (File > Export..., Anki Deck Package)";

/// How much of a package is read at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApkgLimits {
    /// The collection database, decompressed.
    pub collection_bytes: u64,
    /// The `media` map and each media file, decompressed.
    pub entry_bytes: u64,
}

impl Default for ApkgLimits {
    /// Far above a vocabulary deck: its collection is a few megabytes, its
    /// recordings and pictures some kilobytes each.
    fn default() -> Self {
        Self {
            collection_bytes: 1024 * 1024 * 1024,
            entry_bytes: 32 * 1024 * 1024,
        }
    }
}

/// Copies `entry` into `out` unless it is larger than `limit` bytes, by its
/// own claim or in fact; `Ok(false)` when it is.
fn copy_capped<R: Read>(
    entry: &mut zip::read::ZipFile<'_, R>,
    out: &mut impl Write,
    limit: u64,
) -> io::Result<bool> {
    if entry.size() > limit {
        return Ok(false);
    }
    let copied = io::copy(&mut entry.by_ref().take(limit + 1), out)?;
    Ok(copied <= limit)
}

/// Which note fields hold what. The defaults are the field names of the
/// user's deck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApkgFields {
    pub word: String,
    pub sentence: String,
    pub sentence_pinyin: String,
    pub sentence_translation: String,
}

impl Default for ApkgFields {
    fn default() -> Self {
        Self {
            word: "Chinese".into(),
            sentence: "Sentence".into(),
            sentence_pinyin: "Sentence Pinyin".into(),
            sentence_translation: "English Sentence".into(),
        }
    }
}

/// What the pass found and changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnrichReport {
    /// The zip entry the notes were read from.
    pub collection: String,
    pub notes: usize,
    /// Notes whose note type has no word field.
    pub without_word_field: usize,
    pub matched: usize,
    /// Words of the deck that are not curriculum hanzi.
    pub unmatched: Vec<String>,
    /// Further notes for an already matched word (the first note wins).
    pub duplicates: usize,
    /// Items that received a sentence, its pinyin, its translation.
    pub sentences: usize,
    pub sentence_pinyin: usize,
    pub sentence_translations: usize,
    pub vocab_audio: usize,
    pub sentence_audio: usize,
    pub images: usize,
    /// Referenced media that the package does not contain.
    pub media_missing: Vec<String>,
    /// Referenced media names that are not plain file names.
    pub media_rejected: Vec<String>,
    /// Referenced media larger than [`ApkgLimits::entry_bytes`].
    pub media_too_large: Vec<String>,
}

impl fmt::Display for EnrichReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |names: &[String]| {
            let shown: Vec<&str> = names.iter().take(8).map(String::as_str).collect();
            let more = if names.len() > shown.len() {
                ", ..."
            } else {
                ""
            };
            if shown.is_empty() {
                String::new()
            } else {
                format!(" ({}{more})", shown.join(", "))
            }
        };
        writeln!(f, "Anki package enrichment (read {})", self.collection)?;
        writeln!(
            f,
            "  notes: {}, matched curriculum words: {}, not in the curriculum: {}{}, \
             duplicate notes: {}, notes without the word field: {}",
            self.notes,
            self.matched,
            self.unmatched.len(),
            list(&self.unmatched),
            self.duplicates,
            self.without_word_field
        )?;
        writeln!(
            f,
            "  filled: sentences {}, sentence pinyin {}, sentence translations {} \
             (English, not German), word audio {}, sentence audio {}, images {}",
            self.sentences,
            self.sentence_pinyin,
            self.sentence_translations,
            self.vocab_audio,
            self.sentence_audio,
            self.images
        )?;
        writeln!(
            f,
            "  media missing from the package: {}{}; refused media names: {}{}; \
             media over the size limit: {}{}",
            self.media_missing.len(),
            list(&self.media_missing),
            self.media_rejected.len(),
            list(&self.media_rejected),
            self.media_too_large.len(),
            list(&self.media_too_large)
        )
    }
}

#[derive(Deserialize)]
struct NoteType {
    #[serde(default)]
    flds: Vec<FieldDef>,
}

#[derive(Deserialize)]
struct FieldDef {
    name: String,
    #[serde(default)]
    ord: Option<u32>,
}

/// Fills `items` from the package at `apkg` (see the module docs), copying
/// referenced media into `media_out`. Paths stored on items are bare file
/// names relative to `media_out`.
pub fn enrich_from_apkg(
    apkg: &Path,
    items: &mut [CurriculumItem],
    media_out: &Path,
    fields: &ApkgFields,
) -> Result<EnrichReport> {
    enrich_from_apkg_with_limits(apkg, items, media_out, fields, ApkgLimits::default())
}

/// [`enrich_from_apkg`] with other size limits. A collection over its limit
/// is an error; a media file over its limit is skipped and reported.
pub fn enrich_from_apkg_with_limits(
    apkg: &Path,
    items: &mut [CurriculumItem],
    media_out: &Path,
    fields: &ApkgFields,
    limits: ApkgLimits,
) -> Result<EnrichReport> {
    let file = File::open(apkg).map_err(|err| Error::io(apkg, err))?;
    let mut archive = ZipArchive::new(BufReader::new(file))?;
    let names: HashSet<String> = archive.file_names().map(str::to_string).collect();
    if names.contains(ZSTD_COLLECTION) {
        return Err(Error::Apkg(format!(
            "{} is in Anki's new export format ({ZSTD_COLLECTION}, zstd-compressed), \
             which this tool cannot read: {REEXPORT_HINT}",
            apkg.display()
        )));
    }
    let collection = LEGACY_COLLECTIONS
        .iter()
        .find(|name| names.contains(**name))
        .ok_or_else(|| {
            Error::Apkg(format!(
                "{} is not an Anki deck package: it has no {}",
                apkg.display(),
                LEGACY_COLLECTIONS.join(" or ")
            ))
        })?;

    let mut database = tempfile::NamedTempFile::new().map_err(|err| Error::io("temp dir", err))?;
    let fits = copy_capped(
        &mut archive.by_name(collection)?,
        &mut database,
        limits.collection_bytes,
    )
    .map_err(|err| Error::io(database.path(), err))?;
    if !fits {
        return Err(Error::Apkg(format!(
            "{} holds a {collection} larger than {} bytes; that is not a vocabulary deck, \
             so it was not read",
            apkg.display(),
            limits.collection_bytes
        )));
    }
    let conn = Connection::open_with_flags(database.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let note_types = read_note_types(&conn)?;
    let media = read_media_map(&mut archive, limits.entry_bytes)?;

    let by_hanzi: HashMap<String, usize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (item.hanzi.clone(), index))
        .collect();
    let mut report = EnrichReport {
        collection: collection.to_string(),
        ..EnrichReport::default()
    };
    let mut media_out = MediaOut {
        dir: media_out,
        by_name: media,
        extracted: HashSet::new(),
        limit: limits.entry_bytes,
    };
    let mut matched: HashSet<usize> = HashSet::new();

    let mut statement = conn.prepare("SELECT mid, flds FROM notes ORDER BY id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        report.notes += 1;
        let mid: i64 = row.get(0)?;
        let flds: String = row.get(1)?;
        let Some(field_names) = note_types.get(&mid.to_string()) else {
            report.without_word_field += 1;
            continue;
        };
        let note: Vec<(&str, &str)> = field_names
            .iter()
            .map(String::as_str)
            .zip(flds.split('\x1f'))
            .collect();
        let Some(word) = field(&note, &fields.word) else {
            report.without_word_field += 1;
            continue;
        };
        let hanzi = html_to_text(word);
        let Some(&index) = by_hanzi.get(&hanzi) else {
            if !hanzi.is_empty() {
                report.unmatched.push(hanzi);
            }
            continue;
        };
        if !matched.insert(index) {
            report.duplicates += 1;
            continue;
        }
        report.matched += 1;
        let item = &mut items[index];

        let text_of = |name: &str| {
            field(&note, name)
                .map(html_to_text)
                .filter(|t| !t.is_empty())
        };
        if fill(&mut item.example_sentence_source, text_of(&fields.sentence)) {
            report.sentences += 1;
        }
        if fill(
            &mut item.example_sentence_pinyin,
            text_of(&fields.sentence_pinyin),
        ) {
            report.sentence_pinyin += 1;
        }
        if fill(
            &mut item.example_sentence_target,
            text_of(&fields.sentence_translation),
        ) {
            report.sentence_translations += 1;
        }

        let is_sentence_field = |name: &str| name.to_lowercase().contains("sentence");
        let vocab_sound = note
            .iter()
            .filter(|(name, _)| !is_sentence_field(name))
            .find_map(|(_, value)| sound_refs(value).into_iter().next());
        let sentence_sound = note
            .iter()
            .filter(|(name, _)| is_sentence_field(name))
            .find_map(|(_, value)| sound_refs(value).into_iter().next());
        let image = note
            .iter()
            .find_map(|(_, value)| image_sources(value).into_iter().next());
        for (slot, reference, count) in [
            (
                &mut item.vocab_audio_path,
                vocab_sound,
                &mut report.vocab_audio,
            ),
            (
                &mut item.sentence_audio_path,
                sentence_sound,
                &mut report.sentence_audio,
            ),
            (&mut item.image_path, image, &mut report.images),
        ] {
            if slot.is_some() {
                continue;
            }
            let Some(name) = reference else { continue };
            match media_out.extract(&mut archive, &name)? {
                Extracted::Written => {
                    *slot = Some(name);
                    *count += 1;
                }
                Extracted::Missing => report.media_missing.push(name),
                Extracted::Rejected => report.media_rejected.push(name),
                Extracted::TooLarge => report.media_too_large.push(name),
            }
        }
    }
    Ok(report)
}

/// Field names by note type id, in field order.
fn read_note_types(conn: &Connection) -> Result<HashMap<String, Vec<String>>> {
    let models: String = conn.query_row("SELECT models FROM col", [], |row| row.get(0))?;
    let note_types: HashMap<String, NoteType> = serde_json::from_str(&models)
        .map_err(|err| Error::Apkg(format!("col.models is not note type JSON: {err}")))?;
    if note_types.is_empty() {
        return Err(Error::Apkg(format!(
            "the collection keeps its note types outside col.models (a newer schema): \
             {REEXPORT_HINT}"
        )));
    }
    Ok(note_types
        .into_iter()
        .map(|(id, note_type)| {
            let mut defs: Vec<(u32, String)> = note_type
                .flds
                .into_iter()
                .enumerate()
                .map(|(i, def)| (def.ord.unwrap_or(i as u32), def.name))
                .collect();
            defs.sort_by_key(|(ord, _)| *ord);
            (id, defs.into_iter().map(|(_, name)| name).collect())
        })
        .collect())
}

/// The package's `media` map (zip entry -> file name), inverted. A package
/// without one simply has no media.
fn read_media_map<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    limit: u64,
) -> Result<HashMap<String, String>> {
    let mut bytes = Vec::new();
    match archive.by_name("media") {
        Ok(mut entry) => {
            let fits = copy_capped(&mut entry, &mut bytes, limit)
                .map_err(|err| Error::io("media", err))?;
            if !fits {
                return Err(Error::Apkg(format!(
                    "the media map is larger than {limit} bytes; it was not read"
                )));
            }
        }
        Err(zip::result::ZipError::FileNotFound) => return Ok(HashMap::new()),
        Err(err) => return Err(err.into()),
    }
    let map: HashMap<String, String> = serde_json::from_slice(&bytes)
        .map_err(|err| Error::Apkg(format!("the media map is not JSON: {err}")))?;
    Ok(map.into_iter().map(|(entry, name)| (name, entry)).collect())
}

enum Extracted {
    Written,
    Missing,
    Rejected,
    TooLarge,
}

struct MediaOut<'a> {
    dir: &'a Path,
    /// File name -> zip entry.
    by_name: HashMap<String, String>,
    extracted: HashSet<String>,
    /// Largest file written, in bytes.
    limit: u64,
}

impl MediaOut<'_> {
    fn extract<R: Read + Seek>(
        &mut self,
        archive: &mut ZipArchive<R>,
        name: &str,
    ) -> Result<Extracted> {
        if !is_plain_file_name(name) {
            return Ok(Extracted::Rejected);
        }
        if self.extracted.contains(name) {
            return Ok(Extracted::Written);
        }
        let Some(entry_name) = self.by_name.get(name) else {
            return Ok(Extracted::Missing);
        };
        let mut entry = match archive.by_name(entry_name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(Extracted::Missing),
            Err(err) => return Err(err.into()),
        };
        if entry.size() > self.limit {
            return Ok(Extracted::TooLarge);
        }
        std::fs::create_dir_all(self.dir).map_err(|err| Error::io(self.dir, err))?;
        let path = self.dir.join(name);
        // Deleted on drop unless persisted, so nothing partial stays behind.
        let mut part =
            tempfile::NamedTempFile::new_in(self.dir).map_err(|err| Error::io(self.dir, err))?;
        let fits = copy_capped(&mut entry, part.as_file_mut(), self.limit)
            .map_err(|err| Error::io(part.path(), err))?;
        if !fits {
            return Ok(Extracted::TooLarge);
        }
        part.persist(&path)
            .map_err(|err| Error::io(&path, err.error))?;
        self.extracted.insert(name.to_string());
        Ok(Extracted::Written)
    }
}

/// A name that stays inside the media directory: no separators, no drive or
/// stream colon, no leading dot (which also rules out `..`).
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0'])
        && Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name)
}

/// The value of the field called `name` (exact match first, then ignoring
/// case).
fn field<'a>(note: &[(&str, &'a str)], name: &str) -> Option<&'a str> {
    note.iter()
        .find(|(field, _)| *field == name)
        .or_else(|| {
            note.iter()
                .find(|(field, _)| field.eq_ignore_ascii_case(name))
        })
        .map(|(_, value)| *value)
}

/// `Some` written into an empty slot; true if it was.
fn fill(slot: &mut Option<String>, value: Option<String>) -> bool {
    match (slot.is_none(), value) {
        (true, Some(value)) => {
            *slot = Some(value);
            true
        }
        _ => false,
    }
}

/// Tags that end a line in the rendered field, so they separate words.
/// Inline tags (`<b>`, `<span>`) must not: Chinese has no spaces to restore.
const LINE_BREAK_TAGS: &[&str] = &[
    "br", "div", "p", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6",
];

/// Field HTML as plain text: `[sound:...]` references and tags removed
/// (line-level tags become a space), entities decoded once, whitespace
/// collapsed. A `<` that does not start a tag (`a < b`) is text. Every byte
/// is looked at a bounded number of times.
pub fn html_to_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(['<', '[']) {
        text.push_str(&rest[..start]);
        let after = &rest[start..];
        let markup_end = if after.starts_with("[sound:") {
            Some(']')
        } else if after.starts_with('<') && starts_tag(&after[1..]) {
            Some('>')
        } else {
            None
        };
        let Some(close) = markup_end else {
            text.push_str(&after[..1]);
            rest = &after[1..];
            continue;
        };
        let Some(end) = after.find(close) else {
            text.push_str(after);
            rest = "";
            break;
        };
        if close == '>' && is_line_break_tag(&after[1..end]) {
            text.push(' ');
        }
        rest = &after[end + 1..];
    }
    text.push_str(rest);
    let decoded = decode_html_entities(&text.replace("&nbsp;", " "));
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the text after a `<` reads as a tag: a letter, `/` or `!`.
fn starts_tag(after: &str) -> bool {
    after
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!')
}

/// `inner` is a tag's text between `<` and `>`, e.g. `/div` or `br /`.
fn is_line_break_tag(inner: &str) -> bool {
    let name: String = inner
        .trim_start_matches('/')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    LINE_BREAK_TAGS.contains(&name.as_str())
}

/// Every `[sound:NAME]` in a field, in order.
pub fn sound_refs(value: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find("[sound:") {
        rest = &rest[start + "[sound:".len()..];
        let Some(end) = rest.find(']') else { break };
        let name = decode_html_entities(rest[..end].trim());
        if !name.is_empty() {
            names.push(name);
        }
        rest = &rest[end + 1..];
    }
    names
}

/// Every `<img src=...>` in a field, in order; the attribute may be quoted
/// with `"` or `'`, or bare.
pub fn image_sources(value: &str) -> Vec<String> {
    let lower = value.to_ascii_lowercase();
    let mut names = Vec::new();
    let mut from = 0;
    while let Some(offset) = lower[from..].find("<img") {
        let tag_start = from + offset;
        let tag_end = lower[tag_start..]
            .find('>')
            .map_or(lower.len(), |end| tag_start + end);
        let tag = &value[tag_start..tag_end];
        let tag_lower = &lower[tag_start..tag_end];
        if let Some(src) = tag_lower.find("src=").map(|i| &tag[i + "src=".len()..]) {
            let raw = match src.chars().next() {
                Some(quote @ ('"' | '\'')) => src[1..].split(quote).next().unwrap_or(""),
                _ => src
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or(""),
            };
            let name = decode_html_entities(raw.trim());
            if !name.is_empty() {
                names.push(name);
            }
        }
        from = tag_end.max(tag_start + 1);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_html_becomes_plain_text() {
        let cases = [
            ("学习", "学习"),
            ("  学习&nbsp;", "学习"),
            ("<b>学习</b>", "学习"),
            // Inline tags join, line-level tags separate.
            ("<div>我<b>爱</b>你</div>", "我爱你"),
            ("Wǒ ài nǐ.<br/>Nǐ ne?", "Wǒ ài nǐ. Nǐ ne?"),
            ("one<div>two</div><DIV>three</DIV>", "one two three"),
            ("我爱你[sound:a.mp3]", "我爱你"),
            ("Tom &amp; Jerry &lt;3", "Tom & Jerry <3"),
            ("&amp;nbsp;", "&nbsp;"),
            ("a < b > c", "a < b > c"),
            ("[1] note [x]", "[1] note [x]"),
            ("<b unterminated", "<b unterminated"),
            ("[sound:unterminated", "[sound:unterminated"),
            ("", ""),
        ];
        for (html, expected) in cases {
            assert_eq!(html_to_text(html), expected, "{html:?}");
        }
    }

    #[test]
    fn field_parsing_stays_linear_on_hostile_input() {
        let hostile = format!("{}{}", "<[".repeat(200_000), "[sound:".repeat(50_000));
        let started = std::time::Instant::now();
        let text = html_to_text(&hostile);
        assert!(sound_refs(&hostile).is_empty());
        assert!(image_sources(&"<img".repeat(100_000)).is_empty());
        assert!(text.starts_with("<["), "{}", &text[..10]);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn finds_sound_and_image_references() {
        assert_eq!(
            sound_refs("[sound:a.mp3] text [sound: b&amp;c.mp3 ][sound:]"),
            ["a.mp3", "b&c.mp3"]
        );
        assert!(sound_refs("[sound:open").is_empty());
        assert_eq!(
            image_sources(
                r#"<img src="x.jpg"><IMG alt="y" SRC='y z.png'><img src=bare.gif/><img alt="none">"#
            ),
            ["x.jpg", "y z.png", "bare.gif"]
        );
        assert_eq!(image_sources(r#"<img src="a&amp;b.jpg">"#), ["a&b.jpg"]);
        assert!(image_sources("<img").is_empty());
    }

    #[test]
    fn only_plain_file_names_are_extracted() {
        for good in ["a.mp3", "我.jpg", "_font.ttf", "with space.png"] {
            assert!(is_plain_file_name(good), "{good:?}");
        }
        for bad in [
            "",
            ".",
            "..",
            "../x.mp3",
            "a/b.mp3",
            "a\\b.mp3",
            "/etc/passwd",
            "C:x.mp3",
            ".hidden",
        ] {
            assert!(!is_plain_file_name(bad), "{bad:?}");
        }
    }
}
