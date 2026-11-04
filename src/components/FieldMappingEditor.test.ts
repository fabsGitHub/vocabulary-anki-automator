import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AutoMatch, FieldMapping } from '../api'
import { appleStyleMapping, chineseMapping, recommendedMapping } from '../domain/fieldMapping'
import { i18n, setLocale } from '../i18n'
import { callsTo, deferred, onCommand, resetTauri } from '../test/fakeTauri'
import FieldMappingEditor from './FieldMappingEditor.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

const APPLE_FIELDS = ['Front', 'Back', 'Context Sentence', 'Notes', 'Front Audio']
const CHINESE_FIELDS = ['Hanzi', 'Pinyin', 'Meaning', 'Sentence', 'Audio']
const APPLE_V3_FIELDS = ['Front', 'Back', 'English Sentence', 'German Sentence', 'Audio']
const RECOMMENDED_FIELDS = [
  'Front',
  'Back',
  'Sentence',
  'Sentence Translation',
  'Audio',
  'Pinyin',
  'HSK Level',
  'Notes',
]

/** What `suggest_field_mapping` answers for AppleStyleFinalV3, en -> de. */
const APPLE_V3_MATCH: AutoMatch = {
  mapping: {
    fields: [
      { anki_field: 'Front', source: 'front' },
      { anki_field: 'Back', source: 'back' },
      { anki_field: 'English Sentence', source: 'example_in_source_lang' },
      { anki_field: 'German Sentence', source: 'example_in_target_lang' },
    ],
    audio_field: 'Audio',
  },
  guessed: [],
  unmatched: [],
}

interface Props {
  mapping?: FieldMapping
  modelName?: string
  modelFields?: string[] | null
  fieldsLoading?: boolean
  fieldsError?: string | null
  sourceLang?: string
  targetLang?: string
  issue?: string | null
}

/**
 * The editor with `v-model:mapping` wired the way a parent wires it: every
 * `update:mapping` becomes the next `mapping` prop.
 */
function mountEditor(props: Props = {}): VueWrapper {
  const wrapper: VueWrapper = mount(FieldMappingEditor, {
    props: {
      mapping: appleStyleMapping(),
      modelName: 'AppleStyleFinalV3',
      modelFields: APPLE_FIELDS,
      sourceLang: 'en',
      targetLang: 'de',
      ...props,
      'onUpdate:mapping': (mapping: FieldMapping) => void wrapper.setProps({ mapping }),
    },
    global: { plugins: [i18n] },
  })
  return wrapper
}

/** The mapping the editor shows now (its last `update:mapping`, or the initial prop). */
const shown = (wrapper: VueWrapper) => (wrapper.props() as { mapping: FieldMapping }).mapping

function row(wrapper: VueWrapper, field: string) {
  const found = wrapper.find(`tr[data-field="${field}"]`)
  if (!found.exists()) throw new Error(`no row for ${field}`)
  return found
}

const selected = (wrapper: VueWrapper, field: string) =>
  (row(wrapper, field).find('select').element as HTMLSelectElement).value

const chips = (wrapper: VueWrapper, selector: string) =>
  wrapper.findAll(`${selector} code.field-chip`).map((code) => code.text())

beforeEach(() => {
  resetTauri()
  setLocale('en')
})

