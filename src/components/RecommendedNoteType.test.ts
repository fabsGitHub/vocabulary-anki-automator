import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiConfig, AnkiStatus, RecommendedNoteType as NoteType } from '../api'
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
import { recommendedMapping, selectRecommendedNoteType } from '../domain/fieldMapping'
import { i18n, setLocale } from '../i18n'
import {
  callsTo,
  commandNames,
  deferred,
  invokeCalls,
  onCommand,
  resetTauri,
} from '../test/fakeTauri'
import RecommendedNoteType from './RecommendedNoteType.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

const ONLINE: AnkiStatus = {
  online: true,
  version: 6,
  decks: ['Default'],
  models: ['Basic', 'AppleStyleFinalV3'],
  error: null,
}
const WITH_RECOMMENDED: AnkiStatus = {
  ...ONLINE,
  models: [...ONLINE.models, 'Vocabulary Automator'],
}
const OFFLINE: AnkiStatus = { online: false, version: null, decks: [], models: [], error: 'x' }
const SETUP = {
  anki_installed: true,
  ankiconnect_installed: true,
  platform_supported: true,
  anki_version: '24.06.3',
  consent: { anki_install_granted_at: null, ankiconnect_provision_granted_at: null },
}

/** What `create_recommended_note_type` answers after creating the note type (ipc_anki.rs). */
const CREATED: NoteType = {
  model_name: 'Vocabulary Automator',
  mapping: recommendedMapping(),
  created: true,
  matches_by_name: true,
}

let wrapper: VueWrapper | null = null
let scope: ReturnType<typeof effectScope> | null = null
let connection: AnkiConnection
let store: ConfigStore
let used: NoteType[] = []

/**
 * The button for the Anki tab's note type, `modelName` in the form; the
 * saved settings name `saved` (by default the same note type).
 */
async function mountButton(
  status: AnkiStatus,
  modelName = 'AppleStyleFinalV3',
  saved: Partial<AnkiConfig> = {},
) {
  onCommand('anki_status', status)
  onCommand('get_setup_status', SETUP)
  const config = defaultConfig()
  onCommand('get_current_settings', {
    ...config,
    anki: { ...config.anki, model_name: modelName, ...saved },
  })
  scope = effectScope()
  scope.run(() => {
    connection = useAnkiConnection()
    store = useConfig({ isMac: false })
  })
  await connection.check()
  await store.load()
  store.config.value.anki.model_name = modelName
  invokeCalls.length = 0
  used = []
  wrapper = mount(RecommendedNoteType, {
    props: {
      modelName,
      onUse: (noteType: NoteType) => {
        used.push(noteType)
        // The parent selects the note type, as both settings tabs do.
        selectRecommendedNoteType(store.config.value.anki, noteType)
        void wrapper?.setProps({ modelName: noteType.model_name })
      },
    },
    global: {
      plugins: [i18n],
      provide: {
        [ANKI_CONNECTION_KEY as symbol]: connection,
        [CONFIG_STORE_KEY as symbol]: store,
      },
    },
  })
  await flushPromises()
  return wrapper
}

const find = (test: string) => wrapper!.find(`[data-test="${test}"]`)
const button = () => find('create-note-type')

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

describe('when it is offered', () => {
  it('is hidden while Anki is offline, and asks Anki nothing', async () => {
    await mountButton(OFFLINE)
    expect(find('recommended-note-type').exists()).toBe(false)
    expect(invokeCalls).toEqual([])
  })

  it('offers to create the note type, saying what it adds, without asking Anki anything', async () => {
    await mountButton(ONLINE)
    expect(button().text()).toBe('Create the recommended note type')
    expect(button().attributes('disabled')).toBeUndefined()
    expect(find('note-type-hint').text()).toBe(
      'Adds the note type “Vocabulary Automator” to Anki, with a field for everything a card holds: word, translation, example and its translation, pronunciation, pinyin, HSK level and notes.',
    )
    expect(invokeCalls).toEqual([])
  })

  it('offers to use it when Anki already has it', async () => {
    await mountButton(WITH_RECOMMENDED)
    expect(button().text()).toBe('Use the recommended note type')
    expect(find('note-type-hint').text()).toBe(
      'Anki already has a note type named “Vocabulary Automator”. It is selected here if it has every recommended field, and it is never changed.',
    )
  })

  it('is hidden where the note type is already selected', async () => {
    await mountButton(WITH_RECOMMENDED, ' Vocabulary Automator ')
    expect(find('recommended-note-type').exists()).toBe(false)
  })

  it('offers to create it where it is selected but Anki lacks it, without asking to save', async () => {
    await mountButton(ONLINE, 'Vocabulary Automator', { field_mapping: null })
    expect(button().text()).toBe('Create the recommended note type')
    onCommand('create_recommended_note_type', CREATED)
    onCommand('anki_status', WITH_RECOMMENDED)
    await button().trigger('click')
    await flushPromises()
    expect(used).toEqual([CREATED])
    expect(button().exists()).toBe(false)
    expect(store.dirty.value).toBe(false)
    expect(find('note-type-result').text()).toBe(
      '“Vocabulary Automator” was added to Anki. It is selected here, and the saved settings already use it.',
    )
    expect(find('note-type-result').classes()).toContain('success')
  })

  it('says it was already selected where only the unsaved form selects it', async () => {
    await mountButton(ONLINE, 'Vocabulary Automator', { model_name: 'AppleStyleFinalV3' })
    expect(store.dirty.value).toBe(true)
    onCommand('create_recommended_note_type', CREATED)
    await button().trigger('click')
    await flushPromises()
    expect(find('note-type-result').text()).toBe(
      '“Vocabulary Automator” was added to Anki. It is already selected here.',
    )
  })

  it('is disabled while another Anki operation runs', async () => {
    await mountButton(ONLINE)
    connection.activity.value = 'launch'
    await flushPromises()
    expect(button().attributes('disabled')).toBeDefined()
    expect(button().text()).toBe('Create the recommended note type')
  })

  it('speaks German', async () => {
    setLocale('de')
    await mountButton(ONLINE)
    expect(button().text()).toBe('Empfohlenen Notiztyp anlegen')
    expect(find('note-type-hint').text()).toContain('„Vocabulary Automator“')
  })
})

