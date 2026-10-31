import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import App from './App.vue'
import { UI_LANG_STORAGE_KEY } from './i18n'
import type { DailyPromptState, DailySession } from './platform/types'
import {
  deferred,
  fakePlatform,
  fixedClock,
  mountWith,
  pressKey,
  type FakePlatform,
} from './test/fakePlatform'
import * as fx from './test/fixtures'

let wrapper: VueWrapper | null = null

async function mountApp(platform: FakePlatform, clock = fixedClock()): Promise<VueWrapper> {
  wrapper = mountWith(App, { platform, clock }, { attachTo: document.body })
  await flushPromises()
  return wrapper
}

function needsPrompt(platform: FakePlatform, session: DailySession | null = null): FakePlatform {
  platform.getDailyPromptState.mockResolvedValue(fx.promptState({ needs_prompt: true, session }))
  return platform
}

const find = (selector: string) => wrapper!.find(selector)
const prompt = () => find('[data-test="daily-prompt"]')
const countInput = () => find('[data-test="prompt-count"]')

async function click(selector: string): Promise<void> {
  await find(selector).trigger('click')
  await flushPromises()
}

async function openMenuAction(): Promise<void> {
  await click('[data-test="menu"]')
  await click('[data-test="menu-change-goal"]')
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  window.localStorage.clear()
})

