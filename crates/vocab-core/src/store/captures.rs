//! The `captures` table: every card either app knows about, whether it came
//! from a clipboard capture, the curriculum or an Anki import.
//!
//! `Card.script_metadata` is spread over a `script_kind` column plus nullable
//! `pinyin`, `hsk_level` and `script_variant` columns instead of one JSON
//! value, because the daily queue selects on `script_kind` and `front`
//! (`idx_captures_script_front`). The other card fields map one to one;
//! `tags` is JSON text and `audio_front` a BLOB.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use rusqlite::{params, Params, Row, ToSql};
use serde::{Deserialize, Serialize};

use super::{codec, Store, StoreError, StoreResult};
use crate::model::{CaptureOrigin, Card, ScriptMetadata, ScriptVariant, StoredCapture};

const SELECT_CAPTURES: &str = "SELECT id, front, back, example_source, example_target, notes, \
     tags, source_lang, target_lang, captured_at, synced_to_anki, pinyin, hsk_level, \
     script_kind, script_variant, origin, audio_front FROM captures";

const INSERT_CAPTURE: &str = "INSERT INTO captures (front, back, example_source, \
     example_target, notes, tags, source_lang, target_lang, captured_at, script_kind, pinyin, \
     hsk_level, script_variant, audio_front, origin, synced_to_anki) VALUES (:front, :back, \
     :example_source, :example_target, :notes, :tags, :source_lang, :target_lang, :captured_at, \
     :script_kind, :pinyin, :hsk_level, :script_variant, :audio_front, :origin, \
     :synced_to_anki)";

const UPDATE_CAPTURE_CARD: &str = "UPDATE captures SET front = :front, back = :back, \
     example_source = :example_source, example_target = :example_target, notes = :notes, \
     tags = :tags, source_lang = :source_lang, target_lang = :target_lang, \
     captured_at = :captured_at, script_kind = :script_kind, pinyin = :pinyin, \
     hsk_level = :hsk_level, script_variant = :script_variant, audio_front = :audio_front \
     WHERE id = :id";

/// A capture that has no `srs_state` row yet, as the daily queue sees it
/// ([`Store::list_unscheduled_captures`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnscheduledCapture {
    /// The `captures.id`.
    pub card_id: i64,
    pub front: String,
    pub captured_at: DateTime<Utc>,
}

impl Store {
    /// Saves `card` as a new row and returns its id.
    pub fn insert_capture(
        &self,
        card: &Card,
        origin: CaptureOrigin,
        synced_to_anki: bool,
    ) -> StoreResult<i64> {
        let columns = CardColumns::new(card)?;
        let origin = origin.as_str();
        let mut params = columns.params().to_vec();
        params.push((":origin", &origin));
        params.push((":synced_to_anki", &synced_to_anki));
        Ok(self
            .conn()
            .prepare_cached(INSERT_CAPTURE)?
            .insert(params.as_slice())?)
    }

    pub fn get_capture(&self, id: i64) -> StoreResult<Option<StoredCapture>> {
        Ok(self
            .query_captures("WHERE id = ?1", [id])?
            .into_iter()
            .next())
    }

    /// The newest `limit` captures: `captured_at` descending, then the most
    /// recently inserted first among equal timestamps.
    pub fn list_recent_captures(&self, limit: u32) -> StoreResult<Vec<StoredCapture>> {
        self.query_captures("ORDER BY captured_at DESC, id DESC LIMIT ?1", [limit])
    }

    /// Every capture in capture order (oldest first, ties by id), optionally
    /// restricted to one origin. Lessons are built from this listing.
    pub fn list_captures(&self, origin: Option<CaptureOrigin>) -> StoreResult<Vec<StoredCapture>> {
        match origin {
            Some(origin) => self.query_captures(
                "WHERE origin = ?1 ORDER BY captured_at ASC, id ASC",
                [origin.as_str()],
            ),
            None => self.query_captures("ORDER BY captured_at ASC, id ASC", []),
        }
    }

    /// Captures not yet pushed to Anki, oldest row first, for retrying.
    pub fn list_unsynced_captures(&self) -> StoreResult<Vec<StoredCapture>> {
        self.query_captures("WHERE synced_to_anki = 0 ORDER BY id ASC", [])
    }

