/**
 * Builders for the platform shapes of types.ts, filled with plausible values
 * so a test only spells out what it is about.
 */

import type {
  AnkiSyncReport,
  Card,
  DailyPromptState,
  DailySession,
  LearnSettings,
  LearnedCard,
  Lesson,
  LessonCard,
  ListeningChoiceExercise,
  ListeningSentenceExercise,
  MultipleChoiceExercise,
  RatingPreview,
  ReviewResult,
  SentenceBuilderExercise,
  SrsState,
  StatsSummary,
  StoredCapture,
  StreakSummary,
} from '../platform/types'

/** 2026-10-31 12:00:00.000 UTC, the instant every fixture is relative to. */
export const NOW = Date.UTC(2026, 9, 31, 12, 0, 0)
export const TODAY = '2026-10-31'

const iso = (ms: number) => new Date(ms).toISOString()

export function session(overrides: Partial<DailySession> = {}): DailySession {
  return {
    session_date: TODAY,
    requested_new_count: 5,
    reviews_due_count: 2,
    reviews_completed: 0,
    new_items_completed: 0,
    completed_at: null,
    ...overrides,
  }
}

export function promptState(overrides: Partial<DailyPromptState> = {}): DailyPromptState {
  return { needs_prompt: false, today: TODAY, session: session(), ...overrides }
}

export function card(overrides: Partial<Card> = {}): Card {
  return {
    front: '学习',
    back: 'to study',
    example_source: '我在学习中文。',
    example_target: 'I am studying Chinese.',
    notes: '',
    tags: [],
    source_lang: 'zh-CN',
    target_lang: 'en',
    captured_at: iso(NOW),
    audio_front: null,
    script_metadata: {
      kind: 'chinese',
      pinyin: 'xué xí',
      hsk_level: 1,
      script_variant: 'simplified',
    },
    ...overrides,
  }
}

export function capture(
  overrides: Partial<Omit<StoredCapture, 'card'>> & { card?: Partial<Card> } = {},
): StoredCapture {
  return {
    id: 1,
    origin: 'capture',
    synced_to_anki: false,
    ...overrides,
    card: card(overrides.card),
  }
}

/** FSRS-like previews: Again 1 min, Hard 6 min, Good 10 min (learning steps), Easy 3 days. */
export function previews(): RatingPreview[] {
  return [
    { rating: 1, due_date: iso(NOW + 60_000), interval_days: 0 },
    { rating: 2, due_date: iso(NOW + 6 * 60_000), interval_days: 0 },
    { rating: 3, due_date: iso(NOW + 10 * 60_000), interval_days: 0 },
    { rating: 4, due_date: iso(NOW + 3 * 86_400_000), interval_days: 3 },
  ]
}

export function srsState(overrides: Partial<SrsState> = {}): SrsState {
  return {
    card_id: 1,
    algorithm: 'fsrs',
    stability: 3.2,
    difficulty: 5.1,
    interval_days: 3,
    repetitions: 1,
    lapses: 0,
    fsrs_state: 'review',
    elapsed_days: 0,
    due_date: iso(NOW + 3 * 86_400_000),
    last_reviewed_at: iso(NOW),
    last_rating: 3,
    ...overrides,
  }
}

export function streak(overrides: Partial<StreakSummary> = {}): StreakSummary {
  return { current: 4, longest: 9, freezes_available: 1, completed_today: false, ...overrides }
}

export function reviewResult(overrides: Partial<ReviewResult> = {}): ReviewResult {
  return {
    next: srsState(),
    xp_awarded: 10,
    session: session(),
    session_completed: false,
    streak: streak(),
    ...overrides,
  }
}

export function stats(overrides: Partial<StatsSummary> = {}): StatsSummary {
  return {
    xp_total: 1250,
    xp_today: 35,
    streak: streak(),
    learned_count: 42,
    due_today: 7,
    ...overrides,
  }
}

export function learnedCard(overrides: Partial<LearnedCard> = {}): LearnedCard {
  return {
    card_id: 1,
    front: '学习',
    back: 'to study',
    pinyin: 'xué xí',
    hsk_level: 1,
    source_lang: 'zh-CN',
    target_lang: 'en',
    example_source: '我在学习中文。',
    example_target: 'I am studying Chinese.',
    script_kind: 'chinese',
    has_audio: false,
    ...overrides,
  }
}

