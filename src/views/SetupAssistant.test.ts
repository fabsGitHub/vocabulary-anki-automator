import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiStatus, AppConfig, ConsentAction, RecommendedNoteType, SetupStatus } from '../api'
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
import { practiceWord } from '../composables/useSetupAssistant'
import { recommendedMapping } from '../domain/fieldMapping'
import { formatShortcut } from '../domain/shortcuts'
import { i18n, setLocale } from '../i18n'
import {
  callsTo,
  commandNames,
  emitEvent,
  invokeCalls,
  onCommand,
  resetTauri,
} from '../test/fakeTauri'
import SetupAssistant from './SetupAssistant.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)
vi.mock('@tauri-apps/api/event', async () => (await import('../test/fakeTauri')).eventModule)

/** The commands that download, install, start or write anything. */
const ACTIONS = [
  'grant_consent',
  'download_and_install_anki',
  'auto_provision_ankiconnect',
  'launch_anki_desktop',
  'create_recommended_note_type',
  'apply_settings',
  'request_accessibility',
  'close_setup_assistant',
  'restart_app',
]

/** A fake machine: what Anki, the disk and macOS answer, changed by the commands as the real ones would. */
interface Machine {
  setup: SetupStatus
  /** Anki is running with AnkiConnect loaded. */
  online: boolean
  models: string[]
  /** `launch_anki_desktop` brings AnkiConnect up (false: Anki was open before the add-on came). */
  launchConnects: boolean
  accessibilityRequired: boolean
  trusted: boolean
}

let machine: Machine
let wrapper: VueWrapper | null = null
let scope: ReturnType<typeof effectScope> | null = null
let store: ConfigStore
let connection: AnkiConnection

function freshMac(): Machine {
  return {
    setup: {
      anki_installed: false,
      ankiconnect_installed: false,
      platform_supported: true,
      anki_version: '24.06.3',
      consent: { anki_install_granted_at: null, ankiconnect_provision_granted_at: null },
    },
    online: false,
    models: ['Basic'],
    launchConnects: true,
    accessibilityRequired: true,
    trusted: false,
  }
}

function status(): AnkiStatus {
  return machine.online
    ? { online: true, version: 6, decks: ['Default'], models: [...machine.models], error: null }
    : { online: false, version: null, decks: [], models: [], error: 'Connection refused' }
}

function accessibility() {
  return machine.accessibilityRequired
    ? { required: true, trusted: machine.trusted }
    : { required: false, trusted: true }
}

const CREATED: RecommendedNoteType = {
  model_name: 'Polywort',
  mapping: recommendedMapping(),
  created: true,
  matches_by_name: true,
}

function serveMachine(): void {
  onCommand('anki_status', () => status())
  onCommand('get_setup_status', () => machine.setup)
  onCommand('get_accessibility_status', () => accessibility())
  onCommand('request_accessibility', () => accessibility())
  onCommand('grant_consent', (args: { action: ConsentAction } | undefined) => {
    const at = '2026-11-05T10:00:00.000Z'
    if (args?.action === 'anki_install') machine.setup.consent.anki_install_granted_at = at
    else machine.setup.consent.ankiconnect_provision_granted_at = at
    return machine.setup.consent
  })
  onCommand('download_and_install_anki', () => {
    machine.setup.anki_installed = true
    return 'Anki 24.06.3 was installed into /Applications.'
  })
  onCommand('auto_provision_ankiconnect', () => {
    machine.setup.ankiconnect_installed = true
    return 'AnkiConnect was installed. Restart Anki to load it.'
  })
  onCommand('launch_anki_desktop', () => {
    if (machine.launchConnects && machine.setup.anki_installed) machine.online = true
    return 'Starting Anki…'
  })
  onCommand('create_recommended_note_type', () => {
    if (!machine.models.includes('Polywort')) machine.models.push('Polywort')
    return CREATED
  })
  onCommand('apply_settings', null)
  onCommand('close_setup_assistant', (args: { finished: boolean } | undefined) => ({
    closed_at: '2026-11-05T10:30:00.000Z',
    finished: args?.finished ?? false,
  }))
  onCommand('restart_app', () => new Promise(() => {}))
}

/** Lets timers run for `ms` and every promise settle. */
async function settle(ms = 0): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms)
  await flushPromises()
}

