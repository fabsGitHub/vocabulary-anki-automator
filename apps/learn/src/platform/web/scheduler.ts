/**
 * `vocab_core::srs::scheduler`: FSRS over `srs_state` rows (Milestone L1).
 *
 * The memory model is rs-fsrs's ({@link FSRS}); this module converts between
 * the store's rows and rs-fsrs cards and adds what rs-fsrs leaves to its
 * caller, with the core's rules:
 *
 * - **Whole milliseconds.** Every `now` is cut to its millisecond before
 *   rs-fsrs sees it, because the store keeps no more; the clock's reading is
 *   therefore taken in nanoseconds, as Rust's `DateTime` holds it.
 * - **Reviews do not travel back in time.** A review timed before the card's
 *   last one (a clock set back) happens at the last one's instant, so the
 *   days since the last review never turn negative.
 * - **Errors, not guesses.** {@link fsrsCardFromState} refuses a row FSRS
 *   cannot interpret, and `submit_review` with it.
 * - **`review_log.scheduled_days` is the interval the review set**, not
 *   rs-fsrs's previous one.
 */

import type * as ipc from '../types'
import { InvalidRating, StoreError } from './errors'
import { DEFAULT_PARAMETERS, FSRS, type FsrsCard, type FsrsState, type Parameters } from './fsrs'
import { debugString } from './text'
import { type EpochNanos, formatInstant, parseInstantMillis, wholeMillis } from './time'

/** `ReviewRating`: 1 = Again, 2 = Hard, 3 = Good, 4 = Easy. */
export type ReviewRating = ipc.Rating

/** `ReviewRating::ALL`, in the order the buttons show them. */
export const ALL_RATINGS: readonly ReviewRating[] = [1, 2, 3, 4]

/** `ReviewRating::try_from(u8)`. */
export function reviewRating(value: unknown): ReviewRating {
  if (value === 1 || value === 2 || value === 3 || value === 4) return value
  throw new InvalidRating(value)
}

/** The `srs_state.algorithm` this module writes and accepts. */
const ALGORITHM = 'fsrs'

/** One more than the largest count rs-fsrs's `i32` can still add one to. */
const I32_MAX = 2_147_483_647

/** `store::SrsStateRow`, instants in milliseconds. */
export interface SrsStateRow {
  card_id: number
  algorithm: string
  stability: number | null
  difficulty: number | null
  interval_days: number
  repetitions: number
  lapses: number
  /** `new`, `learning`, `review` or `relearning` (other values are refused). */
  fsrs_state: string
  elapsed_days: number
  due_date: number
  last_reviewed_at: number | null
  last_rating: number | null
}

/** `store::ReviewLogRow`, the instant in milliseconds. */
export interface ReviewLogRow {
  /** 0 until the store assigns one. */
  id: number
  card_id: number
  rating: number
  reviewed_at: number
  session_date: ipc.IsoDate
  state_before: string
  scheduled_days: number
  elapsed_days: number
}

/** `ReviewOutcome`. */
export interface ReviewOutcome {
  next: SrsStateRow
  log: ReviewLogRow
}

/** `RatingPreview`, the due date in milliseconds. */
export interface RatingPreview {
  rating: ReviewRating
  due_date: number
  interval_days: number
}

/** `SrsScheduler`: FSRS with fixed parameters, for every card. */
export class SrsScheduler {
  private readonly fsrs: FSRS

  constructor(params: Parameters = DEFAULT_PARAMETERS) {
    this.fsrs = new FSRS(params)
  }

  /** `initial_state`: a card never reviewed, `new` and due at `now`. */
  initialState(cardId: number, now: EpochNanos): SrsStateRow {
    return stateFromFsrsCard(cardId, unseenCard(now), null)
  }

