/**
 * `vocab_core::lessons`: Babbel-style lessons made from the learner's own
 * clipboard captures (Milestone I), each card a word shown in the sentence
 * it was captured with.
 *
 * Only `origin = capture` rows qualify, and only those whose example
 * sentence contains the word and has a translation, so the cloze has
 * something to blank. Cards are grouped by language pair in capture order
 * and cut into lessons; a lesson's id names its first card
 * (`lesson-<card id>`), so it stays stable while captures are added.
 * `Lesson.title` is an English fallback; the UI localizes it from the
 * languages and the lesson's position.
 */

import type * as ipc from '../types'
import { type StoredCapture, cardPinyin } from './model'
import { findWord } from './segment'
import type { Store } from './store'
import { trim } from './text'

/** `CLOZE_BLANK`: what replaces the word in a cloze sentence. */
export const CLOZE_BLANK = '____'

export type Lesson = ipc.Lesson
export type LessonCard = ipc.LessonCard

/**
 * `build_lessons`: lessons of `cardsPerLesson` cards (0 counts as 1), by
 * each language pair's first capture and then by lesson number. `captures`
 * may come in any order; cards follow `captured_at`, then id.
 */
export function buildLessons(captures: readonly StoredCapture[], cardsPerLesson: number): Lesson[] {
  const usable = captures
    .filter((capture) => capture.origin === 'capture')
    .sort((a, b) => a.card.captured_at - b.card.captured_at || a.id - b.id)

  const pairs: { source: string; target: string; cards: LessonCard[] }[] = []
  for (const capture of usable) {
    const card = lessonCard(capture)
    if (card === null) continue
    const { source_lang: source, target_lang: target } = capture.card
    const pair = pairs.find((known) => known.source === source && known.target === target)
    if (pair) pair.cards.push(card)
    else pairs.push({ source, target, cards: [card] })
  }

  const size = Math.max(cardsPerLesson, 1)
  return pairs.flatMap(({ source, target, cards }) => {
    const lessons: Lesson[] = []
    for (let start = 0; start < cards.length; start += size) {
      const chunk = cards.slice(start, start + size)
      lessons.push({
        id: `lesson-${chunk[0].card_id}`,
        title: `${source.toUpperCase()} → ${target.toUpperCase()} · Lesson ${lessons.length + 1}`,
        source_lang: source,
        target_lang: target,
        cards: chunk,
      })
    }
    return lessons
  })
}

/** `lessons_from_store`: {@link buildLessons} over every clipboard capture. */
export async function lessonsFromStore(store: Store, cardsPerLesson: number): Promise<Lesson[]> {
  return buildLessons(await store.listCaptures('capture'), cardsPerLesson)
}

/** `lesson_card`: the card of a capture whose sentence contains its front and has a translation. */
function lessonCard(capture: StoredCapture): LessonCard | null {
  const { card } = capture
  const word = trim(card.front)
  const sentence = trim(card.example_source)
  const sentenceTarget = trim(card.example_target)
  if (sentenceTarget === '') return null
  const found = findWord(sentence, word)
  if (found === null) return null
  const pinyin = cardPinyin(card)
  return {
    card_id: capture.id,
    word,
    translation: trim(card.back),
    sentence_source: sentence,
    sentence_target: sentenceTarget,
    cloze: `${sentence.slice(0, found[0])}${CLOZE_BLANK}${sentence.slice(found[1])}`,
    pinyin: pinyin !== null && trim(pinyin) !== '' ? trim(pinyin) : null,
  }
}
