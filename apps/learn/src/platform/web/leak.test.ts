/**
 * The leak test of the web port (Milestone G+, mirroring
 * crates/vocab-core/tests/quiz_isolation.rs): no exercise may show
 * vocabulary the learner has not learned (Milestone M1,
 * docs/architecture.md § 7.2).
 *
 * The store holds far more than the learner knows: a 300-word curriculum
 * whose example sentences are full of unknown characters, captures that
 * were never introduced (no `srs_state` row) from every origin, and trap
 * words such as 学生 that consist of learned characters without having been
 * learned, which a character check alone would let through. Every exercise
 * kind is generated for every learned card over 200+ seeds, and every
 * string of every exercise is checked against what this file knows to be
 * learned, never against the pool under test, which a broken query would
 * corrupt together with its own `learnedHanziChars`.
 */

import { describe, expect, it } from 'vitest'

import type * as ipc from '../types'
import { QuizError } from './errors'
import { EXERCISE_KINDS, type ExerciseKind, generateExercise } from './exercises'
import { cjkChars, isCjkChar } from './language'
import { type CaptureOrigin, type Card, type CurriculumItem, newCard } from './model'
import { LearnedVocabularyPool } from './pool'
import type { SrsStateRow } from './scheduler'
import { MemoryStorage } from './storage'
import { Store } from './store'
import { chars, trim } from './text'
import { parseInstantMillis } from './time'

/** One card the fixture writes into `captures`. */
interface Word {
  front: string
  back: string
  pinyin: string | null
  pair: [string, string]
  example: [string, string]
  origin: CaptureOrigin
  audio: boolean
}

function zh(
  front: string,
  back: string,
  pinyin: string,
  example: [string, string],
  origin: CaptureOrigin,
  audio: boolean,
): Word {
  return { front, back, pinyin, pair: ['zh-CN', 'de'], example, origin, audio }
}

function other(
  front: string,
  back: string,
  pair: [string, string],
  example: [string, string],
  audio: boolean,
  origin: CaptureOrigin = 'capture',
): Word {
  return { front, back, pinyin: null, pair, example, origin, audio }
}

/**
 * The learner's vocabulary: these get `srs_state` rows. Several example
 * sentences contain unlearned hanzi (的, 他, 气, 吗, 这, 书) and must never be
 * shown; 学生很喜欢学习中文 hides the trap word 学生 among learned words.
 */
const LEARNED: Word[] = [
  zh('我们', 'wir', 'wǒ men', ['我们是朋友。', 'Wir sind Freunde.'], 'capture', true),
  zh(
    '学习',
    'lernen',
    'xué xí',
    ['学生很喜欢学习中文。', 'Die Schüler lernen gern Chinesisch.'],
    'curriculum',
    false,
  ),
  zh(
    '中文',
    'Chinesisch',
    'zhōng wén',
    ['我们的中文老师很好。', 'Unser Chinesischlehrer ist gut.'],
    'capture',
    true,
  ),
  zh('喜欢', 'mögen', 'xǐ huan', ['', ''], 'capture', false),
  zh(
    '朋友',
    'Freund',
    'péng you',
    ['他是我的好朋友。', 'Er ist mein guter Freund.'],
    'curriculum',
    false,
  ),
  zh(
    '老师',
    'Lehrer',
    'lǎo shī',
    ['老师今天很好。', 'Dem Lehrer geht es heute gut.'],
    'capture',
    true,
  ),
  zh(
    '今天',
    'heute',
    'jīn tiān',
    ['今天天气很好。', 'Heute ist das Wetter gut.'],
    'capture',
    false,
  ),
  zh(
    '生活',
    'Leben',
    'shēng huó',
    ['你好！我们喜欢生活。', 'Hallo! Wir mögen das Leben.'],
    'curriculum',
    false,
  ),
  zh('你', 'du', 'nǐ', ['你好吗？', 'Wie geht es dir?'], 'capture', true),
  zh('好', 'gut', 'hǎo', ['', ''], 'curriculum', false),
  zh('是', 'sein', 'shì', ['这是我的书。', 'Das ist mein Buch.'], 'capture', false),
  zh('很', 'sehr', 'hěn', ['很好。', 'Sehr gut.'], 'capture', false),
  other(
    'serendipity',
    'glücklicher Zufall',
    ['en', 'de'],
    ['It was pure serendipity.', 'Es war reiner Zufall.'],
    true,
  ),
  other('house', 'Haus', ['en', 'de'], ['The house is big.', 'Das Haus ist groß.'], false),
  // Learned, but its translation 书 and its example's translation 我读书 hold
  // hanzi no learned front has: only the English sides may be shown.
  other('book', '书', ['en', 'zh-CN'], ['I read a book.', '我读书。'], false),
]

