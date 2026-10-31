import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import type { QueueItem, StoredCapture } from '../platform/types'
import {
  deferred,
  fakePlatform,
  mountWith,
  pressKey,
  type FakePlatform,
} from '../test/fakePlatform'
import * as fx from '../test/fixtures'
import ReviewSession from './ReviewSession.vue'

const QUEUE: QueueItem[] = [
  { kind: 'review', card_id: 1, due_date: '2026-10-31T08:00:00.000Z' },
  { kind: 'review', card_id: 2, due_date: '2026-10-31T09:00:00.000Z' },
  { kind: 'new', hanzi: '学', curriculum_order: 7 },
]

const CARDS: Record<number, StoredCapture> = {
  1: fx.capture({ id: 1 }),
  2: fx.capture({
    id: 2,
    card: {
      front: 'serendipity',
      back: 'glücklicher Zufall',
      source_lang: 'en',
      target_lang: 'de',
      example_source: '',
      example_target: '',
      notes: 'Rare word.',
      script_metadata: { kind: 'none' },
    },
  }),
  30: fx.capture({
    id: 30,
    origin: 'curriculum',
    card: {
      front: '学',
      back: 'to learn',
      example_source: '',
      example_target: '',
      script_metadata: {
        kind: 'chinese',
        pinyin: 'xué',
        hsk_level: 1,
        script_variant: 'simplified',
      },
    },
  }),
}

let wrapper: VueWrapper | null = null

function reviewPlatform(): FakePlatform {
  const platform = fakePlatform()
  platform.getDailyQueue.mockResolvedValue(QUEUE)
  platform.startNewItem.mockResolvedValue(30)
  platform.getCard.mockImplementation(async (cardId) => CARDS[cardId])
  return platform
}

async function mountReview(
  platform: FakePlatform,
  props: { round?: number; newGoal?: number | null } = {},
): Promise<VueWrapper> {
  wrapper = mountWith(
    ReviewSession,
    { platform },
    { props: { round: 0, newGoal: 5, ...props }, attachTo: document.body },
  )
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)
const front = () => find('[data-test="front"]').text()

async function click(selector: string): Promise<void> {
  await find(selector).trigger('click')
  await flushPromises()
}

