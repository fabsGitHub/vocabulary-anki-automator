import { check } from '@tauri-apps/plugin-updater'
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import App from './App.vue'
import { defaultConfig } from './composables/useConfig'
import { i18n, setLocale } from './i18n'
import { callsTo, onCommand, resetTauri, windowState } from './test/fakeTauri'

vi.mock('@tauri-apps/api/core', async () => (await import('./test/fakeTauri')).coreModule)
vi.mock('@tauri-apps/api/event', async () => (await import('./test/fakeTauri')).eventModule)
vi.mock('@tauri-apps/api/window', async () => (await import('./test/fakeTauri')).windowModule)
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn(async () => null) }))

beforeEach(() => {
  resetTauri()
  setLocale('en')
  vi.mocked(check).mockClear()
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  onCommand('get_current_settings', defaultConfig())
  onCommand('get_pending_payload', null)
  onCommand('anki_status', { online: false, version: null, decks: [], models: [], error: null })
  onCommand('get_setup_status', {
    anki_installed: true,
    ankiconnect_installed: true,
    platform_supported: true,
    anki_version: '24.06.3',
    consent: { anki_install_granted_at: null, ankiconnect_provision_granted_at: null },
  })
  onCommand('capture_in_progress', false)
})

afterEach(() => vi.useRealTimers())

describe('App', () => {
  it('renders the approval window in the preview window, without update checks', async () => {
    windowState.label = 'preview'
    const wrapper = mount(App, { global: { plugins: [i18n] } })
    await flushPromises()
    expect(wrapper.find('[data-test="waiting"]').exists()).toBe(true)
    expect(wrapper.find('[role="tablist"]').exists()).toBe(false)
    expect(callsTo('get_pending_payload')).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(check).not.toHaveBeenCalled()
    expect(callsTo('capture_in_progress')).toHaveLength(0)
    wrapper.unmount()
  })

  it('renders the settings in the main window', async () => {
    windowState.label = 'main'
    const wrapper = mount(App, { global: { plugins: [i18n] } })
    await flushPromises()
    expect(wrapper.find('[role="tablist"]').exists()).toBe(true)
    expect(callsTo('get_pending_payload')).toHaveLength(0)
    wrapper.unmount()
  })
})
