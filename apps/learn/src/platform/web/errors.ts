/**
 * The core's typed errors (`vocab_core::store::StoreError`,
 * `vocab_core::quiz::QuizError`) and vocab-learn's command error, with
 * messages that are exactly their Rust `Display` output.
 *
 * The messages are part of what the golden vectors pin: a Learn command that
 * fails rejects with the error's sentence, and the web build must say the
 * same thing as the desktop build for the same failure. Values Rust formats
 * with `{:?}` go through `debugString` where the message is built.
 */

/** `StoreError`'s variants (`Io` cannot happen in a browser). */
export type StoreErrorKind = 'database' | 'json' | 'invalid' | 'not_found'

const STORE_PREFIX: Record<StoreErrorKind, string> = {
  database: 'database error',
  json: 'invalid JSON in the database',
  invalid: 'invalid data',
  not_found: 'not found',
}

/** `vocab_core::store::StoreError`. */
export class StoreError extends Error {
  private constructor(
    readonly kind: StoreErrorKind,
    /** The variant's payload: the text after `invalid data: ` and so on. */
    readonly detail: string,
  ) {
    super(`${STORE_PREFIX[kind]}: ${detail}`)
    this.name = 'StoreError'
  }

  /**
   * `StoreError::Sqlite`: a constraint the SQLite schema enforces, with
   * SQLite's own wording (`UNIQUE constraint failed: hsk_curriculum.hanzi`).
   */
  static database(detail: string): StoreError {
    return new StoreError('database', detail)
  }

  /** `StoreError::Json`. */
  static json(detail: string): StoreError {
    return new StoreError('json', detail)
  }

  /** `StoreError::Invalid`: data this build cannot interpret or store. */
  static invalid(detail: string): StoreError {
    return new StoreError('invalid', detail)
  }

  /** `StoreError::NotFound`. */
  static notFound(detail: string): StoreError {
    return new StoreError('not_found', detail)
  }

  /** `StoreError::in_row`: prefixes an `Invalid` message with the row it came from. */
  inRow(table: string, key: string | number): StoreError {
    return this.kind === 'invalid'
      ? StoreError.invalid(`${table} row ${key}: ${this.detail}`)
      : this
  }
}

/** `vocab_core::quiz::QuizError` (serialized with `kind` and `card_id`). */
export class QuizError extends Error {
  private constructor(
    readonly kind: 'not_in_pool' | 'pool_too_small' | 'no_eligible_sentence',
    readonly cardId: number,
    message: string,
  ) {
    super(message)
    this.name = 'QuizError'
  }

  static notInPool(cardId: number): QuizError {
    return new QuizError('not_in_pool', cardId, `card ${cardId} is not learned vocabulary`)
  }

  static poolTooSmall(cardId: number): QuizError {
    return new QuizError(
      'pool_too_small',
      cardId,
      `not enough learned vocabulary for a choice exercise about card ${cardId}`,
    )
  }

  static noEligibleSentence(cardId: number): QuizError {
    return new QuizError(
      'no_eligible_sentence',
      cardId,
      `no learned example sentence uses card ${cardId}`,
    )
  }
}

/** `vocab_core::srs::InvalidRating`. */
export class InvalidRating extends Error {
  constructor(readonly value: unknown) {
    super(`a review rating is 1 (Again) to 4 (Easy), not ${String(value)}`)
    this.name = 'InvalidRating'
  }
}

/**
 * vocab-learn's `Error` (apps/learn/src-tauri/src/error.rs): what a Learn
 * command rejects with, as the sentence the desktop build shows. The
 * preconditions a learner can run into read as what they are; anything the
 * core refuses is "the vocabulary database failed", with the core's reason.
 */
export class LearnError extends Error {
  private constructor(message: string) {
    super(message)
    this.name = 'LearnError'
  }

  static storeUnavailable(why: string): LearnError {
    return new LearnError(`The vocabulary database is unavailable (${why}).`)
  }

  static store(err: StoreError): LearnError {
    return new LearnError(`The vocabulary database failed (${err.message}).`)
  }

  static curriculumUnavailable(why: string): LearnError {
    return new LearnError(`The HSK curriculum is unavailable (${why}).`)
  }

  static noSuchCard(cardId: number): LearnError {
    return new LearnError(`There is no card ${cardId}.`)
  }

  static notScheduled(cardId: number): LearnError {
    return new LearnError(`Card ${cardId} is not scheduled for review.`)
  }

  static notLearned(cardId: number): LearnError {
    return new LearnError(`Card ${cardId} is not learned vocabulary.`)
  }

  static notInCurriculum(hanzi: string): LearnError {
    return new LearnError(`"${hanzi}" is not in the HSK curriculum.`)
  }

  static noSuchLesson(lessonId: string): LearnError {
    return new LearnError(`There is no lesson "${lessonId}".`)
  }

  static quiz(err: QuizError): LearnError {
    return new LearnError(`No exercise: ${err.message}.`)
  }

  /** Every problem the settings validation found, one sentence per line. */
  static invalidSettings(problems: readonly string[]): LearnError {
    return new LearnError(problems.join('\n'))
  }

  /** An argument the desktop command would not deserialize, in Tauri's words. */
  static invalidArgs(name: string, command: string, reason: string): LearnError {
    return new LearnError(`invalid args \`${name}\` for command \`${command}\`: ${reason}`)
  }

  /** What the web build cannot do: there is no TTS and no AnkiConnect in a browser. */
  static notAvailable(what: string): LearnError {
    return new LearnError(`${what} is not available in the web version.`)
  }

  /** `From<StoreError>`, `From<QuizError>` and the rest: any failure as a command's error. */
  static from(err: unknown): LearnError {
    if (err instanceof LearnError) return err
    if (err instanceof QuizError) return LearnError.quiz(err)
    if (err instanceof StoreError) return LearnError.store(err)
    return new LearnError(`The vocabulary database failed (${errorMessage(err)}).`)
  }
}

/** The sentence an error is shown with; `String(err)` for anything that is not an `Error`. */
export function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}
