/**
 * Learn's TypeScript side of the IPC contract (docs/architecture.md § 10,
 * § 11), checked against what the Rust side really does.
 *
 * `__fixtures__/ipc_shapes.json` is written by
 * `apps/learn/src-tauri/tests/ipc_shapes.rs` from the Rust types and the real
 * invoke handler. Here every sample must pass a schema whose type is
 * compile-time equal to its interface in `./types.ts` (exact keys, literal
 * values, nullability); every command and argument name the backend reads
 * must be what the Tauri adapter sends for each `PlatformAdapter` method,
 * and its result type what the method promises; and the web adapter's
 * parsers and serializers must read every sample and write it back exactly
 * as serde does, so both adapters hand the views the same values.
 */

import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import architecture from '../../../../docs/architecture.md?raw'
import { EXERCISE_KINDS as OFFERED_KINDS } from '../domain/exercises'
import {
  array,
  base64,
  boolean,
  documentedCommands,
  instant,
  integer,
  isoDate,
  literal,
  nullable,
  number,
  object,
  string,
  union,
  type Equals,
  type Mismatches,
} from '../test/shape'
import fixture from './__fixtures__/ipc_shapes.json'
import { TauriAdapter } from './tauri'
import type * as T from './types'
import { dailyPromptStateToJson } from './web/daily'
import { EXERCISE_KINDS, checkAnswer } from './web/exercises'
import { cardFromJson, cardToJson, storedCaptureToJson } from './web/model'
import { queueItemToJson } from './web/queue'
import {
  ratingPreviewToJson,
  reviewRating,
  srsStateFromJson,
  srsStateToJson,
} from './web/scheduler'
import { reviewResultToJson } from './web/session'
import { defaultSettings, normalizeSettings, parseSettings } from './web/settings'
import { dailySessionFromJson, dailySessionToJson } from './web/store'
import { parseInstantMillis } from './web/time'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const invokeMock = vi.mocked(invoke)

// ---------------------------------------------------------------------------
// One schema per type of ./types.ts

