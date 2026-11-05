/**
 * `vocab_core::language`: the part Learn uses, language-code normalization
 * and CJK detection.
 *
 * The quiz engine's isolation rule counts hanzi with {@link isCjkChar}, so
 * its two blocks must be exactly the Rust ones; `\p{Script=Han}` would also
 * count compatibility ideographs and the later extensions.
 */

import { chars, codePoint, trim } from './text'

/** `SUPPORTED_LANGS`. */
export const SUPPORTED_LANGS = ['en', 'de', 'fr', 'es', 'it', 'zh-CN'] as const

/** The canonical code for Simplified Chinese. */
export const CHINESE = 'zh-CN'

/** Spellings of Simplified Chinese, after lowercasing and turning `_` into `-`. */
const CHINESE_ALIASES = ['zh', 'zh-cn', 'zh-hans', 'zh-hans-cn']

/**
 * `normalize_lang`: `"EN"` -> `"en"`, every spelling of Simplified Chinese
 * -> `"zh-CN"`; other codes come back trimmed (Rust whitespace) and
 * lowercased.
 */
export function normalizeLang(code: string): string {
  const lower = trim(code).toLowerCase()
  const unified = lower.replace(/_/g, '-')
  if (CHINESE_ALIASES.includes(unified)) return CHINESE
  return SUPPORTED_LANGS.find((supported) => eqIgnoreAsciiCase(supported, unified)) ?? lower
}

/** `is_chinese`. */
export function isChinese(code: string): boolean {
  return normalizeLang(code) === CHINESE
}

/** `is_cjk_char` of one code point: U+4E00..=U+9FFF and U+3400..=U+4DBF. */
export function isCjkChar(c: string): boolean {
  const point = codePoint(c)
  return (point >= 0x4e00 && point <= 0x9fff) || (point >= 0x3400 && point <= 0x4dbf)
}

/** `contains_cjk`. */
export function containsCjk(text: string): boolean {
  return chars(text).some(isCjkChar)
}

/** `cjk_chars`: in order, duplicates kept. */
export function cjkChars(text: string): string[] {
  return chars(text).filter(isCjkChar)
}

/** `str::eq_ignore_ascii_case`. */
function eqIgnoreAsciiCase(a: string, b: string): boolean {
  return a.length === b.length && asciiLowercase(a) === asciiLowercase(b)
}

function asciiLowercase(text: string): string {
  return text.replace(/[A-Z]/g, (letter) => letter.toLowerCase())
}
