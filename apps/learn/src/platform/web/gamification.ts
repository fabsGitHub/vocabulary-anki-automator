/**
 * `vocab_core::gamification`: the XP ledger, the daily streak and streak
 * freezes (Milestone I), tied to completed review sessions and never to raw
 * captures. Days are the learner's local calendar days.
 *
 * - **Streak:** consecutive local dates ending today, or yesterday while
 *   today's session is still open, that have a completed session or a
 *   freeze.
 * - **Freezes are spent lazily**, the next time the app asks
 *   ({@link applyStreakFreezes}, which {@link statsSummary} and
 *   {@link onSessionCompleted} run first), and only when they cover the
 *   whole gap. Today is never frozen.
 * - **Freezes are earned** one at a time, whenever a completed session
 *   brings the streak to a multiple of seven, up to {@link MAX_STREAK_FREEZES}.
 */

import type * as ipc from '../types'
import { StoreError } from './errors'
import type { Store } from './store'
import { debugString, trim } from './text'
import {
  type Clock,
  addDays,
  dateToDays,
  daysBetween,
  localDayBounds,
  today as clockToday,
  wholeMillis,
} from './time'

export const XP_REVIEW = 10
export const XP_NEW_ITEM = 15
/** Awarded once per local day, for the first completed session. */
export const XP_SESSION_COMPLETE = 50
export const XP_EXERCISE_CORRECT = 5
export const XP_LESSON_COMPLETE = 20

/** Most streak freezes held at once. */
export const MAX_STREAK_FREEZES = 2
/** A freeze is earned each time the streak reaches a multiple of this. */
export const STREAK_FREEZE_INTERVAL = 7
/** `app_state` key holding the number of freezes available. */
export const STREAK_FREEZES_KEY = 'streak_freezes_available'

/** `gamification::reason`: the `xp_ledger.reason` values. */
export const reason = {
  REVIEW: 'review',
  NEW_ITEM: 'new_item',
  SESSION_COMPLETE: 'session_complete',
  EXERCISE: 'exercise',
  LESSON: 'lesson',
} as const

/** `StreakSummary`. */
export type StreakSummary = ipc.StreakSummary

/** `StatsSummary`. */
export type StatsSummary = ipc.StatsSummary

/** `award_xp`: `amount` XP for `why` on today's local date; returns the ledger id. */
export function awardXp(store: Store, amount: number, why: string, clock: Clock): Promise<number> {
  return store.insertXp(clockToday(clock), amount, why, wholeMillis(clock.now()))
}

/**
 * `compute_streak`: `[current, longest]` over the dates that have a
 * completed session or a freeze. Dates after `today` (from a clock that was
 * set back) are ignored.
 */
export function computeStreak(
  completed: Iterable<ipc.IsoDate>,
  frozen: Iterable<ipc.IsoDate>,
  today: ipc.IsoDate,
): [number, number] {
  const days = [...new Set([...completed, ...frozen].map(dateToDays))].sort((a, b) => a - b)
  const last = dateToDays(today)
  let longest = 0
  let run = 0
  let previous: number | null = null
  for (const day of days) {
    if (day > last) break
    run = previous !== null && previous + 1 === day ? run + 1 : 1
    longest = Math.max(longest, run)
    previous = day
  }
  const current = previous !== null && (previous === last || previous + 1 === last) ? run : 0
  return [current, longest]
}

/**
 * `apply_streak_freezes`: freezes the days missed since the last completed
 * or frozen day, up to and including yesterday, and returns them; nothing
 * when there is no gap, nothing before it, or too few freezes for all of it.
 */
export async function applyStreakFreezes(store: Store, clock: Clock): Promise<ipc.IsoDate[]> {
  return store.withTransaction(async (store) => {
    const today = clockToday(clock)
    const covered = [
      ...(await store.listCompletedSessionDates()),
      ...(await store.listStreakFreezeDates()),
    ].filter((date) => dateToDays(date) < dateToDays(today))
    if (covered.length === 0) return []
    const last = covered.reduce((latest, date) =>
      dateToDays(date) > dateToDays(latest) ? date : latest,
    )
    const missed = daysBetween(today, last) - 1
    if (missed <= 0) return []
    const held = await freezesAvailable(store)
    if (missed > held) return []
    const days = Array.from({ length: missed }, (_, index) => addDays(last, index + 1))
    for (const day of days) await store.insertStreakFreeze(day)
    await setFreezesAvailable(store, held - missed)
    return days
  })
}

/**
 * `on_session_completed`: once today's session is complete, the first call
 * of the day awards {@link XP_SESSION_COMPLETE} and, when the streak has
 * just reached a multiple of {@link STREAK_FREEZE_INTERVAL}, one freeze.
 * Later calls that day, and calls while the session is open, change nothing.
 */
export async function onSessionCompleted(store: Store, clock: Clock): Promise<StreakSummary> {
  return store.withTransaction(async (store) => {
    await applyStreakFreezes(store, clock)
    const today = clockToday(clock)
    const summary = await streakSummary(store, today)
    if (!summary.completed_today || (await store.xpRecorded(today, reason.SESSION_COMPLETE))) {
      return summary
    }
    await store.insertXp(
      today,
      XP_SESSION_COMPLETE,
      reason.SESSION_COMPLETE,
      wholeMillis(clock.now()),
    )
    if (
      summary.current % STREAK_FREEZE_INTERVAL === 0 &&
      summary.freezes_available < MAX_STREAK_FREEZES
    ) {
      summary.freezes_available += 1
      await setFreezesAvailable(store, summary.freezes_available)
    }
    return summary
  })
}

/** `stats_summary`: XP, streak and learning counts, after spending due freezes. */
export async function statsSummary(store: Store, clock: Clock): Promise<StatsSummary> {
  return store.withTransaction(async (store) => {
    await applyStreakFreezes(store, clock)
    const today = clockToday(clock)
    const [, endOfToday] = localDayBounds(today, clock.localOffset())
    const dueToday = (await store.listSrsDueBefore(endOfToday)).length
    return {
      xp_total: await store.xpTotal(),
      xp_today: await store.xpForDate(today),
      streak: await streakSummary(store, today),
      learned_count: await store.countSrsStates(),
      due_today: dueToday,
    }
  })
}

async function streakSummary(store: Store, today: ipc.IsoDate): Promise<StreakSummary> {
  const completed = await store.listCompletedSessionDates()
  const frozen = await store.listStreakFreezeDates()
  const [current, longest] = computeStreak(completed, frozen, today)
  return {
    current,
    longest,
    freezes_available: await freezesAvailable(store),
    completed_today: completed.includes(today),
  }
}

/** The stored inventory, capped at {@link MAX_STREAK_FREEZES}; 0 when unset. */
async function freezesAvailable(store: Store): Promise<number> {
  const text = await store.getState(STREAK_FREEZES_KEY)
  if (text === null) return 0
  const held = parseU32(trim(text))
  if (held === null) {
    throw StoreError.invalid(`app_state ${STREAK_FREEZES_KEY} is not a count: ${debugString(text)}`)
  }
  return Math.min(held, MAX_STREAK_FREEZES)
}

async function setFreezesAvailable(store: Store, held: number): Promise<void> {
  await store.setState(STREAK_FREEZES_KEY, String(held))
}

/** `str::parse::<u32>`: ASCII digits after an optional `+`, at most `u32::MAX`. */
function parseU32(text: string): number | null {
  if (!/^\+?[0-9]+$/.test(text)) return null
  const value = Number(text)
  return value <= 4_294_967_295 ? value : null
}
