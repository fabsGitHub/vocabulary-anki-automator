/**
 * `vocab_core::store::Store` over a {@link Transaction}: the tables, rows
 * and rules of the SQLite store (docs/architecture.md § 6), so that the
 * ported core reads and writes what the Rust core would.
 *
 * - **Rows keep the SQL encodings.** Instants are stored as RFC 3339 text
 *   with exactly three fractional digits and dates as `YYYY-MM-DD`, compared
 *   and ordered as text the way the SQL does; a year outside 0000–9999 is
 *   refused on write (`StoreError::Invalid`), and a stored value is decoded
 *   strictly on read, an unknown `script_kind` or `origin` included.
 * - **SQLite's constraints hold here too**, with SQLite's messages: unique
 *   curriculum hanzi and orders, `srs_state` and `review_log` rows only for
 *   existing captures, and deleting a capture deletes both with it.
 * - **Ids are `AUTOINCREMENT` ids**, never reused, and a rolled-back insert
 *   gives its id back, as SQLite's `sqlite_sequence` does.
 * - **{@link Store.withTransaction} nests as a savepoint.** The outermost
 *   transaction is the storage transaction the caller opened; an inner
 *   failure undoes only the inner writes.
 */

import type * as ipc from '../types'
import { CURRICULUM_FINGERPRINT_KEY, curriculumFingerprint } from './curriculum'
import { StoreError } from './errors'
import type { CaptureOrigin, Card, CurriculumItem, ScriptMetadata, StoredCapture } from './model'
import type { ReviewLogRow, SrsStateRow } from './scheduler'
import type { Transaction } from './storage'
import { debugString, trim } from './text'
import {
  dateYear,
  formatExclusiveBound,
  formatInstant,
  formatStoredInstant,
  isStorableYear,
  parseDate,
  parseInstantMillis,
} from './time'

/** `store::progress::DailySessionRow`, `completed_at` in milliseconds. */
export interface DailySessionRow {
  session_date: ipc.IsoDate
  requested_new_count: number
  reviews_due_count: number
  reviews_completed: number
  new_items_completed: number
  completed_at: number | null
}

/** The columns `LearnedVocabularyPool::QUERY` selects. */
export interface LearnedVocabularyRow {
  id: number
  front: string
  back: string
  pinyin: string | null
  hsk_level: number | null
  source_lang: string
  target_lang: string
  example_source: string
  example_target: string
  script_kind: string
  /** `audio_front IS NOT NULL AND length(audio_front) > 0`. */
  has_audio: boolean
}

/** A `captures` row as stored. */
interface CaptureRecord {
  id: number
  front: string
  back: string
  example_source: string
  example_target: string
  notes: string
  tags: string[]
  source_lang: string
  target_lang: string
  captured_at: string
  synced_to_anki: boolean
  pinyin: string | null
  hsk_level: number | null
  script_kind: string
  script_variant: string | null
  origin: string
  audio_front: Uint8Array | null
}

/** An `srs_state` row as stored. */
interface SrsStateRecord extends Omit<SrsStateRow, 'due_date' | 'last_reviewed_at'> {
  due_date: string
  last_reviewed_at: string | null
}

/** A `review_log` row as stored. */
interface ReviewLogRecord extends Omit<ReviewLogRow, 'reviewed_at'> {
  reviewed_at: string
}

/** A `daily_sessions` row as stored. */
interface DailySessionRecord extends Omit<DailySessionRow, 'completed_at'> {
  completed_at: string | null
}

/** An `xp_ledger` row as stored. */
interface XpRecord {
  id: number
  session_date: string
  amount: number
  reason: string
  created_at: string
}

const FOREIGN_KEY_FAILED = 'FOREIGN KEY constraint failed'

/** The store, inside one storage transaction. */
export class Store {
  constructor(private readonly tx: Transaction) {}

  /**
   * `Store::with_transaction`: runs `f` atomically. Inside a storage
   * transaction (always, here) that is a savepoint: when `f` throws, its
   * writes are undone and the error goes on to the caller.
   */
  async withTransaction<T>(f: (store: Store) => Promise<T>): Promise<T> {
    const savepoint = this.tx.savepoint()
    try {
      return await f(this)
    } catch (err) {
      this.tx.rollbackTo(savepoint)
      throw err
    }
  }

  // captures