async function mountAssistant(config: AppConfig = defaultConfig()): Promise<VueWrapper> {
  onCommand('get_current_settings', config)
  scope = effectScope()
  scope.run(() => {
    store = useConfig({ isMac: true })
    connection = useAnkiConnection({
      pollIntervalMs: 100,
      maxPollAttempts: 3,
      onConsentChanged: (consent) => store.setConsent(consent),
    })
  })
  await store.load()
  await connection.check()
  invokeCalls.length = 0
  wrapper = mount(SetupAssistant, {
    props: { accessibilityPollMs: 1000, ankiPollMs: 2000 },
    global: {
      plugins: [i18n],
      provide: {
        [CONFIG_STORE_KEY as symbol]: store,
        [ANKI_CONNECTION_KEY as symbol]: connection,
      },
    },
  })
  await settle()
  return wrapper
}

const find = (test: string) => wrapper!.find(`[data-test="${test}"]`)
const exists = (test: string) => find(test).exists()
const currentStep = () => find('assistant-detail').attributes('data-step')
const ticked = (step: string) => find(`step-${step}`).attributes('data-done') === 'true'
const stepList = () =>
  wrapper!.findAll('.assistant-step').map((step) => step.attributes('data-test'))
const actionsTaken = () => commandNames().filter((name) => ACTIONS.includes(name))

async function click(test: string): Promise<void> {
  await find(test).trigger('click')
  await settle()
}

beforeEach(() => {
  resetTauri()
  setLocale('en')
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  machine = freshMac()
  serveMachine()
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  scope?.stop()
  scope = null
  vi.useRealTimers()
})

describe('SetupAssistant on a fresh Mac', () => {
  it('only checks when it opens, and starts on the first open step', async () => {
    await mountAssistant()
    expect(actionsTaken()).toEqual([])
    expect(callsTo('get_accessibility_status')).toHaveLength(1)
    expect(stepList()).toEqual([
      'step-anki',
      'step-ankiconnect',
      'step-running',
      'step-note_type',
      'step-accessibility',
      'step-try',
    ])
    expect(currentStep()).toBe('anki')
    expect(find('assistant-detail').text()).toContain('Step 1 of 6: Install Anki')
    expect(exists('install-anki')).toBe(true)

    // Nothing polls while the shown step needs nothing to happen by itself.
    await settle(10_000)
    expect(actionsTaken()).toEqual([])
  })

  it('asks once for every download, and Cancel records and downloads nothing', async () => {
    await mountAssistant()
    await click('set-up-everything')

    const dialogs = wrapper!.findAll('[role="dialog"]')
    expect(dialogs).toHaveLength(1)
    const text = dialogs[0].text()
    expect(text).toContain('Install Anki 24.06.3 and AnkiConnect?')
    expect(text).toContain('github.com/ankitects/anki/releases/tag/24.06.3')
    expect(text).toContain(
      'github.com/FooSoft/anki-connect/tree/4064fa142785975255457abd6a496015f5b71f38/plugin',
    )
    expect(exists('consent-anki_install')).toBe(true)
    expect(exists('consent-ankiconnect_provision')).toBe(true)
    expect(find('consent-plan').text()).toContain('the note type “Polywort”')
    expect(find('consent-plan').text()).toContain('Accessibility')
    expect(actionsTaken()).toEqual([])

    await click('consent-cancel')
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false)
    expect(actionsTaken()).toEqual([])
  })

  it('"Set up everything" installs, starts Anki, saves the note type and asks for the permission', async () => {
    await mountAssistant()
    await click('set-up-everything')
    await click('consent-agree')
    await settle(1000)

    expect(actionsTaken()).toEqual([
      'grant_consent',
      'download_and_install_anki',
      'grant_consent',
      'auto_provision_ankiconnect',
      'launch_anki_desktop',
      'create_recommended_note_type',
      'apply_settings',
      'request_accessibility',
    ])
    expect(callsTo('grant_consent').map((call) => call.args?.action)).toEqual([
      'anki_install',
      'ankiconnect_provision',
    ])
    const saved = callsTo('apply_settings')[0].args?.newCfg as AppConfig
    expect(saved.anki.model_name).toBe('Polywort')
    expect(saved.anki.field_mapping).toBeNull()
    expect(store.dirty.value).toBe(false)

    for (const step of ['anki', 'ankiconnect', 'running', 'note_type']) {
      expect(ticked(step), step).toBe(true)
    }
    expect(currentStep()).toBe('accessibility')
    expect(exists('accessibility-watching')).toBe(true)

    // The user flips the switch; the next poll notices and moves on.
    machine.trusted = true
    await settle(1000)
    expect(ticked('accessibility')).toBe(true)
    expect(currentStep()).toBe('try')
    expect(callsTo('request_accessibility')).toHaveLength(1)
  })

  it('stops at the first failed install and says why', async () => {
    onCommand('download_and_install_anki', () => {
      throw 'The download does not match its pinned SHA-256. It was deleted and nothing was installed.'
    })
    await mountAssistant()
    await click('set-up-everything')
    await click('consent-agree')

    expect(actionsTaken()).toEqual(['grant_consent', 'download_and_install_anki'])
    expect(currentStep()).toBe('anki')
    expect(find('anki-feedback').text()).toContain('does not match its pinned SHA-256')
  })

  it('polls the permission every second only while its step is shown', async () => {
    machine = { ...freshMac(), online: true, models: ['AppleStyleFinalV3'] }
    await mountAssistant()
    expect(currentStep()).toBe('accessibility')
    expect(callsTo('get_accessibility_status')).toHaveLength(1)

    await settle(3000)
    expect(callsTo('get_accessibility_status')).toHaveLength(4)

    await click('step-anki')
    await settle(5000)
    expect(callsTo('get_accessibility_status')).toHaveLength(4)

    await click('step-accessibility')
    await click('open-accessibility')
    expect(callsTo('request_accessibility')).toHaveLength(1)
    machine.trusted = true
    await settle(1000)
    expect(currentStep()).toBe('try')
    expect(actionsTaken()).toEqual(['request_accessibility'])
  })
})

