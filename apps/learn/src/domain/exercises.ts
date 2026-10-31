/**
 * The exercise types of Milestone M2 as the practice screen offers them:
 * their labels, and which ones the running platform can play.
 *
 * Listening exercises need sound. A listening choice plays the card's own
 * recording when it has one (`has_audio`), else TTS; a listening sentence is
 * always synthesized, because captures store audio of the word, not of the
 * sentence (vocab_core::quiz::ListeningSentenceExercise). Without TTS
 * (`capabilities.tts`, false in the web build) a view therefore offers only
 * what it can actually play.
 */

import type { MessageKey } from '../i18n'
import type {
  ChoiceDirection,
  ExerciseKind,
  LearnedCard,
  PlatformCapabilities,
} from '../platform/types'

/** Every exercise type, in the order the practice screen lists them. */
export const EXERCISE_KINDS: readonly ExerciseKind[] = [
  'multiple_choice',
  'sentence_builder',
  'listening_choice',
  'listening_sentence',
]

export const KIND_KEYS: Record<ExerciseKind, MessageKey> = {
  multiple_choice: 'kinds.multipleChoice',
  sentence_builder: 'kinds.sentenceBuilder',
  listening_choice: 'kinds.listeningChoice',
  listening_sentence: 'kinds.listeningSentence',
}

/** The question a multiple-choice exercise asks in each direction. */
export const DIRECTION_KEYS: Record<ChoiceDirection, MessageKey> = {
  front_to_back: 'exercise.frontToBack',
  back_to_front: 'exercise.backToFront',
  front_to_pinyin: 'exercise.frontToPinyin',
}

/** Why `kind` cannot be played for `card` here, or `null` when it can. */
export function unavailableReason(
  kind: ExerciseKind,
  card: LearnedCard | null,
  capabilities: PlatformCapabilities,
): MessageKey | null {
  if (capabilities.tts) return null
  if (kind === 'listening_sentence') return 'practice.needsTts'
  if (kind === 'listening_choice' && !card?.has_audio) return 'practice.needsAudio'
  return null
}
