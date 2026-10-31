import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AppConfig } from '../api'
import { callsTo, commandNames, onCommand, resetTauri } from '../test/fakeTauri'
import { defaultConfig, normalizeConfig, toSettingsPayload, useConfig } from './useConfig'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

function backendConfig(): AppConfig {
  return {
    ui_lang: 'de',
    hotkeys: 'CommandOrControl+Shift+C',
    hotkey_profiles: [
      {
        id: 'zh-de',
        label: 'Chinese → German',
        shortcut: 'CommandOrControl+Shift+H',
        source_lang: 'zh-CN',
        target_lang: 'de',
        deck_name: 'Chinese::HSK',
      },
    ],
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
      field_mapping: {
        fields: [{ anki_field: 'Front', source: 'front' }],
        audio_field: 'Front Audio',
      },
    },
    consent: { anki_install_granted_at: null, ankiconnect_provision_granted_at: null },
  }
}

async function loadedStore(config: unknown = backendConfig()) {
  onCommand('get_current_settings', config)
  const store = useConfig({ isMac: false })
  await store.load()
  return store
}

function sentConfig(index = 0): AppConfig {
  const call = callsTo('apply_settings')[index]
  expect(Object.keys(call.args ?? {})).toEqual(['newCfg'])
  return call.args?.newCfg as AppConfig
}

beforeEach(() => resetTauri())

/** A legacy file the backend might still hand over unmapped. */
const LEGACY_EXTRAS = {
  deepl: { api_key: 'secret-key', source_lang: 'EN', target_lang: 'DE', max_chars: 100 },
  api_key: 'secret-key',
}

describe('the two layers that keep legacy keys out', () => {
  it('normalizeConfig drops keys outside § 8.1', () => {
    const normalized = normalizeConfig({ ...backendConfig(), ...LEGACY_EXTRAS })
    expect(JSON.stringify(normalized)).not.toMatch(/deepl|api_key|secret-key/)
    expect(normalized).toEqual(backendConfig())
  })

  it('toSettingsPayload drops keys outside § 8.1, even on the working copy', () => {
    const polluted = { ...backendConfig(), ...LEGACY_EXTRAS } as AppConfig
    polluted.translation = {
      ...polluted.translation,
      api_key: 'secret-key',
    } as AppConfig['translation']
    const payload = toSettingsPayload(polluted)
    expect(JSON.stringify(payload)).not.toMatch(/deepl|api_key|secret-key/)
    expect(payload).toEqual(backendConfig())
  })
})

describe('load', () => {
  it('takes the backend config as the clean working copy', async () => {
    const store = await loadedStore()
    expect(store.loaded.value).toBe(true)
    expect(store.config.value).toEqual(backendConfig())
    expect(store.dirty.value).toBe(false)
    expect(store.isValid.value).toBe(true)
  })

  it('makes legacy language codes canonical and fills sections an older backend lacks', async () => {
    const store = await loadedStore({
      ui_lang: 'en',
      hotkeys: 'CommandOrControl+Shift+C',
      translation: { source_lang: 'EN', target_lang: 'zh_CN', max_chars: 80, provider: 'mymemory' },
      anki: { deck_name: 'Deck', model_name: 'Basic' },
    })
    const config = store.config.value
    expect(config.translation.source_lang).toBe('en')
    expect(config.translation.target_lang).toBe('zh-CN')
    expect(config.translation.contact_email).toBeNull()
    expect(config.hotkey_profiles).toEqual([])
    expect(config.anki.field_mapping).toEqual(defaultConfig().anki.field_mapping)
    expect(config.anki.create_reverse).toBe(true)
    expect(config.consent).toEqual({
      anki_install_granted_at: null,
      ankiconnect_provision_granted_at: null,
    })
  })

  it('reports a failed load and refuses to save the defaults over the real config', async () => {
    onCommand('get_current_settings', () => {
      throw 'config.json is corrupt'
    })
    const store = useConfig({ isMac: false })
    await store.load()
    expect(store.loaded.value).toBe(false)
    expect(store.loadError.value).toBe('config.json is corrupt')
    expect(await store.save()).toBe(false)
    expect(commandNames()).toEqual(['get_current_settings'])
  })
})