describe('binding fields', () => {
  it('lists every field of the note type with what fills it', () => {
    const wrapper = mountEditor()
    expect(
      wrapper.findAll('tbody tr[data-field]').map((tr) => tr.attributes('data-field')),
    ).toEqual(APPLE_FIELDS)
    expect(selected(wrapper, 'Front')).toBe('front')
    expect(selected(wrapper, 'Context Sentence')).toBe('example')
    expect(selected(wrapper, 'Front Audio')).toBe('')
    const options = row(wrapper, 'Front')
      .findAll('option')
      .map((option) => option.attributes('value'))
    expect(options).toEqual([
      '',
      'front',
      'back',
      'example_source',
      'example_target',
      'example_in_source_lang',
      'example_in_target_lang',
      'example',
      'notes',
      'pinyin',
      'hsk_level',
    ])
  })

  it('emits update:mapping when a field is bound and unbound again', async () => {
    const wrapper = mountEditor()
    await row(wrapper, 'Notes').find('select').setValue('hsk_level')
    const emitted = wrapper.emitted<[FieldMapping]>('update:mapping') ?? []
    expect(emitted).toHaveLength(1)
    expect(emitted[0][0].fields).toContainEqual({ anki_field: 'Notes', source: 'hsk_level' })

    await row(wrapper, 'Notes').find('select').setValue('')
    expect(shown(wrapper).fields.map((b) => b.anki_field)).toEqual([
      'Front',
      'Back',
      'Context Sentence',
    ])
  })

  it('chooses the audio field from the note type, or none', async () => {
    const wrapper = mountEditor()
    const audio = wrapper.find('[data-test="audio-field"]')
    expect(audio.findAll('option').map((o) => o.attributes('value'))).toEqual(['', ...APPLE_FIELDS])
    await audio.setValue('Back')
    expect(shown(wrapper).audio_field).toBe('Back')
    await audio.setValue('')
    expect(shown(wrapper).audio_field).toBeNull()
  })

  it('still edits the saved bindings while the note type is unknown', async () => {
    const wrapper = mountEditor({ modelFields: null })
    expect(wrapper.find('[data-test="mapping-offline"]').exists()).toBe(true)
    expect(
      wrapper.findAll('tbody tr[data-field]').map((tr) => tr.attributes('data-field')),
    ).toEqual(['Front', 'Back', 'Context Sentence', 'Notes'])
    await row(wrapper, 'Back').find('select').setValue('example_target')
    expect(shown(wrapper).fields[1]).toEqual({ anki_field: 'Back', source: 'example_target' })
    await wrapper.find('input[data-test="audio-field"]').setValue('Audio')
    expect(shown(wrapper).audio_field).toBe('Audio')
  })

  it('never edits the mapping it was given in place', async () => {
    const original = appleStyleMapping()
    const wrapper = mountEditor({ mapping: original })
    await row(wrapper, 'Notes').find('select').setValue('pinyin')
    await wrapper.find('[data-test="audio-field"]').setValue('')
    expect(original).toEqual(appleStyleMapping())
  })

  it('prefixes its element ids, so several editors fit on one page', () => {
    const wrapper = mount(FieldMappingEditor, {
      props: {
        mapping: appleStyleMapping(),
        modelName: 'AppleStyleFinalV3',
        modelFields: APPLE_FIELDS,
        sourceLang: 'en',
        targetLang: 'de',
        idPrefix: 'learned-0-mapping',
      },
      global: { plugins: [i18n] },
    })
    expect(wrapper.find('[data-test="audio-field"]').attributes('id')).toBe(
      'learned-0-mapping-audio',
    )
    expect(wrapper.find('label[for="learned-0-mapping-audio"]').exists()).toBe(true)
    expect(mountEditor().find('[data-test="audio-field"]').attributes('id')).toBe('mapping-audio')
  })
})

