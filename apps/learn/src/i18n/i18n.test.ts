import { afterEach, describe, expect, it, vi } from 'vitest'

import { DIRECTION_KEYS, KIND_KEYS } from '../domain/exercises'
import { INTERVAL_KEYS, RATING_KEYS } from '../domain/intervals'
import { LANGUAGE_NAME_KEYS } from '../domain/languages'
import { de } from './de'
import { en } from './en'
import {
  UI_LANGS,
  UI_LANG_STORAGE_KEY,
  chooseUiLang,
  i18n,
  initialUiLang,
  setLocale,
  toUiLang,
} from './index'

type Tree = { [key: string]: string | Tree }

function flatten(tree: Tree, prefix = ''): Record<string, string> {
  const flat: Record<string, string> = {}
  for (const [key, value] of Object.entries(tree)) {
    if (typeof value === 'string') flat[`${prefix}${key}`] = value
    else Object.assign(flat, flatten(value, `${prefix}${key}.`))
  }
  return flat
}

const MESSAGES: Record<string, Record<string, string>> = { en: flatten(en), de: flatten(de) }
const KEYS = Object.keys(MESSAGES.en)

const placeholders = (message: string) =>
  [...message.matchAll(/\{\s*([A-Za-z_]\w*)\s*\}/g)].map((match) => match[1]).sort()

/** Every `.vue`/`.ts` source of the UI, as text. */
const SOURCES = import.meta.glob(['../**/*.vue', '../**/*.ts', '!../**/*.test.ts', '!../test/**'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

afterEach(() => {
  window.localStorage.clear()
  setLocale('en')
})

describe('message files', () => {
  it('have exactly the same keys in English and German', () => {
    expect(KEYS.length).toBeGreaterThan(150)
    expect(Object.keys(MESSAGES.de).sort()).toEqual([...KEYS].sort())
  })

  it('use the same placeholders in both languages', () => {
    const mismatched = KEYS.filter(
      (key) => placeholders(MESSAGES.en[key]).join() !== placeholders(MESSAGES.de[key]).join(),
    )
    expect(mismatched).toEqual([])
  })

  it('compile and interpolate every message in every language', () => {
    for (const locale of UI_LANGS) {
      setLocale(locale)
      for (const key of KEYS) {
        const message = MESSAGES[locale][key]
        expect(message.trim(), `${locale}:${key}`).not.toBe('')
        expect(message, `${locale}:${key} uses vue-i18n's plural separator`).not.toContain('|')
        expect(message, `${locale}:${key} starts a linked message`).not.toContain('@')
        const params = Object.fromEntries(placeholders(message).map((name) => [name, `<${name}>`]))
        let output = ''
        expect(() => (output = i18n.global.t(key, params)), `${locale}:${key}`).not.toThrow()
        expect(output, `${locale}:${key}`).not.toBe(key)
        expect(output, `${locale}:${key}`).not.toMatch(/\{\s*\w+\s*\}/)
      }
    }
  })

  it('are really translated, not copied', () => {
    // Words that read the same in both languages are allowed to match.
    const same = new Set([
      'app.title',
      'review.hsk',
      'review.hskBand',
      'exercise.xp',
      'settings.ankiTitle',
      'anki.failure',
    ])
    const copied = KEYS.filter((key) => !same.has(key) && MESSAGES.en[key] === MESSAGES.de[key])
    expect(copied).toEqual([])
  })
})

describe('keys the UI asks for', () => {
  it('exist for every literal t(...) call and <i18n-t keypath> in the sources', () => {
    const used = new Set<string>()
    for (const text of Object.values(SOURCES)) {
      for (const match of text.matchAll(/\bt\(\s*'([^']+)'/g)) used.add(match[1])
      for (const match of text.matchAll(/\bkeypath="([^"]+)"/g)) used.add(match[1])
    }
    expect(used.size).toBeGreaterThan(120)
    expect([...used].filter((key) => !(key in MESSAGES.en))).toEqual([])
  })

  it('exist for every key looked up at runtime', () => {
    const dynamic = [
      ...Object.values(KIND_KEYS),
      ...Object.values(DIRECTION_KEYS),
      ...Object.values(INTERVAL_KEYS),
      ...Object.values(RATING_KEYS),
      ...Object.values(LANGUAGE_NAME_KEYS),
      'tabs.review',
      'tabs.practice',
      'tabs.lessons',
      'tabs.stats',
      'tabs.anki',
    ]
    expect(dynamic.filter((key) => !(key in MESSAGES.en))).toEqual([])
  })
})

describe('locale', () => {
  it('maps a language tag onto a UI language', () => {
    expect(toUiLang('de')).toBe('de')
    expect(toUiLang('DE')).toBe('de')
    expect(toUiLang('de-AT')).toBe('de')
    expect(toUiLang('en-GB')).toBe('en')
    expect(toUiLang('fr')).toBe('en')
    expect(toUiLang(undefined)).toBe('en')
  })

  it('switches every message, the document language and the title', () => {
    setLocale('de')
    expect(i18n.global.t('review.showAnswer')).toBe('Antwort zeigen')
    expect(document.documentElement.lang).toBe('de')
    setLocale('en')
    expect(i18n.global.t('review.showAnswer')).toBe('Show answer')
    expect(document.title).toBe('Vocabulary Learn')
  })

  it('starts in the remembered language, else the system one', () => {
    const language = vi.spyOn(navigator, 'language', 'get').mockReturnValue('de-DE')
    expect(initialUiLang()).toBe('de')
    chooseUiLang('en')
    expect(window.localStorage.getItem(UI_LANG_STORAGE_KEY)).toBe('en')
    expect(initialUiLang()).toBe('en')
    language.mockReturnValue('en-US')
    window.localStorage.clear()
    expect(initialUiLang()).toBe('en')
  })

  it('still starts when storage is unavailable', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError')
    })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new DOMException('denied', 'SecurityError')
    })
    vi.spyOn(navigator, 'language', 'get').mockReturnValue('de')
    expect(initialUiLang()).toBe('de')
    expect(chooseUiLang('de')).toBe('de')
    expect(i18n.global.locale.value).toBe('de')
  })
})
