import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiStatus, AppConfig, AutoMatch, FieldMapping, SetupStatus } from '../api'
import {
  ANKI_CONNECTION_KEY,
  useAnkiConnection,
  type AnkiConnection,
} from '../composables/useAnkiConnection'
import {
  CONFIG_STORE_KEY,
  defaultConfig,
  useConfig,
  type ConfigStore,
} from '../composables/useConfig'
import { appleStyleMapping, chineseMapping, withBinding } from '../domain/fieldMapping'
import { i18n, setLocale } from '../i18n'
import { callsTo, commandNames, invokeCalls, onCommand, resetTauri } from '../test/fakeTauri'
import AnkiConnectionView from './AnkiConnectionView.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

const ONLINE: AnkiStatus = {
  online: true,
  version: 6,
  decks: ['Default', 'English::AutomatedVocabulary'],
  models: ['Basic', 'AppleStyleFinalV3', 'Chinese'],
  error: null,
}
const OFFLINE: AnkiStatus = {
  online: false,
  version: null,
  decks: [],
  models: [],
  error: 'refused',
}
const NO_CONSENT = { anki_install_granted_at: null, ankiconnect_provision_granted_at: null }
const GRANTED_AT = '2026-10-31T12:00:00.000Z'

function setup(overrides: Partial<SetupStatus> = {}): SetupStatus {
  return {
    anki_installed: true,
    ankiconnect_installed: true,
    platform_supported: true,
    anki_version: '24.06.3',
    consent: NO_CONSENT,
    ...overrides,
  }
}

let wrapper: VueWrapper | null = null
let scope: ReturnType<typeof effectScope> | null = null
let store: ConfigStore
let connection: AnkiConnection

async function mountTab(config: AppConfig = defaultConfig()): Promise<VueWrapper> {
  onCommand('get_current_settings', config)
  scope = effectScope()
  scope.run(() => {
    store = useConfig({ isMac: false })
    connection = useAnkiConnection({ onConsentChanged: (consent) => store.setConsent(consent) })
  })
  await store.load()
  await connection.check()
  invokeCalls.length = 0
  wrapper = mount(AnkiConnectionView, {
    global: {
      plugins: [i18n],
      provide: {
        [CONFIG_STORE_KEY as symbol]: store,
        [ANKI_CONNECTION_KEY as symbol]: connection,
      },
    },
  })
  await flushPromises()
  return wrapper
}

const state = () => wrapper?.find('[data-test="anki-state"]').attributes('data-state')
const dialog = () => wrapper?.find('[role="dialog"]')
const find = (test: string) => wrapper?.find(`[data-test="${test}"]`)
const exists = (test: string) => find(test)?.exists() ?? false
const previewRows = () =>
  wrapper
    ?.findAll('[data-test="mapping-preview"] tbody tr')
    .map((row) => row.findAll('td').map((cell) => cell.text()))

/** The default config with a global mapping the user customized. */
function customized(
  mapping: FieldMapping = withBinding(appleStyleMapping(), 'Notes', 'hsk_level'),
) {
  const config = defaultConfig()
  config.anki.field_mapping = mapping
  return config
}

const APPLE_V3_FIELDS = ['Front', 'Back', 'English Sentence', 'German Sentence', 'Audio']
const CLASSIC_FIELDS = ['Front', 'Back', 'Context Sentence', 'Notes', 'Front Audio']

/** What `suggest_field_mapping` answers for the user's AppleStyleFinalV3, en -> de (ipc_anki.rs). */
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

beforeEach(() => {
  resetTauri()
  setLocale('en')
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  scope?.stop()
  scope = null
})

describe('consent before AnkiConnect provisioning', () => {
  beforeEach(() => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup({ ankiconnect_installed: false }))
  })

  it('Agree records the consent first, then provisions', async () => {
    onCommand('grant_consent', { ...NO_CONSENT, ankiconnect_provision_granted_at: GRANTED_AT })
    onCommand('auto_provision_ankiconnect', () => {
      onCommand('get_setup_status', setup())
      return 'Provisioned AnkiConnect.'
    })
    await mountTab()
    expect(state()).toBe('plugin_missing')

    await wrapper?.find('[data-test="provision"]').trigger('click')
    expect(dialog()?.exists()).toBe(true)
    expect(dialog()?.text()).toContain('4064fa142785975255457abd6a496015f5b71f38')
    expect(commandNames()).toEqual([])

    await wrapper?.find('[data-test="consent-agree"]').trigger('click')
    await flushPromises()
    expect(commandNames().slice(0, 2)).toEqual(['grant_consent', 'auto_provision_ankiconnect'])
    expect(callsTo('grant_consent')[0].args).toEqual({ action: 'ankiconnect_provision' })
    expect(dialog()?.exists()).toBe(false)
    expect(state()).toBe('anki_closed')
    expect(store.config.value.consent.ankiconnect_provision_granted_at).toBe(GRANTED_AT)
    expect(store.dirty.value).toBe(false)
  })

  it('Cancel closes the dialog and runs nothing', async () => {
    await mountTab()
    await wrapper?.find('[data-test="provision"]').trigger('click')
    await wrapper?.find('[data-test="consent-cancel"]').trigger('click')
    await flushPromises()
    expect(dialog()?.exists()).toBe(false)
    expect(commandNames()).toEqual([])
    expect(state()).toBe('plugin_missing')
  })
})

