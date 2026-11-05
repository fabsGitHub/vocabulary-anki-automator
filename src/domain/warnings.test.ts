import { beforeEach, describe, expect, it } from 'vitest'

import type { PipelineWarning } from '../api'
import { i18n, setLocale } from '../i18n'
import { formatWarning } from './warnings'

const t = (key: string, params?: Record<string, unknown>) => i18n.global.t(key, params ?? {})
const has = (key: string) => i18n.global.te(key)

const format = (warning: PipelineWarning) => formatWarning(warning, t, has)

describe('formatWarning', () => {
  beforeEach(() => setLocale('en'))

  it("explains MyMemory's daily limit and the contact e-mail setting", () => {
    const shown = format({
      stage: 'translate',
      error: {
        kind: 'quota_exceeded',
        provider: 'mymemory',
        message: 'MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY',
      },
    })
    expect(shown.text).toMatch(/^Translation: /)
    expect(shown.text).toContain('daily translation limit')
    expect(shown.text).toContain('contact e-mail')
    expect(shown.text).toContain('Capture & Language')
    expect(shown.detail).toBe(
      'MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY',
    )
  })

  it('keeps the generic quota sentence for other providers', () => {
    const shown = format({
      stage: 'tts',
      error: { kind: 'quota_exceeded', provider: 'google_tts', message: 'HTTP 429' },
    })
    expect(shown.text).toBe(
      'Pronunciation: Google Text-to-Speech refuses further requests for now (quota reached).',
    )
  })

  it('gives every error kind its own sentence', () => {
    const texts = [
      format({
        stage: 'translate',
        error: { kind: 'network', provider: 'mymemory', message: 'timeout' },
      }),
      format({
        stage: 'example',
        error: { kind: 'http', provider: 'dictionary_api', status: 503, message: 'x' },
      }),
      format({
        stage: 'rarity',
        error: { kind: 'malformed', provider: 'datamuse', message: 'bad json' },
      }),
      format({ stage: 'annotate', error: { kind: 'not_found', provider: 'curriculum' } }),
      format({
        stage: 'annotate',
        error: { kind: 'unsupported', provider: 'pinyin', what: 'Latin text' },
      }),
    ].map((shown) => shown.text)
    expect(texts).toEqual([
      'Translation: MyMemory could not be reached. Check your internet connection.',
      'Example sentence: dictionaryapi.dev answered with an error (HTTP 503).',
      'Rarity check: Datamuse sent a response the app could not read.',
      'Pinyin and HSK level: The HSK curriculum has no result for this text.',
      'Pinyin and HSK level: The pinyin generator does not support Latin text.',
    ])
  })

  it('shows unknown providers by their raw name and has no detail without a message', () => {
    const shown = format({
      stage: 'example_translate',
      error: { kind: 'not_found', provider: 'argos' },
    })
    expect(shown.text).toBe('Example translation: argos has no result for this text.')
    expect(shown.detail).toBeNull()
  })

  it('follows the UI language', () => {
    setLocale('de')
    const shown = format({
      stage: 'translate',
      error: { kind: 'quota_exceeded', provider: 'mymemory', message: '' },
    })
    expect(shown.text).toMatch(/^Übersetzung: /)
    expect(shown.text).toContain('Kontakt-E-Mail')
    expect(shown.detail).toBeNull()
  })
})
