/**
 * Parity with the Rust core: every golden vector, replayed through the port.
 *
 * `__fixtures__/golden.json` holds inputs and the outputs the real Rust
 * implementations gave for them (crates/vocab-core/examples/golden_vectors.rs).
 * Each section below feeds the same inputs to the ported function of the
 * same name and expects the same output, compared as the serde JSON both
 * sides produce: exactly, except FSRS stability and difficulty within
 * 1e-9. The store-backed scenarios replay whole call sequences against a
 * fresh in-memory store, and the adapter scenario replays Learn's commands
 * against the web adapter, so the rows each call leaves behind are compared
 * too, through every later call that reads them.
 */

import { describe, expect, it } from 'vitest'

import type * as ipc from '../types'
import bundledCurriculumText from '../../../../../crates/vocab-core/data/hsk_curriculum.json?raw'
import {
  type FsrsParametersJson,
  type Outcome,
  type Scenario,
  type Step,
  firstDifference,
  golden,
} from './__fixtures__/golden'
import { WebAdapter } from './adapter'
import { curriculumFingerprint, parseCurriculum } from './curriculum'
import {
  dailyPromptState,
  dailyPromptStateToJson,
  dismissDailyPrompt,
  recordSessionProgress,
  reopenDailyPrompt,
  submitDailyQuota,
  type SessionProgress,
} from './daily'
import { errorMessage } from './errors'
import { checkAnswer, generateExercise, type ExerciseKind } from './exercises'
import {
  FSRS,
  type FsrsCard,
  type FsrsRating,
  type FsrsState,
  type Parameters,
  type ReviewLog,
  getRetrievability,
} from './fsrs'
import {
  applyStreakFreezes,
  awardXp,
  computeStreak,
  onSessionCompleted,
  statsSummary,
} from './gamification'
import { cjkChars, normalizeLang } from './language'
import { buildLessons, lessonsFromStore } from './lessons'
import {
  type CaptureOrigin,
  type CurriculumItem,
  type StoredCapture,
  cardFromJson,
  curriculumItemFromJson,
  storedCaptureToJson,
} from './model'
import { LearnedVocabularyPool } from './pool'
import {
  buildDailyQueue,
  buildDailyQueueFrom,
  introduceCapture,
  introduceNewItem,
  queueItemToJson,
  submitReview,
} from './queue'
import { Mulberry32 } from './rng'
import {
  type ReviewRating,
  SrsScheduler,
  fsrsCardFromState,
  ratingPreviewToJson,
  reviewLogToJson,
  reviewOutcomeToJson,
  srsStateFromJson,
  srsStateToJson,
} from './scheduler'
import { defaultSettings } from './settings'
import { MemoryStorage } from './storage'
import { Store, type UnscheduledCapture, dailySessionFromJson, dailySessionToJson } from './store'
import { compareStrings, debugString } from './text'
import {
  FixedClock,
  formatInstant,
  localDayBounds,
  parseDate,
  parseInstant,
  parseInstantMillis,
  today,
} from './time'

/** Fails with the path of the first difference, which says more than a diff of 50 lines. */
function expectParity(actual: unknown, expected: unknown, path: string): void {
  expect(firstDifference(actual, expected, path)).toBeNull()
}

/** Runs `run` and records its result as the generator records Rust's. */
async function outcome(run: () => Promise<unknown> | unknown): Promise<Outcome> {
  try {
    const value = await run()
    return { ok: value === undefined ? null : value }
  } catch (err) {
    return { error: errorMessage(err) }
  }
}

function range(length: number): number[] {
  return Array.from({ length }, (_, index) => index)
}

function sorted(texts: Iterable<string>): string[] {
  return [...texts].sort(compareStrings)
}

/** `store::UnscheduledCapture`'s serde shape. */
interface UnscheduledCaptureJson {
  card_id: number
  front: string
  captured_at: ipc.Instant
}

function unscheduledFromJson(value: UnscheduledCaptureJson): UnscheduledCapture {
  return { ...value, captured_at: parseInstantMillis(value.captured_at) }
}

function unscheduledToJson(row: UnscheduledCapture): UnscheduledCaptureJson {
  return { ...row, captured_at: formatInstant(row.captured_at) }
}

