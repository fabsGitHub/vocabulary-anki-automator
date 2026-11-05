/**
 * `vocab_core::quiz` and `quiz::exercises`: the four exercise types of
 * Milestone M2, built from a {@link LearnedVocabularyPool} and nothing else.
 *
 * An exercise is a pure function of the pool, the card, the kind and the
 * seed, so the port produces the desktop build's exercise only if it draws
 * the random numbers in exactly the Rust order. Each generator below keeps
 * the Rust structure for that reason:
 *
 * - *Multiple choice:* `pick(askable.length)` among the directions that have
 *   a prompt, an answer and at least one distractor; `shuffle` the full
 *   candidate list, then a stable sort by preference (same language pair,
 *   same script, the rest); the first three texts that differ from the
 *   answer and from each other; `shuffle` them; `pick(n + 1)` the answer's
 *   index.
 * - *Listening choice:* the same from the shuffle on, over fronts.
 * - *Sentence builder, listening sentence:* the target's own sentence when
 *   it qualifies, else `pick` among the others; `shuffle` the distractor
 *   candidates and keep two; `shuffle` sentence tiles and distractors.
 *
 * Two rules sit on top of "taken from the pool": every shown text contains
 * learned hanzi only, and no distractor is a second correct answer. Every
 * option of a multiple-choice exercise is in its `option_lang`.
 */

import type * as ipc from '../types'
import { QuizError } from './errors'
import { containsCjk, isCjkChar } from './language'
import type { LearnedVocabularyPool, PoolEntry } from './pool'
import { Mulberry32 } from './rng'
import { Dictionary, findWord, tiles as segmentTiles, wordTile } from './segment'
import { chars, hasWhitespace, isWhitespace, splitWhitespace, trim } from './text'

/** Distractors next to the correct option, at most. */
const CHOICE_DISTRACTORS = 3
/** Extra tiles that do not belong to the sentence, at most. */
const DISTRACTOR_TILES = 2
/** A sentence needs this many tiles to be worth ordering. */
const MIN_SENTENCE_TILES = 2
/** `captures.script_kind` of Chinese cards. */
const CHINESE = 'chinese'

export type ExerciseKind = ipc.ExerciseKind
export type ChoiceDirection = ipc.ChoiceDirection
export type Exercise = ipc.Exercise
export type ExerciseAnswer = ipc.ExerciseAnswer

/** `ExerciseKind::ALL`. */
export const EXERCISE_KINDS: readonly ExerciseKind[] = [
  'multiple_choice',
  'sentence_builder',
  'listening_choice',
  'listening_sentence',
]

/** The order in which a generator considers the directions. */
const DIRECTIONS: readonly ChoiceDirection[] = ['front_to_back', 'back_to_front', 'front_to_pinyin']

/**
 * `generate_exercise`: an exercise of `kind` about the learned card `cardId`
 * from `pool` alone. Throws a {@link QuizError} when there is none.
 */
export function generateExercise(
  pool: LearnedVocabularyPool,
  cardId: number,
  kind: ExerciseKind,
  seed: number,
): Exercise {
  const target = pool.get(cardId)
  if (target === undefined) throw QuizError.notInPool(cardId)
  const rng = new Mulberry32(seed)
  switch (kind) {
    case 'multiple_choice':
      return { kind, ...multipleChoice(pool, target, rng) }
    case 'sentence_builder':
      return { kind, ...sentenceBuilder(pool, target, rng) }
    case 'listening_choice':
      return { kind, ...listeningChoice(pool, target, rng) }
    case 'listening_sentence':
      return { kind, ...listeningSentence(pool, target, rng) }
  }
}

/**
 * `check_answer`: whether `answer` solves `exercise`. An answer of the
 * wrong shape is wrong, as is an order that uses a tile twice or a tile the
 * exercise does not have.
 */
export function checkAnswer(exercise: Exercise, answer: ExerciseAnswer): boolean {
  switch (exercise.kind) {
    case 'multiple_choice':
    case 'listening_choice':
      return (
        answer.kind === 'choice' &&
        choiceIsCorrect(exercise.options, exercise.correct_index, answer.index)
      )
    case 'sentence_builder':
    case 'listening_sentence':
      return (
        answer.kind === 'order' &&
        orderIsCorrect(exercise.tiles, exercise.solution, answer.tile_ids)
      )
  }
}

