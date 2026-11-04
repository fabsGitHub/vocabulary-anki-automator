/**
 * The IPC wire shapes, mirrored from docs/architecture.md § 8.1 (config) and
 * § 11 (serialized shapes).
 *
 * These types describe exactly what the Rust side serializes, field names in
 * snake_case included, so a change to the contract is a change to this one
 * file. Nothing here is derived or convenient on purpose: UI-specific state
 * lives in the composables, not in these interfaces.
 */

/** Script-specific card annotations; `none` for every non-Chinese card. */
export type ScriptMetadata =
  | { kind: 'none' }
  | {
      kind: 'chinese'
      pinyin: string
      /** HSK 3.0 level 1-7, where 7 is the combined 7-9 band; `null` outside the curriculum. */
      hsk_level: number | null
      script_variant: 'simplified' | 'traditional'
    }

/** The neutral, Anki-agnostic card (`vocab_core::Card`). */
export interface Card {
  front: string
  back: string
  example_source: string
  example_target: string
  notes: string
  tags: string[]
  /** Canonical language code (§ 3). */
  source_lang: string
  /** Canonical language code (§ 3). */
  target_lang: string
  /** RFC 3339; with or without milliseconds (§ 2). */
  captured_at: string
  /** TTS of `front` as base64 MP3. */
  audio_front: string | null
  script_metadata: ScriptMetadata
}

/** A typed provider failure (`vocab_core::ProviderError`). */
export type ProviderError =
  | { kind: 'network' | 'quota_exceeded' | 'malformed'; provider: string; message: string }
  | { kind: 'http'; provider: string; status: number; message: string }
  | { kind: 'not_found'; provider: string }
  | { kind: 'unsupported'; provider: string; what: string }

/** The pipeline step a warning comes from (§ 5). */
export type PipelineStage =
  'translate' | 'rarity' | 'example' | 'example_translate' | 'tts' | 'annotate'

/** A provider failure the capture survived; its field was left empty. */
export interface PipelineWarning {
  stage: PipelineStage
  error: ProviderError
}

/** The writing system most of a text's letters belong to (`vocab_core::detect::Script`). */
export type Script = 'latin' | 'han' | 'kana' | 'hangul' | 'cyrillic' | 'other' | 'none'

/**
 * How the source language was chosen: by script (Han text is Chinese), as the
 * only learned language of the script, statistically, by the last approved
 * language breaking a close call, as the fallback, or by the user's switch.
 */
export type DetectionMethod =
  'script' | 'single' | 'statistical' | 'tie_break' | 'fallback' | 'chosen'

/** One candidate's statistical score, 0..1. */
export interface LanguageScore {
  lang: string
  confidence: number
}

/** Which learned language a capture was read in (§ 3.1). */
export interface Detection {
  /** Canonical code the card was built for. */
  lang: string
  method: DetectionMethod
  confident: boolean
  script: Script
  /** The languages the approval window offers, in settings order; contains `lang`. */
  candidates: string[]
  /** The statistical scores, best first; empty unless they were computed. */
  ranked: LanguageScore[]
}

/** Where a pending card will be filed (`AppConfig::filing` on the backend). */
export interface PendingFiling {
  deck: string
  /** The Anki note type. */
  model: string
}

/** The capture waiting in the approval window. */
export interface PendingCapture {
  card: Card
  warnings: PipelineWarning[]
  /** The hotkey profile that captured it; `null` for the default hotkey. */
  profile_id: string | null
  /** `null` when the source language was not detected (a hotkey profile, or no learned languages). */
  detection: Detection | null
  filing: PendingFiling
  /** Which capture this is; a re-run keeps the number (`rerunPendingCapture` names it). */
  generation: number
}

export type CaptureOrigin = 'capture' | 'curriculum' | 'anki_import'

/** A row of the local history (`captures` table). */
export interface StoredCapture {
  id: number
  card: Card
  origin: CaptureOrigin
  synced_to_anki: boolean
}

/** What `execute_card_generation` did with an approved card. */
export interface SubmitResult {
  capture_id: number
  synced_to_anki: boolean
  note_ids: number[]
  /** Set when the card was saved locally but Anki could not take it. */
  anki_error: string | null
}

export interface SyncFailure {
  id: number
  error: string
}

/** What `sync_pending_captures` pushed. */
export interface SyncReport {
  attempted: number
  synced: number
  failed: SyncFailure[]
}

/** The live AnkiConnect check. `decks`/`models` are empty while offline. */
export interface AnkiStatus {
  online: boolean
  version: number | null
  decks: string[]
  models: string[]
  error: string | null
}