  /** `insert_capture`: saves `card` as a new row and returns its id. */
  async insertCapture(card: Card, origin: CaptureOrigin, syncedToAnki: boolean): Promise<number> {
    const columns = cardColumns(card)
    const id = await this.nextId('captures')
    const record: CaptureRecord = { id, ...columns, origin, synced_to_anki: syncedToAnki }
    this.tx.put('captures', id, record)
    return id
  }

  /** `get_capture`. */
  async getCapture(id: number): Promise<StoredCapture | null> {
    const record = await this.tx.get<CaptureRecord>('captures', id)
    return record ? decodeCapture(record) : null
  }

  /** `list_captures`: capture order (`captured_at`, then id), optionally of one origin. */
  async listCaptures(origin: CaptureOrigin | null): Promise<StoredCapture[]> {
    const records = (await this.captureRecords()).filter(
      (record) => origin === null || record.origin === origin,
    )
    records.sort((a, b) => compareText(a.captured_at, b.captured_at) || a.id - b.id)
    return records.map(decodeCapture)
  }

  /** `delete_capture`: with its `srs_state` and `review_log` rows; whether it existed. */
  async deleteCapture(id: number): Promise<boolean> {
    if ((await this.tx.get<CaptureRecord>('captures', id)) === undefined) return false
    this.tx.delete('captures', id)
    this.tx.delete('srs_state', id)
    for (const log of await this.tx.getAll<ReviewLogRecord>('review_log')) {
      if (log.card_id === id) this.tx.delete('review_log', log.id)
    }
    return true
  }

  /** `find_capture_by_front`: the oldest capture with exactly this front and script kind. */
  async findCaptureByFront(front: string, scriptKind: string): Promise<StoredCapture | null> {
    const record = (await this.captureRecords()).find(
      (each) => each.script_kind === scriptKind && each.front === front,
    )
    return record ? decodeCapture(record) : null
  }

  /** `learned_chinese_fronts`: fronts of Chinese captures that have an `srs_state` row. */
  async learnedChineseFronts(): Promise<Set<string>> {
    const learned = await this.srsStateCardIds()
    return new Set(
      (await this.captureRecords())
        .filter((record) => record.script_kind === 'chinese' && learned.has(record.id))
        .map((record) => record.front),
    )
  }

  /**
   * `LearnedVocabularyPool::QUERY`: the columns exercises are built from, of
   * every capture that has an `srs_state` row (`captures INNER JOIN
   * srs_state`), by id. `./pool.ts` is its only caller.
   */
  async queryLearnedVocabulary(): Promise<LearnedVocabularyRow[]> {
    const learned = await this.srsStateCardIds()
    return (await this.captureRecords())
      .filter((record) => learned.has(record.id))
      .map((record) => ({
        id: record.id,
        front: record.front,
        back: record.back,
        pinyin: record.pinyin,
        hsk_level: record.hsk_level,
        source_lang: record.source_lang,
        target_lang: record.target_lang,
        example_source: record.example_source,
        example_target: record.example_target,
        script_kind: record.script_kind,
        has_audio: record.audio_front !== null && record.audio_front.length > 0,
      }))
  }

  // curriculum

  /** `curriculum_count`. */
  curriculumCount(): Promise<number> {
    return this.tx.count('hsk_curriculum')
  }

  /** `replace_curriculum`: all of `items` or, when one is refused, nothing. */
  async replaceCurriculum(items: readonly CurriculumItem[]): Promise<void> {
    const fingerprint = curriculumFingerprint(items)
    await this.withTransaction((store) => store.writeCurriculum(items, fingerprint))
  }

  /**
   * `ensure_curriculum_seeded`: makes the table hold exactly `items`,
   * rewriting it only when the stored fingerprint or the row count differs.
   * Returns whether it wrote.
   */
  async ensureCurriculumSeeded(items: readonly CurriculumItem[]): Promise<boolean> {
    const fingerprint = curriculumFingerprint(items)
    return this.withTransaction(async (store) => {
      const seeded =
        (await store.getState(CURRICULUM_FINGERPRINT_KEY)) === fingerprint &&
        (await store.curriculumCount()) === items.length
      if (seeded) return false
      await store.writeCurriculum(items, fingerprint)
      return true
    })
  }

  /** `get_curriculum_item`: exactly this hanzi, not trimmed. */
  async getCurriculumItem(hanzi: string): Promise<CurriculumItem | null> {
    return (await this.tx.get<CurriculumItem>('hsk_curriculum', hanzi)) ?? null
  }

