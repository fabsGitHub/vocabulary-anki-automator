/**
 * `vocab_core::quiz::segment`: cutting sentences into word tiles
 * (Milestone M2), and finding a word in a sentence.
 *
 * A Chinese sentence is cut by greedy longest match against the learned
 * pool's fronts and nothing else (docs/architecture.md § 7.2): with the HSK
 * curriculum as the dictionary, a learner who knows only 学习 would get the
 * tile 学生, a word they never learned. Where no learned word matches, the
 * sentence falls back to single characters. Sentences without hanzi are cut
 * at whitespace. Punctuation never becomes a tile: word characters are
 * letters, digits and combining marks.
 *
 * Positions are UTF-16 offsets into the string, where Rust's are byte
 * offsets; both delimit the same characters, which is all callers use them
 * for.
 */

import { containsCjk, isCjkChar } from './language'
import type { LearnedVocabularyPool } from './pool'
import { chars, codePoint, isAlphanumeric, splitWhitespace, trim } from './text'

/**
 * `Dictionary`: the learned fronts that can be one tile of a Chinese
 * sentence, i.e. that contain a hanzi and consist of word characters only.
 */
export class Dictionary {
  private readonly words: Set<string>
  /** The longest word's length, in characters. */
  readonly longest: number

  private constructor(fronts: Iterable<string>) {
    this.words = new Set(
      [...fronts].map(trim).filter((front) => containsCjk(front) && chars(front).every(isWordChar)),
    )
    this.longest = [...this.words].reduce(
      (longest, word) => Math.max(longest, chars(word).length),
      0,
    )
  }

  /** `from_pool`. */
  static fromPool(pool: LearnedVocabularyPool): Dictionary {
    return new Dictionary(pool.entries().map((entry) => entry.front))
  }

  /** `from_fronts`. */
  static fromFronts(fronts: Iterable<string>): Dictionary {
    return new Dictionary(fronts)
  }

  /** Whether `word` can be a tile of a Chinese sentence. */
  contains(word: string): boolean {
    return this.words.has(word)
  }

  /** The length of the longest word that `text` starts with at character `start`. */
  longestPrefix(text: readonly string[], start: number): number | null {
    for (let length = Math.min(this.longest, text.length - start); length >= 1; length -= 1) {
      if (this.words.has(text.slice(start, start + length).join(''))) return length
    }
    return null
  }
}

/**
 * `tiles`: greedy longest match against `dictionary` when the sentence has a
 * hanzi; whitespace-separated words without their surrounding punctuation
 * otherwise.
 */
export function tiles(sentence: string, dictionary: Dictionary): string[] {
  if (containsCjk(sentence)) return cjkTiles(sentence, dictionary)
  return splitWhitespace(sentence).flatMap((token) => {
    const tile = wordTile(token)
    return tile === null ? [] : [tile]
  })
}

/**
 * Left to right; where no dictionary word starts, a hanzi is a tile of its
 * own and a run of other word characters (`iPhone` in `我用iPhone`) one tile.
 */
function cjkTiles(sentence: string, dictionary: Dictionary): string[] {
  const text = chars(sentence)
  const found: string[] = []
  let start = 0
  while (start < text.length) {
    if (!isWordChar(text[start])) {
      start += 1
      continue
    }
    let length = dictionary.longestPrefix(text, start)
    if (length === null) {
      if (isCjkChar(text[start])) {
        length = 1
      } else {
        length = 0
        while (
          start + length < text.length &&
          isWordChar(text[start + length]) &&
          !isCjkChar(text[start + length])
        ) {
          length += 1
        }
      }
    }
    found.push(text.slice(start, start + length).join(''))
    start += length
  }
  return found
}

/**
 * `word_tile`: one whitespace-separated word without its surrounding
 * punctuation (`„Haus“,` -> `Haus`); `null` when nothing is left.
 */
export function wordTile(token: string): string | null {
  const text = chars(token)
  let start = 0
  let end = text.length
  while (start < end && !isWordChar(text[start])) start += 1
  while (end > start && !isWordChar(text[end - 1])) end -= 1
  return start < end ? text.slice(start, end).join('') : null
}

/** `is_word_char`: letters, digits and combining marks. */
export function isWordChar(c: string): boolean {
  return isAlphanumeric(c) || isCombiningMark(c)
}

/** The combining-mark blocks. */
function isCombiningMark(c: string): boolean {
  const point = codePoint(c)
  return (
    (point >= 0x0300 && point <= 0x036f) ||
    (point >= 0x1ab0 && point <= 0x1aff) ||
    (point >= 0x1dc0 && point <= 0x1dff) ||
    (point >= 0x20d0 && point <= 0x20ff) ||
    (point >= 0xfe20 && point <= 0xfe2f)
  )
}

/**
 * `find_word`: `[start, end)` of the first occurrence of `word` (trimmed)
 * in `text`: exact when `word` has a hanzi, ignoring case one character at a
 * time otherwise. Outside hanzi, an occurrence that stands as a word of its
 * own wins over an earlier one inside a longer word. A blank word occurs
 * nowhere.
 */
export function findWord(text: string, word: string): [number, number] | null {
  const needle = trim(word)
  if (needle === '') return null
  if (containsCjk(needle)) {
    const start = text.indexOf(needle)
    return start < 0 ? null : [start, start + needle.length]
  }
  const expected = chars(needle)
  const occurrences: [number, number][] = []
  for (let start = 0; start < text.length; start += codeUnits(text, start)) {
    let end = start
    let matched = true
    for (const letter of expected) {
      if (end >= text.length) {
        matched = false
        break
      }
      const found = String.fromCodePoint(text.codePointAt(end) ?? 0)
      if (!sameLetter(found, letter)) {
        matched = false
        break
      }
      end += found.length
    }
    if (matched) occurrences.push([start, end])
  }
  if (occurrences.length === 0) return null
  return occurrences.find((range) => standsAlone(text, range)) ?? occurrences[0]
}

/** Neither preceded nor followed by a word character. */
function standsAlone(text: string, [start, end]: [number, number]): boolean {
  const before = chars(text.slice(0, start)).pop()
  const after = chars(text.slice(end))[0]
  return (
    !(before !== undefined && isWordChar(before)) && !(after !== undefined && isWordChar(after))
  )
}

/** `same_letter`: equal, or equal once each is lowercased on its own. */
function sameLetter(a: string, b: string): boolean {
  return a === b || a.toLowerCase() === b.toLowerCase()
}

/** The UTF-16 length of the character at `index`. */
function codeUnits(text: string, index: number): number {
  return (text.codePointAt(index) ?? 0) > 0xffff ? 2 : 1
}