describe('SetupAssistant with work already done', () => {
  it('ticks every step it can check and offers nothing to set up', async () => {
    machine = {
      ...freshMac(),
      online: true,
      models: ['AppleStyleFinalV3'],
      trusted: true,
    }
    await mountAssistant()
    for (const step of ['anki', 'ankiconnect', 'running', 'note_type', 'accessibility']) {
      expect(ticked(step), step).toBe(true)
    }
    expect(ticked('try')).toBe(false)
    expect(currentStep()).toBe('try')
    expect(find('set-up-everything').attributes('disabled')).toBeDefined()

    // The note type the user already has is kept, with the recommended one on offer.
    await click('step-note_type')
    expect(find('note-type-done').text()).toContain('AppleStyleFinalV3')
    expect(find('set-up-note-type').classes()).toContain('btn-secondary')
    expect(actionsTaken()).toEqual([])
  })

  it('downloads only what is missing', async () => {
    machine.setup.anki_installed = true
    await mountAssistant()
    expect(currentStep()).toBe('ankiconnect')
    await click('set-up-everything')
    expect(exists('consent-anki_install')).toBe(false)
    expect(exists('consent-ankiconnect_provision')).toBe(true)
    expect(wrapper!.find('[role="dialog"] h3').text()).toBe('Install the AnkiConnect add-on?')
  })

  it('needs no dialog when nothing is to be downloaded', async () => {
    machine.setup.anki_installed = true
    machine.setup.ankiconnect_installed = true
    await mountAssistant()
    expect(currentStep()).toBe('running')
    await click('set-up-everything')
    await settle(1000)
    expect(wrapper!.find('[role="dialog"]').exists()).toBe(false)
    expect(actionsTaken()).toEqual([
      'launch_anki_desktop',
      'create_recommended_note_type',
      'apply_settings',
      'request_accessibility',
    ])
  })
})

describe('SetupAssistant and an Anki that was open during the install', () => {
  it('asks for a restart of Anki and moves on once AnkiConnect answers', async () => {
    machine.setup.anki_installed = true
    // Anki is open: launching only brings it to the front, without AnkiConnect.
    machine.launchConnects = false
    await mountAssistant()
    await click('set-up-everything')
    await click('consent-agree')
    await settle(1000)

    expect(currentStep()).toBe('running')
    expect(find('restart-anki-hint').text()).toContain('AnkiConnect was just installed')
    expect(find('restart-anki-hint').classes()).toContain('warning')
    expect(find('anki-feedback').text()).toContain('Anki did not answer yet')
    expect(commandNames()).not.toContain('create_recommended_note_type')

    // The step probes Anki every two seconds until the user restarted it.
    const probes = callsTo('anki_status').length
    await settle(4000)
    expect(callsTo('anki_status').length).toBe(probes + 2)
    machine.online = true
    await settle(2000)
    expect(ticked('running')).toBe(true)
    expect(currentStep()).toBe('note_type')
    expect(commandNames()).not.toContain('create_recommended_note_type')
  })
})

describe('SetupAssistant: the note type', () => {
  it('creates and saves the recommended note type from its step', async () => {
    machine = { ...freshMac(), online: true, trusted: true }
    await mountAssistant()
    expect(currentStep()).toBe('note_type')
    expect(find('set-up-note-type').text()).toBe('Create the recommended note type')
    await click('set-up-note-type')
    expect(actionsTaken()).toEqual(['create_recommended_note_type', 'apply_settings'])
    expect(store.saved.value.anki.model_name).toBe('Polywort')
    expect(currentStep()).toBe('try')
  })

  it('shows why a note type could not be set up and stays on the step', async () => {
    machine = { ...freshMac(), online: true, trusted: true }
    onCommand('create_recommended_note_type', () => {
      throw 'Anki already has a note type named “Polywort”, but some of its fields are missing: “Audio”.'
    })
    await mountAssistant()
    await click('set-up-note-type')
    expect(currentStep()).toBe('note_type')
    expect(find('note-type-error').text()).toContain('some of its fields are missing')
    expect(commandNames()).not.toContain('apply_settings')
  })
})