  /** `curriculum_in_order`. */
  async curriculumInOrder(): Promise<CurriculumItem[]> {
    const items = await this.tx.getAll<CurriculumItem>('hsk_curriculum')
    return items.sort((a, b) => a.curriculum_order - b.curriculum_order)
  }

  private async writeCurriculum(
    items: readonly CurriculumItem[],
    fingerprint: string,
  ): Promise<void> {
    this.tx.clear('hsk_curriculum')
    const hanzi = new Set<string>()
    const orders = new Set<number>()
    for (const item of items) {
      // SQLite checks the later UNIQUE index (curriculum_order) first.
      if (orders.has(item.curriculum_order)) {
        throw StoreError.database('UNIQUE constraint failed: hsk_curriculum.curriculum_order')
      }
      if (hanzi.has(item.hanzi)) {
        throw StoreError.database('UNIQUE constraint failed: hsk_curriculum.hanzi')
      }
      orders.add(item.curriculum_order)
      hanzi.add(item.hanzi)
      this.tx.put('hsk_curriculum', item.hanzi, item)
    }
    await this.setState(CURRICULUM_FINGERPRINT_KEY, fingerprint)
  }

  // srs_state

  /** `get_srs_state`. */
  async getSrsState(cardId: number): Promise<SrsStateRow | null> {
    const record = await this.tx.get<SrsStateRecord>('srs_state', cardId)
    return record ? decodeSrsState(record) : null
  }

  /** `upsert_srs_state`: the card must exist (foreign key). */
  async upsertSrsState(row: SrsStateRow): Promise<void> {
    const record: SrsStateRecord = {
      ...row,
      stability: sqlReal(row.stability),
      difficulty: sqlReal(row.difficulty),
      due_date: formatStoredInstant(row.due_date),
      last_reviewed_at:
        row.last_reviewed_at === null ? null : formatStoredInstant(row.last_reviewed_at),
    }
    await this.requireCapture(row.card_id)
    this.tx.put('srs_state', row.card_id, record)
  }

  /** `list_srs_due_before`: due strictly before `before`, by due date and card id. */
  async listSrsDueBefore(before: number): Promise<SrsStateRow[]> {
    const bound = formatExclusiveBound(before)
    const records = (await this.tx.getAll<SrsStateRecord>('srs_state')).filter(
      (record) => record.due_date < bound,
    )
    records.sort((a, b) => compareText(a.due_date, b.due_date) || a.card_id - b.card_id)
    return records.map(decodeSrsState)
  }

  /** `count_srs_states`: the number of learned cards. */
  countSrsStates(): Promise<number> {
    return this.tx.count('srs_state')
  }

  // review_log

  /** `insert_review_log`: appends `row` (its `id` ignored) and returns the new id. */
  async insertReviewLog(row: ReviewLogRow): Promise<number> {
    const reviewedAt = formatStoredInstant(row.reviewed_at)
    const sessionDate = storedDate(row.session_date)
    await this.requireCapture(row.card_id)
    const id = await this.nextId('review_log')
    const record: ReviewLogRecord = {
      ...row,
      id,
      reviewed_at: reviewedAt,
      session_date: sessionDate,
    }
    this.tx.put('review_log', id, record)
    return id
  }

  /** `review_log_for_card`: oldest first. */
  async reviewLogForCard(cardId: number): Promise<ReviewLogRow[]> {
    const records = (await this.tx.getAll<ReviewLogRecord>('review_log')).filter(
      (record) => record.card_id === cardId,
    )
    records.sort((a, b) => compareText(a.reviewed_at, b.reviewed_at) || a.id - b.id)
    return records.map((record) =>
      inRow('review_log', record.id, () => ({
        ...record,
        reviewed_at: parseInstantMillis(record.reviewed_at),
        session_date: parseStoredDate(record.session_date),
      })),
    )
  }

  // daily_sessions

  /** `get_daily_session`. */
  async getDailySession(date: ipc.IsoDate): Promise<DailySessionRow | null> {
    const record = await this.tx.get<DailySessionRecord>('daily_sessions', storedDate(date))
    return record ? decodeDailySession(record) : null
  }

  /** `upsert_daily_session`: inserts or replaces the session of `row.session_date`. */
  async upsertDailySession(row: DailySessionRow): Promise<void> {
    const sessionDate = storedDate(row.session_date)
    const record: DailySessionRecord = {
      ...row,
      session_date: sessionDate,
      completed_at: row.completed_at === null ? null : formatStoredInstant(row.completed_at),
    }
    this.tx.put('daily_sessions', sessionDate, record)
  }