async function key(name: string, init: KeyboardEventInit = {}): Promise<KeyboardEvent> {
  const event = pressKey(name, init)
  await flushPromises()
  return event
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('a review session', () => {
  it('walks the queue in order, introducing a new word only when it is reached', async () => {
    const platform = reviewPlatform()
    await mountReview(platform)

    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)
    expect(platform.getCard).toHaveBeenCalledWith(1)
    expect(platform.previewRatings).toHaveBeenCalledWith(1)
    expect(platform.startNewItem).not.toHaveBeenCalled()
    expect(front()).toBe('学习')

    await click('[data-test="reveal"]')
    await click('[data-test="rate-3"]')
    expect(platform.submitReviewRating).toHaveBeenLastCalledWith(1, 3)
    expect(front()).toBe('serendipity')
    expect(platform.startNewItem).not.toHaveBeenCalled()

    await click('[data-test="reveal"]')
    await click('[data-test="rate-2"]')
    expect(platform.submitReviewRating).toHaveBeenLastCalledWith(2, 2)

    expect(platform.startNewItem).toHaveBeenCalledTimes(1)
    expect(platform.startNewItem).toHaveBeenCalledWith('学')
    expect(platform.getCard).toHaveBeenLastCalledWith(30)
    const introduced = platform.startNewItem.mock.invocationCallOrder[0]
    expect(introduced).toBeGreaterThan(platform.submitReviewRating.mock.invocationCallOrder[1])
    expect(introduced).toBeLessThan(platform.getCard.mock.invocationCallOrder[2])
    expect(front()).toBe('学')
    expect(find('[data-test="new-badge"]').text()).toBe('New word')

    await click('[data-test="reveal"]')
    await click('[data-test="rate-4"]')
    expect(platform.submitReviewRating).toHaveBeenLastCalledWith(30, 4)
    expect(platform.submitReviewRating).toHaveBeenCalledTimes(3)
    expect(find('[data-test="review-complete"]').exists()).toBe(true)
  })

  it('shows the front first and reveals meaning, pinyin, example and audio', async () => {
    await mountReview(reviewPlatform())
    expect(front()).toBe('学习')
    expect(find('[data-test="front"]').attributes('lang')).toBe('zh-CN')
    expect(find('[data-test="hsk"]').text()).toBe('HSK 1')
    expect(find('[data-test="back"]').exists()).toBe(false)
    expect(find('[data-test="pinyin"]').exists()).toBe(false)
    expect(find('[data-test="example"]').exists()).toBe(false)
    expect(find('[data-test="play-audio"]').exists()).toBe(false)
    expect(find('[data-test="rate-1"]').exists()).toBe(false)

    await click('[data-test="reveal"]')

    expect(find('[data-test="back"]').text()).toBe('to study')
    expect(find('[data-test="answer"]').text()).toContain('Meaning (English)')
    expect(find('[data-test="pinyin"]').text()).toBe('xué xí')
    expect(find('[data-test="example"]').text()).toContain('我在学习中文。')
    expect(find('[data-test="example"]').text()).toContain('I am studying Chinese.')
    expect(find('[data-test="play-audio"]').exists()).toBe(true)
    expect(find('[data-test="reveal"]').exists()).toBe(false)
  })

  it('labels every rating with the interval FSRS would schedule', async () => {
    await mountReview(reviewPlatform())
    await click('[data-test="reveal"]')
    const label = (rating: number) => find(`[data-test="rate-${rating}"]`)
    expect(label(1).text()).toContain('Again')
    expect(label(1).find('[data-test="interval"]').text()).toBe('1 min')
    expect(label(2).find('[data-test="interval"]').text()).toBe('6 min')
    expect(label(3).find('[data-test="interval"]').text()).toBe('10 min')
    expect(label(4).find('[data-test="interval"]').text()).toBe('3 d')
    expect(label(3).attributes('aria-label')).toBe('Good, next review in 10 min')
  })

  it('measures a learning step from the preview, however long the front was shown', async () => {
    const clock = fixedClock()
    wrapper = mountWith(
      ReviewSession,
      { platform: reviewPlatform(), clock },
      { props: { round: 0, newGoal: 5 } },
    )
    await flushPromises()
    clock.set(fx.NOW + 5 * 60_000)
    await click('[data-test="reveal"]')
    expect(find('[data-test="rate-3"] [data-test="interval"]').text()).toBe('10 min')
  })

  it('reveals with Space and rates with 1–4, and only then', async () => {
    const platform = reviewPlatform()
    await mountReview(platform)

    await key('3')
    expect(platform.submitReviewRating).not.toHaveBeenCalled()
    const space = await key(' ')
    expect(space.defaultPrevented).toBe(true)
    expect(find('[data-test="back"]').exists()).toBe(true)

    await key('5')
    await key('3', { metaKey: true })
    await key('3', { ctrlKey: true })
    expect(platform.submitReviewRating).not.toHaveBeenCalled()

    await key('1')
    expect(platform.submitReviewRating).toHaveBeenCalledWith(1, 1)
    expect(front()).toBe('serendipity')

    await key(' ')
    await key('4')
    expect(platform.submitReviewRating).toHaveBeenLastCalledWith(2, 4)
  })

  it('ignores keys typed into a field', async () => {
    const platform = reviewPlatform()
    await mountReview(platform)
    const input = document.createElement('input')
    document.body.appendChild(input)
    input.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true }))
    await flushPromises()
    expect(find('[data-test="back"]').exists()).toBe(false)
  })

  it('rates once, however fast the keys come', async () => {
    const platform = reviewPlatform()
    const pending = deferred<ReturnType<typeof fx.reviewResult>>()
    platform.submitReviewRating.mockReturnValueOnce(pending.promise)
    await mountReview(platform)
    await key(' ')
    await key('3')
    await key('3')
    await click('[data-test="rate-4"]')
    expect(platform.submitReviewRating).toHaveBeenCalledTimes(1)
    pending.resolve(fx.reviewResult())
    await flushPromises()
    expect(front()).toBe('serendipity')
  })

  it('moves the progress bar card by card', async () => {
    await mountReview(reviewPlatform())
    const bar = () => find('[data-test="progress"]')
    expect(bar().attributes('aria-valuemax')).toBe('3')
    expect(bar().attributes('aria-valuenow')).toBe('0')
    expect(bar().attributes('aria-valuetext')).toBe('Card 1 of 3')
    await key(' ')
    await key('3')
    expect(bar().attributes('aria-valuenow')).toBe('1')
    expect(bar().attributes('aria-valuetext')).toBe('Card 2 of 3')
  })

  it('ends with the XP the session earned and the streak', async () => {
    const platform = reviewPlatform()
    platform.submitReviewRating
      .mockResolvedValueOnce(fx.reviewResult({ xp_awarded: 10, streak: fx.streak({ current: 3 }) }))
      .mockResolvedValueOnce(fx.reviewResult({ xp_awarded: 10, streak: fx.streak({ current: 3 }) }))
      .mockResolvedValueOnce(
        fx.reviewResult({
          xp_awarded: 65,
          session_completed: true,
          streak: fx.streak({ current: 4, longest: 9, completed_today: true }),
        }),
      )
    await mountReview(platform)
    for (const rating of ['3', '3', '4']) {
      await key(' ')
      await key(rating)
    }
    expect(find('[data-test="review-complete"]').exists()).toBe(true)
    expect(find('[data-test="summary-xp"]').text()).toBe('85')
    expect(find('[data-test="summary-rated"]').text()).toBe('3')
    expect(find('[data-test="summary-streak"]').text()).toBe('4')
    expect(find('[data-test="summary-longest"]').text()).toBe('9')
    expect(find('[data-test="goal-reached"]').text()).toBe('Today’s goal is reached.')
    expect(platform.getStats).not.toHaveBeenCalled()
  })

  it('does not claim the goal is reached when the backend did not say so', async () => {
    const platform = reviewPlatform()
    platform.getDailyQueue.mockResolvedValue([QUEUE[0]])
    await mountReview(platform)
    await key(' ')
    await key('3')
    expect(find('[data-test="review-complete"]').exists()).toBe(true)
    expect(find('[data-test="goal-reached"]').exists()).toBe(false)
    expect(find('[data-test="summary-xp"]').text()).toBe('10')
  })

  it('starts another pass over a fresh queue on request', async () => {
    const platform = reviewPlatform()
    platform.getDailyQueue.mockResolvedValueOnce([QUEUE[0]]).mockResolvedValueOnce([QUEUE[1]])
    await mountReview(platform)
    await key(' ')
    await key('3')
    await click('[data-test="more"]')
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(2)
    expect(front()).toBe('serendipity')
    expect(find('[data-test="session-xp"]').exists()).toBe(false)
  })

  it('rebuilds the queue when today’s goal changes', async () => {
    const platform = reviewPlatform()
    await mountReview(platform)
    await wrapper!.setProps({ round: 1 })
    await flushPromises()
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(2)
    expect(front()).toBe('学习')
  })

  it('drops answers that belong to a queue it already replaced', async () => {
    const platform = reviewPlatform()
    const stale = deferred<StoredCapture>()
    platform.getCard.mockReturnValueOnce(stale.promise)
    await mountReview(platform)
    platform.getDailyQueue.mockResolvedValueOnce([QUEUE[1]])
    await wrapper!.setProps({ round: 1 })
    await flushPromises()
    stale.resolve(CARDS[1])
    await flushPromises()
    expect(front()).toBe('serendipity')
  })

  it('says when nothing is due, and offers new words on a reviews-only day', async () => {
    const platform = fakePlatform()
    await mountReview(platform, { newGoal: 0 })
    expect(find('[data-test="review-empty"]').text()).toContain('All caught up')
    expect(find('[data-test="goal"]').text()).toBe('Today: reviews only')
    await click('[data-test="change-goal"]')
    expect(wrapper!.emitted('change-goal')).toHaveLength(1)
  })

  it('does not offer a goal change when today has new words', async () => {
    await mountReview(fakePlatform(), { newGoal: 5 })
    expect(find('[data-test="goal"]').text()).toBe('New characters today: 5')
    expect(find('[data-test="change-goal"]').exists()).toBe(false)
  })
})

