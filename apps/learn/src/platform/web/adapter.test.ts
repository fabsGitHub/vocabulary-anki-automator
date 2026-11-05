/**
 * What the web adapter does beyond the replayed Learn commands
 * (golden.test.ts): what a browser cannot do, the settings, the arguments
 * the desktop app would not deserialize, the lazily seeded curriculum, a
 * store that cannot be opened, and one clock reading per command.
 */

import { describe, expect, it } from 'vitest'

import type { Exercise, ExerciseAnswer, LearnSettings, Rating } from '../types'
import { WebAdapter } from './adapter'
import { createWebAdapter } from './index'
import { type CurriculumItem, newCard } from './model'
import { SETTINGS_KEY, defaultSettings } from './settings'
import { MemoryStorage } from './storage'
import { Store } from './store'
import { type Clock, FixedClock, nanosFromMillis, parseInstant, parseInstantMillis } from './time'

const START = parseInstant('2026-10-31T08:00:00.000Z')

function item(hanzi: string, order: number): CurriculumItem {
  return {
    hanzi,
    traditional: null,
    hsk_level: 1,
    frequency: null,
    curriculum_order: order,
    pinyin: `pinyin of ${hanzi}`,
    pos: null,
    meaning_en: `${hanzi} (en)`,
    meaning_de: null,
    cloze_sentence: null,
    example_sentence_source: null,
    example_sentence_pinyin: null,
    example_sentence_target: null,
    vocab_audio_path: null,
    sentence_audio_path: null,
    image_path: null,
  }
}

function adapter(
  options: {
    storage?: MemoryStorage
    clock?: Clock
    curriculum?: () => Promise<CurriculumItem[]>
  } = {},
) {
  const storage = options.storage ?? new MemoryStorage()
  return {
    storage,
    adapter: new WebAdapter({
      storage,
      clock: options.clock ?? new FixedClock(START, 3600),
      loadCurriculum: options.curriculum ?? (async () => [item('我', 0), item('你', 1)]),
    }),
  }
}

/** The rejection of `promise`, which the contract says is a string. */
async function rejection(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise
  } catch (err) {
    return err
  }
  throw new Error('expected a rejection')
}