describe('presets', () => {
  it('apply the exact Apple-style and Chinese mappings', async () => {
    const wrapper = mountEditor({
      mapping: { fields: [], audio_field: null },
      modelFields: CHINESE_FIELDS,
    })
    await wrapper.find('[data-test="preset-chinese"]').trigger('click')
    expect(shown(wrapper)).toEqual(chineseMapping())
    expect(selected(wrapper, 'Hanzi')).toBe('front')
    expect(selected(wrapper, 'Pinyin')).toBe('pinyin')
    expect(selected(wrapper, 'Meaning')).toBe('back')
    expect(selected(wrapper, 'Sentence')).toBe('example')

    await wrapper.find('[data-test="preset-apple"]').trigger('click')
    expect(shown(wrapper)).toEqual(appleStyleMapping())
  })

  it('apply the recommended note type’s mapping, named after it', async () => {
    const wrapper = mountEditor({
      mapping: { fields: [], audio_field: null },
      modelFields: RECOMMENDED_FIELDS,
    })
    const button = wrapper.find('[data-test="preset-recommended"]')
    expect(button.text()).toBe('Vocabulary Automator (recommended)')
    await button.trigger('click')
    expect(shown(wrapper)).toEqual(recommendedMapping())
    expect(selected(wrapper, 'Sentence')).toBe('example_in_source_lang')
    expect(selected(wrapper, 'Sentence Translation')).toBe('example_in_target_lang')
    expect(selected(wrapper, 'HSK Level')).toBe('hsk_level')
    expect(wrapper.find('[data-test="missing-fields"]').exists()).toBe(false)
  })
})

