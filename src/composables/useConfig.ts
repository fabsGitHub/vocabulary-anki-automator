/**
 * Settings state for the main window (Milestones C, G and H): load, edit,
 * validate, save.
 *
 * The pre-refactor form assembled its `apply_settings` payload by hand and kept
 * sending a `deepl` section with an `api_key`. Here the payload is rebuilt
 * field by field from the § 8.1 shape ({@link toSettingsPayload}), so keys
 * the backend no longer knows can never be sent back, whatever
 * `get_current_settings` returned. Dirty tracking and validation both look at
 * that payload, i.e. at exactly what a save would send.
 */

import { computed, inject, provide, ref, type ComputedRef, type InjectionKey, type Ref } from 'vue'

import {
  applySettings,
  errorMessage,
  getCurrentSettings,
  type AppConfig,
  type CardField,
  type ConsentState,
  type HotkeyProfile,
} from '../api'
import { appleStyleMapping } from '../domain/fieldMapping'
import { normalizeLang } from '../domain/language'
import { IS_MAC } from '../domain/shortcuts'
import { validateConfig, type ConfigIssue } from '../domain/validation'

/** The backend's defaults (§ 8.1); shown only until the real config has loaded. */
export function defaultConfig(): AppConfig {
  return {
    ui_lang: 'en',
    hotkeys: 'CommandOrControl+Shift+C',
    hotkey_profiles: [],
    translation: {
      source_lang: 'en',
      target_lang: 'de',
      max_chars: 100,
      provider: 'mymemory',
      contact_email: null,
    },
    anki: {
      connect_url: 'http://127.0.0.1:8765',
      deck_name: 'English::AutomatedVocabulary',
      model_name: 'AppleStyleFinalV3',
      create_reverse: true,
      field_mapping: appleStyleMapping(),
    },
    consent: { anki_install_granted_at: null, ankiconnect_provision_granted_at: null },
  }
}

type Loose<T> = { [K in keyof T]?: T[K] extends object ? Loose<T[K]> | null : T[K] | null }

function text(value: unknown, fallback: string): string {
  return typeof value === 'string' ? value : fallback
}

function optionalText(value: unknown): string | null {
  return typeof value === 'string' ? value : null
}

function blankToNull(value: string | null): string | null {
  return value !== null && value.trim() !== '' ? value.trim() : null
}

/**
 * A complete config from whatever `get_current_settings` returned: missing
 * sections take their defaults and language codes are made canonical, so the
 * form never has to deal with partial or legacy data.
 */
export function normalizeConfig(raw: unknown): AppConfig {
  const base = defaultConfig()
  const source = (raw ?? {}) as Loose<AppConfig>
  const translation = source.translation ?? {}
  const anki = source.anki ?? {}
  const mapping = anki.field_mapping ?? null
  const consent = source.consent ?? {}
  const maxChars = Number(translation.max_chars)

  return {
    ui_lang: text(source.ui_lang, base.ui_lang),
    hotkeys: text(source.hotkeys, base.hotkeys),
    hotkey_profiles: Array.isArray(source.hotkey_profiles)
      ? (source.hotkey_profiles as (Loose<HotkeyProfile> | null)[]).map((entry) => {
          const profile = entry ?? {}
          return {
            id: text(profile.id, ''),
            label: text(profile.label, ''),
            shortcut: text(profile.shortcut, ''),
            source_lang: normalizeLang(text(profile.source_lang, base.translation.source_lang)),
            target_lang: normalizeLang(text(profile.target_lang, base.translation.target_lang)),
            deck_name: optionalText(profile.deck_name),
          }
        })
      : [],
    translation: {
      source_lang: normalizeLang(text(translation.source_lang, base.translation.source_lang)),
      target_lang: normalizeLang(text(translation.target_lang, base.translation.target_lang)),
      max_chars: Number.isFinite(maxChars) ? maxChars : base.translation.max_chars,
      provider: text(translation.provider, base.translation.provider),
      contact_email: optionalText(translation.contact_email),
    },
    anki: {
      connect_url: text(anki.connect_url, base.anki.connect_url),
      deck_name: text(anki.deck_name, base.anki.deck_name),
      model_name: text(anki.model_name, base.anki.model_name),
      create_reverse:
        typeof anki.create_reverse === 'boolean' ? anki.create_reverse : base.anki.create_reverse,
      field_mapping: mapping
        ? {
            fields: Array.isArray(mapping.fields)
              ? mapping.fields.map((binding) => ({
                  anki_field: text(binding?.anki_field, ''),
                  source: text(binding?.source, 'front') as CardField,
                }))
              : [],
            audio_field: optionalText(mapping.audio_field),
          }
        : base.anki.field_mapping,
    },
    consent: {
      anki_install_granted_at: optionalText(consent.anki_install_granted_at),
      ankiconnect_provision_granted_at: optionalText(consent.ankiconnect_provision_granted_at),
    },
  }
}

