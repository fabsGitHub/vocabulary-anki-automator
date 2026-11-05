/**
 * The web build's persistence: a small async key/value store of named
 * tables, with atomic transactions.
 *
 * The desktop store is SQLite, and the core relies on two things it gives:
 * a command's writes land together or not at all (`with_transaction`, also
 * nested as savepoints), and a command never sees another one's half-done
 * writes. IndexedDB transactions cannot provide that for the port's code,
 * because they commit on their own as soon as an `await` waits for anything
 * that is not an IndexedDB request. So a {@link Transaction} here buffers
 * its writes in memory, reads through that buffer, and hands the backend
 * all of them at the end, which writes them in one IndexedDB transaction
 * (`./idb.ts`) or one synchronous step ({@link MemoryStorage}, for tests:
 * jsdom has no IndexedDB). Transactions run one at a time, which is also
 * how the desktop app serializes its commands behind a mutex.
 *
 * Rows are plain data (strings, numbers, booleans, null, arrays, objects and
 * `Uint8Array`s) and are copied on the way in and out, so a caller that
 * changes a row it read cannot change the store behind the transaction's
 * back. Keys order as IndexedDB orders them: numbers before strings,
 * numbers by value, strings by UTF-16 code units.
 */

/** The tables: the SQLite schema's, plus the `AUTOINCREMENT` counters. */
export type TableName =
  | 'captures'
  | 'srs_state'
  | 'hsk_curriculum'
  | 'daily_sessions'
  | 'app_state'
  | 'review_log'
  | 'xp_ledger'
  | 'streak_freezes'
  | 'sequences'

/** Every table, for backends that create them. */
export const TABLES: readonly TableName[] = [
  'captures',
  'srs_state',
  'hsk_curriculum',
  'daily_sessions',
  'app_state',
  'review_log',
  'xp_ledger',
  'streak_freezes',
  'sequences',
]

export type Key = string | number

/** One transaction's view of the store. */
export interface Transaction {
  get<T>(table: TableName, key: Key): Promise<T | undefined>
  /** Every row of `table`, in key order. */
  getAll<T>(table: TableName): Promise<T[]>
  count(table: TableName): Promise<number>
  put<T>(table: TableName, key: Key, row: T): void
  delete(table: TableName, key: Key): void
  clear(table: TableName): void
  /** Marks the current writes, for {@link rollbackTo}. */
  savepoint(): Savepoint
  /** Undoes every write made since `savepoint`. */
  rollbackTo(savepoint: Savepoint): void
}

/** An opaque mark in a transaction's writes. */
export interface Savepoint {
  readonly writes: ReadonlyMap<TableName, TableWrites>
}

/** The rows a transaction wrote to one table. */
export interface TableWrites {
  /** Whether the table was emptied first. */
  readonly cleared: boolean
  /** The rows put, and the keys deleted (`undefined`). */
  readonly rows: ReadonlyMap<Key, unknown>
}

/** Atomic, one-at-a-time transactions over a set of tables. */
export interface Storage {
  /**
   * Runs `body` alone against the store. Its writes are committed together
   * when it resolves and dropped when it rejects.
   */
  transaction<T>(body: (tx: Transaction) => Promise<T>): Promise<T>
}

/** What a backend holds: committed rows, and a way to commit more. */
export interface StorageBackend {
  get(table: TableName, key: Key): Promise<unknown>
  /** Every row of `table` with its key, in key order. */
  getAll(table: TableName): Promise<[Key, unknown][]>
  count(table: TableName): Promise<number>
  /** Applies one transaction's writes atomically. */
  commit(writes: ReadonlyMap<TableName, TableWrites>): Promise<void>
  /** Runs `body` while no other transaction runs (in this tab or any other). */
  exclusive<T>(body: () => Promise<T>): Promise<T>
}

/** {@link Storage} over any backend, with write buffering and savepoints. */
export class BufferedStorage implements Storage {
  constructor(private readonly backend: StorageBackend) {}

  transaction<T>(body: (tx: Transaction) => Promise<T>): Promise<T> {
    return this.backend.exclusive(async () => {
      const tx = new BufferedTransaction(this.backend)
      try {
        const value = await body(tx)
        await this.backend.commit(tx.writes())
        return value
      } finally {
        tx.close()
      }
    })
  }
}

/**
 * Committed rows in memory: the backend of {@link MemoryStorage}, and of the
 * live demo's history, whose rows it hands on whole (src/demo/history.ts).
 */
export class MemoryBackend implements StorageBackend {
  private readonly tables = new Map<TableName, Map<Key, unknown>>()
  private queue: Promise<unknown> = Promise.resolve()

  async get(table: TableName, key: Key): Promise<unknown> {
    return this.table(table).get(key)
  }

  async getAll(table: TableName): Promise<[Key, unknown][]> {
    return [...this.table(table).entries()].sort(([a], [b]) => compareKeys(a, b))
  }

  async count(table: TableName): Promise<number> {
    return this.table(table).size
  }

  async commit(writes: ReadonlyMap<TableName, TableWrites>): Promise<void> {
    for (const [name, { cleared, rows }] of writes) {
      const table = this.table(name)
      if (cleared) table.clear()
      for (const [key, row] of rows) {
        if (row === undefined) table.delete(key)
        else table.set(key, row)
      }
    }
  }