describe('installing Anki', () => {
  beforeEach(() => onCommand('anki_status', OFFLINE))

  it('is offered as "Install Anki 24.06.3" and needs consent first', async () => {
    onCommand('get_setup_status', setup({ anki_installed: false, ankiconnect_installed: false }))
    onCommand('grant_consent', { ...NO_CONSENT, anki_install_granted_at: GRANTED_AT })
    onCommand('download_and_install_anki', 'Installed.')
    await mountTab()
    const install = wrapper?.find('[data-test="install-anki"]')
    expect(install?.text()).toBe('Install Anki 24.06.3')

    await install?.trigger('click')
    expect(dialog()?.text()).toContain('Install Anki 24.06.3?')
    expect(commandNames()).toEqual([])
    await wrapper?.find('[data-test="consent-agree"]').trigger('click')
    await flushPromises()
    expect(commandNames().slice(0, 2)).toEqual(['grant_consent', 'download_and_install_anki'])
    expect(callsTo('grant_consent')[0].args).toEqual({ action: 'anki_install' })
  })

  it('is not offered where the platform has no installer', async () => {
    onCommand('get_setup_status', setup({ anki_installed: false, platform_supported: false }))
    await mountTab()
    expect(state()).toBe('anki_missing')
    expect(wrapper?.find('[data-test="install-anki"]').exists()).toBe(false)
    expect(wrapper?.text()).toContain('Automatic installation is not available')
  })

  it('is not offered once Anki is installed', async () => {
    onCommand('get_setup_status', setup())
    await mountTab()
    expect(state()).toBe('anki_closed')
    expect(wrapper?.find('[data-test="install-anki"]').exists()).toBe(false)
  })
})

describe('Anki closed', () => {
  it('launches Anki on click', async () => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    onCommand('launch_anki_desktop', 'Launching')
    await mountTab()
    await wrapper?.find('[data-test="launch"]').trigger('click')
    expect(callsTo('launch_anki_desktop')).toHaveLength(1)
    expect(wrapper?.find('[data-test="launch"]').text()).toBe('Waiting for Anki to start…')
    connection.dispose()
  })

  it('keeps deck and note type editable as text', async () => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    await mountTab()
    await wrapper?.find('input#anki-model').setValue('Chinese')
    expect(store.config.value.anki.model_name).toBe('Chinese')
    expect(callsTo('anki_model_field_names')).toHaveLength(0)
  })
})

describe('online', () => {
  beforeEach(() => {
    onCommand('anki_status', ONLINE)
    onCommand('anki_model_field_names', ({ model }: { model: string }) =>
      model === 'Chinese' ? ['Hanzi', 'Pinyin', 'Meaning', 'Sentence', 'Audio'] : CLASSIC_FIELDS,
    )
    onCommand('suggest_field_mapping', APPLE_V3_MATCH)
  })

  it("lists Anki's note types and previews the match by field name, through the backend only", async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch')
    await mountTab()
    expect(state()).toBe('online')
    const models = wrapper
      ?.findAll('[data-test="model-select"] option')
      .map((o) => o.attributes('value'))
    expect(models).toEqual(ONLINE.models)
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(callsTo('anki_model_field_names')).toHaveLength(0)
    expect(find('mapping-badge')?.text()).toBe('Matched by field name')

    await wrapper?.find('[data-test="model-select"]').setValue('Chinese')
    await flushPromises()
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
      { model: 'Chinese', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(store.config.value.anki.field_mapping).toBeNull()
    expect(fetchSpy).not.toHaveBeenCalled()
    fetchSpy.mockRestore()
  })

  it("edits a customized mapping against the selected note type's fields", async () => {
    await mountTab(customized())
    expect(callsTo('suggest_field_mapping')).toHaveLength(0)
    expect(callsTo('anki_model_field_names').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3' },
    ])
    expect(exists('mapping-badge')).toBe(false)
    expect(wrapper?.findAll('tr[data-field]').map((tr) => tr.attributes('data-field'))).toEqual(
      CLASSIC_FIELDS,
    )

    await wrapper?.find('[data-test="model-select"]').setValue('Chinese')
    await flushPromises()
    expect(callsTo('anki_model_field_names').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3' },
      { model: 'Chinese' },
    ])
    expect(exists('missing-fields')).toBe(true)
  })

  it('keeps a configured note type Anki does not have, marked as missing', async () => {
    await mountTab()
    store.config.value.anki.model_name = 'Gone'
    await flushPromises()
    const options = wrapper?.findAll('[data-test="model-select"] option').map((o) => o.text())
    expect(options).toContain('Gone (not in Anki)')
  })

  it('J5: warns when the match by name of a Chinese capture fills no Pinyin field', async () => {
    onCommand('suggest_field_mapping', ({ model }: { model: string }) =>
      model === 'Chinese'
        ? { mapping: chineseMapping(), guessed: [], unmatched: [] }
        : APPLE_V3_MATCH,
    )
    await mountTab()
    store.config.value.translation.source_lang = 'zh-CN'
    await flushPromises()
    expect(find('chinese-warning')?.text()).toContain('Hanzi, Pinyin, Meaning and Audio')

    await wrapper?.find('[data-test="model-select"]').setValue('Chinese')
    await flushPromises()
    expect(exists('chinese-warning')).toBe(false)
    expect(store.config.value.anki.field_mapping).toBeNull()
  })
})

