import { invoke } from '@tauri-apps/api/core'
import { describe, expect, it, vi } from 'vitest'

import { getPlatform } from './index'
import { TauriAdapter } from './tauri'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }))

describe('getPlatform', () => {
  it('is the Tauri adapter in the desktop build', async () => {
    expect(__LEARN_PLATFORM__).toBe('tauri')
    const platform = await getPlatform()
    expect(platform).toBeInstanceOf(TauriAdapter)
    expect(platform.capabilities.kind).toBe('tauri')
    await platform.getDailyQueue()
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('get_daily_queue')
  })
})
