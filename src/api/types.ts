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

/** The capture waiting in the approval window. */
export interface PendingCapture {
  card: Card
  warnings: PipelineWarning[]
  /** The hotkey profile that captured it; `null` for the default hotkey. */
  profile_id: string | null
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
  | { kind: 'new_capture'; card_id: number }

/** A card value an Anki field can be filled with (`anki_bridge::CardField`). */
export type CardField =
  | 'front'
  | 'back'
  | 'example_source'
  | 'example_target'
  | 'example'
  | 'notes'
  | 'pinyin'
  | 'hsk_level'

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
}

/** Payload of the app-wide `capture-error` event. */
export interface CaptureErrorPayload {
  message: string
}