export function multipleChoice(
  overrides: Partial<MultipleChoiceExercise> = {},
): MultipleChoiceExercise {
  return {
    kind: 'multiple_choice',
    card_id: 1,
    direction: 'front_to_back',
    prompt: '学习',
    prompt_lang: 'zh-CN',
    option_lang: 'en',
    options: [
      { card_id: 2, text: 'teacher' },
      { card_id: 1, text: 'to study' },
      { card_id: 3, text: 'China' },
      { card_id: 4, text: 'to eat' },
    ],
    correct_index: 1,
    ...overrides,
  }
}

export function sentenceBuilder(
  overrides: Partial<SentenceBuilderExercise> = {},
): SentenceBuilderExercise {
  return {
    kind: 'sentence_builder',
    card_id: 1,
    sentence_card_id: 1,
    prompt: 'I am studying Chinese.',
    sentence: '我在学习中文。',
    sentence_lang: 'zh-CN',
    // Sentence order 我 在 学习 中文 is tile ids 2, 0, 3, 1; tile 4 is a distractor.
    tiles: [
      { id: 0, text: '在' },
      { id: 1, text: '中文' },
      { id: 2, text: '我' },
      { id: 3, text: '学习' },
      { id: 4, text: '老师' },
    ],
    solution: [2, 0, 3, 1],
    ...overrides,
  }
}

export function listeningChoice(
  overrides: Partial<ListeningChoiceExercise> = {},
): ListeningChoiceExercise {
  return {
    kind: 'listening_choice',
    card_id: 1,
    audio: { audio_card_id: 1, text: '学习', lang: 'zh-CN' },
    options: [
      { card_id: 1, text: '学习' },
      { card_id: 2, text: '老师' },
      { card_id: 3, text: '中国' },
    ],
    correct_index: 0,
    ...overrides,
  }
}

export function listeningSentence(
  overrides: Partial<ListeningSentenceExercise> = {},
): ListeningSentenceExercise {
  return {
    kind: 'listening_sentence',
    card_id: 1,
    sentence_card_id: 2,
    audio: { audio_card_id: null, text: '我在学习中文。', lang: 'zh-CN' },
    sentence: '我在学习中文。',
    tiles: [
      { id: 0, text: '学习' },
      { id: 1, text: '我' },
      { id: 2, text: '中文' },
      { id: 3, text: '在' },
    ],
    solution: [1, 3, 0, 2],
    ...overrides,
  }
}

export function lessonCard(overrides: Partial<LessonCard> = {}): LessonCard {
  return {
    card_id: 11,
    word: 'serendipity',
    translation: 'glücklicher Zufall',
    sentence_source: 'Finding it was pure serendipity.',
    sentence_target: 'Es zu finden war reiner Zufall.',
    cloze: 'Finding it was pure ____.',
    pinyin: null,
    ...overrides,
  }
}

export function lesson(overrides: Partial<Lesson> = {}): Lesson {
  return {
    id: 'lesson-11',
    title: 'EN → DE · Lesson 1',
    source_lang: 'en',
    target_lang: 'de',
    cards: [lessonCard()],
    ...overrides,
  }
}

export function settings(edit: (settings: LearnSettings) => void = () => {}): LearnSettings {
  const value: LearnSettings = {
    meaning_lang: 'de',
    anki: {
      connect_url: 'http://127.0.0.1:8765',
      deck_name: 'Chinese::HSK',
      model_name: 'Chinese (Hanzi)',
      field_mapping: {
        fields: [
          { anki_field: 'Hanzi', source: 'front' },
          { anki_field: 'Pinyin', source: 'pinyin' },
          { anki_field: 'Meaning', source: 'back' },
        ],
        audio_field: 'Audio',
      },
      create_reverse: false,
    },
  }
  edit(value)
  return value
}

export function report(overrides: Partial<AnkiSyncReport> = {}): AnkiSyncReport {
  return { attempted: 3, succeeded: 3, skipped: 0, failed: [], ...overrides }
}
