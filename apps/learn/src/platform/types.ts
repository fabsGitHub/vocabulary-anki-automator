// The platform contract of the Learn app (docs/architecture.md § 10).
//
// Every view talks to a PlatformAdapter and nothing else. The desktop build
// implements it with Tauri IPC to the vocab-learn commands (Rust, shared
// SQLite store); the web build implements it with IndexedDB and a TypeScript
// port of vocab-core (decision 10, § 12 item 7). Both must behave the same,
// so every shape below mirrors the serde output of the Rust types named in
// its comment. Changing a shape means changing the Rust type, both adapters
// and docs/architecture.md together.

/** Local calendar date, `YYYY-MM-DD`. */
export type IsoDate = string
/** RFC 3339 instant in UTC (serde's chrono format). */
export type Instant = string

/** vocab_core::model::ScriptMetadata */
export type ScriptMetadata =
  | { kind: 'none' }
  | {
      kind: 'chinese'
      pinyin: string
      hsk_level: number | null
      script_variant: 'simplified' | 'traditional'
    }

/** vocab_core::model::Card */
export interface Card {
  front: string
  back: string
  example_source: string
  example_target: string
  notes: string
  tags: string[]
  source_lang: string
  target_lang: string
  captured_at: Instant
  /** Base64 MP3, or null. */
  audio_front: string | null
  script_metadata: ScriptMetadata
}

/** vocab_core::model::StoredCapture */
export interface StoredCapture {
  id: number
  card: Card
  origin: 'capture' | 'curriculum' | 'anki_import'
  synced_to_anki: boolean
}

/** vocab_core::store::DailySessionRow */
export interface DailySession {
  session_date: IsoDate
  requested_new_count: number
  reviews_due_count: number
  reviews_completed: number
  new_items_completed: number
  completed_at: Instant | null
}

/** vocab_core::srs::DailyPromptState */
export interface DailyPromptState {
  needs_prompt: boolean
  today: IsoDate
  session: DailySession | null
}

/** vocab_core::srs::QueueItem */
export type QueueItem =
  | { kind: 'review'; card_id: number; due_date: Instant }
  | { kind: 'new'; hanzi: string; curriculum_order: number }

/** vocab_core::srs::ReviewRating: 1 = Again, 2 = Hard, 3 = Good, 4 = Easy. */
export type Rating = 1 | 2 | 3 | 4

/** vocab_core::srs::RatingPreview */
export interface RatingPreview {
  rating: Rating
  due_date: Instant
  interval_days: number
}

/** vocab_core::store::SrsStateRow */
export interface SrsState {
  card_id: number
  algorithm: string
  stability: number | null
  difficulty: number | null
  interval_days: number
  repetitions: number
  lapses: number
  fsrs_state: 'new' | 'learning' | 'review' | 'relearning'
  elapsed_days: number
  due_date: Instant
  last_reviewed_at: Instant | null
  last_rating: Rating | null
}

/** vocab_core::gamification::StreakSummary */
export interface StreakSummary {
  current: number
  longest: number
  freezes_available: number
  completed_today: boolean
}

/** vocab_core::gamification::StatsSummary */
export interface StatsSummary {
  xp_total: number
  xp_today: number
  streak: StreakSummary
  learned_count: number
  due_today: number
}

/** vocab-learn's submit_review_rating result. */
export interface ReviewResult {
  next: SrsState
  /** XP awarded for this review, including a session-completion bonus. */
  xp_awarded: number
  session: DailySession | null
  /** True only for the review that completed today's session. */
  session_completed: boolean
  streak: StreakSummary
}

/** vocab_core::quiz::PoolEntry — one learned card. */
export interface LearnedCard {
  card_id: number
  front: string
  back: string
  pinyin: string | null
  hsk_level: number | null
  source_lang: string
  target_lang: string
  example_source: string
  example_target: string
  script_kind: string
  has_audio: boolean
}

/** vocab_core::quiz::ExerciseKind */
export type ExerciseKind =
  'multiple_choice' | 'sentence_builder' | 'listening_choice' | 'listening_sentence'

/** vocab_core::quiz::ChoiceDirection */
export type ChoiceDirection = 'front_to_back' | 'back_to_front' | 'front_to_pinyin'

/** vocab_core::quiz::ChoiceOption */
export interface ChoiceOption {
  card_id: number
  text: string
}

/** vocab_core::quiz::AudioPrompt: play the card's stored audio, else synthesize `text`. */
export interface AudioPrompt {
  audio_card_id: number | null
  text: string
  lang: string
}

/** vocab_core::quiz::Tile */
export interface Tile {
  id: number
  text: string
}

export interface MultipleChoiceExercise {
  kind: 'multiple_choice'
  card_id: number
  direction: ChoiceDirection
  prompt: string
  prompt_lang: string
  option_lang: string
  options: ChoiceOption[]
  correct_index: number
}

export interface SentenceBuilderExercise {
  kind: 'sentence_builder'
  card_id: number
  sentence_card_id: number
  prompt: string
  sentence: string
  sentence_lang: string
  tiles: Tile[]
  solution: number[]
}

export interface ListeningChoiceExercise {
  kind: 'listening_choice'
  card_id: number
  audio: AudioPrompt
  options: ChoiceOption[]
  correct_index: number
}

