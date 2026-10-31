/**
 * The storage layer's promises, which the golden scenarios rely on without
 * isolating them: a transaction's writes land together or not at all,
 * savepoints undo only their own writes, transactions never interleave, and
 * rows cannot be changed behind the store's back.
 */

import { describe, expect, it } from 'vitest'

import { StoreError } from './errors'
import { newCard } from './model'
import { MemoryStorage, type Transaction, compareKeys } from './storage'
import { Store } from './store'
import { parseInstantMillis } from './time'

const NOON = parseInstantMillis('2026-10-31T12:00:00.000Z')

describe('MemoryStorage transactions', () => {
  it('commit every write together, and none of them when the body throws', async () => {
    const storage = new MemoryStorage()
    await storage.transaction(async (tx) => {
      tx.put('app_state', 'a', '1')
      tx.put('app_state', 'b', '2')
    })
    await expect(
      storage.transaction(async (tx) => {
        tx.put('app_state', 'a', 'changed')
        tx.delete('app_state', 'b')
        tx.put('app_state', 'c', '3')
        throw new Error('refused')
      }),
    ).rejects.toThrow('refused')
    const rows = await storage.transaction((tx) => tx.getAll<string>('app_state'))
    expect(rows).toEqual(['1', '2'])
  })

  it('read their own writes, deletes and clears', async () => {
    const storage = new MemoryStorage()
    await storage.transaction(async (tx) => {
      tx.put('xp_ledger', 2, { id: 2 })
      tx.put('xp_ledger', 1, { id: 1 })
    })
    await storage.transaction(async (tx) => {
      tx.delete('xp_ledger', 1)
      tx.put('xp_ledger', 3, { id: 3 })
      expect(await tx.get('xp_ledger', 1)).toBeUndefined()
      expect(await tx.getAll('xp_ledger')).toEqual([{ id: 2 }, { id: 3 }])
      expect(await tx.count('xp_ledger')).toBe(2)
      tx.clear('xp_ledger')
      expect(await tx.get('xp_ledger', 2)).toBeUndefined()
      tx.put('xp_ledger', 9, { id: 9 })
      expect(await tx.getAll('xp_ledger')).toEqual([{ id: 9 }])
    })
    expect(await storage.transaction((tx) => tx.getAll('xp_ledger'))).toEqual([{ id: 9 }])
  })

  it('undo only the writes after a savepoint', async () => {
    const storage = new MemoryStorage()
    await storage.transaction(async (tx) => {
      tx.put('app_state', 'kept', 'yes')
      const savepoint = tx.savepoint()
      tx.put('app_state', 'kept', 'overwritten')
      tx.put('app_state', 'dropped', 'yes')
      tx.clear('captures')
      tx.rollbackTo(savepoint)
      tx.put('app_state', 'after', 'yes')
    })
    const rows = await storage.transaction((tx) => tx.getAll<string>('app_state'))
    expect(rows).toEqual(['yes', 'yes'])
    expect(await storage.transaction((tx) => tx.get('app_state', 'dropped'))).toBeUndefined()
  })

  it('run one at a time, in the order they were started', async () => {
    const storage = new MemoryStorage()
    const events: string[] = []
    const slow = storage.transaction(async (tx) => {
      events.push('slow starts')
      const value = Number((await tx.get<string>('app_state', 'n')) ?? '0')
      await new Promise((resolve) => setTimeout(resolve, 20))
      tx.put('app_state', 'n', String(value + 1))
      events.push('slow ends')
    })
    const fast = storage.transaction(async (tx) => {
      events.push('fast starts')
      const value = Number((await tx.get<string>('app_state', 'n')) ?? '0')
      tx.put('app_state', 'n', String(value + 1))
      events.push('fast ends')
    })
    await Promise.all([slow, fast])
    expect(events).toEqual(['slow starts', 'slow ends', 'fast starts', 'fast ends'])
    expect(await storage.transaction((tx) => tx.get('app_state', 'n'))).toBe('2')
  })

  it('keep working after a transaction failed', async () => {
    const storage = new MemoryStorage()
    const failed = storage.transaction(async () => {
      throw new Error('first')
    })
    const next = storage.transaction(async (tx) => {
      tx.put('app_state', 'k', 'v')
      return 'done'
    })
    await expect(failed).rejects.toThrow('first')
    await expect(next).resolves.toBe('done')
  })

  it('copy rows on the way in and out', async () => {
    const storage = new MemoryStorage()
    const row = { list: [1, 2], bytes: new Uint8Array([1, 2]) }
    await storage.transaction(async (tx) => {
      tx.put('captures', 1, row)
      row.list.push(3)
      row.bytes[0] = 9
      const read = await tx.get<typeof row>('captures', 1)
      read?.list.push(4)
    })
    const stored = await storage.transaction((tx) => tx.get<typeof row>('captures', 1))
    expect(stored?.list).toEqual([1, 2])
    expect([...(stored?.bytes ?? [])]).toEqual([1, 2])
  })

  it('refuse a transaction that is used after it finished', async () => {
    const storage = new MemoryStorage()
    let leaked: Transaction | null = null
    await storage.transaction(async (tx) => {
      leaked = tx
    })
    expect(() => leaked?.put('app_state', 'late', 'x')).toThrow('already finished')
  })

  it('order keys as IndexedDB does', () => {
    const keys = ['b', 10, 'a', 2, 'B', -1]
    expect([...keys].sort(compareKeys)).toEqual([-1, 2, 10, 'B', 'a', 'b'])
  })
})

