/**
 * `vocab_core::model`: the neutral card and the curriculum row, as the port
 * works with them, and their serde shapes (`../types`).
 *
 * Inside the port an instant is a number of milliseconds, the precision the
 * store keeps, and audio is bytes; the IPC shapes carry RFC 3339 text and
 * base64. The `*FromJson` functions apply serde's defaults, so a card or a
 * curriculum row parses exactly when the Rust type would accept it: the
 * bundled curriculum omits every empty optional field.
 */

import type * as ipc from '../types'
import { formatInstant, parseInstantMillis } from './time'

/** `ScriptMetadata`, already in its serde shape. */
export type ScriptMetadata = ipc.ScriptMetadata

/** `CaptureOrigin`. */
export type CaptureOrigin = ipc.StoredCapture['origin']

/** `vocab_core::model::Card`. */
export interface Card {
  front: string
  back: string
  example_source: string
  example_target: string
  notes: string
  tags: string[]
  source_lang: string
  target_lang: string
  /** Milliseconds since the epoch. */
  captured_at: number
  /** TTS audio (MP3) of `front`. */
  audio_front: Uint8Array | null
  script_metadata: ScriptMetadata
}

/** `vocab_core::model::StoredCapture`. */
export interface StoredCapture {
  id: number
  card: Card
  origin: CaptureOrigin
  synced_to_anki: boolean
}

/** `vocab_core::model::CurriculumItem`, a row of the bundled HSK 3.0 curriculum. */
export interface CurriculumItem {
  hanzi: string
  traditional: string | null
  hsk_level: number
  frequency: number | null
  curriculum_order: number
  pinyin: string
  pos: string | null
  meaning_en: string | null
  meaning_de: string | null
  cloze_sentence: string | null
  example_sentence_source: string | null
  example_sentence_pinyin: string | null
  example_sentence_target: string | null
  vocab_audio_path: string | null
  sentence_audio_path: string | null
  image_path: string | null
}

/** `Card::new`: the required fields set, everything else empty. */
export function newCard(
  front: string,
  back: string,
  sourceLang: string,
  targetLang: string,
  capturedAt: number,
): Card {
  return {
    front,
    back,
    example_source: '',
    example_target: '',
    notes: '',
    tags: [],
    source_lang: sourceLang,
    target_lang: targetLang,
    captured_at: capturedAt,
    audio_front: null,
    script_metadata: { kind: 'none' },
  }
}

/** `ScriptMetadata::kind`, the value of `captures.script_kind`. */
export function scriptKind(metadata: ScriptMetadata): 'none' | 'chinese' {
  return metadata.kind
}

/** `Card::pinyin`. */
export function cardPinyin(card: Card): string | null {
  return card.script_metadata.kind === 'chinese' ? card.script_metadata.pinyin : null
}

/** `Card::hsk_level`. */
export function cardHskLevel(card: Card): number | null {
  return card.script_metadata.kind === 'chinese' ? card.script_metadata.hsk_level : null
}

/** A card's serde shape. */
export function cardToJson(card: Card): ipc.Card {
  return {
    front: card.front,
    back: card.back,
    example_source: card.example_source,
    example_target: card.example_target,
    notes: card.notes,
    tags: [...card.tags],
    source_lang: card.source_lang,
    target_lang: card.target_lang,
    captured_at: formatInstant(card.captured_at),
    audio_front: card.audio_front === null ? null : encodeBase64(card.audio_front),
    script_metadata: { ...card.script_metadata },
  }
}

/** A card from its serde shape, with serde's defaults for the optional fields. */
export function cardFromJson(value: ipc.Card): Card {
  const partial = value as Partial<ipc.Card>
  const metadata = partial.script_metadata ?? { kind: 'none' }
  return {
    front: value.front,
    back: value.back,
    example_source: partial.example_source ?? '',
    example_target: partial.example_target ?? '',
    notes: partial.notes ?? '',
    tags: [...(partial.tags ?? [])],
    source_lang: value.source_lang,
    target_lang: value.target_lang,
    captured_at: parseInstantMillis(value.captured_at),
    audio_front: partial.audio_front ? decodeBase64(partial.audio_front) : null,
    script_metadata:
      metadata.kind === 'chinese'
        ? {
            kind: 'chinese',
            pinyin: metadata.pinyin,
            hsk_level: metadata.hsk_level ?? null,
            script_variant: metadata.script_variant ?? 'simplified',
          }
        : { kind: 'none' },
  }
}

/** A stored capture's serde shape. */
export function storedCaptureToJson(capture: StoredCapture): ipc.StoredCapture {
  return {
    id: capture.id,
    card: cardToJson(capture.card),
    origin: capture.origin,
    synced_to_anki: capture.synced_to_anki,
  }
}

/** A curriculum row from JSON in which empty optional fields may be missing. */
export function curriculumItemFromJson(value: Partial<CurriculumItem>): CurriculumItem {
  return {
    hanzi: value.hanzi ?? '',
    traditional: value.traditional ?? null,
    hsk_level: value.hsk_level ?? 0,
    frequency: value.frequency ?? null,
    curriculum_order: value.curriculum_order ?? 0,
    pinyin: value.pinyin ?? '',
    pos: value.pos ?? null,
    meaning_en: value.meaning_en ?? null,
    meaning_de: value.meaning_de ?? null,
    cloze_sentence: value.cloze_sentence ?? null,
    example_sentence_source: value.example_sentence_source ?? null,
    example_sentence_pinyin: value.example_sentence_pinyin ?? null,
    example_sentence_target: value.example_sentence_target ?? null,
    vocab_audio_path: value.vocab_audio_path ?? null,
    sentence_audio_path: value.sentence_audio_path ?? null,
    image_path: value.image_path ?? null,
  }
}

/** Base64 (standard alphabet, padded), as `base64::engine::general_purpose::STANDARD` writes it. */
export function encodeBase64(bytes: Uint8Array): string {
  let binary = ''
  for (let start = 0; start < bytes.length; start += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(start, start + 0x8000))
  }
  return btoa(binary)
}

/** The bytes of a base64 text. */
export function decodeBase64(text: string): Uint8Array {
  const binary = atob(text)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}