describe('SetupAssistant: trying it', () => {
  function ready(): void {
    machine = { ...freshMac(), online: true, models: ['AppleStyleFinalV3'], trusted: true }
  }

  it('shows the practice word and the configured hotkey, and notices the capture', async () => {
    ready()
    const config = defaultConfig()
    config.hotkeys = 'CommandOrControl+Alt+V'
    config.translation.source_lang = 'fr'
    await mountAssistant(config)
    expect(currentStep()).toBe('try')
    expect(find('practice-word').text()).toBe('papillon')
    // ⌥⌘V on a Mac, Ctrl+Alt+V elsewhere.
    expect(find('assistant-detail').text()).toContain(formatShortcut('CommandOrControl+Alt+V'))

    emitEvent('translation-payload', {})
    await settle()
    expect(exists('try-success')).toBe(true)
    expect(ticked('try')).toBe(true)
  })

  it('offers a restart of Polywort when the copy fails although the permission is granted', async () => {
    ready()
    await mountAssistant()
    emitEvent('capture-error', { message: 'Nothing was copied.' })
    await settle()
    expect(find('try-error').text()).toContain('Nothing was copied.')
    expect(exists('restart-offer')).toBe(true)
    await click('restart-app')
    expect(actionsTaken()).toEqual(['restart_app'])
    expect(find('restart-app').text()).toBe('Restarting…')
  })

  it('points back to the permission instead when it is still missing', async () => {
    ready()
    machine.trusted = false
    await mountAssistant()
    await click('step-try')
    emitEvent('capture-error', { message: 'Nothing was copied.' })
    await settle()
    expect(exists('restart-offer')).toBe(false)
    expect(find('assistant-detail').text()).toContain('finish the step “Allow copying” first')
  })

  it('ignores captures made while another step is shown', async () => {
    await mountAssistant()
    emitEvent('translation-payload', {})
    await settle()
    expect(ticked('try')).toBe(false)
  })

  it('picks the practice word in the first language the user learns', () => {
    const config = defaultConfig()
    expect(practiceWord(config)).toBe('butterfly')
    config.learning.languages = [
      {
        lang: 'zh-CN',
        deck_name: 'chinese',
        model_name: 'Polywort',
        field_mapping: null,
        create_reverse: true,
      },
    ]
    expect(practiceWord(config)).toBe('蝴蝶')
  })
})

describe('SetupAssistant: closing', () => {
  it('records "Done" as finished and closes', async () => {
    machine = { ...freshMac(), online: true, models: ['AppleStyleFinalV3'], trusted: true }
    await mountAssistant()
    await click('assistant-done')
    expect(callsTo('close_setup_assistant').map((call) => call.args)).toEqual([{ finished: true }])
    expect(wrapper!.emitted('close')).toHaveLength(1)
    expect(store.saved.value.setup_assistant.finished).toBe(true)
    expect(store.dirty.value).toBe(false)
  })

  it('records a dismissal from any step', async () => {
    await mountAssistant()
    await click('assistant-dismiss')
    expect(callsTo('close_setup_assistant').map((call) => call.args)).toEqual([{ finished: false }])
    expect(wrapper!.emitted('close')).toHaveLength(1)
    expect(actionsTaken()).toEqual(['close_setup_assistant'])
  })

  it('still closes when the close could not be saved', async () => {
    onCommand('close_setup_assistant', () => {
      throw 'The setup assistant could not be closed: disk full'
    })
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    await mountAssistant()
    await click('assistant-dismiss')
    expect(wrapper!.emitted('close')).toHaveLength(1)
    warn.mockRestore()
  })
})

describe('SetupAssistant on Windows and Linux', () => {
  it('has no Accessibility step', async () => {
    machine = { ...freshMac(), accessibilityRequired: false }
    await mountAssistant()
    expect(stepList()).not.toContain('step-accessibility')
    expect(find('assistant-detail').text()).toContain('Step 1 of 5')
    await click('set-up-everything')
    expect(find('consent-plan').text()).not.toContain('Accessibility')
  })
})

describe('SetupAssistant in German', () => {
  it('speaks German', async () => {
    setLocale('de')
    await mountAssistant()
    expect(find('setup-assistant').text()).toContain('Polywort einrichten')
    expect(find('set-up-everything').text()).toBe('Alles einrichten')
  })
})