  exclusive<T>(body: () => Promise<T>): Promise<T> {
    const run = this.queue.then(body, body)
    this.queue = run.catch(() => undefined)
    return run
  }

  private table(name: TableName): Map<Key, unknown> {
    let table = this.tables.get(name)
    if (!table) {
      table = new Map()
      this.tables.set(name, table)
    }
    return table
  }
}

/** A store that lives as long as the page: the tests' stand-in for IndexedDB. */
export class MemoryStorage extends BufferedStorage {
  constructor() {
    super(new MemoryBackend())
  }
}

/**
 * A {@link Transaction} that keeps its writes until the backend commits them.
 *
 * It also keeps every table it read whole: nothing else commits while it
 * runs (transactions are exclusive, across tabs wherever the browser has Web
 * Locks), so the committed rows cannot change under it, and the core reads
 * the same tables several times per command (a rating sums the XP ledger
 * four times). Reading each once keeps a command's cost at one pass over
 * each table it needs.
 */
class BufferedTransaction implements Transaction {
  private tables = new Map<TableName, { cleared: boolean; rows: Map<Key, unknown> }>()
  private readonly committed = new Map<TableName, Promise<Map<Key, unknown>>>()
  private open = true

  constructor(private readonly backend: StorageBackend) {}

  async get<T>(table: TableName, key: Key): Promise<T | undefined> {
    this.check()
    const writes = this.tables.get(table)
    if (writes?.rows.has(key)) return copy(writes.rows.get(key)) as T | undefined
    if (writes?.cleared) return undefined
    const whole = this.committed.get(table)
    const row = whole ? (await whole).get(key) : await this.backend.get(table, key)
    return copy(row) as T | undefined
  }

  async getAll<T>(table: TableName): Promise<T[]> {
    this.check()
    const writes = this.tables.get(table)
    const rows = new Map<Key, unknown>(writes?.cleared ? [] : await this.committedRows(table))
    for (const [key, row] of writes?.rows ?? []) {
      if (row === undefined) rows.delete(key)
      else rows.set(key, row)
    }
    return [...rows.entries()]
      .sort(([a], [b]) => compareKeys(a, b))
      .map(([, row]) => copy(row) as T)
  }

  async count(table: TableName): Promise<number> {
    this.check()
    const writes = this.tables.get(table)
    if (writes) return (await this.getAll(table)).length
    const whole = this.committed.get(table)
    return whole ? (await whole).size : this.backend.count(table)
  }

  put<T>(table: TableName, key: Key, row: T): void {
    this.check()
    this.writesOf(table).rows.set(key, copy(row))
  }

  delete(table: TableName, key: Key): void {
    this.check()
    this.writesOf(table).rows.set(key, undefined)
  }

  clear(table: TableName): void {
    this.check()
    this.tables.set(table, { cleared: true, rows: new Map() })
  }

  savepoint(): Savepoint {
    this.check()
    return { writes: this.snapshot() }
  }

  rollbackTo(savepoint: Savepoint): void {
    this.check()
    this.tables = new Map(
      [...savepoint.writes].map(([table, { cleared, rows }]) => [
        table,
        { cleared, rows: new Map(rows) },
      ]),
    )
  }

  writes(): ReadonlyMap<TableName, TableWrites> {
    return this.snapshot()
  }

  /** After commit or rollback: a use from a promise nobody awaited is a bug. */
  close(): void {
    this.open = false
  }

  private snapshot(): Map<TableName, TableWrites> {
    return new Map(
      [...this.tables].map(([table, { cleared, rows }]) => [
        table,
        { cleared, rows: new Map(rows) },
      ]),
    )
  }

  /** The committed rows of `table`, read from the backend once per transaction. */
  private committedRows(table: TableName): Promise<Map<Key, unknown>> {
    let rows = this.committed.get(table)
    if (rows === undefined) {
      rows = this.backend.getAll(table).then((entries) => new Map(entries))
      this.committed.set(table, rows)
      // A failed read is not kept: the caller sees it, and a later read retries.
      rows.catch(() => this.committed.delete(table))
    }
    return rows
  }

  private writesOf(table: TableName): { cleared: boolean; rows: Map<Key, unknown> } {
    let writes = this.tables.get(table)
    if (!writes) {
      writes = { cleared: false, rows: new Map() }
      this.tables.set(table, writes)
    }
    return writes
  }

  private check(): void {
    if (!this.open) throw new Error('the storage transaction has already finished')
  }
}

/** IndexedDB's key order. */
export function compareKeys(a: Key, b: Key): number {
  if (typeof a === 'number' && typeof b === 'number') return a - b
  if (typeof a === 'number') return -1
  if (typeof b === 'number') return 1
  return a < b ? -1 : a > b ? 1 : 0
}

/** A deep copy of plain row data. */
function copy(value: unknown): unknown {
  if (value === null || typeof value !== 'object') return value
  // Not `instanceof Uint8Array`: bytes cloned in another realm (an iframe, a
  // test environment's structuredClone) fail that test and would come back
  // as a plain object of numbered keys.
  if (ArrayBuffer.isView(value)) return Uint8Array.from(value as Uint8Array)
  if (Array.isArray(value)) return value.map(copy)
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, copy(item)]))
}