describe('save', () => {
  it('sends exactly the § 8.1 shape as newCfg, never a deepl section or api_key', async () => {
    const legacy = {
      ...backendConfig(),
      deepl: { api_key: 'secret-key', source_lang: 'EN', target_lang: 'DE', max_chars: 100 },
      api_key: 'secret-key',
    }
    const store = await loadedStore(legacy)
    onCommand('apply_settings', null)
    store.config.value.translation.contact_email = '  me@example.com  '
    expect(store.dirty.value).toBe(true)

    expect(await store.save()).toBe(true)
    const sent = sentConfig()
    expect(Object.keys(sent).sort()).toEqual(
      ['anki', 'consent', 'hotkey_profiles', 'hotkeys', 'translation', 'ui_lang'].sort(),
    )
    expect(Object.keys(sent.translation).sort()).toEqual(
      ['contact_email', 'max_chars', 'provider', 'source_lang', 'target_lang'].sort(),
    )
    expect(Object.keys(sent.anki).sort()).toEqual(
      ['connect_url', 'create_reverse', 'deck_name', 'field_mapping', 'model_name'].sort(),
    )
    expect(Object.keys(sent.hotkey_profiles[0]).sort()).toEqual(
      ['deck_name', 'id', 'label', 'shortcut', 'source_lang', 'target_lang'].sort(),
    )
    expect(JSON.stringify(sent)).not.toMatch(/deepl|api_key|secret-key/)
    expect(sent.translation.contact_email).toBe('me@example.com')
    expect(sent.hotkey_profiles[0]).toEqual(backendConfig().hotkey_profiles[0])
    expect(store.dirty.value).toBe(false)
  })

  it('sends blank optional values as null', async () => {
    const store = await loadedStore({
      ...backendConfig(),
      translation: { ...backendConfig().translation, contact_email: 'me@example.com' },
    })
    onCommand('apply_settings', null)
    store.config.value.translation.contact_email = ''
    store.config.value.hotkey_profiles[0].deck_name = '   '
    store.config.value.anki.field_mapping.audio_field = ' '
    await store.save()
    const sent = sentConfig()
    expect(sent.translation.contact_email).toBeNull()
    expect(sent.hotkey_profiles[0].deck_name).toBeNull()
    expect(sent.anki.field_mapping.audio_field).toBeNull()
  })

  it('does not call apply_settings while a field is invalid', async () => {
    const store = await loadedStore()
    onCommand('apply_settings', null)
    store.config.value.hotkey_profiles[0].shortcut = 'CommandOrControl+Shift+C'
    expect(store.issueFor('hotkey_profiles.0.shortcut')?.key).toBe(
      'validation.shortcutDuplicateDefault',
    )
    expect(store.hasIssuesUnder('hotkey_profiles')).toBe(true)
    expect(store.hasIssuesUnder('translation')).toBe(false)
    expect(await store.save()).toBe(false)
    expect(callsTo('apply_settings')).toEqual([])
  })

  it('keeps the edits and the backend message when apply_settings refuses', async () => {
    const store = await loadedStore()
    onCommand('apply_settings', () => {
      throw 'HotKey already registered: shift+super+KeyH'
    })
    store.config.value.hotkeys = 'CommandOrControl+Shift+K'
    expect(await store.save()).toBe(false)
    expect(store.saveError.value).toBe('HotKey already registered: shift+super+KeyH')
    expect(store.dirty.value).toBe(true)
    expect(store.config.value.hotkeys).toBe('CommandOrControl+Shift+K')
  })

  it('takes what the backend stored when it rejects after writing the file', async () => {
    const store = await loadedStore()
    onCommand('apply_settings', ({ newCfg }: { newCfg: AppConfig }) => {
      onCommand('get_current_settings', newCfg)
      throw 'The settings were saved, but some hotkeys are not active:\n"CommandOrControl+Shift+K" could not be registered'
    })
    store.config.value.hotkeys = 'CommandOrControl+Shift+K'
    expect(await store.save()).toBe(false)
    expect(store.saveError.value).toContain('some hotkeys are not active')
    expect(store.dirty.value).toBe(false)
    expect(store.saving.value).toBe(false)

    store.revert()
    expect(store.config.value.hotkeys).toBe('CommandOrControl+Shift+K')
    onCommand('apply_settings', null)
    expect(await store.persistUiLang('en')).toBe(true)
    expect(sentConfig(1).hotkeys).toBe('CommandOrControl+Shift+K')
  })
})

