/**
 * The day's review session as the review screen drives it: vocab-learn's
 * `session` module (apps/learn/src-tauri/src/session.rs) on top of the
 * ported core, so that both builds count a rating the same way
 * (docs/architecture.md § 7.1, § 7.3).
 *
 * - **A rating is one transaction**: the FSRS review, its XP, the session
 *   progress it counts as and, when it completes the session, the
 *   completion bonus and the streak.
 * - **Progress is counted on reviews.** A card's first review (its log's
 *   `state_before` is `new`) counts as a new item and earns `XP_NEW_ITEM`;
 *   every other review counts as a review and earns `XP_REVIEW`.
 *   Introducing a word counts nothing.
 * - **Completion is noticed by the write that causes it.**
 *   {@link trackCompletion} compares today's session before and after and
 *   calls `onSessionCompleted` exactly when `completed_at` first appears.
 * - **The queue offers what is left of today's quota**: the requested new
 *   words, minus those done, minus the cards still waiting for their first
 *   review. Without today's session the queue holds reviews only.
 */

import type * as ipc from '../types'
import {
  XP_NEW_ITEM,
  XP_REVIEW,
  awardXp,
  onSessionCompleted,
  reason,
  statsSummary,
  type StreakSummary,
} from './gamification'
import { recordSessionProgress } from './daily'
import { type QueueItem, buildDailyQueueFrom, submitReview } from './queue'
import { type ReviewRating, type SrsScheduler, type SrsStateRow, srsStateToJson } from './scheduler'
import { type DailySessionRow, type Store, dailySessionToJson } from './store'
import { type Clock, localDayBounds, today as clockToday } from './time'

/** `fsrs_state` of a card that has never been reviewed. */
const NEVER_REVIEWED = 'new'

const U32_MAX = 4_294_967_295

/** `ReviewResult` (`../types`), the state's instants in milliseconds. */
export interface ReviewResult {
  next: SrsStateRow
  xp_awarded: number
  session: DailySessionRow | null
  session_completed: boolean
  streak: StreakSummary
}

/** Today's queue: every card due before the end of the local day, then what is left of the quota. */
export async function dailyQueue(store: Store, clock: Clock): Promise<QueueItem[]> {
  const today = clockToday(clock)
  const [, endOfToday] = localDayBounds(today, clock.localOffset())
  const due = await store.listSrsDueBefore(endOfToday)
  const newQuota = remainingNewQuota(await store.getDailySession(today), due)
  const curriculum = await store.curriculumInOrder()
  const learnedHanzi = await store.learnedChineseFronts()
  return buildDailyQueueFrom(due, curriculum, learnedHanzi, newQuota)
}

/** How many new curriculum words today's queue still offers, given the states due today. */
export function remainingNewQuota(
  session: DailySessionRow | null,
  due: readonly SrsStateRow[],
): number {
  if (session === null) return 0
  const waiting = Math.min(
    due.filter((state) => state.fsrs_state === NEVER_REVIEWED).length,
    U32_MAX,
  )
  return Math.max(0, session.requested_new_count - session.new_items_completed - waiting)
}

/**
 * Runs `write`, a core call that saves and returns today's session, and has
 * gamification award the completion bonus if `write` completed it. Returns
 * the session and, in that case, the streak after the bonus.
 */
export async function trackCompletion(
  store: Store,
  clock: Clock,
  write: (store: Store) => Promise<DailySessionRow>,
): Promise<[DailySessionRow, StreakSummary | null]> {
  return store.withTransaction(async (store) => {
    const before = await store.getDailySession(clockToday(clock))
    const session = await write(store)
    const streak = justCompleted(before, session) ? await onSessionCompleted(store, clock) : null
    return [session, streak]
  })
}

/** Whether `after` is the first save of its session that has `completed_at`. */
export function justCompleted(before: DailySessionRow | null, after: DailySessionRow): boolean {
  const wasCompleted =
    before !== null && before.session_date === after.session_date && before.completed_at !== null
  return after.completed_at !== null && !wasCompleted
}

/**
 * Rates the scheduled card `cardId` and does everything the rating counts
 * for, in one transaction. Throws, writing nothing, when the core refuses
 * the review.
 */
export async function rate(
  store: Store,
  scheduler: SrsScheduler,
  cardId: number,
  rating: ReviewRating,
  clock: Clock,
): Promise<ReviewResult> {
  return store.withTransaction(async (store) => {
    const today = clockToday(clock)
    const xpBefore = await store.xpForDate(today)
    const outcome = await submitReview(store, scheduler, cardId, rating, clock)
    const first = outcome.log.state_before === NEVER_REVIEWED
    await awardXp(
      store,
      first ? XP_NEW_ITEM : XP_REVIEW,
      first ? reason.NEW_ITEM : reason.REVIEW,
      clock,
    )
    const [session, completed] = await trackCompletion(store, clock, (store) =>
      recordSessionProgress(store, first ? 'new_item' : 'review', clock),
    )
    const streak = completed ?? (await statsSummary(store, clock)).streak
    return {
      next: outcome.next,
      xp_awarded: (await store.xpForDate(today)) - xpBefore,
      session,
      session_completed: completed !== null,
      streak,
    }
  })
}

/** A review result's serde shape. */
export function reviewResultToJson(result: ReviewResult): ipc.ReviewResult {
  return {
    next: srsStateToJson(result.next),
    xp_awarded: result.xp_awarded,
    session: result.session === null ? null : dailySessionToJson(result.session),
    session_completed: result.session_completed,
    streak: { ...result.streak },
  }
}