describe('match by field name', () => {
  it('asks the backend for the note type and pair, and applies the result', async () => {
    onCommand('suggest_field_mapping', APPLE_V3_MATCH)
    const wrapper = mountEditor({ modelFields: APPLE_V3_FIELDS })
    expect(wrapper.find('[data-test="missing-fields"]').exists()).toBe(true)

    await wrapper.find('[data-test="match-by-name"]').trigger('click')
    await flushPromises()

    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(wrapper.emitted<[FieldMapping]>('update:mapping')?.[0][0]).toEqual(
      APPLE_V3_MATCH.mapping,
    )
    expect(selected(wrapper, 'English Sentence')).toBe('example_in_source_lang')
    expect(selected(wrapper, 'German Sentence')).toBe('example_in_target_lang')
    expect((wrapper.find('[data-test="audio-field"]').element as HTMLSelectElement).value).toBe(
      'Audio',
    )
    expect(wrapper.find('[data-test="missing-fields"]').exists()).toBe(false)
    // Every field found its value by name: nothing to point out.
    expect(wrapper.find('[data-test="match-result"]').exists()).toBe(false)
  })

  it('lists the fields it filled by position and the ones it left empty', async () => {
    onCommand('suggest_field_mapping', {
      mapping: {
        fields: [
          { anki_field: 'Q', source: 'front' },
          { anki_field: 'A', source: 'back' },
        ],
        audio_field: null,
      },
      guessed: ['Q', 'A'],
      unmatched: ['Add Reverse', 'French Sentence'],
    } satisfies AutoMatch)
    const wrapper = mountEditor({
      modelName: 'Odd',
      modelFields: ['Q', 'A', 'Add Reverse', 'French Sentence'],
      sourceLang: 'zh-CN',
      targetLang: 'en',
    })
    await wrapper.find('[data-test="match-by-name"]').trigger('click')
    await flushPromises()

    expect(callsTo('suggest_field_mapping')[0].args).toEqual({
      model: 'Odd',
      sourceLang: 'zh-CN',
      targetLang: 'en',
    })
    expect(wrapper.find('[data-test="matched-guessed"]').text()).toContain(
      'Filled by position because no name fit:',
    )
    expect(chips(wrapper, '[data-test="matched-guessed"]')).toEqual(['Q', 'A'])
    expect(wrapper.find('[data-test="matched-unmatched"]').text()).toContain('Left empty:')
    expect(chips(wrapper, '[data-test="matched-unmatched"]')).toEqual([
      'Add Reverse',
      'French Sentence',
    ])

    // The list describes the matched mapping, so an edit retires it.
    await row(wrapper, 'French Sentence').find('select').setValue('example_in_source_lang')
    expect(wrapper.find('[data-test="match-result"]').exists()).toBe(false)
  })

  it('is disabled while the note type is unknown (Anki offline)', async () => {
    const wrapper = mountEditor({ modelFields: null })
    const button = wrapper.find('[data-test="match-by-name"]')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.attributes('title')).toBe('Connect to Anki to match the fields by name.')
    await button.trigger('click')
    expect(callsTo('suggest_field_mapping')).toHaveLength(0)
    expect(mountEditor().find('[data-test="match-by-name"]').attributes('title')).toBeUndefined()
  })

  it('blames no offline Anki while the fields are being read', () => {
    const wrapper = mountEditor({ modelFields: null, fieldsLoading: true })
    const button = wrapper.find('[data-test="match-by-name"]')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.attributes('title')).toBeUndefined()
    expect(wrapper.text()).toContain('Reading the fields of “AppleStyleFinalV3”…')
    expect(wrapper.find('[data-test="mapping-offline"]').exists()).toBe(false)
  })

  it('blames no offline Anki when the fields could not be read', () => {
    const wrapper = mountEditor({ modelFields: null, fieldsError: 'model was not found' })
    const button = wrapper.find('[data-test="match-by-name"]')
    expect(button.attributes('disabled')).toBeDefined()
    expect(button.attributes('title')).toBeUndefined()
    expect(wrapper.find('[data-test="fields-error"]').exists()).toBe(true)
    expect(wrapper.find('[data-test="mapping-offline"]').exists()).toBe(false)
  })

  it('shows why the fields could not be matched, and keeps the mapping', async () => {
    onCommand('suggest_field_mapping', () => {
      throw 'AnkiConnect error: model was not found: Gone'
    })
    const wrapper = mountEditor({ modelName: 'Gone' })
    await wrapper.find('[data-test="match-by-name"]').trigger('click')
    await flushPromises()
    expect(wrapper.find('[data-test="match-error"]').text()).toBe(
      'The fields could not be matched: AnkiConnect error: model was not found: Gone',
    )
    expect(wrapper.emitted('update:mapping')).toBeUndefined()
    expect(wrapper.find('[data-test="match-by-name"]').attributes('disabled')).toBeUndefined()
  })

  it('reads "Matching…" while it waits, and drops an answer for a note type no longer shown', async () => {
    const answer = deferred<AutoMatch>()
    onCommand('suggest_field_mapping', () => answer.promise)
    const wrapper = mountEditor({ modelFields: APPLE_V3_FIELDS })
    await wrapper.find('[data-test="match-by-name"]').trigger('click')
    const button = wrapper.find('[data-test="match-by-name"]')
    expect(button.text()).toBe('Matching…')
    expect(button.attributes('disabled')).toBeDefined()

    await wrapper.setProps({ modelName: 'Basic', modelFields: ['Front', 'Back'] })
    answer.resolve(APPLE_V3_MATCH)
    await flushPromises()
    expect(wrapper.emitted('update:mapping')).toBeUndefined()
    expect(wrapper.find('[data-test="match-by-name"]').text()).toBe('Match by field name')
  })
})

describe('warnings', () => {
  it('flags mapped fields the note type does not have', () => {
    const wrapper = mountEditor({ modelFields: CHINESE_FIELDS })
    const warning = wrapper.find('[data-test="missing-fields"]')
    expect(warning.findAll('code').map((code) => code.text())).toEqual([
      'Front',
      'Back',
      'Context Sentence',
      'Notes',
      'Front Audio',
    ])
    expect(row(wrapper, 'Context Sentence').text()).toContain('not in this note type')
  })

  it('has nothing to flag when the mapping fits', () => {
    const wrapper = mountEditor()
    expect(wrapper.find('[data-test="missing-fields"]').exists()).toBe(false)
  })

  it('shows why the fields could not be read', () => {
    const wrapper = mountEditor({ modelFields: null, fieldsError: 'model was not found' })
    expect(wrapper.find('[data-test="fields-error"]').text()).toContain('model was not found')
  })

  it('shows the validation message it is given', () => {
    const wrapper = mountEditor({ issue: 'Fill at least one Anki field.' })
    expect(wrapper.find('.field-error').text()).toBe('Fill at least one Anki field.')
    expect(mountEditor().find('.field-error').exists()).toBe(false)
  })
})
