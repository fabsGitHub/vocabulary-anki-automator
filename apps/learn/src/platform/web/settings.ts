/**
 * Learn's settings (`LearnSettings` in `../types`): vocab-learn's
 * `settings` module (apps/learn/src-tauri/src/settings.rs), kept in the
 * same `app_state` key (`learn.settings`, docs/architecture.md § 6.4) as
 * the same JSON.
 *
 * **Lenient load, strict save.** A stored value that does not parse loads as
 * the defaults (and stays until the next save), and missing fields get
 * their defaults, as serde's `#[serde(default)]` gives them. Saving
 * normalizes the input and refuses whatever {@link validateSettings}
 * rejects, with the desktop build's sentences. The web build cannot export
 * to Anki, but it keeps the Anki settings all the same, so a learner who
 * moves to the desktop app finds them as they left them.
 */

import type { CardField, FieldMapping, LearnSettings } from '../types'
import type { Store } from './store'
import { trim } from './text'

/** `SETTINGS_KEY`. */
export const SETTINGS_KEY = 'learn.settings'

/** The note fields `anki_bridge::CardField` names. */
const CARD_FIELDS: readonly CardField[] = [
  'front',
  'back',
  'example_source',
  'example_target',
  'example',
  'notes',
  'pinyin',
  'hsk_level',
]

/**
 * `LearnSettings::default()`: English meanings, the language every bundled
 * meaning has, and a Chinese note type in its own deck (no reverse notes,
 * since a Chinese note type normally has its own production template).
 */
export function defaultSettings(): LearnSettings {
  return {
    meaning_lang: 'en',
    anki: {
      connect_url: 'http://127.0.0.1:8765',
      deck_name: 'Chinese::HSK',
      model_name: 'Chinese (HSK)',
      field_mapping: {
        fields: [
          { anki_field: 'Hanzi', source: 'front' },
          { anki_field: 'Pinyin', source: 'pinyin' },
          { anki_field: 'Meaning', source: 'back' },
          { anki_field: 'Sentence', source: 'example' },
        ],
        audio_field: 'Audio',
      },
      create_reverse: false,
    },
  }
}

/**
 * The settings in `value`, as serde would deserialize them: missing fields
 * take their defaults, unknown ones are ignored, anything of the wrong type
 * is an error (thrown with the reason).
 */
export function parseSettings(value: unknown): LearnSettings {
  const defaults = defaultSettings()
  const root = object(value, 'the settings')
  const anki = root.anki === undefined ? {} : object(root.anki, 'anki')
  return {
    meaning_lang: field(root, 'meaning_lang', defaults.meaning_lang, (raw) => {
      if (raw !== 'de' && raw !== 'en')
        throw new Error(`unknown meaning_lang ${JSON.stringify(raw)}`)
      return raw
    }),
    anki: {
      connect_url: field(anki, 'connect_url', defaults.anki.connect_url, string),
      deck_name: field(anki, 'deck_name', defaults.anki.deck_name, string),
      model_name: field(anki, 'model_name', defaults.anki.model_name, string),
      field_mapping: field(anki, 'field_mapping', defaults.anki.field_mapping, (raw) =>
        fieldMapping(raw),
      ),
      create_reverse: field(anki, 'create_reverse', defaults.anki.create_reverse, (raw, name) => {
        if (typeof raw !== 'boolean') throw new Error(`${name} is not a boolean`)
        return raw
      }),
    },
  }
}

/** `LearnSettings::normalize`: trims every name and the URL; a blank audio field is none. */
export function normalizeSettings(settings: LearnSettings): LearnSettings {
  const { anki } = settings
  const audio = anki.field_mapping.audio_field === null ? '' : trim(anki.field_mapping.audio_field)
  return {
    meaning_lang: settings.meaning_lang,
    anki: {
      connect_url: trim(anki.connect_url),
      deck_name: trim(anki.deck_name),
      model_name: trim(anki.model_name),
      field_mapping: {
        fields: anki.field_mapping.fields.map((binding) => ({
          anki_field: trim(binding.anki_field),
          source: binding.source,
        })),
        audio_field: audio === '' ? null : audio,
      },
      create_reverse: anki.create_reverse,
    },
  }
}

/**
 * `LearnSettings::validate` of normalized settings: every problem, one
 * sentence each; none when they can be saved.
 */
export function validateSettings(settings: LearnSettings): string[] {
  const { anki } = settings
  const problems: string[] = []
  if (!isHttpUrl(anki.connect_url)) {
    problems.push(`The AnkiConnect address "${anki.connect_url}" is not an http:// URL.`)
  }
  if (anki.deck_name === '') problems.push('The Anki deck name must not be empty.')
  if (anki.model_name === '') problems.push('The Anki note type must not be empty.')
  const { fields } = anki.field_mapping
  if (fields.some((binding) => binding.anki_field === '')) {
    problems.push('Every mapped Anki field needs a name.')
  }
  for (const [source, what] of [
    ['front', 'word'],
    ['back', 'meaning'],
  ] as const) {
    if (!fields.some((binding) => binding.source === source)) {
      problems.push(`The field mapping needs an Anki field for the ${what}.`)
    }
  }
  return problems
}

/** `load`: the stored settings, or the defaults when there are none or they do not parse. */
export async function loadSettings(store: Store): Promise<LearnSettings> {
  const text = await store.getState(SETTINGS_KEY)
  if (text === null) return defaultSettings()
  try {
    return normalizeSettings(parseSettings(JSON.parse(text)))
  } catch {
    return defaultSettings()
  }
}

/** `save`: stores `settings` as they are; validating them is the caller's job. */
export async function saveSettings(store: Store, settings: LearnSettings): Promise<void> {
  await store.setState(SETTINGS_KEY, JSON.stringify(settings))
}

function fieldMapping(value: unknown): FieldMapping {
  const mapping = object(value, 'field_mapping')
  if (!Array.isArray(mapping.fields)) throw new Error('field_mapping.fields is not a list')
  const fields = mapping.fields.map((raw: unknown) => {
    const binding = object(raw, 'a field binding')
    const source = binding.source
    if (!CARD_FIELDS.includes(source as CardField)) {
      throw new Error(`unknown card field ${JSON.stringify(source)}`)
    }
    return { anki_field: string(binding.anki_field, 'anki_field'), source: source as CardField }
  })
  const audio = mapping.audio_field
  return {
    fields,
    audio_field: audio === undefined || audio === null ? null : string(audio, 'audio_field'),
  }
}

function field<T>(
  from: Record<string, unknown>,
  name: string,
  fallback: T,
  parse: (raw: unknown, name: string) => T,
): T {
  return from[name] === undefined ? fallback : parse(from[name], name)
}

function object(value: unknown, what: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`${what} is not an object`)
  }
  return value as Record<string, unknown>
}

function string(value: unknown, name: string): string {
  if (typeof value !== 'string') throw new Error(`${name} is not a string`)
  return value
}

/** `Url::parse` succeeds, with an `http` or `https` scheme and a host. */
function isHttpUrl(text: string): boolean {
  try {
    const url = new URL(text)
    return (url.protocol === 'http:' || url.protocol === 'https:') && url.hostname !== ''
  } catch {
    return false
  }
}
