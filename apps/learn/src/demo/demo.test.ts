/**
 * The live demo (docs/live-demo.md): the history it starts with, kept and
 * replaced when it should be, and its clock. Everything runs on the web
 * build's own adapter over memory, with the bundled curriculum, at 10:25 on
 * 2026-10-31 in UTC+1.
 */

import { describe, expect, it } from 'vitest'

import bundledCurriculumText from '../../../../crates/vocab-core/data/hsk_curriculum.json?raw'
import type { PlatformAdapter, QueueItem } from '../platform/types'
import { parseCurriculum } from '../platform/web/curriculum'
import { MemoryStorage, type Storage, type Transaction } from '../platform/web/storage'
import { Store } from '../platform/web/store'
import { DAY_SHIFT_KEY, PREPARED_KEY, STALE_AFTER_DAYS, openDemo, type DemoOptions } from './demo'
import { CURRICULUM_PREFIX, DEMO_WORDS, playHistory } from './history'

const curriculum = parseCurriculum(JSON.parse(bundledCurriculumText))
const NOW = Date.UTC(2026, 9, 31, 9, 25)
const OFFSET = 3600
const DAY = 86_400_000
/** The two words captured after yesterday's session. */
const WAITING = ['咖啡馆', 'Fernweh']

function options(storage: Storage, now = NOW): DemoOptions {
  return {
    openStorage: async () => storage,
    now: () => now,
    localOffset: () => OFFSET,
    loadCurriculum: async () => curriculum,
  }
}

function state(storage: Storage, key: string): Promise<string | null> {
  return storage.transaction((tx) => new Store(tx).getState(key))
}

async function fronts(platform: PlatformAdapter, items: QueueItem[]): Promise<string[]> {
  return Promise.all(
    items.map(async (item) =>
      item.kind === 'new' ? item.hanzi : (await platform.getCard(item.card_id)).card.front,
    ),
  )
}

/** Memory whose next transaction can be made to fail. */
class FlakyStorage extends MemoryStorage {
  failNext = false

  override transaction<T>(body: (tx: Transaction) => Promise<T>): Promise<T> {
    if (this.failNext) {
      this.failNext = false
      return Promise.reject(new Error('the disk is full'))
    }
    return super.transaction(body)
  }
}

