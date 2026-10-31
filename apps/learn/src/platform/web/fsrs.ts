/**
 * rs-fsrs 1.2.1, ported line by line: the FSRS memory model behind every
 * interval Learn schedules (decision 8).
 *
 * The desktop build runs rs-fsrs itself, so this port is only right if it
 * computes what rs-fsrs computes, step for step. It keeps rs-fsrs's
 * structure for that reason: `Parameters`' formulas, a `Scheduler` that
 * prepares the reviewed card, and the basic (short-term learning steps) and
 * long-term schedulers with their state handlers and caches. Fuzz is not
 * ported: vocab_core never enables it, and without it no randomness is
 * involved.
 *
 * Where JavaScript's number semantics differ from Rust's, the Rust ones are
 * reproduced:
 *
 * - `f64::round` rounds halves away from zero (`Math.round` towards +∞).
 * - `f64::max`/`min` ignore a NaN operand (`Math.max` propagates it).
 * - `as i64` truncates and saturates, and turns NaN into 0.
 * - `Duration::num_days` truncates towards zero.
 * - `mul_add` is fused in Rust; here it is a multiply and an add, which can
 *   differ in the last bit. So can `exp` and `powf` between maths
 *   libraries, which is why stability and difficulty are compared with a
 *   tolerance and everything derived from them (intervals, due dates)
 *   exactly.
 *
 * Instants are milliseconds since the epoch: rs-fsrs only adds whole minutes
 * and days to them and counts whole days between them.
 */

import { MS_PER_DAY, numDays } from './time'

/** rs-fsrs `State`, by its stored name. */
export type FsrsState = 'new' | 'learning' | 'review' | 'relearning'

/** rs-fsrs `Rating`: 1 = Again, 2 = Hard, 3 = Good, 4 = Easy. */
export type FsrsRating = 1 | 2 | 3 | 4

/** `Rating::iter()`. */
export const RATINGS: readonly FsrsRating[] = [1, 2, 3, 4]

const AGAIN = 1
const HARD = 2
const GOOD = 3
const EASY = 4

const MS_PER_MINUTE = 60_000
/** The basic scheduler's first step for a new card, by rating. */
const LEARNING_STEP_MINUTES = { [AGAIN]: 1, [HARD]: 5, [GOOD]: 10 } as const

/** rs-fsrs `Card`. */
export interface FsrsCard {
  /** Milliseconds since the epoch. */
  due: number
  stability: number
  difficulty: number
  elapsed_days: number
  scheduled_days: number
  reps: number
  lapses: number
  state: FsrsState
  /** Milliseconds since the epoch. */
  last_review: number
}

/** rs-fsrs `ReviewLog`. */
export interface ReviewLog {
  rating: FsrsRating
  elapsed_days: number
  scheduled_days: number
  state: FsrsState
  /** Milliseconds since the epoch. */
  reviewed_date: number
}

/** rs-fsrs `SchedulingInfo`. */
export interface SchedulingInfo {
  card: FsrsCard
  review_log: ReviewLog
}

/** rs-fsrs `RecordLog`: one outcome per rating. */
export type RecordLog = Map<FsrsRating, SchedulingInfo>

/** rs-fsrs `Parameters` without the fuzz settings. */
export interface Parameters {
  request_retention: number
  maximum_interval: number
  /** The 19 FSRS weights. */
  w: readonly number[]
  /** The basic scheduler (learning steps in minutes) or the long-term one. */
  enable_short_term: boolean
}

/** `DEFAULT_WEIGHTS`. */
export const DEFAULT_WEIGHTS: readonly number[] = [
  0.4072, 1.1829, 3.1262, 15.4722, 7.2102, 0.5316, 1.0651, 0.0234, 1.616, 0.1544, 1.0824, 1.9813,
  0.0953, 0.2975, 2.2042, 0.2407, 2.9466, 0.5034, 0.6567,
]

/** `Parameters::default()`. */
export const DEFAULT_PARAMETERS: Parameters = {
  request_retention: 0.9,
  maximum_interval: 36_500,
  w: DEFAULT_WEIGHTS,
  enable_short_term: true,
}

/** `Parameters::DECAY`. */
export const DECAY = -0.5
/** `Parameters::FACTOR`: (9/10) ^ (1 / DECAY) - 1. */
export const FACTOR = 19 / 81

/** `Parameters::forgetting_curve`. */
export function forgettingCurve(elapsedDays: number, stability: number): number {
  return Math.pow(1 + (FACTOR * elapsedDays) / stability, DECAY)
}