describe('the settings this tab owns', () => {
  beforeEach(() => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
  })

  it('flags a missing deck name', async () => {
    await mountTab()
    const edit = (config: AppConfig) => (config.anki.deck_name = ' ')
    edit(store.config.value)
    await flushPromises()
    expect(wrapper?.text()).toContain('Required.')
  })

  it('matches by field name until customized, and saves no mapping for it', async () => {
    await mountTab()
    expect(store.config.value.anki.field_mapping).toBeNull()
    expect(find('mapping-badge')?.text()).toBe('Matched by field name')
    expect(find('mapping-offline')?.text()).toBe(
      'Connect to Anki to see how the fields of “AppleStyleFinalV3” are matched.',
    )
    expect(exists('match-by-name')).toBe(false)

    onCommand('apply_settings', null)
    store.config.value.anki.deck_name = 'english'
    expect(await store.save()).toBe(true)
    expect(callsTo('apply_settings')[0].args?.newCfg).toMatchObject({
      anki: { deck_name: 'english', field_mapping: null },
    })
  })

  it('edits anki.field_mapping once customized, offline from the Apple-style preset', async () => {
    await mountTab()
    await find('customize-mapping')?.trigger('click')
    expect(store.config.value.anki.field_mapping).toEqual(appleStyleMapping())
    expect(exists('mapping-badge')).toBe(false)
    // The preset itself is what "matched by name" stands for: nothing to save yet.
    expect(store.dirty.value).toBe(false)

    await wrapper?.find('tr[data-field="Notes"] select').setValue('hsk_level')
    expect(store.config.value.anki.field_mapping?.fields).toContainEqual({
      anki_field: 'Notes',
      source: 'hsk_level',
    })
    expect(store.dirty.value).toBe(true)
    await find('preset-chinese')?.trigger('click')
    expect(store.config.value.anki.field_mapping).toEqual(chineseMapping())

    await find('rematch-mapping')?.trigger('click')
    expect(store.config.value.anki.field_mapping).toBeNull()
    expect(find('mapping-badge')?.text()).toBe('Matched by field name')
    expect(store.dirty.value).toBe(false)
  })

  it('says it covers only other languages while languages are listed under "My languages"', async () => {
    await mountTab()
    expect(wrapper?.find('[data-test="anki-fallback-hint"]').exists()).toBe(false)

    store.config.value.learning.languages.push({
      lang: 'fr',
      deck_name: 'french',
      model_name: 'AppleStyleFinalV3',
      field_mapping: null,
      create_reverse: true,
    })
    await flushPromises()
    expect(wrapper?.find('[data-test="anki-fallback-hint"]').text()).toBe(
      'Text in a language listed under “My languages” is filed with that language’s deck, note type and field mapping. These settings apply to all other text.',
    )
  })

  it('toggles the reverse card, matched by name or customized', async () => {
    await mountTab()
    expect(store.config.value.anki.create_reverse).toBe(true)
    await find('create-reverse')?.setValue(false)
    expect(store.config.value.anki.create_reverse).toBe(false)
    await find('customize-mapping')?.trigger('click')
    await find('create-reverse')?.setValue(true)
    expect(store.config.value.anki.create_reverse).toBe(true)
  })

  it('J5: asks for a Chinese note type when Chinese is captured without a pinyin field', async () => {
    await mountTab(customized())
    store.config.value.translation.source_lang = 'zh-CN'
    await flushPromises()
    const warning = wrapper?.find('[data-test="chinese-warning"]')
    expect(warning?.text()).toContain('Hanzi, Pinyin, Meaning and Audio')

    await wrapper?.find('[data-test="preset-chinese"]').trigger('click')
    expect(wrapper?.find('[data-test="chinese-warning"]').exists()).toBe(false)
  })

  it('J5: cannot tell offline whether a mapping matched by name reaches a Pinyin field', async () => {
    await mountTab()
    store.config.value.translation.source_lang = 'zh-CN'
    await flushPromises()
    expect(exists('chinese-warning')).toBe(false)
  })

  it('J5: also warns when only a hotkey profile captures Chinese', async () => {
    await mountTab(customized())
    expect(wrapper?.find('[data-test="chinese-warning"]').exists()).toBe(false)
    store.config.value.hotkey_profiles.push({
      id: 'zh-de',
      label: 'Chinese',
      shortcut: 'CommandOrControl+Shift+H',
      source_lang: 'zh-CN',
      target_lang: 'de',
      deck_name: null,
    })
    await flushPromises()
    expect(wrapper?.find('[data-test="chinese-warning"]').exists()).toBe(true)
  })
})