describe('dirty tracking', () => {
  it('turns dirty on a change and clean again when it is undone or reverted', async () => {
    const store = await loadedStore()
    store.config.value.anki.deck_name = 'Other'
    expect(store.dirty.value).toBe(true)
    store.config.value.anki.deck_name = 'English::AutomatedVocabulary'
    expect(store.dirty.value).toBe(false)
    store.config.value.translation.max_chars = 50
    store.revert()
    expect(store.config.value.translation.max_chars).toBe(100)
    expect(store.dirty.value).toBe(false)
  })

  it('ignores edits a save would normalize away', async () => {
    const store = await loadedStore()
    store.config.value.hotkeys = '  CommandOrControl+Shift+C '
    store.config.value.translation.contact_email = ''
    expect(store.dirty.value).toBe(false)
  })
})

describe('persistUiLang', () => {
  it('saves the language on top of the saved config, leaving unsaved edits out', async () => {
    const store = await loadedStore()
    onCommand('apply_settings', null)
    store.config.value.hotkeys = 'Ctrl+'
    expect(await store.persistUiLang('en')).toBe(true)
    const sent = sentConfig()
    expect(sent.ui_lang).toBe('en')
    expect(sent.hotkeys).toBe('CommandOrControl+Shift+C')
    expect(store.config.value.ui_lang).toBe('en')
    expect(store.config.value.hotkeys).toBe('Ctrl+')
    expect(store.dirty.value).toBe(true)
    store.revert()
    expect(store.config.value.ui_lang).toBe('en')
  })

  it('stays dirty and reports the error when the language cannot be saved', async () => {
    const store = await loadedStore()
    onCommand('apply_settings', () => {
      throw 'disk full'
    })
    expect(await store.persistUiLang('en')).toBe(false)
    expect(store.uiLangError.value).toBe('disk full')
    expect(store.dirty.value).toBe(true)
  })

  it('is not left dirty when the backend stored the language but a hotkey stayed inactive', async () => {
    const store = await loadedStore()
    onCommand('apply_settings', ({ newCfg }: { newCfg: AppConfig }) => {
      onCommand('get_current_settings', newCfg)
      throw 'The settings were saved, but some hotkeys are not active'
    })
    expect(await store.persistUiLang('en')).toBe(false)
    expect(store.uiLangError.value).toContain('some hotkeys are not active')
    expect(store.saved.value.ui_lang).toBe('en')
    expect(store.dirty.value).toBe(false)
  })
})

describe('setConsent', () => {
  it('keeps the granted consent without dirtying the form, so a later save cannot revert it', async () => {
    const store = await loadedStore()
    onCommand('apply_settings', null)
    const granted = {
      anki_install_granted_at: null,
      ankiconnect_provision_granted_at: '2026-10-31T12:00:00.000Z',
    }
    store.setConsent(granted)
    expect(store.dirty.value).toBe(false)
    store.config.value.anki.deck_name = 'Other'
    await store.save()
    expect(sentConfig().consent).toEqual(granted)
  })
})