describe('rng::Mulberry32', () => {
  it('draws the same numbers', () => {
    for (const vector of golden.mulberry32.outputs) {
      const draws = new Mulberry32(vector.seed)
      expect(
        range(8).map(() => draws.nextU32()),
        `seed ${vector.seed}`,
      ).toEqual(vector.next_u32)
      const floats = new Mulberry32(vector.seed)
      expect(
        range(4).map(() => floats.nextF64()),
        `seed ${vector.seed}`,
      ).toEqual(vector.next_f64)
      const below = new Mulberry32(vector.seed)
      expect(
        vector.below.map(({ n }) => below.below(n)),
        `seed ${vector.seed}`,
      ).toEqual(vector.below.map(({ value }) => value))
    }
  })

  it('shuffles in the same order and consumes the same draws', () => {
    for (const vector of golden.mulberry32.shuffles) {
      const items = range(vector.len)
      const rng = new Mulberry32(vector.seed)
      rng.shuffle(items)
      expect(items, `seed ${vector.seed}, ${vector.len} items`).toEqual(vector.result)
      expect(rng.nextU32(), `the draw after seed ${vector.seed}'s shuffle`).toBe(vector.next_u32)
    }
  })
})

describe('language', () => {
  it('normalizes language codes with Rust trimming and lowercasing', () => {
    for (const { input, output } of golden.language.normalize_lang) {
      expect(normalizeLang(input), JSON.stringify(input)).toBe(output)
    }
  })

  it('finds the same CJK ideographs', () => {
    for (const { text, chars } of golden.language.cjk_chars) {
      expect(cjkChars(text), text).toEqual(chars)
    }
  })

  it("quotes strings in messages as Rust's {:?} does", () => {
    for (const { text, debug } of golden.language.debug) {
      expect(debugString(text), JSON.stringify(text)).toBe(debug)
    }
  })
})

describe('time', () => {
  it("parses dates as leniently as chrono's %Y-%m-%d", () => {
    for (const { text, date } of golden.time.parse_date) {
      expect(parseDate(text), JSON.stringify(text)).toBe(date)
    }
  })

  it('bounds local days in every offset', () => {
    for (const vector of golden.time.local_day_bounds) {
      const [start, end] = localDayBounds(vector.date, vector.offset_seconds)
      const name = `${vector.date} at ${vector.offset_seconds} s`
      expect([formatInstant(start), formatInstant(end)], name).toEqual([vector.start, vector.end])
    }
  })

  it('puts instants on the local calendar', () => {
    for (const vector of golden.time.today) {
      const clock = new FixedClock(parseInstant(vector.now), vector.offset_seconds)
      expect(today(clock), `${vector.now} at ${vector.offset_seconds} s`).toBe(vector.today)
    }
  })
})

function fsrsParameters(json: FsrsParametersJson): Parameters {
  expect(json.enable_fuzz).toBe(false)
  return {
    request_retention: json.request_retention,
    maximum_interval: json.maximum_interval,
    w: json.w,
    enable_short_term: json.enable_short_term,
  }
}

function fsrsCardFromJson(json: Record<string, unknown>): FsrsCard {
  return {
    ...(json as unknown as FsrsCard),
    state: json.state as FsrsState,
    due: parseInstantMillis(json.due as string),
    last_review: parseInstantMillis(json.last_review as string),
  }
}

function fsrsCardToJson(card: FsrsCard): Record<string, unknown> {
  return { ...card, due: formatInstant(card.due), last_review: formatInstant(card.last_review) }
}

function fsrsLogToJson(log: ReviewLog): Record<string, unknown> {
  return { ...log, reviewed_date: formatInstant(log.reviewed_date) }
}

describe('rs-fsrs', () => {
  it('replays the reference and random sequences step by step', () => {
    for (const sequence of golden.fsrs.sequences) {
      const fsrs = new FSRS(fsrsParameters(sequence.parameters))
      let card = fsrsCardFromJson(sequence.start)
      sequence.steps.forEach((step, index) => {
        const now = parseInstantMillis(step.now)
        const rating = step.rating as FsrsRating
        const record = fsrs.repeat(card, now)
        const outcomes = [...record].map(([each, info]) => ({
          rating: each,
          card: fsrsCardToJson(info.card),
          log: fsrsLogToJson(info.review_log),
        }))
        expectParity(outcomes, step.outcomes, `${sequence.name}[${index}]`)
        const next = fsrs.next(card, now, rating)
        expectParity(
          { rating, card: fsrsCardToJson(next.card), log: fsrsLogToJson(next.review_log) },
          step.outcomes[rating - 1],
          `${sequence.name}[${index}] next`,
        )
        card = record.get(rating)?.card ?? card
      })
    }
  })

  it('computes the same retrievability', () => {
    for (const vector of golden.fsrs.retrievability) {
      const actual = getRetrievability(fsrsCardFromJson(vector.card), parseInstantMillis(vector.at))
      expectParity({ retrievability: actual }, { retrievability: vector.retrievability }, vector.at)
    }
  })
})

