/**
 * vue-i18n setup (Milestone H): `config.ui_lang` was persisted for a long time
 * without anything branching on it. Both windows share this instance; the
 * main window follows the loaded config and the preview window re-reads it
 * per capture, because a language switch in one webview does not reach the
 * other.
 */

import { createI18n } from 'vue-i18n'

import type { LanguageCode } from '../domain/language'
import { de } from './de'
import { en, type MessageSchema } from './en'

export type UiLang = 'en' | 'de'

/** The UI languages, in the order the switch lists them. */
export const UI_LANGS: readonly UiLang[] = ['en', 'de']

type Leaves<T, Prefix extends string = ''> = {
  [K in keyof T & string]: T[K] extends string ? `${Prefix}${K}` : Leaves<T[K], `${Prefix}${K}.`>
}[keyof T & string]

/** A dotted message key (`approval.approve`), checked against the English schema. */
export type MessageKey = Leaves<MessageSchema>

export const i18n = createI18n<[MessageSchema], UiLang, false>({
  legacy: false,
  locale: 'en',
  fallbackLocale: 'en',
  messages: { en, de },
})

/** The UI language for a stored `ui_lang` (`de`, `DE`, `de-AT` -> `de`); English otherwise. */
export function toUiLang(code: string | null | undefined): UiLang {
  const lower = (code ?? '').trim().toLowerCase()
  return lower === 'de' || lower.startsWith('de-') || lower.startsWith('de_') ? 'de' : 'en'
}

/** Switches every component of this window to `code`'s UI language. */
export function setLocale(code: string): UiLang {
  const lang = toUiLang(code)
  i18n.global.locale.value = lang
  if (typeof document !== 'undefined') document.documentElement.lang = lang
  return lang
}

/** The message key of each supported capture language's display name. */
export const LANGUAGE_NAME_KEYS: Record<LanguageCode, MessageKey> = {
  en: 'languages.en',
  de: 'languages.de',
  fr: 'languages.fr',
  es: 'languages.es',
  it: 'languages.it',
  'zh-CN': 'languages.zhCN',
}
