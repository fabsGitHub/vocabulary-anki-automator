import { describe, expect, it } from 'vitest'

import mapperSource from '../../crates/anki-bridge/src/mapper.rs?raw'
import type { AppConfig, CardField, FieldMapping } from '../api'
import { defaultConfig } from '../composables/useConfig'
import {
  appleStyleMapping,
  bindingFor,
  bindsSource,
  chineseMapping,
  effectiveBindings,
  missingFields,
  needsChineseNoteType,
  withAudioField,
  withBinding,
} from './fieldMapping'

/** Reads a preset straight out of `anki_bridge::FieldMapping`'s Rust source. */
function rustPreset(name: string): FieldMapping {
  const body = mapperSource.split(`pub fn ${name}() -> Self {`)[1]?.split('\n    }\n')[0]
  if (!body) throw new Error(`FieldMapping::${name} not found in mapper.rs`)
  const toSnake = (variant: string) =>
    variant.replace(/[a-z][A-Z]/g, (m) => `${m[0]}_${m[1]}`).toLowerCase()
  const fields = [...body.matchAll(/FieldBinding::new\("([^"]+)", CardField::(\w+)\)/g)].map(
    ([, anki_field, variant]) => ({ anki_field, source: toSnake(variant) as CardField }),
  )
  const audio = /audio_field: Some\("([^"]+)"\.to_string\(\)\)/.exec(body)
  return { fields, audio_field: audio ? audio[1] : null }
}

const names = (...list: string[]) => list

describe('presets', () => {
  it('equal FieldMapping::apple_style_default exactly', () => {
    expect(rustPreset('apple_style_default').fields).toHaveLength(4)
    expect(appleStyleMapping()).toEqual(rustPreset('apple_style_default'))
  })

  it('equal FieldMapping::chinese_default exactly', () => {
    expect(rustPreset('chinese_default').fields).toHaveLength(4)
    expect(chineseMapping()).toEqual(rustPreset('chinese_default'))
  })

  it('are fresh objects, so editing one never changes the next', () => {
    const first = chineseMapping()
    first.fields[0].anki_field = 'Edited'
    expect(chineseMapping().fields[0].anki_field).toBe('Hanzi')
  })
})

describe('first binding wins (case-insensitive field names)', () => {
  const shadowed: FieldMapping = {
    fields: [
      { anki_field: 'Word', source: 'front' },
      { anki_field: 'WORD', source: 'pinyin' },
    ],
    audio_field: null,
  }

  it('ignores a later binding for the same field', () => {
    expect(effectiveBindings(shadowed)).toEqual([{ anki_field: 'Word', source: 'front' }])
    expect(bindsSource(shadowed, 'pinyin')).toBe(false)
    expect(bindingFor(shadowed, 'word')?.source).toBe('front')
  })
})

describe('withBinding', () => {
  it('replaces a binding in place and drops its shadowed duplicates', () => {
    const mapping: FieldMapping = {
      fields: [
        { anki_field: 'A', source: 'front' },
        { anki_field: 'B', source: 'back' },
        { anki_field: 'b', source: 'notes' },
        { anki_field: 'C', source: 'example' },
      ],
      audio_field: 'Audio',
    }
    expect(withBinding(mapping, 'B', 'pinyin').fields).toEqual([
      { anki_field: 'A', source: 'front' },
      { anki_field: 'B', source: 'pinyin' },
      { anki_field: 'C', source: 'example' },
    ])
    expect(mapping.fields).toHaveLength(4)
  })

  it('appends a new field and removes one set to nothing', () => {
    const mapping = appleStyleMapping()
    const appended = withBinding(mapping, 'Pinyin', 'pinyin').fields
    expect(appended).toHaveLength(5)
    expect(appended[4]).toEqual({ anki_field: 'Pinyin', source: 'pinyin' })
    expect(withBinding(mapping, 'notes', null).fields.map((b) => b.anki_field)).toEqual(
      names('Front', 'Back', 'Context Sentence'),
    )
    expect(withBinding(mapping, 'notes', null).audio_field).toBe('Front Audio')
  })
})

describe('withAudioField', () => {
  it('stores a blank audio field as null', () => {
    expect(withAudioField(appleStyleMapping(), '  ').audio_field).toBeNull()
    expect(withAudioField(appleStyleMapping(), 'Audio').audio_field).toBe('Audio')
  })
})

describe('missingFields (mirrors FieldMapping::missing_fields)', () => {
  it('lists unmapped note type fields, in mapping order', () => {
    const chinese = chineseMapping()
    expect(
      missingFields(chinese, names('hanzi', 'Pinyin', 'Meaning', 'Sentence', 'Audio')),
    ).toEqual([])
    expect(missingFields(chinese, names('Front', 'Back', 'Pinyin'))).toEqual(
      names('Hanzi', 'Meaning', 'Sentence', 'Audio'),
    )
    const noAudio = { ...appleStyleMapping(), audio_field: null }
    expect(missingFields(noAudio, names('Front', 'Back', 'Notes'))).toEqual(
      names('Context Sentence'),
    )
  })

  it('reports a shadowed binding only once', () => {
    const shadowed: FieldMapping = {
      fields: [
        { anki_field: 'Word', source: 'front' },
        { anki_field: 'WORD', source: 'pinyin' },
      ],
      audio_field: null,
    }
    expect(missingFields(shadowed, names('Back'))).toEqual(names('Word'))
  })

  it('matches the audio field exactly, as AnkiConnect attaches media', () => {
    expect(
      missingFields(
        appleStyleMapping(),
        names('front', 'back', 'context sentence', 'notes', 'front audio'),
      ),
    ).toEqual(names('Front Audio'))
  })
})

describe('needsChineseNoteType (J5)', () => {
  function config(edit: (c: AppConfig) => void): AppConfig {
    const c = defaultConfig()
    edit(c)
    return c
  }

  it('is off for non-Chinese setups', () => {
    expect(needsChineseNoteType(defaultConfig())).toBe(false)
  })

  it('warns when the default source is Chinese and nothing receives the pinyin', () => {
    expect(needsChineseNoteType(config((c) => (c.translation.source_lang = 'zh-CN')))).toBe(true)
  })

  it('warns when only a profile captures Chinese', () => {
    const withProfile = config((c) =>
      c.hotkey_profiles.push({
        id: 'zh-de',
        label: 'Chinese',
        shortcut: 'CommandOrControl+Shift+H',
        source_lang: 'zh-CN',
        target_lang: 'de',
        deck_name: null,
      }),
    )
    expect(needsChineseNoteType(withProfile)).toBe(true)
  })

  it('is satisfied by the Chinese preset or any pinyin binding', () => {
    const chinese = config((c) => {
      c.translation.source_lang = 'zh-CN'
      c.anki.field_mapping = chineseMapping()
    })
    expect(needsChineseNoteType(chinese)).toBe(false)
    const custom = config((c) => {
      c.translation.source_lang = 'zh-CN'
      c.anki.field_mapping = withBinding(appleStyleMapping(), 'Notes', 'pinyin')
    })
    expect(needsChineseNoteType(custom)).toBe(false)
  })

  it('does not count a pinyin binding that an earlier binding shadows', () => {
    const shadowed = config((c) => {
      c.translation.source_lang = 'zh-CN'
      c.anki.field_mapping = {
        fields: [
          { anki_field: 'Front', source: 'front' },
          { anki_field: 'front', source: 'pinyin' },
        ],
        audio_field: null,
      }
    })
    expect(needsChineseNoteType(shadowed)).toBe(true)
  })
})
