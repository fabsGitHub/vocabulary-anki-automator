import { mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import { i18n, setLocale } from '../i18n'
import DailyPromptModal from './DailyPromptModal.vue'

let wrapper: VueWrapper | null = null

function mountPrompt(
  props: Partial<{ initialCount: number; busy: boolean; error: string | null }>,
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
    expect(find('[data-test="prompt-start"]').text()).toBe('Saving…')
    expect(find('[data-test="prompt-start"]').attributes('disabled')).toBeDefined()
    expect(find('[data-test="prompt-dismiss"]').attributes('disabled')).toBeDefined()
    await find('[data-test="prompt-count"]').trigger('keydown', { key: 'Escape' })
    expect(wrapper!.emitted('dismiss')).toBeUndefined()
  })

  it('is fully translated', () => {
    setLocale('de')
    mountPrompt({ error: 'kaputt' })
    expect(wrapper!.text()).toContain('Wie viele neue Schriftzeichen möchtest du heute lernen?')
    expect(find('[data-test="prompt-dismiss"]').text()).toBe('Heute nicht')
    expect(find('[data-test="prompt-error"]').text()).toContain('kaputt')
    expect(find('[data-test="not-today-hint"]').text()).toContain('nur Wiederholungen')
  })
})
