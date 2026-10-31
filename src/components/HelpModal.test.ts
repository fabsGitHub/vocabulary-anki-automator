import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import { i18n, setLocale } from '../i18n'
import HelpModal from './HelpModal.vue'

const mountHelp = () => mount(HelpModal, { global: { plugins: [i18n] } })

beforeEach(() => setLocale('en'))

describe('HelpModal', () => {
  it('ends with the credits and licenses of the bundled HSK data', () => {
    const wrapper = mountHelp()
    const credits = wrapper.find('[data-test="credits"]')
    expect(credits.find('h4').text()).toBe('Credits and licenses')

    const cedict = credits.find('[data-test="credit-NOTICE-CC-CEDICT"]')
    expect(cedict.find('strong').text()).toBe('CC-CEDICT (MDBG)')
    expect(cedict.text()).toContain('Licensed under CC BY-SA 4.0, and so is the adapted data.')
    expect(cedict.find('.license-text').text()).toContain(
      'https://creativecommons.org/licenses/by-sa/4.0/',
    )
    expect(cedict.find('details').attributes('open')).toBeDefined()

    for (const [file, name] of [
      ['LICENSE-complete-hsk-vocabulary', 'drkameleon/complete-hsk-vocabulary'],
      ['LICENSE-hsk30', 'ivankra/hsk30'],
    ]) {
      const credit = credits.find(`[data-test="credit-${file}"]`)
      expect(credit.find('strong').text()).toBe(name)
      expect(credit.text()).toContain('MIT License.')
      const license = credit.findAll('.license-text p').map((paragraph) => paragraph.text())
      expect(license[0]).toBe('MIT License')
      expect(license[1]).toMatch(/^Copyright \(c\) \d{4} \S/)
      expect(license[license.length - 1]).toMatch(/^THE SOFTWARE IS PROVIDED "AS IS".* SOFTWARE\.$/)
      expect(credit.find('details').attributes('open')).toBeUndefined()
    }
  })

  it('names the credits in German too, with the license texts as written', () => {
    setLocale('de')
    const credits = mountHelp().find('[data-test="credits"]')
    expect(credits.find('h4').text()).toBe('Quellen und Lizenzen')
    expect(credits.text()).toContain('Lizenziert unter CC BY-SA 4.0')
    expect(credits.find('.license-text').text()).toContain(
      'Creative Commons Attribution-ShareAlike 4.0',
    )
  })

  it('closes with Esc, the close button or a click beside it', async () => {
    const wrapper = mountHelp()
    await wrapper.find('[role="dialog"]').trigger('keydown', { key: 'Escape' })
    await wrapper.find('.close-btn').trigger('click')
    await wrapper.find('.modal-overlay').trigger('click')
    await wrapper.find('[role="dialog"]').trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(3)
  })
})
