/**
 * The Anki part of Learn's settings form: the field mapping
 * (`LearnSettings.anki.field_mapping`, `anki_bridge::FieldMapping`) and the
 * AnkiConnect address.
 *
 * Export writes a card into the fields the mapping names and import reads
 * them back (docs/architecture.md § 9, § 10), so the names have to be the
 * fields of the learner's own note type; nothing else works for a deck that
 * does not use the defaults. The backend applies and validates the settings
 * (`LearnSettings::normalize` and `validate` in
 * apps/learn/src-tauri/src/settings.rs); this module only lets the form say
 * up front what a save would refuse, and save what the backend would store.
 */

import type { MessageKey } from '../i18n'
import type { CardField, FieldMapping, LearnSettings } from '../platform/types'

/** Every card value, in the order the form lists them (`anki_bridge::CardField`). */
export const CARD_FIELDS: readonly CardField[] = [
  'front',
  'back',
  'example_source',
  'example_target',
  'example',
  'notes',
  'pinyin',
  'hsk_level',
]

/** The label of each card value. */
export const CARD_FIELD_KEYS: Record<CardField, MessageKey> = {
  front: 'mapping.sources.front',
  back: 'mapping.sources.back',
  example_source: 'mapping.sources.example_source',
  example_target: 'mapping.sources.example_target',
  example: 'mapping.sources.example',
  notes: 'mapping.sources.notes',
  pinyin: 'mapping.sources.pinyin',
  hsk_level: 'mapping.sources.hsk_level',
}

/** `FieldMapping::chinese_default()`, Learn's default: a Hanzi/Pinyin/Meaning/Sentence/Audio note type. */
export function defaultMapping(): FieldMapping {
  return {
    fields: [
      { anki_field: 'Hanzi', source: 'front' },
      { anki_field: 'Pinyin', source: 'pinyin' },
      { anki_field: 'Meaning', source: 'back' },
      { anki_field: 'Sentence', source: 'example' },
    ],
    audio_field: 'Audio',
  }
}

/** What `LearnSettings::validate` refuses about a normalized mapping, as messages. */
export function mappingProblems(mapping: FieldMapping): MessageKey[] {
  const problems: MessageKey[] = []
  if (mapping.fields.some((binding) => binding.anki_field.trim() === '')) {
    problems.push('mapping.nameMissing')
  }
  if (!mapping.fields.some((binding) => binding.source === 'front')) {
    problems.push('mapping.wordMissing')
  }
  if (!mapping.fields.some((binding) => binding.source === 'back')) {
    problems.push('mapping.meaningMissing')
  }
  return problems
}

/** Whether `url` is what `LearnSettings::validate` accepts: http(s), with a host. */
export function isAnkiConnectUrl(url: string): boolean {
  try {
    const parsed = new URL(url.trim())
    return (parsed.protocol === 'http:' || parsed.protocol === 'https:') && parsed.hostname !== ''
  } catch {
    return false
  }
}

/**
 * `LearnSettings::normalize`: the address and every name trimmed, a blank
 * audio field is none. Saving the normalized value keeps the form's copy
 * equal to what the backend stores.
 */
export function normalizeSettings(settings: LearnSettings): LearnSettings {
  const { anki } = settings
  const audio = anki.field_mapping.audio_field?.trim() ?? ''
  return {
    meaning_lang: settings.meaning_lang,
    anki: {
      connect_url: anki.connect_url.trim(),
      deck_name: anki.deck_name.trim(),
      model_name: anki.model_name.trim(),
      field_mapping: {
        fields: anki.field_mapping.fields.map((binding) => ({
          anki_field: binding.anki_field.trim(),
          source: binding.source,
        })),
        audio_field: audio === '' ? null : audio,
      },
      create_reverse: anki.create_reverse,
    },
  }
}
