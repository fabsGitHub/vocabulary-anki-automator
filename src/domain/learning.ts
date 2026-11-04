/**
 * The languages the user learns (`learning.languages`, docs/architecture.md
 * § 8.1): what a new entry starts with and which languages an entry may
 * switch to.
 *
 * The user's own language is `translation.target_lang`: cards are translated
 * into it, so it is never a learned language, and every learned language is
 * listed once. Which deck, note type and mapping a card then uses is decided
 * by the backend (`AppConfig::filing`); nothing here mirrors that.
 */

import type { AppConfig, LearnedLanguage } from '../api'
import { SUPPORTED_LANGS, normalizeLang, type LanguageCode } from './language'

/** The deck a newly added language suggests. */
export const DEFAULT_DECKS: Record<LanguageCode, string> = {
  en: 'english',
  de: 'german',
  fr: 'french',
  es: 'spanish',
  it: 'italian',
  'zh-CN': 'chinese',
}

/** The entry for `lang`, in any accepted spelling. */
export function learnedEntry(config: AppConfig, lang: string): LearnedLanguage | null {
  const code = normalizeLang(lang)
  return config.learning.languages.find((entry) => normalizeLang(entry.lang) === code) ?? null
}

/**
 * The supported languages entry `index` may choose: neither the user's own
 * language nor one another entry has. Its current value stays in the list.
 */
export function availableLanguages(config: AppConfig, index: number): LanguageCode[] {
  const native = normalizeLang(config.translation.target_lang)
  const current = normalizeLang(config.learning.languages[index]?.lang ?? '')
  const taken = new Set(
    config.learning.languages
      .filter((_, other) => other !== index)
      .map((entry) => normalizeLang(entry.lang)),
  )
  return SUPPORTED_LANGS.filter((lang) => lang === current || (lang !== native && !taken.has(lang)))
}

/**
 * A new entry for the first supported language that is neither the user's
 * own nor listed yet, or `null` when every one is. It suggests that
 * language's deck, the global note type matched by field name, and the
 * global reverse setting, except that Chinese starts without reverse notes.
 */
export function newLearnedLanguage(config: AppConfig): LearnedLanguage | null {
  const native = normalizeLang(config.translation.target_lang)
  const taken = new Set(config.learning.languages.map((entry) => normalizeLang(entry.lang)))
  const lang = SUPPORTED_LANGS.find((code) => code !== native && !taken.has(code))
  if (!lang) return null
  return {
    lang,
    deck_name: DEFAULT_DECKS[lang],
    model_name: config.anki.model_name,
    field_mapping: null,
    create_reverse: lang === 'zh-CN' ? false : config.anki.create_reverse,
  }
}
