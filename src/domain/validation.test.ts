import { describe, expect, it } from 'vitest'

import type { AppConfig, HotkeyProfile } from '../api'
import { defaultConfig } from '../composables/useConfig'
import { MAX_CHARS_LIMIT, validateConfig, type ConfigIssue } from './validation'

function profile(overrides: Partial<HotkeyProfile> = {}): HotkeyProfile {
  return {
    id: 'zh-de',
    label: 'Chinese → German',
    shortcut: 'CommandOrControl+Shift+H',
    source_lang: 'zh-CN',
    target_lang: 'de',
    deck_name: null,
    ...overrides,
  }
}

function issuesOf(edit: (config: AppConfig) => void, isMac = false): ConfigIssue[] {
  const config = defaultConfig()
  edit(config)
  return validateConfig(config, isMac)
}

const at = (issues: ConfigIssue[], path: string) => issues.filter((issue) => issue.path === path)

describe('validateConfig', () => {
  it('accepts the defaults and a Chinese profile', () => {
    expect(validateConfig(defaultConfig(), false)).toEqual([])
    expect(issuesOf((c) => c.hotkey_profiles.push(profile()))).toEqual([])
  })

  it('flags a shortcut the backend could not parse', () => {
    expect(
      at(
        issuesOf((c) => (c.hotkeys = 'Ctrl+Shift')),
        'hotkeys',
      ),
    ).toEqual([{ path: 'hotkeys', key: 'validation.shortcutInvalid' }])
    expect(
      at(
        issuesOf((c) => (c.hotkeys = '  ')),
        'hotkeys',
      ),
    ).toEqual([{ path: 'hotkeys', key: 'validation.required' }])
  })

  it('refuses shortcuts that would swallow normal typing, as the backend does', () => {
    for (const shortcut of ['C', 'Shift+C', 'shift+KeyQ', 'Space', 'Shift+Up']) {
      expect(
        at(
          issuesOf((c) => (c.hotkeys = shortcut)),
          'hotkeys',
        ),
        shortcut,
      ).toEqual([{ path: 'hotkeys', key: 'validation.shortcutNeedsModifier' }])
    }
    for (const shortcut of ['F8', 'Shift+F12', 'Alt+C', 'Ctrl+C', 'Super+C', 'Option+Space']) {
      expect(
        at(
          issuesOf((c) => (c.hotkeys = shortcut)),
          'hotkeys',
        ),
        shortcut,
      ).toEqual([])
    }
    const profileIssues = issuesOf((c) => c.hotkey_profiles.push(profile({ shortcut: 'Shift+H' })))
    expect(at(profileIssues, 'hotkey_profiles.0.shortcut')).toEqual([
      { path: 'hotkey_profiles.0.shortcut', key: 'validation.shortcutNeedsModifier' },
    ])
  })

  it('flags a profile reusing the default hotkey, however it is spelled', () => {
    const issues = issuesOf((c) => c.hotkey_profiles.push(profile({ shortcut: 'shift+ctrl+KeyC' })))
    expect(at(issues, 'hotkey_profiles.0.shortcut')).toEqual([
      { path: 'hotkey_profiles.0.shortcut', key: 'validation.shortcutDuplicateDefault' },
    ])
  })

  it('compares CommandOrControl the way the current platform resolves it', () => {
    const cmd = (c: AppConfig) => c.hotkey_profiles.push(profile({ shortcut: 'Command+Shift+C' }))
    expect(at(issuesOf(cmd, true), 'hotkey_profiles.0.shortcut')).toHaveLength(1)
    expect(at(issuesOf(cmd, false), 'hotkey_profiles.0.shortcut')).toHaveLength(0)
  })

  it('names the profile that already uses a shortcut', () => {
    const issues = issuesOf((c) => {
      c.hotkey_profiles.push(profile({ label: 'First' }))
      c.hotkey_profiles.push(profile({ id: 'second', label: 'Second' }))
    })
    expect(at(issues, 'hotkey_profiles.1.shortcut')).toEqual([
      {
        path: 'hotkey_profiles.1.shortcut',
        key: 'validation.shortcutDuplicate',
        params: { other: 'First' },
      },
    ])
    expect(at(issues, 'hotkey_profiles.0.shortcut')).toEqual([])
  })

  it('accepts every supported language, zh-CN included, and nothing else', () => {
    expect(issuesOf((c) => (c.translation.source_lang = 'zh-CN'))).toEqual([])
    expect(
      at(
        issuesOf((c) => (c.translation.source_lang = 'pt')),
        'translation.source_lang',
      ),
    ).toEqual([{ path: 'translation.source_lang', key: 'validation.languageUnsupported' }])
    expect(
      at(
        issuesOf((c) => c.hotkey_profiles.push(profile({ target_lang: 'ja' }))),
        'hotkey_profiles.0.target_lang',
      ),
    ).toEqual([{ path: 'hotkey_profiles.0.target_lang', key: 'validation.languageUnsupported' }])
  })

  it('refuses a pair that translates into itself', () => {
    expect(
      at(
        issuesOf((c) => (c.translation.target_lang = 'en')),
        'translation.target_lang',
      ),
    ).toEqual([{ path: 'translation.target_lang', key: 'validation.languagesEqual' }])
  })

  it.each([0, -1, 2.5, MAX_CHARS_LIMIT + 1, Number.NaN])('rejects max_chars %s', (value) => {
    expect(
      at(
        issuesOf((c) => (c.translation.max_chars = value)),
        'translation.max_chars',
      ),
    ).toEqual([
      {
        path: 'translation.max_chars',
        key: 'validation.maxCharsRange',
        params: { max: MAX_CHARS_LIMIT },
      },
    ])
  })

  it.each([1, 100, MAX_CHARS_LIMIT])('accepts max_chars %s', (value) => {
    expect(
      at(
        issuesOf((c) => (c.translation.max_chars = value)),
        'translation.max_chars',
      ),
    ).toEqual([])
  })

  it('treats the contact e-mail as optional but checks its shape', () => {
    expect(issuesOf((c) => (c.translation.contact_email = null))).toEqual([])
    expect(issuesOf((c) => (c.translation.contact_email = '   '))).toEqual([])
    expect(issuesOf((c) => (c.translation.contact_email = 'me@example.com'))).toEqual([])
    expect(
      at(
        issuesOf((c) => (c.translation.contact_email = 'me@')),
        'translation.contact_email',
      ),
    ).toEqual([{ path: 'translation.contact_email', key: 'validation.emailInvalid' }])
  })

  it('requires profile ids to be present and unique, and labels to be present', () => {
    const issues = issuesOf((c) => {
      c.hotkey_profiles.push(profile({ shortcut: 'Alt+1' }))
      c.hotkey_profiles.push(profile({ shortcut: 'Alt+2', label: ' ' }))
      c.hotkey_profiles.push(profile({ shortcut: 'Alt+3', id: '' }))
    })
    expect(at(issues, 'hotkey_profiles.1.id')).toEqual([
      { path: 'hotkey_profiles.1.id', key: 'validation.idDuplicate' },
    ])
    expect(at(issues, 'hotkey_profiles.1.label')).toEqual([
      { path: 'hotkey_profiles.1.label', key: 'validation.required' },
    ])
    expect(at(issues, 'hotkey_profiles.2.id')).toEqual([
      { path: 'hotkey_profiles.2.id', key: 'validation.required' },
    ])
  })

  it('requires a deck, a note type, a known provider and at least one mapped field', () => {
    const issues = issuesOf((c) => {
      c.anki.deck_name = ''
      c.anki.model_name = ' '
      c.translation.provider = 'deepl'
      c.anki.field_mapping = { fields: [], audio_field: 'Audio' }
    })
    expect(issues.map((issue) => issue.path)).toEqual([
      'translation.provider',
      'anki.deck_name',
      'anki.model_name',
      'anki.field_mapping',
    ])
  })
})