const scriptMetadata = union(
  object({ kind: literal('none') }),
  object({
    kind: literal('chinese'),
    pinyin: string,
    hsk_level: nullable(integer),
    script_variant: literal('simplified', 'traditional'),
  }),
)
const card = object({
  front: string,
  back: string,
  example_source: string,
  example_target: string,
  notes: string,
  tags: array(string),
  source_lang: string,
  target_lang: string,
  captured_at: instant,
  audio_front: nullable(base64),
  script_metadata: scriptMetadata,
})
const storedCapture = object({
  id: integer,
  card,
  origin: literal('capture', 'curriculum', 'anki_import'),
  synced_to_anki: boolean,
})
const dailySession = object({
  session_date: isoDate,
  requested_new_count: integer,
  reviews_due_count: integer,
  reviews_completed: integer,
  new_items_completed: integer,
  completed_at: nullable(instant),
})
const dailyPromptState = object({
  needs_prompt: boolean,
  today: isoDate,
  session: nullable(dailySession),
})
const queueItem = union(
  object({ kind: literal('review'), card_id: integer, due_date: instant }),
  object({ kind: literal('new'), hanzi: string, curriculum_order: integer }),
  object({ kind: literal('new_capture'), card_id: integer }),
)
const rating = literal(1, 2, 3, 4)
const ratingPreview = object({ rating, due_date: instant, interval_days: integer })
const srsState = object({
  card_id: integer,
  algorithm: string,
  stability: nullable(number),
  difficulty: nullable(number),
  interval_days: integer,
  repetitions: integer,
  lapses: integer,
  fsrs_state: literal('new', 'learning', 'review', 'relearning'),
  elapsed_days: integer,
  due_date: instant,
  last_reviewed_at: nullable(instant),
  last_rating: nullable(rating),
})
const streakSummary = object({
  current: integer,
  longest: integer,
  freezes_available: integer,
  completed_today: boolean,
})
const statsSummary = object({
  xp_total: integer,
  xp_today: integer,
  streak: streakSummary,
  learned_count: integer,
  due_today: integer,
})
const reviewResult = object({
  next: srsState,
  xp_awarded: integer,
  session: nullable(dailySession),
  session_completed: boolean,
  streak: streakSummary,
})
const learnedCard = object({
  card_id: integer,
  front: string,
  back: string,
  pinyin: nullable(string),
  hsk_level: nullable(integer),
  source_lang: string,
  target_lang: string,
  example_source: string,
  example_target: string,
  script_kind: string,
  has_audio: boolean,
})
const exerciseKind = literal(
  'multiple_choice',
  'sentence_builder',
  'listening_choice',
  'listening_sentence',
)
const choiceDirection = literal('front_to_back', 'back_to_front', 'front_to_pinyin')
const choiceOption = object({ card_id: integer, text: string })
const audioPrompt = object({ audio_card_id: nullable(integer), text: string, lang: string })
const tile = object({ id: integer, text: string })
const multipleChoiceExercise = object({
  kind: literal('multiple_choice'),
  card_id: integer,
  direction: choiceDirection,
  prompt: string,
  prompt_lang: string,
  option_lang: string,
  options: array(choiceOption),
  correct_index: integer,
})
const sentenceBuilderExercise = object({
  kind: literal('sentence_builder'),
  card_id: integer,
  sentence_card_id: integer,
  prompt: string,
  sentence: string,
  sentence_lang: string,
  tiles: array(tile),
  solution: array(integer),
})
const listeningChoiceExercise = object({
  kind: literal('listening_choice'),
  card_id: integer,
  audio: audioPrompt,
  options: array(choiceOption),
  correct_index: integer,
})
const listeningSentenceExercise = object({
  kind: literal('listening_sentence'),
  card_id: integer,
  sentence_card_id: integer,
  audio: audioPrompt,
  sentence: string,
  tiles: array(tile),
  solution: array(integer),
})
const exercise = union(
  multipleChoiceExercise,
  sentenceBuilderExercise,
  listeningChoiceExercise,
  listeningSentenceExercise,
)
const exerciseAnswer = union(
  object({ kind: literal('choice'), index: integer }),
  object({ kind: literal('order'), tile_ids: array(integer) }),
)
const lessonCard = object({
  card_id: integer,
  word: string,
  translation: string,
  sentence_source: string,
  sentence_target: string,
  cloze: string,
  pinyin: nullable(string),
})
const lesson = object({
  id: string,
  title: string,
  source_lang: string,
  target_lang: string,
  cards: array(lessonCard),
})
const cardField = literal(
  'front',
  'back',
  'example_source',
  'example_target',
  'example',
  'notes',
  'pinyin',
  'hsk_level',
)
const fieldMapping = object({
  fields: array(object({ anki_field: string, source: cardField })),
  audio_field: nullable(string),
})
const learnSettings = object({
  meaning_lang: literal('de', 'en'),
  anki: object({
    connect_url: string,
    deck_name: string,
    model_name: string,
    field_mapping: fieldMapping,
    create_reverse: boolean,
  }),
})
const ankiSyncReport = object({
  attempted: integer,
  succeeded: integer,
  skipped: integer,
  failed: array(object({ card_id: nullable(integer), front: string, error: string })),
})

/** The schemas the fixture has samples for, by type name. */
const SCHEMAS = {
  ScriptMetadata: scriptMetadata,
  Card: card,
  StoredCapture: storedCapture,
  DailySession: dailySession,
  DailyPromptState: dailyPromptState,
  QueueItem: queueItem,
  Rating: rating,
  RatingPreview: ratingPreview,
  SrsState: srsState,
  StreakSummary: streakSummary,
  StatsSummary: statsSummary,
  ReviewResult: reviewResult,
  LearnedCard: learnedCard,
  ExerciseKind: exerciseKind,
  ChoiceDirection: choiceDirection,
  Exercise: exercise,
  ExerciseAnswer: exerciseAnswer,
  Lesson: lesson,
  CardField: cardField,
  FieldMapping: fieldMapping,
  LearnSettings: learnSettings,
  AnkiSyncReport: ankiSyncReport,
}

