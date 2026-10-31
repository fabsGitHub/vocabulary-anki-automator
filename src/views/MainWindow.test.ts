import { check } from '@tauri-apps/plugin-updater'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AnkiStatus, AppConfig } from '../api'
import { defaultConfig } from '../composables/useConfig'
import { i18n, setLocale } from '../i18n'
import { callsTo, emitEvent, onCommand, resetTauri } from '../test/fakeTauri'
import MainWindow from './MainWindow.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)
vi.mock('@tauri-apps/api/event', async () => (await import('../test/fakeTauri')).eventModule)
vi.mock('@tauri-apps/api/window', async () => (await import('../test/fakeTauri')).windowModule)
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn(async () => null) }))

const ONLINE: AnkiStatus = {
  online: true,
  version: 6,
  decks: ['Default'],
  models: ['AppleStyleFinalV3'],
  error: null,
}

function settings(edit: (config: AppConfig) => void = () => {}): AppConfig {
  const config = defaultConfig()
  edit(config)
  return config
}

let wrapper: VueWrapper | null = null

async function mountMain(): Promise<VueWrapper> {
  wrapper = mount(MainWindow, { global: { plugins: [i18n] } })
  await flushPromises()
  return wrapper
}

const tab = (id: string) => wrapper?.find(`[data-test="tab-${id}"]`)
const sentConfigs = () => callsTo('apply_settings').map((call) => call.args?.newCfg as AppConfig)

beforeEach(() => {
  resetTauri()
  setLocale('en')
  onCommand('get_current_settings', settings())
  onCommand('anki_status', ONLINE)
  onCommand('anki_model_field_names', ['Front', 'Back', 'Context Sentence', 'Notes', 'Front Audio'])
  onCommand('apply_settings', null)
  onCommand('list_recent_captures', [])
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('MainWindow', () => {
  it('loads the settings and only checks Anki on start-up', async () => {
    await mountMain()
    expect(callsTo('get_current_settings')).toHaveLength(1)
    expect(callsTo('anki_status').length).toBeGreaterThanOrEqual(1)
    for (const forbidden of [
      'download_and_install_anki',
      'auto_provision_ankiconnect',
      'launch_anki_desktop',
    ]) {
      expect(callsTo(forbidden)).toHaveLength(0)
    }
    expect(wrapper?.find('[data-test="anki-badge"]').text()).toContain('Anki connected')
    expect(wrapper?.findAll('[role="tab"]').map((t) => t.text())).toEqual([
      'Capture & Language',
      'Hotkey profiles',
      'Anki connection',
      'History',
    ])
  })

  it('shows the UI in the stored ui_lang', async () => {
    onCommand(
      'get_current_settings',
      settings((config) => (config.ui_lang = 'de')),
    )
    await mountMain()
    expect(tab('capture')?.text()).toBe('Erfassung & Sprache')
    expect(wrapper?.find('[data-test="save"]').text()).toBe('Einstellungen speichern')
  })

  it('persists a UI-language switch at once and re-renders in that language', async () => {
    await mountMain()
    await wrapper?.find('[data-test="ui-lang"]').setValue('de')
    await flushPromises()
    expect(sentConfigs().map((config) => config.ui_lang)).toEqual(['de'])
    expect(tab('history')?.text()).toBe('Verlauf')
    expect(wrapper?.find('[data-test="save"]').attributes('disabled')).toBeDefined()
  })

  it('saves edits from any settings tab with one button', async () => {
    await mountMain()
    const save = wrapper?.find('[data-test="save"]')
    expect(save?.attributes('disabled')).toBeDefined()
    await wrapper?.find('#contact-email').setValue('me@example.com')
    expect(save?.attributes('disabled')).toBeUndefined()
    await save?.trigger('click')
    await flushPromises()
    expect(sentConfigs()).toHaveLength(1)
    expect(sentConfigs()[0].translation.contact_email).toBe('me@example.com')
    expect(wrapper?.text()).toContain('Settings saved.')
  })

  it('blocks saving invalid settings and marks the tab that has them', async () => {
    await mountMain()
    await wrapper?.find('#hotkey').setValue('Ctrl+')
    expect(wrapper?.find('[data-test="hotkey-error"]').text()).toContain('Not a valid shortcut')
    expect(tab('capture')?.find('.tab-issue-dot').exists()).toBe(true)
    expect(tab('profiles')?.find('.tab-issue-dot').exists()).toBe(false)
    await wrapper?.find('[data-test="save"]').trigger('click')
    await flushPromises()
    expect(callsTo('apply_settings')).toHaveLength(0)
    expect(wrapper?.text()).toContain('Fix the highlighted settings before saving.')
  })

  it('shows the backend error when saving fails', async () => {
    onCommand('apply_settings', () => {
      throw 'HotKey already registered'
    })
    await mountMain()
    await wrapper?.find('#hotkey').setValue('CommandOrControl+Shift+K')
    await wrapper?.find('[data-test="save"]').trigger('click')
    await flushPromises()
    expect(wrapper?.text()).toContain('Saving failed: HotKey already registered')
  })

  it('offers a retry instead of the form when the settings cannot be loaded', async () => {
    onCommand('get_current_settings', () => {
      throw 'config.json unreadable'
    })
    await mountMain()
    expect(wrapper?.find('[data-test="load-error"]').text()).toContain('config.json unreadable')
    expect(wrapper?.find('[data-test="save"]').exists()).toBe(false)
    await tab('history')?.trigger('click')
    await flushPromises()
    expect(callsTo('list_recent_captures')).toHaveLength(1)
  })

  it('shows capture errors', async () => {
    await mountMain()
    emitEvent('capture-error', { message: 'nothing to capture: the text is empty' })
    await flushPromises()
    expect(wrapper?.find('[data-test="capture-error"]').text()).toContain('the text is empty')
  })

  it('opens and closes the help', async () => {
    await mountMain()
    await wrapper?.find('[data-test="help"]').trigger('click')
    expect(wrapper?.text()).toContain('2055492159')
    await wrapper?.find('[role="dialog"]').trigger('keydown', { key: 'Escape' })
    expect(wrapper?.find('[role="dialog"]').exists()).toBe(false)
  })

  it('runs a test capture through capture_text', async () => {
    onCommand(
      'get_current_settings',
      settings((config) =>
        config.hotkey_profiles.push({
          id: 'zh-de',
          label: 'Chinese → German',
          shortcut: 'CommandOrControl+Shift+H',
          source_lang: 'zh-CN',
          target_lang: 'de',
          deck_name: null,
        }),
      ),
    )
    onCommand('capture_text', null)
    await mountMain()
    await wrapper?.find('[data-test="try-text"]').setValue('  serendipity ')
    await wrapper?.find('[data-test="try-capture"]').trigger('click')
    await flushPromises()
    await wrapper?.find('select[aria-label="Hotkey"]').setValue('zh-de')
    await wrapper?.find('[data-test="try-text"]').setValue('学习')
    await wrapper?.find('[data-test="try-text"]').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(callsTo('capture_text').map((call) => call.args)).toEqual([
      { text: 'serendipity', profileId: null },
      { text: '学习', profileId: 'zh-de' },
    ])
    expect(wrapper?.text()).toContain('The approval window opens when the card is ready.')
  })

  it('hosts the update banner', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    onCommand('capture_in_progress', false)
    try {
      await mountMain()
      await vi.advanceTimersByTimeAsync(10_000)
      expect(check).toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })
})