describe('a click', () => {
  it('creates the note type, hands it to the parent and lists it', async () => {
    await mountButton(ONLINE)
    const answer = deferred<NoteType>()
    onCommand('create_recommended_note_type', () => answer.promise)

    await button().trigger('click')
    expect(button().text()).toBe('Creating the note type…')
    expect(button().attributes('disabled')).toBeDefined()
    expect(connection.activity.value).toBe('note_type')

    onCommand('anki_status', WITH_RECOMMENDED)
    answer.resolve(CREATED)
    await flushPromises()

    expect(callsTo('create_recommended_note_type')).toEqual([
      { cmd: 'create_recommended_note_type', args: undefined },
    ])
    expect(used).toEqual([CREATED])
    expect(commandNames()).toEqual(['create_recommended_note_type', 'anki_status'])
    expect(connection.models.value).toContain('Vocabulary Automator')
    expect(connection.activity.value).toBeNull()
    // Selected now: the offer is gone, the result stays.
    expect(button().exists()).toBe(false)
    expect(find('note-type-result').text()).toBe(
      '“Vocabulary Automator” was added to Anki. It is selected here; save the settings to use it.',
    )
    expect(find('note-type-result').classes()).toContain('success')
    expect(store.dirty.value).toBe(true)
  })

  it('says so when Anki already had the note type', async () => {
    await mountButton(WITH_RECOMMENDED)
    onCommand('create_recommended_note_type', { ...CREATED, created: false })
    await button().trigger('click')
    await flushPromises()
    expect(used).toHaveLength(1)
    expect(find('note-type-result').text()).toBe(
      'Anki already has “Vocabulary Automator”, with every recommended field. It is selected here; save the settings to use it.',
    )
  })

  it('drops the request to save once the settings are saved', async () => {
    await mountButton(ONLINE)
    onCommand('create_recommended_note_type', CREATED)
    onCommand('anki_status', WITH_RECOMMENDED)
    await button().trigger('click')
    await flushPromises()
    expect(find('note-type-result').text()).toContain('save the settings')

    onCommand('apply_settings', null)
    expect(await store.save()).toBe(true)
    await flushPromises()
    expect(find('note-type-result').exists()).toBe(false)
    expect(find('recommended-note-type').exists()).toBe(false)
  })

  it('shows the error and selects nothing when the note type cannot be set up', async () => {
    await mountButton(WITH_RECOMMENDED)
    const message =
      'Anki already has a note type named “Vocabulary Automator”, but some of its fields are missing: “Pinyin”.'
    onCommand('create_recommended_note_type', () => Promise.reject(message))
    await button().trigger('click')
    await flushPromises()

    expect(used).toEqual([])
    expect(find('note-type-result').text()).toBe(`The note type could not be set up: ${message}`)
    expect(find('note-type-result').attributes('role')).toBe('alert')
    expect(button().attributes('disabled')).toBeUndefined()
    expect(button().text()).toBe('Use the recommended note type')
  })

  it('forgets its result once the place switches to another note type', async () => {
    await mountButton(ONLINE)
    onCommand('create_recommended_note_type', () => Promise.reject('boom'))
    await button().trigger('click')
    await flushPromises()
    expect(find('note-type-result').exists()).toBe(true)

    await wrapper!.setProps({ modelName: 'Basic' })
    expect(find('note-type-result').exists()).toBe(false)
  })

  it('does nothing while another operation runs', async () => {
    await mountButton(ONLINE)
    connection.activity.value = 'rescan'
    expect(await connection.createRecommendedNoteType()).toBeNull()
    expect(invokeCalls).toEqual([])
  })

  it('goes offline with Anki when Anki stopped answering', async () => {
    await mountButton(ONLINE)
    onCommand('create_recommended_note_type', () =>
      Promise.reject('could not reach AnkiConnect: connection refused'),
    )
    onCommand('anki_status', OFFLINE)
    await button().trigger('click')
    await flushPromises()

    expect(connection.state.value).toBe('anki_closed')
    expect(button().exists()).toBe(false)
    expect(find('note-type-result').text()).toBe(
      'The note type could not be set up: could not reach AnkiConnect: connection refused',
    )
  })
})