describe('Store over storage transactions', () => {
  it('rolls back a failing nested transaction and nothing else', async () => {
    const storage = new MemoryStorage()
    await storage.transaction(async (tx) => {
      const store = new Store(tx)
      await store.setState('outer', '1')
      await expect(
        store.withTransaction(async (inner) => {
          await inner.setState('inner', '1')
          throw StoreError.invalid('refused')
        }),
      ).rejects.toThrow('invalid data: refused')
      expect(await store.getState('inner')).toBeNull()
      expect(await store.getState('outer')).toBe('1')
    })
  })

  it('never reuses an id, and a rolled-back insert gives its id back', async () => {
    const storage = new MemoryStorage()
    const card = newCard('Haus', 'house', 'de', 'en', NOON)
    const first = await storage.transaction((tx) =>
      new Store(tx).insertCapture(card, 'capture', false),
    )
    await expect(
      storage.transaction(async (tx) => {
        await new Store(tx).insertCapture(card, 'capture', false)
        throw new Error('rolled back')
      }),
    ).rejects.toThrow()
    const second = await storage.transaction((tx) =>
      new Store(tx).insertCapture(card, 'capture', false),
    )
    await storage.transaction((tx) => new Store(tx).deleteCapture(second))
    const third = await storage.transaction((tx) =>
      new Store(tx).insertCapture(card, 'capture', false),
    )
    expect([first, second, third]).toEqual([1, 2, 3])
  })

  it('enforces the foreign keys and refuses years it cannot store', async () => {
    const storage = new MemoryStorage()
    const state = {
      card_id: 42,
      algorithm: 'fsrs',
      stability: null,
      difficulty: null,
      interval_days: 0,
      repetitions: 0,
      lapses: 0,
      fsrs_state: 'new',
      elapsed_days: 0,
      due_date: NOON,
      last_reviewed_at: null,
      last_rating: null,
    }
    await expect(storage.transaction((tx) => new Store(tx).upsertSrsState(state))).rejects.toThrow(
      'database error: FOREIGN KEY constraint failed',
    )
    const card = newCard('Haus', 'house', 'de', 'en', Date.UTC(10_000, 0, 1))
    await expect(
      storage.transaction((tx) => new Store(tx).insertCapture(card, 'capture', false)),
    ).rejects.toThrow(
      'invalid data: +10000-01-01 00:00:00 UTC is outside the years 0000 to 9999 that the database can store',
    )
  })
})
