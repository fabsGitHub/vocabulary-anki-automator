/**
 * Language codes, mirrored from `vocab_core::language` (docs/architecture.md § 3).
 *
 * The settings form only offers canonical codes, but a config written by an
 * older build can still hold DeepL-style `EN`/`DE`; normalizing on load keeps
 * the selects, the validation and the saved payload on the one spelling the
 * backend compares against.
 */

/** Canonical codes, in the order the language selects list them. */
export const SUPPORTED_LANGS = ['en', 'de', 'fr', 'es', 'it', 'zh-CN'] as const

export type LanguageCode = (typeof SUPPORTED_LANGS)[number]

const CHINESE: LanguageCode = 'zh-CN'

/** Spellings of Simplified Chinese, after lowercasing and turning `_` into `-`. */
const CHINESE_ALIASES = ['zh', 'zh-cn', 'zh-hans', 'zh-hans-cn']

/**
 * `normalize_lang`: `"EN"` -> `"en"`, `"zh"`/`"zh_CN"`/`"zh-Hans"` -> `"zh-CN"`;
 * anything unknown comes back trimmed and lowercased.
 */
export function normalizeLang(code: string): string {
  const lower = code.trim().toLowerCase()
  const unified = lower.replace(/_/g, '-')
  if (CHINESE_ALIASES.includes(unified)) return CHINESE
  return SUPPORTED_LANGS.find((supported) => supported.toLowerCase() === unified) ?? lower
}

/** Whether `code` is one of the canonical {@link SUPPORTED_LANGS}, spelled canonically. */
export function isSupportedLang(code: string): code is LanguageCode {
  return (SUPPORTED_LANGS as readonly string[]).includes(code)
}

/** Whether `code` denotes (Simplified) Chinese in any accepted spelling. */
export function isChinese(code: string): boolean {
  return normalizeLang(code) === CHINESE
}
