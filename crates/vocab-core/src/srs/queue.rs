//! The daily queue (Milestone L3) and the two writes a session makes with its
//! items: introducing a new curriculum word and submitting a review.
//!
//! Reviews come first, all of them: they are what FSRS spaced against
//! forgetting, and skipping them would let intervals overrun. "Due today" ends
//! at the end of the learner's local day, not at the current instant, so a
//! card that falls due this evening is already in the morning's queue. New
//! words follow in strict `curriculum_order`, skipping every hanzi the learner
//! already has a Chinese card for, including words captured in Automator, so
//! the curriculum never teaches a word twice. A quota of zero is not a mode of
//! its own: it takes zero new words from the same sequence.
//!
//! The queue builder sorts its inputs itself, so the order is a property of
//! [`build_daily_queue_from`] (and of the TypeScript port tested against it),
//! not an assumption about how the caller loaded the rows.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{LocalNow, ReviewOutcome, ReviewRating, SrsScheduler};
use crate::{
    language::normalize_lang,
    model::{CaptureOrigin, Card, CurriculumItem, ScriptMetadata, ScriptVariant},
    store::{SrsStateRow, Store, StoreError, StoreResult},
    time::Clock,
};

/// `captures.script_kind` of Chinese cards ([`ScriptMetadata::kind`]).
const CHINESE_SCRIPT_KIND: &str = "chinese";
/// The language every curriculum word is in.
const CHINESE: &str = "zh-CN";
const GERMAN: &str = "de";
const ENGLISH: &str = "en";

/// One entry of the daily queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueueItem {
    /// A learned card that is due today: rate it with [`submit_review`].
    Review {
        card_id: i64,
        due_date: DateTime<Utc>,
    },
    /// A curriculum word to start with [`introduce_new_item`].
    New {
        hanzi: String,
        curriculum_order: i64,
    },
}

/// The queue for one day, from rows already loaded: every state in `due`, by
/// `due_date` and then `card_id`, followed by the first `new_quota` items of
/// `curriculum` in `curriculum_order` whose hanzi is not in `learned_hanzi`.
pub fn build_daily_queue_from(
    due: &[SrsStateRow],
    curriculum: &[CurriculumItem],
    learned_hanzi: &HashSet<String>,
    new_quota: u32,
) -> Vec<QueueItem> {
    let mut reviews: Vec<&SrsStateRow> = due.iter().collect();
    reviews.sort_by_key(|state| (state.due_date, state.card_id));
    let mut in_order: Vec<&CurriculumItem> = curriculum.iter().collect();
    in_order.sort_by_key(|item| item.curriculum_order);
    let new_quota = usize::try_from(new_quota).unwrap_or(usize::MAX);

    let reviews = reviews.into_iter().map(|state| QueueItem::Review {
        card_id: state.card_id,
        due_date: state.due_date,
    });
    let new_items = in_order
        .into_iter()
        .filter(|item| !learned_hanzi.contains(&item.hanzi))
        .take(new_quota)
        .map(|item| QueueItem::New {
            hanzi: item.hanzi.clone(),
            curriculum_order: item.curriculum_order,
        });
    reviews.chain(new_items).collect()
}

/// Today's queue from the store: every state due before the end of the
/// user's local day, then up to `new_quota` new curriculum words (see
/// [`build_daily_queue_from`]). The front of every Chinese capture counts as
/// a learned hanzi.
pub fn build_daily_queue(
    store: &Store,
    new_quota: u32,
    clock: &dyn Clock,
) -> StoreResult<Vec<QueueItem>> {
    let due = due_today(store, &LocalNow::read(clock))?;
    let curriculum = store.curriculum_in_order()?;
    let learned_hanzi = store.chinese_capture_fronts()?;
    Ok(build_daily_queue_from(
        &due,
        &curriculum,
        &learned_hanzi,
        new_quota,
    ))
}

/// The review part of the queue at `now`.
pub(super) fn due_today(store: &Store, now: &LocalNow) -> StoreResult<Vec<SrsStateRow>> {
    store.list_srs_due_before(now.end_of_today())
}

/// Starts learning the curriculum word `hanzi` and returns its card id: one
/// transaction adds a `captures` row (`origin = curriculum`) and a `new`
/// `srs_state` row due now.
///
/// The card's `back` is the curriculum's German meaning when `target_lang` is
/// German and the meaning is not blank, else its English meaning, else empty,
/// and its `target_lang` is the language `back` is actually in, so that TTS
/// and Anki export never read an English meaning as German. The example
/// sentence, pinyin and HSK level are copied from the curriculum.
///
/// Idempotent: when a Chinese capture of `hanzi` exists already (introduced
/// before, or captured in Automator), nothing new is inserted and the oldest
/// such capture's id is returned, after giving it a `new` state if it had none,
/// so the id is always ready for [`submit_review`]. Fails with
/// [`StoreError::NotFound`] when `hanzi` is neither captured nor in the
/// curriculum.
pub fn introduce_new_item(
    store: &Store,
    scheduler: &SrsScheduler,
    hanzi: &str,
    target_lang: &str,
    clock: &dyn Clock,
) -> StoreResult<i64> {
    let now = LocalNow::read(clock);
    store.with_transaction(|store| {
        if let Some(existing) = store.find_capture_by_front(hanzi, CHINESE_SCRIPT_KIND)? {
            if store.get_srs_state(existing.id)?.is_none() {
                store.upsert_srs_state(&scheduler.initial_state(existing.id, now.at))?;
            }
            return Ok(existing.id);
        }
        let item = store
            .get_curriculum_item(hanzi)?
            .ok_or_else(|| StoreError::NotFound(format!("curriculum item {hanzi:?}")))?;
        let card = curriculum_card(&item, target_lang, now.at);
        let card_id = store.insert_capture(&card, CaptureOrigin::Curriculum, false)?;
        store.upsert_srs_state(&scheduler.initial_state(card_id, now.at))?;
        Ok(card_id)
    })
}

