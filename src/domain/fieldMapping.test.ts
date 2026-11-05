import { describe, expect, it } from 'vitest'

import mapperSource from '../../crates/anki-bridge/src/mapper.rs?raw'
import type { AppConfig, AutoMatch, CardField, FieldMapping } from '../api'
import { defaultConfig } from '../composables/useConfig'
import {
  CARD_FIELDS,
  MAPPING_PRESETS,
  RECOMMENDED_NOTE_TYPE,
  appleStyleMapping,
  bindingFor,
  bindsSource,
  chineseMapping,
  copyMapping,
  effectiveBindings,
  lacksPinyin,
  missingFields,
  needsChineseNoteType,
  recommendedMapping,
  recommendedNoteTypeAction,
  sameMapping,
  selectRecommendedNoteType,
  withAudioField,
  withBinding,
} from './fieldMapping'

/** `ExampleInSourceLang` -> `example_in_source_lang`, as serde spells a `CardField`. */
const toSnake = (variant: string) =>
  variant.replace(/[a-z][A-Z]/g, (m) => `${m[0]}_${m[1]}`).toLowerCase()

/** Reads a preset straight out of `anki_bridge::FieldMapping`'s Rust source. */
function rustPreset(name: string): FieldMapping {
  const body = mapperSource.split(`pub fn ${name}() -> Self {`)[1]?.split('\n    }\n')[0]
  if (!body) throw new Error(`FieldMapping::${name} not found in mapper.rs`)
  const fields = [...body.matchAll(/FieldBinding::new\("([^"]+)", CardField::(\w+)\)/g)].map(
    ([, anki_field, variant]) => ({ anki_field, source: toSnake(variant) as CardField }),
  )
  const audio = /audio_field: Some\("([^"]+)"\.to_string\(\)\)/.exec(body)
  return { fields, audio_field: audio ? audio[1] : null }
}

const names = (...list: string[]) => list

describe('CARD_FIELDS', () => {
  it('offers every anki_bridge::CardField variant exactly once', () => {
    const body = mapperSource.split('pub enum CardField {')[1]?.split('\n}\n')[0]
    if (!body) throw new Error('CardField not found in mapper.rs')
    const variants = [...body.matchAll(/^ {4}([A-Z]\w*),$/gm)].map(([, variant]) =>
      toSnake(variant),
    )
    expect(CARD_FIELDS).toHaveLength(10)
    expect([...CARD_FIELDS].sort()).toEqual(variants.sort())
  })
})

describe('presets', () => {
  it('equal FieldMapping::apple_style_default exactly', () => {
    expect(rustPreset('apple_style_default').fields).toHaveLength(4)
    expect(appleStyleMapping()).toEqual(rustPreset('apple_style_default'))
  })

  it('equal FieldMapping::chinese_default exactly', () => {
    expect(rustPreset('chinese_default').fields).toHaveLength(4)
    expect(chineseMapping()).toEqual(rustPreset('chinese_default'))
  })

  it('equal FieldMapping::recommended exactly', () => {
    expect(rustPreset('recommended').fields).toHaveLength(7)
    expect(recommendedMapping()).toEqual(rustPreset('recommended'))
  })

  it('name the recommended note type as anki_bridge::RECOMMENDED_NOTE_TYPE does', () => {
    const rust = /pub const RECOMMENDED_NOTE_TYPE: &str = "([^"]+)";/.exec(mapperSource)
    expect(rust?.[1]).toBe('Polywort')
    expect(RECOMMENDED_NOTE_TYPE).toBe(rust?.[1])
  })

  it('offer the recommended mapping first', () => {
    expect(Object.keys(MAPPING_PRESETS)).toEqual(['recommended', 'apple', 'chinese'])
    expect(MAPPING_PRESETS.recommended()).toEqual(recommendedMapping())
  })

  it('are fresh objects, so editing one never changes the next', () => {
    const first = chineseMapping()
    first.fields[0].anki_field = 'Edited'
    expect(chineseMapping().fields[0].anki_field).toBe('Hanzi')
  })
})

describe('recommendedNoteTypeAction', () => {
  it('says "Use" once Anki lists the note type, "Create" before', () => {
    expect(recommendedNoteTypeAction(['Basic'])).toBe('noteType.create')
    expect(recommendedNoteTypeAction(['Basic', RECOMMENDED_NOTE_TYPE])).toBe('noteType.use')
  })
})