/** The parts the samples above are made of, checked through their wholes. */
const PART_SCHEMAS = {
  ChoiceOption: choiceOption,
  AudioPrompt: audioPrompt,
  Tile: tile,
  MultipleChoiceExercise: multipleChoiceExercise,
  SentenceBuilderExercise: sentenceBuilderExercise,
  ListeningChoiceExercise: listeningChoiceExercise,
  ListeningSentenceExercise: listeningSentenceExercise,
  LessonCard: lessonCard,
}

/** The interfaces of ./types.ts the schemas stand for, by name. */
interface Types {
  ScriptMetadata: T.ScriptMetadata
  Card: T.Card
  StoredCapture: T.StoredCapture
  DailySession: T.DailySession
  DailyPromptState: T.DailyPromptState
  QueueItem: T.QueueItem
  Rating: T.Rating
  RatingPreview: T.RatingPreview
  SrsState: T.SrsState
  StreakSummary: T.StreakSummary
  StatsSummary: T.StatsSummary
  ReviewResult: T.ReviewResult
  LearnedCard: T.LearnedCard
  ExerciseKind: T.ExerciseKind
  ChoiceDirection: T.ChoiceDirection
  Exercise: T.Exercise
  ExerciseAnswer: T.ExerciseAnswer
  Lesson: T.Lesson
  CardField: T.CardField
  FieldMapping: T.FieldMapping
  LearnSettings: T.LearnSettings
  AnkiSyncReport: T.AnkiSyncReport
}

interface PartTypes {
  ChoiceOption: T.ChoiceOption
  AudioPrompt: T.AudioPrompt
  Tile: T.Tile
  MultipleChoiceExercise: T.MultipleChoiceExercise
  SentenceBuilderExercise: T.SentenceBuilderExercise
  ListeningChoiceExercise: T.ListeningChoiceExercise
  ListeningSentenceExercise: T.ListeningSentenceExercise
  LessonCard: T.LessonCard
}

// A compile error here names the type whose schema differs from its interface.
const schemasMatchTypes: [Mismatches<typeof SCHEMAS, Types>] extends [never]
  ? true
  : Mismatches<typeof SCHEMAS, Types> = true
const partsMatchTypes: [Mismatches<typeof PART_SCHEMAS, PartTypes>] extends [never]
  ? true
  : Mismatches<typeof PART_SCHEMAS, PartTypes> = true

type ShapeName = keyof typeof SCHEMAS
const samples = fixture.shapes as Record<ShapeName, unknown[]>
const all = <K extends ShapeName>(name: K): Types[K][] =>
  samples[name].map((value) => SCHEMAS[name].check(value) as Types[K])

// ---------------------------------------------------------------------------
// The commands behind the PlatformAdapter methods, and their result types

type Method = Exclude<keyof T.PlatformAdapter, 'capabilities'>

/**
 * The result types of the § 10 table, by the name the fixture spells them
 * with. The Tauri adapter resolves a command's `null` as `undefined`, so
 * `null` stands for `void` here.
 */
interface Results {
  DailyPromptState: T.DailyPromptState
  DailySession: T.DailySession
  null: void
  'QueueItem[]': T.QueueItem[]
  number: number
  StoredCapture: T.StoredCapture
  'RatingPreview[]': T.RatingPreview[]
  ReviewResult: T.ReviewResult
  'LearnedCard[]': T.LearnedCard[]
  Exercise: T.Exercise
  boolean: boolean
  'Lesson[]': T.Lesson[]
  StatsSummary: T.StatsSummary
  'string | null': string | null
  LearnSettings: T.LearnSettings
  AnkiSyncReport: T.AnkiSyncReport
}

const RETURNS = {
  getDailyPromptState: 'DailyPromptState',
  submitDailyQuota: 'DailySession',
  dismissDailyPrompt: 'DailySession',
  reopenDailyPrompt: 'null',
  getDailyQueue: 'QueueItem[]',
  startNewItem: 'number',
  startNewCapture: 'number',
  getCard: 'StoredCapture',
  previewRatings: 'RatingPreview[]',
  submitReviewRating: 'ReviewResult',
  listLearnedCards: 'LearnedCard[]',
  nextExercise: 'Exercise',
  checkExercise: 'boolean',
  recordExerciseResult: 'number',
  listLessons: 'Lesson[]',
  completeLesson: 'number',
  getStats: 'StatsSummary',
  getCardAudio: 'string | null',
  synthesizeAudio: 'string | null',
  getSettings: 'LearnSettings',
  saveSettings: 'null',
  ankiExport: 'AnkiSyncReport',
  ankiImport: 'AnkiSyncReport',
} as const satisfies Record<Method, keyof Results>

