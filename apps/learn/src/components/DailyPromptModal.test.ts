import { mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import { i18n, setLocale } from '../i18n'
import DailyPromptModal from './DailyPromptModal.vue'

let wrapper: VueWrapper | null = null

function mountPrompt(
  props: Partial<{ initialCount: number; busy: boolean; error: string | null; web: boolean }>,
) {
  wrapper = mount(DailyPromptModal, {
    props: { initialCount: 5, busy: false, error: null, ...props },
    global: { plugins: [i18n] },
    attachTo: document.body,
  })
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('the daily prompt dialog', () => {
  it('suggests the count it is given', () => {
    mountPrompt({ initialCount: 12 })
    expect((find('[data-test="prompt-count"]').element as HTMLInputElement).value).toBe('12')
  })

  it('keeps Tab inside and gives focus back when it closes', async () => {
    const opener = document.createElement('button')
    document.body.appendChild(opener)
    opener.focus()
    mountPrompt({})
    const start = find('[data-test="prompt-start"]')
    const input = find('[data-test="prompt-count"]')

    ;(start.element as HTMLElement).focus()
    await start.trigger('keydown', { key: 'Tab' })
    expect(document.activeElement).toBe(input.element)
    await input.trigger('keydown', { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(start.element)

    wrapper!.unmount()
    wrapper = null
    expect(document.activeElement).toBe(opener)
  })

  it('locks both answers while one is being saved', async () => {
    mountPrompt({ busy: true })
    const start = find('[data-test="prompt-start"]')
    const dismiss = find('[data-test="prompt-dismiss"]')
    expect(start.text()).toBe('Saving…')
    expect(start.attributes('aria-disabled')).toBe('true')
    expect(dismiss.attributes('aria-disabled')).toBe('true')
    await start.trigger('click')
    await find('form').trigger('submit')
    await dismiss.trigger('click')
    await find('[data-test="prompt-count"]').trigger('keydown', { key: 'Escape' })
    expect(wrapper!.emitted('start')).toBeUndefined()
    expect(wrapper!.emitted('dismiss')).toBeUndefined()
  })

  it('keeps focus, and Esc, in the dialog through a save that fails', async () => {
    mountPrompt({})
    const start = find('[data-test="prompt-start"]')
    ;(start.element as HTMLElement).focus()
    await start.trigger('click')
    expect(wrapper!.emitted('start')).toEqual([[5]])

    await wrapper!.setProps({ busy: true })
    // Not disabled, so the browser does not move focus out to the inert page.
    expect(start.attributes('disabled')).toBeUndefined()
    expect(document.activeElement).toBe(start.element)
    await wrapper!.setProps({ busy: false, error: 'database is locked' })
    expect(document.activeElement).toBe(start.element)
    await start.trigger('keydown', { key: 'Escape' })
    expect(wrapper!.emitted('dismiss')).toHaveLength(1)
  })

  it('says where new words come from: captures and imports first, then the HSK list', () => {
    mountPrompt({})
    const intro = find('#daily-prompt-intro').text()
    expect(intro).toContain('captured in the Automator or imported from Anki, oldest first')
    expect(intro).toContain('then the HSK word list in order')
    wrapper!.unmount()

    // The web build has neither, so it names the word list alone.
    mountPrompt({ web: true })
    expect(find('#daily-prompt-intro').text()).toBe(
      'New words come from the HSK word list, in order. Cards that are due for review always come first.',
    )
  })

  it('is fully translated', () => {
    setLocale('de')
    mountPrompt({ error: 'kaputt' })
    expect(wrapper!.text()).toContain('Wie viele neue Wörter möchtest du heute lernen?')
    expect(find('[data-test="prompt-dismiss"]').text()).toBe('Heute nicht')
    expect(find('[data-test="prompt-error"]').text()).toContain('kaputt')
    expect(find('[data-test="not-today-hint"]').text()).toContain('nur Wiederholungen')
  })
})