/**
 * Captured but never introduced: no `srs_state` row. 学生, 你好, 老友, 中学 and
 * 好友 are made of learned characters only, and so are the sentences of 你好,
 * 老友, 中学 and 好友. Every origin is represented, because none of them makes
 * a card learned.
 */
const NOT_LEARNED: Word[] = [
  zh(
    '学生',
    'Schüler',
    'xué sheng',
    ['学生是我们的朋友。', 'Die Schüler sind unsere Freunde.'],
    'capture',
    true,
  ),
  zh('你好', 'hallo', 'nǐ hǎo', ['你好，老师！', 'Hallo, Herr Lehrer!'], 'capture', true),
  zh('他', 'er', 'tā', ['他很好。', 'Ihm geht es gut.'], 'capture', false),
  zh(
    '老友',
    'alter Freund',
    'lǎo yǒu',
    ['老友今天很好。', 'Dem alten Freund geht es heute gut.'],
    'capture',
    false,
  ),
  zh('事', 'Sache', 'shì', ['这是好事。', 'Das ist eine gute Sache.'], 'capture', false),
  zh(
    '中学',
    'Mittelschule',
    'zhōng xué',
    ['中学老师很好。', 'Die Lehrer der Mittelschule sind gut.'],
    'curriculum',
    true,
  ),
  zh(
    '好友',
    'guter Freund',
    'hǎo yǒu',
    ['好友今天很好。', 'Dem guten Freund geht es heute gut.'],
    'anki_import',
    false,
  ),
  other(
    'school',
    'Schule',
    ['en', 'de'],
    ['The school is old.', 'Die Schule ist alt.'],
    true,
    'anki_import',
  ),
]

/** Unlearned curriculum words made of learned characters only. */
const TRAP_WORDS = [
  '学生',
  '你好',
  '老友',
  '中学',
  '很好',
  '好友',
  '天生',
  '文学',
  '你们',
  '师生',
  '今生',
  '好学',
]

/** Unlearned one-character words; the first eight are learned characters, fine as fallback tiles. */
const SINGLE_CHARACTER_WORDS = ['学', '生', '我', '中', '天', '老', '文', '喜', '他', '事']

const CURRICULUM_SIZE = 300

const CAPTURED_AT = parseInstantMillis('2026-10-01T08:00:00.000Z')

function card(word: Word): Card {
  const result = newCard(word.front, word.back, word.pair[0], word.pair[1], CAPTURED_AT)
  result.example_source = word.example[0]
  result.example_target = word.example[1]
  if (word.pinyin !== null) {
    result.script_metadata = {
      kind: 'chinese',
      pinyin: word.pinyin,
      hsk_level: 1,
      script_variant: 'simplified',
    }
  }
  if (word.audio) result.audio_front = new Uint8Array([0xff, 0xfb, 0x90, 0x00])
  return result
}

