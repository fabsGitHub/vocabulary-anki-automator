/**
 * What the lesson screens derive from `Lesson`s (docs/architecture.md § 7.3).
 *
 * `Lesson.title` is an English fallback ("EN → DE · Lesson 2"); the UI builds
 * its own title from the two languages and the lesson's number, which the
 * core counts per language pair in list order. A lesson card is shown twice:
 * first the word in its sentence, then the sentence as a cloze.
 */

import type { Lesson, LessonCard } from '../platform/types'

/** What replaces the word in `LessonCard.cloze` (vocab_core::lessons::CLOZE_BLANK). */
export const CLOZE_BLANK = '____'

/** Each lesson's number within its language pair, 1-based, in list order. */
export function lessonNumbers(lessons: Lesson[]): number[] {
  const seen = new Map<string, number>()
  return lessons.map((lesson) => {
    const pair = `${lesson.source_lang}\u0000${lesson.target_lang}`
    const number = (seen.get(pair) ?? 0) + 1
    seen.set(pair, number)
    return number
  })
}

export type LessonStep = { kind: 'context'; card: LessonCard } | { kind: 'cloze'; card: LessonCard }

/** The steps of a lesson: every card in context, followed by its cloze. */
export function lessonSteps(lesson: Lesson): LessonStep[] {
  return lesson.cards.flatMap((card): LessonStep[] => [
    { kind: 'context', card },
    { kind: 'cloze', card },
  ])
}

/** The text around the gap of a cloze; `null` when it has no gap. */
export function splitCloze(cloze: string): { before: string; after: string } | null {
  const at = cloze.indexOf(CLOZE_BLANK)
  if (at < 0) return null
  return { before: cloze.slice(0, at), after: cloze.slice(at + CLOZE_BLANK.length) }
}

/**
 * A lesson card's cloze with what fills its gap: the word as the sentence
 * spells it, which is not always `card.word`. The core blanks the word
 * ignoring case outside hanzi, so "Serendipity is rare" for the word
 * `serendipity` must come back with its capital. When the cloze does not
 * frame the sentence (it should always), the gap falls back to the word.
 */
export function clozeParts(
  card: LessonCard,
): { before: string; gap: string; after: string } | null {
  const parts = splitCloze(card.cloze)
  if (!parts) return null
  const { before, after } = parts
  const sentence = card.sentence_source
  const framed =
    sentence.length >= before.length + after.length &&
    sentence.startsWith(before) &&
    sentence.endsWith(after)
  const gap = framed ? sentence.slice(before.length, sentence.length - after.length) : card.word
  return { before, gap, after }
}
