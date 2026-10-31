import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import { setLocale } from '../i18n'
import { fakePlatform, mountWith, type FakePlatform } from '../test/fakePlatform'
import * as fx from '../test/fixtures'
import StatsView from './StatsView.vue'

let wrapper: VueWrapper | null = null

async function mountStats(platform: FakePlatform): Promise<VueWrapper> {
  wrapper = mountWith(StatsView, { platform })
  await flushPromises()
  return wrapper
}

const value = (name: string) => wrapper!.find(`[data-test="${name}"]`).text()

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('stats', () => {
  it('shows XP, the streak, freezes and the learning counts', async () => {
    const platform = fakePlatform()
    platform.getStats.mockResolvedValue(
      fx.stats({
        xp_total: 1250,
        xp_today: 35,
        streak: fx.streak({ current: 4, longest: 9, freezes_available: 2 }),
        learned_count: 42,
        due_today: 7,
      }),
    )
    await mountStats(platform)
    expect(platform.getStats).toHaveBeenCalledTimes(1)
    expect(value('xp-total')).toBe('1,250')
    expect(value('xp-today')).toBe('35')
    expect(value('streak-current')).toBe('4')
    expect(value('streak-longest')).toBe('9')
    expect(value('freezes')).toBe('2')
    expect(value('learned')).toBe('42')
    expect(value('due-today')).toBe('7')
    expect(value('today')).toBe('Today’s session is not complete yet.')
    expect(wrapper!.text()).toContain('Current streak (days)')
  })

  it('says when today’s session is complete, in the UI language', async () => {
    const platform = fakePlatform()
    platform.getStats.mockResolvedValue(
      fx.stats({ xp_total: 1250, streak: fx.streak({ completed_today: true }) }),
    )
    setLocale('de')
    await mountStats(platform)
    expect(value('today')).toBe('Die heutige Sitzung ist abgeschlossen.')
    expect(value('xp-total')).toBe('1.250')
  })

  it('refreshes on request and reports a failure', async () => {
    const platform = fakePlatform()
    await mountStats(platform)
    platform.getStats.mockRejectedValueOnce('database is locked')
    await wrapper!.find('[data-test="refresh"]').trigger('click')
    await flushPromises()
    expect(platform.getStats).toHaveBeenCalledTimes(2)
    expect(value('stats-error')).toContain('database is locked')
  })
})
