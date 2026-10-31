import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiStatus, AppConfig, SetupStatus } from '../api'
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
  it('flags a missing deck name', async () => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', setup())
    await mountTab()
    const edit = (config: AppConfig) => (config.anki.deck_name = ' ')
    edit(store.config.value)
    await flushPromises()
    expect(wrapper?.text()).toContain('Required.')
  })
})
