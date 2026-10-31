/**
 * Runtime schemas of IPC shapes, for checking what the Rust side serializes
 * (the `ipc_shapes.json` fixtures) against the hand-written TypeScript types.
 *
 * A schema checks a JSON value at runtime and has a static type, `Infer<S>`,
 * that a test compares with the interface it stands for (`Equals`). So a
 * fixture value that passes its schema is a value of that interface, checked
 * both ways: the schema cannot drift from the type without a compile error,
 * and the Rust output cannot drift from the schema without a failing test.
 * Objects are exact, as serde writes them: a missing key and an extra key are
 * both mismatches. {@link documentedCommands} reads the command tables of
 * docs/architecture.md, so the binding contract is checked too.
 *
 * A copy of this module serves Learn (apps/learn/src/test/shape.ts); the two
 * apps' sources are built independently.
 */

export interface Shape<T> {
  /** `value` as a `T`; throws a {@link ShapeError} naming the path otherwise. */
  check(value: unknown, path?: string): T
}

export type Infer<S> = S extends Shape<infer T> ? T : never

/** `true` exactly when `A` and `B` are the same type (not just mutually assignable). */
export type Equals<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false

/** The keys of `M` whose schema type differs from the type `T` names for it. */
export type Mismatches<M, T> = {
  [K in keyof T]: K extends keyof M ? (Equals<Infer<M[K]>, T[K]> extends true ? never : K) : K
}[keyof T]

export class ShapeError extends Error {
  constructor(path: string, expected: string, value: unknown) {
    super(`${path}: expected ${expected}, got ${JSON.stringify(value)}`)
    this.name = 'ShapeError'
  }
}

function shape<T>(check: (value: unknown, path: string) => T): Shape<T> {
  return { check: (value, path = '$') => check(value, path) }
}

function refine<T>(base: Shape<T>, expected: string, ok: (value: T) => boolean): Shape<T> {
  return shape((value, path) => {
    const checked = base.check(value, path)
    if (!ok(checked)) throw new ShapeError(path, expected, value)
    return checked
  })
}

export const string: Shape<string> = shape((value, path) => {
  if (typeof value !== 'string') throw new ShapeError(path, 'a string', value)
  return value
})

export const boolean: Shape<boolean> = shape((value, path) => {
  if (typeof value !== 'boolean') throw new ShapeError(path, 'a boolean', value)
  return value
})

/** A finite number (`f64`). */
export const number: Shape<number> = shape((value, path) => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new ShapeError(path, 'a finite number', value)
  }
  return value
})

/** An integer (`i64`, `u32`, `usize`) that a JavaScript number holds exactly. */
export const integer: Shape<number> = refine(number, 'a safe integer', Number.isSafeInteger)

/**
 * chrono's serde form of a `DateTime<Utc>`: RFC 3339 in UTC with `Z`, with
 * fractional digits only when there are any, and readable by `Date.parse`,
 * which is how the frontends read instants.
 */
export const instant: Shape<string> = refine(
  string,
  'an RFC 3339 UTC instant',
  (text) =>
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{1,9})?Z$/.test(text) &&
    !Number.isNaN(Date.parse(text)),
)

/** A `NaiveDate`: `YYYY-MM-DD`. */
export const isoDate: Shape<string> = refine(string, 'a YYYY-MM-DD date', (text) =>
  /^\d{4}-\d{2}-\d{2}$/.test(text),
)

/** Base64 with the standard alphabet and padding (`base64::engine::general_purpose::STANDARD`). */
export const base64: Shape<string> = refine(string, 'standard base64', (text) =>
  /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(text),
)

/** One of `values` (a unit enum variant, a literal union). */
export function literal<const L extends readonly (string | number)[]>(
  ...values: L
): Shape<L[number]> {
  return shape((value, path) => {
    if (!values.includes(value as L[number])) {
      throw new ShapeError(path, `one of ${JSON.stringify(values)}`, value)
    }
    return value as L[number]
  })
}

export function nullable<T>(inner: Shape<T>): Shape<T | null> {
  return shape((value, path) => (value === null ? null : inner.check(value, path)))
}

export function array<T>(inner: Shape<T>): Shape<T[]> {
  return shape((value, path) => {
    if (!Array.isArray(value)) throw new ShapeError(path, 'an array', value)
    return value.map((item, index) => inner.check(item, `${path}[${index}]`))
  })
}

type ObjectOf<P extends Record<string, Shape<unknown>>> = { [K in keyof P]: Infer<P[K]> }

/** An object with exactly the keys of `fields`, each matching its schema. */
export function object<P extends Record<string, Shape<unknown>>>(fields: P): Shape<ObjectOf<P>> {
  return shape((value, path) => {
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
      throw new ShapeError(path, 'an object', value)
    }
    const record = value as Record<string, unknown>
    const extra = Object.keys(record).filter((key) => !(key in fields))
    if (extra.length > 0) throw new ShapeError(path, `no keys ${JSON.stringify(extra)}`, value)
    const checked: Record<string, unknown> = {}
    for (const key of Object.keys(fields)) {
      if (!(key in record)) throw new ShapeError(`${path}.${key}`, 'a value', undefined)
      checked[key] = fields[key].check(record[key], `${path}.${key}`)
    }
    return checked as ObjectOf<P>
  })
}

/** One row of a command table of docs/architecture.md. */
export interface DocumentedCommand {
  command: string
  /** The argument names, in the documented order. */
  args: string[]
  /** The result type, as the first code span of the "returns" cell spells it; `never` without one. */
  returns: string
}

/**
 * The rows of the first `| command | args | returns |` table after `heading`
 * in `markdown` (docs/architecture.md, read with `?raw`).
 */
export function documentedCommands(markdown: string, heading: string): DocumentedCommand[] {
  const start = markdown.indexOf(heading)
  if (start < 0) throw new Error(`no heading ${JSON.stringify(heading)}`)
  const lines = markdown.slice(start).split('\n').slice(1)
  const first = lines.findIndex((line) => line.startsWith('| command'))
  const rows: DocumentedCommand[] = []
  for (const line of lines.slice(first + 2)) {
    if (!line.startsWith('|')) break
    // A `\|` inside a cell is a pipe of a type (`string \| null`), not a column break.
    const [, command, args, returns] = line.split(/(?<!\\)\|/).map((cell) => cell.trim())
    const code = (cell: string) => /`([^`]*)`/.exec(cell)?.[1].replace(/\\\|/g, '|') ?? null
    rows.push({
      command: code(command) ?? command,
      args: args.startsWith('—')
        ? []
        : (code(args) ?? '').split(', ').map((arg) => arg.split(':')[0].trim()),
      returns: code(returns) ?? 'never',
    })
  }
  return rows
}

/** The first of `options` that `value` matches (tagged enums, unions of objects). */
export function union<const S extends readonly Shape<unknown>[]>(
  ...options: S
): Shape<Infer<S[number]>> {
  return shape((value, path) => {
    const reasons: string[] = []
    for (const option of options) {
      try {
        return option.check(value, path) as Infer<S[number]>
      } catch (err) {
        reasons.push(err instanceof Error ? err.message : String(err))
      }
    }
    throw new ShapeError(path, `one of the variants (${reasons.join('; ')})`, value)
  })
}