describe('the demo history', () => {
  it('leaves a learner after ten evenings of practice, asked nothing yet today', async () => {
    const { platform } = await openDemo(options(new MemoryStorage()))

    expect(await platform.getDailyPromptState()).toEqual({
      needs_prompt: true,
      today: '2026-10-31',
      session: null,
    })
    const stats = await platform.getStats()
    expect(stats.streak).toEqual({
      current: 10,
      longest: 10,
      // Earned when the streak reached seven days.
      freezes_available: 1,
      completed_today: false,
    })
    expect(stats.xp_today).toBe(0)
    expect(stats.xp_total).toBeGreaterThan(0)
    expect(stats.due_today).toBeGreaterThan(0)

    // 43 new words in all: every captured word but the two still waiting,
    // and the first 35 HSK words.
    const learned = await platform.listLearnedCards()
    expect(learned).toHaveLength(43)
    const captured = DEMO_WORDS.map((word) => word.front).filter(
      (front) => !WAITING.includes(front),
    )
    expect(learned.filter((card) => !card.hsk_level).map((card) => card.front)).toEqual(captured)
    expect(learned.filter((card) => card.hsk_level).map((card) => card.front)).toEqual(
      curriculum.slice(0, 35).map((item) => item.hanzi),
    )
  })

  it('offers what is due, then the two waiting captures, then the next HSK words', async () => {
    const { platform } = await openDemo(options(new MemoryStorage()))
    await platform.submitDailyQuota(5)

    const queue = await platform.getDailyQueue()
    const reviews = queue.filter((item) => item.kind === 'review')
    expect(reviews.length).toBeGreaterThan(0)
    expect(queue.slice(0, reviews.length)).toEqual(reviews)
    const fresh = queue.slice(reviews.length)
    expect(fresh.map((item) => item.kind)).toEqual([
      'new_capture',
      'new_capture',
      'new',
      'new',
      'new',
    ])
    expect(await fronts(platform, fresh)).toEqual([
      ...WAITING,
      ...curriculum.slice(35, 38).map((item) => item.hanzi),
    ])
  })

  it('makes a lesson of the five words captured in each language', async () => {
    const { platform } = await openDemo(options(new MemoryStorage()))
    const lessons = await platform.listLessons()
    expect(lessons.map((lesson) => [lesson.source_lang, lesson.target_lang])).toEqual([
      ['zh-CN', 'en'],
      ['de', 'en'],
    ])
    for (const lesson of lessons) {
      const words = DEMO_WORDS.filter((word) => word.lang === lesson.source_lang)
      expect(lesson.cards.map((card) => card.word)).toEqual(words.map((word) => word.front))
      expect(lesson.cards.map((card) => card.sentence_target)).toEqual(
        words.map((word) => word.example[1]),
      )
    }
  })

  it('builds exercises from the learned words only, sentences included', async () => {
    const { platform } = await openDemo(options(new MemoryStorage()))
    const learned = await platform.listLearnedCards()
    for (const [seed, card] of learned.entries()) {
      await expect(
        platform.nextExercise(card.card_id, 'multiple_choice', seed),
      ).resolves.toMatchObject({ kind: 'multiple_choice' })
      if (card.example_source !== '') {
        await expect(
          platform.nextExercise(card.card_id, 'sentence_builder', seed),
        ).resolves.toMatchObject({ kind: 'sentence_builder' })
      }
    }

    await platform.submitDailyQuota(5)
    const waiting = (await platform.getDailyQueue()).filter((item) => item.kind === 'new_capture')
    expect(waiting).toHaveLength(2)
    for (const item of waiting) {
      if (item.kind !== 'new_capture') continue
      await expect(platform.nextExercise(item.card_id, 'multiple_choice', 1)).rejects.toMatch(
        'not learned',
      )
      expect(learned.some((card) => card.card_id === item.card_id)).toBe(false)
    }
  })

  it('is the same for every visitor, in every time zone', async () => {
    const play = (offsetSeconds: number) =>
      playHistory({ today: '2026-10-31', offsetSeconds, curriculum })
    const ratings = (rows: Awaited<ReturnType<typeof play>>) =>
      (rows.get('review_log') ?? []).map(([, row]) => (row as { rating: number }).rating)

    expect(await play(OFFSET)).toEqual(await play(OFFSET))
    expect(ratings(await play(-5 * 3600))).toEqual(ratings(await play(OFFSET)))
    expect(ratings(await play(OFFSET)).length).toBeGreaterThan(43)
  })

  it('needs no more of the word list than it is played with, and no captured word is on it', async () => {
    const hanzi = new Set(curriculum.map((item) => item.hanzi))
    expect(DEMO_WORDS.filter((word) => hanzi.has(word.front))).toEqual([])

    const rows = await playHistory({ today: '2026-10-31', offsetSeconds: OFFSET, curriculum })
    const fromList = (rows.get('captures') ?? [])
      .map(([, row]) => row as { origin: string; front: string })
      .filter((row) => row.origin === 'curriculum')
    expect(fromList.length).toBeLessThan(CURRICULUM_PREFIX)
    // Not stored by the history: the demo stores the whole list.
    expect(rows.has('hsk_curriculum')).toBe(false)
    expect((rows.get('app_state') ?? []).map(([key]) => key)).not.toContain(
      'curriculum_fingerprint',
    )
  })
})