/**
 * Exactly what `apply_settings` receives: the § 8.1 keys and nothing else,
 * text trimmed, blank optional values as `null`, language codes canonical.
 */
export function toSettingsPayload(config: AppConfig): AppConfig {
  return {
    ui_lang: config.ui_lang,
    hotkeys: config.hotkeys.trim(),
    hotkey_profiles: config.hotkey_profiles.map((profile) => ({
      id: profile.id.trim(),
      label: profile.label.trim(),
      shortcut: profile.shortcut.trim(),
      source_lang: normalizeLang(profile.source_lang),
      target_lang: normalizeLang(profile.target_lang),
      deck_name: blankToNull(profile.deck_name),
    })),
    translation: {
      source_lang: normalizeLang(config.translation.source_lang),
      target_lang: normalizeLang(config.translation.target_lang),
      max_chars: config.translation.max_chars,
      provider: config.translation.provider,
      contact_email: blankToNull(config.translation.contact_email),
    },
    anki: {
      connect_url: config.anki.connect_url,
      deck_name: config.anki.deck_name.trim(),
      model_name: config.anki.model_name.trim(),
      create_reverse: config.anki.create_reverse,
      field_mapping: {
        fields: config.anki.field_mapping.fields.map((binding) => ({
          anki_field: binding.anki_field,
          source: binding.source,
        })),
        audio_field: blankToNull(config.anki.field_mapping.audio_field),
      },
    },
    consent: {
      anki_install_granted_at: config.consent.anki_install_granted_at,
      ankiconnect_provision_granted_at: config.consent.ankiconnect_provision_granted_at,
    },
  }
}

export interface ConfigStore {
  /** The working copy the form edits. Holds the defaults until {@link ConfigStore.load} succeeds. */
  config: Ref<AppConfig>
  loaded: Ref<boolean>
  loading: Ref<boolean>
  loadError: Ref<string | null>
  saving: Ref<boolean>
  /** The backend's message when the last save failed. */
  saveError: Ref<string | null>
  /** The backend's message when the last language switch could not be saved. */
  uiLangError: Ref<string | null>
  /** Whether a save would change the stored config. */
  dirty: ComputedRef<boolean>
  issues: ComputedRef<ConfigIssue[]>
  isValid: ComputedRef<boolean>
  /** The first problem of the field at `path`. */
  issueFor(path: string): ConfigIssue | undefined
  /** Whether any problem lies at or below `prefix` (`translation`, `hotkey_profiles`). */
  hasIssuesUnder(prefix: string): boolean
  load(): Promise<void>
  /** Validates and saves; `false` (and nothing sent) while anything is invalid. */
  save(): Promise<boolean>
  /** Throws away unsaved edits, and the error of the save that failed on them. */
  revert(): void
  /**
   * Switches `ui_lang` and saves it at once, on top of the last saved config:
   * unsaved (possibly invalid) edits elsewhere in the form are not sent along.
   */
  persistUiLang(lang: string): Promise<boolean>
  /** Takes the state returned by `grant_consent`; consent is not a form edit, so nothing turns dirty. */
  setConsent(consent: ConsentState): void
}