  /**
   * `review`: the state after answering `rating` at `now`, and the review as
   * logged for `sessionDate`. Never fails: a row {@link fsrsCardFromState}
   * refuses is scheduled as a card never seen before.
   */
  review(
    state: SrsStateRow,
    rating: ReviewRating,
    now: EpochNanos,
    sessionDate: ipc.IsoDate,
  ): ReviewOutcome {
    const card = fsrsCardOrNull(state) ?? unseenCard(now)
    return this.reviewCard(state.card_id, card, rating, now, sessionDate)
  }

  /** `try_review`: {@link review}, refusing a row FSRS cannot interpret. */
  tryReview(
    state: SrsStateRow,
    rating: ReviewRating,
    now: EpochNanos,
    sessionDate: ipc.IsoDate,
  ): ReviewOutcome {
    return this.reviewCard(state.card_id, fsrsCardFromState(state), rating, now, sessionDate)
  }

  /** `preview`: each rating's due date and interval, Again to Easy; empty for a refused row. */
  preview(state: SrsStateRow, now: EpochNanos): RatingPreview[] {
    const card = fsrsCardOrNull(state)
    if (card === null) return []
    const outcomes = this.fsrs.repeat(card, reviewInstant(card, now))
    return ALL_RATINGS.flatMap((rating) => {
      const outcome = outcomes.get(rating)
      return outcome
        ? [{ rating, due_date: outcome.card.due, interval_days: outcome.card.scheduled_days }]
        : []
    })
  }

  private reviewCard(
    cardId: number,
    card: FsrsCard,
    rating: ReviewRating,
    now: EpochNanos,
    sessionDate: ipc.IsoDate,
  ): ReviewOutcome {
    const at = reviewInstant(card, now)
    const { card: reviewed, review_log: log } = this.fsrs.next(card, at, rating)
    const next = stateFromFsrsCard(cardId, reviewed, rating)
    return {
      next,
      log: {
        id: 0,
        card_id: cardId,
        rating,
        reviewed_at: at,
        session_date: sessionDate,
        state_before: log.state,
        scheduled_days: next.interval_days,
        elapsed_days: log.elapsed_days,
      },
    }
  }
}

/**
 * `fsrs_card_from_state`: the rs-fsrs card of a stored state, one field for
 * one field. A `new` card has no memory state (`stability`, `difficulty` and
 * `last_reviewed_at` are null; rs-fsrs gets zeros and the due date), every
 * other card has all three, positive and finite. Throws
 * `StoreError::Invalid` for another algorithm, an unknown state, a memory
 * state that contradicts the state, or a count rs-fsrs's `i32` cannot add
 * one to.
 */
export function fsrsCardFromState(state: SrsStateRow): FsrsCard {
  const invalid = (reason: string) =>
    StoreError.invalid(`srs_state row ${state.card_id}: ${reason}`)
  if (state.algorithm !== ALGORITHM) {
    throw invalid(`algorithm ${debugString(state.algorithm)} is not ${debugString(ALGORITHM)}`)
  }
  const fsrsState = parseState(state.fsrs_state)
  if (fsrsState === null) throw invalid(`unknown fsrs_state ${debugString(state.fsrs_state)}`)
  const count = (name: string, value: number) => {
    if (!Number.isInteger(value) || value < 0 || value >= I32_MAX) {
      throw invalid(`${name} ${value} is not a count rs-fsrs can add to`)
    }
    return value
  }
  const reps = count('repetitions', state.repetitions)
  const lapses = count('lapses', state.lapses)
  const { stability, difficulty, last_reviewed_at: lastReview } = state
  let memory: [number, number, number]
  if (fsrsState === 'new') {
    if (stability !== null || difficulty !== null || lastReview !== null) {
      throw invalid('a new card cannot have a stability, difficulty or last review')
    }
    memory = [0, 0, state.due_date]
  } else if (
    stability !== null &&
    difficulty !== null &&
    lastReview !== null &&
    isMemoryValue(stability) &&
    isMemoryValue(difficulty)
  ) {
    memory = [stability, difficulty, lastReview]
  } else {
    throw invalid(
      `a ${state.fsrs_state} card needs a positive stability and difficulty and a last review`,
    )
  }
  return {
    due: state.due_date,
    stability: memory[0],
    difficulty: memory[1],
    elapsed_days: state.elapsed_days,
    scheduled_days: state.interval_days,
    reps,
    lapses,
    state: fsrsState,
    last_review: memory[2],
  }
}

