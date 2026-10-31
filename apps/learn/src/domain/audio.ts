/**
 * Playback of the base64 MP3 the adapter returns (`getCardAudio`,
 * `synthesizeAudio`) through an `Audio` element. One element plays at a
 * time: starting a recording stops the previous one, so tapping "play"
 * repeatedly never stacks voices. Each recording belongs to an owner (the
 * play button that started it), which can stop it without cutting off a
 * recording another button started since.
 */

let current: { player: HTMLAudioElement; owner: object | null } | null = null

/** The `src` of an `Audio` element for base64 MP3 data. */
export function mp3DataUrl(base64: string): string {
  return `data:audio/mpeg;base64,${base64}`
}

/**
 * Plays `base64` MP3 for `owner`, stopping whatever played before. Rejects
 * when the webview refuses to play.
 */
export async function playMp3(base64: string, owner: object | null = null): Promise<void> {
  stopAudio()
  const player = new Audio(mp3DataUrl(base64))
  current = { player, owner }
  await player.play()
}

/** Stops the recording that is playing; with `owner`, only if that owner started it. */
export function stopAudio(owner?: object): void {
  if (!current || (owner !== undefined && current.owner !== owner)) return
  current.player.pause()
  current = null
}
