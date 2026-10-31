/**
 * The made-up history the live demo starts with (docs/live-demo.md): ten
 * days of a learner who studies most evenings, asks for a few new words a
 * day and now and then captures a word in the Automator, in Chinese or in
 * German.
 *
 * The history is played, not written: a {@link WebAdapter} over memory runs
 * the web build's own commands on a clock that walks through the past ten
 * local days, so every row is one the app itself would have written, and
 * the rows of every table are handed on for the demo to store. Ratings come
 * from a seeded generator, so every visitor gets the same history, ending
 * yesterday evening in their own time zone. The last two words were
 * captured after yesterday's session and are still waiting: today's queue
 * offers them before the next HSK words.
 */

import type { IsoDate, Rating } from '../platform/types'
import { WebAdapter } from '../platform/web/adapter'
import { CURRICULUM_FINGERPRINT_KEY } from '../platform/web/curriculum'
import { type Card, type CurriculumItem, newCard } from '../platform/web/model'
import { Mulberry32 } from '../platform/web/rng'
import {
  BufferedStorage,
  MemoryBackend,
  TABLES,
  type Key,
  type Storage,
  type TableName,
} from '../platform/web/storage'
import { Store } from '../platform/web/store'
import { FixedClock, addDays, localDayBounds, nanosFromMillis } from '../platform/web/time'

/** A word captured in the Automator, as the demo's history has it. */
export interface DemoWord {
  /** Days before today; 1 is yesterday. */
  daysAgo: number
  /** Local time of the capture, `HH:MM`. */
  time: string
  lang: 'zh-CN' | 'de'
  front: string
  /** The English translation. */
  back: string
  /** Chinese words only. */
  pinyin?: string
  /** The sentence the word was captured in, and its translation. */
  example: [string, string]
  notes?: string
}

/**
 * The captured words. None is an HSK word, so each is offered as a capture
 * (an HSK word would wait for its turn in the word list), and every hanzi
 * of the Chinese sentences is in a word learned by today, so the sentence
 * builder can use them.
 */
export const DEMO_WORDS: readonly DemoWord[] = [
  {
    daysAgo: 10,
    time: '12:40',
    lang: 'zh-CN',
    front: '电脑包',
    pinyin: 'diàn nǎo bāo',
    back: 'laptop bag',
    example: ['这个电脑包是我的。', 'This laptop bag is mine.'],
  },
  {
    daysAgo: 9,
    time: '08:15',
    lang: 'de',
    front: 'Verspätung',
    back: 'delay',
    example: ['Der Zug hat zehn Minuten Verspätung.', 'The train is ten minutes late.'],
  },
  {
    daysAgo: 8,
    time: '17:50',
    lang: 'de',
    front: 'Feierabend',
    back: 'end of the working day',
    example: ['Nach dem Feierabend gehe ich joggen.', 'After work I go jogging.'],
    notes: 'Schönen Feierabend! – Enjoy your evening!',
  },
  {
    daysAgo: 7,
    time: '13:05',
    lang: 'zh-CN',
    front: '健身房',
    pinyin: 'jiàn shēn fáng',
    back: 'gym',
    example: ['我想去健身房。', 'I want to go to the gym.'],
  },
  {
    daysAgo: 6,
    time: '10:30',
    lang: 'zh-CN',
    front: '雨伞',
    pinyin: 'yǔ sǎn',
    back: 'umbrella',
    example: ['我没有雨伞。', 'I don’t have an umbrella.'],
  },
  {
    daysAgo: 5,
    time: '16:20',
    lang: 'de',
    front: 'gemütlich',
    back: 'cosy',
    example: ['Das Café ist klein und gemütlich.', 'The café is small and cosy.'],
  },
  {
    daysAgo: 3,
    time: '11:45',
    lang: 'zh-CN',
    front: '手机壳',
    pinyin: 'shǒu jī ké',
    back: 'phone case',
    example: ['我要一个手机壳。', 'I want a phone case.'],
  },
  {
    daysAgo: 2,
    time: '14:10',
    lang: 'de',
    front: 'Schnäppchen',
    back: 'bargain',
    example: ['Die Jacke war ein echtes Schnäppchen.', 'The jacket was a real bargain.'],
  },
  {
    daysAgo: 1,
    time: '21:40',
    lang: 'zh-CN',
    front: '咖啡馆',
    pinyin: 'kā fēi guǎn',
    back: 'café',
    example: ['我在咖啡馆等你。', 'I’m waiting for you at the café.'],
  },
  {
    daysAgo: 1,
    time: '21:55',
    lang: 'de',
    front: 'Fernweh',
    back: 'wanderlust',
    example: ['Im Winter habe ich oft Fernweh.', 'In winter I often feel wanderlust.'],
  },
]

/** One evening's session: when it started and how many new words it asked for. */
export interface DemoSession {
  daysAgo: number
  time: string
  /** `null` is "Not today": reviews only. */
  newWords: number | null
}

export const DEMO_SESSIONS: readonly DemoSession[] = [
  { daysAgo: 10, time: '19:30', newWords: 5 },
  { daysAgo: 9, time: '20:10', newWords: 5 },
  { daysAgo: 8, time: '19:05', newWords: 5 },
  { daysAgo: 7, time: '21:00', newWords: 5 },
  { daysAgo: 6, time: '18:40', newWords: 3 },
  { daysAgo: 5, time: '19:50', newWords: 5 },
  { daysAgo: 4, time: '22:15', newWords: null },
  { daysAgo: 3, time: '19:20', newWords: 5 },
  { daysAgo: 2, time: '20:30', newWords: 5 },
  { daysAgo: 1, time: '19:45', newWords: 5 },
]

