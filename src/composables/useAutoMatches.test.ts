import { flushPromises } from '@vue/test-utils'
import { effectScope, nextTick, ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { AutoMatch } from '../api'
import { callsTo, deferred, onCommand, resetTauri } from '../test/fakeTauri'
import { createAutoMatches, type AutoMatchRequest, type AutoMatches } from './useAutoMatches'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

const MATCH: AutoMatch = {
  mapping: { fields: [{ anki_field: 'Front', source: 'front' }], audio_field: 'Audio' },
  guessed: [],
  unmatched: [],
}
const ENGLISH: AutoMatchRequest = {
  model: 'AppleStyleFinalV3',
  sourceLang: 'en',
  targetLang: 'de',
}

let scope: ReturnType<typeof effectScope> | null = null
const online = ref(true)
const requests = ref<AutoMatchRequest[]>([])

function matches(): AutoMatches {
  scope = effectScope()
  const created = scope.run(() =>
    createAutoMatches(
      () => online.value,
      () => requests.value,
    ),
  )
  if (!created) throw new Error('no cache')
  return created
}

beforeEach(() => {
  resetTauri()
  online.value = true
  requests.value = []
})

afterEach(() => {
  scope?.stop()
  scope = null
})

describe('createAutoMatches', () => {
  it('matches each note type and pair once, sharing one request', async () => {
    onCommand('suggest_field_mapping', MATCH)
    requests.value = [ENGLISH, { ...ENGLISH, model: ' AppleStyleFinalV3 ' }]
    const auto = matches()
    expect(auto.state(ENGLISH)).toEqual({ match: null, loading: true, error: null })
    await flushPromises()

    expect(auto.state(ENGLISH)).toEqual({ match: MATCH, loading: false, error: null })
    requests.value = [...requests.value, { ...ENGLISH, sourceLang: 'fr' }]
    await flushPromises()
    expect(callsTo('suggest_field_mapping').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3', sourceLang: 'en', targetLang: 'de' },
      { model: 'AppleStyleFinalV3', sourceLang: 'fr', targetLang: 'de' },
    ])
  })

  it('asks nothing offline or without a note type', async () => {
    onCommand('suggest_field_mapping', MATCH)
    requests.value = [{ ...ENGLISH, model: '  ' }]
    online.value = false
    const auto = matches()
    requests.value = [ENGLISH]
    await flushPromises()
    expect(callsTo('suggest_field_mapping')).toHaveLength(0)
    expect(auto.state(ENGLISH)).toEqual({ match: null, loading: false, error: null })
  })

  it('keeps the reason a match failed', async () => {
    onCommand('suggest_field_mapping', () => {
      throw 'AnkiConnect error: model was not found: Gone'
    })
    requests.value = [{ ...ENGLISH, model: 'Gone' }]
    const auto = matches()
    await flushPromises()
    expect(auto.state({ ...ENGLISH, model: 'Gone' })).toEqual({
      match: null,
      loading: false,
      error: 'AnkiConnect error: model was not found: Gone',
    })
  })

  it('forgets everything when Anki goes offline, including answers still on their way', async () => {
    const answer = deferred<AutoMatch>()
    onCommand('suggest_field_mapping', () => answer.promise)
    requests.value = [ENGLISH]
    const auto = matches()
    online.value = false
    await nextTick()
    answer.resolve(MATCH)
    await flushPromises()
    expect(auto.state(ENGLISH).match).toBeNull()

    onCommand('suggest_field_mapping', MATCH)
    online.value = true
    await flushPromises()
    expect(auto.state(ENGLISH).match).toEqual(MATCH)
    expect(callsTo('suggest_field_mapping')).toHaveLength(2)
  })
})
