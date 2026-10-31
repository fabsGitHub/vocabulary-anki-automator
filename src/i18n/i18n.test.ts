import { afterEach, describe, expect, it } from 'vitest'

import type { CaptureOrigin, PipelineStage, ProviderError } from '../api'
import { CARD_FIELDS, chineseMapping } from '../domain/fieldMapping'
import { de } from './de'
import { en } from './en'
import { UI_LANGS, i18n, setLocale, toUiLang } from './index'

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

afterEach(() => setLocale('en'))

describe('message files', () => {
  it('have exactly the same keys in English and German', () => {
    expect(KEYS.length).toBeGreaterThan(100)
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
        const params = Object.fromEntries(placeholders(message).map((name) => [name, `<${name}>`]))
        let output = ''
        expect(() => (output = i18n.global.t(key, params)), `${locale}:${key}`).not.toThrow()
        expect(output, `${locale}:${key}`).not.toBe(key)
        expect(output, `${locale}:${key}`).not.toMatch(/\{\s*\w+\s*\}/)
      }
    }
  })
})

describe('keys the UI asks for', () => {
  it('exist for every literal t(...) call in the sources', () => {
    const used = new Set<string>()
    for (const text of Object.values(SOURCES)) {
      for (const match of text.matchAll(/\bt\(\s*'([^']+)'/g)) used.add(match[1])
    }
    expect(used.size).toBeGreaterThan(100)
    expect([...used].filter((key) => !(key in MESSAGES.en))).toEqual([])
  })

  it('exist for every key built at runtime', () => {
    const stages: PipelineStage[] = [
      'translate',
      'rarity',
      'example',
      'example_translate',
      'tts',
      'annotate',
    ]
    const kinds: ProviderError['kind'][] = [
      'network',
      'quota_exceeded',
      'http',
      'malformed',
      'not_found',
      'unsupported',
    ]
    const origins: CaptureOrigin[] = ['capture', 'curriculum', 'anki_import']
    const providers = [
      'mymemory',
      'datamuse',
      'dictionary_api',
      'google_tts',
      'pinyin',
      'curriculum',
    ]
    const dynamic = [
      ...CARD_FIELDS.map((field) => `mapping.sources.${field}`),
      ...stages.map((stage) => `warnings.stage.${stage}`),
      ...kinds.map((kind) => `warnings.kind.${kind}`),
      ...origins.map((origin) => `history.origin.${origin}`),
      ...providers.map((provider) => `providers.${provider}`),
    ]
    expect(dynamic.filter((key) => !(key in MESSAGES.en))).toEqual([])
  })
})

describe('Anki field names', () => {
  it('name the Chinese preset fields verbatim in every language, since Anki matches them by name', () => {
    const preset = chineseMapping()
    const names = [
      ...preset.fields
        .filter((binding) => ['front', 'pinyin', 'back'].includes(binding.source))
        .map((binding) => binding.anki_field),
      preset.audio_field ?? '',
    ]
    expect(names).toEqual(['Hanzi', 'Pinyin', 'Meaning', 'Audio'])
    for (const locale of UI_LANGS) {
      for (const key of ['mapping.presetChinese', 'mapping.chineseWarning', 'help.step4Body']) {
        for (const name of names) {
          expect(MESSAGES[locale][key], `${locale}:${key}`).toContain(name)
        }
      }
    }
  })
})

describe('locale', () => {
  it('maps a stored ui_lang onto a UI language', () => {
    expect(toUiLang('de')).toBe('de')
    expect(toUiLang('DE')).toBe('de')
    expect(toUiLang('de-AT')).toBe('de')
    expect(toUiLang('en')).toBe('en')
    expect(toUiLang('fr')).toBe('en')
    expect(toUiLang(undefined)).toBe('en')
  })

  it('switches every message and the document language', () => {
    setLocale('de')
    expect(i18n.global.t('approval.approve')).toBe('Zu Anki hinzufügen')
    expect(document.documentElement.lang).toBe('de')
    setLocale('en')
    expect(i18n.global.t('approval.approve')).toBe('Add to Anki')
  })
})