/// Rates the card `card_id` now. One transaction loads its state, runs FSRS,
/// writes the next state and appends the review to `review_log`; the returned
/// log carries the id it was stored under, and its `session_date` is the
/// user's local date.
///
/// Fails, writing nothing, with [`StoreError::NotFound`] when the card has no
/// `srs_state` row and with [`StoreError::Invalid`] when FSRS cannot interpret
/// the row (see [`super::fsrs_card_from_state`]).
pub fn submit_review(
    store: &Store,
    scheduler: &SrsScheduler,
    card_id: i64,
    rating: ReviewRating,
    clock: &dyn Clock,
) -> StoreResult<ReviewOutcome> {
    let now = LocalNow::read(clock);
    store.with_transaction(|store| {
        let state = store
            .get_srs_state(card_id)?
            .ok_or_else(|| StoreError::NotFound(format!("srs_state of card {card_id}")))?;
        let mut outcome = scheduler.try_review(&state, rating, now.at, now.today)?;
        store.upsert_srs_state(&outcome.next)?;
        outcome.log.id = store.insert_review_log(&outcome.log)?;
        Ok(outcome)
    })
}

fn curriculum_card(item: &CurriculumItem, target_lang: &str, captured_at: DateTime<Utc>) -> Card {
    let target_lang = normalize_lang(target_lang);
    let present = |meaning: &Option<String>| {
        meaning
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
    };
    let german = (target_lang == GERMAN)
        .then(|| present(&item.meaning_de))
        .flatten();
    let (back, back_lang) = match (german, present(&item.meaning_en)) {
        (Some(german), _) => (german, GERMAN.to_owned()),
        (None, Some(english)) => (english, ENGLISH.to_owned()),
        (None, None) => (String::new(), target_lang),
    };
    let mut card = Card::new(item.hanzi.clone(), back, CHINESE, back_lang, captured_at);
    card.example_source = item.example_sentence_source.clone().unwrap_or_default();
    card.example_target = item.example_sentence_target.clone().unwrap_or_default();
    card.script_metadata = ScriptMetadata::Chinese {
        pinyin: item.pinyin.clone(),
        hsk_level: Some(item.hsk_level),
        script_variant: ScriptVariant::Simplified,
    };
    card
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::time::FixedClock;

    fn curriculum_word() -> CurriculumItem {
        CurriculumItem {
            hanzi: "学习".into(),
            traditional: None,
            hsk_level: 1,
            frequency: None,
            curriculum_order: 1,
            pinyin: "xué xí".into(),
            pos: None,
            meaning_en: Some("to study".into()),
            meaning_de: Some("lernen".into()),
            cloze_sentence: None,
            example_sentence_source: None,
            example_sentence_pinyin: None,
            example_sentence_target: None,
            vocab_audio_path: None,
            sentence_audio_path: None,
            image_path: None,
        }
    }

    /// No input makes only the second write fail: both writes take the same
    /// instant, so a date the store refuses already fails the capture. A
    /// trigger that refuses `srs_state` inserts does, and the capture inserted
    /// just before must not survive it.
    #[test]
    fn introducing_writes_the_card_and_its_state_together_or_not_at_all() {
        let mut store = Store::open_in_memory().unwrap();
        store.replace_curriculum(&[curriculum_word()]).unwrap();
        let scheduler = SrsScheduler::default();
        let clock = FixedClock::utc(Utc.with_ymd_and_hms(2026, 10, 31, 8, 0, 0).unwrap());
        store
            .conn()
            .execute_batch(
                "CREATE TRIGGER refuse_srs_state BEFORE INSERT ON srs_state \
                 BEGIN SELECT RAISE(ABORT, 'srs_state refused by the test'); END",
            )
            .unwrap();

        let refused = introduce_new_item(&store, &scheduler, "学习", "de", &clock);
        assert!(matches!(refused, Err(StoreError::Sqlite(_))), "{refused:?}");
        assert!(store.list_captures(None).unwrap().is_empty());
        assert_eq!(store.count_srs_states().unwrap(), 0);

        store
            .conn()
            .execute_batch("DROP TRIGGER refuse_srs_state")
            .unwrap();
        let card_id = introduce_new_item(&store, &scheduler, "学习", "de", &clock).unwrap();
        assert_eq!(store.list_captures(None).unwrap().len(), 1);
        assert_eq!(
            store.get_srs_state(card_id).unwrap(),
            Some(scheduler.initial_state(card_id, clock.now()))
        );
    }
}