  /** `list_completed_session_dates`: oldest first. */
  async listCompletedSessionDates(): Promise<ipc.IsoDate[]> {
    const records = (await this.tx.getAll<DailySessionRecord>('daily_sessions')).filter(
      (record) => record.completed_at !== null,
    )
    records.sort((a, b) => compareText(a.session_date, b.session_date))
    return records.map((record) => parseStoredDate(record.session_date))
  }

  // app_state

  /** `get_state`. */
  async getState(key: string): Promise<string | null> {
    return (await this.tx.get<string>('app_state', key)) ?? null
  }

  /** `set_state`. */
  async setState(key: string, value: string): Promise<void> {
    this.tx.put('app_state', key, value)
  }

  /** `delete_state`: a no-op for a key that is not set. */
  async deleteState(key: string): Promise<void> {
    this.tx.delete('app_state', key)
  }

  // xp_ledger and streak_freezes

  /** `insert_xp`: `date` is the local date the XP counts towards, `at` when it was earned. */
  async insertXp(date: ipc.IsoDate, amount: number, reason: string, at: number): Promise<number> {
    const sessionDate = storedDate(date)
    const createdAt = formatStoredInstant(at)
    const id = await this.nextId('xp_ledger')
    const record: XpRecord = {
      id,
      session_date: sessionDate,
      amount,
      reason,
      created_at: createdAt,
    }
    this.tx.put('xp_ledger', id, record)
    return id
  }

  /** `xp_total`. */
  async xpTotal(): Promise<number> {
    return sum(await this.tx.getAll<XpRecord>('xp_ledger'))
  }

  /** `xp_for_date`: entries recorded for the local date `date`. */
  async xpForDate(date: ipc.IsoDate): Promise<number> {
    const sessionDate = storedDate(date)
    const records = await this.tx.getAll<XpRecord>('xp_ledger')
    return sum(records.filter((record) => record.session_date === sessionDate))
  }

  /** Whether `date` has an XP entry for `reason` (gamification's session-bonus query). */
  async xpRecorded(date: ipc.IsoDate, reason: string): Promise<boolean> {
    const records = await this.tx.getAll<XpRecord>('xp_ledger')
    return records.some((record) => record.session_date === date && record.reason === reason)
  }

  /** `insert_streak_freeze`: idempotent. */
  async insertStreakFreeze(date: ipc.IsoDate): Promise<void> {
    const freezeDate = storedDate(date)
    this.tx.put('streak_freezes', freezeDate, freezeDate)
  }

  /** `list_streak_freeze_dates`: oldest first. */
  async listStreakFreezeDates(): Promise<ipc.IsoDate[]> {
    const dates = await this.tx.getAll<string>('streak_freezes')
    return dates.sort(compareText).map(parseStoredDate)
  }

  private captureRecords(): Promise<CaptureRecord[]> {
    return this.tx.getAll<CaptureRecord>('captures')
  }

  private async srsStateCardIds(): Promise<Set<number>> {
    const records = await this.tx.getAll<SrsStateRecord>('srs_state')
    return new Set(records.map((record) => record.card_id))
  }

  private async requireCapture(cardId: number): Promise<void> {
    if ((await this.tx.get<CaptureRecord>('captures', cardId)) === undefined) {
      throw StoreError.database(FOREIGN_KEY_FAILED)
    }
  }

  /** The next `AUTOINCREMENT` id of `table`. */
  private async nextId(table: 'captures' | 'review_log' | 'xp_ledger'): Promise<number> {
    const id = ((await this.tx.get<number>('sequences', table)) ?? 0) + 1
    this.tx.put('sequences', table, id)
    return id
  }
}

/** A daily session's serde shape. */
export function dailySessionToJson(row: DailySessionRow): ipc.DailySession {
  return {
    ...row,
    completed_at: row.completed_at === null ? null : formatInstant(row.completed_at),
  }
}

/** A daily session from its serde shape. */
export function dailySessionFromJson(value: ipc.DailySession): DailySessionRow {
  return {
    ...value,
    completed_at: value.completed_at === null ? null : parseInstantMillis(value.completed_at),
  }
}