describe('srs::SrsScheduler', () => {
  const scheduler = new SrsScheduler()

  it('creates new states due at the whole millisecond', () => {
    for (const vector of golden.scheduler.initial) {
      const state = scheduler.initialState(vector.card_id, parseInstant(vector.now))
      expectParity(srsStateToJson(state), vector.state, vector.now)
    }
  })

  it('replays seeded sequences, clock set back and sub-millisecond instants included', () => {
    for (const sequence of golden.scheduler.sequences) {
      const sequenceScheduler = new SrsScheduler(fsrsParameters(sequence.parameters))
      let state = srsStateFromJson(sequence.initial as ipc.SrsState)
      sequence.steps.forEach((step, index) => {
        const now = parseInstant(step.now)
        const rating = step.rating as ReviewRating
        const name = `${sequence.name}[${index}]`
        expectParity(
          sequenceScheduler.preview(state, now).map(ratingPreviewToJson),
          step.previews,
          `${name} previews`,
        )
        const reviewed = sequenceScheduler.review(state, rating, now, step.session_date)
        expectParity(reviewOutcomeToJson(reviewed), step.outcome, name)
        state = reviewed.next
      })
    }
  })

  it('reviews before the last review at the last review', () => {
    for (const [index, vector] of golden.scheduler.cases.entries()) {
      const state = srsStateFromJson(vector.state as ipc.SrsState)
      const now = parseInstant(vector.now)
      const rating = vector.rating as ReviewRating
      expectParity(
        reviewOutcomeToJson(scheduler.review(state, rating, now, vector.session_date)),
        vector.outcome,
        `case ${index}`,
      )
      expectParity(
        scheduler.preview(state, now).map(ratingPreviewToJson),
        vector.previews,
        `case ${index} previews`,
      )
    }
  })

  it('refuses rows FSRS cannot interpret, with the same reasons', () => {
    for (const vector of golden.scheduler.invalid) {
      const row = srsStateFromJson(vector.row as ipc.SrsState)
      const now = parseInstant(vector.now)
      let error: string | null = null
      try {
        fsrsCardFromState(row)
      } catch (err) {
        error = errorMessage(err)
      }
      expect(error, vector.name).toBe(vector.error)
      expectParity(
        scheduler.preview(row, now).map(ratingPreviewToJson),
        vector.previews,
        vector.name,
      )
      expectParity(
        reviewOutcomeToJson(scheduler.review(row, 3, now, '2026-11-09')),
        vector.review,
        vector.name,
      )
    }
  })
})

describe('srs::build_daily_queue_from', () => {
  it('orders reviews first, then unstarted captures, then curriculum words skipping learned hanzi', () => {
    for (const vector of golden.queue) {
      const queue = buildDailyQueueFrom(
        vector.due.map((row) => srsStateFromJson(row as ipc.SrsState)),
        vector.unscheduled.map((row) => unscheduledFromJson(row as UnscheduledCaptureJson)),
        vector.curriculum.map((item) => curriculumItemFromJson(item as CurriculumItem)),
        new Set(vector.learned),
        vector.quota,
      )
      expectParity(queue.map(queueItemToJson), vector.queue, vector.name)
    }
  })
})

describe('gamification::compute_streak', () => {
  it('counts the same streaks', () => {
    for (const vector of golden.streak) {
      const streak = computeStreak(vector.completed, vector.frozen, vector.today)
      expect(streak, JSON.stringify(vector)).toEqual([vector.current, vector.longest])
    }
  })
})

function storedCaptureFromJson(json: unknown): StoredCapture {
  const capture = json as ipc.StoredCapture
  return { ...capture, card: cardFromJson(capture.card) }
}

describe('lessons::build_lessons', () => {
  it('builds the same lessons for every lesson size', () => {
    const captures = golden.lessons.captures.map(storedCaptureFromJson)
    for (const vector of golden.lessons.cases) {
      expectParity(
        buildLessons(captures, vector.cards_per_lesson),
        vector.lessons,
        `${vector.cards_per_lesson} cards per lesson`,
      )
    }
  })
})

describe('store::curriculum_fingerprint', () => {
  it('fingerprints curricula byte for byte', () => {
    for (const vector of golden.fingerprint.cases) {
      const items = vector.items.map((item) => curriculumItemFromJson(item as CurriculumItem))
      expect(curriculumFingerprint(items), vector.name).toBe(vector.fingerprint)
    }
  })

  it('fingerprints the bundled curriculum as the desktop apps do', () => {
    const bundled = parseCurriculum(JSON.parse(bundledCurriculumText))
    expect(bundled).toHaveLength(golden.fingerprint.bundled.count)
    expect(curriculumFingerprint(bundled)).toBe(golden.fingerprint.bundled.fingerprint)
  })
})