    /// Fails with [`StoreError::NotFound`] when no capture has this id.
    pub fn set_synced_to_anki(&self, id: i64, synced: bool) -> StoreResult<()> {
        let changed = self.conn().execute(
            "UPDATE captures SET synced_to_anki = ?1 WHERE id = ?2",
            params![synced, id],
        )?;
        require_row(changed, id)
    }

    /// Deletes a capture together with its `srs_state` and `review_log` rows
    /// (via `ON DELETE CASCADE`). Returns whether the capture existed.
    pub fn delete_capture(&self, id: i64) -> StoreResult<bool> {
        let deleted = self
            .conn()
            .execute("DELETE FROM captures WHERE id = ?1", [id])?;
        Ok(deleted > 0)
    }

    /// The oldest capture with exactly this `front` and `script_kind`
    /// ([`ScriptMetadata::kind`]).
    pub fn find_capture_by_front(
        &self,
        front: &str,
        script_kind: &str,
    ) -> StoreResult<Option<StoredCapture>> {
        Ok(self
            .query_captures(
                "WHERE script_kind = ?1 AND front = ?2 ORDER BY id ASC LIMIT 1",
                [script_kind, front],
            )?
            .into_iter()
            .next())
    }

    /// The fronts of every capture with Chinese script metadata, learned or
    /// not.
    pub fn chinese_capture_fronts(&self) -> StoreResult<HashSet<String>> {
        let mut statement = self
            .conn()
            .prepare_cached("SELECT DISTINCT front FROM captures WHERE script_kind = 'chinese'")?;
        let fronts = statement.query_map([], |row| row.get(0))?;
        Ok(fronts.collect::<Result<_, _>>()?)
    }

    /// The fronts of Chinese captures that have an `srs_state` row: the
    /// learned hanzi, which the daily queue must not offer as new. A word
    /// captured in Automator but never scheduled is not in this set, so the
    /// curriculum still offers it (and `introduce_new_item` adopts the
    /// existing capture instead of duplicating it).
    pub fn learned_chinese_fronts(&self) -> StoreResult<HashSet<String>> {
        let mut statement = self.conn().prepare_cached(
            "SELECT DISTINCT c.front FROM captures c \
             INNER JOIN srs_state s ON s.card_id = c.id \
             WHERE c.script_kind = 'chinese'",
        )?;
        let fronts = statement.query_map([], |row| row.get(0))?;
        Ok(fronts.collect::<Result<_, _>>()?)
    }

    /// Every capture of any origin that has no `srs_state` row, in capture
    /// order (oldest first, ties by id): the cards nobody has started to
    /// learn. Only the columns the daily queue needs are read.
    pub fn list_unscheduled_captures(&self) -> StoreResult<Vec<UnscheduledCapture>> {
        let mut statement = self.conn().prepare_cached(
            "SELECT c.id, c.front, c.captured_at FROM captures c \
             WHERE NOT EXISTS (SELECT 1 FROM srs_state s WHERE s.card_id = c.id) \
             ORDER BY c.captured_at ASC, c.id ASC",
        )?;
        let captures = statement.query_and_then([], |row| {
            let card_id: i64 = row.get(0)?;
            let captured_at = codec::parse_timestamp(&row.get::<_, String>(2)?)
                .map_err(|err| err.in_row("captures", card_id))?;
            Ok(UnscheduledCapture {
                card_id,
                front: row.get(1)?,
                captured_at,
            })
        })?;
        captures.collect()
    }

    /// Replaces every card field of an existing capture, `captured_at`
    /// included. `origin` and `synced_to_anki` are left as they are: whether
    /// an edit needs re-syncing is the caller's decision. Fails with
    /// [`StoreError::NotFound`] when no capture has this id.
    pub fn update_capture_card(&self, id: i64, card: &Card) -> StoreResult<()> {
        let columns = CardColumns::new(card)?;
        let mut params = columns.params().to_vec();
        params.push((":id", &id));
        let changed = self
            .conn()
            .prepare_cached(UPDATE_CAPTURE_CARD)?
            .execute(params.as_slice())?;
        require_row(changed, id)
    }

    fn query_captures(&self, clause: &str, params: impl Params) -> StoreResult<Vec<StoredCapture>> {
        let mut statement = self
            .conn()
            .prepare_cached(&format!("{SELECT_CAPTURES} {clause}"))?;
        let captures = statement.query_and_then(params, capture_from_row)?;
        captures.collect()
    }
}