export interface ListeningSentenceExercise {
  kind: 'listening_sentence'
  card_id: number
  sentence_card_id: number
  audio: AudioPrompt
  sentence: string
  tiles: Tile[]
  solution: number[]
}

/** vocab_core::quiz::Exercise (internally tagged by `kind`). */
export type Exercise =
  | MultipleChoiceExercise
  | SentenceBuilderExercise
  | ListeningChoiceExercise
  | ListeningSentenceExercise

/** vocab_core::quiz::ExerciseAnswer */
export type ExerciseAnswer =
  { kind: 'choice'; index: number } | { kind: 'order'; tile_ids: number[] }

/** vocab_core::lessons::LessonCard */
export interface LessonCard {
  card_id: number
  word: string
  translation: string
  sentence_source: string
  sentence_target: string
  cloze: string
  pinyin: string | null
}

/** vocab_core::lessons::Lesson */
export interface Lesson {
  id: string
  title: string
  source_lang: string
  target_lang: string
  cards: LessonCard[]
}

/** anki_bridge::CardField */
export type CardField =
  | 'front'
  | 'back'
  | 'example_source'
  | 'example_target'
  | 'example'
  | 'notes'
  | 'pinyin'
  | 'hsk_level'

/** anki_bridge::FieldMapping */
export interface FieldMapping {
  fields: { anki_field: string; source: CardField }[]
  audio_field: string | null
}

/** Learn's settings (vocab-learn keeps them in app_state 'learn.settings'). */
export interface LearnSettings {
  /** Language of the meanings shown for new curriculum words. */
  meaning_lang: 'de' | 'en'
  anki: {
    connect_url: string
    deck_name: string
    model_name: string
    field_mapping: FieldMapping
    create_reverse: boolean
  }
}

/** Result of anki_export / anki_import. */
export interface AnkiSyncReport {
  attempted: number
  succeeded: number
  /** Notes Anki already had (export) or fronts already captured (import). */
  skipped: number
  failed: { card_id: number | null; front: string; error: string }[]
}

/** What the running platform can do; views hide what it cannot. */
export interface PlatformCapabilities {
  kind: 'tauri' | 'web'
  /** Text-to-speech for cards without stored audio. */
  tts: boolean
  /** AnkiConnect export/import. */
  anki: boolean
}

/**
 * Tauri command for each method (JS args camelCase):
 * get_daily_prompt_state, submit_daily_quota{newCount}, dismiss_daily_prompt,
 * reopen_daily_prompt, get_daily_queue, start_new_item{hanzi}, get_card{cardId},
 * preview_ratings{cardId}, submit_review_rating{cardId, rating},
 * list_learned_cards, next_exercise{cardId, kind, seed},
 * check_exercise{exercise, answer}, record_exercise_result{cardId, kind, correct},
 * get_stats, list_lessons, complete_lesson{lessonId}, get_card_audio{cardId},
 * synthesize_audio{text, lang}, get_settings, save_settings{settings},
 * anki_export, anki_import{deck}.
 *
 * Every method rejects with a human-readable string on failure.
 */
export interface PlatformAdapter {
  readonly capabilities: PlatformCapabilities

  // Daily onboarding prompt (Milestone L2).
  getDailyPromptState(): Promise<DailyPromptState>
  submitDailyQuota(newCount: number): Promise<DailySession>
  /** "Not today": quota 0, reviews only (the documented dismiss rule). */
  dismissDailyPrompt(): Promise<DailySession>
  /** Lets the menu action show the prompt again today. */
  reopenDailyPrompt(): Promise<void>

  // Review session (Milestone L3).
  getDailyQueue(): Promise<QueueItem[]>
  /** Introduces a curriculum word; returns its card id (idempotent). */
  startNewItem(hanzi: string): Promise<number>
  getCard(cardId: number): Promise<StoredCapture>
  previewRatings(cardId: number): Promise<RatingPreview[]>
  submitReviewRating(cardId: number, rating: Rating): Promise<ReviewResult>

  // Quizzes (Milestone M) — content only ever comes from the learned pool.
  listLearnedCards(): Promise<LearnedCard[]>
  nextExercise(cardId: number, kind: ExerciseKind, seed: number): Promise<Exercise>
  checkExercise(exercise: Exercise, answer: ExerciseAnswer): Promise<boolean>
  /** Returns the XP awarded (only correct answers earn XP). */
  recordExerciseResult(cardId: number, kind: ExerciseKind, correct: boolean): Promise<number>

  // Lessons, stats (Milestone I).
  listLessons(): Promise<Lesson[]>
  /** Returns the XP awarded (at most once per lesson per day). */
  completeLesson(lessonId: string): Promise<number>
  getStats(): Promise<StatsSummary>

  // Audio: base64 MP3, or null when unavailable.
  getCardAudio(cardId: number): Promise<string | null>
  synthesizeAudio(text: string, lang: string): Promise<string | null>

  // Settings and Anki portability (Milestone I).
  getSettings(): Promise<LearnSettings>
  saveSettings(settings: LearnSettings): Promise<void>
  ankiExport(): Promise<AnkiSyncReport>
  ankiImport(deck: string): Promise<AnkiSyncReport>
}
