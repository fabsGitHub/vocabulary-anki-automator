/**
 * Client-side settings validation, mirroring what `apply_settings` refuses
 * (docs/architecture.md § 8.1): every shortcut must parse and all of them must
 * be distinct, languages must be supported codes, `max_chars` must be usable.
 *
 * The backend remains the authority and its error is still shown; checking
 * here first ties each problem to the field that caused it, which a single
 * error string from the backend cannot do.
 */

import type { AppConfig } from '../api'
import type { MessageKey } from '../i18n'
import { effectiveBindings } from './fieldMapping'
import { isSupportedLang, normalizeLang } from './language'
import { IS_MAC, parseShortcut, shortcutId } from './shortcuts'

/** One problem, addressed by the dotted path of the offending field (`hotkey_profiles.0.shortcut`). */
export interface ConfigIssue {
  path: string
  key: MessageKey
  params?: Record<string, string | number>
}

/**
 * Upper bound for `translation.max_chars`: MyMemory answers at most 500
 * characters per request, so longer captures could not be translated anyway.
 */
export const MAX_CHARS_LIMIT = 500

/** The translation services the backend knows. */
export const TRANSLATION_PROVIDERS = ['mymemory'] as const

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

/** Whether `email` (already trimmed) looks like an address MyMemory would accept. */
export function isPlausibleEmail(email: string): boolean {
  return EMAIL_PATTERN.test(email)
}

/** Every problem in `config`, in form order. Empty means `apply_settings` may be called. */
export function validateConfig(config: AppConfig, isMac: boolean = IS_MAC): ConfigIssue[] {
  const issues: ConfigIssue[] = []
  const add = (path: string, key: MessageKey, params?: ConfigIssue['params']) =>
    issues.push(params ? { path, key, params } : { path, key })

  const checkPair = (base: string, source: string, target: string) => {
    const from = normalizeLang(source)
    const to = normalizeLang(target)
    if (!isSupportedLang(from)) add(`${base}.source_lang`, 'validation.languageUnsupported')
    if (!isSupportedLang(to)) add(`${base}.target_lang`, 'validation.languageUnsupported')
    else if (from === to) add(`${base}.target_lang`, 'validation.languagesEqual')
  }

  /** Shortcut id -> who registered it first (`null` = the default hotkey). */
  const owners = new Map<string, string | null>()
  const checkShortcut = (path: string, text: string, owner: string | null) => {
    if (!text.trim()) {
      add(path, 'validation.required')
      return
    }
    const parsed = parseShortcut(text, isMac)
    if (!parsed) {
      add(path, 'validation.shortcutInvalid')
      return
    }
    const id = shortcutId(parsed)
    if (!owners.has(id)) {
      owners.set(id, owner)
      return
    }
    const first = owners.get(id) ?? null
    if (first === null) add(path, 'validation.shortcutDuplicateDefault')
    else add(path, 'validation.shortcutDuplicate', { other: first })
  }

  checkPair('translation', config.translation.source_lang, config.translation.target_lang)
  checkShortcut('hotkeys', config.hotkeys, null)

  const maxChars = config.translation.max_chars
  if (!Number.isInteger(maxChars) || maxChars < 1 || maxChars > MAX_CHARS_LIMIT) {
    add('translation.max_chars', 'validation.maxCharsRange', { max: MAX_CHARS_LIMIT })
  }
  if (!(TRANSLATION_PROVIDERS as readonly string[]).includes(config.translation.provider)) {
    add('translation.provider', 'validation.providerUnknown')
  }
  const email = (config.translation.contact_email ?? '').trim()
  if (email && !isPlausibleEmail(email)) add('translation.contact_email', 'validation.emailInvalid')

  const ids = new Set<string>()
  config.hotkey_profiles.forEach((profile, index) => {
    const base = `hotkey_profiles.${index}`
    const id = profile.id.trim()
    if (!id) add(`${base}.id`, 'validation.required')
    else if (ids.has(id)) add(`${base}.id`, 'validation.idDuplicate')
    ids.add(id)
    const label = profile.label.trim()
    if (!label) add(`${base}.label`, 'validation.required')
    checkShortcut(`${base}.shortcut`, profile.shortcut, label || id || `#${index + 1}`)
    checkPair(base, profile.source_lang, profile.target_lang)
  })

  if (!config.anki.deck_name.trim()) add('anki.deck_name', 'validation.required')
  if (!config.anki.model_name.trim()) add('anki.model_name', 'validation.required')
  const filled = effectiveBindings(config.anki.field_mapping).filter((b) => b.anki_field.trim())
  if (filled.length === 0) add('anki.field_mapping', 'validation.mappingEmpty')

  return issues
}