export function useConfig(options: { isMac?: boolean } = {}): ConfigStore {
  const isMac = options.isMac ?? IS_MAC
  const config = ref<AppConfig>(defaultConfig())
  const snapshot = ref(JSON.stringify(toSettingsPayload(config.value)))
  const loaded = ref(false)
  const loading = ref(false)
  const loadError = ref<string | null>(null)
  const saving = ref(false)
  const saveError = ref<string | null>(null)
  const uiLangError = ref<string | null>(null)

  const payload = computed(() => toSettingsPayload(config.value))
  const dirty = computed(() => loaded.value && JSON.stringify(payload.value) !== snapshot.value)
  const issues = computed(() => validateConfig(payload.value, isMac))
  const isValid = computed(() => issues.value.length === 0)

  const savedConfig = (): AppConfig => JSON.parse(snapshot.value) as AppConfig

  async function load(): Promise<void> {
    loading.value = true
    loadError.value = null
    try {
      const loadedConfig = normalizeConfig(await getCurrentSettings())
      config.value = loadedConfig
      snapshot.value = JSON.stringify(toSettingsPayload(loadedConfig))
      loaded.value = true
    } catch (err) {
      loadError.value = errorMessage(err)
    } finally {
      loading.value = false
    }
  }

  async function save(): Promise<boolean> {
    if (!loaded.value || saving.value) return false
    saveError.value = null
    if (!isValid.value) return false
    const body = toSettingsPayload(config.value)
    saving.value = true
    try {
      await applySettings(body)
      snapshot.value = JSON.stringify(body)
      return true
    } catch (err) {
      saveError.value = errorMessage(err)
      return false
    } finally {
      saving.value = false
    }
  }

  function revert(): void {
    if (!loaded.value) return
    config.value = normalizeConfig(savedConfig())
    saveError.value = null
  }

  async function persistUiLang(lang: string): Promise<boolean> {
    config.value.ui_lang = lang
    uiLangError.value = null
    if (!loaded.value) return false
    const saved = savedConfig()
    if (saved.ui_lang === lang) return true
    saved.ui_lang = lang
    try {
      await applySettings(saved)
      snapshot.value = JSON.stringify(saved)
      return true
    } catch (err) {
      uiLangError.value = errorMessage(err)
      return false
    }
  }

  function setConsent(consent: ConsentState): void {
    const copy = {
      anki_install_granted_at: consent.anki_install_granted_at,
      ankiconnect_provision_granted_at: consent.ankiconnect_provision_granted_at,
    }
    config.value.consent = copy
    if (!loaded.value) return
    const saved = savedConfig()
    saved.consent = { ...copy }
    snapshot.value = JSON.stringify(saved)
  }

  return {
    config,
    loaded,
    loading,
    loadError,
    saving,
    saveError,
    uiLangError,
    dirty,
    issues,
    isValid,
    issueFor: (path) => issues.value.find((issue) => issue.path === path),
    hasIssuesUnder: (prefix) =>
      issues.value.some((issue) => issue.path === prefix || issue.path.startsWith(`${prefix}.`)),
    load,
    save,
    revert,
    persistUiLang,
    setConsent,
  }
}

/** Injection key of the main window's store; tests use it to mount a single tab. */
export const CONFIG_STORE_KEY: InjectionKey<ConfigStore> = Symbol('config-store')

/** Makes `store` available to every settings tab below the calling component. */
export function provideConfigStore(store: ConfigStore): void {
  provide(CONFIG_STORE_KEY, store)
}

/** The store of the nearest {@link provideConfigStore} ancestor. */
export function useConfigStore(): ConfigStore {
  const store = inject(CONFIG_STORE_KEY, null)
  if (!store) throw new Error('useConfigStore() needs a provideConfigStore() ancestor')
  return store
}