/**
 * How much of the word list the history is played with. It asks for fewer
 * new words than this in all, and the list is taken in order, so the
 * history never reaches past it; the demo stores the whole list afterwards.
 * Seeding 11,000 rows in memory and reading them for every queue would only
 * make the demo slower to start.
 */
export const CURRICULUM_PREFIX = 100

/** The history's ratings: every visitor's demo starts the same. */
const HISTORY_SEED = 20_261_031
/** A session ends after this many passes over its queue, whatever is left. */
const MAX_PASSES = 4
/** Weights of Again, Hard, Good and Easy: the first time a word is seen, and later. */
const FIRST_LOOK = [10, 15, 65, 10]
const LATER_LOOK = [6, 12, 70, 12]
const NANOS_PER_SECOND = 1_000_000_000n

/** Every table's rows with their keys, as the history left them. */
export type HistoryRows = ReadonlyMap<TableName, readonly [Key, unknown][]>

export interface HistoryOptions {
  /** The learner's local date today; the history ends the evening before. */
  today: IsoDate
  /** Seconds east of UTC, for the whole history. */
  offsetSeconds: number
  /** The bundled curriculum, in `curriculum_order`. */
  curriculum: readonly CurriculumItem[]
}

/**
 * Plays the history and returns its rows: every table but the curriculum,
 * which was only a prefix here, and without the curriculum's fingerprint.
 */
export async function playHistory(options: HistoryOptions): Promise<HistoryRows> {
  const backend = new MemoryBackend()
  const storage = new BufferedStorage(backend)
  const clock = new FixedClock(0n, options.offsetSeconds)
  const prefix = options.curriculum.slice(0, CURRICULUM_PREFIX)
  const app = new WebAdapter({ storage, clock, loadCurriculum: async () => [...prefix] })
  const rng = new Mulberry32(HISTORY_SEED)
  const instant = (daysAgo: number, time: string) =>
    localDayBounds(addDays(options.today, -daysAgo), options.offsetSeconds)[0] +
    minutes(time) * 60_000

  const events = [
    ...DEMO_WORDS.map((word) => ({ at: instant(word.daysAgo, word.time), word })),
    ...DEMO_SESSIONS.map((session) => ({ at: instant(session.daysAgo, session.time), session })),
  ].sort((a, b) => a.at - b.at)
  for (const event of events) {
    clock.set(nanosFromMillis(event.at))
    if ('word' in event) await capture(storage, event.word, event.at)
    else await study(app, clock, rng, event.session.newWords)
  }

  const rows = new Map<TableName, [Key, unknown][]>()
  for (const table of TABLES) {
    if (table === 'hsk_curriculum') continue
    const entries = await backend.getAll(table)
    rows.set(
      table,
      table === 'app_state'
        ? entries.filter(([key]) => key !== CURRICULUM_FINGERPRINT_KEY)
        : entries,
    )
  }
  return rows
}

/** Stores `word` as the Automator would have when it was captured. */
async function capture(storage: Storage, word: DemoWord, capturedAt: number): Promise<void> {
  const card = demoCard(word, capturedAt)
  await storage.transaction((tx) => new Store(tx).insertCapture(card, 'capture', word.daysAgo > 1))
}

export function demoCard(word: DemoWord, capturedAt: number): Card {
  return {
    ...newCard(word.front, word.back, word.lang, 'en', capturedAt),
    example_source: word.example[0],
    example_target: word.example[1],
    notes: word.notes ?? '',
    script_metadata:
      word.pinyin === undefined
        ? { kind: 'none' }
        : { kind: 'chinese', pinyin: word.pinyin, hsk_level: null, script_variant: 'simplified' },
  }
}

/**
 * One evening: the answer to the daily prompt, then passes over the queue
 * as the review screen makes them, each card started when it is reached
 * and rated after a few seconds of thought.
 */
async function study(
  app: WebAdapter,
  clock: FixedClock,
  rng: Mulberry32,
  newWords: number | null,
): Promise<void> {
  if (newWords === null) await app.dismissDailyPrompt()
  else await app.submitDailyQuota(newWords)
  for (let pass = 0; pass < MAX_PASSES; pass += 1) {
    const queue = await app.getDailyQueue()
    if (queue.length === 0) return
    for (const item of queue) {
      const cardId =
        item.kind === 'review'
          ? item.card_id
          : item.kind === 'new'
            ? await app.startNewItem(item.hanzi)
            : await app.startNewCapture(item.card_id)
      clock.advance(BigInt(15 + rng.below(30)) * NANOS_PER_SECOND)
      await app.submitReviewRating(
        cardId,
        rating(rng, item.kind === 'review' ? LATER_LOOK : FIRST_LOOK),
      )
    }
  }
}

function rating(rng: Mulberry32, weights: readonly number[]): Rating {
  let draw = rng.below(weights.reduce((total, weight) => total + weight, 0))
  for (const [index, weight] of weights.entries()) {
    if (draw < weight) return (index + 1) as Rating
    draw -= weight
  }
  return 3
}

/** Minutes after midnight of a `HH:MM` time. */
function minutes(time: string): number {
  const [hours, mins] = time.split(':').map(Number)
  return hours * 60 + mins
}