type WrongResults = {
  [M in Method]: Equals<
    Results[(typeof RETURNS)[M]],
    Awaited<ReturnType<T.PlatformAdapter[M]>>
  > extends true
    ? never
    : M
}[Method]
// A compile error here names the method whose result type is not the backend's.
const resultsMatch: [WrongResults] extends [never] ? true : WrongResults = true

/** Arguments of each method's parameter types, in parameter order. */
const ARGS: { [M in Method]: Parameters<T.PlatformAdapter[M]> } = {
  getDailyPromptState: [],
  submitDailyQuota: [7],
  dismissDailyPrompt: [],
  reopenDailyPrompt: [],
  getDailyQueue: [],
  startNewItem: ['学习'],
  startNewCapture: [14],
  getCard: [12],
  previewRatings: [12],
  submitReviewRating: [12, 3],
  listLearnedCards: [],
  nextExercise: [12, 'listening_sentence', 4_000_000_000],
  checkExercise: [all('Exercise')[1], all('ExerciseAnswer')[1]],
  recordExerciseResult: [12, 'sentence_builder', true],
  listLessons: [],
  completeLesson: ['lesson-7'],
  getStats: [],
  getCardAudio: [12],
  synthesizeAudio: ['我学习中文。', 'zh-CN'],
  getSettings: [],
  saveSettings: [all('LearnSettings')[1]],
  ankiExport: [],
  ankiImport: ['Chinese::HSK'],
}

/** An answer that solves `exercise`. */
function solution(exercise: T.Exercise): T.ExerciseAnswer {
  return exercise.kind === 'multiple_choice' || exercise.kind === 'listening_choice'
    ? { kind: 'choice', index: exercise.correct_index }
    : { kind: 'order', tile_ids: [...exercise.solution] }
}

beforeEach(() => {
  invokeMock.mockReset()
})

describe('the Rust shapes of Learn', () => {
  it('match the TypeScript types at compile time', () => {
    expect([schemasMatchTypes, partsMatchTypes, resultsMatch]).toEqual([true, true, true])
  })

  it('cover the types of ./types.ts that cross the IPC boundary', () => {
    expect(Object.keys(fixture.shapes).sort()).toEqual(Object.keys(SCHEMAS).sort())
  })

  it.each(Object.keys(SCHEMAS) as ShapeName[])(
    '%s: every sample has the TypeScript shape',
    (name) => {
      expect(samples[name].length).toBeGreaterThan(0)
      for (const value of samples[name]) expect(() => SCHEMAS[name].check(value)).not.toThrow()
    },
  )

  it('include every variant of the unions', () => {
    expect(new Set(all('Exercise').map((value) => value.kind))).toEqual(
      new Set(all('ExerciseKind')),
    )
    expect(new Set(all('ExerciseAnswer').map((value) => value.kind))).toEqual(
      new Set(['choice', 'order']),
    )
    expect(new Set(all('QueueItem').map((value) => value.kind))).toEqual(
      new Set(['review', 'new', 'new_capture']),
    )
    expect(all('Rating')).toEqual([1, 2, 3, 4])
  })

  it('match the interface of their own exercise kind and lesson card', () => {
    const byKind = {
      multiple_choice: PART_SCHEMAS.MultipleChoiceExercise,
      sentence_builder: PART_SCHEMAS.SentenceBuilderExercise,
      listening_choice: PART_SCHEMAS.ListeningChoiceExercise,
      listening_sentence: PART_SCHEMAS.ListeningSentenceExercise,
    }
    for (const value of all('Exercise')) {
      expect(() => byKind[value.kind].check(value)).not.toThrow()
    }
    for (const value of all('Lesson').flatMap((lesson) => lesson.cards)) {
      expect(() => PART_SCHEMAS.LessonCard.check(value)).not.toThrow()
    }
  })
})

