/**
 * `vocab_core::srs::queue`: the daily queue (Milestone L3) and the writes a
 * session makes with its items: starting a new card and submitting a review.
 *
 * Reviews come first, all of them, and "due today" ends at the end of the
 * learner's local day. New items follow, from one quota and two sources:
 * first every capture or Anki import without an `srs_state` row, oldest
 * capture first, which {@link introduceCapture} starts; then curriculum
 * words in strict `curriculum_order`, skipping every *learned* hanzi (a
 * Chinese capture with an `srs_state` row). A capture whose front is a
 * curriculum word is left to the curriculum: it is offered when that word's
 * turn comes, and {@link introduceNewItem} adopts the Chinese capture
 * instead of duplicating it. A quota of zero takes zero items from the same
 * sequence; it is not a mode of its own.
 */

import type * as ipc from '../types'
import { StoreError } from './errors'
import { CHINESE, normalizeLang } from './language'
import { type Card, type CurriculumItem, newCard } from './model'
import type { ReviewOutcome, ReviewRating, SrsScheduler, SrsStateRow } from './scheduler'
import type { Store, UnscheduledCapture } from './store'
import { debugString, trim } from './text'
import {
  type Clock,
  type LocalNow,
  endOfToday,
  formatInstant,
  nanosFromMillis,
  readLocalNow,
} from './time'

/** `captures.script_kind` of Chinese cards. */
const CHINESE_SCRIPT_KIND = 'chinese'
const GERMAN = 'de'
const ENGLISH = 'en'

/** `QueueItem`, the due date in milliseconds. */
export type QueueItem =
  | { kind: 'review'; card_id: number; due_date: number }
  | { kind: 'new'; hanzi: string; curriculum_order: number }
  | { kind: 'new_capture'; card_id: number }

/**
 * `build_daily_queue_from`: every state in `due`, by `due_date` and then
 * `card_id`, followed by the first `newQuota` new items. Those are the
 * captures in `unscheduled` whose front is not the hanzi of a `curriculum`
 * item, by `captured_at` and then `card_id`, and after them the items of
 * `curriculum` in `curriculum_order` whose hanzi is not in `learnedHanzi`.
 */
export function buildDailyQueueFrom(
  due: readonly SrsStateRow[],
  unscheduled: readonly UnscheduledCapture[],
  curriculum: readonly CurriculumItem[],
  learnedHanzi: ReadonlySet<string>,
  newQuota: number,
): QueueItem[] {
  const reviews = [...due].sort((a, b) => a.due_date - b.due_date || a.card_id - b.card_id)
  const curriculumWords = new Set(curriculum.map((item) => item.hanzi))
  const captures = unscheduled
    .filter((capture) => !curriculumWords.has(capture.front))
    .sort((a, b) => a.captured_at - b.captured_at || a.card_id - b.card_id)
  const inOrder = [...curriculum].sort((a, b) => a.curriculum_order - b.curriculum_order)
  const queue: QueueItem[] = reviews.map((state) => ({
    kind: 'review',
    card_id: state.card_id,
    due_date: state.due_date,
  }))
  const newItems: QueueItem[] = captures.map((capture) => ({
    kind: 'new_capture',
    card_id: capture.card_id,
  }))
  for (const item of inOrder) {
    if (newItems.length >= newQuota) break
    if (learnedHanzi.has(item.hanzi)) continue
    newItems.push({ kind: 'new', hanzi: item.hanzi, curriculum_order: item.curriculum_order })
  }
  return queue.concat(newItems.slice(0, newQuota))
}

/**
 * `build_daily_queue`: every state due before the end of the local day,
 * then up to `newQuota` new items, the captures and imports without an
 * `srs_state` row before the curriculum words.
 */
export async function buildDailyQueue(
  store: Store,
  newQuota: number,
  clock: Clock,
): Promise<QueueItem[]> {
  const due = await dueToday(store, readLocalNow(clock))
  const unscheduled = await store.listUnscheduledCaptures()
  const curriculum = await store.curriculumInOrder()
  const learnedHanzi = await store.learnedChineseFronts()
  return buildDailyQueueFrom(due, unscheduled, curriculum, learnedHanzi, newQuota)
}

/** `due_today`: the review part of the queue at `now`. */
export function dueToday(store: Store, now: LocalNow): Promise<SrsStateRow[]> {
  return store.listSrsDueBefore(endOfToday(now))
}

