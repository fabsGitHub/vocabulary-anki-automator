/**
 * The IndexedDB backend (`./idb.ts`) against `__fixtures__/fakeIndexedDb.ts`,
 * which models the parts of IndexedDB idb.ts relies on: asynchronous
 * requests, auto-commit, atomic commits, synchronous key errors that leave
 * the transaction open, and version changes. A browser's IndexedDB is still
 * the real test (see the module docs of idb.ts); this one keeps the logic
 * on top of it from regressing.
 */

import { afterEach, describe, expect, it } from 'vitest'

import { FakeIndexedDb, FakeLockManager } from './__fixtures__/fakeIndexedDb'
import { WebAdapter } from './adapter'
import { openIndexedDbStorage } from './idb'
import type { CurriculumItem } from './model'
import { TABLES, type Storage } from './storage'
import { Store } from './store'
import { FixedClock, parseInstant } from './time'

function open(factory: FakeIndexedDb, name = 'learn'): Promise<Storage> {
  return openIndexedDbStorage(name, factory as unknown as IDBFactory)
}

function item(hanzi: string, order: number): CurriculumItem {
  return {
    hanzi,
    traditional: null,
    hsk_level: 1,
    frequency: null,
    curriculum_order: order,
    pinyin: `pinyin of ${hanzi}`,
    pos: null,
    meaning_en: `${hanzi} (en)`,
    meaning_de: null,
    cloze_sentence: null,
    example_sentence_source: null,
    example_sentence_pinyin: null,
    example_sentence_target: null,
    vocab_audio_path: null,
    sentence_audio_path: null,
    image_path: null,
  }
}

afterEach(() => {
  Reflect.deleteProperty(navigator, 'locks')
})

describe('the IndexedDB backend', () => {
  it('creates every table and keeps what a transaction committed for the next connection', async () => {
    const factory = new FakeIndexedDb()
    const first = await open(factory)
    await first.transaction(async (tx) => {
      tx.put('xp_ledger', 10, { id: 10 })
      tx.put('xp_ledger', 2, { id: 2 })
      tx.put('app_state', 'b', '2')
      tx.put('app_state', 'a', '1')
    })
    const second = await open(factory)
    const read = await second.transaction(async (tx) => ({
      xp: await tx.getAll('xp_ledger'),
      state: await tx.getAll('app_state'),
      count: await tx.count('xp_ledger'),
      one: await tx.get('app_state', 'a'),
      missing: await tx.get('app_state', 'z'),
    }))
    // Numeric keys in numeric order, as IndexedDB orders them.
    expect(read).toEqual({
      xp: [{ id: 2 }, { id: 10 }],
      state: ['1', '2'],
      count: 2,
      one: '1',
      missing: undefined,
    })
    expect(factory.storeNames('learn').sort()).toEqual([...TABLES].sort())
  })

  it('commits nothing of a transaction whose body fails', async () => {
    const factory = new FakeIndexedDb()
    const storage = await open(factory)
    await storage.transaction(async (tx) => tx.put('app_state', 'kept', 'yes'))
    await expect(
      storage.transaction(async (tx) => {
        tx.put('app_state', 'kept', 'changed')
        tx.put('app_state', 'new', 'x')
        await new Promise((resolve) => setTimeout(resolve, 5))
        throw new Error('refused')
      }),
    ).rejects.toThrow('refused')
    expect(factory.committed('learn', 'app_state')).toEqual([['kept', 'yes']])
  })

  it('commits nothing when IndexedDB refuses one of the writes', async () => {
    const factory = new FakeIndexedDb()
    const storage = await open(factory)
    await expect(
      storage.transaction(async (tx) => {
        tx.put('app_state', 'first', 'placed before the refused one')
        tx.put('captures', Number.NaN, { id: Number.NaN })
      }),
    ).rejects.toThrow('not a valid key')
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(factory.committed('learn', 'app_state')).toEqual([])
    // The connection is still usable.
    await storage.transaction(async (tx) => tx.put('app_state', 'after', 'yes'))
    expect(factory.committed('learn', 'app_state')).toEqual([['after', 'yes']])
  })

  it('keeps bytes and nested rows as they were written', async () => {
    const factory = new FakeIndexedDb()
    const storage = await open(factory)
    const row = { tags: ['a', 'b'], audio: new Uint8Array([0xff, 0xfb, 0x90, 0x00]) }
    await storage.transaction(async (tx) => tx.put('captures', 1, row))
    const read = await (await open(factory)).transaction((tx) => tx.get<typeof row>('captures', 1))
    expect(read?.tags).toEqual(['a', 'b'])
    expect(read?.audio).toBeInstanceOf(Uint8Array)
    expect([...(read?.audio ?? [])]).toEqual([0xff, 0xfb, 0x90, 0x00])
  })

  it('gives way to a newer version opened elsewhere and then says why it cannot work', async () => {
    const factory = new FakeIndexedDb()
    const storage = await open(factory)
    await new Promise<void>((resolve, reject) => {
      const newer = factory.open('learn', 2)
      newer.onsuccess = () => resolve()
      newer.onerror = () => reject(newer.error)
    })
    await expect(storage.transaction((tx) => tx.get('app_state', 'a'))).rejects.toThrow(
      'connection is closing',
    )
  })

  it('serializes transactions of two connections with a Web Lock', async () => {
    const locks = new FakeLockManager()
    Object.defineProperty(navigator, 'locks', { value: locks, configurable: true })
    const factory = new FakeIndexedDb()
    const tabs = [await open(factory), await open(factory)]
    const bump = (storage: Storage) =>
      storage.transaction(async (tx) => {
        const value = Number((await tx.get<string>('app_state', 'n')) ?? '0')
        await new Promise((resolve) => setTimeout(resolve, 1))
        tx.put('app_state', 'n', String(value + 1))
      })
    await Promise.all(Array.from({ length: 12 }, (_, index) => bump(tabs[index % 2])))
    expect(factory.committed('learn', 'app_state')).toEqual([['n', '12']])
    expect(new Set(locks.requested)).toEqual(new Set(['learn-transaction']))
  })

  it('runs the web adapter: a reopened database keeps the progress and the seeded curriculum', async () => {
    const factory = new FakeIndexedDb()
    const clock = new FixedClock(parseInstant('2026-10-31T08:00:00.000Z'), 3600)
    let loads = 0
    const adapter = () =>
      new WebAdapter({
        storage: open(factory),
        clock,
        loadCurriculum: async () => {
          loads += 1
          return [item('我', 0), item('你', 1), item('好', 2)]
        },
      })
    const web = adapter()
    await web.submitDailyQuota(2)
    const queue = await web.getDailyQueue()
    expect(queue.map((each) => each.kind)).toEqual(['new', 'new'])
    const cardId = await web.startNewItem('我')
    expect((await web.submitReviewRating(cardId, 3)).xp_awarded).toBe(15)

    const reopened = adapter()
    expect(await reopened.getStats()).toMatchObject({ xp_total: 15, learned_count: 1 })
    expect(await reopened.getDailyQueue()).toEqual([
      expect.objectContaining({ kind: 'review', card_id: cardId }),
      { kind: 'new', hanzi: '你', curriculum_order: 1 },
    ])
    expect(loads).toBe(2)
    expect(factory.committed('learn', 'hsk_curriculum')).toHaveLength(3)
    const fingerprints = await (
      await open(factory)
    ).transaction((tx) => new Store(tx).getState('curriculum_fingerprint'))
    expect(fingerprints).toMatch(/^[0-9a-f]{16}$/)
  })
})