/** `Parameters::init_difficulty`. */
export function initDifficulty(params: Parameters, rating: FsrsRating): number {
  return clamp(params.w[4] - Math.exp(params.w[5] * (rating - 1)) + 1, 1, 10)
}

/** `Parameters::init_stability`. */
export function initStability(params: Parameters, rating: FsrsRating): number {
  return rustMax(params.w[rating - 1], 0.1)
}

/** `Parameters::next_interval` with fuzz disabled. */
export function nextInterval(params: Parameters, stability: number): number {
  const interval = (stability / FACTOR) * (Math.pow(params.request_retention, 1 / DECAY) - 1)
  return clamp(roundHalfAwayFromZero(interval), 1, params.maximum_interval)
}

/** `Parameters::next_difficulty`. */
export function nextDifficulty(params: Parameters, difficulty: number, rating: FsrsRating): number {
  const next = mulAdd(params.w[6], -(rating - 3), difficulty)
  return clamp(meanReversion(params, initDifficulty(params, EASY), next), 1, 10)
}

/** `Parameters::short_term_stability`. */
export function shortTermStability(
  params: Parameters,
  stability: number,
  rating: FsrsRating,
): number {
  return stability * Math.exp(params.w[17] * (rating - 3 + params.w[18]))
}

/** `Parameters::next_recall_stability`. */
export function nextRecallStability(
  params: Parameters,
  difficulty: number,
  stability: number,
  retrievability: number,
  rating: FsrsRating,
): number {
  const { w } = params
  const modifier = rating === HARD ? w[15] : rating === EASY ? w[16] : 1
  const growth =
    Math.exp(w[8]) *
    (11 - difficulty) *
    Math.pow(stability, -w[9]) *
    Math.expm1((1 - retrievability) * w[10])
  return stability * mulAdd(growth, modifier, 1)
}

/** `Parameters::next_forget_stability`. */
export function nextForgetStability(
  params: Parameters,
  difficulty: number,
  stability: number,
  retrievability: number,
): number {
  const { w } = params
  return (
    w[11] *
    Math.pow(difficulty, -w[12]) *
    (Math.pow(stability + 1, w[13]) - 1) *
    Math.exp((1 - retrievability) * w[14])
  )
}

function meanReversion(params: Parameters, initial: number, current: number): number {
  return mulAdd(params.w[7], initial, (1 - params.w[7]) * current)
}

/** `Card::get_retrievability`. */
export function getRetrievability(card: FsrsCard, now: number): number {
  if (card.state === 'new') return 0
  return forgettingCurve(numDays(now - card.last_review), card.stability)
}

/** rs-fsrs `FSRS`: picks the scheduler the parameters ask for. */
export class FSRS {
  constructor(readonly parameters: Parameters = DEFAULT_PARAMETERS) {}

  /** `FSRS::repeat`: every rating's outcome. */
  repeat(card: FsrsCard, now: number): RecordLog {
    return this.scheduler(card, now).preview()
  }

  /** `FSRS::next`: the outcome of `rating`. */
  next(card: FsrsCard, now: number, rating: FsrsRating): SchedulingInfo {
    return this.scheduler(card, now).review(rating)
  }

  private scheduler(card: FsrsCard, now: number): Scheduler {
    return this.parameters.enable_short_term
      ? new BasicScheduler(this.parameters, card, now)
      : new LongtermScheduler(this.parameters, card, now)
  }
}

/** rs-fsrs `Scheduler` and its `ImplScheduler::preview`. */
abstract class Scheduler {
  /** The card as it was before this review. */
  protected readonly last: FsrsCard
  /** The card being reviewed now: one more repetition, reviewed at `now`. */
  protected readonly current: FsrsCard
  protected readonly next: RecordLog = new Map()

  constructor(
    protected readonly params: Parameters,
    card: FsrsCard,
    protected readonly now: number,
  ) {
    this.last = { ...card }
    this.current = {
      ...card,
      elapsed_days: card.state === 'new' ? 0 : numDays(now - card.last_review),
      last_review: now,
      reps: card.reps + 1,
    }
  }

  abstract review(rating: FsrsRating): SchedulingInfo

  preview(): RecordLog {
    return new Map(RATINGS.map((rating) => [rating, this.review(rating)]))
  }

  protected buildLog(rating: FsrsRating): ReviewLog {
    return {
      rating,
      state: this.current.state,
      elapsed_days: this.current.elapsed_days,
      scheduled_days: this.current.scheduled_days,
      reviewed_date: this.now,
    }
  }

