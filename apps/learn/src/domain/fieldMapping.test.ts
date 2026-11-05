import { describe, expect, it } from 'vitest'

import type { LearnSettings } from '../platform/types'
import * as web from '../platform/web/settings'
import { settings } from '../test/fixtures'
import {
  defaultMapping,
  isAnkiConnectUrl,
  mappingProblems,
  normalizeSettings,
} from './fieldMapping'

// The settings form judges and normalizes the settings before the backend
// does, so it has to agree with it. The web port of vocab-learn's settings
// module is held to the Rust one (apps/learn/src-tauri/src/settings.rs), so
// the form is compared with that port here.

const problemSentences: Record<string, string> = {
  'mapping.nameMissing': 'Every mapped Anki field needs a name.',
  'mapping.wordMissing': 'The field mapping needs an Anki field for the word.',
  'mapping.meaningMissing': 'The field mapping needs an Anki field for the meaning.',
}

function withMapping(edit: (value: LearnSettings) => void): LearnSettings {
  return settings(edit)
}

describe('the settings form agrees with the backend', () => {
  it('uses the default mapping as the default fields', () => {
    expect(defaultMapping()).toEqual(web.defaultSettings().anki.field_mapping)
  })

  it('normalizes the way the backend stores settings', () => {
    const value = withMapping((s) => {
      s.anki.connect_url = ' http://localhost:8766 '
      s.anki.deck_name = ' Deck '
      s.anki.model_name = '\tModel\n'
      s.anki.field_mapping = {
        fields: [{ anki_field: ' Front ', source: 'front' }],
        audio_field: '  ',
      }
    })
    expect(normalizeSettings(value)).toEqual(web.normalizeSettings(value))
    expect(JSON.stringify(normalizeSettings(value))).toBe(
      JSON.stringify(web.normalizeSettings(value)),
    )
  })

  it('refuses the mappings the backend refuses', () => {
    const cases: LearnSettings[] = [
      settings(),
      withMapping((s) => (s.anki.field_mapping.fields[1].anki_field = ' ')),
      withMapping((s) => (s.anki.field_mapping.fields = [])),
      withMapping((s) => (s.anki.field_mapping.fields[0].source = 'notes')),
      withMapping((s) => (s.anki.field_mapping.fields[2].source = 'example')),
    ]
    for (const value of cases) {
      const normalized = normalizeSettings(value)
      const expected = web
        .validateSettings(normalized)
        .filter((sentence) => Object.values(problemSentences).includes(sentence))
      expect(
        mappingProblems(normalized.anki.field_mapping).map((key) => problemSentences[key]),
      ).toEqual(expected)
    }
  })

  it('accepts the AnkiConnect addresses the backend accepts', () => {
    for (const url of [
      'http://127.0.0.1:8765',
      'https://anki.local',
      'ftp://127.0.0.1',
      'localhost:8765',
      '',
      'http://',
    ]) {
      const value = normalizeSettings(withMapping((s) => (s.anki.connect_url = url)))
      const backend = !web
        .validateSettings(value)
        .some((sentence) => sentence.includes('AnkiConnect address'))
      expect(isAnkiConnectUrl(url), url).toBe(backend)
    }
  })
})
