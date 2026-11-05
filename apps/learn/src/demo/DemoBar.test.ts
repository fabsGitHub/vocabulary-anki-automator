/**
 * The live demo's bar and root (docs/live-demo.md): what the bar says and
 * does, in both UI languages, and that the root mounts Learn afresh
 * whenever the demo's clock moves or the demo starts over.
 */

import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import { setLocale } from '../i18n'
import { fakePlatform, mountWith } from '../test/fakePlatform'
import DemoBar from './DemoBar.vue'
import DemoRoot from './DemoRoot.vue'
import { DEMO_KEY, type DemoController } from './demo'
import { demoMessages } from './messages'

/** Noon UTC on 2026-10-31: the same date in nearly every time zone the tests may run in. */
const NOW = Date.UTC(2026, 9, 31, 12, 0)

/**
 * The demo's components, as text: they look up the demo's own messages
 * (its entry point shows `preparing` directly, and the app's startup error).
 */
const DEMO_SOURCES = Object.values(
  import.meta.glob('./*.vue', { query: '?raw', import: 'default', eager: true }) as Record<
    string,
    string
  >,
).join('\n')

function controller(overrides: Partial<DemoController> = {}): DemoController {
  return {
    epoch: ref(0),
    dayShift: ref(0),
    persistent: true,
    busy: ref(false),
    error: ref(null),
    now: () => NOW,
    nextDay: vi.fn(async () => undefined),
    reset: vi.fn(async () => undefined),
    ...overrides,
  }
}

let wrapper: VueWrapper | null = null

function mountBar(demo: DemoController): VueWrapper {
  wrapper = mountWith(
    DemoBar,
    { platform: fakePlatform({ kind: 'web', tts: false, anki: false }) },
    { global: { provide: { [DEMO_KEY]: demo } } },
  )
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe("the demo's bar", () => {
  it('says what the demo is and offers its two actions', async () => {
    const demo = controller()
    mountBar(demo)

    expect(find('[data-test="demo-bar"]').attributes('aria-label')).toBe('Live demo')
    expect(find('[data-test="demo-summary"]').text()).toBe(demoMessages.en.summary)
    // Today is the real today: no date to point out.
    expect(find('[data-test="demo-date"]').exists()).toBe(false)
    expect(find('[data-test="demo-project-link"]').exists()).toBe(false)
    expect(find('details').text()).toContain('golden test vectors')

    await find('[data-test="demo-next-day"]').trigger('click')
    expect(demo.nextDay).toHaveBeenCalledTimes(1)
    await find('[data-test="demo-start-over"]').trigger('click')
    expect(demo.reset).toHaveBeenCalledTimes(1)
  })

  it("shows the demo's date once the clock has moved", () => {
    mountBar(controller({ dayShift: ref(2), now: () => NOW + 2 * 86_400_000 }))
    expect(find('[data-test="demo-date"]').text()).toBe('Demo date: Mon, Nov 2')
  })

  it('says when nothing is kept, and what failed', () => {
    mountBar(controller({ persistent: false, error: ref('the disk is full') }))
    expect(find('[data-test="demo-summary"]').text()).toBe(demoMessages.en.summaryMemory)
    expect(find('[data-test="demo-error"]').text()).toBe('That did not work: the disk is full')
  })

  it('marks its actions while one runs', () => {
    mountBar(controller({ busy: ref(true) }))
    for (const action of ['demo-next-day', 'demo-start-over']) {
      expect(find(`[data-test="${action}"]`).attributes('aria-disabled')).toBe('true')
    }
  })

  it('has a message for every key it asks for', () => {
    const used = [...DEMO_SOURCES.matchAll(/\bt\(\s*'([^']+)'/g)].map((match) => match[1])
    expect(used.length).toBeGreaterThan(8)
    expect(used.filter((key) => !(key in demoMessages.en))).toEqual([])
  })

  it('speaks German, with the same messages as English', async () => {
    expect(Object.keys(demoMessages.de)).toEqual(Object.keys(demoMessages.en))
    for (const key of Object.keys(demoMessages.en) as (keyof typeof demoMessages.en)[]) {
      expect(demoMessages.de[key], key).not.toBe(demoMessages.en[key])
    }
    setLocale('de')
    mountBar(controller({ dayShift: ref(1), now: () => NOW + 86_400_000 }))
    await flushPromises()
    expect(find('[data-test="demo-next-day"]').text()).toBe('Nächster Tag')
    expect(find('[data-test="demo-date"]').text()).toBe('Demo-Datum: So., 1. Nov.')
  })
})

describe("the demo's root", () => {
  it('shows the bar above the app and mounts the app afresh on every move', async () => {
    const demo = controller()
    const platform = fakePlatform({ kind: 'web', tts: false, anki: false })
    wrapper = mountWith(DemoRoot, { platform }, { global: { provide: { [DEMO_KEY]: demo } } })
    await flushPromises()

    const body = find('[data-test="app-body"]')
    expect(body.element.firstElementChild?.getAttribute('data-test')).toBe('demo-bar')
    expect(platform.getDailyPromptState).toHaveBeenCalledTimes(1)

    demo.epoch.value += 1
    await flushPromises()
    expect(platform.getDailyPromptState).toHaveBeenCalledTimes(2)
    expect(find('[data-test="demo-bar"]').exists()).toBe(true)
  })
})
