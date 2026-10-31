/**
 * Field-mapping helpers for the settings editor (Milestones H and J5).
 *
 * The mapping itself is applied in `anki-bridge` (docs/architecture.md § 9);
 * this module only edits the `anki.field_mapping` value the backend reads and
 * mirrors the two rules the editor has to show truthfully: the first binding
 * for an Anki field wins (field names compared case-insensitively, as
 * AnkiConnect compares them), and `missing_fields` flags what a note type
 * would silently drop. The presets are copies of
 * `FieldMapping::apple_style_default` and `FieldMapping::chinese_default`.
 */

import type { AppConfig, CardField, FieldBinding, FieldMapping } from '../api'
import { isChinese } from './language'

/** Every card value, in the order the binding selects list them. */
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

/** `FieldMapping::apple_style_default`: the AppleStyleFinalV3 note type. */
export function appleStyleMapping(): FieldMapping {
  return {
    fields: [
      { anki_field: 'Front', source: 'front' },
      { anki_field: 'Back', source: 'back' },
      { anki_field: 'Context Sentence', source: 'example' },
      { anki_field: 'Notes', source: 'notes' },
    ],
    audio_field: 'Front Audio',
  }
}

/** `FieldMapping::chinese_default`: a Hanzi/Pinyin/Meaning/Sentence/Audio note type. */
export function chineseMapping(): FieldMapping {
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

export type MappingPresetId = 'apple' | 'chinese'

/** The built-in mappings, as fresh objects on every call. */
export const MAPPING_PRESETS: Record<MappingPresetId, () => FieldMapping> = {
  apple: appleStyleMapping,
  chinese: chineseMapping,
}

function sameField(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase()
}

/** The bindings that take effect: the first one for each Anki field. */
export function effectiveBindings(mapping: FieldMapping): FieldBinding[] {
  return mapping.fields.filter(
    (binding, index) =>
      !mapping.fields
        .slice(0, index)
        .some((earlier) => sameField(earlier.anki_field, binding.anki_field)),
  )
}

/** The binding that fills `ankiField`, if any. */
export function bindingFor(mapping: FieldMapping, ankiField: string): FieldBinding | undefined {
  return mapping.fields.find((binding) => sameField(binding.anki_field, ankiField))
}

/** Whether some effective binding writes `source`. */
export function bindsSource(mapping: FieldMapping, source: CardField): boolean {
  return effectiveBindings(mapping).some((binding) => binding.source === source)
}

/**
 * `mapping` with `ankiField` filled from `source`, or emptied for `null`.
 * Other bindings keep their position, so an unchanged editor value never
 * reorders (and never dirties) the saved mapping.
 */
export function withBinding(
  mapping: FieldMapping,
  ankiField: string,
  source: CardField | null,
): FieldMapping {
  const index = mapping.fields.findIndex((binding) => sameField(binding.anki_field, ankiField))
  const others = mapping.fields.filter((binding) => !sameField(binding.anki_field, ankiField))
  if (source === null) return { ...mapping, fields: others }
  const binding: FieldBinding = { anki_field: ankiField, source }
  if (index === -1) return { ...mapping, fields: [...others, binding] }
  const fields = [...others]
  fields.splice(index, 0, binding)
  return { ...mapping, fields }
}

/** `mapping` sending audio to `field`; blank or `null` sends none. */
export function withAudioField(mapping: FieldMapping, field: string | null): FieldMapping {
  const audio = field !== null && field.trim() !== '' ? field : null
  return { ...mapping, audio_field: audio }
}

/**
 * `FieldMapping::missing_fields`: the fields the mapping writes (audio field
 * included) that a note type with `modelFields` lacks, in mapping order. Text
 * fields compare case-insensitively, the audio field exactly.
 */
export function missingFields(mapping: FieldMapping, modelFields: string[]): string[] {
  const missing: string[] = []
  const flag = (name: string, known: boolean) => {
    if (!known && !missing.includes(name)) missing.push(name)
  }
  for (const binding of effectiveBindings(mapping)) {
    flag(
      binding.anki_field,
      modelFields.some((field) => sameField(field, binding.anki_field)),
    )
  }
  const audio = mapping.audio_field
  if (audio !== null && audio.trim() !== '') {
    flag(
      audio,
      modelFields.some((field) => field === audio),
    )
  }
  return missing
}

/**
 * Milestone J5: Chinese captures carry pinyin and an HSK level that only a
 * Chinese note type can hold. True when the default hotkey or any profile
 * captures from Chinese but nothing in the mapping receives the pinyin.
 */
export function needsChineseNoteType(config: AppConfig): boolean {
  const capturesChinese =
    isChinese(config.translation.source_lang) ||
    config.hotkey_profiles.some((profile) => isChinese(profile.source_lang))
  return capturesChinese && !bindsSource(config.anki.field_mapping, 'pinyin')
}