describe('match by field name', () => {
  it("fills the user's AppleStyleFinalV3 note type without a click", async () => {
    onCommand('anki_status', ONLINE)
    onCommand('anki_model_field_names', APPLE_V3_FIELDS)
    onCommand('suggest_field_mapping', APPLE_V3_MATCH)
    await mountTab()
    expect(previewRows()).toEqual([
      ['Front', 'Word (front)'],
      ['Back', 'Translation (back)'],
      ['English Sentence', 'Example in the language learned'],
      ['German Sentence', 'Example in your language'],
    ])
    expect(find('preview-audio')?.text()).toBe('Pronunciation → Audio')
    expect(exists('missing-fields')).toBe(false)
    expect(store.config.value.anki.field_mapping).toBeNull()
    expect(store.dirty.value).toBe(false)

    await find('customize-mapping')?.trigger('click')
    await flushPromises()
    expect(store.config.value.anki.field_mapping).toEqual(APPLE_V3_MATCH.mapping)
    expect(store.config.value.anki.field_mapping).not.toBe(APPLE_V3_MATCH.mapping)
    expect(store.dirty.value).toBe(true)
    expect(wrapper?.findAll('tr[data-field]').map((tr) => tr.attributes('data-field'))).toEqual(
      APPLE_V3_FIELDS,
    )
    expect(exists('missing-fields')).toBe(false)
  })

  it('previews the classic note type as the Apple-style preset, which customizing leaves unsaved', async () => {
    onCommand('anki_status', ONLINE)
    onCommand('anki_model_field_names', CLASSIC_FIELDS)
    onCommand('suggest_field_mapping', { mapping: appleStyleMapping(), guessed: [], unmatched: [] })
    await mountTab()
    expect(previewRows()?.map(([field]) => field)).toEqual([
      'Front',
      'Back',
      'Context Sentence',
      'Notes',
    ])
    expect(find('preview-audio')?.text()).toBe('Pronunciation → Front Audio')
    await find('customize-mapping')?.trigger('click')
    expect(store.config.value.anki.field_mapping).toEqual(appleStyleMapping())
    expect(store.dirty.value).toBe(false)
  })

  it('re-matches a customized mapping that does not fit with one click', async () => {
    onCommand('anki_status', ONLINE)
    onCommand('anki_model_field_names', APPLE_V3_FIELDS)
    onCommand('suggest_field_mapping', APPLE_V3_MATCH)
    await mountTab(customized())
    expect(
      wrapper?.findAll('[data-test="missing-fields"] code').map((code) => code.text()),
    ).toEqual(['Context Sentence', 'Notes', 'Front Audio'])
    expect(store.dirty.value).toBe(false)

    await find('match-by-name')?.trigger('click')
    await flushPromises()

    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(store.config.value.anki.field_mapping).toEqual(APPLE_V3_MATCH.mapping)
    expect(store.dirty.value).toBe(true)
    expect(exists('missing-fields')).toBe(false)
  })

  it('is disabled for a customized mapping while Anki is offline', async () => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    await mountTab(customized())
    const button = find('match-by-name')
    expect(button?.attributes('disabled')).toBeDefined()
    expect(button?.attributes('title')).toBe('Connect to Anki to match the fields by name.')
  })
})