describe('the web adapter', () => {
  it('says what a browser cannot do, and refuses it with a sentence', async () => {
    const { adapter: web } = adapter()
    expect(web.capabilities).toEqual({ kind: 'web', tts: false, anki: false })
    expect(await rejection(web.synthesizeAudio('你好', 'zh-CN'))).toBe(
      'Text-to-speech is not available in the web version.',
    )
    expect(await rejection(web.ankiExport())).toBe(
      'Anki export is not available in the web version.',
    )
    expect(await rejection(web.ankiImport('Chinese::HSK'))).toBe(
      'Anki import is not available in the web version.',
    )
  })

  it('plays stored recordings, has none to synthesize, and knows unknown cards', async () => {
    const { storage, adapter: web } = adapter()
    const [withAudio, without] = await storage.transaction(async (tx) => {
      const store = new Store(tx)
      const card = newCard(
        '学习',
        'lernen',
        'zh-CN',
        'de',
        parseInstantMillis('2026-10-01T00:00:00Z'),
      )
      const silent = await store.insertCapture(card, 'capture', false)
      card.audio_front = new Uint8Array([0xff, 0xfb, 0x90, 0x00])
      return [await store.insertCapture(card, 'capture', false), silent]
    })
    expect(await web.getCardAudio(withAudio)).toBe('//uQAA==')
    expect(await web.getCardAudio(without)).toBeNull()
    expect(await rejection(web.getCardAudio(99))).toBe('There is no card 99.')
  })

  it('has the desktop defaults until settings are saved', async () => {
    const { adapter: web } = adapter()
    expect(await web.getSettings()).toEqual({
      meaning_lang: 'en',
      anki: {
        connect_url: 'http://127.0.0.1:8765',
        deck_name: 'Chinese::HSK',
        model_name: 'Chinese (HSK)',
        field_mapping: {
          fields: [
            { anki_field: 'Hanzi', source: 'front' },
            { anki_field: 'Pinyin', source: 'pinyin' },
            { anki_field: 'Meaning', source: 'back' },
            { anki_field: 'Sentence', source: 'example' },
          ],
          audio_field: 'Audio',
        },
        create_reverse: false,
      },
    })
  })

  it('normalizes saved settings and uses their meaning language for new words', async () => {
    const { adapter: web } = adapter()
    const settings = defaultSettings()
    settings.meaning_lang = 'de'
    settings.anki.connect_url = ' http://localhost:8765 '
    settings.anki.deck_name = ' Deck '
    settings.anki.model_name = '\tModel\n'
    settings.anki.field_mapping = {
      fields: [
        { anki_field: ' Front ', source: 'front' },
        { anki_field: 'Back', source: 'back' },
      ],
      audio_field: '  ',
    }
    await web.saveSettings(settings)
    const saved = await web.getSettings()
    expect(saved.meaning_lang).toBe('de')
    expect(saved.anki.connect_url).toBe('http://localhost:8765')
    expect(saved.anki.deck_name).toBe('Deck')
    expect(saved.anki.model_name).toBe('Model')
    expect(saved.anki.field_mapping.fields[0].anki_field).toBe('Front')
    expect(saved.anki.field_mapping.audio_field).toBeNull()

    // German is asked for; the curriculum has English only, so the card says English.
    const cardId = await web.startNewItem(' 我 ')
    const capture = await web.getCard(cardId)
    expect(capture.card.back).toBe('我 (en)')
    expect(capture.card.target_lang).toBe('en')
  })

  it('refuses settings that export or import would trip over, naming every problem', async () => {
    const { storage, adapter: web } = adapter()
    const settings = defaultSettings()
    settings.anki.connect_url = 'ftp://127.0.0.1'
    settings.anki.deck_name = '   '
    settings.anki.model_name = ''
    settings.anki.field_mapping = {
      fields: [{ anki_field: '', source: 'pinyin' }],
      audio_field: null,
    }
    expect(await rejection(web.saveSettings(settings))).toBe(
      [
        'The AnkiConnect address "ftp://127.0.0.1" is not an http:// URL.',
        'The Anki deck name must not be empty.',
        'The Anki note type must not be empty.',
        'Every mapped Anki field needs a name.',
        'The field mapping needs an Anki field for the word.',
        'The field mapping needs an Anki field for the meaning.',
      ].join('\n'),
    )
    const stored = await storage.transaction((tx) => new Store(tx).getState(SETTINGS_KEY))
    expect(stored).toBeNull()
    const wrongShape = { meaning_lang: 'fr' } as unknown as LearnSettings
    expect(await rejection(web.saveSettings(wrongShape))).toBe(
      'invalid args `settings` for command `save_settings`: unknown meaning_lang "fr"',
    )
  })

  it('loads stored settings leniently: missing fields default, a broken value is ignored', async () => {
    const { storage, adapter: web } = adapter()
    const store = (value: string) =>
      storage.transaction((tx) => new Store(tx).setState(SETTINGS_KEY, value))
    await store(JSON.stringify({ meaning_lang: 'de', anki: { deck_name: 'Mine' } }))
    const partial = await web.getSettings()
    expect(partial.meaning_lang).toBe('de')
    expect(partial.anki.deck_name).toBe('Mine')
    expect(partial.anki.model_name).toBe('Chinese (HSK)')
    await store('{ broken')
    expect(await web.getSettings()).toEqual(defaultSettings())
    const kept = await storage.transaction((tx) => new Store(tx).getState(SETTINGS_KEY))
    expect(kept).toBe('{ broken')
  })

  it('normalizes stored settings on load, as the desktop app does', async () => {
    const { storage, adapter: web } = adapter()
    const stored = defaultSettings()
    stored.anki.deck_name = ' Mine\n'
    stored.anki.field_mapping.audio_field = ' '
    await storage.transaction((tx) => new Store(tx).setState(SETTINGS_KEY, JSON.stringify(stored)))
    const loaded = await web.getSettings()
    expect(loaded.anki.deck_name).toBe('Mine')
    expect(loaded.anki.field_mapping.audio_field).toBeNull()
  })

  it('has the default settings without a store, since the desktop command cannot fail', async () => {
    const web = await createWebAdapter({ storage: Promise.reject(new Error('no IndexedDB')) })
    expect(await web.getSettings()).toEqual(defaultSettings())
    expect(await rejection(web.saveSettings(defaultSettings()))).toBe(
      'The vocabulary database is unavailable (no IndexedDB).',
    )
  })

  it('refuses arguments the desktop command would not deserialize, in its words', async () => {
    const { adapter: web } = adapter()
    expect(await rejection(web.submitReviewRating(1, 7 as Rating))).toBe(
      'invalid args `rating` for command `submit_review_rating`: a review rating is 1 (Again) to 4 (Easy), not 7',
    )
    expect(await rejection(web.submitDailyQuota(-1))).toBe(
      'invalid args `newCount` for command `submit_daily_quota`: invalid value: integer `-1`, expected u32',
    )
    expect(await rejection(web.getCard(1.5))).toBe(
      'invalid args `cardId` for command `get_card`: invalid type: floating point `1.5`, expected i64',
    )
    expect(await rejection(web.startNewCapture(1.5))).toBe(
      'invalid args `cardId` for command `start_new_capture`: invalid type: floating point `1.5`, expected i64',
    )
    expect(await rejection(web.nextExercise(1, 'flashcard' as 'multiple_choice', 0))).toBe(
      'invalid args `kind` for command `next_exercise`: unknown variant `flashcard`, expected one of ' +
        '`multiple_choice`, `sentence_builder`, `listening_choice`, `listening_sentence`',
    )
    expect(
      await rejection(web.recordExerciseResult(1, 'multiple_choice', 'yes' as unknown as boolean)),
    ).toBe(
      'invalid args `correct` for command `record_exercise_result`: invalid type: string "yes", expected a boolean',
    )
    const shapeless = { kind: 'multiple_choice', card_id: 1 } as unknown as Exercise
    expect(await rejection(web.checkExercise(shapeless, { kind: 'choice', index: 0 }))).toBe(
      'invalid args `exercise` for command `check_exercise`: missing field `options`',
    )
    const choice: Exercise = {
      kind: 'multiple_choice',
      card_id: 1,
      direction: 'front_to_back',
      prompt: '我',
      prompt_lang: 'zh-CN',
      option_lang: 'en',
      options: [
        { card_id: 1, text: 'I' },
        { card_id: 2, text: 'you' },
      ],
      correct_index: 0,
    }
    expect(await web.checkExercise(choice, { kind: 'choice', index: 0 })).toBe(true)
    expect(await web.checkExercise(choice, { kind: 'order', tile_ids: [0] })).toBe(false)
    expect(
      await rejection(web.checkExercise(choice, { kind: 'tap' } as unknown as ExerciseAnswer)),
    ).toBe(
      'invalid args `answer` for command `check_exercise`: unknown variant `tap`, expected one of `choice`, `order`',
    )
    expect(await rejection(web.checkExercise(choice, { kind: 'choice', index: -1 }))).toBe(
      'invalid args `answer` for command `check_exercise`: invalid value: integer `-1`, expected usize',
    )
    expect(
      await rejection(
        web.checkExercise(choice, { kind: 'order', tile_ids: ['0'] as unknown as number[] }),
      ),
    ).toBe(
      'invalid args `answer` for command `check_exercise`: invalid type: string "0", expected u32',
    )
    const unsolvable = { ...choice, correct_index: undefined } as unknown as Exercise
    expect(await rejection(web.checkExercise(unsolvable, { kind: 'choice', index: 0 }))).toBe(
      'invalid args `exercise` for command `check_exercise`: missing field `correct_index`',
    )
    const order: Exercise = {
      kind: 'sentence_builder',
      card_id: 1,
      sentence_card_id: 1,
      prompt: 'I study.',
      sentence: '我学习。',
      sentence_lang: 'zh-CN',
      tiles: [
        { id: 0, text: '学习' },
        { id: 1, text: '我' },
      ],
      solution: [1, 0],
    }
    expect(await web.checkExercise(order, { kind: 'order', tile_ids: [1, 0] })).toBe(true)
    const tileless = { ...order, tiles: [{ id: 0.5, text: '学习' }] } as unknown as Exercise
    expect(await rejection(web.checkExercise(tileless, { kind: 'order', tile_ids: [0] }))).toBe(
      'invalid args `exercise` for command `check_exercise`: invalid type: floating point `0.5`, expected u32',
    )
  })

  it('checks the arguments before it needs the store, as Tauri deserializes them first', async () => {
    const web = await createWebAdapter({ storage: Promise.reject(new Error('no IndexedDB')) })
    expect(await rejection(web.submitDailyQuota(-1))).toBe(
      'invalid args `newCount` for command `submit_daily_quota`: invalid value: integer `-1`, expected u32',
    )
    expect(await rejection(web.submitDailyQuota(1))).toBe(
      'The vocabulary database is unavailable (no IndexedDB).',
    )
  })

  it('seeds the curriculum lazily, once, and retries a failed load', async () => {
    let loads = 0
    let fail = true
    const { storage, adapter: web } = adapter({
      curriculum: async () => {
        loads += 1
        if (fail) throw new Error('offline')
        return [item('我', 0), item('你', 1)]
      },
    })
    await web.getDailyPromptState()
    await web.submitDailyQuota(1)
    expect(loads).toBe(0)

    expect(await rejection(web.getDailyQueue())).toBe(
      'The HSK curriculum is unavailable (offline).',
    )
    expect(await web.getStats()).toMatchObject({ learned_count: 0 })
    fail = false
    expect(await web.getDailyQueue()).toEqual([{ kind: 'new', hanzi: '我', curriculum_order: 0 }])
    await web.startNewItem('我')
    expect(loads).toBe(2)
    const count = await storage.transaction((tx) => new Store(tx).curriculumCount())
    expect(count).toBe(2)
    expect(await rejection(web.startNewItem('没有'))).toBe('"没有" is not in the HSK curriculum.')
  })

  it('retries a curriculum loader that throws instead of rejecting', async () => {
    let loads = 0
    const { adapter: web } = adapter({
      curriculum: () => {
        loads += 1
        if (loads === 1) throw new Error('not ready')
        return Promise.resolve([item('我', 0)])
      },
    })
    await web.submitDailyQuota(1)
    expect(await rejection(web.getDailyQueue())).toBe(
      'The HSK curriculum is unavailable (not ready).',
    )
    expect(await web.getDailyQueue()).toEqual([{ kind: 'new', hanzi: '我', curriculum_order: 0 }])
    expect(loads).toBe(2)
  })

  it('resolves without a store, and every command says why it cannot work', async () => {
    const web = await createWebAdapter({ storage: Promise.reject(new Error('no IndexedDB')) })
    expect(await rejection(web.getDailyPromptState())).toBe(
      'The vocabulary database is unavailable (no IndexedDB).',
    )
    expect(await rejection(web.getDailyQueue())).toBe(
      'The vocabulary database is unavailable (no IndexedDB).',
    )
    const browserless = await createWebAdapter()
    expect(browserless.capabilities.kind).toBe('web')
    expect(await rejection(browserless.getStats())).toBe(
      'The vocabulary database is unavailable (this browser has no IndexedDB).',
    )
  })

  it('reads the clock once per command, so a review and its XP share one local date', async () => {
    let readings = 0
    // Every reading is a day later than the one before.
    const clock: Clock = {
      now: () => START + nanosFromMillis(86_400_000 * readings++),
      localOffset: () => 3600,
    }
    const { adapter: web } = adapter({ clock })
    await web.submitDailyQuota(1)
    expect(readings).toBe(1)
    const cardId = await web.startNewItem('我')
    const before = readings
    const result = await web.submitReviewRating(cardId, 3)
    expect(readings - before).toBe(1)
    expect(result.xp_awarded).toBe(15)
    expect(result.session?.new_items_completed).toBe(1)
    expect((await web.getStats()).xp_total).toBe(15)
  })
})