describe('the daily prompt', () => {
  it('asks before anything renders when today is unanswered, then never again that day', async () => {
    const platform = needsPrompt(fakePlatform())
    await mountApp(platform)

    expect(prompt().exists()).toBe(true)
    expect(prompt().attributes('role')).toBe('dialog')
    expect(prompt().attributes('aria-modal')).toBe('true')
    expect(prompt().text()).toContain('How many new words do you want to learn today?')
    expect((countInput().element as HTMLInputElement).value).toBe('5')
    expect(document.activeElement).toBe(countInput().element)
    // Today's queue depends on the answer, so nothing behind the prompt has started.
    expect(find('[data-test="review"]').exists()).toBe(false)
    expect(find('[role="tablist"]').exists()).toBe(false)
    expect(platform.getDailyQueue).not.toHaveBeenCalled()

    await click('[data-test="prompt-start"]')

    expect(platform.submitDailyQuota).toHaveBeenCalledTimes(1)
    expect(platform.submitDailyQuota).toHaveBeenCalledWith(5)
    expect(platform.dismissDailyPrompt).not.toHaveBeenCalled()
    expect(prompt().exists()).toBe(false)
    expect(find('[data-test="review"]').exists()).toBe(true)
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)
    expect(find('header').text()).toContain('New words today: 5')

    for (const tab of ['stats', 'practice', 'review']) await click(`[data-test="tab-${tab}"]`)
    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(platform.getDailyPromptState).toHaveBeenCalledTimes(1)
    expect(prompt().exists()).toBe(false)
  })

  it('is not shown when today is answered already', async () => {
    const platform = fakePlatform()
    await mountApp(platform)
    expect(platform.getDailyPromptState).toHaveBeenCalledTimes(1)
    expect(prompt().exists()).toBe(false)
    expect(find('[data-test="review"]').exists()).toBe(true)
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)
    expect(platform.submitDailyQuota).not.toHaveBeenCalled()
    expect(platform.dismissDailyPrompt).not.toHaveBeenCalled()
  })

  it('takes a quick pick or a typed count from 0 to 50', async () => {
    const platform = needsPrompt(fakePlatform())
    await mountApp(platform)

    await click('[data-test="quick-pick-10"]')
    expect((countInput().element as HTMLInputElement).value).toBe('10')
    expect(find('[data-test="quick-pick-10"]').attributes('aria-pressed')).toBe('true')

    for (const invalid of ['51', '-1', '2.5', '']) {
      await countInput().setValue(invalid)
      expect(find('[data-test="prompt-invalid"]').exists(), invalid).toBe(true)
      expect(find('[data-test="prompt-start"]').attributes('disabled'), invalid).toBeDefined()
    }
    await find('form').trigger('submit')
    expect(platform.submitDailyQuota).not.toHaveBeenCalled()

    await countInput().setValue('0')
    expect(find('[data-test="prompt-invalid"]').exists()).toBe(false)
    await find('form').trigger('submit')
    await flushPromises()
    expect(platform.submitDailyQuota).toHaveBeenCalledWith(0)
  })

  it('explains "Not today", which asks for reviews only', async () => {
    const platform = needsPrompt(fakePlatform())
    await mountApp(platform)
    expect(find('[data-test="not-today-hint"]').text()).toContain('reviews only')
    expect(find('[data-test="not-today-hint"]').text()).toContain('Change today’s new-word goal')

    await click('[data-test="prompt-dismiss"]')

    expect(platform.dismissDailyPrompt).toHaveBeenCalledTimes(1)
    expect(platform.submitDailyQuota).not.toHaveBeenCalled()
    expect(prompt().exists()).toBe(false)
    expect(find('header').text()).toContain('Today: reviews only')
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)
  })

  it('treats Esc as "Not today"', async () => {
    const platform = needsPrompt(fakePlatform())
    await mountApp(platform)
    await countInput().trigger('keydown', { key: 'Escape' })
    await flushPromises()
    expect(platform.dismissDailyPrompt).toHaveBeenCalledTimes(1)
    expect(prompt().exists()).toBe(false)
  })

  it('stays open and says why when the answer cannot be saved', async () => {
    const platform = needsPrompt(fakePlatform())
    platform.submitDailyQuota.mockRejectedValueOnce('database is locked')
    await mountApp(platform)
    await click('[data-test="prompt-start"]')
    expect(prompt().exists()).toBe(true)
    expect(find('[data-test="prompt-error"]').text()).toContain('database is locked')
    expect(find('[data-test="review"]').exists()).toBe(false)

    await click('[data-test="prompt-start"]')
    expect(prompt().exists()).toBe(false)
    expect(platform.submitDailyQuota).toHaveBeenCalledTimes(2)
  })

  it('can be reopened from the menu to change today’s goal, which rebuilds the queue', async () => {
    const platform = fakePlatform()
    platform.getDailyPromptState.mockResolvedValue(
      fx.promptState({ session: fx.session({ requested_new_count: 8 }) }),
    )
    await mountApp(platform)
    expect(find('header').text()).toContain('New words today: 8')
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)

    await openMenuAction()

    expect(platform.reopenDailyPrompt).toHaveBeenCalledTimes(1)
    expect(prompt().exists()).toBe(true)
    expect((countInput().element as HTMLInputElement).value).toBe('8')

    await countInput().setValue('12')
    await click('[data-test="prompt-start"]')

    expect(platform.submitDailyQuota).toHaveBeenCalledWith(12)
    expect(prompt().exists()).toBe(false)
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(2)
    expect(find('header').text()).toContain('New words today: 12')
  })

  it('suggests the default again when today was reviews only, and a dismissal keeps the queue', async () => {
    const platform = fakePlatform()
    platform.getDailyPromptState.mockResolvedValue(
      fx.promptState({ session: fx.session({ requested_new_count: 0 }) }),
    )
    await mountApp(platform)
    await openMenuAction()
    expect((countInput().element as HTMLInputElement).value).toBe('5')

    await click('[data-test="prompt-dismiss"]')
    expect(platform.dismissDailyPrompt).toHaveBeenCalledTimes(1)
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)
  })

  it('reports a failed reopen without showing the prompt', async () => {
    const platform = fakePlatform()
    platform.reopenDailyPrompt.mockRejectedValueOnce('database is locked')
    await mountApp(platform)
    await openMenuAction()
    expect(prompt().exists()).toBe(false)
    expect(find('[data-test="reopen-error"]').text()).toContain('database is locked')
  })

  it('asks again when a window left open regains focus on a new day', async () => {
    const platform = fakePlatform()
    const clock = fixedClock(new Date(2026, 9, 31, 23, 30).getTime())
    await mountApp(platform, clock)
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)

    window.dispatchEvent(new Event('focus'))
    await flushPromises()
    expect(platform.getDailyPromptState).toHaveBeenCalledTimes(1)

    clock.set(new Date(2026, 10, 1, 0, 15).getTime())
    platform.getDailyPromptState.mockResolvedValue(
      fx.promptState({ needs_prompt: true, today: '2026-11-01', session: null }),
    )
    window.dispatchEvent(new Event('focus'))
    await flushPromises()

    expect(platform.getDailyPromptState).toHaveBeenCalledTimes(2)
    expect(prompt().exists()).toBe(true)
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(2)

    await click('[data-test="prompt-start"]')
    expect(platform.submitDailyQuota).toHaveBeenCalledWith(5)
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(3)
  })

  it('makes everything behind it inert while it is open', async () => {
    const platform = needsPrompt(fakePlatform())
    await mountApp(platform)
    const body = () => find('[data-test="app-body"]')
    expect(body().attributes('inert')).toBeDefined()
    expect(body().find('[data-test="daily-prompt"]').exists()).toBe(false)

    await click('[data-test="prompt-start"]')
    expect(body().attributes('inert')).toBeUndefined()

    await openMenuAction()
    expect(body().attributes('inert')).toBeDefined()
    await click('[data-test="prompt-dismiss"]')
    expect(body().attributes('inert')).toBeUndefined()
  })

  it('keeps the review’s shortcuts away while it is open', async () => {
    const platform = fakePlatform()
    platform.getDailyQueue.mockResolvedValue([
      { kind: 'review', card_id: 1, due_date: '2026-10-31T08:00:00.000Z' },
    ])
    await mountApp(platform)
    pressKey(' ')
    await flushPromises()
    expect(find('[data-test="back"]').exists()).toBe(true)

    await openMenuAction()
    pressKey('3')
    await flushPromises()
    expect(platform.submitReviewRating).not.toHaveBeenCalled()

    await click('[data-test="prompt-dismiss"]')
    pressKey('3')
    await flushPromises()
    expect(platform.submitReviewRating).toHaveBeenCalledWith(1, 3)
  })

  it('shows why today could not be loaded, with a retry', async () => {
    const platform = fakePlatform()
    platform.getDailyPromptState.mockRejectedValueOnce('database is locked')
    await mountApp(platform)
    expect(find('[data-test="app-error"]').text()).toContain('database is locked')
    expect(find('[data-test="review"]').exists()).toBe(false)

    await click('[data-test="app-retry"]')
    expect(find('[data-test="app-error"]').exists()).toBe(false)
    expect(find('[data-test="review"]').exists()).toBe(true)
  })

  it('holds the prompt while the state is loading', async () => {
    const platform = fakePlatform()
    const pending = deferred<DailyPromptState>()
    platform.getDailyPromptState.mockReturnValueOnce(pending.promise)
    await mountApp(platform)
    expect(find('[data-test="app-loading"]').exists()).toBe(true)
    pending.resolve(fx.promptState({ needs_prompt: true }))
    await flushPromises()
    expect(prompt().exists()).toBe(true)
  })
})