describe('the stored demo', () => {
  it('keeps what the visitor did', async () => {
    const storage = new MemoryStorage()
    const first = await openDemo(options(storage))
    await first.platform.submitDailyQuota(3)
    const [item] = await first.platform.getDailyQueue()
    if (item.kind !== 'review') throw new Error('the queue should start with a review')
    await first.platform.submitReviewRating(item.card_id, 3)

    const again = await openDemo(options(storage, NOW + 3_600_000))
    const prompt = await again.platform.getDailyPromptState()
    expect(prompt.needs_prompt).toBe(false)
    expect(prompt.session).toMatchObject({ requested_new_count: 3, reviews_completed: 1 })
    expect(await state(storage, PREPARED_KEY)).toBe('2026-10-31')
  })

  it('starts over by itself when its history is a week old', async () => {
    const storage = new MemoryStorage()
    await openDemo(options(storage))

    await openDemo(options(storage, NOW + (STALE_AFTER_DAYS - 1) * DAY))
    expect(await state(storage, PREPARED_KEY)).toBe('2026-10-31')

    const fresh = await openDemo(options(storage, NOW + STALE_AFTER_DAYS * DAY))
    expect(await state(storage, PREPARED_KEY)).toBe('2026-11-07')
    expect(await fresh.platform.getDailyPromptState()).toMatchObject({
      needs_prompt: true,
      today: '2026-11-07',
    })
    expect((await fresh.platform.getStats()).streak.current).toBe(10)
  })

  it('runs in memory where the browser refuses IndexedDB', async () => {
    // jsdom has no IndexedDB, as some private windows do not.
    const { controller, platform } = await openDemo({
      now: () => NOW,
      localOffset: () => OFFSET,
      loadCurriculum: async () => curriculum,
    })
    expect(controller.persistent).toBe(false)
    expect((await platform.getDailyPromptState()).needs_prompt).toBe(true)

    const refused = await openDemo({
      ...options(new MemoryStorage()),
      openStorage: async () => {
        throw new Error('blocked')
      },
    })
    expect(refused.controller.persistent).toBe(false)
  })
})

describe("the demo's clock", () => {
  it('moves the app one day ahead per "Next day", and keeps the move', async () => {
    const storage = new MemoryStorage()
    const { controller, platform, clock } = await openDemo(options(storage))
    expect(controller.persistent).toBe(true)

    await controller.nextDay()
    expect(controller.dayShift.value).toBe(1)
    expect(controller.epoch.value).toBe(1)
    expect(clock.now()).toBe(NOW + DAY)
    expect(await platform.getDailyPromptState()).toMatchObject({
      needs_prompt: true,
      today: '2026-11-01',
    })
    // Yesterday (the real today) was skipped: the freeze covers it.
    expect((await platform.getStats()).streak).toMatchObject({ current: 11, freezes_available: 0 })

    await controller.nextDay()
    const reopened = await openDemo(options(storage, NOW + 60_000))
    expect(reopened.controller.dayShift.value).toBe(2)
    expect(await reopened.platform.getDailyPromptState()).toMatchObject({ today: '2026-11-02' })
  })

  it('starts over with a fresh history on the real date', async () => {
    const storage = new MemoryStorage()
    const { controller, platform } = await openDemo(options(storage))
    await controller.nextDay()
    await platform.submitDailyQuota(10)

    await controller.reset()
    expect(controller.dayShift.value).toBe(0)
    expect(controller.epoch.value).toBe(2)
    expect(await state(storage, DAY_SHIFT_KEY)).toBeNull()
    expect(await platform.getDailyPromptState()).toEqual({
      needs_prompt: true,
      today: '2026-10-31',
      session: null,
    })
    expect((await platform.getStats()).streak.current).toBe(10)
  })

  it('reports an action that failed and stays where it was', async () => {
    const storage = new FlakyStorage()
    const { controller, platform } = await openDemo(options(storage))

    storage.failNext = true
    await controller.nextDay()
    expect(controller.error.value).toBe('the disk is full')
    expect(controller.busy.value).toBe(false)
    expect(controller.dayShift.value).toBe(0)
    expect(controller.epoch.value).toBe(0)
    expect((await platform.getDailyPromptState()).today).toBe('2026-10-31')

    await controller.nextDay()
    expect(controller.error.value).toBeNull()
    expect(controller.dayShift.value).toBe(1)
  })

  it('runs one action at a time', async () => {
    const { controller } = await openDemo(options(new MemoryStorage()))
    await Promise.all([controller.nextDay(), controller.nextDay(), controller.reset()])
    expect(controller.dayShift.value).toBe(1)
    expect(controller.epoch.value).toBe(1)
  })
})