function prompt(direction: ChoiceDirection, entry: PoolEntry): string {
  return direction === 'back_to_front' ? entry.back : entry.front
}

function answer(direction: ChoiceDirection, entry: PoolEntry): string {
  switch (direction) {
    case 'front_to_back':
      return entry.back
    case 'back_to_front':
      return entry.front
    case 'front_to_pinyin':
      return entry.script_kind === CHINESE ? (entry.pinyin ?? '') : ''
  }
}

function answerKey(direction: ChoiceDirection, text: string): string {
  return direction === 'front_to_pinyin' ? pinyinKey(text) : textKey(text)
}

function promptLang(direction: ChoiceDirection, entry: PoolEntry): string {
  return direction === 'back_to_front' ? entry.target_lang : entry.source_lang
}

function optionLang(direction: ChoiceDirection, entry: PoolEntry): string {
  return direction === 'front_to_back' ? entry.target_lang : entry.source_lang
}

type Fields<T extends Exercise> = Omit<T, 'kind'>

function multipleChoice(
  pool: LearnedVocabularyPool,
  target: PoolEntry,
  rng: Mulberry32,
): Fields<ipc.MultipleChoiceExercise> {
  const askable: [ChoiceDirection, PoolEntry[]][] = []
  for (const direction of DIRECTIONS) {
    const candidates = choiceCandidates(pool, target, direction)
    if (candidates !== null && candidates.length > 0) askable.push([direction, candidates])
  }
  if (askable.length === 0) throw QuizError.poolTooSmall(target.card_id)
  const [direction, candidates] = askable[pick(rng, askable.length)]
  const [options, correctIndex] = chooseOptions(
    target,
    trim(answer(direction, target)),
    [...candidates],
    (entry) => answer(direction, entry),
    (text) => answerKey(direction, text),
    rng,
  )
  return {
    card_id: target.card_id,
    direction,
    prompt: trim(prompt(direction, target)),
    prompt_lang: promptLang(direction, target),
    option_lang: optionLang(direction, target),
    options,
    correct_index: correctIndex,
  }
}

/**
 * `choice_candidates`: the entries that can be wrong answers to `target`
 * asked in `direction`, in pool order; `null` when the target cannot be
 * asked that way (a blank or not-shown prompt or answer, or no pinyin).
 */
function choiceCandidates(
  pool: LearnedVocabularyPool,
  target: PoolEntry,
  direction: ChoiceDirection,
): PoolEntry[] | null {
  const hanzi = pool.hanzi()
  const shownPrompt = shown(prompt(direction, target), hanzi)
  if (shownPrompt === null) return null
  const promptKey = textKey(shownPrompt)
  const shownAnswer = shown(answer(direction, target), hanzi)
  if (shownAnswer === null) return null
  const correctKey = answerKey(direction, shownAnswer)
  const lang = optionLang(direction, target)
  return pool.entries().filter((entry) => {
    if (entry.card_id === target.card_id) return false
    // Every option is in the exercise's option language: an English meaning
    // among German ones would give the answer away.
    if (optionLang(direction, entry) !== lang) return false
    if (textKey(prompt(direction, entry)) === promptKey) return false
    const text = shown(answer(direction, entry), hanzi)
    return text !== null && answerKey(direction, text) !== correctKey
  })
}

function listeningChoice(
  pool: LearnedVocabularyPool,
  target: PoolEntry,
  rng: Mulberry32,
): Fields<ipc.ListeningChoiceExercise> {
  const hanzi = pool.hanzi()
  const front = shown(target.front, hanzi)
  if (front === null) throw QuizError.poolTooSmall(target.card_id)
  const frontKey = textKey(front)
  const sound = target.pinyin === null ? null : pinyinKey(target.pinyin)
  const candidates = pool.entries().filter((entry) => {
    if (entry.card_id === target.card_id) return false
    const text = shown(entry.front, hanzi)
    if (text === null || textKey(text) === frontKey) return false
    return sound === null || (entry.pinyin === null ? null : pinyinKey(entry.pinyin)) !== sound
  })
  if (candidates.length === 0) throw QuizError.poolTooSmall(target.card_id)
  const [options, correctIndex] = chooseOptions(
    target,
    front,
    candidates,
    (entry) => entry.front,
    textKey,
    rng,
  )
  return {
    card_id: target.card_id,
    audio: {
      audio_card_id: target.has_audio ? target.card_id : null,
      text: front,
      lang: target.source_lang,
    },
    options,
    correct_index: correctIndex,
  }
}

