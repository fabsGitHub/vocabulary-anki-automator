/**
 * Field-mapping helpers for the settings editor (Milestones H and J5).
 *
 * The mapping itself is applied in `anki-bridge` (docs/architecture.md § 9);
 * this module only edits the mapping values the backend reads
 * (`anki.field_mapping` and a learned language's saved mapping; `null` for
 * either is matched by field name at push time) and mirrors the rules the
 * editor has to show truthfully: the first binding for an Anki field wins
 * (field names compared case-insensitively, as AnkiConnect compares them),
 * `missing_fields` flags what a note type would silently drop, and a global
 * mapping equal to the Apple-style preset counts as never chosen (§ 8.1).
 * The presets are copies of `FieldMapping::apple_style_default`,
 * `FieldMapping::chinese_default` and `FieldMapping::recommended`, the
 * mapping of the note type the settings offer to create in Anki.
 */

import type {
  AppConfig,
  AutoMatch,
  CardField,
  FieldBinding,
  FieldMapping,
  RecommendedNoteType,
} from '../api'
import { isChinese } from './language'

/** Every card value, in the order the binding selects list them (the examples together). */
export const CARD_FIELDS: readonly CardField[] = [
  'front',
  'back',
  'example_source',
  'example_target',
  'example_in_source_lang',
  'example_in_target_lang',
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

/**
 * The name of the note type `create_recommended_note_type` creates
 * (`anki_bridge::RECOMMENDED_NOTE_TYPE`, which the tests read it from). The
 * UI texts name it through this constant only.
 */
export const RECOMMENDED_NOTE_TYPE = 'Vocabulary Automator'

/**
 * `FieldMapping::recommended`: the recommended note type, a field for every
 * value. The sentences keep their language on the reverse note.
 */
export function recommendedMapping(): FieldMapping {
  return {
    fields: [
      { anki_field: 'Front', source: 'front' },
      { anki_field: 'Back', source: 'back' },
      { anki_field: 'Sentence', source: 'example_in_source_lang' },
      { anki_field: 'Sentence Translation', source: 'example_in_target_lang' },
      { anki_field: 'Pinyin', source: 'pinyin' },
      { anki_field: 'HSK Level', source: 'hsk_level' },
      { anki_field: 'Notes', source: 'notes' },
    ],
    audio_field: 'Audio',
  }
}

export type MappingPresetId = 'recommended' | 'apple' | 'chinese'

/** The built-in mappings, as fresh objects on every call, in the order the editor offers them. */
export const MAPPING_PRESETS: Record<MappingPresetId, () => FieldMapping> = {
  recommended: recommendedMapping,
  apple: appleStyleMapping,
  chinese: chineseMapping,
}

function sameField(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase()
}

/** Whether `a` and `b` are the same mapping, binding for binding, in order. */
export function sameMapping(a: FieldMapping, b: FieldMapping): boolean {
  return (
    a.audio_field === b.audio_field &&
    a.fields.length === b.fields.length &&
    a.fields.every(
      (binding, index) =>
        binding.anki_field === b.fields[index].anki_field &&
        binding.source === b.fields[index].source,
    )
  )
}

/**
 * The global mapping as the backend keeps it (`AppConfig::normalize`): the
 * Apple-style preset is what every config held before the global mapping
 * could be left unset, so it is no choice and becomes `null`, matched by
 * field name like a missing one.
 */
export function globalMapping(mapping: FieldMapping | null): FieldMapping | null {
  return mapping && !sameMapping(mapping, appleStyleMapping()) ? mapping : null
}

/**
 * Points `place`, the Anki tab's settings or a "My languages" entry, at the
 * note type `create_recommended_note_type` set up. Its mapping is left
 * `null`, matched by name, when that gives exactly the recommended mapping
 * (always for a note type the app created: nothing to save, and nothing to
 * go stale); otherwise, for a reused note type whose fields were reordered
 * or renamed, the mapping is saved.
 */
export function selectRecommendedNoteType(
  place: { model_name: string; field_mapping: FieldMapping | null },
  noteType: RecommendedNoteType,
): void {
  place.model_name = noteType.model_name
  place.field_mapping = noteType.matches_by_name ? null : copyMapping(noteType.mapping)
}

/** A copy that shares no object with `mapping`. */
export function copyMapping(mapping: FieldMapping): FieldMapping {
  return {
    fields: mapping.fields.map((binding) => ({ ...binding })),
    audio_field: mapping.audio_field,
  }
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
 * captures from Chinese but nothing in the global mapping receives the
 * pinyin: the saved mapping, or else `match`, the preview of matching the
 * note type by field name (whether a field takes the pinyin does not depend
 * on the language pair). Without either (offline, a mapping matched by name)
 * it stays silent. With Chinese under "My languages" every Chinese capture is
 * filed through that entry, which the languages tab checks on its own the
 * same way.
 */
export function needsChineseNoteType(config: AppConfig, match: AutoMatch | null = null): boolean {
  if (config.learning.languages.some((entry) => isChinese(entry.lang))) return false
  const capturesChinese =
    isChinese(config.translation.source_lang) ||
    config.hotkey_profiles.some((profile) => isChinese(profile.source_lang))
  return capturesChinese && lacksPinyin(config.anki.field_mapping, match)
}

/**
 * Whether the mapping a push would use, `mapping` or, matched by name, the
 * preview `match`, leaves the pinyin out; `false` when neither is known.
 */
export function lacksPinyin(mapping: FieldMapping | null, match: AutoMatch | null): boolean {
  const used = mapping ?? match?.mapping ?? null
  return used !== null && !bindsSource(used, 'pinyin')
}