describe('selectRecommendedNoteType', () => {
  const custom: FieldMapping = { fields: [{ anki_field: 'Q', source: 'front' }], audio_field: null }

  it('leaves the mapping to the match by name when that gives the recommended one', () => {
    const place = { model_name: 'AppleStyleFinalV3', field_mapping: custom }
    selectRecommendedNoteType(place, {
      model_name: RECOMMENDED_NOTE_TYPE,
      mapping: recommendedMapping(),
      created: true,
      matches_by_name: true,
    })
    expect(place).toEqual({ model_name: 'Polywort', field_mapping: null })
  })

  it('saves a copy of the mapping when the match would differ', () => {
    const mapping = recommendedMapping()
    const place = { model_name: 'Basic', field_mapping: null as FieldMapping | null }
    selectRecommendedNoteType(place, {
      model_name: RECOMMENDED_NOTE_TYPE,
      mapping,
      created: false,
      matches_by_name: false,
    })
    expect(place).toEqual({ model_name: 'Polywort', field_mapping: mapping })
    expect(place.field_mapping).not.toBe(mapping)
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

describe('sameMapping and copyMapping', () => {
  it('compare bindings in order, and the audio field', () => {
    expect(sameMapping(appleStyleMapping(), appleStyleMapping())).toBe(true)
    const reordered = appleStyleMapping()
    reordered.fields.reverse()
    expect(sameMapping(reordered, appleStyleMapping())).toBe(false)
    expect(sameMapping(withAudioField(appleStyleMapping(), null), appleStyleMapping())).toBe(false)
    expect(sameMapping(withBinding(appleStyleMapping(), 'Notes', null), appleStyleMapping())).toBe(
      false,
    )
  })

  it('copy without sharing a binding', () => {
    const original = chineseMapping()
    const copy = copyMapping(original)
    expect(copy).toEqual(original)
    copy.fields[0].anki_field = 'Edited'
    expect(original.fields[0].anki_field).toBe('Hanzi')
  })
})

describe('lacksPinyin', () => {
  it('checks the saved mapping, else the preview, and knows nothing without either', () => {
    const preview = { mapping: appleStyleMapping(), guessed: [], unmatched: [] }
    expect(lacksPinyin(appleStyleMapping(), null)).toBe(true)
    expect(lacksPinyin(chineseMapping(), preview)).toBe(false)
    expect(lacksPinyin(null, preview)).toBe(true)
    expect(lacksPinyin(null, { ...preview, mapping: chineseMapping() })).toBe(false)
    expect(lacksPinyin(null, null)).toBe(false)
  })
})

describe('needsChineseNoteType (J5)', () => {
  /** What matching AppleStyleFinalV3 by name gives: no field takes the pinyin. */
  const APPLE_V3_MATCH: AutoMatch = {
    mapping: {
      fields: [
        { anki_field: 'Front', source: 'front' },
        { anki_field: 'Back', source: 'back' },
      ],
      audio_field: 'Audio',
    },
    guessed: [],
    unmatched: [],
  }
  const CHINESE_MATCH: AutoMatch = { mapping: chineseMapping(), guessed: [], unmatched: [] }

  function config(edit: (c: AppConfig) => void): AppConfig {
    const c = defaultConfig()
    c.anki.field_mapping = appleStyleMapping()
    edit(c)
    return c
  }

  const chineseSource = (c: AppConfig) => (c.translation.source_lang = 'zh-CN')

  it('is off for non-Chinese setups', () => {
    expect(needsChineseNoteType(defaultConfig())).toBe(false)
    expect(needsChineseNoteType(defaultConfig(), APPLE_V3_MATCH)).toBe(false)
  })

  it('warns when the default source is Chinese and nothing receives the pinyin', () => {
    expect(needsChineseNoteType(config(chineseSource))).toBe(true)
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

  it('checks a mapping matched by name through its preview, and stays silent without one', () => {
    const matched = config((c) => {
      chineseSource(c)
      c.anki.field_mapping = null
    })
    expect(needsChineseNoteType(matched)).toBe(false)
    expect(needsChineseNoteType(matched, APPLE_V3_MATCH)).toBe(true)
    expect(needsChineseNoteType(matched, CHINESE_MATCH)).toBe(false)
    // A saved mapping wins over any preview.
    expect(needsChineseNoteType(config(chineseSource), CHINESE_MATCH)).toBe(true)
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

  it('leaves Chinese to a Chinese entry under "My languages", which files every Chinese capture', () => {
    const learned = config((c) => {
      c.translation.source_lang = 'zh-CN'
      c.learning.languages.push({
        lang: 'zh-CN',
        deck_name: 'chinese',
        model_name: 'Chinese',
        field_mapping: null,
        create_reverse: false,
      })
    })
    expect(needsChineseNoteType(learned)).toBe(false)
    learned.learning.languages[0].lang = 'fr'
    expect(needsChineseNoteType(learned)).toBe(true)
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
