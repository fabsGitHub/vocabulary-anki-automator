//! drkameleon/complete-hsk-vocabulary's per-level word lists, the primary
//! source: English meanings, traditional forms, part of speech and a
//! frequency rank per word.
//!
//! The `wordlists/exclusive/new/N.json` files hold the words *new* at HSK 3.0
//! level N, and their entries carry no level field, so the level is the file
//! number. Upstream publishes 7–9 as one combined file, `7.json`, which is why
//! `hsk_level` 7 means "7–9" (docs/architecture.md § 12 item 5).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// The level files, 1–6 plus 7 for the combined 7–9 band.
pub const LEVELS: std::ops::RangeInclusive<u8> = 1..=7;

/// One word. Fields the ETL does not use (`radical`, `classifiers`, the other
/// transcriptions) are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
    pub simplified: String,
    /// Corpus frequency rank: lower is more frequent.
    #[serde(default)]
    pub frequency: Option<i64>,
    #[serde(default)]
    pub pos: Vec<String>,
    /// One per reading CC-CEDICT knows, in dictionary order, which is not
    /// HSK order (see `crate::select`).
    #[serde(default)]
    pub forms: Vec<Form>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Form {
    #[serde(default)]
    pub traditional: Option<String>,
    #[serde(default)]
    pub transcriptions: Transcriptions,
    #[serde(default)]
    pub meanings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Transcriptions {
    /// Tone-marked, syllables separated by spaces (mostly; see
    /// `crate::reading`).
    #[serde(default)]
    pub pinyin: String,
}

/// Where level `level`'s file lives under the source directory.
pub fn level_file(source_dir: &Path, level: u8) -> PathBuf {
    source_dir
        .join(crate::sources::VOCABULARY_DIR)
        .join(format!("wordlists/exclusive/new/{level}.json"))
}

pub fn load_level(path: &Path) -> Result<Vec<Entry>> {
    let text = std::fs::read_to_string(path).map_err(|err| Error::io(path, err))?;
    serde_json::from_str(&text).map_err(|err| Error::json(path, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_parse_with_unused_and_missing_fields() {
        let entries: Vec<Entry> = serde_json::from_str(
            r#"[
                {"simplified": "爱", "radical": "爫", "frequency": 130, "pos": ["v", "vn"],
                 "forms": [{"traditional": "愛",
                            "transcriptions": {"pinyin": "ài", "numeric": "ai4"},
                            "meanings": ["to love"], "classifiers": []}]},
                {"simplified": "x", "forms": [{}]}
            ]"#,
        )
        .unwrap();
        assert_eq!(entries[0].frequency, Some(130));
        assert_eq!(entries[0].pos, ["v", "vn"]);
        assert_eq!(entries[0].forms[0].transcriptions.pinyin, "ài");
        assert_eq!(entries[0].forms[0].traditional.as_deref(), Some("愛"));
        assert_eq!(entries[1].frequency, None);
        assert_eq!(entries[1].forms[0].transcriptions.pinyin, "");
        assert!(entries[1].forms[0].meanings.is_empty());
    }
}
