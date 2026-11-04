import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiStatus, AppConfig, AutoMatch, SetupStatus } from '../api'
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
import { chineseMapping } from '../domain/fieldMapping'
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

async function mountTab(): Promise<VueWrapper> {
  onCommand('get_current_settings', defaultConfig())
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
      model === 'Chinese'
        ? ['Hanzi', 'Pinyin', 'Meaning', 'Sentence', 'Audio']
        : ['Front', 'Back', 'Context Sentence', 'Notes', 'Front Audio'],
    )
  })

  it("lists Anki's note types and the selected one's fields, through the backend only", async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch')
    await mountTab()
    expect(state()).toBe('online')
    const models = wrapper
      ?.findAll('[data-test="model-select"] option')
      .map((o) => o.attributes('value'))
    expect(models).toEqual(ONLINE.models)
    expect(callsTo('anki_model_field_names').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3' },
    ])
    expect(wrapper?.findAll('tr[data-field]').map((tr) => tr.attributes('data-field'))).toEqual([
      'Front',
      'Back',
      'Context Sentence',
      'Notes',
      'Front Audio',
    ])

    await wrapper?.find('[data-test="model-select"]').setValue('Chinese')
    await flushPromises()
    expect(callsTo('anki_model_field_names').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3' },
      { model: 'Chinese' },
    ])
    expect(wrapper?.find('[data-test="missing-fields"]').exists()).toBe(true)
    expect(fetchSpy).not.toHaveBeenCalled()
    fetchSpy.mockRestore()
  })

  it('keeps a configured note type Anki does not have, marked as missing', async () => {
    await mountTab()
    store.config.value.anki.model_name = 'Gone'
    await flushPromises()
    const options = wrapper?.findAll('[data-test="model-select"] option').map((o) => o.text())
    expect(options).toContain('Gone (not in Anki)')
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

  it('edits anki.field_mapping through the mapping editor', async () => {
    await mountTab()
    await wrapper?.find('tr[data-field="Notes"] select').setValue('hsk_level')
    expect(store.config.value.anki.field_mapping.fields).toContainEqual({
      anki_field: 'Notes',
      source: 'hsk_level',
    })
    await wrapper?.find('[data-test="preset-chinese"]').trigger('click')
    expect(store.config.value.anki.field_mapping).toEqual(chineseMapping())
    expect(store.dirty.value).toBe(true)
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

  it('toggles the reverse card', async () => {
    await mountTab()
    expect(store.config.value.anki.create_reverse).toBe(true)
    await wrapper?.find('[data-test="create-reverse"]').setValue(false)
    expect(store.config.value.anki.create_reverse).toBe(false)
  })

  it('J5: asks for a Chinese note type when Chinese is captured without a pinyin field', async () => {
    await mountTab()
    store.config.value.translation.source_lang = 'zh-CN'
    await flushPromises()
    const warning = wrapper?.find('[data-test="chinese-warning"]')
    expect(warning?.text()).toContain('Hanzi, Pinyin, Meaning and Audio')

    await wrapper?.find('[data-test="preset-chinese"]').trigger('click')
    expect(wrapper?.find('[data-test="chinese-warning"]').exists()).toBe(false)
  })

  it('J5: also warns when only a hotkey profile captures Chinese', async () => {
    await mountTab()
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
  const APPLE_V3_FIELDS = ['Front', 'Back', 'English Sentence', 'German Sentence', 'Audio']
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

  it("fixes the default mapping for the user's AppleStyleFinalV3 note type", async () => {
    onCommand('anki_status', ONLINE)
    onCommand('anki_model_field_names', APPLE_V3_FIELDS)
    onCommand('suggest_field_mapping', APPLE_V3_MATCH)
    await mountTab()
    expect(
      wrapper?.findAll('[data-test="missing-fields"] code').map((code) => code.text()),
    ).toEqual(['Context Sentence', 'Notes', 'Front Audio'])
    expect(store.dirty.value).toBe(false)

    await wrapper?.find('[data-test="match-by-name"]').trigger('click')
    await flushPromises()

    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(store.config.value.anki.field_mapping).toEqual(APPLE_V3_MATCH.mapping)
    expect(store.dirty.value).toBe(true)
    expect(wrapper?.find('[data-test="missing-fields"]').exists()).toBe(false)
  })

  it('is disabled while Anki is offline', async () => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    await mountTab()
    const button = wrapper?.find('[data-test="match-by-name"]')
    expect(button?.attributes('disabled')).toBeDefined()
    expect(button?.attributes('title')).toBe('Connect to Anki to match the fields by name.')
  })
})
