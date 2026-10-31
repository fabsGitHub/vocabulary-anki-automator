//! The learned-vocabulary pool (Milestone M1): the one place quiz content
//! comes from.
//!
//! The isolation guarantee — an exercise never shows a word the learner has
//! not learned — can only be tested as one thing if every exercise draws from
//! one query. [`LearnedVocabularyPool::QUERY`] is that query, and it defines
//! "learned" the way the store does (§ 6.1): the capture *has an `srs_state`
//! row*. It never reads `hsk_curriculum`: the curriculum is reference data
//! the learner may never have seen, and only the daily queue may look at it
//! (to pick the next new items).
//!
//! [`LearnedVocabularyPool::load`] is the only way to obtain a pool, and its
//! fields are private, so a generator handed a pool cannot be handed
//! vocabulary from anywhere else alongside it.

use std::collections::{HashMap, HashSet};

use rusqlite::Row;
use serde::{Deserialize, Serialize};

use crate::{
    language::is_cjk_char,
    store::{Store, StoreResult},
};

/// One learned card, reduced to the fields exercises are built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoolEntry {
    /// `captures.id`, which is also the `srs_state.card_id`.
    pub card_id: i64,
    /// The expression being learned, in `source_lang`.
    pub front: String,
    /// Its translation, in `target_lang`; empty when none was saved.
    pub back: String,
    /// Tone-marked pinyin of a Chinese card; `None` for other scripts and when
    /// the stored value is blank.
    pub pinyin: Option<String>,
    pub hsk_level: Option<u8>,
    pub source_lang: String,
    pub target_lang: String,
    /// Example sentence in `source_lang`; empty when none.
    pub example_source: String,
    /// `example_source` in `target_lang`; empty when none.
    pub example_target: String,
    /// `captures.script_kind`: `"none"` or `"chinese"`.
    pub script_kind: String,
    /// Whether the capture stores TTS audio of `front` (a non-empty
    /// `audio_front`), which listening exercises play instead of synthesizing.
    pub has_audio: bool,
}

/// Every learned card: each `captures` row that has an `srs_state` row, in
/// `card_id` order.
///
/// Exercises are deterministic for a given pool, so the order is part of the
/// contract: the Learn web build reproduces the same exercises from the same
/// rows in the same order.
#[derive(Debug)]
pub struct LearnedVocabularyPool {
    entries: Vec<PoolEntry>,
    by_card_id: HashMap<i64, usize>,
    hanzi: HashSet<char>,
}

impl LearnedVocabularyPool {
    /// The canonical definition of learned vocabulary: `captures` joined to
    /// `srs_state` on the card id. An `INNER` join, so a capture without a
    /// scheduling state (captured but never introduced) is not learned, and
    /// `hsk_curriculum` is not consulted at all.
    pub const QUERY: &'static str = "SELECT captures.id, captures.front, captures.back, \
         captures.pinyin, captures.hsk_level, captures.source_lang, captures.target_lang, \
         captures.example_source, captures.example_target, captures.script_kind, \
         (captures.audio_front IS NOT NULL AND length(captures.audio_front) > 0) AS has_audio \
         FROM captures INNER JOIN srs_state ON srs_state.card_id = captures.id \
         ORDER BY captures.id ASC";

    /// Reads the pool from the store with [`Self::QUERY`]. This is the only
    /// constructor: every exercise is built from a pool that came from here.
    pub fn load(store: &Store) -> StoreResult<Self> {
        let mut statement = store.conn().prepare_cached(Self::QUERY)?;
        let entries = statement
            .query_map([], entry_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::assemble(entries))
    }

    fn assemble(entries: Vec<PoolEntry>) -> Self {
        let by_card_id = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.card_id, index))
            .collect();
        let hanzi = entries
            .iter()
            .flat_map(|entry| entry.front.chars())
            .filter(|c| is_cjk_char(*c))
            .collect();
        Self {
            entries,
            by_card_id,
            hanzi,
        }
    }

    /// The learned cards, in `card_id` order.
    pub fn entries(&self) -> &[PoolEntry] {
        &self.entries
    }

    /// The learned card with this id; `None` when the card is unknown or not
    /// learned.
    pub fn get(&self, card_id: i64) -> Option<&PoolEntry> {
        self.by_card_id
            .get(&card_id)
            .map(|&index| &self.entries[index])
    }

    /// Every CJK ideograph that occurs in a learned card's front. Exercises
    /// show no other hanzi.
    pub fn learned_hanzi_chars(&self) -> HashSet<char> {
        self.hanzi.clone()
    }

    /// [`Self::learned_hanzi_chars`] without the copy, for the generators.
    pub(crate) fn hanzi(&self) -> &HashSet<char> {
        &self.hanzi
    }
}

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<PoolEntry> {
    let pinyin: Option<String> = row.get(3)?;
    Ok(PoolEntry {
        card_id: row.get(0)?,
        front: row.get(1)?,
        back: row.get(2)?,
        pinyin: pinyin.filter(|text| !text.trim().is_empty()),
        hsk_level: row.get(4)?,
        source_lang: row.get(5)?,
        target_lang: row.get(6)?,
        example_source: row.get(7)?,
        example_target: row.get(8)?,
        script_kind: row.get(9)?,
        has_audio: row.get(10)?,
    })
}