/**
 * `introduce_new_item`: starts learning the curriculum word `hanzi` and
 * returns its card id. One transaction adds a `captures` row (`origin =
 * curriculum`) and a `new` `srs_state` row due now.
 *
 * Idempotent: when a Chinese capture of `hanzi` exists (introduced before,
 * or captured in Automator), the oldest one's id is returned, after giving
 * it a `new` state if it had none. Throws `StoreError::NotFound` when
 * `hanzi` is neither captured nor in the curriculum.
 */
export async function introduceNewItem(
  store: Store,
  scheduler: SrsScheduler,
  hanzi: string,
  targetLang: string,
  clock: Clock,
): Promise<number> {
  const now = readLocalNow(clock)
  const at = nanosFromMillis(now.at)
  return store.withTransaction(async (store) => {
    const existing = await store.findCaptureByFront(hanzi, CHINESE_SCRIPT_KIND)
    if (existing) {
      if ((await store.getSrsState(existing.id)) === null) {
        await store.upsertSrsState(scheduler.initialState(existing.id, at))
      }
      return existing.id
    }
    const item = await store.getCurriculumItem(hanzi)
    if (item === null) throw StoreError.notFound(`curriculum item ${debugString(hanzi)}`)
    const cardId = await store.insertCapture(
      curriculumCard(item, targetLang, now.at),
      'curriculum',
      false,
    )
    await store.upsertSrsState(scheduler.initialState(cardId, at))
    return cardId
  })
}

/**
 * `introduce_capture`: starts learning the stored card `cardId`, a capture
 * or import the queue offered as `new_capture`, and returns its id. It gets
 * a `new` `srs_state` row due now; its content stays as it was captured.
 *
 * Idempotent: a card that has a state already keeps it. Throws
 * `StoreError::NotFound` when no capture has this id.
 */
export async function introduceCapture(
  store: Store,
  scheduler: SrsScheduler,
  cardId: number,
  clock: Clock,
): Promise<number> {
  const now = readLocalNow(clock)
  return store.withTransaction(async (store) => {
    if ((await store.getCapture(cardId)) === null) {
      throw StoreError.notFound(`capture ${cardId}`)
    }
    if ((await store.getSrsState(cardId)) === null) {
      await store.upsertSrsState(scheduler.initialState(cardId, nanosFromMillis(now.at)))
    }
    return cardId
  })
}

/**
 * `submit_review`: rates card `cardId` now. One transaction loads its
 * state, runs FSRS, writes the next state and appends the review to
 * `review_log`; the returned log carries its id, and its `session_date` is
 * the learner's local date. Throws `StoreError::NotFound` for a card without
 * `srs_state` and `StoreError::Invalid` for a row FSRS cannot interpret,
 * writing nothing.
 */
export async function submitReview(
  store: Store,
  scheduler: SrsScheduler,
  cardId: number,
  rating: ReviewRating,
  clock: Clock,
): Promise<ReviewOutcome> {
  const now = readLocalNow(clock)
  return store.withTransaction(async (store) => {
    const state = await store.getSrsState(cardId)
    if (state === null) throw StoreError.notFound(`srs_state of card ${cardId}`)
    const outcome = scheduler.tryReview(state, rating, nanosFromMillis(now.at), now.today)
    await store.upsertSrsState(outcome.next)
    outcome.log.id = await store.insertReviewLog(outcome.log)
    return outcome
  })
}

/**
 * `curriculum_card`: `back` is the German meaning when `targetLang` is
 * German and the meaning is not blank, else the English one, else empty, and
 * `target_lang` is the language `back` is actually in. The curriculum's
 * example translations are English, so only an English card gets one.
 */
export function curriculumCard(item: CurriculumItem, targetLang: string, capturedAt: number): Card {
  const lang = normalizeLang(targetLang)
  const present = (meaning: string | null) =>
    meaning !== null && trim(meaning) !== '' ? meaning : null
  const german = lang === GERMAN ? present(item.meaning_de) : null
  const english = present(item.meaning_en)
  const [back, backLang] =
    german !== null ? [german, GERMAN] : english !== null ? [english, ENGLISH] : ['', lang]
  const card = newCard(item.hanzi, back, CHINESE, backLang, capturedAt)
  card.example_source = item.example_sentence_source ?? ''
  card.example_target = backLang === ENGLISH ? (item.example_sentence_target ?? '') : ''
  card.script_metadata = {
    kind: 'chinese',
    pinyin: item.pinyin,
    hsk_level: item.hsk_level,
    script_variant: 'simplified',
  }
  return card
}

/** A queue item's serde shape. */
export function queueItemToJson(item: QueueItem): ipc.QueueItem {
  return item.kind === 'review' ? { ...item, due_date: formatInstant(item.due_date) } : { ...item }
}
