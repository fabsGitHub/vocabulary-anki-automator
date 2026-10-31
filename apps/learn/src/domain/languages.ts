/**
 * Display names for the canonical language codes of docs/architecture.md § 3
 * (`en`, `de`, `fr`, `es`, `it`, `zh-CN`). Cards and lessons carry codes; the
 * UI shows the name in the current UI language, and an unknown code as
 * itself, upper-cased, rather than nothing.
 */

import type { MessageKey } from '../i18n'

/** The message key of each supported language's display name. */
export const LANGUAGE_NAME_KEYS: Record<string, MessageKey> = {
  en: 'languages.en',
  de: 'languages.de',
  fr: 'languages.fr',
  es: 'languages.es',
  it: 'languages.it',
  'zh-CN': 'languages.zhCN',
}

/** `code`'s display name, translated with `t`. */
export function languageName(code: string, t: (key: MessageKey) => string): string {
  const key = LANGUAGE_NAME_KEYS[code]
  return key ? t(key) : code.toUpperCase()
}
