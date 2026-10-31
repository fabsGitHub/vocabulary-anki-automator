/**
 * The IndexedDB backend of {@link BufferedStorage}: the web build's
 * equivalent of the desktop's SQLite file.
 *
 * It stays small on purpose. Buffering, savepoints and read-your-writes live
 * in `./storage.ts`, where the tests exercise them over memory; this file
 * only reads committed rows, writes one transaction's rows in one
 * `readwrite` IndexedDB transaction (so they land together or not at all)
 * and keeps transactions from overlapping: within the tab by queueing them,
 * across tabs with the Web Locks API where the browser has it. Every table
 * is an object store with out-of-line keys, so a row is stored exactly as
 * the store module encodes it.
 */

import {
  BufferedStorage,
  TABLES,
  type Key,
  type Storage,
  type StorageBackend,
  type TableName,
  type TableWrites,
} from './storage'

/** The database name, shared by every tab of the web build. */
export const DATABASE_NAME = 'vocab-learn'
const DATABASE_VERSION = 1

/** Opens (creating or upgrading) the web build's database. */
export async function openIndexedDbStorage(name: string = DATABASE_NAME): Promise<Storage> {
  if (typeof indexedDB === 'undefined') throw new Error('this browser has no IndexedDB')
  const db = await openDatabase(indexedDB, name)
  return new BufferedStorage(new IndexedDbBackend(db, `${name}-transaction`))
}

function openDatabase(factory: IDBFactory, name: string): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const opening = factory.open(name, DATABASE_VERSION)
    opening.onupgradeneeded = () => {
      const db = opening.result
      for (const table of TABLES) {
        if (!db.objectStoreNames.contains(table)) db.createObjectStore(table)
      }
    }
    opening.onsuccess = () => resolve(opening.result)
    opening.onerror = () => reject(opening.error ?? new Error(`could not open ${name}`))
  })
}

class IndexedDbBackend implements StorageBackend {
  private queue: Promise<unknown> = Promise.resolve()

  constructor(
    private readonly db: IDBDatabase,
    private readonly lockName: string,
  ) {
    // A newer build in another tab wants to upgrade the schema: let it.
    db.onversionchange = () => db.close()
  }

  get(table: TableName, key: Key): Promise<unknown> {
    return settle(this.read(table).get(key))
  }

  async getAll(table: TableName): Promise<[Key, unknown][]> {
    const store = this.read(table)
    const [keys, rows] = await Promise.all([settle(store.getAllKeys()), settle(store.getAll())])
    return keys.map((key, index) => [key as Key, rows[index]])
  }

  count(table: TableName): Promise<number> {
    return settle(this.read(table).count())
  }

  commit(writes: ReadonlyMap<TableName, TableWrites>): Promise<void> {
    const tables = [...writes.keys()]
    if (tables.length === 0) return Promise.resolve()
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(tables, 'readwrite')
      tx.oncomplete = () => resolve()
      tx.onabort = () => reject(tx.error ?? new Error('the IndexedDB transaction was aborted'))
      for (const [table, { cleared, rows }] of writes) {
        const store = tx.objectStore(table)
        if (cleared) store.clear()
        for (const [key, row] of rows) {
          if (row === undefined) store.delete(key)
          else store.put(row, key)
        }
      }
    })
  }

  exclusive<T>(body: () => Promise<T>): Promise<T> {
    const locked = () => withLock(this.lockName, body)
    const run = this.queue.then(locked, locked)
    this.queue = run.catch(() => undefined)
    return run
  }

  private read(table: TableName): IDBObjectStore {
    return this.db.transaction(table, 'readonly').objectStore(table)
  }
}

/** Runs `body` holding the Web Lock `name`, or as it is where there are no Web Locks. */
function withLock<T>(name: string, body: () => Promise<T>): Promise<T> {
  const locks = typeof navigator === 'undefined' ? undefined : navigator.locks
  if (!locks) return body()
  return locks.request(name, () => body()) as Promise<T>
}

function settle<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error ?? new Error('an IndexedDB request failed'))
  })
}