describe('when something fails', () => {
  it('retries building the queue', async () => {
    const platform = reviewPlatform()
    platform.getDailyQueue.mockRejectedValueOnce('database is locked')
    await mountReview(platform)
    expect(find('[data-test="review-error"]').text()).toContain('database is locked')
    await click('[data-test="retry"]')
    expect(front()).toBe('学习')
  })

  it('retries or skips a card that does not load', async () => {
    const platform = reviewPlatform()
    platform.getCard.mockRejectedValueOnce('card 1 not found').mockRejectedValueOnce('again')
    await mountReview(platform)
    expect(find('[data-test="review-card-error"]').text()).toContain(
      'This card could not be loaded: card 1 not found',
    )
    await click('[data-test="retry"]')
    expect(platform.getCard).toHaveBeenCalledTimes(2)
    expect(find('[data-test="review-card-error"]').text()).toContain('again')

    await click('[data-test="skip"]')
    expect(platform.submitReviewRating).not.toHaveBeenCalled()
    expect(front()).toBe('serendipity')
    expect(find('[data-test="review-card-error"]').exists()).toBe(false)
  })

  it('keeps the card when a rating is not saved, so it can be rated again', async () => {
    const platform = reviewPlatform()
    platform.submitReviewRating.mockRejectedValueOnce('database is locked')
    await mountReview(platform)
    await key(' ')
    await key('3')
    expect(find('[data-test="review-card-error"]').text()).toContain(
      'Your answer could not be saved: database is locked',
    )
    expect(front()).toBe('学习')
    await key('3')
    expect(platform.submitReviewRating).toHaveBeenCalledTimes(2)
    expect(front()).toBe('serendipity')
  })

  it('asks for the streak when no rating reported one', async () => {
    const platform = reviewPlatform()
    platform.getDailyQueue.mockResolvedValue([QUEUE[0]])
    platform.getCard.mockRejectedValue('gone')
    platform.getStats.mockResolvedValue(fx.stats({ streak: fx.streak({ current: 6 }) }))
    await mountReview(platform)
    await click('[data-test="skip"]')
    expect(find('[data-test="review-complete"]').exists()).toBe(true)
    expect(find('[data-test="summary-streak"]').text()).toBe('6')
  })
})

