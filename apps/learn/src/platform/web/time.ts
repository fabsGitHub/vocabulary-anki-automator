/**
 * Time the way `vocab_core::time`, `vocab_core::srs` (`LocalNow`) and chrono
 * keep it (docs/architecture.md § 2).
 *
 * Everything that depends on "now" or on the learner's calendar takes a
 * {@link Clock}, as in Rust: tests use {@link FixedClock}, the app
 * {@link systemClock}. Three representations are used, each for a reason:
 *
 * - A clock reading is {@link EpochNanos}, a `bigint` of nanoseconds. The core
 *   cuts readings to whole milliseconds before using them, and the golden
 *   vectors pin that cut with sub-millisecond instants that a `number` of
 *   milliseconds could not hold exactly.
 * - Every instant after that cut is a `number` of milliseconds since the
 *   epoch, which is all the store keeps.
 * - Calendar dates are `YYYY-MM-DD` strings, which compare as text in
 *   chronological order for the years 0000–9999 the store accepts; date
 *   arithmetic goes through day numbers.
 *
 * Instants are written in two forms: chrono's serde form for everything the
 * adapter returns (`2026-10-31T09:25:00Z`, with fractional digits only when
 * there are any), because that is what the Tauri commands send; and the
 * store's form with exactly three fractional digits, whose text order is
 * chronological.
 */

import type { IsoDate } from '../types'
import { StoreError } from './errors'
import { trim } from './text'

/** Nanoseconds since 1970-01-01T00:00:00Z. */
export type EpochNanos = bigint

export const MS_PER_DAY = 86_400_000
const NANOS_PER_MS = 1_000_000n

/** `vocab_core::time::Clock`. */
export interface Clock {
  /** The current instant. */
  now(): EpochNanos
  /** Seconds east of UTC used for the learner's calendar date, sampled at "now". */
  localOffset(): number
}

/** The real clock: `Date.now()` and the browser's current UTC offset. */
export const systemClock: Clock = {
  now: () => BigInt(Date.now()) * NANOS_PER_MS,
  localOffset: () => -new Date().getTimezoneOffset() * 60,
}

/** A clock for tests: fixed until explicitly moved. */
export class FixedClock implements Clock {
  private instant: EpochNanos

  constructor(
    now: EpochNanos,
    private readonly offsetSeconds = 0,
  ) {
    this.instant = now
  }

  set(now: EpochNanos): void {
    this.instant = now
  }

  advance(byNanos: bigint): void {
    this.instant += byNanos
  }

  now(): EpochNanos {
    return this.instant
  }

  localOffset(): number {
    return this.offsetSeconds
  }
}

/** A clock frozen at one reading of `clock`, so a whole command sees one instant. */
export function snapshot(clock: Clock): FixedClock {
  return new FixedClock(clock.now(), clock.localOffset())
}

/** `Clock::today`: the learner's local date at `clock.now()`. */
export function today(clock: Clock): IsoDate {
  return localDate(wholeMillis(clock.now()), clock.localOffset())
}

/** `srs::whole_millis`: `at` cut to its whole millisecond (towards the past). */
export function wholeMillis(at: EpochNanos): number {
  const remainder = at % NANOS_PER_MS
  const floored = remainder < 0n ? at - remainder - NANOS_PER_MS : at - remainder
  return Number(floored / NANOS_PER_MS)
}

export function nanosFromMillis(ms: number): EpochNanos {
  return BigInt(ms) * NANOS_PER_MS
}

/** `srs::LocalNow`: one reading of a clock, cut to whole milliseconds, and its local date. */
export interface LocalNow {
  at: number
  today: IsoDate
  offset: number
}

/** `LocalNow::read`: the offset first, then the instant, as in Rust. */
export function readLocalNow(clock: Clock): LocalNow {
  const offset = clock.localOffset()
  const at = wholeMillis(clock.now())
  return { at, today: localDate(at, offset), offset }
}

/** `LocalNow::end_of_today`: the first instant of the next local day. */
export function endOfToday(now: LocalNow): number {
  return localDayBounds(now.today, now.offset)[1]
}

/** `time::local_day_bounds`: `[start, end)` of a local day, in milliseconds. */
export function localDayBounds(date: IsoDate, offsetSeconds: number): [number, number] {
  const start = dateToDays(date) * MS_PER_DAY - offsetSeconds * 1000
  return [start, start + MS_PER_DAY]
}

/** The local date of the instant `ms` at `offsetSeconds` east of UTC. */
export function localDate(ms: number, offsetSeconds: number): IsoDate {
  return daysToDate(Math.floor((ms + offsetSeconds * 1000) / MS_PER_DAY))
}

/** Whole days from `earlier` to `later` (`signed_duration_since(..).num_days()`). */
export function daysBetween(later: IsoDate, earlier: IsoDate): number {
  return dateToDays(later) - dateToDays(earlier)
}

export function addDays(date: IsoDate, days: number): IsoDate {
  return daysToDate(dateToDays(date) + days)
}

/** `chrono::Duration::num_days` of a difference in milliseconds: truncated towards zero. */
export function numDays(diffMs: number): number {
  return (diffMs - (diffMs % MS_PER_DAY)) / MS_PER_DAY
}

/** `time::parse_date`: `YYYY-MM-DD` of a real calendar day, else `null`. */
export function parseDate(text: string): IsoDate | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text)
  if (!match) return null
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])]
  if (month < 1 || month > 12 || day < 1 || day > daysInMonth(year, month)) return null
  return text
}

/** Days since 1970-01-01 of a `YYYY-MM-DD` date. */
export function dateToDays(date: IsoDate): number {
  const [year, month, day] = date.split('-').map(Number)
  return daysFromCivil(year, month, day)
}

