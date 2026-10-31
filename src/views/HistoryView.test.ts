import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { Card, StoredCapture, SyncReport } from '../api'
import { i18n, setLocale } from '../i18n'
import {
  callsTo,
  deferred,
  emitEvent,
  listenerCount,
  onCommand,
  resetTauri,
} from '../test/fakeTauri'
import HistoryView from './HistoryView.vue'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)
vi.mock('@tauri-apps/api/event', async () => (await import('../test/fakeTauri')).eventModule)

function card(overrides: Partial<Card>): Card {
  return {
    front: '',
    back: '',
    example_source: '',
    example_target: '',
    notes: '',
    tags: [],
    source_lang: 'en',
    target_lang: 'de',
    captured_at: '2026-10-31T12:00:00.000Z',
    audio_front: null,
    script_metadata: { kind: 'none' },
    ...overrides,
  }
}

const CHINESE: StoredCapture = {
  id: 2,
  card: card({
    front: '学习',
    back: 'lernen',
    source_lang: 'zh-CN',
    captured_at: '2026-10-31T12:30:00.000Z',
    script_metadata: {
      kind: 'chinese',
      pinyin: 'xué xí',
      hsk_level: 1,
      script_variant: 'simplified',
    },
  }),
  origin: 'capture',
  synced_to_anki: false,
}

const ENGLISH: StoredCapture = {
  id: 1,
  card: card({
    front: 'serendipity',
    back: 'glücklicher Zufall',
    captured_at: '2026-10-30T08:00:00Z',
  }),
  origin: 'anki_import',
  synced_to_anki: true,
}

let wrapper: VueWrapper | null = null

async function mountHistory(): Promise<VueWrapper> {
  wrapper = mount(HistoryView, { global: { plugins: [i18n] } })
  await flushPromises()
  return wrapper
}

const items = () => wrapper?.findAll('[data-test="history-item"]') ?? []

beforeEach(() => {
  resetTauri()
  setLocale('en')
  onCommand('list_recent_captures', [CHINESE, ENGLISH])
})

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('the list', () => {
  it('shows the latest 50 captures with front, back, date, origin and sync state', async () => {
    await mountHistory()
    expect(callsTo('list_recent_captures').map((call) => call.args)).toEqual([{ limit: 50 }])
    expect(items()).toHaveLength(2)

    const [chinese, english] = items()
    expect(chinese.find('[data-test="front"]').text()).toBe('学习')
    expect(chinese.find('[data-test="back"]').text()).toBe('lernen')
    expect(chinese.text()).toContain('xué xí')
    expect(chinese.find('[data-test="origin"]').text()).toBe('Capture')
    expect(chinese.find('[data-test="synced"]').text()).toBe('Not in Anki yet')
    expect(chinese.find('[data-test="date"]').attributes('datetime')).toBe(
      '2026-10-31T12:30:00.000Z',
    )
    expect(chinese.find('[data-test="date"]').text()).toContain('2026')

    expect(english.find('[data-test="front"]').text()).toBe('serendipity')
    expect(english.find('[data-test="origin"]').text()).toBe('Anki import')
    expect(english.find('[data-test="synced"]').text()).toBe('In Anki')
  })

  it('says so when there is nothing yet', async () => {
    onCommand('list_recent_captures', [])
    await mountHistory()
    expect(wrapper?.find('[data-test="history-empty"]').exists()).toBe(true)
  })

  it('shows why the list could not be loaded', async () => {
    onCommand('list_recent_captures', () => {
      throw 'database is locked'
    })
    await mountHistory()
    expect(wrapper?.find('[data-test="history-error"]').text()).toContain('database is locked')
  })

  it('reloads when the backend reports captures-changed', async () => {
    onCommand('list_recent_captures', [])
    await mountHistory()
    expect(items()).toHaveLength(0)

    onCommand('list_recent_captures', [CHINESE])
    emitEvent('captures-changed')
    await flushPromises()
    expect(items()).toHaveLength(1)
    expect(callsTo('list_recent_captures')).toHaveLength(2)
  })

  it('keeps the newest answer when an older reload finishes last', async () => {
    await mountHistory()
    const slow = deferred<StoredCapture[]>()
    onCommand('list_recent_captures', () => slow.promise)
    emitEvent('captures-changed')
    onCommand('list_recent_captures', [ENGLISH])
    emitEvent('captures-changed')
    await flushPromises()
    slow.resolve([CHINESE, ENGLISH])
    await flushPromises()
    expect(items()).toHaveLength(1)
  })

  it('stops listening when unmounted', async () => {
    await mountHistory()
    expect(listenerCount('captures-changed')).toBe(1)
    wrapper?.unmount()
    wrapper = null
    expect(listenerCount('captures-changed')).toBe(0)
  })
})

