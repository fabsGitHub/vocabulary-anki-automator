import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { effectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiStatus, AppConfig, AutoMatch, LearnedLanguage } from '../api'
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
import { appleStyleMapping, chineseMapping, recommendedMapping } from '../domain/fieldMapping'
import { i18n, setLocale } from '../i18n'
import { callsTo, deferred, invokeCalls, onCommand, resetTauri } from '../test/fakeTauri'
import LearnedLanguagesEditor from './LearnedLanguagesEditor.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

const ONLINE: AnkiStatus = {
  online: true,
  version: 6,
  decks: ['Default', 'french'],
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
const SETUP = {
  anki_installed: true,
  ankiconnect_installed: true,
  platform_supported: true,
  anki_version: '24.06.3',
  consent: { anki_install_granted_at: null, ankiconnect_provision_granted_at: null },
}

const APPLE_V3_FIELDS = ['Front', 'Back', 'English Sentence', 'German Sentence', 'Audio']

/** What `suggest_field_mapping` answers for AppleStyleFinalV3, en -> de (ipc_anki.rs). */
const APPLE_V3_EN: AutoMatch = {
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

/** The same note type for French: the English sentence has no French counterpart. */
const APPLE_V3_FR: AutoMatch = {
  mapping: {
    fields: [
      { anki_field: 'Front', source: 'front' },
      { anki_field: 'Back', source: 'back' },
      { anki_field: 'German Sentence', source: 'example_in_target_lang' },
    ],
    audio_field: 'Audio',
  },
  guessed: [],
  unmatched: ['English Sentence'],
}

/** A Chinese note type for Chinese cards: every value has its field. */
const CHINESE_ZH: AutoMatch = {
  mapping: {
    fields: [
      { anki_field: 'Hanzi', source: 'front' },
      { anki_field: 'Pinyin', source: 'pinyin' },
      { anki_field: 'Meaning', source: 'back' },
    ],
    audio_field: 'Audio',
  },
  guessed: [],
  unmatched: [],
}

function entry(overrides: Partial<LearnedLanguage> = {}): LearnedLanguage {
  return {
    lang: 'fr',
    deck_name: 'french',
    model_name: 'AppleStyleFinalV3',
    field_mapping: null,
    create_reverse: true,
    ...overrides,
  }
}

function settings(languages: LearnedLanguage[] = []): AppConfig {
  const config = defaultConfig()
  config.learning.languages = languages
  return config
}

let wrapper: VueWrapper | null = null
let scope: ReturnType<typeof effectScope> | null = null
let store: ConfigStore
let connection: AnkiConnection

async function mountEditor(config: AppConfig = settings()): Promise<VueWrapper> {
  onCommand('get_current_settings', config)
  scope = effectScope()
  scope.run(() => {
    store = useConfig({ isMac: false })
    connection = useAnkiConnection()
  })
  await store.load()
  await connection.check()
  invokeCalls.length = 0
  wrapper = mount(LearnedLanguagesEditor, {
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

const cards = () => wrapper?.findAll('[data-test="learned-language"]') ?? []
const card = (index: number) => {
  const found = cards()[index]
  if (!found) throw new Error(`no entry ${index}`)
  return found
}
const languageOptions = (index: number) =>
  card(index)
    .findAll(`#learned-${index}-lang option`)
    .map((option) => option.attributes('value'))
const chips = (index: number, selector: string) =>
  card(index)
    .findAll(`${selector} code.field-chip`)
    .map((code) => code.text())

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

describe('the list of languages', () => {
  beforeEach(() => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', SETUP)
  })

  it('starts empty, naming the language cards are translated into', async () => {
    await mountEditor()
    expect(wrapper?.find('[data-test="no-languages"]').text()).toContain('No languages yet.')
    expect(wrapper?.find('[data-test="native-language"]').text()).toBe(
      'Your language: German. Cards are translated into it (set on “Capture & Language”).',
    )
    expect(cards()).toHaveLength(0)
  })

  it('adds prefilled entries, the first free language each time, and removes them', async () => {
    await mountEditor()
    store.config.value.anki.model_name = 'Basic'
    const add = wrapper?.find('[data-test="add-language"]')
    expect(add?.attributes('disabled')).toBeUndefined()
    expect(add?.attributes('title')).toBeUndefined()
    await add?.trigger('click')
    await add?.trigger('click')

    expect(store.config.value.learning.languages).toEqual([
      entry({ lang: 'en', deck_name: 'english', model_name: 'Basic' }),
      entry({ lang: 'fr', deck_name: 'french', model_name: 'Basic' }),
    ])
    expect(cards()).toHaveLength(2)
    expect(wrapper?.find('[data-test="no-languages"]').exists()).toBe(false)
    expect(store.dirty.value).toBe(true)

    await card(0).find('[data-test="remove-language"]').trigger('click')
    expect(store.config.value.learning.languages.map((language) => language.lang)).toEqual(['fr'])
    expect(cards()).toHaveLength(1)
  })

  it('edits an entry, and changing its language keeps what was edited', async () => {
    await mountEditor(settings([entry()]))
    const first = card(0)
    await first.find('[data-test="learned-deck"]').setValue('Französisch')
    await first.find('[data-test="learned-model"]').setValue('Basic')
    await first.find('[data-test="learned-reverse"]').setValue(false)
    await first.find('#learned-0-lang').setValue('es')

    expect(store.config.value.learning.languages[0]).toEqual(
      entry({ lang: 'es', deck_name: 'Französisch', model_name: 'Basic', create_reverse: false }),
    )
    expect(store.issues.value).toEqual([])
    expect(first.find('[data-test="learned-deck"]').attributes('placeholder')).toBe('e.g. spanish')
  })

  it('brings the deck and reverse default of a new language along while they are untouched', async () => {
    await mountEditor()
    expect(store.config.value.anki.create_reverse).toBe(true)
    await wrapper?.find('[data-test="add-language"]').trigger('click')
    expect(store.config.value.learning.languages[0]).toMatchObject({
      lang: 'en',
      deck_name: 'english',
      create_reverse: true,
    })

    await card(0).find('#learned-0-lang').setValue('zh-CN')
    expect(store.config.value.learning.languages[0]).toMatchObject({
      lang: 'zh-CN',
      deck_name: 'chinese',
      create_reverse: false,
    })
    expect(card(0).find<HTMLInputElement>('[data-test="learned-deck"]').element.value).toBe(
      'chinese',
    )

    await card(0).find('#learned-0-lang').setValue('en')
    expect(store.config.value.learning.languages[0]).toMatchObject({
      lang: 'en',
      deck_name: 'english',
      create_reverse: store.config.value.anki.create_reverse,
    })
  })

  it('offers only languages that are neither the native one nor taken by another entry', async () => {
    await mountEditor(settings([entry({ lang: 'fr' }), entry({ lang: 'zh-CN' })]))
    expect(languageOptions(0)).toEqual(['en', 'fr', 'es', 'it'])
    expect(languageOptions(1)).toEqual(['en', 'es', 'it', 'zh-CN'])
    expect(card(1).find('#learned-1-lang option[value="zh-CN"]').text()).toBe(
      'Chinese (Simplified)',
    )
  })

  it('cannot add a language once every supported one is listed', async () => {
    await mountEditor(settings(['en', 'fr', 'es', 'it', 'zh-CN'].map((lang) => entry({ lang }))))
    const add = wrapper?.find('[data-test="add-language"]')
    expect(add?.attributes('disabled')).toBeDefined()
    expect(add?.attributes('title')).toBe('Every supported language is listed.')
  })

  it('shows the problems of each entry next to its fields', async () => {
    await mountEditor(
      settings([
        entry({ lang: 'de', deck_name: ' ', model_name: '' }),
        entry({ lang: 'fr', field_mapping: { fields: [], audio_field: null } }),
        entry({ lang: 'fr' }),
      ]),
    )
    expect(card(0).find('#learned-0-lang').classes()).toContain('has-error')
    expect(card(0).text()).toContain('Cards are translated into this language; choose another one.')
    expect(
      card(0)
        .findAll('.field-error')
        .map((error) => error.text()),
    ).toEqual([
      'Cards are translated into this language; choose another one.',
      'Required.',
      'Required.',
    ])
    expect(card(1).find('[data-test="mapping-issue"]').text()).toBe('Fill at least one Anki field.')
    expect(card(2).find('.field-error').text()).toBe('This language is already listed.')
    // A mapping matched by name is never empty.
    expect(card(2).find('[data-test="mapping-issue"]').exists()).toBe(false)
  })
})

describe('offline', () => {
  beforeEach(() => {
    onCommand('anki_status', OFFLINE)
    onCommand('get_setup_status', SETUP)
  })

  it('types the note type in, and says the match needs Anki', async () => {
    await mountEditor(settings([entry()]))
    const model = card(0).find('[data-test="learned-model"]')
    expect(model.element.tagName).toBe('INPUT')
    await model.setValue('Chinese')
    expect(store.config.value.learning.languages[0].model_name).toBe('Chinese')
    expect(card(0).find('[data-test="mapping-badge"]').text()).toBe('Matched by field name')
    expect(card(0).find('[data-test="mapping-badge"]').classes()).toContain('badge-muted')
    expect(card(0).find('[data-test="mapping-offline"]').text()).toBe(
      'Connect to Anki to see how the fields of “Chinese” are matched.',
    )
    expect(callsTo('suggest_field_mapping')).toHaveLength(0)
    expect(callsTo('anki_model_field_names')).toHaveLength(0)
  })

  it('cannot tell whether a Chinese entry matched by name reaches a Pinyin field', async () => {
    await mountEditor(settings([entry({ lang: 'zh-CN', deck_name: 'chinese' })]))
    expect(card(0).find('[data-test="chinese-warning"]').exists()).toBe(false)
  })

  it('Customize copies the global mapping', async () => {
    const config = settings([entry()])
    config.anki.field_mapping = chineseMapping()
    await mountEditor(config)
    await card(0).find('[data-test="customize-mapping"]').trigger('click')
    expect(store.config.value.learning.languages[0].field_mapping).toEqual(chineseMapping())
    expect(store.config.value.learning.languages[0].field_mapping).not.toBe(
      store.config.value.anki.field_mapping,
    )
    expect(store.dirty.value).toBe(true)
    expect(card(0).find('[data-test="mapping-badge"]').text()).toBe('Customized')
  })

  it('Customize starts from the Apple-style preset while the global mapping is matched by name too', async () => {
    await mountEditor(settings([entry()]))
    expect(store.config.value.anki.field_mapping).toBeNull()
    await card(0).find('[data-test="customize-mapping"]').trigger('click')
    expect(store.config.value.learning.languages[0].field_mapping).toEqual(appleStyleMapping())
    // An entry's own mapping is a choice even when it equals the preset.
    expect(store.dirty.value).toBe(true)
  })
})

describe('online', () => {
  beforeEach(() => {
    onCommand('anki_status', ONLINE)
    onCommand('anki_model_field_names', APPLE_V3_FIELDS)
    onCommand('suggest_field_mapping', ({ sourceLang }: { sourceLang: string }) =>
      sourceLang === 'fr' ? APPLE_V3_FR : APPLE_V3_EN,
    )
  })

  it('lists the note types Anki has, keeping a configured one it lacks', async () => {
    await mountEditor(settings([entry({ model_name: 'Gone' })]))
    const model = card(0).find('[data-test="learned-model"]')
    expect(model.element.tagName).toBe('SELECT')
    expect(model.findAll('option').map((option) => option.text())).toEqual([
      'Basic',
      'AppleStyleFinalV3',
      'Chinese',
      'Gone (not in Anki)',
    ])
    expect(wrapper?.findAll('#learned-decks option').map((o) => o.attributes('value'))).toEqual([
      'Default',
      'french',
    ])
  })

  it("previews the match for the user's AppleStyleFinalV3 without touching the form", async () => {
    await mountEditor(settings([entry({ lang: 'en', deck_name: 'english' })]))
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
    ])
    const preview = card(0).find('[data-test="mapping-preview"]')
    expect(
      preview.findAll('tbody tr').map((row) => row.findAll('td').map((cell) => cell.text())),
    ).toEqual([
      ['Front', 'Word (front)'],
      ['Back', 'Translation (back)'],
      ['English Sentence', 'Example in the language learned'],
      ['German Sentence', 'Example in your language'],
    ])
    expect(card(0).find('[data-test="preview-audio"]').text()).toBe('Pronunciation → Audio')
    expect(card(0).find('[data-test="matched-unmatched"]').exists()).toBe(false)
    expect(card(0).find('[data-test="matched-guessed"]').exists()).toBe(false)
    expect(card(0).text()).toContain('Each field of “AppleStyleFinalV3” is filled')
    expect(store.config.value.learning.languages[0].field_mapping).toBeNull()
    expect(store.dirty.value).toBe(false)
  })

  it('lists the fields the match leaves empty or could only fill by position', async () => {
    onCommand('suggest_field_mapping', {
      mapping: { fields: [{ anki_field: 'Q', source: 'front' }], audio_field: null },
      guessed: ['Q'],
      unmatched: ['Add Reverse'],
    } satisfies AutoMatch)
    await mountEditor(settings([entry({ model_name: 'Odd' })]))
    expect(chips(0, '[data-test="matched-guessed"]')).toEqual(['Q'])
    expect(chips(0, '[data-test="matched-unmatched"]')).toEqual(['Add Reverse'])
    expect(card(0).find('[data-test="preview-audio"]').text()).toBe(
      'No field for the pronunciation',
    )
  })

  it('matches each entry for its own language, once per note type and pair', async () => {
    await mountEditor(settings([entry({ lang: 'fr' }), entry({ lang: 'en', deck_name: 'en' })]))
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'fr', targetLang: 'de' },
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(chips(0, '[data-test="matched-unmatched"]')).toEqual(['English Sentence'])
    expect(card(1).find('[data-test="matched-unmatched"]').exists()).toBe(false)

    // Editing something else asks nothing again; a new note type does.
    await card(0).find('[data-test="learned-deck"]').setValue('fr')
    await card(0).find('[data-test="learned-model"]').setValue('Basic')
    await flushPromises()
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toHaveLength(3)
    expect(callsTo('suggest_field_mapping')[2].args).toEqual({
      model: 'Basic',
      sourceLang: 'fr',
      targetLang: 'de',
    })
    expect(store.issues.value).toEqual([])
  })

  it('asks for no match without a note type, and shows why a match failed', async () => {
    onCommand('suggest_field_mapping', () => {
      throw 'AnkiConnect error: model was not found: Gone'
    })
    await mountEditor(
      settings([entry({ model_name: '' }), entry({ lang: 'en', model_name: 'Gone' })]),
    )
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'Gone', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(card(0).find('[data-test="mapping-preview"]').exists()).toBe(false)
    expect(card(1).find('[data-test="preview-error"]').text()).toBe(
      'The fields could not be matched: AnkiConnect error: model was not found: Gone',
    )
  })

  it('Customize saves the previewed match, waiting for it first', async () => {
    const answer = deferred<AutoMatch>()
    onCommand('suggest_field_mapping', () => answer.promise)
    await mountEditor(settings([entry({ lang: 'en' })]))
    const customize = card(0).find('[data-test="customize-mapping"]')
    expect(customize.attributes('disabled')).toBeDefined()
    expect(card(0).text()).toContain('Matching…')

    answer.resolve(APPLE_V3_EN)
    await flushPromises()
    expect(store.dirty.value).toBe(false)
    await customize.trigger('click')
    await flushPromises()

    expect(store.config.value.learning.languages[0].field_mapping).toEqual(APPLE_V3_EN.mapping)
    expect(store.dirty.value).toBe(true)
    expect(card(0).find('[data-test="mapping-badge"]').text()).toBe('Customized')
    expect(card(0).find('[data-test="mapping-badge"]').classes()).not.toContain('badge-muted')
    // The saved mapping is edited against the note type's fields.
    expect(callsTo('anki_model_field_names').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3' },
    ])
    expect(
      card(0)
        .findAll('tr[data-field]')
        .map((row) => row.attributes('data-field')),
    ).toEqual(APPLE_V3_FIELDS)
    expect(card(0).find('[data-test="audio-field"]').attributes('id')).toBe(
      'learned-0-mapping-audio',
    )
  })

  it('edits a saved mapping and matches it by name again', async () => {
    await mountEditor(settings([entry({ lang: 'en', field_mapping: appleStyleMapping() })]))
    expect(callsTo('suggest_field_mapping')).toHaveLength(0)
    expect(card(0).find('[data-test="missing-fields"]').exists()).toBe(true)
    await card(0).find('tr[data-field="German Sentence"] select').setValue('example_in_target_lang')
    expect(store.config.value.learning.languages[0].field_mapping?.fields).toContainEqual({
      anki_field: 'German Sentence',
      source: 'example_in_target_lang',
    })

    await card(0).find('[data-test="rematch-mapping"]').trigger('click')
    await flushPromises()
    expect(store.config.value.learning.languages[0].field_mapping).toBeNull()
    expect(card(0).find('[data-test="mapping-badge"]').text()).toBe('Matched by field name')
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
    ])
    expect(card(0).find('[data-test="mapping-preview"]').exists()).toBe(true)
  })

  it('warns when a saved Chinese mapping leaves the pinyin out', async () => {
    await mountEditor(
      settings([
        entry({ lang: 'zh-CN', deck_name: 'chinese', field_mapping: appleStyleMapping() }),
        entry({ lang: 'fr', field_mapping: appleStyleMapping() }),
      ]),
    )
    const warning = card(0).find('[data-test="chinese-warning"]')
    expect(warning.text()).toContain('Hanzi, Pinyin, Meaning and Audio')
    expect(card(1).find('[data-test="chinese-warning"]').exists()).toBe(false)

    await card(0).find('[data-test="preset-chinese"]').trigger('click')
    expect(card(0).find('[data-test="chinese-warning"]').exists()).toBe(false)
    // Matched by name, AppleStyleFinalV3 has no Pinyin field either.
    await card(0).find('[data-test="rematch-mapping"]').trigger('click')
    await flushPromises()
    expect(card(0).find('[data-test="mapping-preview"]').exists()).toBe(true)
    expect(card(0).find('[data-test="chinese-warning"]').exists()).toBe(true)
  })

  it('warns when the match by name of a Chinese entry fills no Pinyin field', async () => {
    onCommand('suggest_field_mapping', ({ model }: { model: string }) =>
      model === 'Chinese' ? CHINESE_ZH : APPLE_V3_EN,
    )
    await mountEditor(
      settings([entry({ lang: 'zh-CN', deck_name: 'chinese', model_name: 'Chinese' })]),
    )
    expect(card(0).find('[data-test="mapping-preview"]').exists()).toBe(true)
    expect(card(0).find('[data-test="chinese-warning"]').exists()).toBe(false)

    await card(0).find('[data-test="learned-model"]').setValue('AppleStyleFinalV3')
    await flushPromises()
    expect(card(0).find('[data-test="chinese-warning"]').text()).toContain(
      'Hanzi, Pinyin, Meaning and Audio',
    )
    expect(store.config.value.learning.languages[0].field_mapping).toBeNull()
  })
})

