import { check, type Update } from '@tauri-apps/plugin-updater'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n, setLocale } from '../i18n'
import { callsTo, commandNames, onCommand, resetTauri } from '../test/fakeTauri'
import UpdateBanner from './UpdateBanner.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)
vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }))

const IDLE_MS = 10_000

function fakeUpdate(version = '0.2.0') {
  return {
    version,
    downloadAndInstall: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  }
}

let wrapper: VueWrapper | null = null

function mountBanner(): VueWrapper {
  wrapper = mount(UpdateBanner, { global: { plugins: [i18n] } })
  return wrapper
}

const banner = () => wrapper?.find('[data-test="update-banner"]')

beforeEach(() => {
  resetTauri()
  setLocale('en')
  vi.mocked(check).mockReset()
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('when the check runs', () => {
  it('waits for the app to be idle for 10 s after start-up', async () => {
    onCommand('capture_in_progress', false)
    vi.mocked(check).mockResolvedValue(null)
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS - 1)
    expect(check).not.toHaveBeenCalled()
    expect(commandNames()).toEqual([])
    await vi.advanceTimersByTimeAsync(1)
    expect(check).toHaveBeenCalledTimes(1)
    expect(banner()?.exists()).toBe(false)
  })

  it('never checks while a capture is in progress, and retries later', async () => {
    let capturing = true
    onCommand('capture_in_progress', () => capturing)
    vi.mocked(check).mockResolvedValue(null)
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS)
    expect(callsTo('capture_in_progress')).toHaveLength(1)
    expect(check).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(IDLE_MS)
    expect(callsTo('capture_in_progress')).toHaveLength(2)
    expect(check).not.toHaveBeenCalled()

    capturing = false
    await vi.advanceTimersByTimeAsync(IDLE_MS)
    expect(check).toHaveBeenCalledTimes(1)
  })

  it('treats an unanswerable capture_in_progress as busy', async () => {
    onCommand('capture_in_progress', () => {
      throw 'not available'
    })
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    vi.mocked(check).mockResolvedValue(null)
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS * 10)
    expect(check).not.toHaveBeenCalled()
  })

  it('does nothing once the window is gone', async () => {
    onCommand('capture_in_progress', false)
    mountBanner()
    wrapper?.unmount()
    wrapper = null
    await vi.advanceTimersByTimeAsync(IDLE_MS * 2)
    expect(commandNames()).toEqual([])
    expect(check).not.toHaveBeenCalled()
  })
})

describe('an available update', () => {
  it('shows a banner and installs, then restarts, only after the click', async () => {
    const update = fakeUpdate('0.2.0')
    const order: string[] = []
    update.downloadAndInstall.mockImplementation(async () => {
      order.push('downloadAndInstall')
    })
    onCommand('capture_in_progress', false)
    onCommand('restart_app', () => {
      order.push('restart_app')
      return new Promise(() => {})
    })
    vi.mocked(check).mockResolvedValue(update as unknown as Update)
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS)

    expect(banner()?.text()).toContain('Version 0.2.0 is available – restart to update.')
    await vi.advanceTimersByTimeAsync(IDLE_MS * 10)
    expect(order).toEqual([])

    await wrapper?.find('[data-test="update-install"]').trigger('click')
    await flushPromises()
    expect(order).toEqual(['downloadAndInstall', 'restart_app'])
    expect(banner()?.text()).toContain('Updating…')
  })

  it('can be dismissed, which releases the update', async () => {
    const update = fakeUpdate()
    onCommand('capture_in_progress', false)
    vi.mocked(check).mockResolvedValue(update as unknown as Update)
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS)
    await wrapper?.find('[data-test="update-dismiss"]').trigger('click')
    expect(banner()?.exists()).toBe(false)
    expect(update.close).toHaveBeenCalledTimes(1)
    expect(update.downloadAndInstall).not.toHaveBeenCalled()
    expect(callsTo('restart_app')).toHaveLength(0)
  })

  it('refuses to restart in the middle of a capture', async () => {
    const update = fakeUpdate()
    let capturing = false
    onCommand('capture_in_progress', () => capturing)
    onCommand('restart_app', () => new Promise(() => {}))
    vi.mocked(check).mockResolvedValue(update as unknown as Update)
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS)
    capturing = true
    await wrapper?.find('[data-test="update-install"]').trigger('click')
    await flushPromises()
    expect(update.downloadAndInstall).not.toHaveBeenCalled()
    expect(callsTo('restart_app')).toHaveLength(0)
    expect(banner()?.text()).toContain('Finish the current capture first')
  })
})

describe('failures', () => {
  it('stay silent when the check fails: logged, no banner, no dialog', async () => {
    onCommand('capture_in_progress', false)
    vi.mocked(check).mockRejectedValue(new Error('Could not fetch a valid release JSON'))
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => {})
    const confirm = vi.spyOn(window, 'confirm').mockImplementation(() => true)
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS)
    expect(error).toHaveBeenCalled()
    expect(banner()?.exists()).toBe(false)
    expect(alert).not.toHaveBeenCalled()
    expect(confirm).not.toHaveBeenCalled()
    expect(callsTo('restart_app')).toHaveLength(0)
  })

  it('never restart after a failed install, and allow another try', async () => {
    const update = fakeUpdate()
    update.downloadAndInstall.mockRejectedValue(new Error('signature verification failed'))
    onCommand('capture_in_progress', false)
    onCommand('restart_app', () => new Promise(() => {}))
    vi.mocked(check).mockResolvedValue(update as unknown as Update)
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => {})
    mountBanner()
    await vi.advanceTimersByTimeAsync(IDLE_MS)
    await wrapper?.find('[data-test="update-install"]').trigger('click')
    await flushPromises()
    expect(error).toHaveBeenCalled()
    expect(alert).not.toHaveBeenCalled()
    expect(callsTo('restart_app')).toHaveLength(0)
    expect(banner()?.text()).toContain('could not be installed')
    expect(
      (wrapper?.find('[data-test="update-install"]').element as HTMLButtonElement).disabled,
    ).toBe(false)
  })
})