/**
 * `choose_options`: the correct option plus up to {@link CHOICE_DISTRACTORS}
 * distractors with distinct texts, in random order, and the correct one's
 * index. The whole candidate list is shuffled, then stably sorted by
 * preference, so distractors are uniform within each preference.
 */
function chooseOptions(
  target: PoolEntry,
  correct: string,
  candidates: PoolEntry[],
  textOf: (entry: PoolEntry) => string,
  keyOf: (text: string) => string,
  rng: Mulberry32,
): [ipc.ChoiceOption[], number] {
  rng.shuffle(candidates)
  candidates.sort((a, b) => preference(target, a) - preference(target, b))
  const seen = new Set([keyOf(correct)])
  const options: ipc.ChoiceOption[] = []
  for (const entry of candidates) {
    if (options.length === CHOICE_DISTRACTORS) break
    const text = trim(textOf(entry))
    const key = keyOf(text)
    if (seen.has(key)) continue
    seen.add(key)
    options.push({ card_id: entry.card_id, text })
  }
  rng.shuffle(options)
  const correctIndex = pick(rng, options.length + 1)
  options.splice(correctIndex, 0, { card_id: target.card_id, text: correct })
  return [options, correctIndex]
}

/** 0 for the target's language pair, 1 for its script, 2 for anything else. */
function preference(target: PoolEntry, entry: PoolEntry): number {
  if (entry.source_lang === target.source_lang && entry.target_lang === target.target_lang) {
    return 0
  }
  return entry.script_kind === target.script_kind ? 1 : 2
}

function sentenceBuilder(
  pool: LearnedVocabularyPool,
  target: PoolEntry,
  rng: Mulberry32,
): Fields<ipc.SentenceBuilderExercise> {
  const dictionary = Dictionary.fromPool(pool)
  const source = pickSentence(pool, target, true, dictionary, rng)
  const [tiles, solution] = buildTiles(pool, source, dictionary, rng)
  return {
    card_id: target.card_id,
    sentence_card_id: source.entry.card_id,
    prompt: trim(source.entry.example_target),
    sentence: source.sentence,
    sentence_lang: source.entry.source_lang,
    tiles,
    solution,
  }
}

function listeningSentence(
  pool: LearnedVocabularyPool,
  target: PoolEntry,
  rng: Mulberry32,
): Fields<ipc.ListeningSentenceExercise> {
  const dictionary = Dictionary.fromPool(pool)
  const source = pickSentence(pool, target, false, dictionary, rng)
  const [tiles, solution] = buildTiles(pool, source, dictionary, rng)
  return {
    card_id: target.card_id,
    sentence_card_id: source.entry.card_id,
    audio: { audio_card_id: null, text: source.sentence, lang: source.entry.source_lang },
    sentence: source.sentence,
    tiles,
    solution,
  }
}

/** `SentenceSource`: a usable example sentence, already cut into tiles. */
interface SentenceSource {
  entry: PoolEntry
  /** The trimmed `example_source`. */
  sentence: string
  tiles: string[]
}

/** `pick_sentence`: the target's own sentence when it qualifies, else a random other one. */
function pickSentence(
  pool: LearnedVocabularyPool,
  target: PoolEntry,
  needsTranslation: boolean,
  dictionary: Dictionary,
  rng: Mulberry32,
): SentenceSource {
  const usable = (entry: PoolEntry) =>
    usableSentence(pool, target, entry, needsTranslation, dictionary)
  const own = usable(target)
  if (own !== null) return own
  const others = pool.entries().flatMap((entry) => {
    if (entry.card_id === target.card_id) return []
    const source = usable(entry)
    return source === null ? [] : [source]
  })
  if (others.length === 0) throw QuizError.noEligibleSentence(target.card_id)
  return others[pick(rng, others.length)]
}

/**
 * `usable_sentence`: `entry`'s example sentence, if it contains the target's
 * front, shows learned hanzi only (and so does its translation, when the
 * exercise displays it) and has at least {@link MIN_SENTENCE_TILES} tiles.
 */
function usableSentence(
  pool: LearnedVocabularyPool,
  target: PoolEntry,
  entry: PoolEntry,
  needsTranslation: boolean,
  dictionary: Dictionary,
): SentenceSource | null {
  const hanzi = pool.hanzi()
  const sentence = shown(entry.example_source, hanzi)
  if (sentence === null) return null
  if (findWord(sentence, target.front) === null) return null
  if (needsTranslation && !learnedOnly(entry.example_target, hanzi)) return null
  const tiles = segmentTiles(sentence, dictionary)
  return tiles.length >= MIN_SENTENCE_TILES ? { entry, sentence, tiles } : null
}

