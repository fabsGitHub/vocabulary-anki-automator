/**
 * `vocab_core::srs::daily`: the daily onboarding prompt and the day's
 * session (Milestone L2), with the decisions docs/architecture.md § 7.1
 * records:
 *
 * - Dismissing the prompt means "reviews only today": a quota of 0 and no
 *   further prompt that day, until the menu action reopens it; a later
 *   answer replaces the quota and keeps the progress made.
 * - The day's targets are fixed when the session is set up:
 *   `reviews_due_count` is the queue's reviews at that moment, without the
 *   cards never rated, whose first rating counts as a new item.
 * - A session completes when both targets are met and at least one review
 *   or new word was done, whichever write gets there first. A day with
 *   nothing done never completes, and `completed_at` is never cleared.
 * - Progress made before the day's prompt still counts: it opens the day's
 *   session with a quota of 0, without answering the prompt.
 */

import type * as ipc from '../types'
import { dueToday } from './queue'
import { type DailySessionRow, type Store, dailySessionToJson } from './store'
import { type Clock, type LocalNow, parseDate, readLocalNow } from './time'

/** `LAST_PROMPTED_KEY`: the local date the prompt was last answered or dismissed. */
export const LAST_PROMPTED_KEY = 'last_prompted_date'

/** `fsrs_state` of a card that has never been rated. */
const NEVER_REVIEWED = 'new'

const U32_MAX = 4_294_967_295

/** `should_prompt`: not answered or dismissed on `today` (or ever). */
export function shouldPrompt(lastPrompted: ipc.IsoDate | null, today: ipc.IsoDate): boolean {
  return lastPrompted !== today
}

/** `DailyPromptState`. */
export interface DailyPromptState {
  needs_prompt: boolean
  today: ipc.IsoDate
  session: DailySessionRow | null
}

/** `SessionProgress`. */
export type SessionProgress = 'review' | 'new_item'

/**
 * `daily_prompt_state`: whether to prompt now, and today's session if there
 * is one. A stored prompt date that does not parse counts as never prompted.
 */
export async function dailyPromptState(store: Store, clock: Clock): Promise<DailyPromptState> {
  const { today } = readLocalNow(clock)
  return {
    needs_prompt: shouldPrompt(await lastPrompted(store), today),
    today,
    session: await store.getDailySession(today),
  }
}

/**
 * `submit_daily_quota`: today's session asks for `newCount` new items, and
 * the prompt is not shown again today. The first answer of a day sets the
 * session up; a later one changes only `requested_new_count`.
 */
export async function submitDailyQuota(
  store: Store,
  newCount: number,
  clock: Clock,
): Promise<DailySessionRow> {
  const now = readLocalNow(clock)
  return store.withTransaction(async (store) => {
    const existing = await store.getDailySession(now.today)
    const session = existing
      ? { ...existing, requested_new_count: newCount }
      : await openSession(store, now, newCount)
    const saved = await saveSession(store, session, now.at)
    await markPrompted(store, now.today)
    return saved
  })
}

/**
 * `dismiss_daily_prompt`: reviews only today (a quota of 0), unless today's
 * session was set up already, which then stays as it is.
 */
export async function dismissDailyPrompt(store: Store, clock: Clock): Promise<DailySessionRow> {
  const now = readLocalNow(clock)
  return store.withTransaction(async (store) => {
    const session = (await store.getDailySession(now.today)) ?? (await openSession(store, now, 0))
    const saved = await saveSession(store, session, now.at)
    await markPrompted(store, now.today)
    return saved
  })
}

/** `reopen_daily_prompt`: forgets that today was prompted; the session stays. */
export async function reopenDailyPrompt(store: Store, _clock: Clock): Promise<void> {
  await store.deleteState(LAST_PROMPTED_KEY)
}

/**
 * `record_session_progress`: counts one review or new item towards today's
 * session (opening it with a quota of 0 if the day changed mid-session).
 */
export async function recordSessionProgress(
  store: Store,
  kind: SessionProgress,
  clock: Clock,
): Promise<DailySessionRow> {
  const now = readLocalNow(clock)
  return store.withTransaction(async (store) => {
    const session = (await store.getDailySession(now.today)) ?? (await openSession(store, now, 0))
    if (kind === 'review') {
      session.reviews_completed = Math.min(session.reviews_completed + 1, U32_MAX)
    } else {
      session.new_items_completed = Math.min(session.new_items_completed + 1, U32_MAX)
    }
    return saveSession(store, session, now.at)
  })
}

/** A prompt state's serde shape. */
export function dailyPromptStateToJson(state: DailyPromptState): ipc.DailyPromptState {
  return {
    needs_prompt: state.needs_prompt,
    today: state.today,
    session: state.session === null ? null : dailySessionToJson(state.session),
  }
}

async function lastPrompted(store: Store): Promise<ipc.IsoDate | null> {
  const stored = await store.getState(LAST_PROMPTED_KEY)
  return stored === null ? null : parseDate(stored)
}

async function markPrompted(store: Store, today: ipc.IsoDate): Promise<void> {
  await store.setState(LAST_PROMPTED_KEY, today)
}

/**
 * `open_session`: a fresh session for today, with the reviews due now as its
 * target. A due card that was never rated is not one of them: its first
 * rating counts as a new item.
 */
async function openSession(
  store: Store,
  now: LocalNow,
  requestedNewCount: number,
): Promise<DailySessionRow> {
  const reviewsDue = (await dueToday(store, now)).filter(
    (state) => state.fsrs_state !== NEVER_REVIEWED,
  ).length
  return {
    session_date: now.today,
    requested_new_count: requestedNewCount,
    reviews_due_count: Math.min(reviewsDue, U32_MAX),
    reviews_completed: 0,
    new_items_completed: 0,
    completed_at: null,
  }
}

/** `save_session`: stores `session`, completing it at `at` if this write meets both targets after some work. */
async function saveSession(
  store: Store,
  session: DailySessionRow,
  at: number,
): Promise<DailySessionRow> {
  const targetsMet =
    session.reviews_completed >= session.reviews_due_count &&
    session.new_items_completed >= session.requested_new_count
  const didWork = session.reviews_completed > 0 || session.new_items_completed > 0
  const saved =
    targetsMet && didWork && session.completed_at === null
      ? { ...session, completed_at: at }
      : session
  await store.upsertDailySession(saved)
  return saved
}
