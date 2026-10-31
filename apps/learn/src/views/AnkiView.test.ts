import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import type { LearnSettings } from '../platform/types'
import { deferred, fakePlatform, mountWith, type FakePlatform } from '../test/fakePlatform'
import * as fx from '../test/fixtures'
import AnkiView from './AnkiView.vue'

let wrapper: VueWrapper | null = null

// Attached, because a detached form never submits (the save and import buttons are submit buttons).
async function mountAnki(platform: FakePlatform): Promise<VueWrapper> {
  wrapper = mountWith(AnkiView, { platform }, { attachTo: document.body })
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)
const input = (selector: string) => find(selector).element as HTMLInputElement

async function click(selector: string): Promise<void> {
  await find(selector).trigger('click')
  await flushPromises()
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('settings', () => {
  it('loads the meaning language, deck, note type and reverse cards', async () => {
    const platform = fakePlatform()
    await mountAnki(platform)
    expect(platform.getSettings).toHaveBeenCalledTimes(1)
    expect((find('[data-test="meaning-lang"]').element as HTMLSelectElement).value).toBe('de')
    expect(input('[data-test="deck"]').value).toBe('Chinese::HSK')
    expect(input('[data-test="model"]').value).toBe('Chinese (Hanzi)')
    expect(input('[data-test="reverse"]').checked).toBe(false)
    expect(find('[data-test="save"]').attributes('disabled')).toBeDefined()
    expect(wrapper!.text()).toContain('German meanings are not bundled yet')
  })

  it('saves the edits and keeps everything it does not edit', async () => {
    const platform = fakePlatform()
    await mountAnki(platform)
    await find('[data-test="meaning-lang"]').setValue('en')
    await find('[data-test="deck"]').setValue('  Mandarin::Daily  ')
    await find('[data-test="model"]').setValue('Basic')
    await find('[data-test="reverse"]').setValue(true)
    expect(wrapper!.text()).toContain('Unsaved changes')

    await click('[data-test="save"]')

    const expected: LearnSettings = fx.settings((value) => {
      value.meaning_lang = 'en'
      value.anki.deck_name = 'Mandarin::Daily'
      value.anki.model_name = 'Basic'
      value.anki.create_reverse = true
    })
    expect(platform.saveSettings).toHaveBeenCalledTimes(1)
    expect(platform.saveSettings).toHaveBeenCalledWith(expected)
    expect(find('[data-test="saved"]').text()).toBe('Settings saved.')
    expect(input('[data-test="deck"]').value).toBe('Mandarin::Daily')
    expect(find('[data-test="save"]').attributes('disabled')).toBeDefined()
  })

  it('requires a deck and a note type', async () => {
    const platform = fakePlatform()
    await mountAnki(platform)
    await find('[data-test="deck"]').setValue('   ')
    expect(wrapper!.text()).toContain('Required.')
    expect(find('[data-test="save"]').attributes('disabled')).toBeDefined()
    await find('form').trigger('submit')
    await flushPromises()
    expect(platform.saveSettings).not.toHaveBeenCalled()
  })

  it('reverts unsaved edits', async () => {
    await mountAnki(fakePlatform())
    await find('[data-test="model"]').setValue('Basic')
    await click('[data-test="revert"]')
    expect(input('[data-test="model"]').value).toBe('Chinese (Hanzi)')
    expect(find('[data-test="revert"]').exists()).toBe(false)
  })

  it('shows why saving failed and keeps the edits', async () => {
    const platform = fakePlatform()
    platform.saveSettings.mockRejectedValueOnce('database is locked')
    await mountAnki(platform)
    await find('[data-test="model"]').setValue('Basic')
    await click('[data-test="save"]')
    expect(find('[data-test="save-error"]').text()).toContain('database is locked')
    expect(input('[data-test="model"]').value).toBe('Basic')
  })

  it('offers a retry when the settings do not load', async () => {
    const platform = fakePlatform()
    platform.getSettings.mockRejectedValueOnce('database is locked')
    await mountAnki(platform)
    expect(find('[data-test="settings-error"]').text()).toContain('database is locked')
    await click('[data-test="settings-retry"]')
    expect(input('[data-test="deck"]').value).toBe('Chinese::HSK')
  })
})

describe('the fields of the note type', () => {
  const rows = () => wrapper!.findAll('[data-test="mapping-row"]')
  const names = () => rows().map((row) => (row.find('input').element as HTMLInputElement).value)
  const sources = () => rows().map((row) => (row.find('select').element as HTMLSelectElement).value)
  const saveDisabled = () => find('[data-test="save"]').attributes('disabled') !== undefined

  it('shows the saved field mapping and AnkiConnect address', async () => {
    await mountAnki(fakePlatform())
    expect(names()).toEqual(['Hanzi', 'Pinyin', 'Meaning'])
    expect(sources()).toEqual(['front', 'pinyin', 'back'])
    expect(input('[data-test="mapping-audio"]').value).toBe('Audio')
    expect(input('[data-test="connect-url"]').value).toBe('http://127.0.0.1:8765')
  })

  // Export writes, and import reads, exactly the fields the mapping names: a
  // deck on any other note type (Anki's Basic one, say) transfers only once
  // the mapping can be set to its fields.
  it('saves a mapping for the learner’s own note type and another address', async () => {
    const platform = fakePlatform()
    await mountAnki(platform)
    await find('[data-test="mapping-name-0"]').setValue(' Front ')
    await find('[data-test="mapping-name-2"]').setValue('Back')
    await click('[data-test="mapping-remove-1"]')
    expect(names()).toEqual([' Front ', 'Back'])
    await click('[data-test="mapping-add"]')
    expect(sources()[2]).toBe('example_source')
    await find('[data-test="mapping-name-2"]').setValue('Notes')
    await find('[data-test="mapping-source-2"]').setValue('notes')
    await find('[data-test="mapping-audio"]').setValue('  ')
    await find('[data-test="connect-url"]').setValue(' http://localhost:8766 ')

    await click('[data-test="save"]')

    const expected: LearnSettings = fx.settings((value) => {
      value.anki.connect_url = 'http://localhost:8766'
      value.anki.field_mapping = {
        fields: [
          { anki_field: 'Front', source: 'front' },
          { anki_field: 'Back', source: 'back' },
          { anki_field: 'Notes', source: 'notes' },
        ],
        audio_field: null,
      }
    })
    expect(platform.saveSettings).toHaveBeenCalledWith(expected)
    expect(find('[data-test="saved"]').exists()).toBe(true)
    expect(saveDisabled()).toBe(true)
  })

  it('says what a save would refuse, and does not save it', async () => {
    const platform = fakePlatform()
    await mountAnki(platform)
    await find('[data-test="mapping-name-1"]').setValue('  ')
    expect(find('[data-test="mapping-problems"]').text()).toBe('Every Anki field needs a name.')
    expect(find('[data-test="mapping-name-1"]').attributes('aria-invalid')).toBe('true')
    expect(saveDisabled()).toBe(true)

    await find('[data-test="mapping-name-1"]').setValue('Pinyin')
    await find('[data-test="mapping-source-0"]').setValue('notes')
    await find('[data-test="mapping-source-2"]').setValue('example')
    expect(find('[data-test="mapping-problems"]').text()).toContain(
      'One Anki field has to hold the word.',
    )
    expect(find('[data-test="mapping-problems"]').text()).toContain(
      'One Anki field has to hold the meaning.',
    )
    expect(saveDisabled()).toBe(true)
    await find('form').trigger('submit')
    await flushPromises()
    expect(platform.saveSettings).not.toHaveBeenCalled()
  })

  it('refuses an AnkiConnect address that is not http(s)', async () => {
    const platform = fakePlatform()
    await mountAnki(platform)
    await find('[data-test="connect-url"]').setValue('ftp://127.0.0.1:8765')
    expect(wrapper!.text()).toContain('Enter an http:// or https:// address')
    expect(find('[data-test="connect-url"]').attributes('aria-invalid')).toBe('true')
    expect(saveDisabled()).toBe(true)
    await find('form').trigger('submit')
    await flushPromises()
    expect(platform.saveSettings).not.toHaveBeenCalled()
  })

  it('goes back to the default fields', async () => {
    await mountAnki(fakePlatform())
    await click('[data-test="mapping-default"]')
    expect(names()).toEqual(['Hanzi', 'Pinyin', 'Meaning', 'Sentence'])
    expect(sources()).toEqual(['front', 'pinyin', 'back', 'example'])
    expect(input('[data-test="mapping-audio"]').value).toBe('Audio')
  })

  it('keeps the keyboard focus while fields come and go', async () => {
    await mountAnki(fakePlatform())
    await click('[data-test="mapping-add"]')
    expect(document.activeElement).toBe(find('[data-test="mapping-name-3"]').element)

    ;(find('[data-test="mapping-remove-1"]').element as HTMLElement).focus()
    await click('[data-test="mapping-remove-1"]')
    expect(document.activeElement).toBe(find('[data-test="mapping-remove-1"]').element)

    await click('[data-test="mapping-remove-0"]')
    await click('[data-test="mapping-remove-0"]')
    await click('[data-test="mapping-remove-0"]')
    expect(find('[data-test="mapping-empty"]').text()).toBe('No Anki field is filled yet.')
    expect(document.activeElement).toBe(find('[data-test="mapping-add"]').element)
  })
})

describe('Anki export', () => {
  it('exports with the saved settings and summarizes the report', async () => {
    const platform = fakePlatform()
    platform.ankiExport.mockResolvedValue(
      fx.report({
        attempted: 5,
        succeeded: 2,
        skipped: 2,
        failed: [{ card_id: 4, front: '老师', error: 'model was not found: Chinese (Hanzi)' }],
      }),
    )
    await mountAnki(platform)
    await click('[data-test="export"]')
    expect(platform.ankiExport).toHaveBeenCalledTimes(1)
    const report = find('[data-test="export-report"]')
    expect(report.find('[data-test="report-summary"]').text()).toBe('Exported: 2 of 5.')
    expect(report.find('[data-test="report-skipped"]').text()).toBe('Already in Anki: 2.')
    expect(report.find('[data-test="report-failures"]').text()).toBe('Not transferred: 1')
    expect(report.find('[data-test="report-failure"]').text()).toBe(
      '老师: model was not found: Chinese (Hanzi)',
    )
    expect(report.classes()).toContain('warning')
  })

  it('waits for unsaved changes, since it exports to the saved deck', async () => {
    const platform = fakePlatform()
    await mountAnki(platform)
    await find('[data-test="deck"]').setValue('Other')
    expect(find('[data-test="export"]').attributes('disabled')).toBeDefined()
    expect(find('[data-test="save-first"]').text()).toContain('Save your changes first')
    await click('[data-test="save"]')
    expect(find('[data-test="export"]').attributes('disabled')).toBeUndefined()
  })

  it('says there was nothing to export', async () => {
    const platform = fakePlatform()
    platform.ankiExport.mockResolvedValue(fx.report({ attempted: 0, succeeded: 0 }))
    await mountAnki(platform)
    await click('[data-test="export"]')
    expect(find('[data-test="report-summary"]').text()).toBe('There was nothing to export.')
    expect(find('[data-test="export-report"]').classes()).toContain('success')
  })

  it('explains a failed export', async () => {
    const platform = fakePlatform()
    platform.ankiExport.mockRejectedValueOnce('Anki is not reachable at http://127.0.0.1:8765')
    await mountAnki(platform)
    await click('[data-test="export"]')
    const error = find('[data-test="export-error"]')
    expect(error.attributes('role')).toBe('alert')
    expect(error.text()).toContain('The export failed: Anki is not reachable')
    expect(error.text()).toContain('AnkiConnect')
  })

  it('runs one transfer at a time', async () => {
    const platform = fakePlatform()
    const pending = deferred<ReturnType<typeof fx.report>>()
    platform.ankiExport.mockReturnValueOnce(pending.promise)
    await mountAnki(platform)
    const exportButton = find('[data-test="export"]')
    ;(exportButton.element as HTMLElement).focus()
    await click('[data-test="export"]')
    expect(exportButton.text()).toBe('Exporting…')
    // Unavailable while it runs, but still focusable: a keyboard user keeps their place.
    expect(exportButton.attributes('aria-disabled')).toBe('true')
    expect(exportButton.attributes('disabled')).toBeUndefined()
    expect(document.activeElement).toBe(exportButton.element)
    expect(find('[data-test="import"]').attributes('aria-disabled')).toBe('true')
    await click('[data-test="import"]')
    await click('[data-test="export"]')
    expect(platform.ankiImport).not.toHaveBeenCalled()
    expect(platform.ankiExport).toHaveBeenCalledTimes(1)

    pending.resolve(fx.report())
    await flushPromises()
    expect(find('[data-test="import"]').attributes('aria-disabled')).toBeUndefined()
    expect(exportButton.attributes('aria-disabled')).toBeUndefined()
  })
})

describe('Anki import', () => {
  it('imports the typed deck, starting from the saved one', async () => {
    const platform = fakePlatform()
    platform.ankiImport.mockResolvedValue(fx.report({ attempted: 3, succeeded: 2, skipped: 1 }))
    await mountAnki(platform)
    expect(input('[data-test="import-deck"]').value).toBe('Chinese::HSK')
    await find('[data-test="import-deck"]').setValue(' Mandarin::Old deck ')
    await click('[data-test="import"]')
    expect(platform.ankiImport).toHaveBeenCalledWith('Mandarin::Old deck')
    expect(find('[data-test="import-report"]').text()).toContain('Imported: 2 of 3.')
    expect(find('[data-test="import-report"]').text()).toContain('Already captured: 1.')
  })

  it('names cards without a front by their id', async () => {
    const platform = fakePlatform()
    platform.ankiImport.mockResolvedValue(
      fx.report({ attempted: 1, succeeded: 0, failed: [{ card_id: null, front: '', error: 'x' }] }),
    )
    await mountAnki(platform)
    await click('[data-test="import"]')
    expect(find('[data-test="report-failure"]').text()).toBe('—: x')
  })

  it('needs a deck name and explains a failure', async () => {
    const platform = fakePlatform()
    platform.ankiImport.mockRejectedValueOnce('deck not found: Nope')
    await mountAnki(platform)
    await find('[data-test="import-deck"]').setValue('')
    expect(find('[data-test="import"]').attributes('disabled')).toBeDefined()
    await find('[data-test="import-deck"]').setValue('Nope')
    await click('[data-test="import"]')
    expect(find('[data-test="import-error"]').text()).toContain('The import failed: deck not found')
  })
})

describe('without AnkiConnect', () => {
  it('keeps the meaning language but hides the Anki settings, export and import', async () => {
    const platform = fakePlatform({ kind: 'web', tts: false, anki: false })
    await mountAnki(platform)
    expect(find('[data-test="anki-unavailable"]').text()).toBe(
      'Anki export and import need the desktop app.',
    )
    expect(find('[data-test="export"]').exists()).toBe(false)
    expect(find('[data-test="import"]').exists()).toBe(false)
    for (const field of ['deck', 'model', 'mapping', 'reverse', 'connect-url']) {
      expect(find(`[data-test="${field}"]`).exists(), field).toBe(false)
    }

    await find('[data-test="meaning-lang"]').setValue('en')
    await click('[data-test="save"]')
    expect(platform.saveSettings).toHaveBeenCalledWith(
      fx.settings((value) => {
        value.meaning_lang = 'en'
      }),
    )
  })
})