/** `CardColumns::new`: the columns that hold a card's fields. */
function cardColumns(card: Card): Omit<CaptureRecord, 'id' | 'origin' | 'synced_to_anki'> {
  const metadata = card.script_metadata
  const chinese = metadata.kind === 'chinese' ? metadata : null
  return {
    front: card.front,
    back: card.back,
    example_source: card.example_source,
    example_target: card.example_target,
    notes: card.notes,
    tags: [...card.tags],
    source_lang: card.source_lang,
    target_lang: card.target_lang,
    captured_at: formatStoredInstant(card.captured_at),
    script_kind: metadata.kind,
    pinyin: chinese ? chinese.pinyin : null,
    hsk_level: chinese ? chinese.hsk_level : null,
    script_variant: chinese ? chinese.script_variant : null,
    audio_front: card.audio_front,
  }
}

/** `decode_capture`: strict about the enum columns. */
function decodeCapture(record: CaptureRecord): StoredCapture {
  return inRow('captures', record.id, () => {
    const scriptMetadata = decodeScriptMetadata(
      record.script_kind,
      record.pinyin,
      record.hsk_level,
      record.script_variant,
    )
    const origin = parseOrigin(record.origin)
    return {
      id: record.id,
      card: {
        front: record.front,
        back: record.back,
        example_source: record.example_source,
        example_target: record.example_target,
        notes: record.notes,
        tags: [...record.tags],
        source_lang: record.source_lang,
        target_lang: record.target_lang,
        captured_at: parseInstantMillis(record.captured_at),
        audio_front: record.audio_front,
        script_metadata: scriptMetadata,
      },
      origin,
      synced_to_anki: record.synced_to_anki,
    }
  })
}

/** `decode_script_metadata`: a missing pinyin or variant falls back; an unknown one is refused. */
function decodeScriptMetadata(
  kind: string,
  pinyin: string | null,
  hskLevel: number | null,
  scriptVariant: string | null,
): ScriptMetadata {
  if (kind === 'none') return { kind: 'none' }
  if (kind !== 'chinese') throw StoreError.invalid(`unknown script_kind ${debugString(kind)}`)
  if (scriptVariant !== null && scriptVariant !== 'simplified' && scriptVariant !== 'traditional') {
    throw StoreError.invalid(`unknown script_variant ${debugString(scriptVariant)}`)
  }
  return {
    kind: 'chinese',
    pinyin: pinyin ?? '',
    hsk_level: hskLevel,
    script_variant: scriptVariant ?? 'simplified',
  }
}

function parseOrigin(origin: string): CaptureOrigin {
  if (origin === 'capture' || origin === 'curriculum' || origin === 'anki_import') return origin
  throw StoreError.invalid(`unknown origin ${debugString(origin)}`)
}

function decodeSrsState(record: SrsStateRecord): SrsStateRow {
  return inRow('srs_state', record.card_id, () => ({
    ...record,
    due_date: parseInstantMillis(record.due_date),
    last_reviewed_at:
      record.last_reviewed_at === null ? null : parseInstantMillis(record.last_reviewed_at),
  }))
}

function decodeDailySession(record: DailySessionRecord): DailySessionRow {
  return inRow('daily_sessions', record.session_date, () => ({
    ...record,
    session_date: parseStoredDate(record.session_date),
    completed_at: record.completed_at === null ? null : parseInstantMillis(record.completed_at),
  }))
}

/** `codec::format_date`: refuses years without a four-digit form. */
function storedDate(date: ipc.IsoDate): string {
  if (!isStorableYear(dateYear(date))) {
    throw StoreError.invalid(
      `${date} is outside the years 0000 to 9999 that the database can store`,
    )
  }
  return date
}

/** `codec::parse_date`. */
function parseStoredDate(text: string): ipc.IsoDate {
  const date = parseDate(trim(text))
  if (date === null) throw StoreError.invalid(`not a YYYY-MM-DD date: ${debugString(text)}`)
  return date
}

/** `StoreError::in_row` around a row's decoding. */
function inRow<T>(table: string, key: string | number, decode: () => T): T {
  try {
    return decode()
  } catch (err) {
    throw err instanceof StoreError ? err.inRow(table, key) : err
  }
}

/** SQLite stores a NaN `REAL` as NULL. */
function sqlReal(value: number | null): number | null {
  return value === null || Number.isNaN(value) ? null : value
}

/** SQL's comparison of text: by code units, which for stored text is by bytes. */
function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0
}

function sum(records: XpRecord[]): number {
  return records.reduce((total, record) => total + record.amount, 0)
}