fn require_row(changed: usize, id: i64) -> StoreResult<()> {
    if changed == 0 {
        return Err(StoreError::NotFound(format!("capture {id}")));
    }
    Ok(())
}

/// The values of the columns that hold `Card` fields, bound by name so that
/// INSERT and UPDATE cannot disagree about them.
struct CardColumns<'a> {
    card: &'a Card,
    tags: String,
    captured_at: String,
    script_kind: &'static str,
    pinyin: Option<&'a str>,
    hsk_level: Option<u8>,
    script_variant: Option<&'static str>,
}

impl<'a> CardColumns<'a> {
    fn new(card: &'a Card) -> StoreResult<Self> {
        let (pinyin, hsk_level, script_variant) = match &card.script_metadata {
            ScriptMetadata::None => (None, None, None),
            ScriptMetadata::Chinese {
                pinyin,
                hsk_level,
                script_variant,
            } => (
                Some(pinyin.as_str()),
                *hsk_level,
                Some(script_variant.as_str()),
            ),
        };
        Ok(Self {
            card,
            tags: codec::encode_tags(&card.tags)?,
            captured_at: codec::format_timestamp(card.captured_at)?,
            script_kind: card.script_metadata.kind(),
            pinyin,
            hsk_level,
            script_variant,
        })
    }

    fn params(&self) -> [(&'static str, &dyn ToSql); 14] {
        [
            (":front", &self.card.front),
            (":back", &self.card.back),
            (":example_source", &self.card.example_source),
            (":example_target", &self.card.example_target),
            (":notes", &self.card.notes),
            (":tags", &self.tags),
            (":source_lang", &self.card.source_lang),
            (":target_lang", &self.card.target_lang),
            (":captured_at", &self.captured_at),
            (":script_kind", &self.script_kind),
            (":pinyin", &self.pinyin),
            (":hsk_level", &self.hsk_level),
            (":script_variant", &self.script_variant),
            (":audio_front", &self.card.audio_front),
        ]
    }
}

fn capture_from_row(row: &Row<'_>) -> StoreResult<StoredCapture> {
    let id: i64 = row.get("id")?;
    decode_capture(id, row).map_err(|err| err.in_row("captures", id))
}

fn decode_capture(id: i64, row: &Row<'_>) -> StoreResult<StoredCapture> {
    let script_metadata = decode_script_metadata(
        &row.get::<_, String>("script_kind")?,
        row.get("pinyin")?,
        row.get("hsk_level")?,
        row.get("script_variant")?,
    )?;
    let origin: String = row.get("origin")?;
    let origin = CaptureOrigin::parse(&origin)
        .ok_or_else(|| StoreError::Invalid(format!("unknown origin {origin:?}")))?;
    let card = Card {
        front: row.get("front")?,
        back: row.get("back")?,
        example_source: row.get("example_source")?,
        example_target: row.get("example_target")?,
        notes: row.get("notes")?,
        tags: codec::decode_tags(&row.get::<_, String>("tags")?)?,
        source_lang: row.get("source_lang")?,
        target_lang: row.get("target_lang")?,
        captured_at: codec::parse_timestamp(&row.get::<_, String>("captured_at")?)?,
        audio_front: row.get("audio_front")?,
        script_metadata,
    };
    Ok(StoredCapture {
        id,
        card,
        origin,
        synced_to_anki: row.get("synced_to_anki")?,
    })
}

/// Rebuilds `ScriptMetadata` from its columns. A missing pinyin or variant on
/// a Chinese row falls back to empty / Simplified (the variant's serde
/// default); a kind or variant this build does not know is an error rather
/// than a guess.
fn decode_script_metadata(
    kind: &str,
    pinyin: Option<String>,
    hsk_level: Option<u8>,
    script_variant: Option<String>,
) -> StoreResult<ScriptMetadata> {
    match kind {
        "none" => Ok(ScriptMetadata::None),
        "chinese" => {
            let script_variant = match script_variant.as_deref() {
                None => ScriptVariant::default(),
                Some(text) => ScriptVariant::parse(text).ok_or_else(|| {
                    StoreError::Invalid(format!("unknown script_variant {text:?}"))
                })?,
            };
            Ok(ScriptMetadata::Chinese {
                pinyin: pinyin.unwrap_or_default(),
                hsk_level,
                script_variant,
            })
        }
        other => Err(StoreError::Invalid(format!(
            "unknown script_kind {other:?}"
        ))),
    }
}
