import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import { setLocale } from '../i18n'
import { fakePlatform, mountWith } from '../test/fakePlatform'
import CreditsDialog from './CreditsDialog.vue'

let wrapper: VueWrapper | null = null
let opener: HTMLButtonElement | null = null

async function mountCredits(): Promise<VueWrapper> {
  opener = document.createElement('button')
  document.body.append(opener)
  opener.focus()
  wrapper = mountWith(CreditsDialog, { platform: fakePlatform() }, { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)
const dialog = () => find('[data-test="credits"]')

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  opener?.remove()
  opener = null
  setLocale('en')
})

describe('the credits dialog', () => {
  it('names each source, what Learn uses it for and its license, with the notice in full', async () => {
    await mountCredits()
    expect(dialog().attributes('role')).toBe('dialog')
    expect(dialog().attributes('aria-modal')).toBe('true')

    const cedict = find('[data-test="credit-NOTICE-CC-CEDICT"]')
    expect(cedict.find('h3').text()).toBe('CC-CEDICT (MDBG)')
    expect(cedict.text()).toContain('Licensed under CC BY-SA 4.0, and so is the adapted data.')
    expect(cedict.find('.license-text').text()).toContain(
      'https://creativecommons.org/licenses/by-sa/4.0/',
    )
    expect(cedict.find('details').attributes('open')).toBeDefined()

    for (const [file, name] of [
      ['LICENSE-complete-hsk-vocabulary', 'drkameleon/complete-hsk-vocabulary'],
      ['LICENSE-hsk30', 'ivankra/hsk30'],
    ]) {
      const credit = find(`[data-test="credit-${file}"]`)
      expect(credit.find('h3').text()).toBe(name)
      expect(credit.text()).toContain('MIT License.')
      const license = credit.findAll('.license-text p').map((paragraph) => paragraph.text())
      expect(license[0]).toBe('MIT License')
      expect(license[1]).toMatch(/^Copyright \(c\) \d{4} \S/)
      expect(license[license.length - 1]).toMatch(/^THE SOFTWARE IS PROVIDED "AS IS".* SOFTWARE\.$/)
      // The long license texts fold away.
      expect(credit.find('details').attributes('open')).toBeUndefined()
    }
  })

  it('speaks German too, while the license texts stay as written', async () => {
    setLocale('de')
    await mountCredits()
    expect(find('#credits-title').text()).toBe('Quellen und Lizenzen')
    const cedict = find('[data-test="credit-NOTICE-CC-CEDICT"]')
    expect(cedict.text()).toContain('Lizenziert unter CC BY-SA 4.0')
    expect(cedict.find('.license-text').text()).toContain(
      'Creative Commons Attribution-ShareAlike 4.0',
    )
  })

  it('takes the focus, keeps it inside and gives it back when it closes', async () => {
    await mountCredits()
    const close = find('[data-test="credits-close"]')
    expect(document.activeElement).toBe(close.element)

    const summaries = wrapper!.findAll('summary')
    ;(summaries[summaries.length - 1].element as HTMLElement).focus()
    await dialog().trigger('keydown', { key: 'Tab' })
    expect(document.activeElement).toBe(close.element)
    await dialog().trigger('keydown', { key: 'Tab', shiftKey: true })
    expect(document.activeElement).toBe(summaries[summaries.length - 1].element)

    wrapper!.unmount()
    wrapper = null
    expect(document.activeElement).toBe(opener)
  })

  it('closes with Esc, the close button or a click beside it', async () => {
    await mountCredits()
    await dialog().trigger('keydown', { key: 'Escape' })
    await find('[data-test="credits-close"]').trigger('click')
    await find('.modal-overlay').trigger('click')
    // A click inside the dialog does not close it.
    await dialog().trigger('click')
    expect(wrapper!.emitted('close')).toHaveLength(3)
  })
})