  protected cached(rating: FsrsRating): SchedulingInfo | null {
    const exist = this.next.get(rating)
    return exist ? cloneInfo(exist) : null
  }

  protected remember(rating: FsrsRating, card: FsrsCard): SchedulingInfo {
    const item = { card, review_log: this.buildLog(rating) }
    this.next.set(rating, cloneInfo(item))
    return item
  }

  /** `next_difficulty_stability`, shared by both schedulers. */
  protected nextDifficultyStability(
    cards: Record<FsrsRating, FsrsCard>,
    difficulty: number,
    stability: number,
    retrievability: number,
  ): void {
    const { params } = this
    cards[AGAIN].difficulty = nextDifficulty(params, difficulty, AGAIN)
    cards[AGAIN].stability = nextForgetStability(params, difficulty, stability, retrievability)
    for (const rating of [HARD, GOOD, EASY] as const) {
      cards[rating].difficulty = nextDifficulty(params, difficulty, rating)
      cards[rating].stability = nextRecallStability(
        params,
        difficulty,
        stability,
        retrievability,
        rating,
      )
    }
  }

  /** `self.scheduler.now + Duration::days(interval as i64)`. */
  protected schedule(card: FsrsCard, interval: number): void {
    card.scheduled_days = asI64(interval)
    card.due = this.now + card.scheduled_days * MS_PER_DAY
  }

  /** Four copies of `current`, one per rating. */
  protected copies(): Record<FsrsRating, FsrsCard> {
    return {
      [AGAIN]: { ...this.current },
      [HARD]: { ...this.current },
      [GOOD]: { ...this.current },
      [EASY]: { ...this.current },
    }
  }
}

/** rs-fsrs `BasicScheduler`: learning and relearning steps in minutes. */
class BasicScheduler extends Scheduler {
  review(rating: FsrsRating): SchedulingInfo {
    switch (this.last.state) {
      case 'new':
        return this.newState(rating)
      case 'learning':
      case 'relearning':
        return this.learningState(rating)
      case 'review':
        return this.reviewState(rating)
    }
  }

  private newState(rating: FsrsRating): SchedulingInfo {
    const exist = this.cached(rating)
    if (exist) return exist
    const next = { ...this.current }
    next.difficulty = initDifficulty(this.params, rating)
    next.stability = initStability(this.params, rating)
    switch (rating) {
      case AGAIN:
      case HARD:
      case GOOD:
        next.scheduled_days = 0
        next.due = this.now + LEARNING_STEP_MINUTES[rating] * MS_PER_MINUTE
        next.state = 'learning'
        break
      case EASY:
        this.schedule(next, nextInterval(this.params, next.stability))
        next.state = 'review'
        break
    }
    return this.remember(rating, next)
  }

  private learningState(rating: FsrsRating): SchedulingInfo {
    const exist = this.cached(rating)
    if (exist) return exist
    const { params, last } = this
    const next = { ...this.current }
    next.difficulty = nextDifficulty(params, last.difficulty, rating)
    next.stability = shortTermStability(params, last.stability, rating)
    switch (rating) {
      case AGAIN:
      case HARD:
        next.scheduled_days = 0
        next.due = this.now + (rating === AGAIN ? 5 : 10) * MS_PER_MINUTE
        next.state = last.state
        break
      case GOOD:
        this.schedule(next, nextInterval(params, next.stability))
        next.state = 'review'
        break
      case EASY: {
        const goodStability = shortTermStability(params, last.stability, GOOD)
        const goodInterval = nextInterval(params, goodStability)
        this.schedule(next, rustMax(nextInterval(params, next.stability), goodInterval + 1))
        next.state = 'review'
        break
      }
    }
    return this.remember(rating, next)
  }