/** `build_tiles`: sentence and distractor tiles, shuffled, and the ids in sentence order. */
function buildTiles(
  pool: LearnedVocabularyPool,
  source: SentenceSource,
  dictionary: Dictionary,
  rng: Mulberry32,
): [ipc.Tile[], number[]] {
  const distractors = distractorTiles(pool, source, dictionary, rng)
  const solution = new Array<number>(source.tiles.length).fill(0)
  const pieces: [number | null, string][] = [
    ...source.tiles.map((text, position): [number | null, string] => [position, text]),
    ...distractors.map((text): [number | null, string] => [null, text]),
  ]
  rng.shuffle(pieces)
  const tiles = pieces.map(([position, text], id) => {
    if (position !== null) solution[position] = id
    return { id, text }
  })
  return [tiles, solution]
}

/**
 * `distractor_tiles`: up to {@link DISTRACTOR_TILES} fronts of the
 * sentence's language that could be one of its tiles but do not occur in
 * it, so no arrangement that uses one spells the sentence.
 */
function distractorTiles(
  pool: LearnedVocabularyPool,
  source: SentenceSource,
  dictionary: Dictionary,
  rng: Mulberry32,
): string[] {
  const chinese = containsCjk(source.sentence)
  const joined = source.tiles.join('')
  const seen = new Set<string>()
  const candidates: string[] = []
  for (const entry of pool.entries()) {
    if (entry.source_lang !== source.entry.source_lang) continue
    const text = frontAsTile(entry.front, chinese, dictionary)
    if (text === null) continue
    const occurs = chinese ? joined.includes(text) : findWord(source.sentence, text) !== null
    if (occurs) continue
    const key = textKey(text)
    if (seen.has(key)) continue
    seen.add(key)
    candidates.push(text)
  }
  rng.shuffle(candidates)
  return candidates.slice(0, DISTRACTOR_TILES)
}

/** A front as a tile of a Chinese sentence (a dictionary word) or of another one (one word, no hanzi). */
function frontAsTile(front: string, chinese: boolean, dictionary: Dictionary): string | null {
  const text = trim(front)
  if (chinese) return dictionary.contains(text) ? text : null
  if (containsCjk(text) || hasWhitespace(text)) return null
  return wordTile(text)
}

/** `choice_is_correct`. */
function choiceIsCorrect(
  options: readonly ipc.ChoiceOption[],
  correctIndex: number,
  index: number,
): boolean {
  return index === correctIndex && index < options.length
}

/**
 * `order_is_correct`: as many tiles as the solution, none twice, and the
 * same texts in the same order (tiles with equal texts are interchangeable).
 */
function orderIsCorrect(
  tiles: readonly ipc.Tile[],
  solution: readonly number[],
  given: readonly number[],
): boolean {
  const text = (id: number) => tiles.find((tile) => tile.id === id)?.text
  const used = new Set<number>()
  return (
    given.length === solution.length &&
    given.every((id, index) => {
      if (used.has(id)) return false
      used.add(id)
      return text(id) !== undefined && text(id) === text(solution[index])
    })
  )
}

/** `shown`: `text` trimmed, when it is not blank and every hanzi in it is learned. */
function shown(text: string, hanzi: ReadonlySet<string>): string | null {
  const trimmed = trim(text)
  return trimmed !== '' && learnedOnly(trimmed, hanzi) ? trimmed : null
}

function learnedOnly(text: string, hanzi: ReadonlySet<string>): boolean {
  return chars(text).every((c) => !isCjkChar(c) || hanzi.has(c))
}

/** `text_key`: case and runs of whitespace do not make two texts different. */
function textKey(text: string): string {
  return splitWhitespace(text).join(' ').toLowerCase()
}

/** `pinyin_key`: `xuéxí` and `xué xí` are the same sound. */
function pinyinKey(text: string): string {
  return chars(text)
    .filter((c) => !isWhitespace(c))
    .join('')
    .toLowerCase()
}

/** `pick`: a uniform index below `length`. */
function pick(rng: Mulberry32, length: number): number {
  return rng.below(Math.min(length, 4_294_967_295))
}
