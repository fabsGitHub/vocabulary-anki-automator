/**
 * The golden vectors (`golden.json`, written by
 * crates/vocab-core/examples/golden_vectors.rs from the Rust core) and the
 * comparison the parity tests make with them.
 *
 * The file is read as text and parsed at run time rather than imported as a
 * JSON module, so the type checker does not infer a type for 50,000 lines;
 * the interfaces below describe the parts the tests read.
 */

import goldenText from './golden.json?raw'

/** A recorded result: the value, or the error's sentence. */
export type Outcome = { ok: unknown } | { error: string }

/** One call of a scenario: `op` for a core function, `command` for a Learn command. */
export interface Step {
  op?: string
  command?: string
  args?: unknown[]
  result?: Outcome
  [field: string]: unknown
}

export interface Scenario {
  name: string
  start: string
  offset_seconds: number
  steps: Step[]
  meaning_lang?: 'de' | 'en'
  curriculum?: unknown[]
}

/** Every section of golden.json, loosely typed: the tests compare whole values. */
export interface Golden {
  mulberry32: {
    outputs: {
      seed: number
      next_u32: number[]
      next_f64: number[]
      below: { n: number; value: number }[]
    }[]
    shuffles: { seed: number; len: number; result: number[]; next_u32: number }[]
  }
  language: {
    normalize_lang: { input: string; output: string }[]
    cjk_chars: { text: string; chars: string[] }[]
    debug: { text: string; debug: string }[]
  }
  time: {
    parse_date: { text: string; date: string | null }[]
    local_day_bounds: { date: string; offset_seconds: number; start: string; end: string }[]
    today: { now: string; offset_seconds: number; today: string }[]
  }
  fsrs: {
    sequences: {
      name: string
      parameters: FsrsParametersJson
      start: Record<string, unknown>
      steps: {
        now: string
        rating: number
        outcomes: { rating: number; card: unknown; log: unknown }[]
      }[]
    }[]
    retrievability: { card: Record<string, unknown>; at: string; retrievability: number }[]
  }
  scheduler: {
    initial: { card_id: number; now: string; state: unknown }[]
    sequences: {
      name: string
      parameters: FsrsParametersJson
      initial: unknown
      steps: {
        now: string
        rating: number
        session_date: string
        outcome: unknown
        previews: unknown
      }[]
    }[]
    cases: {
      state: unknown
      now: string
      rating: number
      session_date: string
      outcome: unknown
      previews: unknown
    }[]
    invalid: {
      name: string
      row: unknown
      error: string | null
      now: string
      previews: unknown
      review: unknown
    }[]
  }
  queue: {
    name: string
    due: unknown[]
    unscheduled: unknown[]
    curriculum: unknown[]
    learned: string[]
    quota: number
    queue: unknown
  }[]
  streak: {
    today: string
    completed: string[]
    frozen: string[]
    current: number
    longest: number
  }[]
  lessons: {
    captures: unknown[]
    cases: { cards_per_lesson: number; lessons: unknown }[]
  }
  fingerprint: {
    cases: { name: string; items: unknown[]; fingerprint: string }[]
    bundled: { count: number; fingerprint: string }
  }
  exercises: {
    captures: { card: unknown; origin: string; learned: boolean; id: number }[]
    pool: unknown
    learned_hanzi: string[]
    cases: { card_id: number; kind: string; seed: number; result: Outcome }[]
    answers: { exercise: unknown; answer: unknown; correct: boolean }[]
  }
  scenarios: Scenario[]
  adapter: Scenario[]
}

export interface FsrsParametersJson {
  request_retention: number
  maximum_interval: number
  w: number[]
  enable_short_term: boolean
  enable_fuzz: boolean
}

export const golden: Golden = JSON.parse(goldenText)

/** FSRS memory values: equal within this, relative above 1, absolute below. */
export const FLOAT_TOLERANCE = 1e-9

/**
 * Fields whose last bits may differ between platforms: the FSRS memory
 * state, which `exp`, `powf` and Rust's fused `mul_add` round differently.
 * Everything derived from them (intervals, due dates) must match exactly.
 */
const FLOAT_FIELDS = new Set(['stability', 'difficulty', 'retrievability'])

/**
 * The path of the first place where `actual` differs from `expected`, or
 * `null` when they are equal: exactly, except for {@link FLOAT_FIELDS}.
 */
export function firstDifference(actual: unknown, expected: unknown, path: string): string | null {
  return walk(actual, expected, path, null)
}

function walk(actual: unknown, expected: unknown, path: string, key: string | null): string | null {
  if (typeof expected === 'number' && typeof actual === 'number') {
    if (actual === expected) return null
    const close =
      key !== null &&
      FLOAT_FIELDS.has(key) &&
      Math.abs(actual - expected) <= FLOAT_TOLERANCE * Math.max(1, Math.abs(expected))
    return close ? null : `${path}: ${actual} instead of ${expected}`
  }
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual)) return `${path}: ${describe(actual)} instead of an array`
    if (actual.length !== expected.length) {
      return `${path}: ${actual.length} items instead of ${expected.length}`
    }
    for (let index = 0; index < expected.length; index += 1) {
      const found = walk(actual[index], expected[index], `${path}[${index}]`, key)
      if (found !== null) return found
    }
    return null
  }
  if (expected !== null && typeof expected === 'object') {
    if (actual === null || typeof actual !== 'object' || Array.isArray(actual)) {
      return `${path}: ${describe(actual)} instead of an object`
    }
    const want = expected as Record<string, unknown>
    const have = actual as Record<string, unknown>
    const keys = [...new Set([...Object.keys(want), ...Object.keys(have)])].sort()
    for (const field of keys) {
      if (!(field in want)) return `${path}.${field}: unexpected`
      if (!(field in have)) return `${path}.${field}: missing`
      const found = walk(have[field], want[field], `${path}.${field}`, field)
      if (found !== null) return found
    }
    return null
  }
  return Object.is(actual, expected)
    ? null
    : `${path}: ${describe(actual)} instead of ${describe(expected)}`
}

function describe(value: unknown): string {
  return value === undefined ? 'undefined' : JSON.stringify(value)
}