  private reviewState(rating: FsrsRating): SchedulingInfo {
    const exist = this.cached(rating)
    if (exist) return exist
    const { params, last } = this
    const retrievability = getRetrievability(last, this.now)
    const cards = this.copies()
    this.nextDifficultyStability(cards, last.difficulty, last.stability, retrievability)

    let hardInterval = nextInterval(params, cards[HARD].stability)
    let goodInterval = nextInterval(params, cards[GOOD].stability)
    hardInterval = rustMin(hardInterval, goodInterval)
    goodInterval = rustMax(goodInterval, hardInterval + 1)
    const easyInterval = rustMax(nextInterval(params, cards[EASY].stability), goodInterval + 1)
    cards[AGAIN].scheduled_days = 0
    cards[AGAIN].due = this.now + 5 * MS_PER_MINUTE
    this.schedule(cards[HARD], hardInterval)
    this.schedule(cards[GOOD], goodInterval)
    this.schedule(cards[EASY], easyInterval)

    cards[AGAIN].state = 'relearning'
    cards[HARD].state = 'review'
    cards[GOOD].state = 'review'
    cards[EASY].state = 'review'
    cards[AGAIN].lapses += 1

    for (const each of RATINGS) this.remember(each, cards[each])
    return this.cached(rating) as SchedulingInfo
  }
}

/** rs-fsrs `LongtermScheduler`: every outcome is a review in days. */
class LongtermScheduler extends Scheduler {
  review(rating: FsrsRating): SchedulingInfo {
    return this.last.state === 'new' ? this.newState(rating) : this.reviewState(rating)
  }

  private newState(rating: FsrsRating): SchedulingInfo {
    const exist = this.cached(rating)
    if (exist) return exist
    // The copies keep `current`'s days; only the logs see them zeroed.
    const cards = this.copies()
    this.current.scheduled_days = 0
    this.current.elapsed_days = 0
    for (const each of RATINGS) {
      cards[each].difficulty = initDifficulty(this.params, each)
      cards[each].stability = initStability(this.params, each)
    }
    this.nextIntervals(cards)
    this.updateNext(cards)
    return this.cached(rating) as SchedulingInfo
  }

  /** `learning_state` delegates here too. */
  private reviewState(rating: FsrsRating): SchedulingInfo {
    const exist = this.cached(rating)
    if (exist) return exist
    const { last } = this
    const retrievability = getRetrievability(last, this.now)
    const cards = this.copies()
    this.nextDifficultyStability(cards, last.difficulty, last.stability, retrievability)
    this.nextIntervals(cards)
    cards[AGAIN].lapses += 1
    this.updateNext(cards)
    return this.cached(rating) as SchedulingInfo
  }

  /** `next_interval` and `next_state`: ordered intervals, all in the review state. */
  private nextIntervals(cards: Record<FsrsRating, FsrsCard>): void {
    const { params } = this
    let again = nextInterval(params, cards[AGAIN].stability)
    let hard = nextInterval(params, cards[HARD].stability)
    let good = nextInterval(params, cards[GOOD].stability)
    let easy = nextInterval(params, cards[EASY].stability)
    again = rustMin(again, hard)
    hard = rustMax(hard, again + 1)
    good = rustMax(good, hard + 1)
    easy = rustMax(easy, good + 1)
    this.schedule(cards[AGAIN], again)
    this.schedule(cards[HARD], hard)
    this.schedule(cards[GOOD], good)
    this.schedule(cards[EASY], easy)
    for (const each of RATINGS) cards[each].state = 'review'
  }

  private updateNext(cards: Record<FsrsRating, FsrsCard>): void {
    for (const each of RATINGS) this.remember(each, cards[each])
  }
}

function cloneInfo(info: SchedulingInfo): SchedulingInfo {
  return { card: { ...info.card }, review_log: { ...info.review_log } }
}

/** `f64::mul_add`, unfused: see the module docs. */
function mulAdd(a: number, b: number, c: number): number {
  return a * b + c
}

/** `f64::clamp`: NaN stays NaN. */
function clamp(value: number, min: number, max: number): number {
  if (value < min) return min
  if (value > max) return max
  return value
}

/** `f64::round`. */
export function roundHalfAwayFromZero(value: number): number {
  return value < 0 ? -Math.round(-value) : Math.round(value)
}

/** `f64::max`: a NaN operand loses. */
export function rustMax(a: number, b: number): number {
  if (Number.isNaN(a)) return b
  if (Number.isNaN(b)) return a
  return Math.max(a, b)
}

/** `f64::min`: a NaN operand loses. */
export function rustMin(a: number, b: number): number {
  if (Number.isNaN(a)) return b
  if (Number.isNaN(b)) return a
  return Math.min(a, b)
}

/** The magnitude of `i64::MIN`; also the number nearest to `i64::MAX`. */
const I64_LIMIT = 2 ** 63

/** `value as i64`: truncation, saturation, NaN as 0. */
export function asI64(value: number): number {
  if (Number.isNaN(value)) return 0
  return Math.trunc(Math.min(Math.max(value, -I64_LIMIT), I64_LIMIT))
}