/** The `YYYY-MM-DD` date of a day number, written like chrono's `NaiveDate` Display. */
export function daysToDate(days: number): IsoDate {
  const [year, month, day] = civilFromDays(days)
  return `${formatYear(year)}-${pad(month, 2)}-${pad(day, 2)}`
}

/**
 * chrono's serde form of an instant (RFC 3339, `SecondsFormat::AutoSi`, `Z`):
 * the form every Instant the adapter returns has.
 */
export function formatInstant(ms: number): string {
  const days = Math.floor(ms / MS_PER_DAY)
  const ofDay = ms - days * MS_PER_DAY
  const millis = ofDay % 1000
  const fraction = millis === 0 ? '' : `.${pad(millis, 3)}`
  return `${daysToDate(days)}T${clockTime(ofDay)}${fraction}Z`
}

/**
 * `store::codec::format_timestamp`: three fractional digits, refusing years
 * outside 0000–9999, which have no fixed-width form.
 */
export function formatStoredInstant(ms: number): string {
  const days = Math.floor(ms / MS_PER_DAY)
  const year = civilFromDays(days)[0]
  if (year < 0 || year > 9999) {
    const ofDay = ms - days * MS_PER_DAY
    const millis = ofDay % 1000
    const fraction = millis === 0 ? '' : `.${pad(millis, 3)}`
    throw StoreError.invalid(
      `${daysToDate(days)} ${clockTime(ofDay)}${fraction} UTC is outside the years 0000 to 9999 that the database can store`,
    )
  }
  const ofDay = ms - days * MS_PER_DAY
  return `${daysToDate(days)}T${clockTime(ofDay)}.${pad(ofDay % 1000, 3)}Z`
}

const RFC_3339 =
  /^(\d{4})-(\d{2})-(\d{2})[Tt ](\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,9}))?(?:([Zz])|([+-])(\d{2}):(\d{2}))?$/

/**
 * `store::codec::parse_timestamp`: RFC 3339 with up to nine fractional
 * digits, `T` or a space, any offset, and offset-less text taken as UTC.
 * Throws a `StoreError` for anything else.
 */
export function parseInstant(text: string): EpochNanos {
  const trimmed = trim(text)
  const match = RFC_3339.exec(trimmed)
  const invalid = () => StoreError.invalid(`not an RFC 3339 timestamp: ${JSON.stringify(trimmed)}`)
  if (!match) throw invalid()
  const [year, month, day, hour, minute, second] = match.slice(1, 7).map(Number)
  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > daysInMonth(year, month) ||
    hour > 23 ||
    minute > 59 ||
    second > 59
  ) {
    throw invalid()
  }
  let offsetSeconds = 0
  if (match[9]) {
    const sign = match[9] === '-' ? -1 : 1
    offsetSeconds = sign * (Number(match[10]) * 3600 + Number(match[11]) * 60)
  }
  const seconds =
    daysFromCivil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offsetSeconds
  const fraction = BigInt((match[7] ?? '').padEnd(9, '0'))
  return BigInt(seconds) * 1_000_000_000n + fraction
}

/** {@link parseInstant} cut to whole milliseconds. */
export function parseInstantMillis(text: string): number {
  return wholeMillis(parseInstant(text))
}

function clockTime(msOfDay: number): string {
  const seconds = Math.floor(msOfDay / 1000)
  return `${pad(Math.floor(seconds / 3600), 2)}:${pad(Math.floor(seconds / 60) % 60, 2)}:${pad(seconds % 60, 2)}`
}

/** chrono writes years outside 0000–9999 with a sign and at least four digits. */
function formatYear(year: number): string {
  if (year >= 0 && year <= 9999) return pad(year, 4)
  return `${year < 0 ? '-' : '+'}${pad(Math.abs(year), 4)}`
}

function pad(value: number, width: number): string {
  return String(value).padStart(width, '0')
}

function isLeapYear(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0
}

function daysInMonth(year: number, month: number): number {
  if (month === 2) return isLeapYear(year) ? 29 : 28
  return [4, 6, 9, 11].includes(month) ? 30 : 31
}

/** Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`). */
function daysFromCivil(year: number, month: number, day: number): number {
  const y = month <= 2 ? year - 1 : year
  const era = Math.floor(y / 400)
  const yearOfEra = y - era * 400
  const shiftedMonth = (month + 9) % 12
  const dayOfYear = Math.floor((153 * shiftedMonth + 2) / 5) + day - 1
  const dayOfEra =
    yearOfEra * 365 + Math.floor(yearOfEra / 4) - Math.floor(yearOfEra / 100) + dayOfYear
  return era * 146_097 + dayOfEra - 719_468
}

/** The inverse of {@link daysFromCivil} (`civil_from_days`). */
function civilFromDays(days: number): [number, number, number] {
  const z = days + 719_468
  const era = Math.floor(z / 146_097)
  const dayOfEra = z - era * 146_097
  const yearOfEra = Math.floor(
    (dayOfEra -
      Math.floor(dayOfEra / 1460) +
      Math.floor(dayOfEra / 36_524) -
      Math.floor(dayOfEra / 146_096)) /
      365,
  )
  const dayOfYear =
    dayOfEra - (365 * yearOfEra + Math.floor(yearOfEra / 4) - Math.floor(yearOfEra / 100))
  const shiftedMonth = Math.floor((5 * dayOfYear + 2) / 153)
  const day = dayOfYear - Math.floor((153 * shiftedMonth + 2) / 5) + 1
  const month = shiftedMonth < 10 ? shiftedMonth + 3 : shiftedMonth - 9
  const year = yearOfEra + era * 400 + (month <= 2 ? 1 : 0)
  return [year, month, day]
}
