/**
 * Playback of the base64 MP3 the adapter returns (`getCardAudio`,
 * `synthesizeAudio`) through an `Audio` element. One element plays at a
 * time: starting a recording stops the previous one, so tapping "play"
 * repeatedly never stacks voices.
 */

let current: HTMLAudioElement | null = null

/** The `src` of an `Audio` element for base64 MP3 data. */
export function mp3DataUrl(base64: string): string {
  return `data:audio/mpeg;base64,${base64}`
}

/** Plays `base64` MP3, stopping whatever played before. Rejects when the webview refuses to play. */
export async function playMp3(base64: string): Promise<void> {
  current?.pause()
  const player = new Audio(mp3DataUrl(base64))
  current = player
  await player.play()
}

/** Stops the recording that is playing, if any. */
export function stopAudio(): void {
  current?.pause()
  current = null
}
