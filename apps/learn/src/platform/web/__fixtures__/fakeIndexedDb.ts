/**
 * A small IndexedDB for the tests of `../idb.ts`: jsdom has none, and no
 * IndexedDB package may be installed. It implements only what idb.ts calls,
 * but with the semantics of the specification that idb.ts depends on and
 * can get wrong:
 *
 * - Requests complete asynchronously and in order, and a transaction
 *   commits on its own once its last request has completed without another
 *   one being placed, as browsers auto-commit.
 * - A transaction's writes reach other transactions only when it commits;
 *   `abort()` drops them and fails what is still pending.
 * - Placing a request with a key IndexedDB cannot store throws a
 *   `DataError` right away **without ending the transaction**, so the
 *   requests placed before it still commit unless the caller aborts.
 * - Rows are stored as structured clones.
 * - Opening a newer version tells the open connections (`versionchange`);
 *   a closed connection refuses new transactions.
 *
 * Connections to the same name share their data, like two tabs of one
 * origin.
 */

type Key = string | number
type Table = Map<Key, unknown>

/** What the fake's callers see of a request. */
class FakeRequest<T> {
  result: T | undefined = undefined
  error: DOMException | null = null
  onsuccess: (() => void) | null = null
  onerror: (() => void) | null = null
  onupgradeneeded: (() => void) | null = null
}

/** One named database: its version and committed tables, shared by connections. */
interface Backing {
  version: number
  tables: Map<string, Table>
  connections: Set<FakeDatabase>
}

/** `IDBFactory`. */
export class FakeIndexedDb {
  private readonly databases = new Map<string, Backing>()

  open(name: string, version: number): FakeRequest<FakeDatabase> {
    const request = new FakeRequest<FakeDatabase>()
    later(() => {
      let backing = this.databases.get(name)
      if (backing === undefined) {
        backing = { version: 0, tables: new Map(), connections: new Set() }
        this.databases.set(name, backing)
      }
      if (version < backing.version) {
        request.error = new DOMException(`version ${version} is older`, 'VersionError')
        request.onerror?.()
        return
      }
      const db = new FakeDatabase(name, backing)
      if (version > backing.version) {
        for (const other of backing.connections) other.onversionchange?.()
        db.upgrading = true
        backing.version = version
        request.result = db
        request.onupgradeneeded?.()
        db.upgrading = false
      }
      backing.connections.add(db)
      request.result = db
      request.onsuccess?.()
    })
    return request
  }

  /** The object stores database `name` has. */
  storeNames(name: string): string[] {
    return [...(this.databases.get(name)?.tables.keys() ?? [])]
  }

  /** The committed rows of `table` in database `name`, in key order. */
  committed(name: string, table: string): [Key, unknown][] {
    const rows = this.databases.get(name)?.tables.get(table) ?? new Map<Key, unknown>()
    return sortedEntries(rows)
  }
}

/** `IDBDatabase`. */
export class FakeDatabase {
  onversionchange: (() => void) | null = null
  upgrading = false
  private closed = false

  constructor(
    readonly name: string,
    private readonly backing: Backing,
  ) {}

  get objectStoreNames(): { contains(name: string): boolean } {
    return { contains: (name: string) => this.backing.tables.has(name) }
  }

  createObjectStore(name: string): void {
    if (!this.upgrading) throw new DOMException('not in an upgrade', 'InvalidStateError')
    this.backing.tables.set(name, new Map())
  }

  transaction(
    names: string | string[],
    mode: 'readonly' | 'readwrite' = 'readonly',
  ): FakeTransaction {
    if (this.closed)
      throw new DOMException('The database connection is closing.', 'InvalidStateError')
    const scope = typeof names === 'string' ? [names] : names
    for (const name of scope) {
      if (!this.backing.tables.has(name)) {
        throw new DOMException(`no object store ${name}`, 'NotFoundError')
      }
    }
    return new FakeTransaction(this.backing, scope, mode)
  }

  close(): void {
    this.closed = true
    this.backing.connections.delete(this)
  }
}

/** `IDBTransaction`. */
export class FakeTransaction {
  oncomplete: (() => void) | null = null
  onabort: (() => void) | null = null
  error: DOMException | null = null
  private readonly pending: { request: FakeRequest<unknown>; run: () => unknown }[] = []
  private readonly working = new Map<string, Table>()
  private finished = false
  private scheduled = false

  constructor(
    private readonly backing: Backing,
    private readonly scope: string[],
    private readonly mode: 'readonly' | 'readwrite',
  ) {
    // A transaction without requests commits at the end of the task that made it.
    this.schedule()
  }

  objectStore(name: string): FakeObjectStore {
    if (!this.scope.includes(name))
      throw new DOMException(`${name} is not in scope`, 'NotFoundError')
    return new FakeObjectStore(this, name)
  }

