/**
 * `vocab_core::quiz::pool`: the learned-vocabulary pool (Milestone M1), the
 * one place quiz content comes from.
 *
 * Learned means what the store says it means (docs/architecture.md § 6.1):
 * the capture has an `srs_state` row. {@link LearnedVocabularyPool.load} is
 * the only way to get a pool, so a generator handed one cannot be handed
 * vocabulary from anywhere else, and the curriculum, which the learner may
 * never have seen, is not read at all.
 */

import { isCjkChar } from './language'
import type { Store } from './store'
import { chars, trim } from './text'

/** `PoolEntry`: one learned card, reduced to what exercises are built from. */
export interface PoolEntry {
  card_id: number
  front: string
  back: string
  /** `null` for other scripts and when the stored pinyin is blank. */
  pinyin: string | null
  hsk_level: number | null
  source_lang: string
  target_lang: string
  example_source: string
  example_target: string
  script_kind: string
  has_audio: boolean
}

/** `LearnedVocabularyPool`: every learned card, in `card_id` order. */
export class LearnedVocabularyPool {
  private readonly byCardId: Map<number, PoolEntry>
  private readonly learnedHanzi: Set<string>

  private constructor(private readonly all: readonly PoolEntry[]) {
    this.byCardId = new Map(all.map((entry) => [entry.card_id, entry]))
    this.learnedHanzi = new Set(all.flatMap((entry) => chars(entry.front).filter(isCjkChar)))
  }

  /** `load`: the pool from `LearnedVocabularyPool::QUERY`, the only constructor. */
  static async load(store: Store): Promise<LearnedVocabularyPool> {
    const rows = await store.queryLearnedVocabulary()
    return new LearnedVocabularyPool(
      rows.map((row) => ({
        card_id: row.id,
        front: row.front,
        back: row.back,
        pinyin: row.pinyin !== null && trim(row.pinyin) !== '' ? row.pinyin : null,
        hsk_level: row.hsk_level,
        source_lang: row.source_lang,
        target_lang: row.target_lang,
        example_source: row.example_source,
        example_target: row.example_target,
        script_kind: row.script_kind,
        has_audio: row.has_audio,
      })),
    )
  }

  /** The learned cards, by `card_id`. */
  entries(): readonly PoolEntry[] {
    return this.all
  }

  /** The learned card `cardId`, or `undefined` when it is unknown or not learned. */
  get(cardId: number): PoolEntry | undefined {
    return this.byCardId.get(cardId)
  }

  /** Every CJK ideograph in a learned front: exercises show no other hanzi. */
  learnedHanziChars(): Set<string> {
    return new Set(this.learnedHanzi)
  }

  /** {@link learnedHanziChars} without the copy, for the generators. */
  hanzi(): ReadonlySet<string> {
    return this.learnedHanzi
  }
}
