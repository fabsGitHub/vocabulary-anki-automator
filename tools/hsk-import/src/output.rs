//! Reading and writing the curriculum JSON and the German overrides file.
//!
//! The bundled curriculum is committed and reviewed as a diff, so the writer
//! is deterministic and line-oriented: a JSON array with one item per line,
//! keys in `CurriculumItem`'s field order, and `None` fields left out. Every
//! optional `CurriculumItem` field is `#[serde(default)]`, so an omitted key
//! reads back as `None`; leaving out the ~10 empty fields of every item keeps
//! the file roughly half the size.
//!
//! Every write goes to a temporary sibling first and is renamed into place,
//! so an interrupted run (the resumable German backfill writes after every
//! item) never leaves a truncated file behind.

use std::{collections::BTreeMap, path::Path};

use serde::Serialize;
use vocab_core::CurriculumItem;

use crate::error::{Error, Result};

/// German meanings by hanzi, as written by `backfill-de` and merged by
/// `build`.
pub type Overrides = BTreeMap<String, String>;

/// Writes `content` to `path` via a temporary sibling and a rename, creating
/// missing parent directories.
pub fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|err| Error::io(dir, err))?;
    }
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(".tmp");
    let temp = path.with_file_name(temp_name);
    std::fs::write(&temp, content).map_err(|err| Error::io(&temp, err))?;
    std::fs::rename(&temp, path).map_err(|err| Error::io(path, err))
}

/// `CurriculumItem` with `None` fields skipped. Built by destructuring
/// without `..`, so a new `CurriculumItem` field does not compile until it is
/// written here too.
#[derive(Serialize)]
struct ItemOut<'a> {
    hanzi: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    traditional: Option<&'a str>,
    hsk_level: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency: Option<i64>,
    curriculum_order: i64,
    pinyin: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pos: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meaning_en: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meaning_de: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cloze_sentence: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    example_sentence_source: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    example_sentence_pinyin: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    example_sentence_target: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vocab_audio_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sentence_audio_path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_path: Option<&'a str>,
}

impl<'a> From<&'a CurriculumItem> for ItemOut<'a> {
    fn from(item: &'a CurriculumItem) -> Self {
        let CurriculumItem {
            hanzi,
            traditional,
            hsk_level,
            frequency,
            curriculum_order,
            pinyin,
            pos,
            meaning_en,
            meaning_de,
            cloze_sentence,
            example_sentence_source,
            example_sentence_pinyin,
            example_sentence_target,
            vocab_audio_path,
            sentence_audio_path,
            image_path,
        } = item;
        Self {
            hanzi,
            traditional: traditional.as_deref(),
            hsk_level: *hsk_level,
            frequency: *frequency,
            curriculum_order: *curriculum_order,
            pinyin,
            pos: pos.as_deref(),
            meaning_en: meaning_en.as_deref(),
            meaning_de: meaning_de.as_deref(),
            cloze_sentence: cloze_sentence.as_deref(),
            example_sentence_source: example_sentence_source.as_deref(),
            example_sentence_pinyin: example_sentence_pinyin.as_deref(),
            example_sentence_target: example_sentence_target.as_deref(),
            vocab_audio_path: vocab_audio_path.as_deref(),
            sentence_audio_path: sentence_audio_path.as_deref(),
            image_path: image_path.as_deref(),
        }
    }
}

/// The curriculum as written to disk (see the module docs).
pub fn curriculum_json(items: &[CurriculumItem]) -> Result<String> {
    let lines = items
        .iter()
        .map(|item| {
            serde_json::to_string(&ItemOut::from(item))
                .map_err(|err| Error::Data(format!("could not serialize {}: {err}", item.hanzi)))
        })
        .collect::<Result<Vec<String>>>()?;
    if lines.is_empty() {
        return Ok("[]\n".to_string());
    }
    Ok(format!("[\n{}\n]\n", lines.join(",\n")))
}

pub fn write_curriculum(path: &Path, items: &[CurriculumItem]) -> Result<()> {
    write_atomic(path, curriculum_json(items)?.as_bytes())
}

