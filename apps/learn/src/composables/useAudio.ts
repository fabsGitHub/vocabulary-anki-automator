/**
 * Plays a word or sentence the way the core's `AudioPrompt` describes it:
 * the card's stored recording (`getCardAudio`) when there is one, else TTS
 * of the text (`synthesizeAudio`), which only a platform with `tts` offers.
 *
 * Recordings are fetched when first played, never ahead (TTS is a network
 * call), and kept for repeats. A refused autoplay is not an error: the
 * listening exercises try to play on their own, and the "play" button is
 * there for when the webview wants a click first.
 *
 * Sound belongs to the component that asked for it. Once it is gone (the
 * next card, the next exercise) its recording stops, and a fetch still on
 * its way is never played: a slow TTS answer would otherwise speak the
 * previous word over the exercise that replaced it.
 */

import { onBeforeUnmount, ref } from 'vue'

import { usePlatform } from '../context'
import { playMp3, stopAudio } from '../domain/audio'
import { errorMessage } from '../domain/errors'

export interface AudioSource {
  /** The card whose stored recording comes first; `null` to go straight to TTS. */
  cardId: number | null
  /** A recording already at hand (a card's `audio_front`). */
  inline?: string | null
  /** What TTS speaks when there is no recording. */
  text: string
  lang: string
}

export type AudioStatus = 'idle' | 'loading' | 'unavailable' | 'failed'

export function useAudio() {
  const platform = usePlatform()
  const status = ref<AudioStatus>('idle')
  const error = ref<string | null>(null)
  const cache = new Map<string, string | null>()
  /** Marks this component's recordings, so only its own are stopped. */
  const owner = {}
  let disposed = false

  async function fetchAudio(source: AudioSource): Promise<string | null> {
    if (source.inline) return source.inline
    if (source.cardId !== null) {
      const stored = await platform.getCardAudio(source.cardId)
      if (stored) return stored
    }
    if (!platform.capabilities.tts || !source.text.trim()) return null
    return platform.synthesizeAudio(source.text, source.lang)
  }

  /** Plays `source`; `quiet` keeps a refused autoplay silent. */
  async function play(source: AudioSource, { quiet = false } = {}): Promise<void> {
    if (status.value === 'loading' || disposed) return
    status.value = 'loading'
    error.value = null
    const key = JSON.stringify([source.cardId, source.text, source.lang, Boolean(source.inline)])
    let audio: string | null | undefined = cache.get(key)
    try {
      if (audio === undefined) {
        audio = await fetchAudio(source)
        cache.set(key, audio)
      }
    } catch (err) {
      status.value = 'failed'
      error.value = errorMessage(err)
      return
    }
    if (disposed) return
    if (!audio) {
      status.value = 'unavailable'
      return
    }
    try {
      await playMp3(audio, owner)
      status.value = 'idle'
    } catch (err) {
      status.value = quiet ? 'idle' : 'failed'
      error.value = quiet ? null : errorMessage(err)
    }
  }

  onBeforeUnmount(() => {
    disposed = true
    stopAudio(owner)
  })

  return { status, error, play }
}
