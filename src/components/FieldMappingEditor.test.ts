import { mount, type VueWrapper } from '@vue/test-utils'
import { beforeEach, describe, expect, it } from 'vitest'

import type { AppConfig } from '../api'
import { CONFIG_STORE_KEY, useConfig, type ConfigStore } from '../composables/useConfig'
import { appleStyleMapping, chineseMapping } from '../domain/fieldMapping'
import { i18n, setLocale } from '../i18n'
import FieldMappingEditor from './FieldMappingEditor.vue'

const APPLE_FIELDS = ['Front', 'Back', 'Context Sentence', 'Notes', 'Front Audio']
const CHINESE_FIELDS = ['Hanzi', 'Pinyin', 'Meaning', 'Sentence', 'Audio']

function makeStore(edit: (config: AppConfig) => void = () => {}): ConfigStore {
  const store = useConfig({ isMac: false })
  edit(store.config.value)
  return store
}

function mountEditor(
  store: ConfigStore,
  props: { modelFields: string[] | null; fieldsError?: string | null } = {
    modelFields: APPLE_FIELDS,
  },
): VueWrapper {
  return mount(FieldMappingEditor, {
    props,
    global: { plugins: [i18n], provide: { [CONFIG_STORE_KEY as symbol]: store } },
  })
}

function row(wrapper: VueWrapper, field: string) {
  const found = wrapper.find(`tr[data-field="${field}"]`)
  if (!found.exists()) throw new Error(`no row for ${field}`)
  return found
}

const selected = (wrapper: VueWrapper, field: string) =>
  (row(wrapper, field).find('select').element as HTMLSelectElement).value

beforeEach(() => setLocale('en'))

describe('binding fields', () => {
  it('lists every field of the note type with what fills it', () => {
    const wrapper = mountEditor(makeStore())
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
      'example',
      'notes',
      'pinyin',
      'hsk_level',
    ])
  })

  it('binds a field to a card value and unbinds it again', async () => {
    const store = makeStore()
    const wrapper = mountEditor(store)
    await row(wrapper, 'Notes').find('select').setValue('hsk_level')
    expect(store.config.value.anki.field_mapping.fields).toContainEqual({
      anki_field: 'Notes',
      source: 'hsk_level',
    })
    await row(wrapper, 'Notes').find('select').setValue('')
    expect(store.config.value.anki.field_mapping.fields.map((b) => b.anki_field)).toEqual([
      'Front',
      'Back',
      'Context Sentence',
    ])
  })

  it('chooses the audio field from the note type, or none', async () => {
    const store = makeStore()
    const wrapper = mountEditor(store)
    const audio = wrapper.find('[data-test="audio-field"]')
    expect(audio.findAll('option').map((o) => o.attributes('value'))).toEqual(['', ...APPLE_FIELDS])
    await audio.setValue('Back')
    expect(store.config.value.anki.field_mapping.audio_field).toBe('Back')
    await audio.setValue('')
    expect(store.config.value.anki.field_mapping.audio_field).toBeNull()
  })

  it('toggles the reverse card', async () => {
    const store = makeStore()
    const wrapper = mountEditor(store)
    expect(store.config.value.anki.create_reverse).toBe(true)
    await wrapper.find('[data-test="create-reverse"]').setValue(false)
    expect(store.config.value.anki.create_reverse).toBe(false)
  })

  it('still edits the saved bindings while the note type is unknown', async () => {
    const store = makeStore()
    const wrapper = mountEditor(store, { modelFields: null })
    expect(wrapper.find('[data-test="mapping-offline"]').exists()).toBe(true)
    expect(
      wrapper.findAll('tbody tr[data-field]').map((tr) => tr.attributes('data-field')),
    ).toEqual(['Front', 'Back', 'Context Sentence', 'Notes'])
    await row(wrapper, 'Back').find('select').setValue('example_target')
    expect(store.config.value.anki.field_mapping.fields[1]).toEqual({
      anki_field: 'Back',
      source: 'example_target',
    })
    await wrapper.find('input[data-test="audio-field"]').setValue('Audio')
    expect(store.config.value.anki.field_mapping.audio_field).toBe('Audio')
  })
})

describe('presets', () => {
  it('apply the exact Apple-style and Chinese mappings', async () => {
    const store = makeStore(
      (config) => (config.anki.field_mapping = { fields: [], audio_field: null }),
    )
    const wrapper = mountEditor(store, { modelFields: CHINESE_FIELDS })
    await wrapper.find('[data-test="preset-chinese"]').trigger('click')
    expect(store.config.value.anki.field_mapping).toEqual(chineseMapping())
    expect(selected(wrapper, 'Hanzi')).toBe('front')
    expect(selected(wrapper, 'Pinyin')).toBe('pinyin')
    expect(selected(wrapper, 'Meaning')).toBe('back')
    expect(selected(wrapper, 'Sentence')).toBe('example')

    await wrapper.find('[data-test="preset-apple"]').trigger('click')
    expect(store.config.value.anki.field_mapping).toEqual(appleStyleMapping())
  })
})

describe('warnings', () => {
  it('flags mapped fields the note type does not have', () => {
    const wrapper = mountEditor(makeStore(), { modelFields: CHINESE_FIELDS })
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
    const wrapper = mountEditor(makeStore())
    expect(wrapper.find('[data-test="missing-fields"]').exists()).toBe(false)
  })

  it('J5: asks for a Chinese note type when Chinese is captured without a pinyin field', async () => {
    const store = makeStore((config) => (config.translation.source_lang = 'zh-CN'))
    const wrapper = mountEditor(store, { modelFields: CHINESE_FIELDS })
    const warning = wrapper.find('[data-test="chinese-warning"]')
    expect(warning.text()).toContain('Hanzi, Pinyin, Meaning and Audio')

    await wrapper.find('[data-test="preset-chinese"]').trigger('click')
    expect(wrapper.find('[data-test="chinese-warning"]').exists()).toBe(false)
  })

  it('J5: also warns when only a hotkey profile captures Chinese', () => {
    const store = makeStore((config) =>
      config.hotkey_profiles.push({
        id: 'zh-de',
        label: 'Chinese',
        shortcut: 'CommandOrControl+Shift+H',
        source_lang: 'zh-CN',
        target_lang: 'de',
        deck_name: null,
      }),
    )
    expect(mountEditor(store).find('[data-test="chinese-warning"]').exists()).toBe(true)
    expect(mountEditor(makeStore()).find('[data-test="chinese-warning"]').exists()).toBe(false)
  })

  it('shows why the fields could not be read', () => {
    const wrapper = mountEditor(makeStore(), {
      modelFields: null,
      fieldsError: 'model was not found',
    })
    expect(wrapper.find('[data-test="fields-error"]').text()).toContain('model was not found')
  })
})