pub fn read_curriculum(path: &Path) -> Result<Vec<CurriculumItem>> {
    let text = std::fs::read_to_string(path).map_err(|err| Error::io(path, err))?;
    serde_json::from_str(&text).map_err(|err| Error::json(path, err))
}

/// The overrides at `path`, or `None` when the file does not exist.
pub fn read_overrides(path: &Path) -> Result<Option<Overrides>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|err| Error::json(path, err)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Error::io(path, err)),
    }
}

/// Pretty-printed, one entry per line, sorted by hanzi.
pub fn write_overrides(path: &Path, overrides: &Overrides) -> Result<()> {
    let mut json = serde_json::to_string_pretty(overrides).map_err(|err| Error::json(path, err))?;
    json.push('\n');
    write_atomic(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_item() -> CurriculumItem {
        CurriculumItem {
            hanzi: "学习".into(),
            traditional: Some("學習".into()),
            hsk_level: 1,
            frequency: Some(250),
            curriculum_order: 3,
            pinyin: "xué xí".into(),
            pos: Some("v,vn".into()),
            meaning_en: Some("to learn; to study".into()),
            meaning_de: Some("lernen".into()),
            cloze_sentence: Some("我__中文。".into()),
            example_sentence_source: Some("我学习中文。".into()),
            example_sentence_pinyin: Some("wǒ xué xí zhōng wén".into()),
            example_sentence_target: Some("I study Chinese.".into()),
            vocab_audio_path: Some("xuexi.mp3".into()),
            sentence_audio_path: Some("xuexi_s.mp3".into()),
            image_path: Some("xuexi.jpg".into()),
        }
    }

    fn bare_item() -> CurriculumItem {
        CurriculumItem {
            hanzi: "吧".into(),
            traditional: None,
            hsk_level: 7,
            frequency: None,
            curriculum_order: 0,
            pinyin: "ba".into(),
            pos: None,
            meaning_en: None,
            meaning_de: None,
            cloze_sentence: None,
            example_sentence_source: None,
            example_sentence_pinyin: None,
            example_sentence_target: None,
            vocab_audio_path: None,
            sentence_audio_path: None,
            image_path: None,
        }
    }

    #[test]
    fn writes_one_item_per_line_without_null_fields() {
        let json = curriculum_json(&[bare_item(), full_item()]).unwrap();
        let lines: Vec<&str> = json.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "[");
        assert_eq!(
            lines[1],
            r#"{"hanzi":"吧","hsk_level":7,"curriculum_order":0,"pinyin":"ba"},"#
        );
        assert!(lines[2].starts_with(r#"{"hanzi":"学习","traditional":"學習","hsk_level":1,"#));
        assert_eq!(lines[3], "]");
        assert!(!json.contains("null"));
        assert_eq!(curriculum_json(&[]).unwrap(), "[]\n");
    }

    #[test]
    fn written_items_read_back_as_the_same_curriculum_items() {
        let items = vec![bare_item(), full_item()];
        let json = curriculum_json(&items).unwrap();
        let back: Vec<CurriculumItem> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, items);
    }

    #[test]
    fn written_keys_are_exactly_the_curriculum_item_field_names() {
        let ours: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&ItemOut::from(&full_item())).unwrap())
                .unwrap();
        let theirs = serde_json::to_value(full_item()).unwrap();
        let keys = |value: &serde_json::Value| {
            value
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&ours), keys(&theirs));
        assert_eq!(ours, theirs);
    }

    #[test]
    fn atomic_writes_create_directories_and_replace_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/file.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["file.json"]);
    }

    #[test]
    fn overrides_round_trip_sorted_and_missing_files_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("de.json");
        assert_eq!(read_overrides(&path).unwrap(), None);
        let overrides: Overrides = [("了", "(Partikel)"), ("吧", "nicht wahr?")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        write_overrides(&path, &overrides).unwrap();
        assert_eq!(read_overrides(&path).unwrap(), Some(overrides));
        // Sorted by code point: 了 (U+4E86) before 吧 (U+5427).
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.find("了").unwrap() < text.find("吧").unwrap(),
            "{text}"
        );
    }
}
