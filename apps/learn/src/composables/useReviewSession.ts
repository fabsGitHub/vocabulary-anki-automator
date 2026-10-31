/**
 * One pass over today's queue (Milestone L3): the review view's state.
 *
 * The queue is walked in the order `getDailyQueue` returns it: every due
 * review first, then today's new words. A new word is introduced with
 * `startNewItem` when the pass reaches it, never earlier, and is then
 * reviewed like any card; the call is idempotent, so a retry or a rebuilt
 * queue never duplicates it. Progress, XP and the streak are the backend's
 * business (docs/architecture.md § 7.1 and § 7.3): it counts a card's first
 * review as a new item and every other review as a review. This state only
 * adds up the `xp_awarded` of each rating and keeps the latest streak for
 * the summary.
 *
 * A pass ends after its last item. Cards that came due again meanwhile
 * (learning steps, "Again") are picked up by starting a new pass over a
 * fresh queue. Every pass has a number, and an answer that arrives for an
 * older pass is dropped, so a rebuilt queue never mixes with the old one.
 */

import { computed, ref } from 'vue'

import type { Clock } from '../context'
import { errorMessage } from '../domain/errors'
import type {
  PlatformAdapter,
  QueueItem,
  Rating,
  RatingPreview,
  StoredCapture,
  StreakSummary,
} from '../platform/types'

export type ReviewPhase = 'loading' | 'error' | 'empty' | 'card' | 'complete'

/** The card on screen. */
export interface CurrentCard {
  item: QueueItem
  cardId: number
  capture: StoredCapture
  previews: RatingPreview[]
  /**
   * When the previews were computed. A learning step's length is its due
   * date minus this, however long the learner looks at the front first.
   */
  previewedAt: number
}

/** What failed: building the queue, loading or introducing the current card, or saving a rating. */
export interface ReviewError {
  step: 'queue' | 'card' | 'rate'
  message: string
}

export function useReviewSession(platform: PlatformAdapter, clock: Clock) {
  const phase = ref<ReviewPhase>('loading')
  const queue = ref<QueueItem[]>([])
  const index = ref(0)
  const current = ref<CurrentCard | null>(null)
  const revealed = ref(false)
  const busy = ref(false)
  const error = ref<ReviewError | null>(null)
  const xpEarned = ref(0)
  const rated = ref(0)
  const streak = ref<StreakSummary | null>(null)
  const goalReached = ref(false)
  let pass = 0

  const total = computed(() => queue.value.length)
  const isNew = computed(() => current.value?.item.kind === 'new')

  /** Starts a pass over a freshly built queue. */
  async function start(): Promise<void> {
    const run = ++pass
    phase.value = 'loading'
    error.value = null
    current.value = null
    revealed.value = false
    busy.value = false
    xpEarned.value = 0
    rated.value = 0
    goalReached.value = false
    streak.value = null
    try {
      const items = await platform.getDailyQueue()
      if (run !== pass) return
      queue.value = items
      index.value = 0
      if (items.length === 0) {
        phase.value = 'empty'
        return
      }
      phase.value = 'card'
      await loadCard(run)
    } catch (err) {
      if (run !== pass) return
      error.value = { step: 'queue', message: errorMessage(err) }
      phase.value = 'error'
    }
  }

  async function loadCard(run: number): Promise<void> {
    const item = queue.value[index.value]
    current.value = null
    revealed.value = false
    busy.value = true
    error.value = null
    try {
      const cardId = item.kind === 'new' ? await platform.startNewItem(item.hanzi) : item.card_id
      const [capture, previews] = await Promise.all([
        platform.getCard(cardId),
        // The intervals only label the buttons (the core itself has none for
        // a state FSRS cannot read), so without them the card is still rated.
        platform.previewRatings(cardId).catch((): RatingPreview[] => []),
      ])
      if (run !== pass) return
      current.value = { item, cardId, capture, previews, previewedAt: clock.now() }
    } catch (err) {
      if (run === pass) error.value = { step: 'card', message: errorMessage(err) }
    } finally {
      if (run === pass) busy.value = false
    }
  }

  async function advance(run: number): Promise<void> {
    index.value += 1
    if (index.value < queue.value.length) {
      await loadCard(run)
      return
    }
    current.value = null
    phase.value = 'complete'
    if (streak.value === null) await loadStreak(run)
  }

  /** The streak for a summary no rating reported one for (every card skipped). */
  async function loadStreak(run: number): Promise<void> {
    try {
      const stats = await platform.getStats()
      if (run === pass) streak.value = stats.streak
    } catch {
      // The summary shows the streak only when it is known.
    }
  }

  function reveal(): void {
    if (current.value && !busy.value) revealed.value = true
  }

  async function rate(rating: Rating): Promise<void> {
    const card = current.value
    if (!card || !revealed.value || busy.value) return
    const run = pass
    busy.value = true
    error.value = null
    try {
      const result = await platform.submitReviewRating(card.cardId, rating)
      if (run !== pass) return
      xpEarned.value += result.xp_awarded
      rated.value += 1
      streak.value = result.streak
      if (result.session_completed) goalReached.value = true
    } catch (err) {
      if (run !== pass) return
      error.value = { step: 'rate', message: errorMessage(err) }
      busy.value = false
      return
    }
    busy.value = false
    await advance(run)
  }

  /** Moves past a card that could not be loaded or rated; it stays due. */
  async function skip(): Promise<void> {
    if (busy.value || phase.value !== 'card' || !error.value) return
    await advance(pass)
  }

  /** Repeats what failed. A failed rating is repeated by rating again. */
  async function retry(): Promise<void> {
    if (busy.value || !error.value) return
    if (error.value.step === 'queue') await start()
    else if (error.value.step === 'card') await loadCard(pass)
  }

  return {
    phase,
    queue,
    index,
    total,
    current,
    isNew,
    revealed,
    busy,
    error,
    xpEarned,
    rated,
    streak,
    goalReached,
    start,
    reveal,
    rate,
    skip,
    retry,
  }
}

export type ReviewSessionState = ReturnType<typeof useReviewSession>