/** The generator's `learned_state`: only that the row exists matters to exercises. */
function learnedState(cardId: number) {
  return {
    card_id: cardId,
    algorithm: 'fsrs',
    stability: 3.2,
    difficulty: 5.1,
    interval_days: 3,
    repetitions: 2,
    lapses: 0,
    fsrs_state: 'review',
    elapsed_days: 1,
    due_date: parseInstantMillis('2026-11-02T08:00:00.000Z'),
    last_reviewed_at: parseInstantMillis('2026-10-30T08:00:00.000Z'),
    last_rating: 3,
  }
}

describe('quiz', () => {
  async function fixturePool(): Promise<LearnedVocabularyPool> {
    const storage = new MemoryStorage()
    return storage.transaction(async (tx) => {
      const store = new Store(tx)
      for (const capture of golden.exercises.captures) {
        const card = cardFromJson(capture.card as ipc.Card)
        const id = await store.insertCapture(card, capture.origin as CaptureOrigin, false)
        expect(id).toBe(capture.id)
        if (capture.learned) await store.upsertSrsState(learnedState(id))
      }
      return LearnedVocabularyPool.load(store)
    })
  }

  it('loads the same pool', async () => {
    const pool = await fixturePool()
    expectParity(pool.entries(), golden.exercises.pool, 'pool')
    expect(sorted(pool.learnedHanziChars())).toEqual(golden.exercises.learned_hanzi)
  })

  it('generates the same exercise for every kind, card and seed', async () => {
    const pool = await fixturePool()
    for (const vector of golden.exercises.cases) {
      const result = await outcome(() =>
        generateExercise(pool, vector.card_id, vector.kind as ExerciseKind, vector.seed),
      )
      expectParity(
        result,
        vector.result,
        `${vector.kind} of card ${vector.card_id}, seed ${vector.seed}`,
      )
    }
  })

  it('checks answers the same way', () => {
    for (const [index, vector] of golden.exercises.answers.entries()) {
      const correct = checkAnswer(
        vector.exercise as ipc.Exercise,
        vector.answer as ipc.ExerciseAnswer,
      )
      expect(correct, `answer ${index}: ${JSON.stringify(vector.answer)}`).toBe(vector.correct)
    }
  })
})

