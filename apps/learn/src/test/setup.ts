/**
 * Runs before every test file. jsdom implements no media playback, so
 * `play`/`pause` of every `Audio` element are stubbed (and observable as
 * spies); tests that need a refused playback override `play` themselves.
 */

import { afterEach, beforeEach, vi } from 'vitest'

import { stopAudio } from '../domain/audio'
import { setLocale } from '../i18n'

// A test file may run in the Node environment instead (`@vitest-environment
// node`), which has no DOM to stub or clear.
const hasDom = typeof document !== 'undefined'

beforeEach(() => {
  if (hasDom) {
    vi.spyOn(HTMLMediaElement.prototype, 'play').mockResolvedValue(undefined)
    vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(() => {})
  }
  setLocale('en')
})

afterEach(() => {
  stopAudio()
  vi.restoreAllMocks()
  if (hasDom) document.body.innerHTML = ''
})
