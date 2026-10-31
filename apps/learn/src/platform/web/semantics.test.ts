/**
 * Rust semantics of the port that the golden vectors cannot carry:
 *
 * - JSON has no NaN or Infinity (serde_json writes both as `null`), so the
 *   f64 rules of rs-fsrs and the store's NaN-as-NULL are pinned here.
 * - An empty recording never reaches the port through JSON (Rust reads the
 *   serialized `""` back as no audio), but the store must still count one as
 *   no recording, as `length(audio_front) > 0` does.
 * - One clock reading per command means a write never returns a session of
 *   another day, so `just_completed`'s date check is pinned on its own, as
 *   vocab-learn's unit test does.
 */

import { describe, expect, it } from 'vitest'

import { asI64, roundHalfAwayFromZero, rustMax, rustMin } from './fsrs'
import { newCard } from './model'
import { LearnedVocabularyPool } from './pool'
import { SrsScheduler, type SrsStateRow, fsrsCardFromState } from './scheduler'
import { justCompleted } from './session'
import { MemoryStorage } from './storage'
import { type DailySessionRow, Store } from './store'
import { nanosFromMillis, parseInstantMillis } from './time'

const NOW = parseInstantMillis('2026-11-09T08:00:00.000Z')

/** A card reviewed before, as tests/srs_fsrs.rs's `review_state`. */
function reviewState(cardId: number): SrsStateRow {
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

describe('the f64 rules of the FSRS port', () => {
  it('refuses a memory state that is not finite, as fsrs_card_from_state does', () => {
    const rows: [string, SrsStateRow][] = [
      ['an infinite stability', { ...reviewState(7), stability: Infinity }],
      ['an infinite difficulty', { ...reviewState(7), difficulty: Infinity }],
      ['a NaN stability', { ...reviewState(7), stability: NaN }],
    ]
    for (const [what, row] of rows) {
      expect(() => fsrsCardFromState(row), what).toThrow(
        'invalid data: srs_state row 7: a review card needs a positive stability and difficulty and a last review',
      )
      expect(new SrsScheduler().preview(row, nanosFromMillis(NOW)), what).toEqual([])
    }
  })

  it('lets a NaN operand lose, as f64::max and f64::min do', () => {
    expect([rustMax(NaN, 3), rustMax(3, NaN), rustMax(2, 3)]).toEqual([3, 3, 3])
    expect([rustMin(NaN, 3), rustMin(3, NaN), rustMin(2, 3)]).toEqual([3, 3, 2])
  })

  it('truncates, saturates and turns NaN into 0, as `as i64` does', () => {
    expect([asI64(2.9), asI64(-2.9), asI64(NaN)]).toEqual([2, -2, 0])
    expect([asI64(Infinity), asI64(-Infinity)]).toEqual([2 ** 63, -(2 ** 63)])
  })

  it('rounds halves away from zero, as f64::round does', () => {
    expect([2.5, -2.5, 0.49999999999999994, -0.5].map(roundHalfAwayFromZero)).toEqual([
      3, -3, 0, -1,
    ])
  })
})

describe('store values JSON cannot carry', () => {
  it('reads a NaN memory value back as NULL, as SQLite stores it', async () => {
    const row = await new MemoryStorage().transaction(async (tx) => {
      const store = new Store(tx)
      const card = newCard('Haus', 'house', 'de', 'en', NOW)
      const id = await store.insertCapture(card, 'capture', false)
      await store.upsertSrsState({ ...reviewState(id), stability: NaN, difficulty: NaN })
      return store.getSrsState(id)
    })
    expect([row?.stability, row?.difficulty]).toEqual([null, null])
  })

  it('counts an empty recording as no recording', async () => {
    const pool = await new MemoryStorage().transaction(async (tx) => {
      const store = new Store(tx)
      for (const audio of [null, new Uint8Array(0), new Uint8Array([0xff, 0xfb])]) {
        const card = { ...newCard('Haus', 'house', 'de', 'en', NOW), audio_front: audio }
        const id = await store.insertCapture(card, 'capture', false)
        await store.upsertSrsState(reviewState(id))
      }
      return LearnedVocabularyPool.load(store)
    })
    expect(pool.entries().map((entry) => entry.has_audio)).toEqual([false, false, true])
  })
})

describe('session::just_completed', () => {
  it('judges completion per session date', () => {
    const open: DailySessionRow = {
      session_date: '2026-10-31',
      requested_new_count: 1,
      reviews_due_count: 0,
      reviews_completed: 0,
      new_items_completed: 0,
      completed_at: null,
    }
    const done = { ...open, new_items_completed: 1, completed_at: NOW }
    expect(justCompleted(open, done)).toBe(true)
    expect(justCompleted(null, done)).toBe(true)
    expect(justCompleted(done, done)).toBe(false)
    expect(justCompleted(open, open)).toBe(false)
    expect(justCompleted({ ...done, session_date: '2026-10-30' }, done)).toBe(true)
  })
})