describe('the main window', () => {
  it('mounts a tab on its first visit and keeps it, refreshing the stats on every visit', async () => {
    const platform = fakePlatform()
    await mountApp(platform)
    expect(platform.getStats).not.toHaveBeenCalled()

    await click('[data-test="tab-stats"]')
    expect(find('[data-test="tab-stats"]').attributes('aria-selected')).toBe('true')
    expect(find('#panel-stats').isVisible()).toBe(true)
    expect(find('#panel-review').isVisible()).toBe(false)
    expect(platform.getStats).toHaveBeenCalledTimes(1)

    await click('[data-test="tab-review"]')
    await click('[data-test="tab-stats"]')
    expect(platform.getStats).toHaveBeenCalledTimes(2)
    // The review was kept, not rebuilt.
    expect(platform.getDailyQueue).toHaveBeenCalledTimes(1)
  })

  it('moves between tabs with the arrow keys', async () => {
    await mountApp(fakePlatform())
    await find('[data-test="tab-review"]').trigger('keydown', { key: 'ArrowRight' })
    expect(find('[data-test="tab-practice"]').attributes('aria-selected')).toBe('true')
    expect(document.activeElement).toBe(find('[data-test="tab-practice"]').element)
    await find('[data-test="tab-practice"]').trigger('keydown', { key: 'End' })
    expect(find('[data-test="tab-anki"]').attributes('aria-selected')).toBe('true')
    await find('[data-test="tab-anki"]').trigger('keydown', { key: 'ArrowRight' })
    expect(find('[data-test="tab-review"]').attributes('aria-selected')).toBe('true')
  })

  it('opens the menu from the keyboard and closes it with Esc', async () => {
    await mountApp(fakePlatform())
    const button = find('[data-test="menu"]')
    await button.trigger('keydown', { key: 'ArrowDown' })
    await flushPromises()
    expect(button.attributes('aria-expanded')).toBe('true')
    expect(document.activeElement).toBe(find('[data-test="menu-change-goal"]').element)
    await find('[data-test="menu-change-goal"]').trigger('keydown', { key: 'Escape' })
    expect(button.attributes('aria-expanded')).toBe('false')
    expect(document.activeElement).toBe(button.element)
  })

  it('reads the meaning language again when the review comes back', async () => {
    const platform = fakePlatform()
    platform.getDailyQueue.mockResolvedValue([{ kind: 'new', hanzi: '学', curriculum_order: 7 }])
    platform.startNewItem.mockResolvedValue(30)
    platform.getCard.mockResolvedValue(
      fx.capture({ id: 30, origin: 'curriculum', card: { front: '学', back: 'to learn' } }),
    )
    await mountApp(platform)
    pressKey(' ')
    await flushPromises()
    // A German learner's curriculum word with its English meaning.
    expect(find('[data-test="meaning-fallback"]').exists()).toBe(true)

    platform.getSettings.mockResolvedValue(fx.settings((value) => (value.meaning_lang = 'en')))
    await click('[data-test="tab-anki"]')
    await click('[data-test="tab-review"]')
    expect(find('[data-test="back"]').text()).toBe('to learn')
    expect(find('[data-test="meaning-fallback"]').exists()).toBe(false)
  })

  it('switches the UI language and remembers it', async () => {
    await mountApp(fakePlatform())
    await find('[data-test="ui-lang"]').setValue('de')
    expect(find('[data-test="tab-review"]').text()).toBe('Wiederholen')
    expect(document.documentElement.lang).toBe('de')
    expect(window.localStorage.getItem(UI_LANG_STORAGE_KEY)).toBe('de')
  })
})