describe('the Tauri adapter', () => {
  it('has a method for exactly the backend commands', () => {
    const methods = Object.getOwnPropertyNames(TauriAdapter.prototype).filter(
      (name) => name !== 'constructor',
    )
    expect(methods.sort()).toEqual(fixture.commands.map((command) => command.method).sort())
    expect(Object.keys(RETURNS).sort()).toEqual(methods)
  })

  it.each(fixture.commands)(
    '$method invokes $command with the arguments the backend reads',
    async ({ method, command, args, returns }) => {
      invokeMock.mockResolvedValueOnce(null)
      const given = ARGS[method as Method] as unknown[]
      const adapter = new TauriAdapter() as unknown as Record<string, (...a: unknown[]) => unknown>
      await adapter[method](...given)
      const named = Object.fromEntries(args.map((name, index) => [name, given[index]]))
      expect(invokeMock.mock.calls).toEqual([args.length === 0 ? [command] : [command, named]])
      expect(RETURNS[method as Method]).toBe(returns)
    },
  )
})

describe('docs/architecture.md § 10', () => {
  it('documents every command with the backend’s arguments and result type', () => {
    const byCommand = (a: { command: string }, b: { command: string }) =>
      a.command.localeCompare(b.command)
    expect(documentedCommands(architecture, '## 10. Learn').sort(byCommand)).toEqual(
      fixture.commands
        .map(({ command, args, returns }) => ({ command, args, returns }))
        .sort(byCommand),
    )
  })
})

describe('the web adapter reads and writes the Rust shapes', () => {
  it('starts from the backend defaults and offers the backend exercise kinds', () => {
    expect(defaultSettings()).toEqual(samples.LearnSettings[0])
    expect(new Set(EXERCISE_KINDS)).toEqual(new Set(samples.ExerciseKind))
    expect(new Set(OFFERED_KINDS)).toEqual(new Set(samples.ExerciseKind))
  })

  it('round-trips cards and stored captures', () => {
    for (const value of all('Card')) expect(cardToJson(cardFromJson(value))).toEqual(value)
    for (const value of all('StoredCapture')) {
      expect(storedCaptureToJson({ ...value, card: cardFromJson(value.card) })).toEqual(value)
    }
  })

  it('round-trips sessions, prompt states, states and review results', () => {
    for (const value of all('DailySession')) {
      expect(dailySessionToJson(dailySessionFromJson(value))).toEqual(value)
    }
    for (const value of all('DailyPromptState')) {
      const session = value.session === null ? null : dailySessionFromJson(value.session)
      expect(dailyPromptStateToJson({ ...value, session })).toEqual(value)
    }
    for (const value of all('SrsState')) {
      expect(srsStateToJson(srsStateFromJson(value))).toEqual(value)
    }
    for (const value of all('ReviewResult')) {
      const internal = {
        ...value,
        next: srsStateFromJson(value.next),
        session: value.session === null ? null : dailySessionFromJson(value.session),
      }
      expect(reviewResultToJson(internal)).toEqual(value)
    }
  })

  it('writes queue items and rating previews as serde does', () => {
    for (const value of all('QueueItem')) {
      const internal =
        value.kind === 'review' ? { ...value, due_date: parseInstantMillis(value.due_date) } : value
      expect(queueItemToJson(internal)).toEqual(value)
    }
    for (const value of all('RatingPreview')) {
      const internal = {
        ...value,
        rating: reviewRating(value.rating),
        due_date: parseInstantMillis(value.due_date),
      }
      expect(ratingPreviewToJson(internal)).toEqual(value)
    }
  })

  it('accepts every settings value the desktop app stores', () => {
    for (const value of all('LearnSettings')) {
      expect(parseSettings(value)).toEqual(value)
      expect(normalizeSettings(parseSettings(value))).toEqual(value)
    }
  })

  it('checks answers to every exercise the core generates', () => {
    for (const value of all('Exercise')) {
      expect(checkAnswer(value, solution(value))).toBe(true)
      const wrong: T.ExerciseAnswer =
        value.kind === 'multiple_choice' || value.kind === 'listening_choice'
          ? { kind: 'choice', index: value.options.length }
          : { kind: 'order', tile_ids: [] }
      expect(checkAnswer(value, wrong)).toBe(false)
    }
  })
})
