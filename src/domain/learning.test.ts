import { describe, expect, it } from 'vitest'

import type { AppConfig, LearnedLanguage } from '../api'
import { defaultConfig } from '../composables/useConfig'
import { SUPPORTED_LANGS } from './language'
import {
  DEFAULT_DECKS,
  availableLanguages,
  defaultReverse,
  learnedEntry,
  newLearnedLanguage,
  switchLanguage,
} from './learning'

function config(languages: Partial<LearnedLanguage>[] = [], native = 'de'): AppConfig {
  const value = defaultConfig()
  value.translation.target_lang = native
  value.learning.languages = languages.map((overrides) => ({
    lang: 'fr',
    deck_name: 'french',
    model_name: 'AppleStyleFinalV3',
    field_mapping: null,
    create_reverse: true,
    ...overrides,
  }))
  return value
}

describe('DEFAULT_DECKS', () => {
  it('names a deck for every supported language', () => {
    expect(Object.keys(DEFAULT_DECKS).sort()).toEqual([...SUPPORTED_LANGS].sort())
    expect(DEFAULT_DECKS).toEqual({
      en: 'english',
      de: 'german',
      fr: 'french',
      es: 'spanish',
      it: 'italian',
      'zh-CN': 'chinese',
    })
  })
})

describe('learnedEntry', () => {
  it('finds the entry of a language in any spelling', () => {
    const value = config([{ lang: 'fr' }, { lang: 'zh-CN', deck_name: 'chinese' }])
    expect(learnedEntry(value, 'zh')?.deck_name).toBe('chinese')
    expect(learnedEntry(value, 'FR')?.deck_name).toBe('french')
    expect(learnedEntry(value, 'en')).toBeNull()
  })
})

describe('newLearnedLanguage', () => {
  it('prefills the first free language with its deck and the global note type, matched by name', () => {
    const value = config()
    value.anki.model_name = 'Basic'
    value.anki.create_reverse = false
    expect(newLearnedLanguage(value)).toEqual({
      lang: 'en',
      deck_name: 'english',
      model_name: 'Basic',
      field_mapping: null,
      create_reverse: false,
    })
  })

  it('skips the native language and those already listed', () => {
    expect(newLearnedLanguage(config([{ lang: 'en' }]))?.lang).toBe('fr')
    expect(newLearnedLanguage(config([{ lang: 'de' }], 'en'))?.lang).toBe('fr')
  })

  it('starts Chinese without reverse cards', () => {
    const value = config(['en', 'fr', 'es', 'it'].map((lang) => ({ lang })))
    expect(value.anki.create_reverse).toBe(true)
    const chinese = newLearnedLanguage(value)
    expect(chinese?.lang).toBe('zh-CN')
    expect(chinese?.deck_name).toBe('chinese')
    expect(chinese?.create_reverse).toBe(false)
  })

  it('is null once every language but the native one is listed', () => {
    expect(
      newLearnedLanguage(config(['en', 'fr', 'es', 'it', 'zh-CN'].map((lang) => ({ lang })))),
    ).toBeNull()
  })
})

describe('defaultReverse', () => {
  it('is off for Chinese and the global setting otherwise', () => {
    const value = config()
    for (const global of [true, false]) {
      value.anki.create_reverse = global
      expect(defaultReverse(value, 'fr')).toBe(global)
      expect(defaultReverse(value, 'zh')).toBe(false)
    }
  })
})

describe('switchLanguage', () => {
  it('brings the new language’s deck and reverse default along while they are the old defaults', () => {
    const value = config([{ lang: 'en', deck_name: 'english', create_reverse: true }])
    const entry = value.learning.languages[0]

    switchLanguage(value, entry, 'zh-CN')
    expect(entry).toMatchObject({ lang: 'zh-CN', deck_name: 'chinese', create_reverse: false })

    switchLanguage(value, entry, 'fr')
    expect(entry).toMatchObject({ lang: 'fr', deck_name: 'french', create_reverse: true })
  })

  it('keeps a deck, reverse setting, note type and mapping the user chose', () => {
    const value = config([
      {
        lang: 'en',
        deck_name: 'Vocabulary',
        model_name: 'Basic',
        field_mapping: { fields: [{ anki_field: 'Front', source: 'front' }], audio_field: null },
        create_reverse: false,
      },
    ])
    const entry = value.learning.languages[0]
    const before = structuredClone(entry)

    switchLanguage(value, entry, 'zh-CN')
    expect(entry).toEqual({ ...before, lang: 'zh-CN' })

    // Off for Chinese is its default, so on is the user's choice.
    entry.create_reverse = true
    switchLanguage(value, entry, 'es')
    expect(entry).toEqual({ ...before, lang: 'es', create_reverse: true })
  })

  it('leaves the deck alone for a language it has no default deck for', () => {
    const value = config([{ lang: 'pt', deck_name: 'french' }])
    const entry = value.learning.languages[0]
    switchLanguage(value, entry, 'es')
    expect(entry).toMatchObject({ lang: 'es', deck_name: 'french' })
  })
})

describe('availableLanguages', () => {
  it('offers the supported languages that are neither native nor taken by another entry', () => {
    const value = config([{ lang: 'fr' }, { lang: 'zh-CN' }])
    expect(availableLanguages(value, 0)).toEqual(['en', 'fr', 'es', 'it'])
    expect(availableLanguages(value, 1)).toEqual(['en', 'es', 'it', 'zh-CN'])
  })

  it('keeps the current value visible even when it is no longer allowed', () => {
    const value = config([{ lang: 'de' }])
    expect(availableLanguages(value, 0)).toEqual(['en', 'de', 'fr', 'es', 'it', 'zh-CN'])
  })
})