/** Runs one recorded core call against `store`, returning its serde JSON. */
async function runOp(
  store: Store,
  clock: FixedClock,
  scheduler: SrsScheduler,
  step: Step,
): Promise<unknown> {
  const field = <T>(name: string) => step[name] as T
  const items = () =>
    field<unknown[]>('items').map((item) => curriculumItemFromJson(item as CurriculumItem))
  switch (step.op) {
    case 'replace_curriculum':
      return store.replaceCurriculum(items())
    case 'ensure_curriculum_seeded':
      return store.ensureCurriculumSeeded(items())
    case 'curriculum_count':
      return store.curriculumCount()
    case 'insert_capture':
      return store.insertCapture(
        cardFromJson(field<ipc.Card>('card')),
        field<CaptureOrigin>('origin'),
        false,
      )
    case 'upsert_srs_state':
      return store.upsertSrsState(srsStateFromJson(field<ipc.SrsState>('row')))
    case 'upsert_daily_session':
      return store.upsertDailySession(dailySessionFromJson(field<ipc.DailySession>('row')))
    case 'set_state':
      return store.setState(field('key'), field('value'))
    case 'get_state':
      return store.getState(field('key'))
    case 'insert_streak_freeze':
      return store.insertStreakFreeze(field('date'))
    case 'list_srs_due_before':
      return (await store.listSrsDueBefore(parseInstantMillis(field('before')))).map(srsStateToJson)
    case 'build_daily_queue':
      return (await buildDailyQueue(store, field('quota'), clock)).map(queueItemToJson)
    case 'introduce_new_item':
      return introduceNewItem(store, scheduler, field('hanzi'), field('target_lang'), clock)
    case 'introduce_capture':
      return introduceCapture(store, scheduler, field('card_id'), clock)
    case 'submit_review':
      return reviewOutcomeToJson(
        await submitReview(store, scheduler, field('card_id'), field('rating'), clock),
      )
    case 'preview': {
      const state = await store.getSrsState(field('card_id'))
      return state === null ? null : scheduler.preview(state, clock.now()).map(ratingPreviewToJson)
    }
    case 'get_capture': {
      const capture = await store.getCapture(field('id'))
      return capture === null ? null : storedCaptureToJson(capture)
    }
    case 'get_srs_state': {
      const state = await store.getSrsState(field('card_id'))
      return state === null ? null : srsStateToJson(state)
    }
    case 'review_log_for_card':
      return (await store.reviewLogForCard(field('card_id'))).map(reviewLogToJson)
    case 'list_captures':
      return (await store.listCaptures(null)).map(storedCaptureToJson)
    case 'list_unscheduled_captures':
      return (await store.listUnscheduledCaptures()).map(unscheduledToJson)
    case 'learned_chinese_fronts':
      return sorted(await store.learnedChineseFronts())
    case 'delete_capture':
      return store.deleteCapture(field('id'))
    case 'daily_prompt_state':
      return dailyPromptStateToJson(await dailyPromptState(store, clock))
    case 'submit_daily_quota':
      return dailySessionToJson(await submitDailyQuota(store, field('new_count'), clock))
    case 'dismiss_daily_prompt':
      return dailySessionToJson(await dismissDailyPrompt(store, clock))
    case 'reopen_daily_prompt':
      return reopenDailyPrompt(store, clock)
    case 'record_session_progress':
      return dailySessionToJson(
        await recordSessionProgress(store, field<SessionProgress>('kind'), clock),
      )
    case 'award_xp':
      return awardXp(store, field('amount'), field('reason'), clock)
    case 'apply_streak_freezes':
      return applyStreakFreezes(store, clock)
    case 'on_session_completed':
      return onSessionCompleted(store, clock)
    case 'stats_summary':
      return statsSummary(store, clock)
    case 'load_pool': {
      const pool = await LearnedVocabularyPool.load(store)
      return { entries: pool.entries(), learned_hanzi: sorted(pool.learnedHanziChars()) }
    }
    case 'generate_exercise':
      return generateExercise(
        await LearnedVocabularyPool.load(store),
        field('card_id'),
        field('kind'),
        field('seed'),
      )
    case 'lessons_from_store':
      return lessonsFromStore(store, field('cards_per_lesson'))
    default:
      throw new Error(`the replay does not know ${String(step.op)}`)
  }
}

async function replayScenario(scenario: Scenario): Promise<void> {
  const storage = new MemoryStorage()
  const clock = new FixedClock(parseInstant(scenario.start), scenario.offset_seconds)
  const scheduler = new SrsScheduler()
  for (const [index, step] of scenario.steps.entries()) {
    if (step.op === 'set_clock') {
      clock.set(parseInstant(step.now as string))
      continue
    }
    const result = await outcome(() =>
      storage.transaction((tx) => runOp(new Store(tx), clock, scheduler, step)),
    )
    expectParity(result, step.result, `${scenario.name}, step ${index} (${step.op})`)
  }
}

describe('store-backed scenarios', () => {
  for (const scenario of golden.scenarios) {
    it(scenario.name, () => replayScenario(scenario))
  }
})

type Command = (...args: unknown[]) => Promise<unknown>

async function replayAdapter(scenario: Scenario): Promise<void> {
  const storage = new MemoryStorage()
  const clock = new FixedClock(parseInstant(scenario.start), scenario.offset_seconds)
  const curriculum = (scenario.curriculum ?? []).map((item) =>
    curriculumItemFromJson(item as CurriculumItem),
  )
  const adapter = new WebAdapter({ storage, clock, loadCurriculum: async () => curriculum })
  await adapter.saveSettings({ ...defaultSettings(), meaning_lang: scenario.meaning_lang ?? 'en' })
  const scheduler = new SrsScheduler()
  for (const [index, step] of scenario.steps.entries()) {
    const name = `${scenario.name}, step ${index} (${step.command ?? step.op})`
    if (step.op === 'set_clock') {
      clock.set(parseInstant(step.now as string))
      continue
    }
    if (step.op !== undefined) {
      const result = await outcome(() =>
        storage.transaction((tx) => runOp(new Store(tx), clock, scheduler, step)),
      )
      expectParity(result, step.result, name)
      continue
    }
    const command = (adapter as unknown as Record<string, Command>)[step.command as string]
    const result = await outcome(() => command.apply(adapter, step.args ?? []))
    expectParity(result, step.result, name)
  }
}

describe('Learn commands over the web adapter', () => {
  for (const scenario of golden.adapter) {
    it(scenario.name, () => replayAdapter(scenario))
  }
})