describe('the card', () => {
  it('names the meaning’s language and notes the English fallback of German learners', async () => {
    const platform = reviewPlatform()
    platform.getDailyQueue.mockResolvedValue([QUEUE[2], QUEUE[1]])
    await mountReview(platform)
    await key(' ')
    expect(find('[data-test="answer"]').text()).toContain('Meaning (English)')
    expect(find('[data-test="meaning-fallback"]').text()).toContain('No German meaning yet')

    await key('3')
    await key(' ')
    expect(find('[data-test="answer"]').text()).toContain('Meaning (German)')
    expect(find('[data-test="meaning-fallback"]').exists()).toBe(false)
    expect(find('[data-test="notes"]').text()).toContain('Rare word.')
    expect(find('[data-test="pinyin"]').exists()).toBe(false)
  })

  it('shows no fallback note to an English learner', async () => {
    const platform = reviewPlatform()
    platform.getSettings.mockResolvedValue(fx.settings((value) => (value.meaning_lang = 'en')))
    platform.getDailyQueue.mockResolvedValue([QUEUE[2]])
    await mountReview(platform)
    await key(' ')
    expect(find('[data-test="meaning-fallback"]').exists()).toBe(false)
  })

  it('plays the stored recording, else TTS of the front', async () => {
    const play = HTMLMediaElement.prototype.play as unknown as {
      mock: { contexts: HTMLAudioElement[] }
    }
    const platform = reviewPlatform()
    await mountReview(platform)
    await key(' ')
    await click('[data-test="play-audio"]')
    expect(platform.getCardAudio).toHaveBeenCalledWith(1)
    expect(platform.synthesizeAudio).toHaveBeenCalledWith('学习', 'zh-CN')
    expect(play.mock.contexts[play.mock.contexts.length - 1]?.src).toBe(
      'data:audio/mpeg;base64,U1lOVEg=',
    )

    await click('[data-test="play-audio"]')
    expect(platform.synthesizeAudio).toHaveBeenCalledTimes(1)
    expect(play.mock.contexts).toHaveLength(2)
  })

  it('plays a recording the card carries without asking for it', async () => {
    const play = HTMLMediaElement.prototype.play as unknown as {
      mock: { contexts: HTMLAudioElement[] }
    }
    const platform = reviewPlatform()
    platform.getCard.mockResolvedValue(fx.capture({ id: 1, card: { audio_front: 'SUQz' } }))
    await mountReview(platform)
    await key(' ')
    await click('[data-test="play-audio"]')
    expect(platform.getCardAudio).not.toHaveBeenCalled()
    expect(platform.synthesizeAudio).not.toHaveBeenCalled()
    expect(play.mock.contexts[play.mock.contexts.length - 1]?.src).toBe(
      'data:audio/mpeg;base64,SUQz',
    )
  })

  it('says so when there is nothing to play', async () => {
    const platform = reviewPlatform()
    platform.capabilities.tts = false
    await mountReview(platform)
    await key(' ')
    await click('[data-test="play-audio"]')
    expect(platform.synthesizeAudio).not.toHaveBeenCalled()
    expect(find('[data-test="audio-status"]').text()).toBe('No audio is available for this.')
  })
})