/** `state_from_fsrs_card`: the inverse of {@link fsrsCardFromState}. */
export function stateFromFsrsCard(
  cardId: number,
  card: FsrsCard,
  lastRating: number | null,
): SrsStateRow {
  const reviewed = card.state !== 'new'
  return {
    card_id: cardId,
    algorithm: ALGORITHM,
    stability: reviewed ? card.stability : null,
    difficulty: reviewed ? card.difficulty : null,
    interval_days: card.scheduled_days,
    repetitions: card.reps,
    lapses: card.lapses,
    fsrs_state: card.state,
    elapsed_days: card.elapsed_days,
    due_date: card.due,
    last_reviewed_at: reviewed ? card.last_review : null,
    last_rating: lastRating,
  }
}

/** A state row's serde shape. */
export function srsStateToJson(row: SrsStateRow): ipc.SrsState {
  return {
    card_id: row.card_id,
    algorithm: row.algorithm,
    stability: row.stability,
    difficulty: row.difficulty,
    interval_days: row.interval_days,
    repetitions: row.repetitions,
    lapses: row.lapses,
    fsrs_state: row.fsrs_state as ipc.SrsState['fsrs_state'],
    elapsed_days: row.elapsed_days,
    due_date: formatInstant(row.due_date),
    last_reviewed_at: row.last_reviewed_at === null ? null : formatInstant(row.last_reviewed_at),
    last_rating: row.last_rating as ipc.Rating | null,
  }
}

/** A state row from its serde shape. */
export function srsStateFromJson(value: ipc.SrsState): SrsStateRow {
  return {
    ...value,
    due_date: parseInstantMillis(value.due_date),
    last_reviewed_at:
      value.last_reviewed_at === null ? null : parseInstantMillis(value.last_reviewed_at),
  }
}

/** A review log row's serde shape. */
export function reviewLogToJson(log: ReviewLogRow): Record<string, unknown> {
  return { ...log, reviewed_at: formatInstant(log.reviewed_at) }
}

/** A review outcome's serde shape. */
export function reviewOutcomeToJson(outcome: ReviewOutcome): Record<string, unknown> {
  return { next: srsStateToJson(outcome.next), log: reviewLogToJson(outcome.log) }
}

/** A preview's serde shape. */
export function ratingPreviewToJson(preview: RatingPreview): ipc.RatingPreview {
  return { ...preview, due_date: formatInstant(preview.due_date) }
}

function fsrsCardOrNull(state: SrsStateRow): FsrsCard | null {
  try {
    return fsrsCardFromState(state)
  } catch (err) {
    if (err instanceof StoreError) return null
    throw err
  }
}

/** `unseen_card`: built without rs-fsrs's `Card::new()`, which reads the clock. */
function unseenCard(now: EpochNanos): FsrsCard {
  return {
    due: wholeMillis(now),
    stability: 0,
    difficulty: 0,
    elapsed_days: 0,
    scheduled_days: 0,
    reps: 0,
    lapses: 0,
    state: 'new',
    last_review: 0,
  }
}

/** `review_instant`: `now` in whole milliseconds, and no earlier than the previous review. */
function reviewInstant(card: FsrsCard, now: EpochNanos): number {
  const at = wholeMillis(now)
  return card.state === 'new' ? at : Math.max(at, card.last_review)
}

function isMemoryValue(value: number): boolean {
  return Number.isFinite(value) && value > 0
}

function parseState(name: string): FsrsState | null {
  return name === 'new' || name === 'learning' || name === 'review' || name === 'relearning'
    ? name
    : null
}