describe('the recommended note type', () => {
  beforeEach(() => {
    onCommand('anki_status', ONLINE)
    onCommand('anki_model_field_names', APPLE_V3_FIELDS)
    onCommand('suggest_field_mapping', APPLE_V3_FR)
  })

  it('is offered in each entry, and selected only in the one clicked', async () => {
    const saved = appleStyleMapping()
    await mountEditor(
      settings([entry(), entry({ lang: 'es', deck_name: 'spanish', field_mapping: saved })]),
    )
    expect(cards().map((each) => each.find('[data-test="create-note-type"]').exists())).toEqual([
      true,
      true,
    ])
    onCommand('create_recommended_note_type', {
      model_name: 'Vocabulary Automator',
      mapping: recommendedMapping(),
      created: true,
      matches_by_name: true,
    })
    onCommand('anki_status', { ...ONLINE, models: [...ONLINE.models, 'Vocabulary Automator'] })

    await card(1).find('[data-test="create-note-type"]').trigger('click')
    await flushPromises()

    const [french, spanish] = store.config.value.learning.languages
    expect(spanish).toMatchObject({ model_name: 'Vocabulary Automator', field_mapping: null })
    expect(french).toEqual(entry())
    expect(callsTo('create_recommended_note_type')).toHaveLength(1)
    expect(card(1).find('[data-test="create-note-type"]').exists()).toBe(false)
    expect(card(1).find('[data-test="note-type-result"]').exists()).toBe(true)
    // The other entry may now use it too: Anki lists it.
    expect(card(0).find('[data-test="create-note-type"]').text()).toBe(
      'Use the recommended note type',
    )
    expect(card(0).find('[data-test="note-type-result"]').exists()).toBe(false)
    expect(store.dirty.value).toBe(true)
  })
})