/** When the user agreed to each install action (§ 8.4); `null` = never. */
export interface ConsentState {
  anki_install_granted_at: string | null
  ankiconnect_provision_granted_at: string | null
}

/** The install actions that need consent first. */
export type ConsentAction = 'anki_install' | 'ankiconnect_provision'

/** The disk-only setup check: nothing is downloaded or launched to answer it. */
export interface SetupStatus {
  anki_installed: boolean
  ankiconnect_installed: boolean
  /** Whether `download_and_install_anki` can run here (macOS and Windows). */
  platform_supported: boolean
  /** The pinned Anki release the installer would fetch. */
  anki_version: string
  consent: ConsentState
}

/** Learn's daily queue entry (§ 11); unused by the Automator, mirrored for completeness. */
export type QueueItem =
  | { kind: 'review'; card_id: number; due_date: string }
  | { kind: 'new'; hanzi: string; curriculum_order: number }

/**
 * A card value an Anki field can be filled with (`anki_bridge::CardField`).
 * `example_source`/`example_target` follow the note's sides (swapped on the
 * reverse note); `example_in_source_lang`/`example_in_target_lang` follow
 * the card's languages and are never swapped.
 */
export type CardField =
  | 'front'
  | 'back'
  | 'example_source'
  | 'example_target'
  | 'example'
  | 'notes'
  | 'pinyin'
  | 'hsk_level'
  | 'example_in_source_lang'
  | 'example_in_target_lang'

/** "Write `source` into the Anki field `anki_field`." */
export interface FieldBinding {
  anki_field: string
  source: CardField
}

/** How cards map onto one note type; `audio_field: null` sends no audio. */
export interface FieldMapping {
  fields: FieldBinding[]
  audio_field: string | null
}

/** A mapping matched from a note type's field names (`anki_bridge::AutoMatch`). */
export interface AutoMatch {
  /** Bindings in note-type order. */
  mapping: FieldMapping
  /** Fields bound only by position, because no name fit. */
  guessed: string[]
  /** Fields left empty (the audio field excluded). */
  unmatched: string[]
}

/** A card value a push would not deliver (`anki_bridge::LostValue`). */
export type LostValue =
  /** The mapping names a field the note type lacks, and the card has a value for it. */
  | { kind: 'missing_field'; anki_field: string; source: CardField }
  /** A non-empty value no field of the note type receives. */
  | { kind: 'unmapped'; source: CardField }
  /** Pronunciation audio without a field (`null`) or with one the note type lacks. */
  | { kind: 'audio'; audio_field: string | null }

/** What `check_pending_fields` found out about the pending card's note type. */
export type FieldCheck =
  /** Anki could not be reached. */
  | { status: 'unchecked' }
  /** Anki answered with an error, e.g. the note type does not exist. */
  | { status: 'failed'; model: string; error: string }
  /** `lost` lists what a push would drop; `auto_matched`: the mapping was matched by field name. */
  | { status: 'checked'; model: string; auto_matched: boolean; lost: LostValue[] }

/** An extra capture shortcut with its own language pair and optional deck (Milestone H). */
export interface HotkeyProfile {
  id: string
  label: string
  shortcut: string
  source_lang: string
  target_lang: string
  /** `null` uses `anki.deck_name`. */
  deck_name: string | null
}

/** A language the user learns, and where its cards go (Milestone O). */
export interface LearnedLanguage {
  /** Canonical code (§ 3); never `translation.target_lang`. */
  lang: string
  deck_name: string
  model_name: string
  /** `null`: matched by field name against the note type at push time, never saved. */
  field_mapping: FieldMapping | null
  create_reverse: boolean
}

/** The languages the default hotkey detects among; empty keeps the fixed pair. */
export interface LearningConfig {
  /** In the order the approval window lists them. */
  languages: LearnedLanguage[]
}

export interface TranslationConfig {
  source_lang: string
  target_lang: string
  max_chars: number
  provider: string
  /** Sent to MyMemory to raise the daily quota (Milestone C); `null` = anonymous. */
  contact_email: string | null
}

export interface AnkiConfig {
  connect_url: string
  deck_name: string
  model_name: string
  create_reverse: boolean
  field_mapping: FieldMapping
}

/** `config.json` (§ 8.1), exactly as `get_current_settings` returns and `apply_settings` takes it. */
export interface AppConfig {
  ui_lang: string
  /** The default capture shortcut; uses `translation.*` and `anki.deck_name`. */
  hotkeys: string
  hotkey_profiles: HotkeyProfile[]
  translation: TranslationConfig
  anki: AnkiConfig
  consent: ConsentState
  learning: LearningConfig
}

/** Payload of the app-wide `capture-error` event. */
export interface CaptureErrorPayload {
  message: string
}