  abort(): void {
    if (this.finished) throw new DOMException('the transaction has finished', 'InvalidStateError')
    this.finished = true
    this.working.clear()
    for (const { request } of this.pending.splice(0)) {
      request.error = new DOMException('the transaction was aborted', 'AbortError')
      request.onerror?.()
    }
    later(() => this.onabort?.())
  }

  /** Places a request; `run` executes when its turn comes. */
  place<T>(run: () => T, write = false): FakeRequest<T> {
    if (this.finished) {
      throw new DOMException('the transaction has finished', 'TransactionInactiveError')
    }
    if (write && this.mode !== 'readwrite') {
      throw new DOMException('the transaction is read-only', 'ReadOnlyError')
    }
    const request = new FakeRequest<T>()
    this.pending.push({ request: request as FakeRequest<unknown>, run })
    this.schedule()
    return request
  }

  /** The rows of `name` as this transaction sees them. */
  rows(name: string): Table {
    return this.working.get(name) ?? (this.backing.tables.get(name) as Table)
  }

  /** The rows of `name`, to be changed by this transaction. */
  writable(name: string): Table {
    let rows = this.working.get(name)
    if (rows === undefined) {
      rows = new Map(this.backing.tables.get(name))
      this.working.set(name, rows)
    }
    return rows
  }

  private schedule(): void {
    if (this.scheduled) return
    this.scheduled = true
    later(() => {
      this.scheduled = false
      this.step()
    })
  }

  private step(): void {
    if (this.finished) return
    const next = this.pending.shift()
    if (next === undefined) {
      this.commit()
      return
    }
    try {
      next.request.result = next.run()
      next.request.onsuccess?.()
    } catch (err) {
      next.request.error = err instanceof DOMException ? err : new DOMException(String(err))
      next.request.onerror?.()
      if (!this.finished) {
        this.error = next.request.error
        this.abort()
      }
      return
    }
    this.schedule()
  }

  private commit(): void {
    this.finished = true
    for (const [name, rows] of this.working) this.backing.tables.set(name, rows)
    later(() => this.oncomplete?.())
  }
}

/** `IDBObjectStore`, with out-of-line keys. */
export class FakeObjectStore {
  constructor(
    private readonly tx: FakeTransaction,
    private readonly name: string,
  ) {}

  get(key: unknown): FakeRequest<unknown> {
    const valid = validKey(key)
    return this.tx.place(() => clone(this.tx.rows(this.name).get(valid)))
  }

  getAll(): FakeRequest<unknown[]> {
    return this.tx.place(() => sortedEntries(this.tx.rows(this.name)).map(([, row]) => clone(row)))
  }

  getAllKeys(): FakeRequest<Key[]> {
    return this.tx.place(() => sortedEntries(this.tx.rows(this.name)).map(([key]) => key))
  }

  count(): FakeRequest<number> {
    return this.tx.place(() => this.tx.rows(this.name).size)
  }

  put(value: unknown, key: unknown): FakeRequest<Key> {
    const valid = validKey(key)
    const row = clone(value)
    return this.tx.place(() => {
      this.tx.writable(this.name).set(valid, row)
      return valid
    }, true)
  }

  delete(key: unknown): FakeRequest<undefined> {
    const valid = validKey(key)
    return this.tx.place(() => {
      this.tx.writable(this.name).delete(valid)
      return undefined
    }, true)
  }

  clear(): FakeRequest<undefined> {
    return this.tx.place(() => {
      this.tx.writable(this.name).clear()
      return undefined
    }, true)
  }
}

/** A Web Locks `LockManager` for one page: exclusive locks, granted in request order. */
export class FakeLockManager {
  private readonly held = new Map<string, Promise<unknown>>()
  readonly requested: string[] = []

  request<T>(name: string, callback: () => Promise<T>): Promise<T> {
    this.requested.push(name)
    const previous = this.held.get(name) ?? Promise.resolve()
    const run = previous.then(callback, callback)
    this.held.set(
      name,
      run.catch(() => undefined),
    )
    return run
  }
}

function validKey(key: unknown): Key {
  if (typeof key === 'string') return key
  if (typeof key === 'number' && !Number.isNaN(key)) return key
  throw new DOMException('The parameter is not a valid key.', 'DataError')
}

function clone<T>(value: T): T {
  return value === undefined ? value : structuredClone(value)
}

/** IndexedDB's key order: numbers before strings, numbers by value, strings by code unit. */
function sortedEntries(rows: Table): [Key, unknown][] {
  return [...rows.entries()].sort(([a], [b]) => {
    if (typeof a === 'number' && typeof b === 'number') return a - b
    if (typeof a === 'number') return -1
    if (typeof b === 'number') return 1
    return a < b ? -1 : a > b ? 1 : 0
  })
}

/** After the current task, as IndexedDB delivers its events. */
function later(callback: () => void): void {
  setTimeout(callback, 0)
}
