import { describe, expect, it } from 'vitest'

import type { AppConfig, LearnedLanguage } from '../api'
import { defaultConfig } from '../composables/useConfig'
import { SUPPORTED_LANGS } from './language'
import { DEFAULT_DECKS, availableLanguages, learnedEntry, newLearnedLanguage } from './learning'

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
