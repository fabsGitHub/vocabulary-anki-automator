/**
 * vue-i18n setup for Learn (English and German, like the Automator).
 *
 * Learn's settings (`LearnSettings`, shared with the backend) have no UI
 * language, so the language follows the system's on first start and the
 * switch in the header remembers a choice per device. That memory is a
 * convenience of this webview only: when storage is unavailable (private
 * browsing in the web build) the app still starts, in the system language.
 */

import { createI18n } from 'vue-i18n'

import { de } from './de'
import { en, type MessageSchema } from './en'

export type UiLang = 'en' | 'de'

/** The UI languages, in the order the switch lists them. */
export const UI_LANGS: readonly UiLang[] = ['en', 'de']

/** Each UI language in its own language, for the switch. */
export const UI_LANG_LABELS: Record<UiLang, string> = { en: 'English', de: 'Deutsch' }

type Leaves<T, Prefix extends string = ''> = {
  [K in keyof T & string]: T[K] extends string ? `${Prefix}${K}` : Leaves<T[K], `${Prefix}${K}.`>
}[keyof T & string]

/** A dotted message key (`review.showAnswer`), checked against the English schema. */
export type MessageKey = Leaves<MessageSchema>

export const i18n = createI18n<[MessageSchema], UiLang, false>({
  legacy: false,
  locale: 'en',
  fallbackLocale: 'en',
  messages: { en, de },
})

/** Where the header switch remembers the chosen language. */
export const UI_LANG_STORAGE_KEY = 'learn.uiLang'

/** The UI language for a language tag (`de`, `DE`, `de-AT` -> `de`); English otherwise. */
export function toUiLang(code: string | null | undefined): UiLang {
  const lower = (code ?? '').trim().toLowerCase()
  return lower === 'de' || lower.startsWith('de-') || lower.startsWith('de_') ? 'de' : 'en'
}

/** The remembered choice, else the system language. */
export function initialUiLang(): UiLang {
  let stored: string | null = null
  try {
    stored = window.localStorage.getItem(UI_LANG_STORAGE_KEY)
  } catch {
    stored = null
  }
  if (stored) return toUiLang(stored)
  return toUiLang(typeof navigator === 'undefined' ? null : navigator.language)
}

/** Switches the whole window to `code`'s UI language. */
export function setLocale(code: string): UiLang {
  const lang = toUiLang(code)
  i18n.global.locale.value = lang
  if (typeof document !== 'undefined') {
    document.documentElement.lang = lang
    document.title = i18n.global.t('app.title')
  }
  return lang
}

/** {@link setLocale}, remembered for the next start. */
export function chooseUiLang(code: string): UiLang {
  const lang = setLocale(code)
  try {
    window.localStorage.setItem(UI_LANG_STORAGE_KEY, lang)
  } catch {
    // Storage is a convenience; the switch still works for this session.
  }
  return lang
}