function learnedState(cardId: number): SrsStateRow {
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

function learnedChars(): Set<string> {
  return new Set(LEARNED.flatMap((word) => chars(word.front).filter(isCjkChar)))
}

/**
 * 300 distinct words: the learned Chinese words (the curriculum-introduced
 * ones with their sentence), the trap and single-character words, and filler
 * words of unlearned hanzi. Every other example sentence mixes a learned word
 * into unlearned hanzi, so a curriculum sentence that leaks passes the
 * "contains the target" test.
 */
function curriculum(): CurriculumItem[] {
  const learned = learnedChars()
  const unlearned: string[] = []
  for (let point = 0x5000; point <= 0x9fff && unlearned.length < 1200; point += 1) {
    const c = String.fromCodePoint(point)
    if (!learned.has(c)) unlearned.push(c)
  }
  const chinese = LEARNED.filter((word) => word.pinyin !== null)
  const words: [string, Word | null][] = chinese.map((word) => [word.front, word])
  for (const word of [...TRAP_WORDS, ...SINGLE_CHARACTER_WORDS]) words.push([word, null])
  for (let filler = 0; words.length < CURRICULUM_SIZE; filler += 2) {
    words.push([unlearned[filler] + unlearned[filler + 1], null])
  }
  const u = (n: number) => unlearned[(600 + n) % unlearned.length]
  return words.map(([hanzi, word], order) => {
    const fromCurriculum = word !== null && word.origin === 'curriculum'
    const inner = chinese[order % chinese.length].front
    const sentence = fromCurriculum
      ? word.example[0]
      : `${u(3 * order)}${u(3 * order + 1)}${inner}${u(3 * order + 2)}的${hanzi}。`
    return {
      hanzi,
      traditional: null,
      hsk_level: (order % 6) + 1,
      frequency: order,
      curriculum_order: order,
      pinyin: fromCurriculum ? (word.pinyin ?? '') : `pīn ${order}`,
      pos: null,
      meaning_en: `meaning ${order}`,
      meaning_de: fromCurriculum ? word.back : `Bedeutung ${order}`,
      cloze_sentence: null,
      example_sentence_source: sentence === '' ? null : sentence,
      example_sentence_pinyin: null,
      example_sentence_target: `Beispielsatz ${order}`,
      vocab_audio_path: null,
      sentence_audio_path: null,
      image_path: null,
    }
  })
}

/** What the fixture knows to be learned, built from its own tables. */
interface Truth {
  learned: Map<number, Word>
  notLearned: Map<number, Word>
  learnedChars: Set<string>
  learnedFronts: Set<string>
  /** Curriculum words and captured fronts that are not learned fronts. */
  unlearnedWords: Set<string>
  curriculum: CurriculumItem[]
}

interface Fixture {
  storage: MemoryStorage
  truth: Truth
}

async function fixture(): Promise<Fixture> {
  const storage = new MemoryStorage()
  const items = curriculum()
  const learned = new Map<number, Word>()
  const notLearned = new Map<number, Word>()
  await storage.transaction(async (tx) => {
    const store = new Store(tx)
    await store.replaceCurriculum(items)
    // Interleaved, so learned and unlearned card ids alternate.
    for (const [index, word] of LEARNED.entries()) {
      const id = await store.insertCapture(card(word), word.origin, false)
      await store.upsertSrsState(learnedState(id))
      learned.set(id, word)
      const unlearned = NOT_LEARNED[index]
      if (unlearned) {
        notLearned.set(
          await store.insertCapture(card(unlearned), unlearned.origin, false),
          unlearned,
        )
      }
    }
  })
  const learnedFronts = new Set(LEARNED.map((word) => word.front))
  const unlearnedWords = new Set(
    [...items.map((item) => item.hanzi), ...NOT_LEARNED.map((word) => word.front)].filter(
      (word) => !learnedFronts.has(word),
    ),
  )
  return {
    storage,
    truth: {
      learned,
      notLearned,
      learnedChars: learnedChars(),
      learnedFronts,
      unlearnedWords,
      curriculum: items,
    },
  }
}

function loadPool(storage: MemoryStorage): Promise<LearnedVocabularyPool> {
  return storage.transaction((tx) => LearnedVocabularyPool.load(new Store(tx)))
}

function seeds(): number[] {
  return [...Array.from({ length: 200 }, (_, seed) => seed), 0x8000_0000, 0xdead_beef, 0xffff_ffff]
}

type Role = 'prompt' | 'option' | 'audio' | 'sentence' | 'tile'

function shownTexts(exercise: ipc.Exercise): [Role, string][] {
  const options = (choices: ipc.ChoiceOption[]) =>
    choices.map((option): [Role, string] => ['option', option.text])
  const tiles = (all: ipc.Tile[]) => all.map((tile): [Role, string] => ['tile', tile.text])
  switch (exercise.kind) {
    case 'multiple_choice':
      return [['prompt', exercise.prompt], ...options(exercise.options)]
    case 'listening_choice':
      return [['audio', exercise.audio.text], ...options(exercise.options)]
    case 'sentence_builder':
      return [
        ['prompt', exercise.prompt],
        ['sentence', exercise.sentence],
        ...tiles(exercise.tiles),
      ]
    case 'listening_sentence':
      return [
        ['audio', exercise.audio.text],
        ['sentence', exercise.sentence],
        ...tiles(exercise.tiles),
      ]
  }
}

/** Every string anywhere in the exercise, including fields this file does not know about yet. */
function allStrings(value: unknown, out: string[] = []): string[] {
  if (typeof value === 'string') out.push(value)
  else if (Array.isArray(value)) value.forEach((item) => allStrings(item, out))
  else if (value !== null && typeof value === 'object') {
    Object.values(value).forEach((item) => allStrings(item, out))
  }
  return out
}

function promptSide(word: Word, direction: ipc.ChoiceDirection): string {
  return direction === 'back_to_front' ? word.back : word.front
}

function answerSide(word: Word, direction: ipc.ChoiceDirection): string {
  switch (direction) {
    case 'front_to_back':
      return word.back
    case 'back_to_front':
      return word.front
    case 'front_to_pinyin':
      return word.pinyin ?? ''
  }
}

/** Everything wrong with `exercise`; empty when it shows learned vocabulary only, taken from where it should be. */
function violations(truth: Truth, exercise: ipc.Exercise): string[] {
  const found: string[] = []
  for (const text of allStrings(exercise)) {
    for (const c of cjkChars(text)) {
      if (!truth.learnedChars.has(c)) found.push(`unlearned hanzi ${c} in ${JSON.stringify(text)}`)
    }
  }
  for (const [role, raw] of shownTexts(exercise)) {
    const text = trim(raw)
    const fallbackTile =
      role === 'tile' &&
      chars(text).length === 1 &&
      chars(text).every((c) => truth.learnedChars.has(c))
    if (truth.unlearnedWords.has(text) && !fallbackTile) {
      found.push(`unlearned word ${JSON.stringify(text)} shown as ${role}`)
    }
  }
  return [...found, ...provenance(truth, exercise)]
}

/** Whether every part of `exercise` comes from the learned card it claims. */
function provenance(truth: Truth, exercise: ipc.Exercise): string[] {
  const target = truth.learned.get(exercise.card_id)
  if (target === undefined) return [`target card ${exercise.card_id} is not learned`]
  const found: string[] = []
  switch (exercise.kind) {
    case 'multiple_choice':
      if (exercise.prompt !== promptSide(target, exercise.direction)) {
        found.push(`prompt ${JSON.stringify(exercise.prompt)} is not the target's`)
      }
      found.push(
        ...optionProvenance(
          truth,
          exercise.card_id,
          exercise.options,
          exercise.correct_index,
          (w) => answerSide(w, exercise.direction),
        ),
      )
      break
    case 'listening_choice': {
      found.push(
        ...optionProvenance(
          truth,
          exercise.card_id,
          exercise.options,
          exercise.correct_index,
          (w) => w.front,
        ),
      )
      if (exercise.audio.text !== target.front || exercise.audio.lang !== target.pair[0]) {
        found.push(`audio ${JSON.stringify(exercise.audio)} is not the target's front`)
      }
      const expected = target.audio ? exercise.card_id : null
      if (exercise.audio.audio_card_id !== expected) {
        found.push(`audio card ${exercise.audio.audio_card_id}, expected ${expected}`)
      }
      break
    }
    case 'sentence_builder': {
      found.push(...sentenceProvenance(truth, exercise))
      const word = truth.learned.get(exercise.sentence_card_id)
      if (word && exercise.prompt !== word.example[1]) {
        found.push(
          `prompt ${JSON.stringify(exercise.prompt)} is not card ${exercise.sentence_card_id}'s translation`,
        )
      }
      break
    }
    case 'listening_sentence':
      found.push(...sentenceProvenance(truth, exercise))
      if (exercise.audio.text !== exercise.sentence || exercise.audio.audio_card_id !== null) {
        found.push(`audio ${JSON.stringify(exercise.audio)} is not the sentence`)
      }
      break
  }
  return found
}

function optionProvenance(
  truth: Truth,
  targetId: number,
  options: ipc.ChoiceOption[],
  correct: number,
  side: (word: Word) => string,
): string[] {
  const found: string[] = []
  if (options[correct]?.card_id !== targetId) found.push(`option ${correct} is not the target card`)
  if (new Set(options.map((option) => option.text)).size !== options.length) {
    found.push(`options repeat a text: ${JSON.stringify(options)}`)
  }
  for (const option of options) {
    const word = truth.learned.get(option.card_id)
    if (word === undefined) {
      found.push(
        `option ${JSON.stringify(option.text)} comes from card ${option.card_id}, which is not learned`,
      )
    } else if (side(word) !== option.text) {
      found.push(
        `option ${JSON.stringify(option.text)} is not what learned card ${option.card_id} says (${JSON.stringify(side(word))})`,
      )
    }
  }
  return found
}

function sentenceProvenance(
  truth: Truth,
  exercise: ipc.SentenceBuilderExercise | ipc.ListeningSentenceExercise,
): string[] {
  const found: string[] = []
  const { sentence, sentence_card_id: cardId } = exercise
  const word = truth.learned.get(cardId)
  if (word === undefined) {
    found.push(
      `sentence ${JSON.stringify(sentence)} comes from card ${cardId}, which is not learned`,
    )
  } else if (word.example[0] !== sentence) {
    found.push(
      `sentence ${JSON.stringify(sentence)} is not learned card ${cardId}'s example ${JSON.stringify(word.example[0])}`,
    )
  }
  for (const tile of exercise.tiles) {
    const inSolution = exercise.solution.includes(tile.id)
    if (inSolution && !sentence.includes(tile.text)) {
      found.push(`tile ${JSON.stringify(tile.text)} is not part of the sentence`)
    }
    if (!inSolution && !truth.learnedFronts.has(tile.text)) {
      found.push(`distractor tile ${JSON.stringify(tile.text)} is not a learned front`)
    }
  }
  return found
}

/** Which generator paths a run went through, so the leak test cannot pass by generating nothing. */
class Coverage {
  generated = new Map<ExerciseKind, number>()
  directions = new Set<ipc.ChoiceDirection>()
  storedAudio = false
  synthesizedAudio = false
  borrowedSentence = false
  distractorTiles = false
  latinSentence = false
  trapSentence = false
  fallbackTiles = false

  record(exercise: ipc.Exercise, truth: Truth): void {
    this.generated.set(exercise.kind, (this.generated.get(exercise.kind) ?? 0) + 1)
    switch (exercise.kind) {
      case 'multiple_choice':
        this.directions.add(exercise.direction)
        break
      case 'listening_choice':
        if (exercise.audio.audio_card_id === null) this.synthesizedAudio = true
        else this.storedAudio = true
        break
      case 'sentence_builder':
      case 'listening_sentence':
        this.borrowedSentence ||= exercise.sentence_card_id !== exercise.card_id
        this.distractorTiles ||= exercise.tiles.length > exercise.solution.length
        this.latinSentence ||= !chars(exercise.sentence).some(isCjkChar)
        this.trapSentence ||= exercise.sentence.includes('学生')
        this.fallbackTiles ||= exercise.tiles.some((tile) => truth.unlearnedWords.has(tile.text))
        break
    }
  }

  assertComplete(): void {
    for (const kind of EXERCISE_KINDS) {
      expect(this.generated.get(kind) ?? 0, `${kind} exercises`).toBeGreaterThan(100)
    }
    expect(this.directions.size, 'directions').toBe(3)
    expect(this.storedAudio && this.synthesizedAudio, 'both audio sources').toBe(true)
    expect(this.borrowedSentence, 'a sentence borrowed from another entry').toBe(true)
    expect(this.distractorTiles, 'distractor tiles').toBe(true)
    expect(this.latinSentence, 'a sentence without hanzi').toBe(true)
    expect(this.trapSentence, 'the sentence hiding the trap word 学生').toBe(true)
    expect(this.fallbackTiles, 'single-character fallback tiles').toBe(true)
  }
}

describe('the learned-vocabulary isolation of the web port', () => {
  it('has a fixture with far more vocabulary than the learner knows', async () => {
    const { storage, truth } = await fixture()
    const count = await storage.transaction((tx) => new Store(tx).curriculumCount())
    expect(count).toBe(CURRICULUM_SIZE)
    expect(new Set(truth.curriculum.map((item) => item.hanzi)).size).toBe(CURRICULUM_SIZE)
    expect(truth.learned.size).toBe(LEARNED.length)
    expect(truth.notLearned.size).toBe(NOT_LEARNED.length)
    expect(truth.unlearnedWords.size).toBeGreaterThanOrEqual(CURRICULUM_SIZE - LEARNED.length)
    for (const trap of TRAP_WORDS) {
      expect(truth.unlearnedWords.has(trap), trap).toBe(true)
      expect(
        chars(trap).every((c) => truth.learnedChars.has(c)),
        trap,
      ).toBe(true)
    }
    const withUnlearnedHanzi = truth.curriculum.filter((item) =>
      chars(item.example_sentence_source ?? '').some(
        (c) => isCjkChar(c) && !truth.learnedChars.has(c),
      ),
    )
    expect(withUnlearnedHanzi.length).toBeGreaterThan(CURRICULUM_SIZE - 10)
  })

  it('never shows vocabulary that was not learned, in any exercise kind, for any seed', async () => {
    const { storage, truth } = await fixture()
    const pool = await loadPool(storage)
    const coverage = new Coverage()
    const leaks: string[] = []
    const errors = new Map<string, string>()
    for (const cardId of truth.learned.keys()) {
      for (const kind of EXERCISE_KINDS) {
        for (const seed of seeds()) {
          try {
            const exercise = generateExercise(pool, cardId, kind, seed)
            coverage.record(exercise, truth)
            leaks.push(
              ...violations(truth, exercise).map(
                (leak) => `${kind} for card ${cardId}, seed ${seed}: ${leak}`,
              ),
            )
          } catch (err) {
            expect(err).toBeInstanceOf(QuizError)
            expect(
              (err as QuizError).kind,
              `learned card ${cardId} is missing from the pool`,
            ).not.toBe('not_in_pool')
            errors.set(`${cardId} ${kind}`, (err as QuizError).message)
          }
        }
      }
    }
    expect(leaks.slice(0, 15), `${leaks.length} leaks`).toEqual([])
    coverage.assertComplete()

    // Only `book`'s Chinese sides cannot be shown, and it has nothing else to
    // be asked about in a choice or with a translated prompt.
    const book = [...truth.learned].find(([, word]) => word.front === 'book')?.[0]
    expect(errors).toEqual(
      new Map([
        [`${book} multiple_choice`, QuizError.poolTooSmall(book ?? 0).message],
        [`${book} sentence_builder`, QuizError.noEligibleSentence(book ?? 0).message],
      ]),
    )
  })

  it('keeps curriculum rows and captures never introduced out of the pool', async () => {
    const { storage, truth } = await fixture()
    const pool = await loadPool(storage)
    expect(new Set([...truth.notLearned.values()].map((word) => word.origin))).toEqual(
      new Set(['capture', 'curriculum', 'anki_import']),
    )
    expect(new Set(pool.entries().map((entry) => entry.card_id))).toEqual(
      new Set(truth.learned.keys()),
    )
    expect(pool.learnedHanziChars()).toEqual(truth.learnedChars)
    for (const entry of pool.entries()) {
      expect(truth.learnedFronts.has(entry.front), entry.front).toBe(true)
      expect(truth.unlearnedWords.has(entry.front), entry.front).toBe(false)
    }
    for (const [cardId, word] of truth.notLearned) {
      expect(pool.get(cardId), word.front).toBeUndefined()
      for (const kind of EXERCISE_KINDS) {
        for (const seed of [0, 1, 99]) {
          expect(() => generateExercise(pool, cardId, kind, seed), `${word.front} ${kind}`).toThrow(
            QuizError.notInPool(cardId).message,
          )
        }
      }
    }

    // An srs_state row is what makes a card learned, and all that does.
    const he = [...truth.notLearned].find(([, word]) => word.front === '他')?.[0] ?? 0
    expect(pool.learnedHanziChars().has('他')).toBe(false)
    await storage.transaction((tx) => new Store(tx).upsertSrsState(learnedState(he)))
    const withHe = await loadPool(storage)
    expect(withHe.get(he)?.front).toBe('他')
    expect(withHe.learnedHanziChars().has('他')).toBe(true)
    expect(await storage.transaction((tx) => new Store(tx).deleteCapture(he))).toBe(true)
    const withoutHe = await loadPool(storage)
    expect(withoutHe.get(he)).toBeUndefined()
    expect(withoutHe.learnedHanziChars().has('他')).toBe(false)
  })

  it('has a checker that flags deliberately leaky exercises', async () => {
    const { storage, truth } = await fixture()
    const pool = await loadPool(storage)
    const idOf = (front: string) =>
      [...truth.learned, ...truth.notLearned].find(([, word]) => word.front === front)?.[0] ?? 0
    const generate = (front: string, kind: ExerciseKind) => {
      const exercise = generateExercise(pool, idOf(front), kind, 7)
      expect(violations(truth, exercise), `untampered ${kind}`).toEqual([])
      return exercise
    }
    const flagged = (exercise: ipc.Exercise, mentions: string) => {
      const found = violations(truth, exercise)
      expect(
        found.some((problem) => problem.includes(mentions)),
        `expected a violation mentioning ${mentions}, got ${JSON.stringify(found)}`,
      ).toBe(true)
    }
    const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value))

    const choice = generate('学习', 'multiple_choice') as ipc.MultipleChoiceExercise
    const distractor = (choice.correct_index + 1) % choice.options.length
    // A curriculum word as distractor: learned characters, unlearned word.
    const trap = clone(choice)
    trap.options[distractor].text = '学生'
    flagged(trap, 'unlearned word "学生"')
    // A captured but never introduced card as distractor.
    const unintroduced = clone(choice)
    unintroduced.options[distractor] = { card_id: idOf('他'), text: 'er' }
    flagged(unintroduced, 'which is not learned')
    // A curriculum meaning: no hanzi at all, caught by provenance.
    const meaning = clone(choice)
    meaning.options[distractor].text = 'Bedeutung 42'
    flagged(meaning, 'is not what learned card')
    // An unlearned hanzi in the prompt.
    const prompt = clone(choice)
    prompt.prompt = '书'
    flagged(prompt, 'unlearned hanzi 书')

    const builder = generate('学习', 'sentence_builder') as ipc.SentenceBuilderExercise
    // The trap word as an extra tile: every character is learned.
    const extraTile = clone(builder)
    extraTile.tiles.push({ id: extraTile.tiles.length, text: '学生' })
    flagged(extraTile, 'unlearned word "学生" shown as tile')
    // A tile made of an unlearned character.
    const unlearnedTile = clone(builder)
    unlearnedTile.tiles[0].text = '他'
    flagged(unlearnedTile, 'unlearned hanzi 他')
    // A curriculum sentence.
    const curriculumSentence = clone(builder)
    curriculumSentence.sentence = truth.curriculum[200].example_sentence_source ?? ''
    flagged(curriculumSentence, 'is not learned card')

    const listening = clone(generate('老师', 'listening_choice') as ipc.ListeningChoiceExercise)
    listening.audio.text = '他很好'
    flagged(listening, 'unlearned hanzi 他')

    const heard = clone(generate('老师', 'listening_sentence') as ipc.ListeningSentenceExercise)
    // The sentence of a captured but never introduced card, all learned hanzi.
    heard.sentence_card_id = idOf('你好')
    heard.sentence = '你好，老师！'
    heard.audio.text = heard.sentence
    flagged(heard, 'which is not learned')
  })
})