describe('deleting', () => {
  it('asks first, and cancelling deletes nothing', async () => {
    onCommand('delete_capture', true)
    await mountHistory()
    await items()[0].find('[data-test="delete"]').trigger('click')
    expect(items()[0].find('[data-test="confirm-text"]').text()).toContain(
      'Delete “学习” from the history?',
    )
    expect(callsTo('delete_capture')).toHaveLength(0)

    await items()[0].find('[data-test="cancel-delete"]').trigger('click')
    expect(items()[0].find('[data-test="confirm-text"]').exists()).toBe(false)
    expect(callsTo('delete_capture')).toHaveLength(0)
  })

  it('deletes the confirmed capture by id and reloads', async () => {
    onCommand('delete_capture', () => {
      onCommand('list_recent_captures', [ENGLISH])
      return true
    })
    await mountHistory()
    await items()[0].find('[data-test="delete"]').trigger('click')
    await items()[0].find('[data-test="confirm-delete"]').trigger('click')
    await flushPromises()
    expect(callsTo('delete_capture').map((call) => call.args)).toEqual([{ id: 2 }])
    expect(items()).toHaveLength(1)
    expect(items()[0].find('[data-test="front"]').text()).toBe('serendipity')
  })

  it('reports a capture that was already gone', async () => {
    onCommand('delete_capture', false)
    await mountHistory()
    await items()[1].find('[data-test="delete"]').trigger('click')
    await items()[1].find('[data-test="confirm-delete"]').trigger('click')
    await flushPromises()
    expect(wrapper?.find('[data-test="history-notice"]').text()).toContain('already been deleted')
  })
})

describe('syncing pending captures', () => {
  it('pushes them and summarizes the report, naming what failed', async () => {
    const report: SyncReport = {
      attempted: 2,
      synced: 1,
      failed: [{ id: 2, error: 'could not reach AnkiConnect' }],
    }
    onCommand('sync_pending_captures', report)
    await mountHistory()
    await wrapper?.find('[data-test="sync"]').trigger('click')
    await flushPromises()
    const summary = wrapper?.find('[data-test="sync-report"]').text() ?? ''
    expect(summary).toContain('Synced 1 of 2 cards.')
    expect(summary).toContain('学习: could not reach AnkiConnect')
    expect(callsTo('sync_pending_captures')).toHaveLength(1)
    expect(callsTo('list_recent_captures')).toHaveLength(2)
  })

  it('says when there was nothing to sync', async () => {
    onCommand('sync_pending_captures', { attempted: 0, synced: 0, failed: [] })
    await mountHistory()
    await wrapper?.find('[data-test="sync"]').trigger('click')
    await flushPromises()
    expect(wrapper?.find('[data-test="sync-report"]').text()).toContain('Nothing to sync')
  })

  it('shows a failed sync', async () => {
    onCommand('sync_pending_captures', () => {
      throw 'store unavailable'
    })
    await mountHistory()
    await wrapper?.find('[data-test="sync"]').trigger('click')
    await flushPromises()
    expect(wrapper?.find('[data-test="history-notice"]').text()).toContain('store unavailable')
  })

  it('runs one sync at a time', async () => {
    const pending = deferred<SyncReport>()
    onCommand('sync_pending_captures', () => pending.promise)
    await mountHistory()
    await wrapper?.find('[data-test="sync"]').trigger('click')
    await wrapper?.find('[data-test="sync"]').trigger('click')
    expect(callsTo('sync_pending_captures')).toHaveLength(1)
    pending.resolve({ attempted: 1, synced: 1, failed: [] })
    await flushPromises()
  })
})
