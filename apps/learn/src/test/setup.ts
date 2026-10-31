/**
 * Runs before every test file. jsdom implements no media playback, so
 * `play`/`pause` of every `Audio` element are stubbed (and observable as
 * spies); tests that need a refused playback override `play` themselves.
 */

import { afterEach, beforeEach, vi } from 'vitest'

import { stopAudio } from '../domain/audio'
import { setLocale } from '../i18n'

beforeEach(() => {
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(undefined)
  vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {})
  setLocale('en')
})

afterEach(() => {
  stopAudio()
  vi.restoreAllMocks()
  document.body.innerHTML = ''
})
